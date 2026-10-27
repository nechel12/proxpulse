use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::io::{Error, ErrorKind};
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex as StdMutex, OnceLock,
};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{
    ipc::Channel,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State,
};
use tauri_plugin_dialog::DialogExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Semaphore};

mod proxy;
use proxy::*;

// ================= results =================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyResult {
    pub proxy: String,
    /// Normalized URL actually used for the check (keeps the real scheme).
    pub url: String,
    pub proto: String,
    pub alive: bool,
    pub latency_ms: Option<u64>,
    pub jitter_ms: Option<u64>,
    pub success_rate: f32,
    pub attempts: u32,
    pub ip: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub city: Option<String>,
    pub isp: Option<String>,
    pub org: Option<String>,
    pub asn: Option<String>,
    pub ip_type: Option<String>,
    pub anonymity: Option<String>,
    pub tamper: Option<String>,
    pub tls: Option<String>,
    pub tls_info: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IpApi {
    status: Option<String>,
    query: Option<String>,
    country: Option<String>,
    #[serde(rename = "countryCode")]
    country_code: Option<String>,
    city: Option<String>,
    isp: Option<String>,
    org: Option<String>,
    #[serde(rename = "as")]
    asn: Option<String>,
    mobile: Option<bool>,
    proxy: Option<bool>,
    hosting: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct HttpbinIp {
    origin: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HttpbinHeaders {
    headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Default)]
struct JudgeGeo {
    country: Option<String>,
    country_code: Option<String>,
    city: Option<String>,
    asn: Option<u64>,
    aso: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct JudgeBody {
    ip: Option<String>,
    anonymity: Option<String>,
    ip_type: Option<String>,
    geo: Option<JudgeGeo>,
    content_sha256: Option<String>,
}

fn try_parse_judge_body(res: &mut ProxyResult, body: &str) -> Option<String> {
    let jb: JudgeBody = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return None,
    };
    if let Some(ip) = jb.ip {
        let t = ip.trim().to_string();
        if !t.is_empty() {
            res.ip = Some(t);
        }
    }
    res.anonymity = jb.anonymity;
    res.ip_type = jb.ip_type;
    if let Some(g) = jb.geo {
        res.country = g.country;
        res.country_code = g.country_code;
        res.city = g.city;
        res.asn = g.asn.map(|n| n.to_string());
        res.org = g.aso;
    }
    jb.content_sha256
}

#[derive(Debug, Clone, Deserialize)]
struct GeoCacheEntry {
    proxy: String,
    ip: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
    city: Option<String>,
    isp: Option<String>,
    org: Option<String>,
    asn: Option<String>,
    ip_type: Option<String>,
}

/// Direct judge endpoint used for the content hash baseline. For a custom
/// path (e.g. `https://host/svc`) the URL is used as-is: the same endpoint
/// is what the checked proxy requests.
fn judge_direct_url(test_url: &str) -> Option<String> {
    let t = test_url.trim_end_matches('/');
    if t.is_empty() {
        return None;
    }
    if t.ends_with("/judge") {
        return Some(t.to_string());
    }
    if let Some(base) = t.strip_suffix("/generate_204") {
        if !base.is_empty() {
            return Some(format!("{base}/judge"));
        }
    }
    Some(t.to_string())
}

async fn fetch_direct_judge_sha(judge_url: &str, timeout: Duration) -> Option<String> {
    let client = build_client(None, timeout, true).ok()?;
    let r = client.get(judge_url).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let body = read_body_capped(r, BODY_LIMIT_PAGE).await.ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    v.get("content_sha256")?.as_str().map(|s| s.to_string())
}

struct Baseline {
    direct_ip: Option<String>,
    example_len: Option<usize>,
    example_hash: Option<u64>,
    example_cert_fp: Option<String>,
    judge_content_sha: Option<String>,
}

#[derive(Default)]
struct BaselineCache {
    direct_ip: Option<(String, Instant)>,
    example: Option<(usize, u64, Instant)>,
    cert: Option<(String, Instant)>,
}

static BASELINE_CACHE: std::sync::OnceLock<Mutex<BaselineCache>> = std::sync::OnceLock::new();

const BASELINE_IP_TTL: Duration = Duration::from_secs(300);
const BASELINE_PAGE_TTL: Duration = Duration::from_secs(1800);

/// The free ip-api tier is heavily rate-limited. Cap concurrent geo lookups
/// instead of firing one request per checked proxy at 500-way concurrency.
const GEO_CONCURRENCY: usize = 8;
const GEO_COOLDOWN: Duration = Duration::from_secs(60);
static GEO_SEM: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();
static GEO_COOLDOWN_UNTIL: Mutex<Option<Instant>> = Mutex::const_new(None);

/// Deep checks open one extra connection per stage. Without a cap, a
/// 500-way check can spawn ~2500 sockets at once. These limits are per stage
/// and skip (rather than queue) when saturated, so the per-proxy budget is
/// not consumed by waiting.
const DEEP_CONCURRENCY: usize = 64;
static ANON_SEM: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();
static TAMPER_SEM: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();
static TLS_SEM: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();

fn geo_semaphore() -> &'static Semaphore {
    GEO_SEM.get_or_init(|| Semaphore::new(GEO_CONCURRENCY))
}

fn deep_semaphore(slot: &'static std::sync::OnceLock<Semaphore>) -> &'static Semaphore {
    slot.get_or_init(|| Semaphore::new(DEEP_CONCURRENCY))
}

fn baseline_cache() -> &'static Mutex<BaselineCache> {
    BASELINE_CACHE.get_or_init(|| Mutex::const_new(BaselineCache::default()))
}

fn cache_fresh(ts: Instant, ttl: Duration) -> bool {
    ts.elapsed() < ttl
}

fn body_hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[derive(Debug, Default)]
struct CheckState {
    cancel: Arc<AtomicBool>,
    /// Prevents a manual check and an auto check (or two manual checks)
    /// from running at the same time.
    busy: Arc<AtomicBool>,
}

struct BusyGuard(Arc<AtomicBool>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

const CHECK_BUSY: &str = "another check is running";

#[derive(Clone)]
struct CheckFlags {
    geo: bool,
    anon: bool,
    tamper: bool,
    tls: bool,
    precheck: bool,
    precheck_timeout: Duration,
    judge: bool,
    /// Optional ip-api Pro key. Used only for the separate geo lookup; the
    /// main test URL is built by the frontend. The key is visible to the
    /// checked proxy because the request originates from it; see README.
    ipapi_key: Option<Arc<str>>,
}

// ================= http helpers =================

/// Truncate to at most `max` bytes without splitting a UTF-8 code point.
/// `String::truncate` panics when the byte index is not a char boundary.
fn truncate_utf8(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

fn short_error(e: &str) -> String {
    // reqwest format: "error sending request for url (URL): cause".
    // The URL (often with ?direct_ip=...) only hides the real cause — drop it.
    let mut s = e.replace('\n', " ");
    if s.starts_with("error sending request for url (") {
        if let Some(pos) = s.find("): ") {
            let after = s[pos + 3..].trim();
            if !after.is_empty() {
                s = after.to_string();
            }
        }
    }
    let low = s.to_lowercase();
    if low.contains("timed out")
        || low.contains("timeout")
        || low.contains("deadline")
        || low.contains("elapsed")
    {
        return "timeout".to_string();
    }
    if low.contains("proxy authentication") || low.contains("407") {
        return "proxy auth required".to_string();
    }
    if low.contains("connection refused") {
        return "connection refused".to_string();
    }
    if low.contains("failed to lookup")
        || low.contains("dns")
        || low.contains("could not resolve")
        || low.contains("no route to host")
        || low.contains("network unreachable")
    {
        return "dns error".to_string();
    }
    if low.contains("connection closed")
        || low.contains("connection reset")
        || low.contains("broken pipe")
        || low.contains("unexpected eof")
        || low.contains("connection closed before message completed")
    {
        return "connection reset".to_string();
    }
    if low.contains("rate limited") || low.contains("too many requests") {
        return "rate limited".to_string();
    }
    if low.contains("socks") {
        return "socks error".to_string();
    }
    if low.contains("proxy") {
        return "proxy error".to_string();
    }
    if low.contains("certificate")
        || low.contains("tls")
        || low.contains("ssl")
        || low.contains("handshake failure")
    {
        return "tls error".to_string();
    }
    if low.contains("redirect") {
        return "redirect error".to_string();
    }
    if low.contains("incomplete message") || low.contains("incompletemessage") {
        return "incomplete response".to_string();
    }
    truncate_utf8(&mut s, 160);
    s.trim().to_string()
}

const CLIENT_CACHE_MAX: usize = 256;

struct ClientCache {
    map: HashMap<(String, u64, bool), reqwest::Client>,
    order: VecDeque<(String, u64, bool)>,
}

fn client_cache() -> &'static StdMutex<ClientCache> {
    static CACHE: OnceLock<StdMutex<ClientCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        StdMutex::new(ClientCache {
            map: HashMap::new(),
            order: VecDeque::new(),
        })
    })
}

fn build_client(
    proxy_url: Option<&str>,
    timeout: Duration,
    no_proxy: bool,
) -> Result<reqwest::Client, String> {
    let key = (
        proxy_url.unwrap_or_default().to_string(),
        timeout.as_millis() as u64,
        no_proxy,
    );
    {
        let cache = client_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(client) = cache.map.get(&key) {
            return Ok(client.clone());
        }
    }
    let mut b = reqwest::Client::builder()
        .timeout(timeout)
        // Reuse the process-wide rustls config instead of rebuilding the
        // webpki root store for every proxy client.
        .use_preconfigured_tls((*tls_client_config()).clone())
        .user_agent(concat!(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) ProxPulse/",
            env!("CARGO_PKG_VERSION")
        ));
    if no_proxy {
        b = b.no_proxy();
    }
    if let Some(u) = proxy_url {
        let p = reqwest::Proxy::all(u).map_err(|e| short_error(&e.to_string()))?;
        b = b.proxy(p);
    }
    let client = b.build().map_err(|e| short_error(&e.to_string()))?;
    let mut cache = client_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = cache.map.get(&key) {
        return Ok(existing.clone());
    }
    if cache.order.len() >= CLIENT_CACHE_MAX {
        if let Some(old) = cache.order.pop_front() {
            cache.map.remove(&old);
        }
    }
    cache.order.push_back(key.clone());
    cache.map.insert(key, client.clone());
    Ok(client)
}

const BODY_LIMIT_SMALL: usize = 256 * 1024;
const BODY_LIMIT_PAGE: usize = 1024 * 1024;
const BODY_LIMIT_IMPORT: usize = 2 * 1024 * 1024;
const BODY_LIMIT_WEBHOOK: usize = 8 * 1024;

/// Read a response body with a hard limit. Untrusted proxies can otherwise
/// return arbitrarily large bodies and exhaust memory.
async fn read_body_capped(mut r: reqwest::Response, max: usize) -> Result<String, String> {
    if let Some(len) = r.content_length() {
        if len > max as u64 {
            return Err("response too big".to_string());
        }
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(|e| short_error(&e.to_string()))? {
        if body.len().saturating_add(chunk.len()) > max {
            return Err("response too big".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| "not text".to_string())
}

fn looks_like_ip(s: &str) -> bool {
    s.trim().parse::<std::net::IpAddr>().is_ok()
}

fn derive_ip_type(g: &IpApi) -> Option<String> {
    if g.mobile == Some(true) {
        return Some("mobile".to_string());
    }
    if g.hosting == Some(true) {
        return Some("datacenter".to_string());
    }
    if g.proxy == Some(true) {
        return Some("proxy/vpn".to_string());
    }
    if g.country.is_some() || g.isp.is_some() {
        return Some("residential/isp".to_string());
    }
    None
}

fn apply_ipapi(res: &mut ProxyResult, g: &IpApi) {
    if let Some(q) = &g.query {
        if !q.is_empty() {
            res.ip = Some(q.clone());
        }
    }
    res.country.clone_from(&g.country);
    res.country_code.clone_from(&g.country_code);
    res.city.clone_from(&g.city);
    res.isp.clone_from(&g.isp);
    res.org.clone_from(&g.org);
    res.asn.clone_from(&g.asn);
    if res.ip_type.is_none() {
        res.ip_type = derive_ip_type(g);
    }
}

/// Fill geo from the frontend cache. The frontend filters entries by a TTL;
/// when the test URL does not reveal the exit IP, the cached IP is used.
/// Reusing a cached IP here also lets the frontend refresh its timestamp.
fn apply_cached_geo(res: &mut ProxyResult, entry: &GeoCacheEntry) -> bool {
    let use_cached = match (&res.ip, &entry.ip) {
        (Some(a), Some(b)) => a == b,
        (None, _) => true,
        _ => false,
    };
    if !use_cached {
        return false;
    }
    if res.ip.is_none() {
        res.ip = entry.ip.clone();
    }
    if res.country.is_none() {
        res.country = entry.country.clone();
    }
    if res.country_code.is_none() {
        res.country_code = entry.country_code.clone();
    }
    if res.city.is_none() {
        res.city = entry.city.clone();
    }
    if res.isp.is_none() {
        res.isp = entry.isp.clone();
    }
    if res.org.is_none() {
        res.org = entry.org.clone();
    }
    if res.asn.is_none() {
        res.asn = entry.asn.clone();
    }
    if res.ip_type.is_none() {
        res.ip_type = entry.ip_type.clone();
    }
    true
}

fn try_parse_test_body(res: &mut ProxyResult, test_url: &str, body: &str) {
    let low = test_url.to_lowercase();
    if low.contains("ip-api.com") {
        if let Ok(g) = serde_json::from_str::<IpApi>(body) {
            apply_ipapi(res, &g);
        }
        return;
    }
    if low.contains("httpbin.org/ip") {
        if let Ok(j) = serde_json::from_str::<HttpbinIp>(body) {
            if let Some(o) = j.origin {
                let first = o.split(',').next().unwrap_or("").trim().to_string();
                if !first.is_empty() {
                    res.ip = Some(first);
                }
            }
        }
        return;
    }
    if low.contains("ipify.org") {
        let t = body.trim().to_string();
        if looks_like_ip(&t) {
            res.ip = Some(t);
        }
        return;
    }
    let t = body.trim();
    if !t.is_empty() && t.len() <= 128 && !t.contains('<') && looks_like_ip(t) {
        res.ip = Some(t.to_string());
    }
}

const GEO_FIELDS: &str =
    "status,message,query,country,countryCode,city,isp,org,as,mobile,proxy,hosting";

/// The free ip-api tier is HTTP-only. Pro uses the same schema over HTTPS
/// and is authenticated by a key GET parameter (see members.ip-api.com).
fn geo_url(ipapi_key: Option<&str>) -> String {
    match ipapi_key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(key) => {
            let mut u = reqwest::Url::parse("https://pro.ip-api.com/json/")
                .expect("static ip-api Pro URL is valid");
            u.query_pairs_mut()
                .append_pair("key", key)
                .append_pair("fields", GEO_FIELDS);
            u.to_string()
        }
        None => format!("http://ip-api.com/json/?fields={GEO_FIELDS}"),
    }
}

async fn fetch_geo(client: &reqwest::Client, ipapi_key: Option<&str>) -> Option<IpApi> {
    {
        let until = *GEO_COOLDOWN_UNTIL.lock().await;
        if until.is_some_and(|t| Instant::now() < t) {
            return None;
        }
    }
    let url = geo_url(ipapi_key);
    let r = client.get(url).send().await.ok()?;
    let status = r.status();
    if status.as_u16() == 429 || status.as_u16() == 403 {
        *GEO_COOLDOWN_UNTIL.lock().await = Some(Instant::now() + GEO_COOLDOWN);
        return None;
    }
    if !status.is_success() {
        return None;
    }
    let body = read_body_capped(r, BODY_LIMIT_SMALL).await.ok()?;
    match serde_json::from_str::<IpApi>(&body) {
        Ok(g) if g.status.as_deref() != Some("fail") => Some(g),
        _ => None,
    }
}

async fn detect_anonymity(client: &reqwest::Client, direct_ip: Option<&str>) -> Option<String> {
    // Without a direct-IP baseline "elite" would be a guess, not a result.
    // Return before spending a request through the proxy.
    let d = direct_ip.filter(|d| !d.is_empty())?;
    let r = client
        .get("https://httpbin.org/headers")
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    let body = read_body_capped(r, BODY_LIMIT_SMALL).await.ok()?;
    let parsed = serde_json::from_str::<HttpbinHeaders>(&body).ok()?;
    let headers = parsed.headers.unwrap_or_default();
    let mut lower: HashMap<String, String> = HashMap::new();
    for (k, v) in headers {
        lower.insert(k.to_lowercase(), v);
    }
    if lower.values().any(|v| v.contains(d)) {
        return Some("transparent".to_string());
    }
    if let Some(xff) = lower.get("x-forwarded-for") {
        if xff.contains(',') {
            return Some("transparent".to_string());
        }
    }
    let telltales = [
        "via",
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-by",
        "forwarded-for",
        "proxy-connection",
        "proxy-authorization",
        "x-proxy-id",
        "proxy-agent",
    ];
    if telltales.iter().any(|k| lower.contains_key(*k)) {
        return Some("anonymous".to_string());
    }
    Some("elite".to_string())
}

async fn detect_tamper(
    client: &reqwest::Client,
    base_len: Option<usize>,
    base_hash: Option<u64>,
) -> Option<String> {
    let (bl, bh) = match (base_len, base_hash) {
        (Some(a), Some(b)) => (a, b),
        _ => return None,
    };
    let r = client.get("https://example.com").send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let body = read_body_capped(r, BODY_LIMIT_PAGE).await.ok()?;
    if body.len() == bl && body_hash(&body) == bh {
        return Some("ok".to_string());
    }
    Some("modified".to_string())
}

// ================= tcp via upstream (shared by TLS check + dispatcher) =================

#[derive(Debug, Clone)]
struct Upstream {
    raw: String,
    /// Normalized URL actually used for checks (keeps the real scheme).
    url: String,
    scheme: String,
    host: String,
    port: u16,
    user: Option<String>,
    pass: Option<String>,
    latency: Option<u64>,
    country: Option<String>,
}

type UpstreamParts = (String, String, u16, Option<String>, Option<String>);

fn parse_upstream_url(url: &str) -> Option<UpstreamParts> {
    let (scheme, rest) = url.split_once("://")?;
    let after = rest.rsplit('@').next()?;
    let userinfo = rest.rsplit_once('@').map(|(u, _)| u.to_string());
    let (user, pass) = match userinfo {
        Some(u) => match u.split_once(':') {
            Some((a, b)) => (Some(a.to_string()), Some(b.to_string())),
            None => (Some(u), None),
        },
        None => (None, None),
    };
    let hp = after.split('/').next()?;
    let (host, port) = if let Some(stripped) = hp.strip_prefix('[') {
        let end = stripped.find(']')?;
        let host = stripped[..end].to_string();
        let port: u16 = stripped[end + 1..].strip_prefix(':')?.parse().ok()?;
        (host, port)
    } else {
        let (h, p) = hp.rsplit_once(':')?;
        (h.to_string(), p.parse().ok()?)
    };
    Some((canonical_scheme(scheme), host, port, user, pass))
}

fn host_for_connect(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

enum ProxyStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for ProxyStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ProxyStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ProxyStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ProxyStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_flush(cx),
            ProxyStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ProxyStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

async fn read_exact_t<S: AsyncRead + Unpin>(
    s: &mut S,
    buf: &mut [u8],
    d: Duration,
) -> std::io::Result<()> {
    match tokio::time::timeout(d, s.read_exact(buf)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "read timeout")),
    }
}

async fn write_all_t<S: AsyncWrite + Unpin>(
    s: &mut S,
    buf: &[u8],
    d: Duration,
) -> std::io::Result<()> {
    match tokio::time::timeout(d, s.write_all(buf)).await {
        Ok(r) => r,
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "write timeout")),
    }
}

async fn socks5_connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    t_host: &str,
    t_port: u16,
    user: Option<String>,
    pass: Option<String>,
    d: Duration,
) -> std::io::Result<S> {
    if user.is_some() {
        write_all_t(&mut s, &[0x05, 0x01, 0x02], d).await?;
    } else {
        write_all_t(&mut s, &[0x05, 0x01, 0x00], d).await?;
    }
    let mut resp = [0u8; 2];
    read_exact_t(&mut s, &mut resp, d).await?;
    if resp[0] != 0x05 {
        return Err(Error::new(ErrorKind::InvalidData, "bad socks5 version"));
    }
    if resp[1] == 0x02 {
        let (u, p) = (user.unwrap_or_default(), pass.unwrap_or_default());
        let ub = u.as_bytes();
        let pb = p.as_bytes();
        if ub.len() > 255 || pb.len() > 255 {
            return Err(Error::new(ErrorKind::InvalidInput, "socks5 creds too long"));
        }
        let mut req = vec![0x01, ub.len() as u8];
        req.extend_from_slice(ub);
        req.push(pb.len() as u8);
        req.extend_from_slice(pb);
        write_all_t(&mut s, &req, d).await?;
        read_exact_t(&mut s, &mut resp, d).await?;
        if resp[1] != 0x00 {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "socks5 auth failed",
            ));
        }
    } else if resp[1] != 0x00 {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "socks5 auth required",
        ));
    }
    let mut req = vec![0x05, 0x01, 0x00];
    if let Ok(ip) = t_host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(v) => {
                req.push(0x01);
                req.extend_from_slice(&v.octets());
            }
            IpAddr::V6(v) => {
                req.push(0x04);
                req.extend_from_slice(&v.octets());
            }
        }
    } else {
        let hb = t_host.as_bytes();
        if hb.len() > 255 {
            return Err(Error::new(ErrorKind::InvalidInput, "hostname too long"));
        }
        req.push(0x03);
        req.push(hb.len() as u8);
        req.extend_from_slice(hb);
    }
    req.extend_from_slice(&t_port.to_be_bytes());
    write_all_t(&mut s, &req, d).await?;
    let mut hdr = [0u8; 4];
    read_exact_t(&mut s, &mut hdr, d).await?;
    if hdr[0] != 0x05 || hdr[1] != 0x00 {
        return Err(Error::new(
            ErrorKind::ConnectionRefused,
            format!("socks5 connect failed: {:02x}", hdr[1]),
        ));
    }
    match hdr[3] {
        0x01 => {
            let mut b = [0u8; 6];
            read_exact_t(&mut s, &mut b, d).await?;
        }
        0x04 => {
            let mut b = [0u8; 18];
            read_exact_t(&mut s, &mut b, d).await?;
        }
        0x03 => {
            let mut l = [0u8; 1];
            read_exact_t(&mut s, &mut l, d).await?;
            let mut b = vec![0u8; l[0] as usize + 2];
            read_exact_t(&mut s, &mut b, d).await?;
        }
        _ => return Err(Error::new(ErrorKind::InvalidData, "bad socks5 atyp")),
    }
    Ok(s)
}

async fn socks4_connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    t_host: &str,
    t_port: u16,
    user: Option<String>,
    allow_ipv6_domain: bool,
    d: Duration,
) -> std::io::Result<S> {
    let req = build_socks4_request(t_host, t_port, user.as_deref(), allow_ipv6_domain)?;
    write_all_t(&mut s, &req, d).await?;
    let mut resp = [0u8; 8];
    read_exact_t(&mut s, &mut resp, d).await?;
    if resp[1] != 90 {
        return Err(Error::new(
            ErrorKind::ConnectionRefused,
            format!("socks4 connect failed: {}", resp[1]),
        ));
    }
    Ok(s)
}

fn build_socks4_request(
    t_host: &str,
    t_port: u16,
    user: Option<&str>,
    allow_ipv6_domain: bool,
) -> std::io::Result<Vec<u8>> {
    let mut req = vec![0x04, 0x01];
    req.extend_from_slice(&t_port.to_be_bytes());
    let domain = match t_host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v)) => {
            req.extend_from_slice(&v.octets());
            None
        }
        Ok(IpAddr::V6(_)) if allow_ipv6_domain => {
            req.extend_from_slice(&[0, 0, 0, 1]);
            Some(t_host)
        }
        Ok(IpAddr::V6(_)) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "socks4 does not support IPv6; use socks5 or socks4a",
            ))
        }
        Err(_) => {
            req.extend_from_slice(&[0, 0, 0, 1]);
            Some(t_host)
        }
    };
    let uid = user.unwrap_or("");
    if uid.len() > 255 {
        return Err(Error::new(ErrorKind::InvalidInput, "socks4 user too long"));
    }
    req.extend_from_slice(uid.as_bytes());
    req.push(0x00);
    if let Some(dm) = domain {
        if dm.is_empty() || dm.len() > 255 {
            return Err(Error::new(ErrorKind::InvalidInput, "socks4 host too long"));
        }
        req.extend_from_slice(dm.as_bytes());
        req.push(0x00);
    }
    Ok(req)
}

fn basic_proxy_auth(user: Option<&str>, pass: Option<&str>) -> Option<String> {
    user.map(|u| {
        base64::engine::general_purpose::STANDARD.encode(format!(
            "{}:{}",
            u,
            pass.unwrap_or_default()
        ))
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

/// Remove one header from the header block, preserving the body. The client's
/// local-proxy credentials must never be forwarded to an upstream proxy or
/// to the origin.
fn strip_header(buf: &[u8], hdr_end: usize, name: &str) -> Vec<u8> {
    let end = hdr_end.min(buf.len());
    let head = &buf[..end];
    let mut out = Vec::with_capacity(buf.len());
    let mut start = 0;
    for (i, &b) in head.iter().enumerate() {
        if b != b'\n' {
            continue;
        }
        let line = &head[start..=i];
        let is_target = String::from_utf8_lossy(line)
            .split_once(':')
            .map(|(k, _)| k.trim().eq_ignore_ascii_case(name))
            .unwrap_or(false);
        if !is_target {
            out.extend_from_slice(line);
        }
        start = i + 1;
    }
    out.extend_from_slice(&head[start..]);
    out.extend_from_slice(&buf[end..]);
    out
}

/// For plain HTTP through an HTTP(S) proxy, the request must be sent in
/// absolute-form with Proxy-Authorization. Unlike CONNECT, this works with
/// proxies that only allow CONNECT to port 443.
fn prepare_forward_request(buf: &[u8], hdr_end: usize, up: &Upstream) -> Vec<u8> {
    // Drop any Proxy-Authorization sent by the client: it is meant for the
    // local listener, not for the upstream. Then add this upstream's own
    // credentials, if any.
    let stripped = strip_header(buf, hdr_end, "proxy-authorization");
    let Some(auth) = basic_proxy_auth(up.user.as_deref(), up.pass.as_deref()) else {
        return stripped;
    };
    let end = find_header_end(&stripped).unwrap_or(stripped.len());
    let head_end = end.saturating_sub(4);
    let mut out = Vec::with_capacity(stripped.len() + auth.len() + 48);
    out.extend_from_slice(&stripped[..head_end]);
    out.extend_from_slice(b"\r\nProxy-Authorization: Basic ");
    out.extend_from_slice(auth.as_bytes());
    out.extend_from_slice(b"\r\n\r\n");
    out.extend_from_slice(&stripped[end..]);
    out
}

async fn http_connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    t_host: &str,
    t_port: u16,
    user: Option<String>,
    pass: Option<String>,
    d: Duration,
) -> std::io::Result<S> {
    let host = host_for_connect(t_host);
    let mut req = format!("CONNECT {host}:{t_port} HTTP/1.1\r\nHost: {host}:{t_port}\r\n");
    if let Some(enc) = basic_proxy_auth(user.as_deref(), pass.as_deref()) {
        req.push_str(&format!("Proxy-Authorization: Basic {enc}\r\n"));
    }
    req.push_str("\r\n");
    write_all_t(&mut s, req.as_bytes(), d).await?;
    let deadline = Instant::now() + d;
    let mut buf = Vec::new();
    // Read exactly up to the end of the header. Reading a larger chunk can
    // consume bytes that the proxy already sent for the tunnel and lose them.
    let mut one = [0u8; 1];
    loop {
        if buf.len() > 32768 {
            return Err(Error::new(ErrorKind::InvalidData, "proxy header too big"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::new(ErrorKind::TimedOut, "proxy read timeout"));
        }
        let n = match tokio::time::timeout(remaining, s.read(&mut one)).await {
            Ok(Ok(0)) => return Err(Error::new(ErrorKind::UnexpectedEof, "proxy closed")),
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(Error::new(ErrorKind::TimedOut, "proxy read timeout")),
        };
        buf.extend_from_slice(&one[..n]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let code = head
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("");
    if code != "200" {
        return Err(Error::new(
            ErrorKind::ConnectionRefused,
            format!("http proxy: {code}"),
        ));
    }
    Ok(s)
}

async fn tls_to_proxy(
    stream: TcpStream,
    host: &str,
    d: Duration,
) -> std::io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    use rustls_pki_types::ServerName;
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "bad proxy tls name"))?;
    let connector = tokio_rustls::TlsConnector::from(tls_client_config());
    match tokio::time::timeout(d, connector.connect(name, stream)).await {
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => Err(Error::other(format!("proxy tls: {e}"))),
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "proxy tls timeout")),
    }
}

async fn connect_to_http_proxy(up: &Upstream, timeout: Duration) -> std::io::Result<ProxyStream> {
    let addr = format!("{}:{}", host_for_connect(&up.host), up.port);
    let stream = match tokio::time::timeout(timeout, TcpStream::connect(addr.as_str())).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(Error::new(ErrorKind::TimedOut, "upstream timeout")),
    };
    stream.set_nodelay(true).ok();
    if up.scheme == "https" {
        Ok(ProxyStream::Tls(Box::new(
            tls_to_proxy(stream, &up.host, timeout).await?,
        )))
    } else {
        Ok(ProxyStream::Plain(stream))
    }
}

/// socks4/socks5 resolve the destination locally; socks4a/socks5h send the
/// hostname to the proxy. This matches the checker (reqwest) and strict
/// SOCKS4 servers that do not implement the SOCKS4a extension.
async fn socks_target_host(
    scheme: &str,
    t_host: &str,
    t_port: u16,
    timeout: Duration,
) -> std::io::Result<String> {
    let local = matches!(scheme, "socks4" | "socks5");
    if !local || t_host.parse::<IpAddr>().is_ok() {
        return Ok(t_host.to_string());
    }
    match tokio::time::timeout(timeout, tokio::net::lookup_host((t_host, t_port))).await {
        Ok(Ok(mut addrs)) => addrs
            .next()
            .map(|a| a.ip().to_string())
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "socks target did not resolve")),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "socks dns timeout")),
    }
}

async fn connect_via(
    up: &Upstream,
    t_host: &str,
    t_port: u16,
    timeout: Duration,
) -> std::io::Result<ProxyStream> {
    let target = socks_target_host(&up.scheme, t_host, t_port, timeout).await?;
    let target = target.as_str();
    let addr = format!("{}:{}", host_for_connect(&up.host), up.port);
    let stream = match tokio::time::timeout(timeout, TcpStream::connect(addr.as_str())).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(Error::new(ErrorKind::TimedOut, "upstream timeout")),
    };
    stream.set_nodelay(true).ok();
    match up.scheme.as_str() {
        "socks5" | "socks5h" => {
            socks5_connect(
                ProxyStream::Plain(stream),
                target,
                t_port,
                up.user.clone(),
                up.pass.clone(),
                timeout,
            )
            .await
        }
        "socks4" => {
            socks4_connect(
                ProxyStream::Plain(stream),
                target,
                t_port,
                up.user.clone(),
                false,
                timeout,
            )
            .await
        }
        "socks4a" => {
            socks4_connect(
                ProxyStream::Plain(stream),
                target,
                t_port,
                up.user.clone(),
                true,
                timeout,
            )
            .await
        }
        "https" => {
            let tls = tls_to_proxy(stream, &up.host, timeout).await?;
            http_connect(
                ProxyStream::Tls(Box::new(tls)),
                t_host,
                t_port,
                up.user.clone(),
                up.pass.clone(),
                timeout,
            )
            .await
        }
        _ => {
            http_connect(
                ProxyStream::Plain(stream),
                t_host,
                t_port,
                up.user.clone(),
                up.pass.clone(),
                timeout,
            )
            .await
        }
    }
}

// ================= TLS fingerprint =================

static TLS_CFG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();

fn tls_client_config() -> Arc<rustls::ClientConfig> {
    TLS_CFG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            // NOTE: provider is pinned explicitly. Several rustls providers
            // (ring + aws-lc-rs) are compiled into the dependency tree, so
            // the feature-based auto-detect in ClientConfig::builder()
            // panics — see CryptoProvider docs.
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            Arc::new(
                rustls::ClientConfig::builder_with_provider(provider)
                    .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                    .expect("ring supports TLS 1.2/1.3")
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

fn cert_fp_hex(der: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(der);
    format!("{:x}", h.finalize())
}

async fn tls_handshake_fp<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    timeout: Duration,
) -> std::io::Result<(String, String)> {
    use rustls_pki_types::ServerName;
    let name = ServerName::try_from("example.com")
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "bad tls name"))?
        .to_owned();
    let connector = tokio_rustls::TlsConnector::from(tls_client_config());
    let tls = match tokio::time::timeout(timeout, connector.connect(name, stream)).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(Error::new(ErrorKind::TimedOut, "tls handshake timeout")),
    };
    let (_io, conn) = tls.get_ref();
    let certs = conn
        .peer_certificates()
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "no peer certificate"))?;
    let leaf = certs
        .first()
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "empty certificate chain"))?;
    let fp = cert_fp_hex(leaf.as_ref());
    let ver = match conn.protocol_version() {
        Some(rustls::ProtocolVersion::TLSv1_3) => "TLS 1.3",
        Some(rustls::ProtocolVersion::TLSv1_2) => "TLS 1.2",
        _ => "TLS ?",
    };
    let cipher = conn
        .negotiated_cipher_suite()
        .map(|s| format!("{:?}", s.suite()))
        .unwrap_or_else(|| "?".to_string());
    let short = fp.chars().take(12).collect::<String>();
    Ok((fp, format!("{ver} {cipher} {short}")))
}

fn is_cert_error(e: &std::io::Error) -> bool {
    let s = e.to_string().to_ascii_lowercase();
    s.contains("certificate") || s.contains("unknownissuer") || s.contains("invalidcertificate")
}

async fn tls_direct_fp() -> Option<String> {
    let timeout = Duration::from_millis(9000);
    let stream = match tokio::time::timeout(timeout, TcpStream::connect(("example.com", 443))).await
    {
        Ok(Ok(s)) => s,
        _ => return None,
    };
    tls_handshake_fp(stream, timeout)
        .await
        .ok()
        .map(|(fp, _)| fp)
}

async fn tls_via_proxy(
    proxy_url: &str,
    timeout: Duration,
    base_fp: Option<&str>,
) -> (Option<String>, Option<String>) {
    let (scheme, host, port, user, pass) = match parse_upstream_url(proxy_url) {
        Some(v) => v,
        None => return (None, None),
    };
    let up = Upstream {
        raw: proxy_url.to_string(),
        url: proxy_url.to_string(),
        scheme,
        host,
        port,
        user,
        pass,
        latency: None,
        country: None,
    };
    let stream = match connect_via(&up, "example.com", 443, timeout).await {
        Ok(s) => s,
        Err(_) => return (None, None),
    };
    match tls_handshake_fp(stream, timeout).await {
        Ok((fp, info)) => match base_fp {
            Some(b) if fp == b => (Some("ok".to_string()), Some(info)),
            Some(_) => (Some("modified".to_string()), Some(info)),
            None => (None, Some(info)),
        },
        Err(e) if is_cert_error(&e) => (
            Some("modified".to_string()),
            Some(format!(
                "certificate rejected: {}",
                short_error(&e.to_string())
            )),
        ),
        // A failed handshake is not proof of tampering (the proxy may block
        // example.com:443), but it must not look like a successful check.
        Err(e) => (
            Some("unverified".to_string()),
            Some(format!("handshake failed: {}", short_error(&e.to_string()))),
        ),
    }
}

// ================= checker =================

fn dead_result(raw: String, proto: String, attempts: u32, err: String) -> ProxyResult {
    ProxyResult {
        url: raw.clone(),
        proxy: raw,
        proto,
        alive: false,
        latency_ms: None,
        jitter_ms: None,
        success_rate: 0.0,
        attempts,
        ip: None,
        country: None,
        country_code: None,
        city: None,
        isp: None,
        org: None,
        asn: None,
        ip_type: None,
        anonymity: None,
        tamper: None,
        tls: None,
        tls_info: None,
        error: Some(err),
    }
}

#[tauri::command]
fn cancel_check(state: State<'_, CheckState>) -> Result<(), String> {
    state.cancel.store(true, Ordering::SeqCst);
    Ok(())
}

fn candidate_budget(timeout: Duration, repeats: usize, flags: &CheckFlags) -> Duration {
    // Main attempts: repeats × timeout. Deep stages run concurrently; the
    // slowest is TLS via an HTTPS proxy: TCP + CONNECT + handshake = 3×.
    // A local DNS lookup for socks4/socks5 adds one more timeout.
    // Without TLS, one timeout is enough for the whole deep group.
    let deep = if flags.judge {
        Duration::ZERO
    } else if flags.tls {
        timeout * 4
    } else {
        timeout
    };
    flags.precheck_timeout + timeout * (repeats as u32 + 1) + deep + Duration::from_secs(5)
}

#[allow(clippy::too_many_arguments)]
async fn check_candidate(
    raw: String,
    url: String,
    proto: String,
    test_url: String,
    timeout: Duration,
    repeats: usize,
    flags: CheckFlags,
    baseline: Arc<Baseline>,
    geo_cache: Arc<HashMap<String, GeoCacheEntry>>,
    cancel: Arc<AtomicBool>,
) -> ProxyResult {
    // absolute ceiling: a single proxy may never outlive the sum of its own timeouts
    let budget = candidate_budget(timeout, repeats, &flags);
    let raw_fb = raw.clone();
    let proto_fb = proto.clone();
    match tokio::time::timeout(
        budget,
        check_candidate_inner(
            raw, url, proto, test_url, timeout, repeats, flags, baseline, geo_cache, cancel,
        ),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => dead_result(raw_fb, proto_fb, 0, "timeout".to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn check_candidate_inner(
    raw: String,
    url: String,
    proto: String,
    test_url: String,
    timeout: Duration,
    repeats: usize,
    flags: CheckFlags,
    baseline: Arc<Baseline>,
    geo_cache: Arc<HashMap<String, GeoCacheEntry>>,
    cancel: Arc<AtomicBool>,
) -> ProxyResult {
    // stage 1: fast tcp precheck
    if flags.precheck {
        if let Some((h, p)) = host_port_of_url(&url) {
            let r =
                tokio::time::timeout(flags.precheck_timeout, TcpStream::connect((h.as_str(), p)))
                    .await;
            let ok = matches!(r, Ok(Ok(_)));
            if !ok {
                let err = match r {
                    Ok(Err(e)) => short_error(&e.to_string()),
                    _ => "timeout".to_string(),
                };
                return dead_result(raw, proto, 0, err);
            }
        }
    }

    let client = match build_client(Some(&url), timeout, false) {
        Ok(c) => c,
        Err(e) => return dead_result(raw, proto, 0, e),
    };

    // judge mode: pass our direct IP so the judge can detect leaks
    let req_url = if flags.judge {
        match baseline.direct_ip.as_deref() {
            Some(dip) if !dip.is_empty() => {
                let sep = if test_url.contains('?') { '&' } else { '?' };
                format!("{test_url}{sep}direct_ip={dip}")
            }
            _ => test_url.clone(),
        }
    } else {
        test_url.clone()
    };

    let mut latencies: Vec<u64> = Vec::new();
    let mut first_body: Option<String> = None;
    let mut last_err: Option<String> = None;

    for _ in 0..repeats {
        if cancel.load(Ordering::Relaxed) {
            last_err = Some("cancelled".to_string());
            break;
        }
        let start = Instant::now();
        match client.get(&req_url).send().await {
            Ok(r) => {
                let status = r.status();
                let ms = start.elapsed().as_millis() as u64;
                if status.is_success() {
                    latencies.push(ms);
                    if first_body.is_none() {
                        first_body = read_body_capped(r, BODY_LIMIT_PAGE).await.ok();
                    }
                    last_err = None;
                } else {
                    last_err = Some(format!("http {}", status.as_u16()));
                }
            }
            Err(e) => {
                last_err = Some(short_error(&e.to_string()));
            }
        }
    }

    let attempts = repeats as u32;
    let successes = latencies.len() as u32;
    if successes == 0 {
        return dead_result(
            raw,
            proto,
            attempts,
            last_err.unwrap_or_else(|| "failed".to_string()),
        );
    }

    let avg = latencies.iter().sum::<u64>() / latencies.len() as u64;
    let jitter = if latencies.len() > 1 {
        Some(latencies.iter().max().unwrap() - latencies.iter().min().unwrap())
    } else {
        Some(0)
    };
    let success_rate = successes as f32 / attempts as f32;

    let mut res = ProxyResult {
        proxy: raw.clone(),
        url: url.clone(),
        proto,
        alive: true,
        latency_ms: Some(avg),
        jitter_ms: jitter,
        success_rate,
        attempts,
        ip: None,
        country: None,
        country_code: None,
        city: None,
        isp: None,
        org: None,
        asn: None,
        ip_type: None,
        anonymity: None,
        tamper: None,
        tls: None,
        tls_info: None,
        error: None,
    };

    if let Some(b) = first_body {
        if flags.judge {
            let proxy_sha = try_parse_judge_body(&mut res, &b);
            res.tamper = match (&baseline.judge_content_sha, &proxy_sha) {
                (Some(a), Some(c)) => Some(if a == c {
                    "ok".to_string()
                } else {
                    "modified".to_string()
                }),
                _ => None,
            };
        } else {
            try_parse_test_body(&mut res, &test_url, &b);
        }
    }
    // Reuse previously seen geo. When the test URL does not reveal the exit
    // IP, the frontend-provided cache (TTL-filtered) is the only way to skip
    // a geo lookup for the default generate_204 URL.
    if let Some(entry) = geo_cache.get(&strip_credentials(&raw)) {
        apply_cached_geo(&mut res, entry);
    }
    // deep stages run concurrently: worst case ~1 timeout instead of 4 in a row
    if !flags.judge && !cancel.load(Ordering::Relaxed) {
        let need_geo = flags.geo && res.country.is_none() && res.ip_type.is_none();
        let want_anon = flags.anon;
        let want_tamper = flags.tamper;
        let want_tls = flags.tls;
        let direct_ip = baseline.direct_ip.clone();
        let (ex_len, ex_hash) = (baseline.example_len, baseline.example_hash);
        let cert_fp = baseline.example_cert_fp.clone();
        let url_c = url.clone();
        let (geo_r, anon_r, tamper_r, tls_r) = tokio::join!(
            async {
                if !need_geo {
                    return None;
                }
                // Skip geo instead of waiting: waiting would eat the
                // per-proxy check budget and turn live proxies into timeouts.
                let _permit = geo_semaphore().try_acquire().ok()?;
                fetch_geo(&client, flags.ipapi_key.as_deref()).await
            },
            async {
                if !want_anon {
                    return None;
                }
                let _permit = deep_semaphore(&ANON_SEM).try_acquire().ok()?;
                detect_anonymity(&client, direct_ip.as_deref()).await
            },
            async {
                if !want_tamper {
                    return None;
                }
                let _permit = deep_semaphore(&TAMPER_SEM).try_acquire().ok()?;
                detect_tamper(&client, ex_len, ex_hash).await
            },
            async {
                if !want_tls {
                    return None;
                }
                let _permit = deep_semaphore(&TLS_SEM).try_acquire().ok()?;
                Some(tls_via_proxy(&url_c, timeout, cert_fp.as_deref()).await)
            },
        );
        if let Some(g) = geo_r {
            apply_ipapi(&mut res, &g);
        }
        if anon_r.is_some() {
            res.anonymity = anon_r;
        }
        if tamper_r.is_some() {
            res.tamper = tamper_r;
        }
        if let Some((v, info)) = tls_r {
            res.tls = v;
            res.tls_info = info;
        }
    }
    res
}

async fn fetch_baseline(flags: CheckFlags) -> Baseline {
    let timeout = Duration::from_millis(9000);
    let want_ip = flags.judge || flags.anon;
    let direct_ip = async {
        if !want_ip {
            return None;
        }
        {
            let c = baseline_cache().lock().await;
            if let Some((v, ts)) = &c.direct_ip {
                if cache_fresh(*ts, BASELINE_IP_TTL) {
                    return Some(v.clone());
                }
            }
        }
        let c = build_client(None, timeout, true).ok()?;
        let r = c.get("https://api.ipify.org").send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        let t = read_body_capped(r, BODY_LIMIT_SMALL).await.ok()?;
        let t = t.trim().to_string();
        if looks_like_ip(&t) {
            baseline_cache().lock().await.direct_ip = Some((t.clone(), Instant::now()));
            Some(t)
        } else {
            None
        }
    };
    let example = async {
        if !flags.tamper {
            return None;
        }
        {
            let c = baseline_cache().lock().await;
            if let Some((len, hash, ts)) = c.example {
                if cache_fresh(ts, BASELINE_PAGE_TTL) {
                    return Some((len, hash));
                }
            }
        }
        let c = build_client(None, timeout, true).ok()?;
        let r = c.get("https://example.com").send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        let t = read_body_capped(r, BODY_LIMIT_PAGE).await.ok()?;
        let val = (t.len(), body_hash(&t));
        baseline_cache().lock().await.example = Some((val.0, val.1, Instant::now()));
        Some(val)
    };
    let cert = async {
        if !flags.tls {
            return None;
        }
        {
            let c = baseline_cache().lock().await;
            if let Some((v, ts)) = &c.cert {
                if cache_fresh(*ts, BASELINE_PAGE_TTL) {
                    return Some(v.clone());
                }
            }
        }
        let fp = tls_direct_fp().await;
        if let Some(v) = &fp {
            baseline_cache().lock().await.cert = Some((v.clone(), Instant::now()));
        }
        fp
    };
    let (ip, ex, cert) = tokio::join!(direct_ip, example, cert);
    let (len, hash) = ex.unzip();
    Baseline {
        direct_ip: ip,
        example_len: len,
        example_hash: hash,
        example_cert_fp: cert,
        judge_content_sha: None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_checks(
    proxies: Vec<String>,
    test_url: String,
    timeout: Duration,
    conc: usize,
    repeats: usize,
    default_proto: &str,
    flags: CheckFlags,
    geo_cache: Vec<GeoCacheEntry>,
    cancel: Arc<AtomicBool>,
    on_result: Option<&Channel<ProxyResult>>,
) -> Result<Vec<ProxyResult>, String> {
    let test_url = test_url.trim().to_string();
    if test_url.is_empty() {
        return Err("empty test url".to_string());
    }
    if !is_http_url(&test_url) {
        return Err("test url must start with http(s)://".to_string());
    }

    let mut jobs: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut seen = HashSet::new();
    let mut seen_urls = HashSet::new();
    for raw in proxies {
        let t = raw.trim().to_string();
        if t.is_empty() || !seen.insert(t.clone()) {
            continue;
        }
        let mut cands = normalize_candidates(&t, default_proto);
        let was_valid = !cands.is_empty();
        // `1.2.3.4:8080` and `http://1.2.3.4:8080` are the same proxy.
        cands.retain(|(url, _)| seen_urls.insert(url.clone()));
        if was_valid && cands.is_empty() {
            continue;
        }
        jobs.push((t, cands));
        if jobs.len() >= 20_000 {
            break;
        }
    }

    let geo_map: Arc<HashMap<String, GeoCacheEntry>> = Arc::new(
        geo_cache
            .into_iter()
            .map(|e| (e.proxy.clone(), e))
            .collect(),
    );

    // Direct judge content hash (for tamper check in judge mode). The fast
    // mode returns an empty 204 body, so the hash would never be used; skip
    // the extra /judge request entirely.
    let judge_url = if flags.judge && !test_url.trim_end_matches('/').ends_with("/generate_204") {
        judge_direct_url(&test_url)
    } else {
        None
    };
    let (baseline_res, judge_sha) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(20), fetch_baseline(flags.clone())),
        async {
            match &judge_url {
                Some(u) => fetch_direct_judge_sha(u, timeout).await,
                None => None,
            }
        }
    );
    let mut baseline_inner = baseline_res.unwrap_or(Baseline {
        direct_ip: None,
        example_len: None,
        example_hash: None,
        example_cert_fp: None,
        judge_content_sha: None,
    });
    baseline_inner.judge_content_sha = judge_sha;
    let baseline = Arc::new(baseline_inner);

    let sem = Arc::new(Semaphore::new(conc));
    let mut set = tokio::task::JoinSet::new();
    let mut task_raw: HashMap<tokio::task::Id, String> = HashMap::new();

    for (raw, cands) in jobs {
        if cands.is_empty() {
            let raw_key = raw.clone();
            let r = dead_result(raw, "BAD".to_string(), 0, "bad format".to_string());
            let handle = set.spawn(async move { r });
            task_raw.insert(handle.id(), raw_key);
            continue;
        }
        let sem_c = sem.clone();
        let tu = test_url.clone();
        let base = baseline.clone();
        let gc = geo_map.clone();
        let cl = cancel.clone();
        let fl = flags.clone();
        let raw_key = raw.clone();
        let handle = set.spawn(async move {
            let _p = sem_c.acquire_owned().await.unwrap();
            if cands.len() == 1 {
                let (url, proto) = &cands[0];
                check_candidate(
                    raw,
                    url.clone(),
                    proto.clone(),
                    tu,
                    timeout,
                    repeats,
                    fl,
                    base,
                    gc,
                    cl,
                )
                .await
            } else {
                let (u1, p1) = &cands[0];
                let r1 = check_candidate(
                    raw.clone(),
                    u1.clone(),
                    p1.clone(),
                    tu.clone(),
                    timeout,
                    repeats,
                    fl.clone(),
                    base.clone(),
                    gc.clone(),
                    cl.clone(),
                )
                .await;
                if r1.alive {
                    return r1;
                }
                if cl.load(Ordering::Relaxed) {
                    return r1;
                }
                let (u2, p2) = &cands[1];
                let r2 = check_candidate(
                    raw.clone(),
                    u2.clone(),
                    p2.clone(),
                    tu,
                    timeout,
                    repeats,
                    fl,
                    base,
                    gc,
                    cl,
                )
                .await;
                if r2.alive {
                    return r2;
                }
                let e1 = r1.error.unwrap_or_default();
                let e2 = r2.error.unwrap_or_default();
                let mut combined = format!("http: {e1} / socks5: {e2}");
                truncate_utf8(&mut combined, 120);
                dead_result(raw, "HTTP/SOCKS5".to_string(), r1.attempts, combined)
            }
        });
        task_raw.insert(handle.id(), raw_key);
    }

    let mut out = Vec::new();
    loop {
        // Stop must abort in-flight requests immediately instead of waiting
        // for the first task to finish its own timeout.
        tokio::select! {
            maybe = set.join_next_with_id() => match maybe {
                Some(Ok((id, v))) => {
                    task_raw.remove(&id);
                    if let Some(ch) = on_result {
                        let _ = ch.send(v.clone());
                    }
                    out.push(v);
                }
                Some(Err(e)) => {
                    let id = e.id();
                    let raw = task_raw.remove(&id).unwrap_or_else(|| "?".to_string());
                    if e.is_cancelled() {
                        continue;
                    }
                    out.push(dead_result(
                        raw,
                        "?".to_string(),
                        0,
                        short_error(&e.to_string()),
                    ));
                }
                None => break,
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if cancel.load(Ordering::Relaxed) {
                    set.abort_all();
                    break;
                }
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        set.abort_all();
    }
    out.sort_by(|a, b| {
        b.alive.cmp(&a.alive).then(
            a.latency_ms
                .unwrap_or(u64::MAX)
                .cmp(&b.latency_ms.unwrap_or(u64::MAX)),
        )
    });
    Ok(out)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn check_proxies(
    proxies: Vec<String>,
    test_url: String,
    timeout_ms: u64,
    concurrency: usize,
    default_proto: String,
    repeats: Option<usize>,
    with_geo: Option<bool>,
    with_anonymity: Option<bool>,
    with_tamper: Option<bool>,
    with_tls: Option<bool>,
    precheck: Option<bool>,
    precheck_timeout_ms: Option<u64>,
    judge_mode: Option<bool>,
    ipapi_key: Option<String>,
    geo_cache: Option<Vec<GeoCacheEntry>>,
    state: State<'_, CheckState>,
    on_result: Channel<ProxyResult>,
) -> Result<(), String> {
    let busy = state.busy.clone();
    if busy.swap(true, Ordering::SeqCst) {
        return Err(CHECK_BUSY.to_string());
    }
    let _guard = BusyGuard(busy);
    state.cancel.store(false, Ordering::SeqCst);

    let timeout = Duration::from_millis(timeout_ms.clamp(500, 60_000));
    let conc = concurrency.clamp(1, 500);
    let repeats = repeats.unwrap_or(1).clamp(1, 5);
    let flags = CheckFlags {
        geo: with_geo.unwrap_or(true),
        anon: with_anonymity.unwrap_or(true),
        tamper: with_tamper.unwrap_or(true),
        tls: with_tls.unwrap_or(true),
        precheck: precheck.unwrap_or(true),
        precheck_timeout: Duration::from_millis(
            precheck_timeout_ms.unwrap_or(1500).clamp(300, 10_000),
        ),
        judge: judge_mode.unwrap_or(false),
        ipapi_key: ipapi_key
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| Arc::from(s.as_str())),
    };

    run_checks(
        proxies,
        test_url,
        timeout,
        conc,
        repeats,
        &default_proto,
        flags,
        geo_cache.unwrap_or_default(),
        state.cancel.clone(),
        Some(&on_result),
    )
    .await?;
    // The channel carries the authoritative result list; this marker tells
    // the UI that the channel is drained. Returning the Vec as well duplicated
    // the whole payload over IPC.
    let _ = on_result.send(dead_result(
        String::new(),
        "__done__".to_string(),
        0,
        String::new(),
    ));
    Ok(())
}

#[tauri::command]
fn check_busy(state: State<'_, CheckState>) -> bool {
    state.busy.load(Ordering::SeqCst)
}

#[tauri::command]
async fn check_direct(test_url: String, timeout_ms: u64) -> Result<u64, String> {
    let test_url = test_url.trim().to_string();
    if !is_http_url(&test_url) {
        return Err("test url must start with http(s)://".to_string());
    }
    let timeout = Duration::from_millis(timeout_ms.clamp(500, 30_000));
    let client = build_client(None, timeout, true).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let r = client
        .get(test_url)
        .send()
        .await
        .map_err(|e| short_error(&e.to_string()))?;
    if !r.status().is_success() {
        return Err(format!("http {}", r.status().as_u16()));
    }
    Ok(start.elapsed().as_millis() as u64)
}

fn validate_export_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() || path.contains('\0') {
        return Err("invalid path".to_string());
    }
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return Err("path must be absolute".to_string());
    }
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    if !matches!(ext.as_deref(), Some("txt" | "json" | "csv" | "log")) {
        return Err("unsupported file extension".to_string());
    }
    Ok(())
}

fn with_export_extension(mut path: std::path::PathBuf, ext: &str) -> std::path::PathBuf {
    let ext = ext.trim_start_matches('.');
    if !ext.is_empty() && path.extension().is_none() {
        path.set_extension(ext);
    }
    path
}

#[tauri::command]
async fn export_text_file(
    app: tauri::AppHandle,
    contents: String,
    default_name: String,
    filter_name: String,
    extensions: Vec<String>,
) -> Result<Option<String>, String> {
    let ext = extensions
        .first()
        .cloned()
        .unwrap_or_else(|| "txt".to_string());
    let default_name = with_export_extension(std::path::PathBuf::from(default_name), &ext)
        .to_string_lossy()
        .to_string();
    let extensions: Vec<String> = extensions
        .into_iter()
        .map(|e| e.trim_start_matches('.').to_string())
        .filter(|e| !e.is_empty())
        .collect();
    let filter_name = if filter_name.trim().is_empty() {
        "Text".to_string()
    } else {
        filter_name
    };
    let choice = tauri::async_runtime::spawn_blocking(move || {
        let refs: Vec<&str> = extensions.iter().map(String::as_str).collect();
        app.dialog()
            .file()
            .set_file_name(default_name)
            .add_filter(filter_name, &refs)
            .blocking_save_file()
    })
    .await
    .map_err(|e| short_error(&e.to_string()))?;
    let Some(path) = choice else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| short_error(&e.to_string()))?;
    let path = with_export_extension(path, &ext);
    validate_export_path(&path.to_string_lossy())?;
    let write_path = path.clone();
    tauri::async_runtime::spawn_blocking(move || std::fs::write(&write_path, contents))
        .await
        .map_err(|e| short_error(&e.to_string()))?
        .map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().to_string()))
}

// ================= local proxy dispatcher =================

#[derive(Debug, Clone, Deserialize)]
struct DispatchItem {
    raw: String,
    /// Normalized URL from the checker. Falls back to `raw` for old callers.
    #[serde(default)]
    url: Option<String>,
    latency: Option<u64>,
    #[serde(default)]
    country: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DispatchStatus {
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
struct DispatchPoolItem {
    raw: String,
    url: String,
    latency: Option<u64>,
    country: Option<String>,
}

async fn pool_items(inner: &Arc<DispatchInner>) -> Vec<DispatchPoolItem> {
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

struct DispatchInner {
    running: AtomicBool,
    port: Mutex<u16>,
    mode: Mutex<String>,
    listener: Mutex<String>,
    pool: Mutex<Vec<Upstream>>,
    rr: AtomicUsize,
    requests: AtomicU64,
    errors: AtomicU64,
    current: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    lifecycle: Mutex<()>,
    /// All active client connections, so Stop can terminate tunnels instead
    /// of leaving them running after the listener is gone.
    connections: Mutex<tokio::task::JoinSet<()>>,
    /// Cached loopback checks for upstream hostnames (`host:port` -> result).
    dns_cache: Mutex<HashMap<String, (bool, Instant)>>,
    /// Caps local client connections; without it a local process could
    /// exhaust file descriptors.
    conn_sem: Arc<Semaphore>,
}

const MAX_CONNECTIONS: usize = 1024;

struct DispatchState {
    inner: Arc<DispatchInner>,
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
            }),
        }
    }
}

// ================= scheduled checks =================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutoSettings {
    disp_enabled: bool,
    disp_interval_min: u64,
    auto_enabled: bool,
    auto_interval_min: u64,
    max_ping_ms: u64,
    webhook_urls: Vec<String>,
    webhook_format: String,
    webhook_include_credentials: bool,
    test_url: String,
    timeout_ms: u64,
    concurrency: usize,
    repeats: usize,
    default_proto: String,
    with_geo: bool,
    with_anonymity: bool,
    with_tamper: bool,
    with_tls: bool,
    precheck: bool,
    precheck_timeout_ms: u64,
    judge_mode: bool,
    ipapi_key: Option<String>,
}

impl Default for AutoSettings {
    fn default() -> Self {
        Self {
            disp_enabled: false,
            disp_interval_min: 30,
            auto_enabled: false,
            auto_interval_min: 30,
            max_ping_ms: 0,
            webhook_urls: Vec::new(),
            webhook_format: "txt".to_string(),
            webhook_include_credentials: false,
            test_url: "https://www.google.com/generate_204".to_string(),
            timeout_ms: 5000,
            concurrency: 50,
            repeats: 1,
            default_proto: "http".to_string(),
            with_geo: true,
            with_anonymity: true,
            with_tamper: true,
            with_tls: true,
            precheck: true,
            precheck_timeout_ms: 1500,
            judge_mode: false,
            ipapi_key: None,
        }
    }
}

struct AutoInner {
    settings: Mutex<AutoSettings>,
    disp_next: AtomicU64,
    auto_next: AtomicU64,
    running: Arc<AtomicBool>,
}

struct AutoState {
    inner: Arc<AutoInner>,
}

impl Default for AutoState {
    fn default() -> Self {
        Self {
            inner: Arc::new(AutoInner {
                settings: Mutex::new(AutoSettings::default()),
                disp_next: AtomicU64::new(0),
                auto_next: AtomicU64::new(0),
                running: Arc::new(AtomicBool::new(false)),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum AutoKind {
    Dispatch,
    Auto,
}

impl AutoKind {
    fn schedule(self) -> &'static str {
        match self {
            AutoKind::Dispatch => "disp",
            AutoKind::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoStatus {
    schedule: String,
    state: String,
    alive: usize,
    total: usize,
    avg_ms: Option<u64>,
    error: Option<String>,
    at: u64,
}

#[derive(Debug, Clone, Serialize)]
struct AutoPoolPayload {
    items: Vec<DispatchPoolItem>,
}

#[derive(Debug, Clone, Serialize)]
struct WebhookResult {
    index: usize,
    ok: bool,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct AutoWebhookPayload {
    results: Vec<WebhookResult>,
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn emit_auto_status(
    app: &tauri::AppHandle,
    kind: AutoKind,
    state: &str,
    alive: usize,
    total: usize,
    avg_ms: Option<u64>,
    error: Option<String>,
) {
    let _ = app.emit(
        "auto://status",
        AutoStatus {
            schedule: kind.schedule().to_string(),
            state: state.to_string(),
            alive,
            total,
            avg_ms,
            error,
            at: epoch_secs(),
        },
    );
}

// ================= tray status =================

#[derive(Debug, Default, Clone)]
struct TrayInfo {
    webhook_on: bool,
    webhook_url: String,
    lang: String,
}

#[derive(Default)]
struct TrayState {
    info: Mutex<TrayInfo>,
    last_menu: Mutex<String>,
}

fn fmt_countdown(s: Option<u64>, ru: bool) -> String {
    match s {
        None => {
            if ru {
                "выкл".to_string()
            } else {
                "off".to_string()
            }
        }
        Some(x) if x >= 3600 => format!("{}h {:02}m", x / 3600, (x % 3600) / 60),
        Some(x) if x >= 60 => format!("{}m", x / 60),
        Some(_) => {
            if ru {
                "скоро".to_string()
            } else {
                "soon".to_string()
            }
        }
    }
}

/// Only the host is shown in the tray: webhook URLs may contain secrets.
fn url_host(u: &str) -> String {
    reqwest::Url::parse(u)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

async fn refresh_tray_menu(app: &tauri::AppHandle) {
    let disp: State<DispatchState> = app.state();
    let tray: State<TrayState> = app.state();
    let running = disp.inner.running.load(Ordering::SeqCst);
    let port = *disp.inner.port.lock().await;
    let info = tray.info.lock().await.clone();
    let auto: State<AutoState> = app.state();
    let now = epoch_secs();
    let remaining = |v: u64| {
        if v == 0 {
            None
        } else {
            Some(v.saturating_sub(now))
        }
    };
    let disp_next = remaining(auto.inner.disp_next.load(Ordering::SeqCst));
    let auto_next = remaining(auto.inner.auto_next.load(Ordering::SeqCst));
    let ru = info.lang == "ru";
    let dispatch_txt = if running {
        if ru {
            format!("Диспетчер: ВКЛ :{port}")
        } else {
            format!("Dispatcher: ON :{port}")
        }
    } else if ru {
        "Диспетчер: ВЫКЛ".to_string()
    } else {
        "Dispatcher: OFF".to_string()
    };
    let next_disp_txt = format!(
        "{} {}",
        if ru {
            "След. диспетчер:"
        } else {
            "Next dispatch:"
        },
        fmt_countdown(disp_next, ru)
    );
    let next_auto_txt = format!(
        "{} {}",
        if ru {
            "След. авто:"
        } else {
            "Next auto:"
        },
        fmt_countdown(auto_next, ru)
    );
    let sig = format!(
        "{dispatch_txt}|{next_disp_txt}|{next_auto_txt}|{}|{}|{}",
        info.webhook_on, info.webhook_url, info.lang
    );
    {
        let last = tray.last_menu.lock().await;
        if *last == sig {
            return;
        }
    }
    let build = || -> tauri::Result<()> {
        let show = MenuItem::with_id(
            app,
            "show",
            if ru { "Показать" } else { "Show" },
            true,
            None::<&str>,
        )?;
        let hide = MenuItem::with_id(
            app,
            "hide",
            if ru { "Скрыть" } else { "Hide" },
            true,
            None::<&str>,
        )?;
        let dispatch =
            MenuItem::with_id(app, "dispatch_toggle", &dispatch_txt, true, None::<&str>)?;
        let next_disp = MenuItem::with_id(app, "next_disp", &next_disp_txt, false, None::<&str>)?;
        let next_auto = MenuItem::with_id(app, "next_auto", &next_auto_txt, false, None::<&str>)?;
        let webhook_txt = if info.webhook_on && !info.webhook_url.trim().is_empty() {
            let label = if ru {
                "Вебхук: вкл"
            } else {
                "Webhook: on"
            };
            let host = url_host(info.webhook_url.trim());
            if host.is_empty() {
                label.to_string()
            } else {
                format!("{label} {host}")
            }
        } else if ru {
            "Вебхук: выкл".to_string()
        } else {
            "Webhook: off".to_string()
        };
        let webhook = MenuItem::with_id(app, "webhook_info", &webhook_txt, false, None::<&str>)?;
        let quit = MenuItem::with_id(
            app,
            "quit",
            if ru { "Выход" } else { "Quit" },
            true,
            None::<&str>,
        )?;
        let sep1 = PredefinedMenuItem::separator(app)?;
        let sep2 = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(
            app,
            &[
                &show, &hide, &sep1, &dispatch, &next_disp, &next_auto, &webhook, &sep2, &quit,
            ],
        )?;
        if let Some(tray_icon) = app.tray_by_id("main") {
            tray_icon.set_menu(Some(menu))?;
        }
        Ok(())
    };
    if build().is_ok() {
        *tray.last_menu.lock().await = sig;
    }
}

#[tauri::command]
async fn update_tray_status(
    app: tauri::AppHandle,
    tray: State<'_, TrayState>,
    webhook_on: bool,
    webhook_url: String,
    lang: Option<String>,
) -> Result<(), String> {
    let lang = match lang.as_deref() {
        Some("ru") => "ru",
        _ => "en",
    };
    *tray.info.lock().await = TrayInfo {
        webhook_on,
        webhook_url,
        lang: lang.to_string(),
    };
    refresh_tray_menu(&app).await;
    Ok(())
}

/// Return at most `limit` upstream indexes. The pool is kept sorted by
/// latency in `set_dispatch_pool`, so "fastest"/"failover" are just the head
/// of the list; no per-request sort or full-pool allocation is needed.
fn pick_order(len: usize, mode: &str, rr: &AtomicUsize, limit: usize) -> Vec<usize> {
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

fn parse_target(method: &str, target: &str) -> Option<(String, u16)> {
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

fn is_self_literal(host: &str) -> bool {
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

fn is_self_upstream(up: &Upstream, port: u16) -> bool {
    up.port == port && is_self_literal(&up.host)
}

/// Resolve hostname aliases such as `127.0.0.1.nip.io` that point at the
/// local listener. Results are cached for a few minutes; only entries with
/// the listener's port are resolved, so the cost is normally tiny.
async fn is_self_upstream_async(
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

struct CountingIo<T> {
    inner: T,
    written: Arc<AtomicU64>,
}

impl<T> CountingIo<T> {
    fn new(inner: T, written: Arc<AtomicU64>) -> Self {
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

async fn abort_connections(inner: &Arc<DispatchInner>) {
    let mut conns = inner.connections.lock().await;
    conns.abort_all();
    while conns.join_next().await.is_some() {}
}

async fn handle_http(mut client: TcpStream, st: Arc<DispatchInner>) {
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
                let write_res = if use_forward {
                    let forward = prepare_forward_request(&buf, hdr_end, up);
                    ups.write_all(&forward).await
                } else {
                    // SOCKS tunnel: strip local-proxy headers before handing
                    // the request to the origin.
                    let stripped = strip_header(&buf, hdr_end, "proxy-authorization");
                    let end = find_header_end(&stripped).unwrap_or(stripped.len());
                    let stripped = strip_header(&stripped, end, "proxy-connection");
                    ups.write_all(&stripped).await
                };
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

async fn handle_socks5(mut client: TcpStream, st: Arc<DispatchInner>) {
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
fn upstream_from_item(
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

async fn store_pool(inner: &Arc<DispatchInner>, items: Vec<DispatchItem>) -> usize {
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
    let stop_empty = n == 0 && running;
    *inner.pool.lock().await = pool;
    if stop_empty {
        // A listener with zero upstreams only replies 502. Clearing the pool
        // is an explicit user action, so stop the dispatcher with it.
        stop_proxy_inner(inner).await;
    }
    n
}

#[tauri::command]
async fn set_dispatch_pool(
    items: Vec<DispatchItem>,
    state: State<'_, DispatchState>,
) -> Result<usize, String> {
    Ok(store_pool(&state.inner, items).await)
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

async fn start_proxy_inner(
    inner: &Arc<DispatchInner>,
    req_port: u16,
    mode: String,
    listener: String,
) -> Result<String, String> {
    let _lifecycle = inner.lifecycle.lock().await;
    start_proxy_locked(inner, req_port, mode, listener).await
}

async fn stop_proxy_inner(inner: &Arc<DispatchInner>) {
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
async fn start_local_proxy(
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
async fn stop_local_proxy(state: State<'_, DispatchState>) -> Result<(), String> {
    stop_proxy_inner(&state.inner).await;
    Ok(())
}

#[tauri::command]
async fn get_dispatch_pool(
    state: State<'_, DispatchState>,
) -> Result<Vec<DispatchPoolItem>, String> {
    Ok(pool_items(&state.inner).await)
}

#[tauri::command]
async fn local_proxy_status(state: State<'_, DispatchState>) -> Result<DispatchStatus, String> {
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

fn webhook_proxies(items: &[DispatchItem], include_credentials: bool) -> Vec<String> {
    items
        .iter()
        .map(|i| {
            let source = i
                .url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(&i.raw);
            if include_credentials {
                source.to_string()
            } else {
                strip_credentials(source)
            }
        })
        .collect()
}

fn csv_cell(s: &str) -> String {
    let guarded = if s
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r'))
    {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

fn webhook_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .use_rustls_tls()
        .user_agent(concat!("ProxPulse/", env!("CARGO_PKG_VERSION")))
        // A webhook may redirect to an arbitrary (including plain HTTP)
        // host. Following such a redirect would re-send the body, including
        // proxy credentials, outside the original HTTPS endpoint.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

async fn send_webhook_one(
    url: &str,
    format: &str,
    include_credentials: bool,
    items: &[DispatchItem],
) -> Result<String, String> {
    let url = url.trim().to_string();
    if !is_http_url(&url) {
        return Err("bad webhook url".to_string());
    }
    if include_credentials && url.to_ascii_lowercase().starts_with("http://") {
        return Err("https required when sending proxy credentials".to_string());
    }
    let proxies = webhook_proxies(items, include_credentials);
    let (body, ctype) = match format {
        "json" => {
            let arr: Vec<serde_json::Value> = items
                .iter()
                .enumerate()
                .map(|(n, i)| {
                    serde_json::json!({"proxy": proxies[n].clone(), "latency_ms": i.latency})
                })
                .collect();
            (
                serde_json::to_string(&arr).map_err(|e| e.to_string())?,
                "application/json",
            )
        }
        "csv" => {
            let mut s = String::from("proxy,latency_ms\n");
            for (n, i) in items.iter().enumerate() {
                s.push_str(&format!(
                    "{},{}\n",
                    csv_cell(&proxies[n]),
                    i.latency.map(|v| v.to_string()).unwrap_or_default()
                ));
            }
            (s, "text/csv")
        }
        _ => (proxies.join("\n"), "text/plain"),
    };
    let client = webhook_client()?;
    let r = client
        .post(url)
        .header("Content-Type", ctype)
        .body(body)
        .send()
        .await
        .map_err(|e| short_error(&e.to_string()))?;
    let code = r.status().as_u16();
    let txt = read_body_capped(r, BODY_LIMIT_WEBHOOK)
        .await
        .unwrap_or_default();
    let short: String = txt.chars().take(80).collect();
    Ok(format!("http {code} {short}"))
}

async fn send_webhooks_inner(
    urls: &[String],
    format: &str,
    include_credentials: bool,
    items: Arc<Vec<DispatchItem>>,
) -> Vec<WebhookResult> {
    let mut set = tokio::task::JoinSet::new();
    for (index, url) in urls.iter().take(10).cloned().enumerate() {
        let format = format.to_string();
        let items = items.clone();
        set.spawn(async move {
            let res = send_webhook_one(&url, &format, include_credentials, &items).await;
            match res {
                Ok(message) => WebhookResult {
                    index,
                    ok: true,
                    message,
                },
                Err(message) => WebhookResult {
                    index,
                    ok: false,
                    message,
                },
            }
        });
    }
    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        match r {
            Ok(v) => out.push(v),
            Err(e) => out.push(WebhookResult {
                index: usize::MAX,
                ok: false,
                message: short_error(&e.to_string()),
            }),
        }
    }
    out.sort_by_key(|r| r.index);
    out
}

#[tauri::command]
async fn send_webhooks(
    urls: Vec<String>,
    format: String,
    include_credentials: Option<bool>,
    items: Vec<DispatchItem>,
) -> Result<Vec<WebhookResult>, String> {
    Ok(send_webhooks_inner(
        &urls,
        &format,
        include_credentials.unwrap_or(false),
        Arc::new(items),
    )
    .await)
}

#[tauri::command]
async fn send_webhook(
    url: String,
    format: String,
    include_credentials: Option<bool>,
    items: Vec<DispatchItem>,
) -> Result<String, String> {
    send_webhook_one(&url, &format, include_credentials.unwrap_or(false), &items).await
}

/// Merge checked results with the pool as it is *after* the check. A proxy
/// deleted (or a pool cleared) by the user while the check was running must
/// stay deleted; entries added meanwhile stay too.
fn merge_auto_items(
    results: &[ProxyResult],
    current: &HashMap<String, (String, Option<u64>, Option<String>)>,
    checked: &HashSet<String>,
    max_ping_ms: u64,
) -> (Vec<DispatchItem>, Option<u64>) {
    let mut items = Vec::new();
    let mut avg_sum = 0u64;
    let mut avg_n = 0u64;
    for r in results {
        if !r.alive {
            continue;
        }
        if let Some(ms) = r.latency_ms {
            avg_sum += ms;
            avg_n += 1;
        }
        let Some((raw, _, old_country)) = current.get(&r.url) else {
            continue;
        };
        if max_ping_ms > 0 && r.latency_ms.is_some_and(|ms| ms > max_ping_ms) {
            continue;
        }
        let country = r
            .country_code
            .clone()
            .or_else(|| r.country.clone())
            .or_else(|| old_country.clone());
        items.push(DispatchItem {
            raw: raw.clone(),
            url: Some(r.url.clone()),
            latency: r.latency_ms,
            country,
        });
    }
    for (url, (raw, latency, country)) in current {
        if checked.contains(url) {
            continue;
        }
        items.push(DispatchItem {
            raw: raw.clone(),
            url: Some(url.clone()),
            latency: *latency,
            country: country.clone(),
        });
    }
    (items, avg_sum.checked_div(avg_n))
}

async fn run_auto_cycle(app: &tauri::AppHandle, kind: AutoKind) {
    let auto: State<'_, AutoState> = app.state();
    if auto.inner.running.swap(true, Ordering::SeqCst) {
        emit_auto_status(app, kind, "skipped", 0, 0, None, None);
        return;
    }
    let _run_guard = BusyGuard(auto.inner.running.clone());

    let settings = auto.inner.settings.lock().await.clone();
    let disp: State<'_, DispatchState> = app.state();
    let (urls, checked) = {
        let pool = disp.inner.pool.lock().await;
        let urls: Vec<String> = pool.iter().map(|u| u.url.clone()).collect();
        let checked: HashSet<String> = urls.iter().cloned().collect();
        (urls, checked)
    };
    let input_total = urls.len();
    if input_total == 0 {
        emit_auto_status(app, kind, "empty", 0, 0, None, None);
        return;
    }

    let check: State<'_, CheckState> = app.state();
    if check.busy.swap(true, Ordering::SeqCst) {
        emit_auto_status(app, kind, "skipped", 0, input_total, None, None);
        return;
    }
    let _busy_guard = BusyGuard(check.busy.clone());
    check.cancel.store(false, Ordering::SeqCst);
    emit_auto_status(app, kind, "running", 0, input_total, None, None);

    let timeout = Duration::from_millis(settings.timeout_ms.clamp(500, 60_000));
    let flags = CheckFlags {
        geo: settings.with_geo,
        anon: settings.with_anonymity,
        tamper: settings.with_tamper,
        tls: settings.with_tls,
        precheck: settings.precheck,
        precheck_timeout: Duration::from_millis(settings.precheck_timeout_ms.clamp(300, 10_000)),
        judge: settings.judge_mode,
        ipapi_key: settings
            .ipapi_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(Arc::from),
    };
    let result = run_checks(
        urls,
        settings.test_url.clone(),
        timeout,
        settings.concurrency.clamp(1, 500),
        settings.repeats.clamp(1, 5),
        &settings.default_proto,
        flags,
        Vec::new(),
        check.cancel.clone(),
        None,
    )
    .await;
    let canceled = check.cancel.load(Ordering::Relaxed);

    match result {
        Err(e) => emit_auto_status(app, kind, "error", 0, input_total, None, Some(e)),
        Ok(results) => {
            if canceled {
                let alive = results.iter().filter(|r| r.alive).count();
                emit_auto_status(app, kind, "stopped", alive, results.len(), None, None);
                return;
            }
            let total = results.len();
            let current: HashMap<String, (String, Option<u64>, Option<String>)> = {
                let pool = disp.inner.pool.lock().await;
                pool.iter()
                    .map(|u| (u.url.clone(), (u.raw.clone(), u.latency, u.country.clone())))
                    .collect()
            };
            let (items, avg) = merge_auto_items(&results, &current, &checked, settings.max_ping_ms);
            let kept = store_pool(&disp.inner, items).await;
            let pool = pool_items(&disp.inner).await;
            let _ = app.emit(
                "auto://pool",
                AutoPoolPayload {
                    items: pool.clone(),
                },
            );
            // The check and pool update are done; only webhook delivery
            // remains, and it must not keep a manual check blocked.
            drop(_busy_guard);
            emit_auto_status(app, kind, "done", kept, total, avg, None);

            if matches!(kind, AutoKind::Auto) && !settings.webhook_urls.is_empty() {
                let webhook_items: Vec<DispatchItem> = pool
                    .iter()
                    .map(|p| DispatchItem {
                        raw: p.raw.clone(),
                        url: Some(p.url.clone()),
                        latency: p.latency,
                        country: p.country.clone(),
                    })
                    .collect();
                let results = send_webhooks_inner(
                    &settings.webhook_urls,
                    &settings.webhook_format,
                    settings.webhook_include_credentials,
                    Arc::new(webhook_items),
                )
                .await;
                let _ = app.emit("auto://webhook", AutoWebhookPayload { results });
            }
        }
    }
}

async fn auto_scheduler_loop(app: tauri::AppHandle) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let auto: State<'_, AutoState> = app.state();
        if auto.inner.running.load(Ordering::SeqCst) {
            continue;
        }
        let now = epoch_secs();
        let settings = auto.inner.settings.lock().await.clone();
        let disp_next = auto.inner.disp_next.load(Ordering::SeqCst);
        let auto_next = auto.inner.auto_next.load(Ordering::SeqCst);
        let kind = if settings.disp_enabled && disp_next != 0 && now >= disp_next {
            Some(AutoKind::Dispatch)
        } else if settings.auto_enabled && auto_next != 0 && now >= auto_next {
            Some(AutoKind::Auto)
        } else {
            None
        };
        let Some(kind) = kind else {
            continue;
        };
        run_auto_cycle(&app, kind).await;
        let now = epoch_secs();
        let settings = auto.inner.settings.lock().await.clone();
        match kind {
            AutoKind::Dispatch => auto.inner.disp_next.store(
                if settings.disp_enabled {
                    now + settings.disp_interval_min.clamp(1, 1440) * 60
                } else {
                    0
                },
                Ordering::SeqCst,
            ),
            AutoKind::Auto => auto.inner.auto_next.store(
                if settings.auto_enabled {
                    now + settings.auto_interval_min.clamp(1, 1440) * 60
                } else {
                    0
                },
                Ordering::SeqCst,
            ),
        }
    }
}

fn sanitize_auto_settings(mut s: AutoSettings) -> AutoSettings {
    s.disp_interval_min = s.disp_interval_min.clamp(1, 1440);
    s.auto_interval_min = s.auto_interval_min.clamp(1, 1440);
    s.timeout_ms = s.timeout_ms.clamp(500, 60_000);
    s.concurrency = s.concurrency.clamp(1, 500);
    s.repeats = s.repeats.clamp(1, 5);
    s.precheck_timeout_ms = s.precheck_timeout_ms.clamp(300, 10_000);
    s.test_url = s.test_url.trim().to_string();
    if s.test_url.is_empty() {
        s.test_url = AutoSettings::default().test_url;
    }
    s.webhook_format = match s.webhook_format.as_str() {
        "json" => "json".to_string(),
        "csv" => "csv".to_string(),
        _ => "txt".to_string(),
    };
    s.webhook_urls = s
        .webhook_urls
        .into_iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .take(10)
        .collect();
    s.ipapi_key = s
        .ipapi_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    s
}

#[tauri::command]
async fn set_auto_config(
    settings: AutoSettings,
    state: State<'_, AutoState>,
) -> Result<(), String> {
    let s = sanitize_auto_settings(settings);

    let old = {
        let mut cur = state.inner.settings.lock().await;
        let old = cur.clone();
        *cur = s.clone();
        old
    };
    let now = epoch_secs();
    if s.disp_enabled {
        if !old.disp_enabled
            || old.disp_interval_min != s.disp_interval_min
            || state.inner.disp_next.load(Ordering::SeqCst) == 0
        {
            state
                .inner
                .disp_next
                .store(now + s.disp_interval_min * 60, Ordering::SeqCst);
        }
    } else {
        state.inner.disp_next.store(0, Ordering::SeqCst);
    }
    if s.auto_enabled {
        if !old.auto_enabled
            || old.auto_interval_min != s.auto_interval_min
            || state.inner.auto_next.load(Ordering::SeqCst) == 0
        {
            state
                .inner
                .auto_next
                .store(now + s.auto_interval_min * 60, Ordering::SeqCst);
        }
    } else {
        state.inner.auto_next.store(0, Ordering::SeqCst);
    }
    Ok(())
}

#[tauri::command]
async fn run_auto_now(app: tauri::AppHandle, schedule: String) -> Result<(), String> {
    let kind = if schedule == "disp" {
        AutoKind::Dispatch
    } else {
        AutoKind::Auto
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        run_auto_cycle(&app, kind).await;
    });
    Ok(())
}

#[cfg(windows)]
fn detect_install_kind() -> String {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    fn uninstall_kind(root: RegKey, path: &str) -> Option<&'static str> {
        let key = root.open_subkey_with_flags(path, KEY_READ).ok()?;
        for name in key.enum_keys().flatten() {
            let Ok(sub) = key.open_subkey(&name) else {
                continue;
            };
            let Ok(display) = sub.get_value::<String, _>("DisplayName") else {
                continue;
            };
            if !display.to_ascii_lowercase().contains("proxpulse") {
                continue;
            }
            let uninstall: String = sub.get_value("UninstallString").unwrap_or_default();
            let low = uninstall.to_ascii_lowercase();
            if low.contains("msiexec") {
                return Some("msi");
            }
            if low.contains("uninstall") {
                return Some("nsis");
            }
        }
        None
    }

    let path = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    if let Some(kind) = uninstall_kind(RegKey::predef(HKEY_LOCAL_MACHINE), path)
        .or_else(|| uninstall_kind(RegKey::predef(HKEY_CURRENT_USER), path))
    {
        return kind.to_string();
    }
    if let Ok(exe) = std::env::current_exe() {
        let exe = exe.to_string_lossy().to_ascii_lowercase();
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let base = format!("{}\\programs", local.to_ascii_lowercase());
            if exe.starts_with(&base) {
                return "nsis".to_string();
            }
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Ok(base) = std::env::var(var) {
                if exe.starts_with(&base.to_ascii_lowercase()) {
                    return "msi".to_string();
                }
            }
        }
    }
    "portable".to_string()
}

#[cfg(target_os = "linux")]
fn detect_install_kind() -> String {
    if std::env::var_os("APPIMAGE").is_some() {
        "auto".to_string()
    } else {
        "package".to_string()
    }
}

#[cfg(all(not(windows), not(target_os = "linux")))]
fn detect_install_kind() -> String {
    "auto".to_string()
}

#[tauri::command]
fn install_kind() -> String {
    detect_install_kind()
}

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || (o[0] == 100 && (o[1] & 0xC0) == 64)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (o[1] & 0xFE) == 18)
                || o[0] >= 240
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xFE00) == 0xFC00
                || (v6.segments()[0] & 0xFFC0) == 0xFE80
            {
                return true;
            }
            v6.to_ipv4_mapped()
                .map(|v4| is_disallowed_ip(IpAddr::V4(v4)))
                .unwrap_or(false)
        }
    }
}

fn is_disallowed_url_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return true;
    };
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return is_disallowed_ip(ip);
    }
    let lower = bare.to_ascii_lowercase();
    lower == "localhost" || lower.ends_with(".localhost") || lower.ends_with(".local")
}

async fn build_import_client(url: &reqwest::Url) -> Result<reqwest::Client, String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("bad url".to_string());
    }
    if is_disallowed_url_host(url) {
        return Err("private address".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "bad url".to_string())?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = url.port_or_known_default().unwrap_or(80);
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .use_rustls_tls()
        .user_agent(concat!("ProxPulse/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none());
    if host.parse::<IpAddr>().is_err() {
        let mut chosen: Option<SocketAddr> = None;
        let addrs = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| short_error(&e.to_string()))?;
        for addr in addrs {
            if is_disallowed_ip(addr.ip()) {
                return Err("private address".to_string());
            }
            if chosen.is_none() {
                chosen = Some(addr);
            }
        }
        let addr = chosen.ok_or_else(|| "no address".to_string())?;
        builder = builder.resolve(&host, addr);
    }
    builder.build().map_err(|e| e.to_string())
}

#[tauri::command]
async fn fetch_url_text(url: String) -> Result<String, String> {
    let mut current = reqwest::Url::parse(url.trim()).map_err(|_| "bad url".to_string())?;
    for _ in 0..6 {
        let client = build_import_client(&current).await?;
        let r = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| short_error(&e.to_string()))?;
        if r.status().is_redirection() {
            let location = r
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "bad redirect".to_string())?;
            current = current
                .join(location)
                .map_err(|_| "bad redirect".to_string())?;
            continue;
        }
        if !r.status().is_success() {
            return Err(format!("http {}", r.status().as_u16()));
        }
        return read_body_capped(r, BODY_LIMIT_IMPORT).await;
    }
    Err("too many redirects".to_string())
}

#[derive(Serialize)]
struct UpdateInfo {
    available: bool,
    version: Option<String>,
    current: String,
    notes: Option<String>,
}

#[tauri::command]
async fn check_app_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;
    let current = app.package_info().version.to_string();
    let updater = app.updater_builder().build().map_err(|e| e.to_string())?;
    match updater.check().await.map_err(|e| e.to_string())? {
        Some(u) => Ok(UpdateInfo {
            available: true,
            version: Some(u.version.clone()),
            current,
            notes: u.body.clone(),
        }),
        None => Ok(UpdateInfo {
            available: false,
            version: None,
            current,
            notes: None,
        }),
    }
}

#[tauri::command]
async fn install_app_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater_builder().build().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("no update".to_string());
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn hide_to_tray(app: tauri::AppHandle) -> Result<(), String> {
    if app.tray_by_id("main").is_none() {
        return Err("tray is unavailable".to_string());
    }
    app.get_webview_window("main")
        .ok_or_else(|| "no window".to_string())
        .and_then(|w| w.hide().map_err(|e| e.to_string()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Keep a single instance: two processes would share localStorage and
        // could send duplicate webhooks / run duplicate auto-checks.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(DispatchState::default())
        .manage(CheckState::default())
        .manage(TrayState::default())
        .manage(AutoState::default())
        .invoke_handler(tauri::generate_handler![
            check_proxies,
            check_busy,
            cancel_check,
            check_direct,
            export_text_file,
            install_kind,
            fetch_url_text,
            set_dispatch_pool,
            get_dispatch_pool,
            start_local_proxy,
            stop_local_proxy,
            local_proxy_status,
            send_webhook,
            send_webhooks,
            set_auto_config,
            run_auto_now,
            hide_to_tray,
            app_version,
            check_app_update,
            install_app_update,
            update_tray_status
        ])
        .setup(|app| {
            if let Some(icon) = app.default_window_icon().cloned() {
                let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
                let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
                let dispatch = MenuItem::with_id(
                    app,
                    "dispatch_toggle",
                    "Dispatcher: OFF",
                    true,
                    None::<&str>,
                )?;
                let next_disp =
                    MenuItem::with_id(app, "next_disp", "Next dispatch: off", false, None::<&str>)?;
                let next_auto =
                    MenuItem::with_id(app, "next_auto", "Next auto: off", false, None::<&str>)?;
                let webhook =
                    MenuItem::with_id(app, "webhook_info", "Webhook: off", false, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let sep1 = PredefinedMenuItem::separator(app)?;
                let sep2 = PredefinedMenuItem::separator(app)?;
                let menu = Menu::with_items(
                    app,
                    &[
                        &show, &hide, &sep1, &dispatch, &next_disp, &next_auto, &webhook, &sep2,
                        &quit,
                    ],
                )?;
                TrayIconBuilder::with_id("main")
                    .icon(icon)
                    .tooltip("ProxPulse")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "hide" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.hide();
                            }
                        }
                        "dispatch_toggle" => {
                            let ah = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let disp: State<DispatchState> = ah.state();
                                if disp.inner.running.load(Ordering::SeqCst) {
                                    stop_proxy_inner(&disp.inner).await;
                                } else {
                                    let port = *disp.inner.port.lock().await;
                                    let mode = disp.inner.mode.lock().await.clone();
                                    let listener = disp.inner.listener.lock().await.clone();
                                    if start_proxy_inner(&disp.inner, port, mode, listener)
                                        .await
                                        .is_err()
                                    {
                                        *disp.inner.last_error.lock().await = Some(
                                            "toggle failed: pool empty or ports busy".to_string(),
                                        );
                                    }
                                }
                                refresh_tray_menu(&ah).await;
                            });
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            if let Some(w) = app.get_webview_window("main") {
                                if w.is_visible().unwrap_or(true) {
                                    let _ = w.hide();
                                } else {
                                    let _ = w.show();
                                    let _ = w.set_focus();
                                }
                            }
                        }
                    })
                    .build(app)?;
            }
            // Scheduling and webhooks run in Rust, so hiding the window or
            // a throttled WebView does not stop scheduled checks.
            let auto_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                auto_scheduler_loop(auto_handle).await;
            });
            let tray_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    refresh_tray_menu(&tray_handle).await;
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(raw: &str, def: &str) -> Option<(String, String)> {
        normalize_proxy(raw, def)
    }

    #[test]
    fn tls_config_builds() {
        // must not panic: with several rustls providers compiled in,
        // auto-detect would blow up (see CryptoProvider docs)
        let _ = tls_client_config();
    }

    #[test]
    fn basic_formats() {
        assert_eq!(
            norm("1.2.3.4:8080", "http"),
            Some(("http://1.2.3.4:8080".into(), "HTTP".into()))
        );
        assert_eq!(
            norm("user:pass@1.2.3.4:8080", "http"),
            Some(("http://user:pass@1.2.3.4:8080".into(), "HTTP".into()))
        );
        assert_eq!(
            norm("socks5://1.2.3.4:1080", "http"),
            Some(("socks5://1.2.3.4:1080".into(), "SOCKS5".into()))
        );
        assert_eq!(
            norm("SOCKS5://1.2.3.4:1080", "http"),
            Some(("socks5://1.2.3.4:1080".into(), "SOCKS5".into()))
        );
        assert_eq!(norm("bad-line", "http"), None);
        assert_eq!(norm("# comment", "http"), None);
    }

    #[test]
    fn seller_and_reverse_formats() {
        assert_eq!(
            norm("1.2.3.4:8080:u:p", "http"),
            Some(("http://u:p@1.2.3.4:8080".into(), "HTTP".into()))
        );
        assert_eq!(
            norm("u:p:1.2.3.4:8080", "http"),
            Some(("http://u:p@1.2.3.4:8080".into(), "HTTP".into()))
        );
        assert_eq!(
            norm("1.2.3.4:8080:socks5", "http"),
            Some(("socks5://1.2.3.4:8080".into(), "SOCKS5".into()))
        );
        assert_eq!(
            norm("1.2.3.4,8080,u,p", "http"),
            Some(("http://u:p@1.2.3.4:8080".into(), "HTTP".into()))
        );
    }

    #[test]
    fn numeric_passwords_keep_column_order() {
        let expected = Some(("http://user:1234@1.2.3.4:9090".into(), "HTTP".into()));
        assert_eq!(norm("user:1234:1.2.3.4:9090", "http"), expected);
        assert_eq!(norm("1.2.3.4:9090:user:1234", "http"), expected);
        assert_eq!(norm("user,1234,1.2.3.4,9090", "http"), expected);
        assert_eq!(norm("1.2.3.4,9090,user,1234", "http"), expected);
        assert_eq!(
            norm("user,1234,proxy.example.com,9090", "http"),
            Some((
                "http://user:1234@proxy.example.com:9090".into(),
                "HTTP".into()
            ))
        );
        assert_eq!(
            norm("proxy.example.com,9090,user,1234", "http"),
            Some((
                "http://user:1234@proxy.example.com:9090".into(),
                "HTTP".into()
            ))
        );
    }

    #[test]
    fn csv_ipv6_is_bracketed() {
        assert_eq!(
            norm("2001:db8::1,8080", "http"),
            Some(("http://[2001:db8::1]:8080".into(), "HTTP".into()))
        );
        assert_eq!(
            norm("[2001:db8::1],8080", "http"),
            Some(("http://[2001:db8::1]:8080".into(), "HTTP".into()))
        );
        assert_eq!(
            norm("[::1]:1080:socks5", "http"),
            Some(("socks5://[::1]:1080".into(), "SOCKS5".into()))
        );
    }

    #[test]
    fn empty_scheme_is_rejected() {
        assert_eq!(norm("://host:80", "http"), None);
    }

    #[test]
    fn dual_mode_expands() {
        let c = normalize_candidates("1.2.3.4:8080", "http+socks5");
        assert_eq!(c.len(), 2);
        assert!(c[0].0.starts_with("http://"));
        assert!(c[1].0.starts_with("socks5://"));
        // explicit scheme stays single
        let c2 = normalize_candidates("socks5://1.2.3.4:1080", "http+socks5");
        assert_eq!(c2.len(), 1);
    }

    #[test]
    fn host_port_extraction() {
        assert_eq!(
            host_port_of_url("http://u:p@1.2.3.4:8080"),
            Some(("1.2.3.4".into(), 8080))
        );
        assert_eq!(
            host_port_of_url("socks5://[::1]:1080"),
            Some(("::1".into(), 1080))
        );
        assert_eq!(host_port_of_url("http://nonsense"), None);
    }

    #[test]
    fn target_parsing() {
        assert_eq!(
            parse_target("CONNECT", "example.com:443"),
            Some(("example.com".into(), 443))
        );
        assert_eq!(
            parse_target("CONNECT", "[::1]:443"),
            Some(("::1".into(), 443))
        );
        assert_eq!(
            parse_target("GET", "http://example.com:8080/path?q=1"),
            Some(("example.com".into(), 8080))
        );
        assert_eq!(
            parse_target("GET", "http://example.com/path"),
            Some(("example.com".into(), 80))
        );
        assert_eq!(
            parse_target("GET", "HTTP://example.com:8080/path"),
            Some(("example.com".into(), 8080))
        );
        assert_eq!(
            parse_target("GET", "http://example.com?x=1"),
            Some(("example.com".into(), 80))
        );
        assert_eq!(
            parse_target("GET", "http://example.com#frag"),
            Some(("example.com".into(), 80))
        );
        assert_eq!(parse_target("GET", "/relative"), None);
    }

    #[test]
    fn order_modes() {
        let rr = AtomicUsize::new(0);
        // set_dispatch_pool keeps the pool sorted by latency.
        assert_eq!(pick_order(3, "fastest", &rr, 3), vec![0]);
        assert_eq!(pick_order(3, "failover", &rr, 2), vec![0, 1]);
        assert_eq!(pick_order(0, "fastest", &rr, 3), Vec::<usize>::new());
        let a = pick_order(3, "round-robin", &rr, 3);
        let b = pick_order(3, "round-robin", &rr, 3);
        assert_eq!(a, vec![0, 1, 2]);
        assert_eq!(b, vec![1, 2, 0]);
    }

    #[test]
    fn ipv6_authority_brackets() {
        assert_eq!(host_for_connect("::1"), "[::1]");
        assert_eq!(host_for_connect("[::1]"), "[::1]");
        assert_eq!(host_for_connect("1.2.3.4"), "1.2.3.4");
        assert_eq!(host_for_connect("example.com"), "example.com");
    }

    #[test]
    fn ip_validation_is_strict() {
        assert!(looks_like_ip("1.2.3.4"));
        assert!(looks_like_ip("2001:db8::1"));
        assert!(!looks_like_ip("abc.def"));
        assert!(!looks_like_ip("1.2.3.4:8080"));
        assert!(!looks_like_ip("<html>"));
    }

    #[test]
    fn csv_escaping() {
        assert_eq!(csv_cell("1.2.3.4:80"), "1.2.3.4:80");
        assert_eq!(csv_cell("user,pass"), "\"user,pass\"");
        assert_eq!(csv_cell("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_cell("line\nbreak"), "\"line\nbreak\"");
    }

    #[test]
    fn short_error_mapping() {
        // reqwest URL prefix is stripped, the cause is mapped
        assert_eq!(
            short_error("error sending request for url (https://proxycheck.lmtunnel.com/judge?direct_ip=1.2.3.4): operation timed out"),
            "timeout"
        );
        assert_eq!(
            short_error(
                "error sending request for url (https://example.com/): SOCKS5 connection refused"
            ),
            "connection refused"
        );
        assert_eq!(
            short_error(
                "error sending request for url (https://example.com/): socks connect failed"
            ),
            "socks error"
        );
        assert_eq!(
            short_error("error sending request for url (https://example.com/)"),
            "error sending request for url (https://example.com/)"
        );
        assert!(short_error(&"x".repeat(500)).len() <= 160);
    }

    #[test]
    fn judge_direct_url_rules() {
        assert_eq!(
            judge_direct_url("https://proxycheck.lmtunnel.com/judge"),
            Some("https://proxycheck.lmtunnel.com/judge".to_string())
        );
        assert_eq!(
            judge_direct_url("https://proxycheck.lmtunnel.com/generate_204/"),
            Some("https://proxycheck.lmtunnel.com/judge".to_string())
        );
        assert_eq!(
            judge_direct_url("https://proxycheck.lmtunnel.com/svc"),
            Some("https://proxycheck.lmtunnel.com/svc".to_string())
        );
        assert_eq!(judge_direct_url(""), None);
    }

    #[test]
    fn https_proxy_scheme_is_preserved() {
        assert_eq!(
            norm("https://1.2.3.4:8443", "http"),
            Some(("https://1.2.3.4:8443".into(), "HTTPS".into()))
        );
        assert_eq!(
            parse_upstream_url("https://1.2.3.4:8443").map(|v| v.0),
            Some("https".to_string())
        );
        assert!(build_client(Some("https://1.2.3.4:8443"), Duration::from_secs(1), false).is_ok());
    }

    #[test]
    fn socks4_ipv6_policy() {
        assert!(build_socks4_request("2001:db8::1", 443, None, false).is_err());
        let req = build_socks4_request("2001:db8::1", 443, None, true).unwrap();
        assert_eq!(&req[..8], &[0x04, 0x01, 0x01, 0xbb, 0, 0, 0, 1]);
        assert!(req.ends_with(b"2001:db8::1\0"));

        let v4 = build_socks4_request("1.2.3.4", 8080, Some("u"), false).unwrap();
        assert_eq!(&v4[..9], &[0x04, 0x01, 0x1f, 0x90, 1, 2, 3, 4, b'u']);
        assert_eq!(v4.last(), Some(&0));
    }

    fn mk_upstream(host: &str, port: u16) -> Upstream {
        Upstream {
            raw: String::new(),
            url: format!("http://{host}:{port}"),
            scheme: "http".into(),
            host: host.into(),
            port,
            user: None,
            pass: None,
            latency: None,
            country: None,
        }
    }

    #[test]
    fn self_loop_detection() {
        assert!(is_self_upstream(&mk_upstream("127.0.0.1", 1080), 1080));
        assert!(is_self_upstream(&mk_upstream("127.0.0.2", 1080), 1080));
        assert!(is_self_upstream(&mk_upstream("localhost", 1080), 1080));
        assert!(is_self_upstream(&mk_upstream("[::1]", 1080), 1080));
        assert!(is_self_upstream(&mk_upstream("0.0.0.0", 1080), 1080));
        assert!(is_self_upstream(&mk_upstream("::", 1080), 1080));
        assert!(!is_self_upstream(&mk_upstream("127.0.0.1", 1081), 1080));
        assert!(!is_self_upstream(&mk_upstream("1.2.3.4", 1080), 1080));
        assert!(is_self_literal("localhost."));
        assert!(is_self_literal("[::1]"));
        assert!(!is_self_literal("example.com"));
    }

    #[tokio::test]
    async fn self_loop_hostname_check_is_port_scoped() {
        let cache = Mutex::new(HashMap::new());
        let up = mk_upstream("localhost", 1080);
        assert!(is_self_upstream_async(&up, 1080, &cache).await);
        assert!(!is_self_upstream_async(&up, 1081, &cache).await);
    }

    #[tokio::test]
    async fn abort_connections_drains_active_tasks() {
        let st = DispatchState::default();
        {
            let mut conns = st.inner.connections.lock().await;
            conns.spawn(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
            });
        }
        abort_connections(&st.inner).await;
        assert_eq!(st.inner.connections.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn url_import_rejects_loopback_before_connecting() {
        let err = fetch_url_text("http://127.0.0.1:9/list".to_string())
            .await
            .unwrap_err();
        assert_eq!(err, "private address");
    }
    #[test]
    fn import_urls_reject_private_targets() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_disallowed_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "2606:4700:4700::1111"] {
            assert!(!is_disallowed_ip(ip.parse().unwrap()), "{ip}");
        }
        for url in [
            "http://localhost/list",
            "http://foo.localhost/list",
            "http://printer.local/list",
            "http://[::1]/list",
            "http://169.254.169.254/latest",
        ] {
            assert!(
                is_disallowed_url_host(&reqwest::Url::parse(url).unwrap()),
                "{url}"
            );
        }
        assert!(!is_disallowed_url_host(
            &reqwest::Url::parse("https://raw.githubusercontent.com/a/b").unwrap()
        ));
    }
    #[tokio::test]
    async fn counting_io_tracks_written_bytes() {
        let (client, mut peer) = tokio::io::duplex(64);
        let written = Arc::new(AtomicU64::new(0));
        let mut counted = CountingIo::new(client, written.clone());
        counted.write_all(b"hello").await.unwrap();
        assert_eq!(written.load(Ordering::Relaxed), 5);
        let mut got = [0u8; 5];
        peer.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"hello");
    }

    #[test]
    fn utf8_truncation_is_safe() {
        let mut s = format!("a{}", "é".repeat(100));
        truncate_utf8(&mut s, 160);
        assert!(s.len() <= 160);
        assert!(s.is_char_boundary(s.len()));
        assert!(s.starts_with('a'));
        assert!(s.chars().skip(1).all(|c| c == 'é'));

        let mut short = "ok".to_string();
        truncate_utf8(&mut short, 160);
        assert_eq!(short, "ok");
    }

    #[test]
    fn short_error_handles_non_ascii() {
        let input = format!("a{}", "é".repeat(100));
        let out = short_error(&input);
        assert!(out.len() <= 160);
        assert!(out.is_char_boundary(out.len()));
        assert!(out.chars().all(|c| c == 'a' || c == 'é'));
    }

    #[test]
    fn cached_geo_is_used_without_exit_ip() {
        let entry = GeoCacheEntry {
            proxy: "1.2.3.4:8080".into(),
            ip: Some("5.6.7.8".into()),
            country: Some("Testland".into()),
            country_code: Some("TL".into()),
            city: Some("Testville".into()),
            isp: Some("ISP".into()),
            org: Some("ORG".into()),
            asn: Some("AS1".into()),
            ip_type: Some("residential/isp".into()),
        };
        let mut res = dead_result("1.2.3.4:8080".into(), "HTTP".into(), 0, String::new());
        res.alive = true;
        assert!(apply_cached_geo(&mut res, &entry));
        assert_eq!(res.ip.as_deref(), Some("5.6.7.8"));
        assert_eq!(res.country.as_deref(), Some("Testland"));
        assert_eq!(res.city.as_deref(), Some("Testville"));

        let mut other = dead_result("1.2.3.4:8080".into(), "HTTP".into(), 0, String::new());
        other.ip = Some("9.9.9.9".into());
        assert!(!apply_cached_geo(&mut other, &entry));
        assert!(other.country.is_none());
    }

    #[test]
    fn ipapi_geo_url_modes() {
        let free = geo_url(None);
        assert!(free.starts_with("http://ip-api.com/json/?fields="));
        assert!(free.contains("status,message,query"));
        assert_eq!(geo_url(Some("   ")), free);

        let pro = geo_url(Some("abc 123"));
        assert!(pro.starts_with("https://pro.ip-api.com/json/?"));
        let u = reqwest::Url::parse(&pro).unwrap();
        let pairs: HashMap<_, _> = u.query_pairs().into_owned().collect();
        assert_eq!(pairs.get("key").map(String::as_str), Some("abc 123"));
        assert_eq!(pairs.get("fields").map(String::as_str), Some(GEO_FIELDS));
    }

    #[test]
    fn candidate_budget_covers_deep_tls() {
        let timeout = Duration::from_secs(5);
        let flags = CheckFlags {
            geo: true,
            anon: true,
            tamper: true,
            tls: true,
            precheck: true,
            precheck_timeout: Duration::from_millis(1500),
            judge: false,
            ipapi_key: None,
        };
        // attempts (10s) + DNS/CONNECT/TCP/TLS (20s) + precheck (1.5s) + 5s margin
        assert_eq!(
            candidate_budget(timeout, 1, &flags),
            Duration::from_millis(36_500)
        );

        let no_tls = CheckFlags {
            tls: false,
            ..flags.clone()
        };
        assert_eq!(
            candidate_budget(timeout, 1, &no_tls),
            Duration::from_millis(21_500)
        );

        let judge = CheckFlags {
            judge: true,
            tls: true,
            ..flags
        };
        assert_eq!(
            candidate_budget(timeout, 1, &judge),
            Duration::from_millis(16_500)
        );
    }

    #[tokio::test]
    async fn socks_local_dns_modes() {
        let timeout = Duration::from_secs(2);
        let local = socks_target_host("socks5", "localhost", 80, timeout)
            .await
            .unwrap();
        assert!(local.parse::<IpAddr>().unwrap().is_loopback());

        let remote = socks_target_host("socks5h", "localhost", 80, timeout)
            .await
            .unwrap();
        assert_eq!(remote, "localhost");

        assert_eq!(
            socks_target_host("socks4", "127.0.0.1", 80, timeout)
                .await
                .unwrap(),
            "127.0.0.1"
        );
    }

    #[test]
    fn baseline_cache_ttl() {
        assert!(cache_fresh(Instant::now(), Duration::from_secs(1)));
        let old = Instant::now() - Duration::from_secs(2);
        assert!(!cache_fresh(old, Duration::from_secs(1)));
    }

    #[test]
    fn credentials_stripping() {
        assert_eq!(
            strip_credentials("http://user:pass@1.2.3.4:8080"),
            "http://1.2.3.4:8080"
        );
        assert_eq!(strip_credentials("socks5://u:p@h:1080"), "socks5://h:1080");
        assert_eq!(strip_credentials("user:pass@1.2.3.4:8080"), "1.2.3.4:8080");
        assert_eq!(strip_credentials("u:p@ss@host:80"), "host:80");
        assert_eq!(strip_credentials("1.2.3.4:8080"), "1.2.3.4:8080");
    }

    #[test]
    fn http_url_detection_is_case_insensitive() {
        assert!(is_http_url("http://example.com"));
        assert!(is_http_url("HTTPS://example.com"));
        assert!(is_http_url("  https://example.com  "));
        assert!(!is_http_url("ftp://example.com"));
        assert!(!is_http_url(""));
    }

    #[test]
    fn export_path_validation() {
        let ok = std::env::temp_dir().join("proxpulse-audit.txt");
        assert!(validate_export_path(&ok.to_string_lossy()).is_ok());
        assert!(validate_export_path("relative.txt").is_err());
        assert!(validate_export_path("C:\\temp\\x.exe").is_err());
        assert!(validate_export_path("bad\0path.txt").is_err());
        assert!(validate_export_path("").is_err());
    }

    #[test]
    fn forward_request_adds_proxy_auth_once() {
        let up = Upstream {
            raw: "http://u:p@proxy:8080".into(),
            url: "http://u:p@proxy:8080".into(),
            scheme: "http".into(),
            host: "proxy".into(),
            port: 8080,
            user: Some("u".into()),
            pass: Some("p".into()),
            latency: None,
            country: None,
        };
        let body = b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\nBODY";
        let hdr_end = body
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap();
        let out = prepare_forward_request(body, hdr_end, &up);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.matches("Proxy-Authorization: Basic ").count(), 1);
        assert!(text.contains("Basic dTpw"));
        assert!(text.ends_with("BODY"));

        let no_auth = Upstream {
            user: None,
            pass: None,
            ..up
        };
        assert_eq!(
            prepare_forward_request(body, hdr_end, &no_auth),
            body.to_vec()
        );
    }

    #[test]
    fn forward_request_replaces_client_proxy_auth() {
        let up = Upstream {
            raw: "http://u:p@proxy:8080".into(),
            url: "http://u:p@proxy:8080".into(),
            scheme: "http".into(),
            host: "proxy".into(),
            port: 8080,
            user: Some("u".into()),
            pass: Some("p".into()),
            latency: None,
            country: None,
        };
        let body = b"GET http://example.com/ HTTP/1.1\r\nProxy-Authorization: Basic old\r\n\r\n";
        let hdr_end = body
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap();
        let text = String::from_utf8(prepare_forward_request(body, hdr_end, &up)).unwrap();
        assert_eq!(text.matches("Proxy-Authorization").count(), 1);
        assert!(text.contains("Basic dTpw"));
        assert!(!text.contains("Basic old"));
    }

    #[test]
    fn forward_request_strips_client_auth_without_upstream_creds() {
        let up = Upstream {
            raw: "http://proxy:8080".into(),
            url: "http://proxy:8080".into(),
            scheme: "http".into(),
            host: "proxy".into(),
            port: 8080,
            user: None,
            pass: None,
            latency: None,
            country: None,
        };
        let body = b"GET http://example.com/ HTTP/1.1\r\nproxy-authorization: Basic old\r\nHost: example.com\r\n\r\nBODY";
        let hdr_end = body
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap();
        let out = strip_header(body, hdr_end, "proxy-authorization");
        let text = String::from_utf8(out.clone()).unwrap();
        assert!(!text.to_ascii_lowercase().contains("proxy-authorization"));
        assert!(text.ends_with("BODY"));
        assert_eq!(prepare_forward_request(body, hdr_end, &up), out);
    }

    #[test]
    fn strip_header_keeps_other_headers_and_body() {
        let body = b"GET / HTTP/1.1\r\nX-Test: 1\r\nProxy-Authorization: x\r\n\r\nP";
        let end = find_header_end(body).unwrap();
        let stripped = strip_header(body, end, "Proxy-Authorization");
        let text = String::from_utf8(stripped).unwrap();
        assert!(text.contains("X-Test: 1"));
        assert!(!text.contains("Proxy-Authorization"));
        assert!(text.ends_with("P"));
    }

    #[test]
    fn dispatch_item_preserves_normalized_scheme() {
        let (_, up) = upstream_from_item(
            "1.2.3.4:1080",
            Some("socks5://1.2.3.4:1080"),
            Some(42),
            Some("DE".into()),
        )
        .unwrap();
        assert_eq!(up.scheme, "socks5");
        assert_eq!(up.host, "1.2.3.4");
        assert_eq!(up.port, 1080);
        assert_eq!(up.raw, "1.2.3.4:1080");
        assert_eq!(up.url, "socks5://1.2.3.4:1080");
        assert_eq!(up.latency, Some(42));
        assert_eq!(up.country.as_deref(), Some("DE"));

        let (_, explicit) = upstream_from_item("socks5://1.2.3.4:1080", None, None, None).unwrap();
        assert_eq!(explicit.scheme, "socks5");
        assert_eq!(explicit.url, "socks5://1.2.3.4:1080");
    }

    fn alive_result(url: &str, latency: u64, country: &str) -> ProxyResult {
        let mut r = dead_result(url.into(), "HTTP".into(), 1, String::new());
        r.alive = true;
        r.url = url.into();
        r.latency_ms = Some(latency);
        r.country = Some(country.into());
        r.country_code = Some(country.into());
        r
    }

    #[test]
    fn auto_pool_merge_respects_deletions_and_additions() {
        let current = HashMap::from([
            (
                "http://a:80".to_string(),
                ("raw-a".to_string(), Some(100), Some("DE".to_string())),
            ),
            (
                "http://b:80".to_string(),
                ("raw-b".to_string(), Some(200), Some("NL".to_string())),
            ),
            (
                "http://c:80".to_string(),
                ("raw-c".to_string(), Some(300), Some("US".to_string())),
            ),
        ]);
        let checked = HashSet::from(["http://a:80".to_string(), "http://b:80".to_string()]);
        let results = vec![
            alive_result("http://a:80", 10, "FR"),
            alive_result("http://b:80", 999, "FR"),
        ];
        let (items, avg) = merge_auto_items(&results, &current, &checked, 100);
        let urls: Vec<&str> = items.iter().map(|i| i.url.as_deref().unwrap()).collect();
        assert_eq!(urls, vec!["http://a:80", "http://c:80"]);
        assert_eq!(items[0].raw, "raw-a");
        assert_eq!(items[0].latency, Some(10));
        assert_eq!(items[0].country.as_deref(), Some("FR"));
        assert_eq!(items[1].raw, "raw-c");
        assert_eq!(avg, Some((10 + 999) / 2));

        // A proxy deleted while the check was running is not resurrected.
        let current = HashMap::from([(
            "http://b:80".to_string(),
            ("raw-b".to_string(), Some(200), None),
        )]);
        let results = vec![alive_result("http://a:80", 10, "FR")];
        let (items, _) = merge_auto_items(&results, &current, &checked, 0);
        assert!(items.is_empty());
    }

    #[test]
    fn auto_settings_are_sanitized() {
        let s = AutoSettings {
            disp_interval_min: 0,
            auto_interval_min: 9999,
            timeout_ms: 1,
            concurrency: 0,
            repeats: 99,
            precheck_timeout_ms: 1,
            test_url: "   ".into(),
            webhook_format: "xml".into(),
            webhook_urls: vec![" ".into(), " https://hook ".into()],
            ipapi_key: Some("   ".into()),
            ..AutoSettings::default()
        };
        let s = sanitize_auto_settings(s);
        assert_eq!(s.disp_interval_min, 1);
        assert_eq!(s.auto_interval_min, 1440);
        assert_eq!(s.timeout_ms, 500);
        assert_eq!(s.concurrency, 1);
        assert_eq!(s.repeats, 5);
        assert_eq!(s.precheck_timeout_ms, 300);
        assert_eq!(s.test_url, AutoSettings::default().test_url);
        assert_eq!(s.webhook_format, "txt");
        assert_eq!(s.webhook_urls, vec!["https://hook"]);
        assert!(s.ipapi_key.is_none());
    }

    #[tokio::test]
    async fn store_pool_keeps_normalized_url_and_country() {
        let st = DispatchState::default();
        let items = vec![
            DispatchItem {
                raw: "u:p@1.2.3.4:8080".into(),
                url: Some("socks5://u:p@1.2.3.4:8080".into()),
                latency: Some(42),
                country: Some("DE".into()),
            },
            DispatchItem {
                raw: "1.2.3.4:8080".into(),
                url: Some("socks5://u:p@1.2.3.4:8080".into()),
                latency: Some(10),
                country: Some("NL".into()),
            },
        ];
        assert_eq!(store_pool(&st.inner, items).await, 1);
        let pool = pool_items(&st.inner).await;
        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0].url, "socks5://u:p@1.2.3.4:8080");
        assert_eq!(pool[0].raw, "u:p@1.2.3.4:8080");
        assert_eq!(pool[0].latency, Some(42));
        assert_eq!(pool[0].country.as_deref(), Some("DE"));
    }

    #[test]
    fn webhook_payload_redacts_credentials_by_default() {
        let items = vec![
            DispatchItem {
                raw: "u:p@host:8080".to_string(),
                url: Some("http://u:p@host:8080".to_string()),
                latency: Some(10),
                country: None,
            },
            DispatchItem {
                raw: "socks5://host:1080".to_string(),
                url: None,
                latency: None,
                country: None,
            },
        ];
        assert_eq!(
            webhook_proxies(&items, false),
            vec!["http://host:8080", "socks5://host:1080"]
        );
        assert_eq!(
            webhook_proxies(&items, true),
            vec!["http://u:p@host:8080", "socks5://host:1080"]
        );
    }

    #[test]
    fn certificate_errors_are_classified() {
        assert!(is_cert_error(&Error::other(
            "invalid peer certificate: UnknownIssuer"
        )));
        assert!(is_cert_error(&Error::other("certificate verify failed")));
        assert!(!is_cert_error(&Error::other("connection refused")));
        assert!(!is_cert_error(&Error::new(
            ErrorKind::TimedOut,
            "tls handshake timeout"
        )));
    }

    #[test]
    fn tray_countdown_is_localized() {
        assert_eq!(fmt_countdown(None, true), "выкл");
        assert_eq!(fmt_countdown(None, false), "off");
        assert_eq!(fmt_countdown(Some(59), true), "скоро");
        assert_eq!(fmt_countdown(Some(59), false), "soon");
        assert_eq!(fmt_countdown(Some(60), true), "1m");
        assert_eq!(fmt_countdown(Some(3660), false), "1h 01m");
    }

    #[test]
    fn csv_formula_injection_guard() {
        assert_eq!(csv_cell("=1+1"), "'=1+1");
        assert_eq!(csv_cell("+1"), "'+1");
        assert_eq!(csv_cell("-1"), "'-1");
        assert_eq!(csv_cell("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(csv_cell("=a,b"), "\"'=a,b\"");
    }

    #[tokio::test]
    async fn http_connect_preserves_tunnel_bytes() {
        let mock = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = mock.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = mock.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 256];
            loop {
                let n = s.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = s
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\nEXTRA")
                .await;
        });
        let client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut stream = http_connect(
            ProxyStream::Plain(client),
            "example.com",
            80,
            None,
            None,
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        let mut extra = [0u8; 5];
        tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut extra))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&extra, b"EXTRA");
    }

    #[tokio::test]
    async fn webhook_redirects_are_not_followed() {
        let redirector = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let redirect_port = redirector.local_addr().unwrap().port();
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_port = target.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = redirector.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:{target_port}/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = s.write_all(response.as_bytes()).await;
            let _ = s.shutdown().await;
        });

        let items = vec![DispatchItem {
            raw: "u:p@host:8080".to_string(),
            url: Some("http://u:p@host:8080".to_string()),
            latency: Some(1),
            country: None,
        }];
        let result = send_webhook_one(
            &format!("http://127.0.0.1:{redirect_port}/hook"),
            "txt",
            false,
            &items,
        )
        .await
        .unwrap();
        assert!(result.starts_with("http 307"));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), target.accept())
                .await
                .is_err(),
            "webhook client must not follow redirects"
        );
    }

    #[tokio::test]
    async fn socks5_dispatcher_tunnels_via_http_proxy() {
        let mock = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let mock_port = mock.local_addr().unwrap().port();
        let (connect_tx, connect_rx) = tokio::sync::oneshot::channel::<String>();
        let (ping_tx, ping_rx) = tokio::sync::oneshot::channel::<Vec<u8>>();
        tokio::spawn(async move {
            let (mut s, _) = mock.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                let n = s.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = connect_tx.send(String::from_utf8_lossy(&buf).to_string());
            let _ = s
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await;
            let mut ping = [0u8; 4];
            let _ = s.read_exact(&mut ping).await;
            let _ = ping_tx.send(ping.to_vec());
            let _ = s.write_all(b"pong").await;
            let _ = s.shutdown().await;
        });

        let proxy = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let proxy_port = proxy.local_addr().unwrap().port();
        let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
        let (server, _) = proxy.accept().await.unwrap();
        let st = DispatchState::default();
        *st.inner.port.lock().await = proxy_port;
        *st.inner.mode.lock().await = "failover".to_string();
        *st.inner.pool.lock().await = vec![Upstream {
            raw: format!("http://127.0.0.1:{mock_port}"),
            url: format!("http://127.0.0.1:{mock_port}"),
            scheme: "http".to_string(),
            host: "127.0.0.1".to_string(),
            port: mock_port,
            user: None,
            pass: None,
            latency: None,
            country: None,
        }];
        let inner = st.inner.clone();
        let h = tokio::spawn(async move {
            handle_socks5(server, inner).await;
        });

        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [0x05, 0x00]);

        let mut connect = vec![0x05, 0x01, 0x00, 0x03, 11];
        connect.extend_from_slice(b"example.com");
        connect.extend_from_slice(&80u16.to_be_bytes());
        client.write_all(&connect).await.unwrap();
        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply[..2], &[0x05, 0x00]);

        client.write_all(b"ping").await.unwrap();
        let mut pong = [0u8; 4];
        tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut pong))
            .await
            .expect("pong timeout")
            .unwrap();
        assert_eq!(&pong, b"pong");
        let _ = client.shutdown().await;
        tokio::time::timeout(Duration::from_secs(5), h)
            .await
            .expect("handle_socks5 should finish")
            .unwrap();

        let captured = tokio::time::timeout(Duration::from_secs(2), connect_rx)
            .await
            .unwrap()
            .unwrap();
        assert!(captured.starts_with("CONNECT example.com:80 "));
        let ping = tokio::time::timeout(Duration::from_secs(2), ping_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ping, b"ping");
    }

    #[tokio::test]
    async fn http_forward_uses_absolute_form_and_proxy_auth() {
        let mock = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let mock_port = mock.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        tokio::spawn(async move {
            let (mut s, _) = mock.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                let n = s.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&buf).to_string());
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await;
            let _ = s.shutdown().await;
        });

        let proxy = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let proxy_port = proxy.local_addr().unwrap().port();
        let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
        let (server, _) = proxy.accept().await.unwrap();

        let st = DispatchState::default();
        *st.inner.port.lock().await = proxy_port;
        *st.inner.mode.lock().await = "failover".to_string();
        *st.inner.pool.lock().await = vec![Upstream {
            raw: format!("http://u:p@127.0.0.1:{mock_port}"),
            url: format!("http://u:p@127.0.0.1:{mock_port}"),
            scheme: "http".to_string(),
            host: "127.0.0.1".to_string(),
            port: mock_port,
            user: Some("u".to_string()),
            pass: Some("p".to_string()),
            latency: None,
            country: None,
        }];
        let inner = st.inner.clone();
        let h = tokio::spawn(async move {
            handle_http(server, inner).await;
        });

        client
            .write_all(b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n")
            .await
            .unwrap();
        // Half-close the request side so copy_bidirectional can finish once
        // the mock proxy has sent its response and closed the upstream.
        let _ = client.shutdown().await;
        let mut resp = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut resp)).await;
        tokio::time::timeout(Duration::from_secs(5), h)
            .await
            .expect("handle_http should finish")
            .unwrap();

        let captured = tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .unwrap()
            .unwrap();
        assert!(captured.starts_with("GET http://example.com/ HTTP/1.1"));
        assert!(captured.contains("Proxy-Authorization: Basic dTpw"));
        assert!(String::from_utf8_lossy(&resp).contains("200 OK"));
    }
}
