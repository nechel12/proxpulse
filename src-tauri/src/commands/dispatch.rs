use crate::commands::prefs::{remember_dispatch_options, remember_dispatcher, DispatcherPrefs};
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

/// Dispatcher options that can change while it runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DispatchOptions {
    /// Only upstreams from these countries (codes or names); empty = all.
    pub(crate) countries: Vec<String>,
    /// Keep one upstream per destination host for this many minutes; 0 = off.
    pub(crate) sticky_min: u64,
    /// Login local clients must send; empty = no login.
    pub(crate) auth_user: String,
    pub(crate) auth_pass: String,
}

impl DispatchOptions {
    pub(crate) fn sanitized(mut self) -> Self {
        self.countries = self
            .countries
            .iter()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty() && c.len() <= 64)
            .take(50)
            .collect();
        self.sticky_min = self.sticky_min.min(1440);
        self.auth_user = self.auth_user.trim().chars().take(255).collect();
        self.auth_pass = self.auth_pass.chars().take(255).collect();
        if self.auth_user.is_empty() {
            self.auth_pass.clear();
        }
        self
    }

    fn auth(&self) -> Option<(&str, &str)> {
        (!self.auth_user.is_empty()).then_some((self.auth_user.as_str(), self.auth_pass.as_str()))
    }
}

/// A failed upstream is skipped for this long.
pub(crate) const COOLDOWN: Duration = Duration::from_secs(60);
const STICKY_MAX: usize = 10_000;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DispatchStatus {
    running: bool,
    port: u16,
    mode: String,
    listener: String,
    upstreams: usize,
    /// Upstreams skipped for a minute after a failure.
    cooling: usize,
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
    /// Pool id: "main" or a named pool's id.
    pub(crate) id: String,
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
    pub(crate) options: Mutex<DispatchOptions>,
    /// Destination host -> upstream URL and when the binding expires.
    pub(crate) sticky: Mutex<HashMap<String, (String, Instant)>>,
    /// Upstream URL -> end of its cooldown after a failure.
    pub(crate) cooldown: Mutex<HashMap<String, Instant>>,
}

const MAX_CONNECTIONS: usize = 1024;

/// The pool every install has; named pools come on top of it.
pub(crate) const MAIN_POOL: &str = "main";

impl DispatchInner {
    pub(crate) fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
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
            options: Mutex::new(DispatchOptions::default()),
            sticky: Mutex::new(HashMap::new()),
            cooldown: Mutex::new(HashMap::new()),
        }
    }
}

pub(crate) struct DispatchState {
    /// The main pool, reached directly by the tray and the setup code.
    pub(crate) inner: Arc<DispatchInner>,
    /// Named pools by id.
    extra: std::sync::RwLock<HashMap<String, Arc<DispatchInner>>>,
}

impl Default for DispatchState {
    fn default() -> Self {
        Self {
            inner: Arc::new(DispatchInner::new(MAIN_POOL)),
            extra: std::sync::RwLock::new(HashMap::new()),
        }
    }
}

impl DispatchState {
    /// The pool with this id; `None` means the main pool.
    pub(crate) fn pool(&self, id: Option<&str>) -> Result<Arc<DispatchInner>, String> {
        match id.unwrap_or(MAIN_POOL) {
            MAIN_POOL => Ok(self.inner.clone()),
            id => self
                .extra
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .get(id)
                .cloned()
                .ok_or_else(|| "unknown pool".to_string()),
        }
    }

    pub(crate) fn add(&self, id: &str) -> Arc<DispatchInner> {
        let mut extra = self.extra.write().unwrap_or_else(|e| e.into_inner());
        extra
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(DispatchInner::new(id)))
            .clone()
    }

    pub(crate) fn remove(&self, id: &str) -> Option<Arc<DispatchInner>> {
        self.extra
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
    }

    /// Every pool, the main one first.
    pub(crate) fn all(&self) -> Vec<Arc<DispatchInner>> {
        let mut out = vec![self.inner.clone()];
        let extra = self.extra.read().unwrap_or_else(|e| e.into_inner());
        let mut named: Vec<_> = extra.values().cloned().collect();
        named.sort_by(|a, b| a.id.cmp(&b.id));
        out.extend(named);
        out
    }
}

/// Ports of every running dispatcher: an upstream pointing at any of them
/// would loop traffic between our own listeners.
fn listener_ports() -> &'static std::sync::Mutex<HashSet<u16>> {
    static PORTS: std::sync::OnceLock<std::sync::Mutex<HashSet<u16>>> = std::sync::OnceLock::new();
    PORTS.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

pub(crate) fn is_listener_port(port: u16) -> bool {
    listener_ports()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&port)
}

fn set_listener_port(port: u16, active: bool) {
    let mut ports = listener_ports().lock().unwrap_or_else(|e| e.into_inner());
    if active {
        ports.insert(port);
    } else {
        ports.remove(&port);
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
    if up.port != port && !is_listener_port(up.port) {
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

pub(crate) fn country_matches(up: &Upstream, countries: &[String]) -> bool {
    countries.is_empty()
        || up
            .country
            .as_deref()
            .is_some_and(|c| countries.iter().any(|w| w.eq_ignore_ascii_case(c)))
}

/// Upstream order for one request: only the chosen countries, without
/// upstreams in cooldown unless no other is left, and the host's sticky
/// upstream first.
pub(crate) fn plan_upstreams(
    pool: &[Upstream],
    mode: &str,
    rr: &AtomicUsize,
    countries: &[String],
    cooling: &HashSet<String>,
    sticky: Option<&str>,
    limit: usize,
) -> Vec<Upstream> {
    let allowed: Vec<usize> = (0..pool.len())
        .filter(|&i| country_matches(&pool[i], countries))
        .collect();
    let ready: Vec<usize> = allowed
        .iter()
        .copied()
        .filter(|&i| !cooling.contains(&pool[i].url))
        .collect();
    let ready = if ready.is_empty() { allowed } else { ready };
    let mut order: Vec<usize> = pick_order(ready.len(), mode, rr, limit)
        .into_iter()
        .map(|i| ready[i])
        .collect();
    if let Some(url) = sticky {
        if let Some(&i) = ready.iter().find(|&&i| pool[i].url == url) {
            order.retain(|&j| j != i);
            order.insert(0, i);
            order.truncate(limit);
        }
    }
    order.into_iter().map(|i| pool[i].clone()).collect()
}

async fn take_upstreams(st: &Arc<DispatchInner>, host: &str) -> Vec<Upstream> {
    let port = *st.port.lock().await;
    let mode = st.mode.lock().await.clone();
    let countries = st.options.lock().await.countries.clone();
    let now = Instant::now();
    let cooling: HashSet<String> = {
        let mut c = st.cooldown.lock().await;
        c.retain(|_, until| *until > now);
        c.keys().cloned().collect()
    };
    let sticky = {
        let mut s = st.sticky.lock().await;
        s.retain(|_, (_, until)| *until > now);
        s.get(host).map(|(url, _)| url.clone())
    };
    let candidates: Vec<Upstream> = {
        let pool = st.pool.lock().await;
        if pool.is_empty() {
            return vec![];
        }
        plan_upstreams(
            &pool,
            &mode,
            &st.rr,
            &countries,
            &cooling,
            sticky.as_deref(),
            MAX_UPSTREAM_ATTEMPTS + 4,
        )
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

/// Remember the upstream that served `host` when sessions are sticky.
async fn note_success(st: &Arc<DispatchInner>, host: &str, url: &str) {
    let minutes = st.options.lock().await.sticky_min;
    if minutes == 0 {
        return;
    }
    let mut s = st.sticky.lock().await;
    if s.len() >= STICKY_MAX {
        s.clear();
    }
    s.insert(
        host.to_ascii_lowercase(),
        (
            url.to_string(),
            Instant::now() + Duration::from_secs(minutes * 60),
        ),
    );
}

/// Skip an upstream that could not connect for a while.
async fn note_failure(st: &Arc<DispatchInner>, url: &str) {
    st.cooldown
        .lock()
        .await
        .insert(url.to_string(), Instant::now() + COOLDOWN);
}

fn same_secret(a: &[u8], b: &[u8]) -> bool {
    // Compare in constant time for equal lengths.
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether the request head carries the expected `Proxy-Authorization`.
pub(crate) fn http_auth_ok(head: &str, user: &str, pass: &str) -> bool {
    basic_auth_ok(head, "proxy-authorization", user, pass)
}

/// Whether the request head carries the expected Basic login in `header`.
pub(crate) fn basic_auth_ok(head: &str, header: &str, user: &str, pass: &str) -> bool {
    use base64::Engine;
    let expected = format!("{user}:{pass}");
    head.split("\r\n").skip(1).any(|line| {
        let Some((k, v)) = line.split_once(':') else {
            return false;
        };
        if !k.trim().eq_ignore_ascii_case(header) {
            return false;
        }
        let mut parts = v.trim().splitn(2, ' ');
        let scheme = parts.next().unwrap_or("");
        let token = parts.next().unwrap_or("").trim();
        scheme.eq_ignore_ascii_case("basic")
            && base64::engine::general_purpose::STANDARD
                .decode(token)
                .is_ok_and(|d| same_secret(&d, expected.as_bytes()))
    })
}

/// RFC 1929 username/password sub-negotiation body (after the version byte
/// pair has been read): returns user and password.
pub(crate) fn parse_socks_auth(body: &[u8]) -> Option<(&[u8], &[u8])> {
    let ulen = *body.first()? as usize;
    let user = body.get(1..1 + ulen)?;
    let plen = *body.get(1 + ulen)? as usize;
    let pass = body.get(2 + ulen..2 + ulen + plen)?;
    Some((user, pass))
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

    let auth = st
        .options
        .lock()
        .await
        .auth()
        .map(|(u, p)| (u.to_string(), p.to_string()));
    if let Some((user, pass)) = auth {
        if !http_auth_ok(&head, &user, &pass) {
            let _ = client
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"ProxPulse\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            return;
        }
    }

    let order = take_upstreams(&st, &t_host).await;
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
                *st.current.lock().await = Some(strip_credentials(&up.url));
                st.requests.fetch_add(1, Ordering::Relaxed);
                note_success(&st, &t_host, &up.url).await;
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
                note_failure(&st, &up.url).await;
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
    let auth = st
        .options
        .lock()
        .await
        .auth()
        .map(|(u, p)| (u.to_string(), p.to_string()));
    match &auth {
        Some((user, pass)) => {
            // Username/password (RFC 1929) is the only accepted method.
            if !methods.contains(&0x02) {
                let _ = write_all_t(&mut client, &[0x05, 0xFF], d).await;
                return;
            }
            if write_all_t(&mut client, &[0x05, 0x02], d).await.is_err() {
                return;
            }
            let mut ver = [0u8; 1];
            if read_exact_t(&mut client, &mut ver, d).await.is_err() || ver[0] != 0x01 {
                return;
            }
            let mut body = vec![0u8; 1];
            if read_exact_t(&mut client, &mut body, d).await.is_err() {
                return;
            }
            let mut user_and_len = vec![0u8; body[0] as usize + 1];
            if read_exact_t(&mut client, &mut user_and_len, d)
                .await
                .is_err()
            {
                return;
            }
            body.extend_from_slice(&user_and_len);
            let mut pass_buf = vec![0u8; *body.last().unwrap_or(&0) as usize];
            if read_exact_t(&mut client, &mut pass_buf, d).await.is_err() {
                return;
            }
            body.extend_from_slice(&pass_buf);
            let ok = parse_socks_auth(&body).is_some_and(|(u, p)| {
                same_secret(u, user.as_bytes()) & same_secret(p, pass.as_bytes())
            });
            if write_all_t(&mut client, &[0x01, if ok { 0x00 } else { 0x01 }], d)
                .await
                .is_err()
                || !ok
            {
                return;
            }
        }
        None => {
            if !methods.contains(&0x00) {
                let _ = write_all_t(&mut client, &[0x05, 0xFF], d).await;
                return;
            }
            if write_all_t(&mut client, &[0x05, 0x00], d).await.is_err() {
                return;
            }
        }
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

    let order = take_upstreams(&st, &t_host).await;
    if order.is_empty() {
        let _ = write_all_t(&mut client, &[0x05, 0x02, 0, 1, 0, 0, 0, 0, 0, 0], d).await;
        return;
    }
    let mut last_err = String::new();
    for up in &order {
        match connect_via(up, &t_host, t_port, Duration::from_secs(8)).await {
            Ok(mut u) => {
                *st.current.lock().await = Some(strip_credentials(&up.url));
                st.requests.fetch_add(1, Ordering::Relaxed);
                note_success(&st, &t_host, &up.url).await;
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
                note_failure(&st, &up.url).await;
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

/// `pool.json` for the main pool, `pool-<id>.json` for named pools.
pub(crate) fn pool_file_name(id: &str) -> String {
    if id == MAIN_POOL {
        POOL_FILE.to_string()
    } else {
        format!("pool-{id}.json")
    }
}

fn pool_path(app: &tauri::AppHandle, id: &str) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join(pool_file_name(id)))
}

/// Remove a deleted pool's file once the pool left the registry: a save in
/// progress finishes first, and later saves see that the pool is gone.
pub(crate) async fn delete_pool_file(app: &tauri::AppHandle, inner: &DispatchInner) {
    let _guard = inner.save_lock.lock().await;
    if let Some(path) = pool_path(app, &inner.id) {
        let _ = std::fs::remove_file(pool_backup_path(&path));
        let _ = std::fs::remove_file(path);
    }
}

/// `pool.json.bak` next to `pool.json`: the pool as it was before the last
/// scheduled check removed entries from it.
pub(crate) fn pool_backup_path(pool_file: &Path) -> PathBuf {
    pool_file.with_extension("json.bak")
}

/// Keep one copy of the pool before a scheduled check removes entries, so a
/// wrongly emptied pool, credentials included, can be put back by hand.
pub(crate) async fn backup_pool(app: &tauri::AppHandle, inner: &Arc<DispatchInner>) {
    let Some(path) = pool_path(app, &inner.id) else {
        return;
    };
    let _guard = inner.save_lock.lock().await;
    // As in `save_pool`: a pool deleted meanwhile must not get a file back.
    if app.state::<DispatchState>().pool(Some(&inner.id)).is_err() {
        return;
    }
    let items = pool_items(inner).await;
    let backup = pool_backup_path(&path);
    let _ = tauri::async_runtime::spawn_blocking(move || write_pool_file(&backup, &items)).await;
}

/// Write the pool atomically. The file holds proxy credentials.
pub(crate) fn write_pool_file(path: &Path, items: &[DispatchPoolItem]) -> std::io::Result<()> {
    let json = serde_json::to_vec(items).map_err(std::io::Error::other)?;
    write_private_file(path, &json)
}

/// Write a file atomically (temp file + rename); on Unix it is readable by
/// the owner only.
pub(crate) fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
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
        f.write_all(bytes)?;
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
    let Some(path) = pool_path(app, &inner.id) else {
        return;
    };
    let _guard = inner.save_lock.lock().await;
    // A pool deleted while its scheduled check ran must not get its file
    // (with credentials) back.
    if app.state::<DispatchState>().pool(Some(&inner.id)).is_err() {
        return;
    }
    let items = pool_items(inner).await;
    let _ = tauri::async_runtime::spawn_blocking(move || write_pool_file(&path, &items)).await;
}

pub(crate) async fn load_pool(app: &tauri::AppHandle, inner: &Arc<DispatchInner>) {
    let Some(path) = pool_path(app, &inner.id) else {
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
    pool: Option<String>,
    items: Vec<DispatchItem>,
    state: State<'_, DispatchState>,
) -> Result<usize, String> {
    let inner = state.pool(pool.as_deref())?;
    let n = store_pool(&inner, items, true).await;
    save_pool(&app, &inner).await;
    if n == 0 {
        // Clear pool: its backup holds the same logins and passwords.
        if let Some(path) = pool_path(&app, &inner.id) {
            let _guard = inner.save_lock.lock().await;
            let _ = std::fs::remove_file(pool_backup_path(&path));
        }
    }
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
        set_listener_port(*inner.port.lock().await, false);
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
    set_listener_port(port, true);
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
        set_listener_port(*inner.port.lock().await, false);
    }
    // Stop must terminate active tunnels too, not only new connections.
    abort_connections(inner).await;
    inner.running.store(false, Ordering::SeqCst);
    *inner.current.lock().await = None;
    *inner.last_error.lock().await = None;
}

pub(crate) fn normalize_mode(mode: &str) -> String {
    match mode {
        "fastest" | "failover" => mode.to_string(),
        _ => "round-robin".to_string(),
    }
}

pub(crate) fn normalize_listener(listener: &str) -> String {
    match listener {
        "http" | "socks5" => listener.to_string(),
        _ => "both".to_string(),
    }
}

#[tauri::command]
pub(crate) async fn start_local_proxy(
    app: tauri::AppHandle,
    pool: Option<String>,
    port: u16,
    mode: String,
    listener: Option<String>,
    state: State<'_, DispatchState>,
) -> Result<String, String> {
    if port == 0 {
        return Err("bad port".to_string());
    }
    let inner = state.pool(pool.as_deref())?;
    let listener = normalize_listener(listener.as_deref().unwrap_or("both"));
    let mode = normalize_mode(&mode);
    let addr = start_proxy_inner(&inner, port, mode.clone(), listener.clone()).await?;
    // The dispatcher started at launch reuses the last configuration.
    let options = inner.options.lock().await.clone();
    remember_dispatcher(
        &app,
        &inner.id,
        DispatcherPrefs {
            port,
            mode,
            listener,
            options,
            started: true,
        },
    )
    .await;
    Ok(addr)
}

/// Apply dispatcher options at once, running or not, and keep them for the
/// next start.
#[tauri::command]
pub(crate) async fn set_dispatch_options(
    app: tauri::AppHandle,
    pool: Option<String>,
    options: DispatchOptions,
    state: State<'_, DispatchState>,
) -> Result<DispatchOptions, String> {
    let inner = state.pool(pool.as_deref())?;
    let options = options.sanitized();
    *inner.options.lock().await = options.clone();
    // Bindings made under the old countries may point at excluded upstreams.
    inner.sticky.lock().await.clear();
    remember_dispatch_options(&app, &inner.id, options.clone()).await;
    Ok(options)
}

#[tauri::command]
pub(crate) async fn stop_local_proxy(
    pool: Option<String>,
    state: State<'_, DispatchState>,
) -> Result<(), String> {
    stop_proxy_inner(&state.pool(pool.as_deref())?).await;
    Ok(())
}

#[tauri::command]
pub(crate) async fn get_dispatch_pool(
    pool: Option<String>,
    state: State<'_, DispatchState>,
) -> Result<Vec<DispatchPoolItem>, String> {
    Ok(pool_items(&state.pool(pool.as_deref())?).await)
}

pub(crate) async fn dispatch_status(inner: &Arc<DispatchInner>) -> DispatchStatus {
    let pool_n = inner.pool.lock().await.len();
    DispatchStatus {
        running: inner.running.load(Ordering::SeqCst),
        port: *inner.port.lock().await,
        mode: inner.mode.lock().await.clone(),
        listener: inner.listener.lock().await.clone(),
        upstreams: pool_n,
        cooling: {
            let now = Instant::now();
            inner
                .cooldown
                .lock()
                .await
                .values()
                .filter(|u| **u > now)
                .count()
        },
        current: inner.current.lock().await.clone(),
        requests: inner.requests.load(Ordering::SeqCst),
        errors: inner.errors.load(Ordering::SeqCst),
        last_error: inner.last_error.lock().await.clone(),
    }
}

#[tauri::command]
pub(crate) async fn local_proxy_status(
    pool: Option<String>,
    state: State<'_, DispatchState>,
) -> Result<DispatchStatus, String> {
    Ok(dispatch_status(&state.pool(pool.as_deref())?).await)
}
