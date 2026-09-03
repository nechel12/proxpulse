use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{Error, ErrorKind};
use std::net::IpAddr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use tauri::{
    State,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Semaphore};

// ================= results =================

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

struct Baseline {
    direct_ip: Option<String>,
    example_len: Option<usize>,
    example_hash: Option<u64>,
    example_cert_fp: Option<String>,
}

fn body_hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[derive(Clone, Copy)]
struct CheckFlags {
    geo: bool,
    anon: bool,
    tamper: bool,
    tls: bool,
    precheck: bool,
    precheck_timeout: Duration,
}

// ================= proxy parsing =================

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
                let scheme = canonical_scheme(def);
                return Some((
                    format!("{scheme}://{}:{}@{}:{}", p[2], p[3], p[0], p[1]),
                    proto_label(&scheme),
                ));
            }
            if valid_port(p[3]) && !p[2].is_empty() {
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

fn normalize_proxy(raw: &str, default_proto: &str) -> Option<(String, String)> {
    let mut s = clean_token(strip_list_prefix(raw.trim()));
    if s.is_empty() || s.starts_with('#') || s.starts_with("//") {
        return None;
    }
    if let Some(pos) = s.find(" #") {
        s.truncate(pos);
        s = s.trim().to_string();
    }

    let def = match default_proto.to_lowercase().as_str() {
        "socks5" => "socks5",
        "socks4" => "socks4",
        _ => "http",
    };

    if s.contains(',') || s.contains(';') || s.contains('|') || s.contains('\t') {
        let parts: Vec<String> = s
            .split([',', ';', '|', '\t'])
            .map(clean_token)
            .filter(|x| !x.is_empty())
            .collect();
        if parts.len() >= 2 {
            if let Some(v) = from_columns(&parts, def) {
                return Some(v);
            }
        }
        if parts.len() != 1 {
            return None;
        }
        s = parts.into_iter().next().unwrap_or_default();
    }
    if !s.contains("://") && !s.contains('@') {
        let ws: Vec<String> = s.split_whitespace().map(clean_token).collect();
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
    if s.contains(':') || s.contains('@') {
        s = s.split_whitespace().collect::<String>();
    }
    if s.is_empty() {
        return None;
    }

    let (explicit_scheme, mut authority) = match s.split_once("://") {
        Some((sch, rest)) => (Some(sch.to_lowercase()), rest.to_string()),
        None => (None, s.clone()),
    };
    for sep in ['/', '?', '#'] {
        if let Some(pos) = authority.find(sep) {
            authority.truncate(pos);
        }
    }
    authority = authority.trim_matches('/').to_string();
    if authority.is_empty() {
        return None;
    }

    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u.to_string()), h.to_string()),
        None => (None, authority.clone()),
    };
    if hostport.is_empty() {
        return None;
    }

    if hostport.starts_with('[') {
        if let Some(end) = hostport.find(']') {
            let host = hostport[..=end].to_string();
            let rest = hostport[end + 1..].to_string();
            let tail: Vec<&str> = rest.split(':').collect();
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
            let (host, port) = (parts[0], parts[1]);
            if host.is_empty() {
                return None;
            }
            build_url(&scheme_for(), userinfo.as_deref(), host, port)
        }
        3 => {
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

/// host:port of an already normalized proxy url
fn host_port_of_url(url: &str) -> Option<(String, u16)> {
    let rest = url.split_once("://")?.1;
    let after = rest.rsplit('@').next()?;
    let hp = after.split('/').next()?;
    if let Some(stripped) = hp.strip_prefix('[') {
        let end = stripped.find(']')?;
        let host = &stripped[..end];
        let port: u16 = stripped[end + 1..].strip_prefix(':')?.parse().ok()?;
        return Some((host.to_string(), port));
    }
    let (h, p) = hp.rsplit_once(':')?;
    Some((h.to_string(), p.parse().ok()?))
}

// ================= http helpers =================

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
    let t = body.trim();
    if !t.is_empty() && t.len() <= 128 && !t.contains('<') && looks_like_ip(t) {
        res.ip = Some(t.to_string());
    }
}

async fn geo_lookup(client: &reqwest::Client, res: &mut ProxyResult) {
    if res.country.is_some() || res.ip_type.is_some() {
        return;
    }
    let url = "http://ip-api.com/json/?fields=status,message,query,country,countryCode,city,isp,org,as,mobile,proxy,hosting";
    let r = match client.get(url).send().await {
        Ok(v) => v,
        Err(_) => return,
    };
    if !r.status().is_success() {
        return;
    }
    let body = match r.text().await {
        Ok(t) => t,
        Err(_) => return,
    };
    if let Ok(g) = serde_json::from_str::<IpApi>(&body) {
        if g.status.as_deref() == Some("fail") {
            return;
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
    if let Some(d) = direct_ip {
        if !d.is_empty() && lower.values().any(|v| v.contains(d)) {
            return Some("transparent".to_string());
        }
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
    let body = r.text().await.ok()?;
    if body.len() == bl && body_hash(&body) == bh {
        return Some("ok".to_string());
    }
    Some("modified".to_string())
}

// ================= tcp via upstream (shared by TLS check + dispatcher) =================

#[derive(Debug, Clone)]
struct Upstream {
    raw: String,
    scheme: String,
    host: String,
    port: u16,
    user: Option<String>,
    pass: Option<String>,
    latency: Option<u64>,
}

fn parse_upstream_url(url: &str) -> Option<(String, String, u16, Option<String>, Option<String>)> {
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

async fn read_exact_t(s: &mut TcpStream, buf: &mut [u8], d: Duration) -> std::io::Result<()> {
    match tokio::time::timeout(d, s.read_exact(buf)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "read timeout")),
    }
}

async fn write_all_t(s: &mut TcpStream, buf: &[u8], d: Duration) -> std::io::Result<()> {
    match tokio::time::timeout(d, s.write_all(buf)).await {
        Ok(r) => r,
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "write timeout")),
    }
}

async fn socks5_connect(
    mut s: TcpStream,
    t_host: &str,
    t_port: u16,
    user: Option<String>,
    pass: Option<String>,
    d: Duration,
) -> std::io::Result<TcpStream> {
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
            return Err(Error::new(ErrorKind::PermissionDenied, "socks5 auth failed"));
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

async fn socks4_connect(
    mut s: TcpStream,
    t_host: &str,
    t_port: u16,
    user: Option<String>,
    d: Duration,
) -> std::io::Result<TcpStream> {
    let mut req = vec![0x04, 0x01];
    req.extend_from_slice(&t_port.to_be_bytes());
    let mut domain: Option<&[u8]> = None;
    if let Ok(IpAddr::V4(v)) = t_host.parse::<IpAddr>() {
        req.extend_from_slice(&v.octets());
    } else {
        req.extend_from_slice(&[0, 0, 0, 1]);
        domain = Some(t_host.as_bytes());
    }
    let uid = user.unwrap_or_default();
    req.extend_from_slice(uid.as_bytes());
    req.push(0x00);
    if let Some(dm) = domain {
        req.extend_from_slice(dm);
        req.push(0x00);
    }
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

async fn http_connect(
    mut s: TcpStream,
    t_host: &str,
    t_port: u16,
    user: Option<String>,
    pass: Option<String>,
    d: Duration,
) -> std::io::Result<TcpStream> {
    let mut req = format!("CONNECT {t_host}:{t_port} HTTP/1.1\r\nHost: {t_host}:{t_port}\r\n");
    if let Some(u) = user {
        let p = pass.unwrap_or_default();
        let enc = base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}"));
        req.push_str(&format!("Proxy-Authorization: Basic {enc}\r\n"));
    }
    req.push_str("\r\n");
    write_all_t(&mut s, req.as_bytes(), d).await?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        if buf.len() > 32768 {
            return Err(Error::new(ErrorKind::InvalidData, "proxy header too big"));
        }
        let n = match tokio::time::timeout(d, s.read(&mut tmp)).await {
            Ok(Ok(0)) => return Err(Error::new(ErrorKind::UnexpectedEof, "proxy closed")),
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(Error::new(ErrorKind::TimedOut, "proxy read timeout")),
        };
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
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

async fn connect_via(
    up: &Upstream,
    t_host: &str,
    t_port: u16,
    timeout: Duration,
) -> std::io::Result<TcpStream> {
    let addr = format!("{}:{}", up.host, up.port);
    let stream = match tokio::time::timeout(timeout, TcpStream::connect(addr.as_str())).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(Error::new(ErrorKind::TimedOut, "upstream timeout")),
    };
    stream.set_nodelay(true).ok();
    match up.scheme.as_str() {
        "socks5" | "socks5h" => {
            socks5_connect(
                stream,
                t_host,
                t_port,
                up.user.clone(),
                up.pass.clone(),
                timeout,
            )
            .await
        }
        "socks4" | "socks4a" => {
            socks4_connect(stream, t_host, t_port, up.user.clone(), timeout).await
        }
        _ => {
            http_connect(
                stream,
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
            Arc::new(
                rustls::ClientConfig::builder()
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

async fn tls_handshake_fp(
    stream: TcpStream,
    timeout: Duration,
) -> Option<(String, String)> {
    use rustls_pki_types::ServerName;
    let connector =
        tokio_rustls::TlsConnector::from(tls_client_config());
    let name = ServerName::try_from("example.com").ok()?.to_owned();
    let fut = connector.connect(name, stream);
    let tls = match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(t)) => t,
        _ => return None,
    };
    let (_io, conn) = tls.get_ref();
    let certs = conn.peer_certificates()?;
    let leaf = certs.first()?;
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
    Some((fp, format!("{ver} {cipher} {short}")))
}

async fn tls_direct_fp() -> Option<String> {
    let timeout = Duration::from_millis(9000);
    let stream =
        match tokio::time::timeout(timeout, TcpStream::connect(("example.com", 443))).await {
            Ok(Ok(s)) => s,
            _ => return None,
        };
    tls_handshake_fp(stream, timeout).await.map(|(fp, _)| fp)
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
        scheme,
        host,
        port,
        user,
        pass,
        latency: None,
    };
    let stream = match connect_via(&up, "example.com", 443, timeout).await {
        Ok(s) => s,
        Err(_) => return (None, None),
    };
    match tls_handshake_fp(stream, timeout).await {
        Some((fp, info)) => match base_fp {
            Some(b) if fp == b => (Some("ok".to_string()), Some(info)),
            Some(_) => (Some("modified".to_string()), Some(info)),
            None => (None, Some(info)),
        },
        None => (None, None),
    }
}

// ================= checker =================

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
        tls: None,
        tls_info: None,
        error: Some(err),
    }
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

    let client = match build_client(Some(&url), timeout) {
        Ok(c) => c,
        Err(e) => return dead_result(raw, proto, 0, e),
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
        tls: None,
        tls_info: None,
        error: None,
    };

    if let Some(b) = first_body {
        try_parse_test_body(&mut res, &test_url, &b);
    }
    if flags.geo {
        geo_lookup(&client, &mut res).await;
    }
    if flags.anon {
        res.anonymity = detect_anonymity(&client, baseline.direct_ip.as_deref()).await;
    }
    if flags.tamper {
        res.tamper = detect_tamper(&client, baseline.example_len, baseline.example_hash).await;
    }
    if flags.tls {
        let (v, info) =
            tls_via_proxy(&url, timeout, baseline.example_cert_fp.as_deref()).await;
        res.tls = v;
        res.tls_info = info;
    }
    res
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
    let (ip, ex, cert) = tokio::join!(direct_ip, example, tls_direct_fp());
    let (len, hash) = ex.unzip();
    Baseline {
        direct_ip: ip,
        example_len: len,
        example_hash: hash,
        example_cert_fp: cert,
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
    with_tls: Option<bool>,
    precheck: Option<bool>,
    precheck_timeout_ms: Option<u64>,
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
    let repeats = repeats.unwrap_or(3).clamp(1, 5);
    let flags = CheckFlags {
        geo: with_geo.unwrap_or(true),
        anon: with_anonymity.unwrap_or(true),
        tamper: with_tamper.unwrap_or(true),
        tls: with_tls.unwrap_or(true),
        precheck: precheck.unwrap_or(true),
        precheck_timeout: Duration::from_millis(precheck_timeout_ms.unwrap_or(1500).clamp(300, 10_000)),
    };

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
                    flags,
                    base,
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
                    flags,
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
                    flags,
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

// ================= local proxy dispatcher =================

#[derive(Debug, Deserialize)]
struct DispatchItem {
    raw: String,
    latency: Option<u64>,
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
}

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
            }),
        }
    }
}

fn pick_order(len: usize, mode: &str, pool: &[Upstream], rr: &AtomicUsize) -> Vec<usize> {
    if len == 0 {
        return vec![];
    }
    if mode == "fastest" {
        let mut idx: Vec<usize> = (0..len).collect();
        idx.sort_by_key(|&i| pool[i].latency.unwrap_or(u64::MAX));
        idx
    } else if mode == "failover" {
        (0..len).collect()
    } else {
        let start = rr.fetch_add(1, Ordering::Relaxed) % len;
        (0..len).map(|i| (start + i) % len).collect()
    }
}

fn parse_target(method: &str, target: &str) -> Option<(String, u16)> {
    if method.eq_ignore_ascii_case("CONNECT") {
        let (h, p) = target.rsplit_once(':')?;
        return Some((h.trim().to_string(), p.trim().parse().ok()?));
    }
    let rest = target.strip_prefix("http://")?;
    let auth = rest.split('/').next()?;
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

async fn take_upstreams(st: &Arc<DispatchInner>) -> Vec<Upstream> {
    let pool = st.pool.lock().await;
    if pool.is_empty() {
        return vec![];
    }
    let mode = st.mode.lock().await.clone();
    let order = pick_order(pool.len(), &mode, &pool, &st.rr);
    order.iter().map(|&i| pool[i].clone()).collect()
}

async fn handle_http(mut client: TcpStream, st: Arc<DispatchInner>) {
    // read request head
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_ok = loop {
        if buf.len() > 65536 {
            return;
        }
        let n = match tokio::time::timeout(Duration::from_secs(10), client.read(&mut tmp)).await
        {
            Ok(Ok(0)) => return,
            Ok(Ok(n)) => n,
            _ => return,
        };
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break true;
        }
    };
    if !head_ok {
        return;
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let mut it = head.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let target = it.next().unwrap_or("").to_string();
    let (t_host, t_port) = match parse_target(&method, &target) {
        Some(v) => v,
        None => {
            let _ = client
                .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                .await;
            return;
        }
    };
    let is_connect = method.eq_ignore_ascii_case("CONNECT");

    let order = take_upstreams(&st).await;
    if order.is_empty() {
        let _ = client
            .write_all(b"HTTP/1.1 502 No upstreams\r\n\r\n")
            .await;
        return;
    }

    let mut last_err = String::new();
    for up in &order {
        match connect_via(up, &t_host, t_port, Duration::from_secs(8)).await {
            Ok(mut ups) => {
                *st.current.lock().await = Some(up.raw.clone());
                st.requests.fetch_add(1, Ordering::Relaxed);
                let res: std::io::Result<()> = async {
                    if is_connect {
                        client
                            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                            .await?;
                    } else {
                        ups.write_all(&buf).await?;
                    }
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut ups).await;
                    Ok(())
                }
                .await;
                if res.is_err() {
                    st.errors.fetch_add(1, Ordering::Relaxed);
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
            (
                segs.join(":"),
                u16::from_be_bytes([b[16], b[17]]),
            )
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
                *st.current.lock().await = Some(up.raw.clone());
                st.requests.fetch_add(1, Ordering::Relaxed);
                if write_all_t(&mut client, &[0x05, 0x00, 0x00, 1, 0, 0, 0, 0, 0, 0], d)
                    .await
                    .is_err()
                {
                    st.errors.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                let _ = tokio::io::copy_bidirectional(&mut client, &mut u).await;
                return;
            }
            Err(e) => {
                last_err = e.to_string();
            }
        }
    }
    st.errors.fetch_add(1, Ordering::Relaxed);
    *st.last_error.lock().await =
        Some(format!("socks {t_host}:{t_port}: {last_err}"));
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
            Err(_) => continue,
        };
        let stc = st.clone();
        tokio::spawn(async move {
            handle_conn(stream, stc).await;
        });
    }
}

#[tauri::command]
async fn set_dispatch_pool(
    items: Vec<DispatchItem>,
    state: State<'_, DispatchState>,
) -> Result<usize, String> {
    let mut pool: Vec<Upstream> = Vec::new();
    let mut seen = HashSet::new();
    for it in items {
        let raw = it.raw.trim().to_string();
        if raw.is_empty() || !seen.insert(raw.clone()) {
            continue;
        }
        if let Some((url, _)) = normalize_proxy(&raw, "http") {
            if let Some((scheme, host, port, user, pass)) = parse_upstream_url(&url) {
                pool.push(Upstream {
                    raw,
                    scheme,
                    host,
                    port,
                    user,
                    pass,
                    latency: it.latency,
                });
            }
        }
        if pool.len() >= 20_000 {
            break;
        }
    }
    pool.sort_by_key(|u| u.latency.unwrap_or(u64::MAX));
    let n = pool.len();
    *state.inner.pool.lock().await = pool;
    Ok(n)
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
    if let Some(h) = state.inner.handle.lock().await.take() {
        h.abort();
    }
    {
        let pool = state.inner.pool.lock().await;
        if pool.is_empty() {
            return Err("pool is empty".to_string());
        }
    }
    let listener = listener.unwrap_or_else(|| "both".to_string());
    let listener = match listener.as_str() {
        "http" | "socks5" => listener,
        _ => "both".to_string(),
    };
    let listener_desc = match listener.as_str() {
        "http" => "http",
        "socks5" => "socks5",
        _ => "http+socks5",
    }
    .to_string();
    let tcp = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("bind 127.0.0.1:{port}: {e}"))?;
    *state.inner.port.lock().await = port;
    let mode = match mode.as_str() {
        "fastest" | "failover" => mode,
        _ => "round-robin".to_string(),
    };
    *state.inner.mode.lock().await = mode.clone();
    *state.inner.listener.lock().await = listener;
    state.inner.running.store(true, Ordering::SeqCst);
    let inner = state.inner.clone();
    let h = tokio::spawn(async move {
        run_server(tcp, inner).await;
    });
    *state.inner.handle.lock().await = Some(h);
    Ok(format!("127.0.0.1:{port} ({mode}, {listener_desc})"))
}

#[tauri::command]
async fn stop_local_proxy(state: State<'_, DispatchState>) -> Result<(), String> {
    if let Some(h) = state.inner.handle.lock().await.take() {
        h.abort();
    }
    state.inner.running.store(false, Ordering::SeqCst);
    Ok(())
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

#[tauri::command]
async fn send_webhook(
    url: String,
    format: String,
    items: Vec<DispatchItem>,
) -> Result<String, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("bad webhook url".to_string());
    }
    let (body, ctype) = match format.as_str() {
        "json" => {
            let arr: Vec<serde_json::Value> = items
                .iter()
                .map(|i| serde_json::json!({"proxy": i.raw, "latency_ms": i.latency}))
                .collect();
            (
                serde_json::to_string(&arr).map_err(|e| e.to_string())?,
                "application/json",
            )
        }
        "csv" => {
            let mut s = String::from("proxy,latency_ms\n");
            for i in &items {
                s.push_str(&format!(
                    "{},{}\n",
                    i.raw,
                    i.latency.map(|v| v.to_string()).unwrap_or_default()
                ));
            }
            (s, "text/csv")
        }
        _ => (
            items
                .iter()
                .map(|i| i.raw.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            "text/plain",
        ),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("ProxPulse/0.3")
        .build()
        .map_err(|e| e.to_string())?;
    let r = client
        .post(url)
        .header("Content-Type", ctype)
        .body(body)
        .send()
        .await
        .map_err(|e| short_error(&e.to_string()))?;
    let code = r.status().as_u16();
    let txt = r.text().await.unwrap_or_default();
    let short: String = txt.chars().take(80).collect();
    Ok(format!("http {code} {short}"))
}

#[tauri::command]
fn hide_to_tray(app: tauri::AppHandle) -> Result<(), String> {
    app.get_webview_window("main")
        .ok_or_else(|| "no window".to_string())
        .and_then(|w| w.hide().map_err(|e| e.to_string()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(DispatchState::default())
        .invoke_handler(tauri::generate_handler![
            check_proxies,
            check_direct,
            write_text_file,
            set_dispatch_pool,
            start_local_proxy,
            stop_local_proxy,
            local_proxy_status,
            send_webhook,
            hide_to_tray
        ])
        .setup(|app| {
            if let Some(icon) = app.default_window_icon().cloned() {
                let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
                let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show, &hide, &quit])?;
                TrayIconBuilder::new()
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
            parse_target("GET", "http://example.com:8080/path?q=1"),
            Some(("example.com".into(), 8080))
        );
        assert_eq!(
            parse_target("GET", "http://example.com/path"),
            Some(("example.com".into(), 80))
        );
        assert_eq!(parse_target("GET", "/relative"), None);
    }

    #[test]
    fn order_modes() {
        let rr = AtomicUsize::new(0);
        let mk = |lat: Option<u64>| Upstream {
            raw: String::new(),
            scheme: "http".into(),
            host: String::new(),
            port: 0,
            user: None,
            pass: None,
            latency: lat,
        };
        let pool = vec![mk(Some(900)), mk(Some(100)), mk(None)];
        assert_eq!(
            pick_order(3, "fastest", &pool, &rr),
            vec![1, 0, 2]
        );
        assert_eq!(pick_order(3, "failover", &pool, &rr), vec![0, 1, 2]);
        let a = pick_order(3, "round-robin", &pool, &rr);
        let b = pick_order(3, "round-robin", &pool, &rr);
        assert_eq!(a, vec![0, 1, 2]);
        assert_eq!(b, vec![1, 2, 0]);
    }
}
