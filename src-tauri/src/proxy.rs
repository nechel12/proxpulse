// Proxy URL normalization and parsing helpers.

// ================= proxy parsing =================

pub(crate) fn proto_label(scheme: &str) -> String {
    match scheme.to_lowercase().as_str() {
        "http" => "HTTP".to_string(),
        "https" => "HTTPS".to_string(),
        "socks5" | "socks5h" | "socks" => "SOCKS5".to_string(),
        "socks4" | "socks4a" => "SOCKS4".to_string(),
        other => other.to_uppercase(),
    }
}

pub(crate) fn canonical_scheme(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "socks" => "socks5".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn is_proto_name(s: &str) -> bool {
    matches!(
        s.to_lowercase().as_str(),
        "http" | "https" | "socks" | "socks4" | "socks4a" | "socks5" | "socks5h"
    )
}

/// Remove `user:pass@` from a proxy URL without touching the real scheme.
/// Used for cache keys, webhook payloads and UI status strings.
pub(crate) fn strip_credentials(raw: &str) -> String {
    let Some((_, host)) = raw.rsplit_once('@') else {
        return raw.to_string();
    };
    if host.is_empty() {
        return raw.to_string();
    }
    match raw.split_once("://") {
        Some((scheme, _)) => format!("{scheme}://{host}"),
        None => host.to_string(),
    }
}

/// Geo cache key for a proxy URL: `scheme://host:port`, plus a hash of the
/// username when there is one. Gateways choose the exit country by username
/// (`user-country-de`), so two logins on one host are two proxies; the
/// password never goes into the key. `geoKey` in geo-store.ts must match.
pub(crate) fn geo_key(url: &str) -> String {
    let base = strip_credentials(url);
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    match rest
        .rsplit_once('@')
        .map(|(info, _)| info.split(':').next().unwrap_or(""))
    {
        Some(user) if !user.is_empty() => format!("{base}#u{:08x}", fnv1a32(user.as_bytes())),
        _ => base,
    }
}

/// 32-bit FNV-1a: short, stable, and easy to repeat in TypeScript.
fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |h, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

pub(crate) fn is_http_url(s: &str) -> bool {
    let s = s.trim();
    s.get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
        || s.get(..8)
            .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
}

pub(crate) fn valid_port(p: &str) -> bool {
    if p.is_empty() || p.len() > 5 || !p.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match p.parse::<u32>() {
        Ok(n) => (1..=65535).contains(&n),
        Err(_) => false,
    }
}

pub(crate) fn clean_token(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == '<' || c == '>')
        .trim()
        .to_string()
}

pub(crate) fn strip_list_prefix(s: &str) -> &str {
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

/// Heuristic for ambiguous four-column rows. A host usually looks like an
/// IP address or contains a dot; usernames are commonly bare words.
pub(crate) fn looks_like_host(s: &str) -> bool {
    let t = s.trim().trim_matches(|c| c == '[' || c == ']');
    if t.is_empty() {
        return false;
    }
    if t.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    t.contains('.') || t.contains(':') || t.eq_ignore_ascii_case("localhost")
}

/// `true` means `host,port,user,pass`, `false` means `user,pass,host,port`.
/// When both interpretations have a valid port (a numeric password is
/// common), the host-looking column wins.
fn four_columns_host_first(p: &[&str]) -> Option<bool> {
    let host_first = valid_port(p[1]) && !p[0].is_empty();
    let host_last = valid_port(p[3]) && !p[2].is_empty();
    match (host_first, host_last) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        (true, true) => Some(!(looks_like_host(p[2]) && !looks_like_host(p[0]))),
        _ => None,
    }
}

pub(crate) fn from_columns(parts: &[String], default_proto: &str) -> Option<(String, String)> {
    let def = match default_proto.to_lowercase().as_str() {
        "socks5" => "socks5",
        "socks4" => "socks4",
        _ => "http",
    };
    match parts.len() {
        2 => {
            let (host, port) = (parts[0].trim(), parts[1].trim());
            build_url(def, None, host, port)
        }
        3 => {
            let a = parts[0].trim();
            let b = parts[1].trim();
            let c = parts[2].trim();
            if valid_port(b) && is_proto_name(c) {
                return build_url(&canonical_scheme(c), None, a, b);
            }
            if is_proto_name(a) && valid_port(c) {
                return build_url(&canonical_scheme(a), None, b, c);
            }
            // `host:port user pass`.
            if let Some((host, port)) = a
                .rsplit_once(':')
                .filter(|(h, p)| !h.is_empty() && valid_port(p))
            {
                if !b.is_empty() && !c.is_empty() {
                    return build_url(def, Some(&format!("{b}:{c}")), host, port);
                }
            }
            None
        }
        4 => {
            let p: Vec<&str> = parts.iter().map(|x| x.trim()).collect();
            // A table row (`1.2.3.4 8080 US elite`): no login in it.
            if valid_port(p[1]) && looks_like_host(p[0]) && is_table_cell(p[2]) {
                return build_url(def, None, p[0], p[1]);
            }
            if let Some(host_first) = four_columns_host_first(&p) {
                return if host_first {
                    build_url(def, Some(&format!("{}:{}", p[2], p[3])), p[0], p[1])
                } else {
                    build_url(def, Some(&format!("{}:{}", p[0], p[1])), p[2], p[3])
                };
            }
            None
        }
        5 => {
            let p: Vec<&str> = parts.iter().map(|x| x.trim()).collect();
            if is_proto_name(p[0]) && valid_port(p[2]) {
                return build_url(
                    &canonical_scheme(p[0]),
                    Some(&format!("{}:{}", p[3], p[4])),
                    p[1],
                    p[2],
                );
            }
            None
        }
        _ => None,
    }
}

/// `user:pass` as a URL may carry it. Passwords from column formats hold
/// `/`, `?`, `#` or `@`, which would end or split the URL, so every character
/// a URL's userinfo may not hold is percent-encoded; an existing `%XX` is kept,
/// so a URL that was already encoded stays as it is. The dispatcher and
/// reqwest decode it again.
pub(crate) fn encode_userinfo(userinfo: &str) -> String {
    let b = userinfo.as_bytes();
    let hex = |c: u8| c.is_ascii_hexdigit();
    let mut out = String::with_capacity(b.len());
    for (i, &c) in b.iter().enumerate() {
        // Userinfo may hold `:`: the first one separates user and password,
        // later ones belong to the password.
        let keep = c.is_ascii_alphanumeric()
            || b"-._~!$&'()*+,;=:".contains(&c)
            || (c == b'%'
                && b.get(i + 1).is_some_and(|&x| hex(x))
                && b.get(i + 2).is_some_and(|&x| hex(x)));
        if keep {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

/// Whether a delimiter-free reading of the whole token is a proxy:
/// `host:port:user:pass` (or `user:pass:host:port`), or `user:pass@host:port`.
/// Such a token is not split at `,;|` and tabs, which a password may hold.
fn whole_token_is_proxy(s: &str) -> bool {
    if s.chars().any(char::is_whitespace) {
        return false;
    }
    if let Some((_, host)) = s.rsplit_once('@') {
        return host
            .rsplit_once(':')
            .is_some_and(|(h, p)| !h.is_empty() && valid_port(p));
    }
    let p: Vec<&str> = s.split(':').collect();
    p.len() == 4
        && ((valid_port(p[1]) && !p[0].is_empty()) || (valid_port(p[3]) && !p[2].is_empty()))
}

/// A column of a proxy table that is not a login: a country code or name
/// column's code, an anonymity or protocol word, a yes/no flag.
pub(crate) fn is_table_cell(s: &str) -> bool {
    let t = s.trim();
    let country_code = t.len() == 2 && t.chars().all(|c| c.is_ascii_uppercase());
    country_code
        || [
            "elite",
            "anonymous",
            "anon",
            "transparent",
            "high",
            "high-anonymous",
            "yes",
            "no",
            "-",
            "http",
            "https",
            "socks4",
            "socks5",
        ]
        .iter()
        .any(|w| t.eq_ignore_ascii_case(w))
}

pub(crate) fn build_url(
    scheme: &str,
    userinfo: Option<&str>,
    host: &str,
    port: &str,
) -> Option<(String, String)> {
    let host = host.trim();
    if host.is_empty() || !valid_port(port) {
        return None;
    }
    // Column/CSV input often contains bare IPv6 addresses. Bracket them so
    // the resulting URL is actually parseable.
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let scheme = canonical_scheme(scheme);
    let url = match userinfo {
        Some(u) if !u.is_empty() => format!("{scheme}://{}@{host}:{port}", encode_userinfo(u)),
        _ => format!("{scheme}://{host}:{port}"),
    };
    let label = proto_label(&scheme);
    Some((url, label))
}

pub(crate) fn normalize_proxy(raw: &str, default_proto: &str) -> Option<(String, String)> {
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

    if (s.contains(',') || s.contains(';') || s.contains('|') || s.contains('\t'))
        && !whole_token_is_proxy(&s)
    {
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
        // Only schemes the checker can speak: a `vless://`, `trojan://` or
        // `ss://` link is not a proxy it can check, and passing it on would
        // make the check go out directly (see `http::reqwest_proxy_url`).
        Some((sch, rest)) if is_proto_name(sch.trim()) => {
            (Some(sch.trim().to_lowercase()), rest.to_string())
        }
        Some(_) => return None,
        None => (None, s.clone()),
    };
    // A path, query or fragment ends the address, but only after the host: a
    // password may hold `/`, `?` or `#`, and column formats carry it last.
    let host_from = authority.rfind('@').map_or(0, |at| at + 1);
    let columns = host_from == 0 && !authority.contains('[') && authority.matches(':').count() >= 3;
    if !columns {
        if let Some(pos) = authority[host_from..].find(['/', '?', '#']) {
            authority.truncate(host_from + pos);
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
    // `host:port@user:pass`: the address first, the login after the @.
    let port_of = |s: &str| {
        s.rsplit_once(':')
            .is_some_and(|(h, p)| !h.is_empty() && valid_port(p))
    };
    let (userinfo, hostport) = match userinfo {
        Some(u) if !port_of(&hostport) && port_of(&u) && !u.contains('[') => (Some(hostport), u),
        other => (other, hostport),
    };
    if hostport.is_empty() {
        return None;
    }
    // Bare IPv6 with a port (`fe80::1:8080`) cannot be told from an address:
    // brackets are required.
    if !hostport.starts_with('[') && hostport.contains("::") {
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
                if tail.len() == 3 && is_proto_name(tail[2]) {
                    let scheme = canonical_scheme(tail[2]);
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
            if let Some(host_first) = four_columns_host_first(&parts) {
                let scheme = scheme_for();
                return if host_first {
                    let u = format!("{}:{}", parts[2], parts[3]);
                    build_url(&scheme, Some(&u), parts[0], parts[1])
                } else {
                    let u = format!("{}:{}", parts[0], parts[1]);
                    build_url(&scheme, Some(&u), parts[2], parts[3])
                };
            }
            None
        }
        _ => None,
    }
}

pub(crate) fn normalize_candidates(raw: &str, default_proto: &str) -> Vec<(String, String)> {
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
pub(crate) fn host_port_of_url(url: &str) -> Option<(String, u16)> {
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
