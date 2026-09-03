use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyResult {
    pub proxy: String,
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

struct Baseline {
    direct_ip: Option<String>,
    example_len: Option<usize>,
    example_hash: Option<u64>,
}

fn body_hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

// ---------- proxy parsing ----------

fn proto_label(scheme: &str) -> String {
    match scheme.to_lowercase().as_str() {
        "http" | "https" => "HTTP".to_string(),
        "socks5" | "socks5h" | "socks" => "SOCKS5".to_string(),
        "socks4" | "socks4a" => "SOCKS4".to_string(),
        other => other.to_uppercase(),
    }
}

fn canonical_scheme(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "socks" => "socks5".to_string(),
        "https" => "http".to_string(),
        other => other.to_string(),
    }
}

fn is_proto_name(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "http" | "https" | "socks" | "socks4" | "socks4a" | "socks5" | "socks5h"
    )
}

fn valid_port(p: &str) -> bool {
    if p.is_empty() || p.len() > 5 || !p.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match p.parse::<u32>() {
        Ok(n) => n >= 1 && n <= 65535,
        Err(_) => false,
    }
}

fn clean_token(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '<' || c == '>')
        .trim()
        .to_string()
}

fn strip_list_prefix(s: &str) -> &str {
    let t = s.trim_start();
    // "1) ", "12. ", "3: ", "- ", "* ", "> "
    let mut idx = 0;
    for c in t.chars() {
        if c.is_ascii_digit() {
            idx += c.len_utf8();
        } else {
            break;
        }
    }
    if idx > 0 && idx < t.len() {
        let rest = &t[idx..];
        if let Some(c) = rest.chars().next() {
            if c == ')' || c == '.' || c == ':' || c == '-' {
                let after = &rest[c.len_utf8()..];
                if after.starts_with(' ') || after.starts_with('\t') {
                    return after.trim_start();
                }
            }
        }
    }
    if let Some(stripped) = t.strip_prefix("- ") {
        return stripped;
    }
    if let Some(stripped) = t.strip_prefix("* ") {
        return stripped;
    }
    if let Some(stripped) = t.strip_prefix("> ") {
        return stripped;
    }
    t
}

/// host,port,user,pass style columns (csv / space separated)
fn from_columns(parts: &[String], default_proto: &str) -> Option<(String, String)> {
    let def = match default_proto.to_lowercase().as_str() {
        "socks5" => "socks5",
        "socks4" => "socks4",
        _ => "http",
    };
    match parts.len() {
        2 => {
            let (host, port) = (parts[0].trim(), parts[1].trim());
            if !host.is_empty() && valid_port(port) {
                let scheme = canonical_scheme(def);
                return Some((
                    format!("{scheme}://{host}:{port}"),
                    proto_label(&scheme),
                ));
            }
            None
        }
        3 => {
            let a = parts[0].trim();
            let b = parts[1].trim();
            let c = parts[2].trim();
            if valid_port(b) && is_proto_name(c) {
                let scheme = canonical_scheme(c);
                return Some((format!("{scheme}://{a}:{b}"), proto_label(&scheme)));
            }
            if is_proto_name(a) && valid_port(c) {
                let scheme = canonical_scheme(a);
                return Some((format!("{scheme}://{b}:{c}"), proto_label(&scheme)));
            }
            None
        }
        4 => {
            let p: Vec<&str> = parts.iter().map(|x| x.trim()).collect();
            if valid_port(p[1]) && !p[0].is_empty() {
                // host,port,user,pass
                let scheme = canonical_scheme(def);
                return Some((
                    format!("{scheme}://{}:{}@{}:{}", p[2], p[3], p[0], p[1]),
                    proto_label(&scheme),
                ));
            }
            if valid_port(p[3]) && !p[2].is_empty() {
                // user,pass,host,port
                let scheme = canonical_scheme(def);
                return Some((
                    format!("{scheme}://{}:{}@{}:{}", p[0], p[1], p[2], p[3]),
                    proto_label(&scheme),
                ));
            }
            None
        }
        5 => {
            let p: Vec<&str> = parts.iter().map(|x| x.trim()).collect();
            if is_proto_name(p[0]) && valid_port(p[2]) {
                // proto,host,port,user,pass
                let scheme = canonical_scheme(p[0]);
                return Some((
                    format!("{scheme}://{}:{}@{}:{}", p[3], p[4], p[1], p[2]),
                    proto_label(&scheme),
                ));
            }
            None
        }
        _ => None,
    }
}

fn build_url(
    scheme: &str,
    userinfo: Option<&str>,
    host: &str,
    port: &str,
) -> Option<(String, String)> {
    if host.is_empty() || !valid_port(port) {
        return None;
    }
    let scheme = canonical_scheme(scheme);
    let url = match userinfo {
        Some(u) if !u.is_empty() => format!("{scheme}://{u}@{host}:{port}"),
        _ => format!("{scheme}://{host}:{port}"),
    };
    let label = proto_label(&scheme);
    Some((url, label))
}

/// Normalize one proxy string into (full_url, proto_label).
/// Supports: ip:port, host:port, user:pass@host:port,
/// scheme://..., host:port:user:pass, user:pass:host:port,
/// host:port:proto, proto:host:port, [ipv6]:port, csv columns, etc.
fn normalize_proxy(raw: &str, default_proto: &str) -> Option<(String, String)> {
    let mut s = clean_token(strip_list_prefix(raw.trim()));
    if s.is_empty() || s.starts_with('#') || s.starts_with("//") {
        return None;
    }
    // cut inline comment
    if let Some(pos) = s.find(" #") {
        s.truncate(pos);
        s = s.trim().to_string();
    }

    let def = match default_proto.to_lowercase().as_str() {
        "socks5" => "socks5",
        "socks4" => "socks4",
        _ => "http",
    };

    // csv / delimited columns: host,port,user,pass etc.
    if s.contains(',') || s.contains(';') || s.contains('|') || s.contains('\t') {
        let parts: Vec<String> = s
            .split([',', ';', '|', '\t'])
            .map(|x| clean_token(x))
            .filter(|x| !x.is_empty())
            .collect();
        if parts.len() >= 2 {
            if let Some(v) = from_columns(&parts, def) {
                return Some(v);
            }
        }
        // fall through: maybe single url with commas in password -> continue generic path
        if parts.len() != 1 {
            return None;
        }
        s = parts.into_iter().next().unwrap_or_default();
    }
    // space separated columns (no scheme, no @)
    if !s.contains("://") && !s.contains('@') {
        let ws: Vec<String> = s.split_whitespace().map(|x| clean_token(x)).collect();
        if ws.len() >= 2 && ws.len() <= 5 {
            let looks_like_cols = ws.iter().all(|x| {
                !x.contains('/') && !x.contains('?') && !x.contains('#') && x.len() <= 256
            });
            if looks_like_cols {
                if let Some(v) = from_columns(&ws, def) {
                    return Some(v);
                }
            }
            if ws.len() > 1 {
                return None;
            }
            s = ws.into_iter().next().unwrap_or_default();
        } else if ws.len() > 1 {
            return None;
        }
    }
    // remove inner spaces around : and @ (passwords with spaces are not supported)
    if s.contains(':') || s.contains('@') {
        s = s.split_whitespace().collect::<String>();
    }
    if s.is_empty() {
        return None;
    }

    // split scheme
    let (explicit_scheme, mut authority) = match s.split_once("://") {
        Some((sch, rest)) => (Some(sch.to_lowercase()), rest.to_string()),
        None => (None, s.clone()),
    };
    // strip path / query / fragment (proxy url ignores them)
    for sep in ['/', '?', '#'] {
        if let Some(pos) = authority.find(sep) {
            authority.truncate(pos);
        }
    }
    authority = authority.trim_matches('/').to_string();
    if authority.is_empty() {
        return None;
    }

    // userinfo split at LAST @ (username may contain @ encoded, take last)
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u.to_string()), h.to_string()),
        None => (None, authority.clone()),
    };
    if hostport.is_empty() {
        return None;
    }

    // IPv6 in brackets: [::1]:8080 (+ optional :user:pass tail)
    if hostport.starts_with('[') {
        if let Some(end) = hostport.find(']') {
            let host = hostport[..=end].to_string();
            let rest = hostport[end + 1..].to_string(); // ":8080" or ":8080:user:pass"
            let tail: Vec<&str> = rest.split(':').collect();
            // tail[0] == "" because rest starts with ':'
            if tail.len() >= 2 && valid_port(tail[1]) {
                let scheme = explicit_scheme
                    .map(|x| canonical_scheme(&x))
                    .unwrap_or_else(|| def.to_string());
                if tail.len() == 2 {
                    return build_url(&scheme, userinfo.as_deref(), &host, tail[1]);
                }
                if tail.len() == 4 && userinfo.is_none() {
                    let u = format!("{}:{}", tail[2], tail[3]);
                    return build_url(&scheme, Some(&u), &host, tail[1]);
                }
            }
            return None;
        }
        return None;
    }

    let colon_count = hostport.matches(':').count();
    // bare IPv6 without brackets: a:b:c...:port -> last segment is port
    if colon_count > 4 && userinfo.is_none() && explicit_scheme.is_none() {
        if let Some(pos) = hostport.rfind(':') {
            let (h, p) = (&hostport[..pos], &hostport[pos + 1..]);
            if valid_port(p) && !h.is_empty() {
                let scheme = def.to_string();
                return build_url(&scheme, None, &format!("[{h}]"), p);
            }
        }
        return None;
    }

    let parts: Vec<&str> = hostport.split(':').collect();
    let scheme_for = || {
        explicit_scheme
            .clone()
            .map(|x| canonical_scheme(&x))
            .unwrap_or_else(|| def.to_string())
    };
    match parts.len() {
        2 => {
            // host:port
            let (host, port) = (parts[0], parts[1]);
            if host.is_empty() {
                return None;
            }
            build_url(&scheme_for(), userinfo.as_deref(), host, port)
        }
        3 => {
            // host:port:proto  |  proto:host:port
            if valid_port(parts[1]) && is_proto_name(parts[2]) && userinfo.is_none() {
                let scheme = canonical_scheme(parts[2]);
                return build_url(&scheme, None, parts[0], parts[1]);
            }
            if is_proto_name(parts[0]) && valid_port(parts[2]) && userinfo.is_none() {
                let scheme = canonical_scheme(parts[0]);
                return build_url(&scheme, None, parts[1], parts[2]);
            }
            None
        }
        4 => {
            // host:port:user:pass  |  user:pass:host:port (no @)
            if userinfo.is_some() {
                return None;
            }
            if valid_port(parts[1]) {
                let scheme = scheme_for();
                let u = format!("{}:{}", parts[2], parts[3]);
                return build_url(&scheme, Some(&u), parts[0], parts[1]);
            }
            if valid_port(parts[3]) && !valid_port(parts[1]) {
                let scheme = scheme_for();
                let u = format!("{}:{}", parts[0], parts[1]);
                return build_url(&scheme, Some(&u), parts[2], parts[3]);
            }
            None
        }
        _ => None,
    }
}

/// One input line may expand to 1-2 candidates (dual http+socks5 mode).
fn normalize_candidates(raw: &str, default_proto: &str) -> Vec<(String, String)> {
    let dual = matches!(
        default_proto.to_lowercase().as_str(),
        "dual" | "auto" | "http+socks5" | "http_socks5" | "both"
    );
    let has_scheme = raw.contains("://");
    if dual && !has_scheme {
        let mut out = Vec::new();
        if let Some(v) = normalize_proxy(raw, "http") {
            out.push(v);
        }
        if let Some(v) = normalize_proxy(raw, "socks5") {
            if !out.iter().any(|(u, _)| u == &v.0) {
                out.push(v);
            }
        }
        out
    } else if let Some(v) = normalize_proxy(raw, default_proto) {
        vec![v]
    } else {
        vec![]
    }
}

// ---------- http helpers ----------

fn short_error(e: &str) -> String {
    let low = e.to_lowercase();
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
    {
        return "dns error".to_string();
    }
    if low.contains("connection closed")
        || low.contains("connection reset")
        || low.contains("broken pipe")
        || low.contains("unexpected eof")
    {
        return "connection reset".to_string();
    }
    if low.contains("rate limited") || low.contains("too many requests") {
        return "rate limited".to_string();
    }
    let mut s = e.replace('\n', " ");
    if s.len() > 120 {
        s.truncate(120);
    }
    s.trim().to_string()
}

fn build_client(
    proxy_url: Option<&str>,
    timeout: Duration,
) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) ProxPulse/0.2");
    if let Some(u) = proxy_url {
        let p = reqwest::Proxy::all(u).map_err(|e| short_error(&e.to_string()))?;
        b = b.proxy(p);
    }
    b.build().map_err(|e| short_error(&e.to_string()))
}

fn looks_like_ip(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.len() > 64 || t.contains('<') || t.contains(' ') || t.contains('\n') {
        return false;
    }
    t.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == ':' || c == '-')
        && (t.contains('.') || t.contains(':'))
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
        // Точный residential vs business без платной ASN-базы невозможен —
        // эвристика: обычный ISP.
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
    // generic: короткий ответ без html — вероятно IP
    let t = body.trim();
    if !t.is_empty() && t.len() <= 128 && !t.contains('<') && looks_like_ip(t) {
        res.ip = Some(t.to_string());
    }
}

async fn geo_lookup(
    client: &reqwest::Client,
    res: &mut ProxyResult,
) {
    if res.country.is_some() || res.ip_type.is_some() {
        return;
    }
    let url = "http://ip-api.com/json/?fields=status,message,query,country,countryCode,city,isp,org,as,mobile,proxy,hosting";
    let r = client.get(url).send().await;
    let resp = match r {
        Ok(v) => v,
        Err(_) => return,
    };
    if !resp.status().is_success() {
        return;
    }
    let body = match resp.text().await {
        Ok(t) => t,
        Err(_) => return,
    };
    if let Ok(g) = serde_json::from_str::<IpApi>(&body) {
        if g.status.as_deref() == Some("fail") {
            return; // чаще всего rate limit бесплатного тарифа
        }
        apply_ipapi(res, &g);
    }
}

async fn detect_anonymity(
    client: &reqwest::Client,
    direct_ip: Option<&str>,
) -> Option<String> {
    let r = client.get("https://httpbin.org/headers").send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let body = r.text().await.ok()?;
    let parsed = serde_json::from_str::<HttpbinHeaders>(&body).ok()?;
    let headers = parsed.headers.unwrap_or_default();
    let mut lower: HashMap<String, String> = HashMap::new();
    for (k, v) in headers {
        lower.insert(k.to_lowercase(), v);
    }
    // утечка реального IP в значениях заголовков
    if let Some(d) = direct_ip {
        if !d.is_empty() && lower.values().any(|v| v.contains(d)) {
            return Some("transparent".to_string());
        }
    }
    // x-forwarded-for вида "a, b" тоже намекает на цепочку
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
    let body = r.text().await.ok()?;
    if body.len() == bl && body_hash(&body) == bh {
        return Some("ok".to_string());
    }
    // допуск: example.com статичен; заметное расхождение = модификация
    let diff = (body.len() as i64 - bl as i64).unsigned_abs() as f64 / bl.max(1) as f64;
    if diff > 0.10 || body_hash(&body) != bh {
        // хэш строгий: любое отличие байтов считаем модификацией
        return Some("modified".to_string());
    }
    Some("ok".to_string())
}

#[allow(clippy::too_many_arguments)]
async fn check_candidate(
    raw: String,
    url: String,
    proto: String,
    test_url: String,
    timeout: Duration,
    repeats: usize,
    with_geo: bool,
    with_anonymity: bool,
    with_tamper: bool,
    baseline: Arc<Baseline>,
) -> ProxyResult {
    let client = match build_client(Some(&url), timeout) {
        Ok(c) => c,
        Err(e) => {
            return ProxyResult {
                proxy: raw,
                proto,
                alive: false,
                latency_ms: None,
                jitter_ms: None,
                success_rate: 0.0,
                attempts: 0,
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
                error: Some(e),
            }
        }
    };

    let mut latencies: Vec<u64> = Vec::new();
    let mut first_body: Option<String> = None;
    let mut last_err: Option<String> = None;

    for _ in 0..repeats {
        let start = Instant::now();
        match client.get(&test_url).send().await {
            Ok(r) => {
                let status = r.status();
                let ms = start.elapsed().as_millis() as u64;
                if status.is_success() {
                    latencies.push(ms);
                    if first_body.is_none() {
                        first_body = r.text().await.ok();
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
        return ProxyResult {
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
            error: last_err.or(Some("failed".to_string())),
        };
    }

    let avg = latencies.iter().sum::<u64>() / latencies.len() as u64;
    let jitter = if latencies.len() > 1 {
        Some(latencies.iter().max().unwrap() - latencies.iter().min().unwrap())
    } else {
        Some(0)
    };
    let success_rate = successes as f32 / attempts as f32;

    let mut res = ProxyResult {
        proxy: raw,
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
        error: None,
    };

    if let Some(b) = first_body {
        try_parse_test_body(&mut res, &test_url, &b);
    }
    if with_geo {
        geo_lookup(&client, &mut res).await;
    }
    if with_anonymity {
        res.anonymity = detect_anonymity(&client, baseline.direct_ip.as_deref()).await;
    }
    if with_tamper {
        res.tamper =
            detect_tamper(&client, baseline.example_len, baseline.example_hash).await;
    }
    res
}

fn dead_result(raw: String, proto: String, attempts: u32, err: String) -> ProxyResult {
    ProxyResult {
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
        error: Some(err),
    }
}

#[tauri::command]
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
) -> Result<Vec<ProxyResult>, String> {
    let test_url = test_url.trim().to_string();
    if test_url.is_empty() {
        return Err("empty test url".to_string());
    }
    if !(test_url.starts_with("http://") || test_url.starts_with("https://")) {
        return Err("test url must start with http(s)://".to_string());
    }

    let timeout = Duration::from_millis(timeout_ms.clamp(500, 60_000));
    let conc = concurrency.clamp(1, 500);
    let repeats = repeats.unwrap_or(2).clamp(1, 5);
    let with_geo = with_geo.unwrap_or(true);
    let with_anonymity = with_anonymity.unwrap_or(true);
    let with_tamper = with_tamper.unwrap_or(true);

    // normalize + dedup (keep order)
    let mut jobs: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut seen = HashSet::new();
    for raw in proxies {
        let t = raw.trim().to_string();
        if t.is_empty() || !seen.insert(t.clone()) {
            continue;
        }
        let cands = normalize_candidates(&t, &default_proto);
        jobs.push((t, cands));
        if jobs.len() >= 20_000 {
            break;
        }
    }

    // shared baselines (direct connection, no proxy)
    let baseline = Arc::new(fetch_baseline().await);

    let sem = Arc::new(Semaphore::new(conc));
    let mut set = tokio::task::JoinSet::new();

    for (raw, cands) in jobs {
        if cands.is_empty() {
            let r = dead_result(raw, "BAD".to_string(), 0, "bad format".to_string());
            set.spawn(async move { r });
            continue;
        }
        let sem_c = sem.clone();
        let tu = test_url.clone();
        let base = baseline.clone();
        set.spawn(async move {
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
                    with_geo,
                    with_anonymity,
                    with_tamper,
                    base,
                )
                .await
            } else {
                // dual: сначала http, при неудаче socks5
                let (u1, p1) = &cands[0];
                let r1 = check_candidate(
                    raw.clone(),
                    u1.clone(),
                    p1.clone(),
                    tu.clone(),
                    timeout,
                    repeats,
                    with_geo,
                    with_anonymity,
                    with_tamper,
                    base.clone(),
                )
                .await;
                if r1.alive {
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
                    with_geo,
                    with_anonymity,
                    with_tamper,
                    base,
                )
                .await;
                if r2.alive {
                    return r2;
                }
                let e1 = r1.error.unwrap_or_default();
                let e2 = r2.error.unwrap_or_default();
                let mut combined = format!("http: {e1} / socks5: {e2}");
                if combined.len() > 120 {
                    combined.truncate(120);
                }
                dead_result(raw, "HTTP/SOCKS5".to_string(), r1.attempts, combined)
            }
        });
    }

    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        match r {
            Ok(v) => out.push(v),
            Err(e) => out.push(dead_result(
                "?".to_string(),
                "?".to_string(),
                0,
                short_error(&e.to_string()),
            )),
        }
    }
    out.sort_by(|a, b| {
        b.alive
            .cmp(&a.alive)
            .then(a.latency_ms.unwrap_or(u64::MAX).cmp(&b.latency_ms.unwrap_or(u64::MAX)))
    });
    Ok(out)
}

async fn fetch_baseline() -> Baseline {
    let timeout = Duration::from_millis(9000);
    let direct_ip = async {
        let c = build_client(None, timeout).ok()?;
        let r = c.get("https://api.ipify.org").send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        let t = r.text().await.ok()?;
        let t = t.trim().to_string();
        if looks_like_ip(&t) {
            Some(t)
        } else {
            None
        }
    };
    let example = async {
        let c = build_client(None, timeout).ok()?;
        let r = c.get("https://example.com").send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        let t = r.text().await.ok()?;
        Some((t.len(), body_hash(&t)))
    };
    let (ip, ex) = tokio::join!(direct_ip, example);
    let (len, hash) = ex.unzip();
    // unzip on Option<(usize,u64)> -> (Option<usize>, Option<u64>)
    Baseline {
        direct_ip: ip,
        example_len: len,
        example_hash: hash,
    }
}

#[tauri::command]
async fn check_direct(test_url: String, timeout_ms: u64) -> Result<u64, String> {
    let timeout = Duration::from_millis(timeout_ms.clamp(500, 30_000));
    let client = build_client(None, timeout).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let r = client
        .get(test_url.trim())
        .send()
        .await
        .map_err(|e| short_error(&e.to_string()))?;
    if !r.status().is_success() {
        return Err(format!("http {}", r.status().as_u16()));
    }
    Ok(start.elapsed().as_millis() as u64)
}

#[tauri::command]
async fn write_text_file(path: String, contents: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("empty path".to_string());
    }
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            check_proxies,
            check_direct,
            write_text_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
