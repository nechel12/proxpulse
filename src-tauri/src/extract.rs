// Proxy extraction from free-form text (lists, CSV, JSON, logs, prose).
//
// This is the only proxy-list parser: the UI shows the count of what it
// returns and the checker receives exactly that list. Every candidate must
// also pass `normalize_proxy`, so the count matches what a check will test.

use crate::proxy::{is_proto_name, looks_like_host, normalize_proxy, valid_port};
use serde_json::Value;
use std::collections::HashSet;

const PROTO_KEYS: [&str; 4] = ["protocol", "proto", "type", "scheme"];
const HOST_KEYS: [&str; 6] = ["host", "hostname", "ip", "server", "address", "addr"];
const PORT_KEYS: [&str; 3] = ["port", "port_number", "portnumber"];
const USER_KEYS: [&str; 5] = ["user", "username", "login", "usr", "name"];
const PASS_KEYS: [&str; 6] = ["pass", "password", "pwd", "passw", "passwd", "secret"];
const LIST_KEYS: [&str; 7] = [
    "proxies", "proxy", "data", "list", "items", "results", "rows",
];
const URL_SCHEMES: [&str; 7] = [
    "socks5", "socks5h", "socks4", "socks4a", "socks", "http", "https",
];

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn host_for_url(host: &str) -> String {
    let h = host.trim();
    if h.contains(':') && !h.starts_with('[') {
        format!("[{h}]")
    } else {
        h.to_string()
    }
}

fn trim_quotes(s: &str) -> &str {
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`')
}

/// Strip trailing punctuation and leading brackets around a token.
pub(crate) fn clean_tail(s: &str) -> &str {
    s.trim_end_matches(['.', ',', ';', '!', '?', ')', ']', '}', '>'])
        .trim_start_matches(['(', '<', '{'])
        .trim()
}

/// Port at the end of the authority (after the last `@`), not a numeric
/// password. Bare IPv6 is ambiguous and not supported: it needs brackets.
fn has_valid_port(s: &str) -> bool {
    let after_scheme = s.split_once("://").map_or(s, |(_, rest)| rest);
    let authority = after_scheme
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if !authority.contains(']') && authority.matches(':').count() > 1 {
        return false;
    }
    authority
        .rsplit_once(':')
        .is_some_and(|(_, port)| valid_port(port))
}

fn is_colon_columns(s: &str) -> bool {
    let p: Vec<&str> = s.split(':').collect();
    match p.len() {
        3 => (valid_port(p[1]) && is_proto_name(p[2])) || (is_proto_name(p[0]) && valid_port(p[2])),
        4 => (valid_port(p[1]) && !p[0].is_empty()) || (valid_port(p[3]) && !p[2].is_empty()),
        _ => false,
    }
}

fn looks_like_proxy_value(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with("://")
        && !s.chars().any(char::is_whitespace)
        && (has_valid_port(s) || is_colon_columns(s))
}

pub(crate) fn build_from_parts(
    host: &str,
    port: &str,
    user: Option<&str>,
    pass: Option<&str>,
    proto: Option<&str>,
) -> Option<String> {
    let host = host_for_url(host.trim().trim_end_matches('/'));
    let port = port.trim();
    if host.is_empty() || !valid_port(port) {
        return None;
    }
    let prefix = match proto.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) if p.contains("://") => p.to_lowercase(),
        Some(p) => format!("{}://", p.to_lowercase()),
        None => String::new(),
    };
    Some(match (user, pass) {
        (Some(u), Some(p)) => format!("{prefix}{u}:{p}@{host}:{port}"),
        (Some(u), None) => format!("{prefix}{u}@{host}:{port}"),
        _ => format!("{prefix}{host}:{port}"),
    })
}

fn scalar(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => match (n.as_u64(), n.as_i64(), n.as_f64()) {
            (Some(u), _, _) => u.to_string(),
            (_, Some(i), _) => i.to_string(),
            (_, _, Some(f)) if f.fract() == 0.0 => format!("{f:.0}"),
            _ => n.to_string(),
        },
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    (!s.is_empty()).then_some(s)
}

/// Case-insensitive lookup of the first non-empty key.
fn get_key(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| {
        obj.iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(k))
            .and_then(|(_, v)| scalar(v))
    })
}

pub(crate) fn proxies_from_json(v: &Value, out: &mut Vec<String>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    match v {
        Value::String(s) => {
            let s = s.trim();
            if s.chars().count() > 2 {
                out.push(s.to_string());
            }
        }
        Value::Array(items) => {
            for el in items {
                if out.len() >= limit {
                    break;
                }
                proxies_from_json(el, out, limit);
            }
        }
        Value::Object(o) => {
            let host = get_key(o, &HOST_KEYS);
            let port = get_key(o, &PORT_KEYS);
            if let (Some(h), Some(p)) = (&host, &port) {
                let user = get_key(o, &USER_KEYS);
                let pass = get_key(o, &PASS_KEYS);
                let proto = get_key(o, &PROTO_KEYS);
                if let Some(built) =
                    build_from_parts(h, p, user.as_deref(), pass.as_deref(), proto.as_deref())
                {
                    out.push(built);
                }
            }
            for k in LIST_KEYS {
                if let Some(Value::Array(items)) = o.get(k) {
                    for el in items {
                        if out.len() >= limit {
                            break;
                        }
                        proxies_from_json(el, out, limit);
                    }
                    return;
                }
            }
            if host.is_none() {
                for val in o.values() {
                    if out.len() >= limit {
                        break;
                    }
                    if matches!(val, Value::String(_) | Value::Array(_) | Value::Object(_)) {
                        proxies_from_json(val, out, limit);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Column rows: `host,port[,user,pass]`, `user,pass,host,port`,
/// `host,port,proto`, `proto,host,port[,user,pass]`, also space-separated.
pub(crate) fn try_csv_line(line: &str) -> Option<String> {
    let parts: Vec<&str> = if line.contains([',', ';', '|', '\t']) {
        line.split([',', ';', '|', '\t'])
            .map(|s| trim_quotes(s.trim()))
            .filter(|s| !s.is_empty())
            .collect()
    } else if !line.contains("://") && !line.contains('@') {
        line.split_whitespace().collect()
    } else {
        return None;
    };
    match parts.len() {
        2 if valid_port(parts[1]) => Some(format!("{}:{}", host_for_url(parts[0]), parts[1])),
        3 if valid_port(parts[1]) && is_proto_name(parts[2]) => Some(format!(
            "{}://{}:{}",
            parts[2],
            host_for_url(parts[0]),
            parts[1]
        )),
        3 if is_proto_name(parts[0]) && valid_port(parts[2]) => Some(format!(
            "{}://{}:{}",
            parts[0],
            host_for_url(parts[1]),
            parts[2]
        )),
        4 => {
            let host_first = valid_port(parts[1]) && !parts[0].is_empty();
            let host_last = valid_port(parts[3]) && !parts[2].is_empty();
            let first = match (host_first, host_last) {
                (true, false) => true,
                (false, true) => false,
                (true, true) => !(looks_like_host(parts[2]) && !looks_like_host(parts[0])),
                (false, false) => return None,
            };
            Some(if first {
                format!(
                    "{}:{}@{}:{}",
                    parts[2],
                    parts[3],
                    host_for_url(parts[0]),
                    parts[1]
                )
            } else {
                format!(
                    "{}:{}@{}:{}",
                    parts[0],
                    parts[1],
                    host_for_url(parts[2]),
                    parts[3]
                )
            })
        }
        5 if is_proto_name(parts[0]) && valid_port(parts[2]) => Some(format!(
            "{}://{}:{}@{}:{}",
            parts[0],
            parts[3],
            parts[4],
            host_for_url(parts[1]),
            parts[2]
        )),
        _ => None,
    }
}

/// `scheme://...` URLs inside a line of prose. The scheme must be a whole
/// word; the URL ends at whitespace, quotes, brackets or `,;`.
pub(crate) fn scheme_urls(line: &str) -> Vec<&str> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = line[from..].find("://") {
        let sep = from + rel;
        let mut start = sep;
        while start > 0 && is_word(b[start - 1]) {
            start -= 1;
        }
        let scheme = &line[start..sep];
        let mut end = sep + 3;
        while end < b.len() && !b[end].is_ascii_whitespace() && !b"\"'<>,;()[]".contains(&b[end]) {
            end += 1;
        }
        if end > sep + 3 && URL_SCHEMES.iter().any(|s| s.eq_ignore_ascii_case(scheme)) {
            out.push(&line[start..end]);
            from = end;
        } else {
            from = sep + 3;
        }
    }
    out
}

/// Remove a list marker: `1) `, `2. `, `- `, `* `, `• `, `> `, `[x] `.
fn strip_list_marker(line: &str) -> &str {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=6).contains(&digits) {
        let rest = &line[digits..];
        if let Some(after) = rest.strip_prefix([')', '.', '-', ':']) {
            if after.starts_with(char::is_whitespace) {
                return after.trim_start();
            }
        }
    }
    if let Some(after) = line.strip_prefix(['-', '*', '•', '>']) {
        if after.starts_with(char::is_whitespace) {
            return after.trim_start();
        }
    }
    if let Some(after) = line.strip_prefix('[') {
        let inner = after.trim_start();
        let inner = inner
            .strip_prefix(['x', 'X', '✓'])
            .unwrap_or(inner)
            .trim_start();
        if let Some(rest) = inner.strip_prefix(']') {
            return rest.trim_start();
        }
    }
    line
}

/// `a.b.c.d:port` tokens in arbitrary text (fallback for prose and logs).
fn ipv4_ports(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let digits_at = |i: usize| b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() || (i > 0 && is_word(b[i - 1])) {
            i += 1;
            continue;
        }
        let mut j = i;
        let mut ok = true;
        for octet in 0..4 {
            let n = digits_at(j);
            let sep = if octet < 3 { b'.' } else { b':' };
            if !(1..=3).contains(&n) || b.get(j + n) != Some(&sep) {
                ok = false;
                break;
            }
            j += n + 1;
        }
        if ok {
            let n = digits_at(j);
            let end = j + n;
            if (2..=5).contains(&n) && b.get(end).is_none_or(|c| !is_word(*c)) {
                out.push(&text[i..end]);
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

struct Collector<'a> {
    out: Vec<String>,
    /// Normalized URLs: `1.2.3.4:8080` and `http://1.2.3.4:8080` are one proxy.
    seen: HashSet<String>,
    limit: usize,
    default_proto: &'a str,
}

impl Collector<'_> {
    fn full(&self) -> bool {
        self.out.len() >= self.limit
    }

    fn push(&mut self, s: &str) {
        if self.full() {
            return;
        }
        let s = clean_tail(trim_quotes(s.trim()));
        let n = s.chars().count();
        if !(3..=512).contains(&n) || s.starts_with('#') || s.starts_with("//") {
            return;
        }
        // Soft shape check first (a bare string from a JSON log or prose must
        // not become a "proxy"), then the checker's own normalization.
        if !looks_like_proxy_value(s) {
            return;
        }
        let Some((url, _)) = normalize_proxy(s, self.default_proto) else {
            return;
        };
        if self.seen.insert(url) {
            self.out.push(s.to_string());
        }
    }
}

pub(crate) fn extract_proxies(text: &str, limit: usize, default_proto: &str) -> Vec<String> {
    let mut c = Collector {
        out: Vec::new(),
        seen: HashSet::new(),
        limit,
        default_proto,
    };

    if let Ok(json) = serde_json::from_str::<Value>(text) {
        let mut found = Vec::new();
        proxies_from_json(&json, &mut found, limit);
        if !found.is_empty() {
            for p in &found {
                c.push(p);
                if c.full() {
                    break;
                }
            }
            return c.out;
        }
    }

    for raw in text.lines() {
        if c.full() {
            break;
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let mut line = strip_list_marker(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(csv) = try_csv_line(line) {
            c.push(&csv);
            continue;
        }
        let urls = scheme_urls(line);
        if !urls.is_empty() {
            for u in urls {
                c.push(u);
                if c.full() {
                    break;
                }
            }
            continue;
        }
        if let Some(pos) = line.find(" #").filter(|&p| p > 0) {
            line = line[..pos].trim();
        }
        // A proxy token followed by a comment: `1.2.3.4:8080, note`.
        let token = line
            .split(|ch: char| ch.is_whitespace() || ",;|".contains(ch))
            .next()
            .unwrap_or("");
        let proxy_like = token.contains('@')
            || token.contains("://")
            || token
                .match_indices(':')
                .any(|(i, _)| token[i + 1..].starts_with(|ch: char| ch.is_ascii_digit()));
        if !token.is_empty() && token != line && proxy_like {
            line = token;
        }
        c.push(line);
    }

    if c.out.len() < 5 && !c.full() {
        for f in ipv4_ports(text) {
            if c.out.iter().any(|s| s.contains(f)) {
                continue;
            }
            c.push(f);
            if c.full() {
                break;
            }
        }
    }
    c.out
}
