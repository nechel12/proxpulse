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

pub(crate) fn short_error(e: &str) -> String {
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
    while let Some(chunk) = r.chunk().await.map_err(|e| short_error(&e.to_string()))? {
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

/// Fill geo from the frontend cache. The frontend filters entries by a TTL;
/// when the test URL does not reveal the exit IP, the cached IP is used.
/// Reusing a cached IP here also lets the frontend refresh its timestamp.
pub(crate) fn apply_cached_geo(res: &mut ProxyResult, entry: &GeoCacheEntry) -> bool {
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

pub(crate) async fn fetch_geo(client: &reqwest::Client, ipapi_key: Option<&str>) -> Option<IpApi> {
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

pub(crate) async fn detect_anonymity(
    client: &reqwest::Client,
    direct_ip: Option<&str>,
) -> Option<String> {
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

pub(crate) async fn detect_tamper(
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
