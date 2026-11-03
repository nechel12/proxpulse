use crate::http::short_error;
use crate::tls::tls_client_config;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// IPv4 address carried in the low 32 bits of an IPv6 address.
fn low_v4(s: &[u16; 8]) -> Ipv4Addr {
    Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8)
}

pub(crate) fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                // 0.0.0.0/8 ("this network"), not only 0.0.0.0 itself
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xC0) == 64)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && (o[1] & 0xFE) == 18)
                || o[0] >= 240
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xFE00) == 0xFC00
                || (s[0] & 0xFFC0) == 0xFE80
                // deprecated site-local fec0::/10, still routed by some stacks
                || (s[0] & 0xFFC0) == 0xFEC0
                // documentation 2001:db8::/32 and local-use NAT64 64:ff9b:1::/48
                || (s[0] == 0x2001 && s[1] == 0x0db8)
                || (s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 0x0001)
            {
                return true;
            }
            // Prefixes that embed an IPv4 address reach it through a gateway,
            // so they inherit its verdict: IPv4-mapped/-compatible (::/96),
            // well-known NAT64 (64:ff9b::/96) and 6to4 (2002::/16).
            let embedded = if let Some(v4) = v6.to_ipv4() {
                Some(v4)
            } else if s[..6] == [0x0064, 0xff9b, 0, 0, 0, 0] {
                Some(low_v4(&s))
            } else if s[0] == 0x2002 {
                Some(Ipv4Addr::new(
                    (s[1] >> 8) as u8,
                    s[1] as u8,
                    (s[2] >> 8) as u8,
                    s[2] as u8,
                ))
            } else {
                None
            };
            embedded.is_some_and(|v4| is_disallowed_ip(IpAddr::V4(v4)))
        }
    }
}

pub(crate) fn is_disallowed_url_host(url: &reqwest::Url) -> bool {
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

/// Validate a user-supplied URL and resolve its host. DNS must not point to a
/// private address, because the URL import and webhooks must not reach local
/// services. Returns the address to pin for the request, if any.
pub(crate) async fn resolve_public_target(
    url: &reqwest::Url,
) -> Result<Option<(String, SocketAddr)>, String> {
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
    if host.parse::<IpAddr>().is_ok() {
        return Ok(None);
    }
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
    Ok(Some((host, addr)))
}

/// Client for user-supplied URLs: ignores the ambient system proxy, reuses the
/// pinned rustls config and never follows redirects (callers revalidate hops).
pub(crate) fn guarded_client_builder(timeout: Duration) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(timeout)
        .no_proxy()
        .tls_backend_preconfigured((*tls_client_config()).clone())
        .user_agent(concat!("ProxPulse/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
}

pub(crate) async fn build_guarded_client(
    url: &reqwest::Url,
    timeout: Duration,
) -> Result<reqwest::Client, String> {
    let mut builder = guarded_client_builder(timeout);
    if let Some((host, addr)) = resolve_public_target(url).await? {
        builder = builder.resolve(&host, addr);
    }
    builder.build().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_targets() {
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
            "fec0::1",
            "::ffff:127.0.0.1",
            // 0.0.0.0/8, not only 0.0.0.0
            "0.1.2.3",
            // IPv4-compatible, NAT64 and 6to4 forms of private addresses
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b:1::1",
            "2002:c0a8:0101::1",
            "2002:7f00:0001::",
            "2001:db8::1",
        ] {
            assert!(is_disallowed_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "1.1.1.1",
            "2606:4700:4700::1111",
            // the same gateways in front of public addresses stay allowed
            "64:ff9b::101:101",
            "2002:0101:0101::1",
        ] {
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
    async fn resolver_rejects_private_literals() {
        let url = reqwest::Url::parse("http://127.0.0.1:9/hook").unwrap();
        assert_eq!(
            resolve_public_target(&url).await.unwrap_err(),
            "private address"
        );
    }
}
