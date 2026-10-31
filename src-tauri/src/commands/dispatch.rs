use crate::transport::{
    close_after_response, connect_to_http_proxy, connect_via, find_header_end, parse_upstream_url,
    prepare_forward_request, read_exact_t, strip_header, write_all_t, Upstream,
};
use crate::{normalize_proxy, strip_credentials};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tauri::{Manager, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Semaphore};

// ================= local proxy dispatcher =================

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DispatchItem {
    pub(crate) raw: String,
    /// Normalized URL from the checker. Falls back to `raw` for old callers.
    #[serde(default)]
    pub(crate) url: Option<String>,
    pub(crate) latency: Option<u64>,
    #[serde(default)]
    pub(crate) country: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DispatchStatus {
    running: bool,
    port: u16,
    mode: String,
    listener: String,
    upstreams: usize,
    current: Option<String>,
    requests: u64,
    errors: u64,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DispatchPoolItem {
    pub(crate) raw: String,
    pub(crate) url: String,
    pub(crate) latency: Option<u64>,
    pub(crate) country: Option<String>,
}

pub(crate) async fn pool_items(inner: &Arc<DispatchInner>) -> Vec<DispatchPoolItem> {
    inner
        .pool
        .lock()
        .await
        .iter()
        .map(|u| DispatchPoolItem {
            raw: u.raw.clone(),
            url: u.url.clone(),
            latency: u.latency,
            country: u.country.clone(),
        })
        .collect()
}

pub(crate) struct DispatchInner {
    pub(crate) running: AtomicBool,
    pub(crate) port: Mutex<u16>,
    pub(crate) mode: Mutex<String>,
    pub(crate) listener: Mutex<String>,
    pub(crate) pool: Mutex<Vec<Upstream>>,
    pub(crate) rr: AtomicUsize,
    pub(crate) requests: AtomicU64,
    pub(crate) errors: AtomicU64,
    pub(crate) current: Mutex<Option<String>>,
    pub(crate) last_error: Mutex<Option<String>>,
    pub(crate) handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(crate) lifecycle: Mutex<()>,
    /// All active client connections, so Stop can terminate tunnels instead
    /// of leaving them running after the listener is gone.
    pub(crate) connections: Mutex<tokio::task::JoinSet<()>>,
    /// Cached loopback checks for upstream hostnames (`host:port` -> result).
    pub(crate) dns_cache: Mutex<HashMap<String, (bool, Instant)>>,
    /// Caps local client connections; without it a local process could
    /// exhaust file descriptors.
    pub(crate) conn_sem: Arc<Semaphore>,
    /// Consecutive failed auto-checks per upstream URL. A proxy is dropped
    /// only after several misses, so one flaky cycle does not empty the pool.
    pub(crate) fails: Mutex<HashMap<String, u32>>,
    /// Serializes writes of the persisted pool file.
    pub(crate) save_lock: Mutex<()>,
}

const MAX_CONNECTIONS: usize = 1024;

pub(crate) struct DispatchState {
    pub(crate) inner: Arc<DispatchInner>,
}

impl Default for DispatchState {
    fn default() -> Self {
        Self {
            inner: Arc::new(DispatchInner {
                running: AtomicBool::new(false),
                port: Mutex::new(1080),
                mode: Mutex::new("round-robin".to_string()),
                listener: Mutex::new("both".to_string()),
                pool: Mutex::new(Vec::new()),
                rr: AtomicUsize::new(0),
                requests: AtomicU64::new(0),
                errors: AtomicU64::new(0),
                current: Mutex::new(None),
                last_error: Mutex::new(None),
                handle: Mutex::new(None),
                lifecycle: Mutex::new(()),
                connections: Mutex::new(tokio::task::JoinSet::new()),
                dns_cache: Mutex::new(HashMap::new()),
                conn_sem: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
                fails: Mutex::new(HashMap::new()),
                save_lock: Mutex::new(()),
            }),
        }
    }
}

/// Return at most `limit` upstream indexes. The pool is kept sorted by
/// latency in `set_dispatch_pool`, so "fastest"/"failover" are just the head
/// of the list; no per-request sort or full-pool allocation is needed.
pub(crate) fn pick_order(len: usize, mode: &str, rr: &AtomicUsize, limit: usize) -> Vec<usize> {
    if len == 0 || limit == 0 {
        return vec![];
    }
    match mode {
        // Strictly one attempt: the fastest upstream is either used or the
        // request fails. Fallback is the explicit "failover" mode.
        "fastest" => vec![0],
        "round-robin" => {
            let start = rr.fetch_add(1, Ordering::Relaxed) % len;
            (0..len.min(limit)).map(|i| (start + i) % len).collect()
        }
        _ => (0..len.min(limit)).collect(),
    }
}

pub(crate) fn parse_target(method: &str, target: &str) -> Option<(String, u16)> {
    if method.eq_ignore_ascii_case("CONNECT") {
        let (h, p) = target.rsplit_once(':')?;
        let h = h.trim();
        let host = h
            .strip_prefix('[')
            .and_then(|x| x.strip_suffix(']'))
            .unwrap_or(h);
        return Some((host.to_string(), p.trim().parse().ok()?));
    }
    let rest = target
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("http://"))
        .map(|_| &target[7..])?;
    let auth = rest.split(['/', '?', '#']).next()?;
    if let Some(stripped) = auth.strip_prefix('[') {
        let end = stripped.find(']')?;
        let host = stripped[..end].to_string();
        let after = &stripped[end + 1..];
        let port: u16 = after.strip_prefix(':').unwrap_or("80").parse().ok()?;
        return Some((host, port));
    }
    match auth.rsplit_once(':') {
        Some((h, p)) => Some((h.to_string(), p.parse().ok()?)),
        None => Some((auth.to_string(), 80)),
    }
}

const MAX_UPSTREAM_ATTEMPTS: usize = 5;

pub(crate) fn is_self_literal(host: &str) -> bool {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if let Ok(ip) = host.parse::<IpAddr>() {
        return ip.is_loopback() || ip.is_unspecified();
    }
    matches!(
        host.as_str(),
        "localhost" | "localhost.localdomain" | "ip6-localhost"
    )
}

pub(crate) fn is_self_upstream(up: &Upstream, port: u16) -> bool {
    up.port == port && is_self_literal(&up.host)
}

/// Resolve hostname aliases such as `127.0.0.1.nip.io` that point at the
/// local listener. Results are cached for a few minutes; only entries with
/// the listener's port are resolved, so the cost is normally tiny.
pub(crate) async fn is_self_upstream_async(
    up: &Upstream,
    port: u16,
    cache: &Mutex<HashMap<String, (bool, Instant)>>,
) -> bool {
    if up.port != port {
        return false;
    }
    if is_self_literal(&up.host) {
        return true;
    }
    if up.host.parse::<IpAddr>().is_ok() {
        return false;
    }
    let key = format!("{}:{}", up.host.to_ascii_lowercase(), up.port);
    {
        let c = cache.lock().await;
        if let Some((v, ts)) = c.get(&key) {
            if ts.elapsed() < Duration::from_secs(300) {
                return *v;
            }
        }
    }
    let is_self = match tokio::time::timeout(
        Duration::from_millis(800),
        tokio::net::lookup_host((up.host.as_str(), up.port)),
    )
    .await
    {
        Ok(Ok(addrs)) => addrs
            .into_iter()
            .any(|a| a.ip().is_loopback() || a.ip().is_unspecified()),
        _ => false,
    };
    let mut c = cache.lock().await;
    if c.len() > 1000 {
        c.clear();
    }
    c.insert(key, (is_self, Instant::now()));
    is_self
}

async fn take_upstreams(st: &Arc<DispatchInner>) -> Vec<Upstream> {
    let port = *st.port.lock().await;
    let mode = st.mode.lock().await.clone();
    let candidates: Vec<Upstream> = {
        let pool = st.pool.lock().await;
        if pool.is_empty() {
            return vec![];
        }
        let order = pick_order(pool.len(), &mode, &st.rr, MAX_UPSTREAM_ATTEMPTS + 4);
        order.iter().map(|&i| pool[i].clone()).collect()
    };
    let mut out = Vec::new();
    for up in candidates {
        if out.len() >= MAX_UPSTREAM_ATTEMPTS {
            break;
        }
        if is_self_upstream_async(&up, port, &st.dns_cache).await {
            continue;
        }
        out.push(up);
    }
    out
}

pub(crate) struct CountingIo<T> {
    inner: T,
    written: Arc<AtomicU64>,
}

impl<T> CountingIo<T> {
    pub(crate) fn new(inner: T, written: Arc<AtomicU64>) -> Self {
        Self { inner, written }
    }

    fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for CountingIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for CountingIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                if n > 0 {
                    this.written.fetch_add(n as u64, Ordering::Relaxed);
                }
                Poll::Ready(Ok(n))
            }
            other => other,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

pub(crate) async fn abort_connections(inner: &Arc<DispatchInner>) {
    let mut conns = inner.connections.lock().await;
    conns.abort_all();
    while conns.join_next().await.is_some() {}
}

pub(crate) async fn handle_http(mut client: TcpStream, st: Arc<DispatchInner>) {
    // read request head
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        if buf.len() > 65536 {
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        let n = match tokio::time::timeout(remaining, client.read(&mut tmp)).await {
            Ok(Ok(0)) => return,
            Ok(Ok(n)) => n,
            _ => return,
        };
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let hdr_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(buf.len());
    let mut it = head.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let target = it.next().unwrap_or("").to_string();
    let (t_host, t_port) = match parse_target(&method, &target) {
        Some(v) => v,
        None => {
            let _ = client.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
            return;
        }
    };
    let is_connect = method.eq_ignore_ascii_case("CONNECT");

    let order = take_upstreams(&st).await;
    if order.is_empty() {
        let _ = client.write_all(b"HTTP/1.1 502 No upstreams\r\n\r\n").await;
        return;
    }

    // Retrying a request after a partial write is only safe for methods
    // without a request body.
    let can_retry = matches!(
        method.to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS" | "TRACE"
    );
    let mut last_err = String::new();
    for up in &order {
        // Plain HTTP via an HTTP(S) proxy is forwarded in absolute-form;
        // SOCKS and CONNECT always use a tunnel.
        let use_forward = !is_connect && matches!(up.scheme.as_str(), "http" | "https");
        let connected = if use_forward {
            connect_to_http_proxy(up, Duration::from_secs(8)).await
        } else {
            connect_via(up, &t_host, t_port, Duration::from_secs(8)).await
        };
        match connected {
            Ok(mut ups) => {
                *st.current.lock().await = Some(strip_credentials(&up.raw));
                st.requests.fetch_add(1, Ordering::Relaxed);
                if is_connect {
                    // The client already received 200 before the tunnel
                    // starts, so a failure here cannot be retried.
                    let res: std::io::Result<()> = async {
                        client
                            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                            .await?;
                        if hdr_end < buf.len() {
                            ups.write_all(&buf[hdr_end..]).await?;
                        }
                        tokio::io::copy_bidirectional(&mut client, &mut ups).await?;
                        Ok(())
                    }
                    .await;
                    if res.is_err() {
                        st.errors.fetch_add(1, Ordering::Relaxed);
                    }
                    return;
                }
                let request = if use_forward {
                    prepare_forward_request(&buf, hdr_end, up)
                } else {
                    // SOCKS tunnel: strip local-proxy headers before handing
                    // the request to the origin.
                    let stripped = strip_header(&buf, hdr_end, "proxy-authorization");
                    let end = find_header_end(&stripped).unwrap_or(stripped.len());
                    let stripped = strip_header(&stripped, end, "proxy-connection");
                    let end = find_header_end(&stripped).unwrap_or(stripped.len());
                    strip_header(&stripped, end, "keep-alive")
                };
                // One request per client connection: only this request was
                // rewritten for (and routed to) this upstream.
                let end = find_header_end(&request).unwrap_or(request.len());
                let write_res = ups.write_all(&close_after_response(&request, end)).await;
                if let Err(e) = write_res {
                    st.errors.fetch_add(1, Ordering::Relaxed);
                    last_err = e.to_string();
                    if can_retry {
                        continue;
                    }
                    return;
                }
                let written = Arc::new(AtomicU64::new(0));
                let mut counted = CountingIo::new(client, written.clone());
                let copy_res = tokio::io::copy_bidirectional(&mut counted, &mut ups).await;
                client = counted.into_inner();
                if let Err(e) = copy_res {
                    st.errors.fetch_add(1, Ordering::Relaxed);
                    last_err = e.to_string();
                    if can_retry && written.load(Ordering::Relaxed) == 0 {
                        continue;
                    }
                }
                return;
            }
            Err(e) => {
                last_err = e.to_string();
                continue;
            }
        }
    }
    st.errors.fetch_add(1, Ordering::Relaxed);
    *st.last_error.lock().await = Some(format!(
        "{}:{} via {} upstreams: {}",
        t_host,
        t_port,
        order.len(),
        last_err
    ));
    let _ = client
        .write_all(b"HTTP/1.1 502 All upstreams failed\r\n\r\n")
        .await;
}

pub(crate) async fn handle_socks5(mut client: TcpStream, st: Arc<DispatchInner>) {
    let d = Duration::from_secs(10);
    let mut hdr = [0u8; 2];
    if read_exact_t(&mut client, &mut hdr, d).await.is_err() || hdr[0] != 0x05 {
        return;
    }
    let n = hdr[1] as usize;
    if n == 0 || n > 16 {
        return;
    }
    let mut methods = vec![0u8; n];
    if read_exact_t(&mut client, &mut methods, d).await.is_err() {
        return;
    }
    if !methods.contains(&0x00) {
        let _ = write_all_t(&mut client, &[0x05, 0xFF], d).await;
        return;
    }
    if write_all_t(&mut client, &[0x05, 0x00], d).await.is_err() {
        return;
    }
    let mut rh = [0u8; 4];
    if read_exact_t(&mut client, &mut rh, d).await.is_err() {
        return;
    }
    if rh[0] != 0x05 || rh[1] != 0x01 {
        let _ = write_all_t(&mut client, &[0x05, 0x07, 0, 1, 0, 0, 0, 0, 0, 0], d).await;
        return;
    }
    let (t_host, t_port) = match rh[3] {
        0x01 => {
            let mut b = [0u8; 6];
            if read_exact_t(&mut client, &mut b, d).await.is_err() {
                return;
            }
            (
                format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]),
                u16::from_be_bytes([b[4], b[5]]),
            )
        }
        0x03 => {
            let mut l = [0u8; 1];
            if read_exact_t(&mut client, &mut l, d).await.is_err() {
                return;
            }
            let mut b = vec![0u8; l[0] as usize + 2];
            if read_exact_t(&mut client, &mut b, d).await.is_err() {
                return;
            }
            let dl = l[0] as usize;
            (
                String::from_utf8_lossy(&b[..dl]).to_string(),
                u16::from_be_bytes([b[dl], b[dl + 1]]),
            )
        }
        0x04 => {
            let mut b = [0u8; 18];
            if read_exact_t(&mut client, &mut b, d).await.is_err() {
                return;
            }
            let segs: Vec<String> = b[..16]
                .chunks(2)
                .map(|c| format!("{:02x}{:02x}", c[0], c[1]))
                .collect();
            (segs.join(":"), u16::from_be_bytes([b[16], b[17]]))
        }
        _ => {
            let _ = write_all_t(&mut client, &[0x05, 0x08, 0, 1, 0, 0, 0, 0, 0, 0], d).await;
            return;
        }
    };

    let order = take_upstreams(&st).await;
    if order.is_empty() {
        let _ = write_all_t(&mut client, &[0x05, 0x02, 0, 1, 0, 0, 0, 0, 0, 0], d).await;
        return;
    }
    let mut last_err = String::new();
    for up in &order {
        match connect_via(up, &t_host, t_port, Duration::from_secs(8)).await {
            Ok(mut u) => {
                *st.current.lock().await = Some(strip_credentials(&up.raw));
                st.requests.fetch_add(1, Ordering::Relaxed);
                if write_all_t(&mut client, &[0x05, 0x00, 0x00, 1, 0, 0, 0, 0, 0, 0], d)
                    .await
                    .is_err()
                {
                    st.errors.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                if tokio::io::copy_bidirectional(&mut client, &mut u)
                    .await
                    .is_err()
                {
                    st.errors.fetch_add(1, Ordering::Relaxed);
                }
                return;
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
    }
    st.errors.fetch_add(1, Ordering::Relaxed);
    *st.last_error.lock().await = Some(format!("socks {t_host}:{t_port}: {last_err}"));
    let _ = write_all_t(&mut client, &[0x05, 0x05, 0, 1, 0, 0, 0, 0, 0, 0], d).await;
}

async fn handle_conn(client: TcpStream, st: Arc<DispatchInner>) {
    let lmode = st.listener.lock().await.clone();
    if lmode != "http" {
        // SOCKS5 handshake starts with 0x05, HTTP with ASCII method
        let mut one = [0u8; 1];
        match tokio::time::timeout(Duration::from_secs(10), client.peek(&mut one)).await {
            Ok(Ok(1)) if one[0] == 0x05 => {
                handle_socks5(client, st).await;
                return;
            }
            Ok(Ok(_)) => {}
            _ => {
                if lmode == "socks5" {
                    handle_socks5(client, st).await;
                }
                return;
            }
        }
        if lmode == "socks5" {
            handle_socks5(client, st).await;
            return;
        }
    }
    handle_http(client, st).await;
}

async fn run_server(listener: TcpListener, st: Arc<DispatchInner>) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(v) => v,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(permit) = st.conn_sem.clone().try_acquire_owned() else {
            continue;
        };
        let stc = st.clone();
        let mut conns = st.connections.lock().await;
        while conns.try_join_next().is_some() {}
        conns.spawn(async move {
            let _permit = permit;
            handle_conn(stream, stc).await;
        });
    }
}

/// Build an upstream from a pool item. The normalized `url` from the checker
/// takes precedence over the raw input, so a proxy tested as SOCKS5 is not
/// silently re-normalized as HTTP in the dispatcher.
pub(crate) fn upstream_from_item(
    raw: &str,
    url: Option<&str>,
    latency: Option<u64>,
    country: Option<String>,
) -> Option<(String, Upstream)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let source = url.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(raw);
    let (normalized, _) = normalize_proxy(source, "http")?;
    let (scheme, host, port, user, pass) = parse_upstream_url(&normalized)?;
    Some((
        normalized.clone(),
        Upstream {
            raw: raw.to_string(),
            url: normalized,
            scheme,
            host,
            port,
            user,
            pass,
            latency,
            country,
        },
    ))
}

/// Replace the pool. `stop_when_empty` is for explicit user actions: clearing
/// the pool stops the dispatcher. Automatic pruning passes `false` so the
/// listener survives and resumes serving once new proxies are added.
pub(crate) async fn store_pool(
    inner: &Arc<DispatchInner>,
    items: Vec<DispatchItem>,
    stop_when_empty: bool,
) -> usize {
    let mut pool: Vec<Upstream> = Vec::new();
    let mut seen = HashSet::new();
    for it in items {
        let Some((key, up)) =
            upstream_from_item(&it.raw, it.url.as_deref(), it.latency, it.country.clone())
        else {
            continue;
        };
        if !seen.insert(key) {
            continue;
        }
        pool.push(up);
        if pool.len() >= 20_000 {
            break;
        }
    }
    pool.sort_by_key(|u| u.latency.unwrap_or(u64::MAX));
    let running = inner.running.load(Ordering::SeqCst);
    if running {
        let port = *inner.port.lock().await;
        pool.retain(|u| !is_self_upstream(u, port));
    }
    let n = pool.len();
    let stop_empty = stop_when_empty && n == 0 && running;
    {
        let urls: HashSet<&str> = pool.iter().map(|u| u.url.as_str()).collect();
        inner
            .fails
            .lock()
            .await
            .retain(|url, _| urls.contains(url.as_str()));
    }
    *inner.pool.lock().await = pool;
    if stop_empty {
        // A listener with zero upstreams only replies 502. Clearing the pool
        // is an explicit user action, so stop the dispatcher with it.
        stop_proxy_inner(inner).await;
    }
    n
}

const POOL_FILE: &str = "pool.json";
const POOL_FILE_MAX: u64 = 16 * 1024 * 1024;

fn pool_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(POOL_FILE))
}

/// Write the pool atomically (temp file + rename). The file holds proxy
/// credentials, so on Unix it is readable by the owner only.
pub(crate) fn write_pool_file(path: &Path, items: &[DispatchPoolItem]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec(items).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    {
        use std::io::Write;
        let mut f = opts.open(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

pub(crate) fn read_pool_file(path: &Path) -> Vec<DispatchItem> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Vec::new();
    };
    if meta.len() > POOL_FILE_MAX {
        return Vec::new();
    }
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Persist the current pool so it survives restarts; scheduled checks would
/// otherwise start from an empty pool after every launch.
pub(crate) async fn save_pool(app: &tauri::AppHandle, inner: &Arc<DispatchInner>) {
    let Some(path) = pool_path(app) else {
        return;
    };
    let _guard = inner.save_lock.lock().await;
    let items = pool_items(inner).await;
    let _ = tauri::async_runtime::spawn_blocking(move || write_pool_file(&path, &items)).await;
}

pub(crate) async fn load_pool(app: &tauri::AppHandle, inner: &Arc<DispatchInner>) {
    let Some(path) = pool_path(app) else {
        return;
    };
    let items = tauri::async_runtime::spawn_blocking(move || read_pool_file(&path))
        .await
        .unwrap_or_default();
    if !items.is_empty() {
        store_pool(inner, items, false).await;
    }
}

#[tauri::command]
pub(crate) async fn set_dispatch_pool(
    app: tauri::AppHandle,
    items: Vec<DispatchItem>,
    state: State<'_, DispatchState>,
) -> Result<usize, String> {
    let n = store_pool(&state.inner, items, true).await;
    save_pool(&app, &state.inner).await;
    Ok(n)
}

async fn start_proxy_locked(
    inner: &Arc<DispatchInner>,
    req_port: u16,
    mode: String,
    listener: String,
) -> Result<String, String> {
    {
        let pool = inner.pool.lock().await;
        if pool.is_empty() {
            return Err("pool is empty".to_string());
        }
    }
    if let Some(h) = inner.handle.lock().await.take() {
        h.abort();
        // Wait until the accept loop (and therefore the listener) is gone
        // before binding; otherwise the old address may still be in use and
        // the requested port silently drifts to +1.
        let _ = h.await;
    }
    abort_connections(inner).await;
    inner.running.store(false, Ordering::SeqCst);
    // fallback: requested port, then +1, +2, ... while busy
    let mut port = req_port;
    let tcp = loop {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => break l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => match port.checked_add(1) {
                Some(p) if p <= req_port.saturating_add(100) => {
                    port = p;
                    continue;
                }
                _ => return Err(format!("no free port near 127.0.0.1:{req_port}")),
            },
            Err(e) => return Err(format!("bind 127.0.0.1:{req_port}: {e}")),
        }
    };
    *inner.port.lock().await = port;
    {
        let mut pool = inner.pool.lock().await;
        pool.retain(|u| !is_self_upstream(u, port));
        if pool.is_empty() {
            return Err("pool contains only the local listener".to_string());
        }
    }
    *inner.mode.lock().await = mode.clone();
    *inner.listener.lock().await = listener.clone();
    *inner.last_error.lock().await = None;
    *inner.current.lock().await = None;
    let listener_desc = match listener.as_str() {
        "http" => "http",
        "socks5" => "socks5",
        _ => "http+socks5",
    }
    .to_string();
    let inner_c = inner.clone();
    let h = tokio::spawn(async move {
        run_server(tcp, inner_c).await;
    });
    *inner.handle.lock().await = Some(h);
    inner.running.store(true, Ordering::SeqCst);
    Ok(format!("127.0.0.1:{port} ({mode}, {listener_desc})"))
}

pub(crate) async fn start_proxy_inner(
    inner: &Arc<DispatchInner>,
    req_port: u16,
    mode: String,
    listener: String,
) -> Result<String, String> {
    let _lifecycle = inner.lifecycle.lock().await;
    start_proxy_locked(inner, req_port, mode, listener).await
}

pub(crate) async fn stop_proxy_inner(inner: &Arc<DispatchInner>) {
    let _lifecycle = inner.lifecycle.lock().await;
    if let Some(h) = inner.handle.lock().await.take() {
        h.abort();
        let _ = h.await;
    }
    // Stop must terminate active tunnels too, not only new connections.
    abort_connections(inner).await;
    inner.running.store(false, Ordering::SeqCst);
    *inner.current.lock().await = None;
    *inner.last_error.lock().await = None;
}

#[tauri::command]
pub(crate) async fn start_local_proxy(
    port: u16,
    mode: String,
    listener: Option<String>,
    state: State<'_, DispatchState>,
) -> Result<String, String> {
    if port == 0 {
        return Err("bad port".to_string());
    }
    let listener = listener.unwrap_or_else(|| "both".to_string());
    let listener = match listener.as_str() {
        "http" | "socks5" => listener,
        _ => "both".to_string(),
    };
    let mode = match mode.as_str() {
        "fastest" | "failover" => mode,
        _ => "round-robin".to_string(),
    };
    start_proxy_inner(&state.inner, port, mode, listener).await
}

#[tauri::command]
pub(crate) async fn stop_local_proxy(state: State<'_, DispatchState>) -> Result<(), String> {
    stop_proxy_inner(&state.inner).await;
    Ok(())
}

#[tauri::command]
pub(crate) async fn get_dispatch_pool(
    state: State<'_, DispatchState>,
) -> Result<Vec<DispatchPoolItem>, String> {
    Ok(pool_items(&state.inner).await)
}

#[tauri::command]
pub(crate) async fn local_proxy_status(
    state: State<'_, DispatchState>,
) -> Result<DispatchStatus, String> {
    let pool_n = state.inner.pool.lock().await.len();
    Ok(DispatchStatus {
        running: state.inner.running.load(Ordering::SeqCst),
        port: *state.inner.port.lock().await,
        mode: state.inner.mode.lock().await.clone(),
        listener: state.inner.listener.lock().await.clone(),
        upstreams: pool_n,
        current: state.inner.current.lock().await.clone(),
        requests: state.inner.requests.load(Ordering::SeqCst),
        errors: state.inner.errors.load(Ordering::SeqCst),
        last_error: state.inner.last_error.lock().await.clone(),
    })
}
