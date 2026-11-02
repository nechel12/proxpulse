//! Download speed through a proxy, with the time split into connecting to
//! the proxy, the TLS handshake and the wait for the first byte.

use crate::commands::check::{BusyGuard, CheckState};
use crate::http::short_error;
use crate::tls::tls_client_config;
use crate::transport::{connect_via, parse_upstream_url, Upstream};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::State;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const SPEED_HOST: &str = "speed.cloudflare.com";
/// Proxies measured at once; more would split the bandwidth between them.
const SPEED_CONCURRENCY: usize = 3;
const SPEED_MAX_PROXIES: usize = 200;
const SPEED_TIMEOUT: Duration = Duration::from_secs(10);
/// The download stops here even if not all bytes arrived.
const DOWNLOAD_LIMIT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpeedResult {
    pub(crate) proxy: String,
    pub(crate) ok: bool,
    pub(crate) connect_ms: Option<u64>,
    pub(crate) tls_ms: Option<u64>,
    pub(crate) ttfb_ms: Option<u64>,
    /// Megabits per second over the body download.
    pub(crate) mbps: Option<f64>,
    pub(crate) bytes: u64,
    pub(crate) error: Option<String>,
}

impl SpeedResult {
    fn failed(proxy: &str, error: String) -> Self {
        Self {
            proxy: proxy.to_string(),
            ok: false,
            connect_ms: None,
            tls_ms: None,
            ttfb_ms: None,
            mbps: None,
            bytes: 0,
            error: Some(error),
        }
    }
}

/// Megabits per second; None when the interval is too short to mean anything.
pub(crate) fn mbps(bytes: u64, elapsed: Duration) -> Option<f64> {
    let secs = elapsed.as_secs_f64();
    (secs >= 0.001 && bytes > 0)
        .then(|| (bytes as f64 * 8.0 / 1_000_000.0 / secs * 100.0).round() / 100.0)
}

/// Position after the header block and the Content-Length, if any.
pub(crate) fn parse_head(buf: &[u8]) -> Option<(usize, u16, Option<u64>)> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = std::str::from_utf8(&buf[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let length = lines.find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| v.trim().parse().ok())?
    });
    Some((end, status, length))
}

async fn download<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    bytes: u64,
    result: &mut SpeedResult,
) -> Result<(), String> {
    let req = format!(
        "GET /__down?bytes={bytes} HTTP/1.1\r\nHost: {SPEED_HOST}\r\nUser-Agent: ProxPulse\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n"
    );
    let sent = Instant::now();
    s.write_all(req.as_bytes())
        .await
        .map_err(|e| short_error(&e.to_string()))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut head = Vec::new();
    let (mut body, status, length) = loop {
        let n = tokio::time::timeout(SPEED_TIMEOUT, s.read(&mut buf))
            .await
            .map_err(|_| "timeout".to_string())?
            .map_err(|e| short_error(&e.to_string()))?;
        if n == 0 {
            return Err("connection closed".to_string());
        }
        if result.ttfb_ms.is_none() {
            result.ttfb_ms = Some(sent.elapsed().as_millis() as u64);
        }
        head.extend_from_slice(&buf[..n]);
        if let Some((end, status, length)) = parse_head(&head) {
            break ((head.len() - end) as u64, status, length);
        }
        if head.len() > 64 * 1024 {
            return Err("bad response".to_string());
        }
    };
    if status != 200 {
        return Err(format!("http {status}"));
    }
    let want = length.unwrap_or(bytes);
    let started = Instant::now();
    while body < want && started.elapsed() < DOWNLOAD_LIMIT {
        match tokio::time::timeout(SPEED_TIMEOUT, s.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => body += n as u64,
            Ok(Err(e)) => return Err(short_error(&e.to_string())),
            Err(_) => break,
        }
    }
    result.bytes = body;
    result.mbps = mbps(body, started.elapsed());
    result.ok = result.mbps.is_some();
    if !result.ok {
        result.error = Some("no data".to_string());
    }
    Ok(())
}

async fn measure(proxy: String, url: String, bytes: u64) -> SpeedResult {
    let Some((scheme, host, port, user, pass)) = parse_upstream_url(&url) else {
        return SpeedResult::failed(&proxy, "bad proxy url".to_string());
    };
    let up = Upstream {
        raw: url.clone(),
        url,
        scheme,
        host,
        port,
        user,
        pass,
        latency: None,
        country: None,
    };
    let mut result = SpeedResult::failed(&proxy, String::new());
    result.error = None;
    let start = Instant::now();
    let stream = match connect_via(&up, SPEED_HOST, 443, SPEED_TIMEOUT).await {
        Ok(s) => s,
        Err(e) => return SpeedResult::failed(&proxy, short_error(&e.to_string())),
    };
    result.connect_ms = Some(start.elapsed().as_millis() as u64);
    let name = match rustls_pki_types::ServerName::try_from(SPEED_HOST) {
        Ok(n) => n.to_owned(),
        Err(_) => return SpeedResult::failed(&proxy, "bad tls name".to_string()),
    };
    let tls_start = Instant::now();
    let connector = tokio_rustls::TlsConnector::from(tls_client_config());
    let tls = match tokio::time::timeout(SPEED_TIMEOUT, connector.connect(name, stream)).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return SpeedResult::failed(&proxy, short_error(&e.to_string())),
        Err(_) => return SpeedResult::failed(&proxy, "tls handshake timeout".to_string()),
    };
    result.tls_ms = Some(tls_start.elapsed().as_millis() as u64);
    if let Err(e) = download(tls, bytes, &mut result).await {
        result.ok = false;
        result.error = Some(e);
    }
    result
}

/// A proxy to measure: the result row it belongs to and its normalized URL.
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct SpeedTarget {
    proxy: String,
    url: String,
}

#[tauri::command]
pub(crate) async fn speed_test(
    targets: Vec<SpeedTarget>,
    bytes: u64,
    state: State<'_, CheckState>,
    on_result: Channel<SpeedResult>,
) -> Result<(), String> {
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("another check is running".to_string());
    }
    let _busy = BusyGuard(state.busy.clone());
    state.cancel.store(false, Ordering::SeqCst);
    let cancel: Arc<AtomicBool> = state.cancel.clone();
    let bytes = bytes.clamp(100_000, 100_000_000);
    let sem = Arc::new(tokio::sync::Semaphore::new(SPEED_CONCURRENCY));
    let mut set = tokio::task::JoinSet::new();
    for t in targets.into_iter().take(SPEED_MAX_PROXIES) {
        let sem = sem.clone();
        let cancel = cancel.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.ok()?;
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let limit = SPEED_TIMEOUT * 3 + DOWNLOAD_LIMIT;
            let proxy = t.proxy.clone();
            Some(
                tokio::time::timeout(limit, measure(t.proxy, t.url, bytes))
                    .await
                    .unwrap_or_else(|_| SpeedResult::failed(&proxy, "timeout".to_string())),
            )
        });
    }
    while let Some(r) = set.join_next().await {
        if let Ok(Some(result)) = r {
            let _ = on_result.send(result);
        }
    }
    Ok(())
}
