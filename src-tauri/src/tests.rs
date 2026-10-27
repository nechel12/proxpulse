use crate::commands::check::{candidate_budget, dead_result, CheckFlags};
use crate::commands::dispatch::{
    abort_connections, handle_http, handle_socks5, is_self_literal, is_self_upstream,
    is_self_upstream_async, parse_target, pick_order, pool_items, store_pool, upstream_from_item,
    CountingIo, DispatchItem,
};
use crate::tls::{is_cert_error, tls_client_config};
use crate::transport::*;
use crate::*;
use std::io::{Error, ErrorKind};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::TcpStream;

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
        short_error("error sending request for url (https://example.com/): socks connect failed"),
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
