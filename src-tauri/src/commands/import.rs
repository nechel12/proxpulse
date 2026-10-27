use crate::{read_body_capped, short_error, BODY_LIMIT_IMPORT};
use std::net::{IpAddr, SocketAddr};

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
        .timeout(std::time::Duration::from_secs(20))
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
pub(crate) async fn fetch_url_text(url: String) -> Result<String, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
