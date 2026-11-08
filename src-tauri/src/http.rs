use crate::tls::tls_client_config;
use crate::{
    body_hash, GeoCacheEntry, HttpbinHeaders, HttpbinIp, IpApi, ProxyResult, GEO_COOLDOWN,
    GEO_COOLDOWN_UNTIL,
};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

// ================= http helpers =================

/// Truncate to at most `max` bytes without splitting a UTF-8 code point.
/// `String::truncate` panics when the byte index is not a char boundary.
pub(crate) fn truncate_utf8(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// A short, URL-free description of a failed request. reqwest's own text is
/// "error sending request for url (URL)": it names the URL, which can carry an
/// API key, a bot token or the user's IP, and leaves the cause out, which is
/// only reachable through `source()`.
pub(crate) fn request_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "timeout".to_string();
    }
    let mut cause = String::new();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        if let Some(label) = s.downcast_ref::<std::io::Error>().and_then(io_label) {
            return label.to_string();
        }
        if !cause.is_empty() {
            cause.push_str(": ");
        }
        cause.push_str(&s.to_string());
        source = s.source();
    }
    if !cause.is_empty() {
        return short_error(&cause);
    }
    if e.is_redirect() {
        "redirect error"
    } else if e.is_connect() {
        "connect error"
    } else if e.is_body() || e.is_decode() {
        "incomplete response"
    } else {
        "request error"
    }
    .to_string()
}

/// A socket error by its kind, which does not depend on the OS language
/// (Windows says "actively refused", localized, for a refused connection).
pub(crate) fn io_error(e: &std::io::Error) -> String {
    io_label(e).map_or_else(|| short_error(&e.to_string()), str::to_string)
}

fn io_label(e: &std::io::Error) -> Option<&'static str> {
    use std::io::ErrorKind::*;
    Some(match e.kind() {
        ConnectionRefused => "connection refused",
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof => "connection reset",
        TimedOut => "timeout",
        HostUnreachable | NetworkUnreachable => "network unreachable",
        _ => return None,
    })
}

/// Maps an error text to a short label. Not for reqwest errors, whose text
/// holds the URL and not the cause: those go through [`request_error`].
pub(crate) fn short_error(e: &str) -> String {
    let mut s = e.replace('\n', " ");
    let low = s.to_lowercase();
    if low.contains("timed out")
        || low.contains("timeout")
        || low.contains("deadline")
        || low.contains("elapsed")
    {
        return "timeout".to_string();
    }
    if low.contains("proxy authentication")
        || low.contains("proxy authorization")
        || low.contains("407")
    {
        return "proxy auth required".to_string();
    }
    if low.contains("connection refused") {
        return "connection refused".to_string();
    }
    if low.contains("failed to lookup") || low.contains("dns") || low.contains("could not resolve")
    {
        return "dns error".to_string();
    }
    if low.contains("no route to host") || low.contains("network unreachable") {
        return "network unreachable".to_string();
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
    // "tunnel error: unsuccessful": the proxy refused the CONNECT.
    if low.contains("proxy") || low.contains("tunnel") {
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

/// The proxy URL as reqwest should get it, or an error for a scheme it cannot
/// use. reqwest accepts any URL as a proxy, but hyper-util silently skips a
/// scheme it does not know and the request then goes out directly, from the
/// user's own address: a `vless://` line would be "checked" without a proxy.
/// SOCKS5 becomes `socks5h`, so names are resolved by the proxy, as the
/// dispatcher does, instead of leaking to the local resolver.
pub(crate) fn reqwest_proxy_url(url: &str) -> Result<String, String> {
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err("invalid proxy URL".to_string());
    };
    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "socks5" | "socks" | "socks5h" => "socks5h",
        s @ ("http" | "https" | "socks4" | "socks4a") => {
            return Ok(format!("{s}://{rest}"));
        }
        _ => return Err("unsupported proxy protocol".to_string()),
    };
    Ok(format!("{scheme}://{rest}"))
}

const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) ProxPulse/",
    env!("CARGO_PKG_VERSION")
);

/// A direct client that does not follow redirects, for answers that must come
/// from the address asked: a redirect could hand the judge's key lookup to
/// another host, and make a key from it look delivered over TLS.
pub(crate) fn build_client_no_redirects(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .tls_backend_preconfigured((*tls_client_config()).clone())
        .user_agent(USER_AGENT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| short_error(&e.to_string()))
}

pub(crate) fn build_client(
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
        .tls_backend_preconfigured((*tls_client_config()).clone())
        .user_agent(USER_AGENT);
    if no_proxy {
        b = b.no_proxy();
    }
    if let Some(u) = proxy_url {
        // reqwest's own message would repeat the URL, password included.
        let p = reqwest::Proxy::all(reqwest_proxy_url(u)?)
            .map_err(|_| "invalid proxy URL".to_string())?;
        // No idle connections through a checked proxy: every sample (and
        // every later run) then measures the full path, TCP + proxy
        // handshake + request. A reused tunnel would make later samples look
        // faster, turning "jitter" into the handshake cost.
        b = b.proxy(p).pool_max_idle_per_host(0);
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

pub(crate) const BODY_LIMIT_SMALL: usize = 256 * 1024;
pub(crate) const BODY_LIMIT_PAGE: usize = 1024 * 1024;
pub(crate) const BODY_LIMIT_IMPORT: usize = 2 * 1024 * 1024;
pub(crate) const BODY_LIMIT_WEBHOOK: usize = 8 * 1024;

/// Read a response body with a hard limit. Untrusted proxies can otherwise
/// return arbitrarily large bodies and exhaust memory.
pub(crate) async fn read_body_capped(
    mut r: reqwest::Response,
    max: usize,
) -> Result<String, String> {
    if let Some(len) = r.content_length() {
        if len > max as u64 {
            return Err("response too big".to_string());
        }
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(|e| request_error(&e))? {
        if body.len().saturating_add(chunk.len()) > max {
            return Err("response too big".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| "not text".to_string())
}

pub(crate) fn looks_like_ip(s: &str) -> bool {
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

pub(crate) fn apply_ipapi(res: &mut ProxyResult, g: &IpApi) {
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

/// Fill geo from a cache: the frontend's (TTL-filtered, always with an exit
/// IP) or the pool's known countries for scheduled checks (no IP). When the
/// test URL does not reveal the exit IP, the cached IP is used; reusing it
/// also lets the frontend refresh its timestamp. Only a known *different*
/// exit IP invalidates an entry.
pub(crate) fn apply_cached_geo(res: &mut ProxyResult, entry: &GeoCacheEntry) -> bool {
    let use_cached = match (&res.ip, &entry.ip) {
        (Some(a), Some(b)) => a == b,
        _ => true,
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

pub(crate) fn try_parse_test_body(res: &mut ProxyResult, test_url: &str, body: &str) {
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

pub(crate) const GEO_FIELDS: &str =
    "status,message,query,country,countryCode,city,isp,org,as,mobile,proxy,hosting";

/// The free ip-api tier is HTTP-only. Pro uses the same schema over HTTPS
/// and is authenticated by a key GET parameter (see members.ip-api.com).
pub(crate) fn geo_url(ipapi_key: Option<&str>) -> String {
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

/// After ip-api refuses the Pro key (429/403), geo lookups pause for
/// `GEO_COOLDOWN`.
pub(crate) async fn geo_cooling_down() -> bool {
    GEO_COOLDOWN_UNTIL
        .lock()
        .await
        .is_some_and(|t| Instant::now() < t)
}

/// Whether a 429/403 to this lookup speaks for every proxy in the run. Only
/// the Pro key is shared, and it goes over HTTPS, so the answer is ip-api's.
/// The free tier is limited per exit IP, and over plain HTTP a 403 may be the
/// proxy's own page: one proxy must not switch geo off for the whole run.
pub(crate) fn geo_refusal_is_shared(ipapi_key: Option<&str>) -> bool {
    ipapi_key.is_some_and(|k| !k.trim().is_empty())
}

pub(crate) async fn fetch_geo(client: &reqwest::Client, ipapi_key: Option<&str>) -> Option<IpApi> {
    if geo_cooling_down().await {
        return None;
    }
    let url = geo_url(ipapi_key);
    let r = client.get(url).send().await.ok()?;
    let status = r.status();
    if status.as_u16() == 429 || status.as_u16() == 403 {
        if geo_refusal_is_shared(ipapi_key) {
            *GEO_COOLDOWN_UNTIL.lock().await = Some(Instant::now() + GEO_COOLDOWN);
        }
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

/// Header and content checks must use plain HTTP. For an `https://` target an
/// HTTP proxy only opens a CONNECT tunnel: it can neither add `Via` /
/// `X-Forwarded-For` nor modify the body, so every proxy would look elite and
/// untampered. `show_env=1` makes httpbin return the forwarding headers it
/// hides by default.
pub(crate) const ANON_URL: &str = "http://httpbin.org/headers?show_env=1";
pub(crate) const TAMPER_URL: &str = "http://example.com/";

/// Added by httpbin's own load balancer, not by the checked proxy.
const ANON_INFRA_HEADERS: [&str; 3] = ["x-forwarded-proto", "x-forwarded-port", "x-amzn-trace-id"];

const ANON_TELLTALES: [&str; 15] = [
    "via",
    "forwarded",
    "forwarded-for",
    "x-forwarded-host",
    "x-forwarded-server",
    "x-forwarded-by",
    "x-real-ip",
    "x-client-ip",
    "client-ip",
    "x-proxy-id",
    "x-proxy-connection",
    "proxy-agent",
    "proxy-connection",
    "proxy-authorization",
    "x-bluecoat-via",
];

/// Whole-token IP match: `1.2.3.4` must not match inside `11.2.3.45`.
fn text_has_ip(text: &str, ip: std::net::IpAddr) -> bool {
    text.split(|c: char| !(c.is_ascii_hexdigit() || c == '.' || c == ':'))
        .filter(|t| !t.is_empty())
        .any(|t| {
            t.parse::<std::net::IpAddr>().ok() == Some(ip)
                || t.rsplit_once(':')
                    .and_then(|(h, _)| h.parse::<std::net::IpAddr>().ok())
                    == Some(ip)
        })
}

/// Classify the headers httpbin saw. The load balancer appends the address
/// that connected to it (the proxy exit) to `X-Forwarded-For`, so only the
/// entries before it were added by the proxy.
pub(crate) fn classify_anonymity(
    headers: &HashMap<String, String>,
    direct_ip: &str,
) -> Option<String> {
    let direct = direct_ip.trim().parse::<std::net::IpAddr>().ok()?;
    let lower: HashMap<String, &str> = headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.as_str()))
        .collect();
    let mut xff: Vec<&str> = lower
        .get("x-forwarded-for")
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    xff.pop();
    let leaked_in_xff = xff.iter().any(|e| text_has_ip(e, direct));
    let leaked_elsewhere = lower.iter().any(|(k, v)| {
        k != "x-forwarded-for"
            && !ANON_INFRA_HEADERS.contains(&k.as_str())
            && text_has_ip(v, direct)
    });
    if leaked_in_xff || leaked_elsewhere {
        return Some("transparent".to_string());
    }
    if !xff.is_empty() || ANON_TELLTALES.iter().any(|k| lower.contains_key(*k)) {
        return Some("anonymous".to_string());
    }
    Some("elite".to_string())
}

pub(crate) async fn detect_anonymity(
    client: &reqwest::Client,
    direct_ip: Option<&str>,
) -> Option<String> {
    // Without a direct-IP baseline "elite" would be a guess, not a result.
    // Return before spending a request through the proxy.
    let d = direct_ip.filter(|d| !d.is_empty())?;
    let r = client.get(ANON_URL).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let body = read_body_capped(r, BODY_LIMIT_SMALL).await.ok()?;
    let parsed = serde_json::from_str::<HttpbinHeaders>(&body).ok()?;
    classify_anonymity(&parsed.headers.unwrap_or_default(), d)
}

pub(crate) async fn detect_tamper(
    client: &reqwest::Client,
    base_len: Option<usize>,
    base_hash: Option<u64>,
) -> Option<String> {
    let (bl, bh) = match (base_len, base_hash) {
        (Some(a), Some(b)) => (a, b),
        _ => return None,
    };
    let r = client.get(TAMPER_URL).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let body = read_body_capped(r, BODY_LIMIT_PAGE).await.ok()?;
    if body.len() == bl && body_hash(&body) == bh {
        return Some("ok".to_string());
    }
    Some("modified".to_string())
}
