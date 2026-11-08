use crate::commands::check::{
    candidate_budget, dead_result, is_tls_target, wants_direct_ip, CheckConfig, CheckFlags,
};
use crate::tls::{cert_fp_hex, is_cert_error, tls_client_config};
use crate::*;
use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use std::time::Duration;

fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn deep_checks_use_plain_http() {
    // Over https:// a proxy only tunnels and can neither add headers nor
    // modify the body, so these probes must be plain HTTP.
    assert!(ANON_URL.starts_with("http://"));
    assert!(ANON_URL.contains("show_env=1"));
    assert!(TAMPER_URL.starts_with("http://"));
    assert!(is_tls_target("HTTPS://judge.example.com/judge"));
    assert!(is_tls_target("  https://x"));
    assert!(!is_tls_target("http://judge.example.com/judge"));
}

#[test]
fn direct_ip_is_requested_only_where_a_leak_is_visible() {
    let flags = |judge: bool, anon: bool| CheckFlags {
        geo: false,
        anon,
        tamper: false,
        tls: false,
        precheck: false,
        precheck_timeout: Duration::from_millis(300),
        judge,
        ipapi_key: None,
        sites: Arc::from(Vec::<String>::new()),
        judge_http: None,
        dnsbl: false,
    };
    // An HTTPS judge never sees proxy headers: it does not get our IP.
    assert!(!wants_direct_ip(
        &flags(true, false),
        "https://judge.example.com/judge"
    ));
    assert!(wants_direct_ip(
        &flags(true, false),
        "http://judge.example.com/judge"
    ));
    assert!(wants_direct_ip(
        &flags(false, true),
        "https://www.google.com/generate_204"
    ));
    assert!(!wants_direct_ip(
        &flags(false, false),
        "https://www.google.com/generate_204"
    ));
}

#[test]
fn anonymity_classification() {
    let me = "203.0.113.7";
    // httpbin's load balancer appends the connecting address (the proxy exit)
    // and adds X-Forwarded-Proto; neither is a proxy telltale.
    let elite = headers(&[
        ("Host", "httpbin.org"),
        ("X-Forwarded-For", "198.51.100.1"),
        ("X-Forwarded-Proto", "http"),
    ]);
    assert_eq!(classify_anonymity(&elite, me).as_deref(), Some("elite"));

    let via = headers(&[("Via", "1.1 squid"), ("X-Forwarded-For", "198.51.100.1")]);
    assert_eq!(classify_anonymity(&via, me).as_deref(), Some("anonymous"));

    let fake_xff = headers(&[("X-Forwarded-For", "10.1.2.3,198.51.100.1")]);
    assert_eq!(
        classify_anonymity(&fake_xff, me).as_deref(),
        Some("anonymous")
    );

    let leak_xff = headers(&[("X-Forwarded-For", "203.0.113.7, 198.51.100.1")]);
    assert_eq!(
        classify_anonymity(&leak_xff, me).as_deref(),
        Some("transparent")
    );

    let leak_real_ip = headers(&[("X-Real-Ip", "203.0.113.7")]);
    assert_eq!(
        classify_anonymity(&leak_real_ip, me).as_deref(),
        Some("transparent")
    );

    let leak_forwarded = headers(&[("Forwarded", "for=203.0.113.7:4711;proto=http")]);
    assert_eq!(
        classify_anonymity(&leak_forwarded, me).as_deref(),
        Some("transparent")
    );

    // Whole-token match only: 203.0.113.7 is not inside 203.0.113.77.
    let similar = headers(&[("X-Forwarded-For", "203.0.113.77,198.51.100.1")]);
    assert_eq!(
        classify_anonymity(&similar, me).as_deref(),
        Some("anonymous")
    );

    assert_eq!(classify_anonymity(&elite, "not an ip"), None);
}

#[test]
fn tls_config_builds() {
    // must not panic: with several rustls providers compiled in,
    // auto-detect would blow up (see CryptoProvider docs)
    let _ = tls_client_config();
}

#[test]
fn short_error_mapping() {
    // Texts of the causes reqwest and hyper-util report through source().
    assert_eq!(short_error("operation timed out"), "timeout");
    assert_eq!(
        short_error("SOCKS5 connection refused"),
        "connection refused"
    );
    assert_eq!(
        short_error("SOCKS error: io error during SOCKS handshake"),
        "socks error"
    );
    assert_eq!(
        short_error("tunnel error: proxy authorization required"),
        "proxy auth required"
    );
    assert_eq!(short_error("tunnel error: unsuccessful"), "proxy error");
    assert!(short_error(&"x".repeat(500)).len() <= 160);
}

/// A URL with a secret in it, as ip-api Pro, Telegram and the judge use.
const SECRET_URL_PATH: &str =
    "/bot123456:SECRET-token/sendMessage?key=SECRET&direct_ip=203.0.113.9";

#[tokio::test]
async fn request_errors_name_the_cause_not_the_url() {
    // Nothing listens on a port that was just freed: the connection is refused.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let client = build_client(None, Duration::from_secs(5), true).unwrap();
    let err = client
        .get(format!("http://127.0.0.1:{port}{SECRET_URL_PATH}"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(request_error(&err), "connection refused");

    // A server that accepts and never answers: the client timeout.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = silent.local_addr().unwrap();
    let hold = tokio::spawn(async move {
        let _conn = silent.accept().await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let client = build_client(None, Duration::from_millis(300), true).unwrap();
    let err = client
        .get(format!("http://{addr}{SECRET_URL_PATH}"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(request_error(&err), "timeout");
    assert!(!request_error(&err).contains("SECRET"));
    hold.abort();
}

#[test]
fn socket_errors_map_by_kind_not_by_text() {
    // Windows words a refused connection differently, and localizes it.
    let refused = Error::new(ErrorKind::ConnectionRefused, "Подключение не установлено");
    assert_eq!(io_error(&refused), "connection refused");
    assert_eq!(io_error(&Error::new(ErrorKind::TimedOut, "x")), "timeout");
    assert_eq!(
        io_error(&Error::new(ErrorKind::BrokenPipe, "x")),
        "connection reset"
    );
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

    // Pool countries carry no IP: they apply whatever exit IP was seen.
    let pool_entry = GeoCacheEntry {
        ip: None,
        city: None,
        ..entry
    };
    let mut seen_ip = dead_result("1.2.3.4:8080".into(), "HTTP".into(), 0, String::new());
    seen_ip.ip = Some("9.9.9.9".into());
    assert!(apply_cached_geo(&mut seen_ip, &pool_entry));
    assert_eq!(seen_ip.ip.as_deref(), Some("9.9.9.9"));
    assert_eq!(seen_ip.country_code.as_deref(), Some("TL"));
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
        sites: Arc::from(Vec::<String>::new()),
        judge_http: None,
        dnsbl: false,
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

    // Sites are requested after the deep stages: one more timeout.
    let with_sites = CheckFlags {
        sites: Arc::from(vec!["https://www.youtube.com".to_string()]),
        ..no_tls.clone()
    };
    assert_eq!(
        candidate_budget(timeout, 1, &with_sites),
        Duration::from_millis(26_500)
    );

    let judge = CheckFlags {
        judge: true,
        tls: true,
        ..flags
    };
    assert_eq!(
        candidate_budget(timeout, 1, &judge),
        Duration::from_millis(26_500)
    );
}

#[test]
fn baseline_cache_ttl() {
    assert!(cache_fresh(Instant::now(), Duration::from_secs(1)));
    let old = Instant::now() - Duration::from_secs(2);
    assert!(!cache_fresh(old, Duration::from_secs(1)));
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
fn certificate_fingerprint_is_lowercase_sha256_hex() {
    assert_eq!(
        cert_fp_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        cert_fp_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn skipped_deep_checks_are_recorded() {
    use crate::commands::check::Deep;
    let mut skipped = Vec::new();
    assert_eq!(Deep::Ran(Some(1)).value("geo", &mut skipped), Some(1));
    assert_eq!(Deep::<u8>::Ran(None).value("anon", &mut skipped), None);
    assert_eq!(Deep::<u8>::Skipped.value("tls", &mut skipped), None);
    assert_eq!(skipped, vec!["tls"], "only a skipped stage is reported");
}

#[test]
fn check_events_are_tagged() {
    use crate::commands::check::CheckEvent;
    let r = dead_result("1.2.3.4:80".into(), "HTTP".into(), 1, "timeout".into());
    let v = serde_json::to_value(CheckEvent::Result(Box::new(r))).unwrap();
    assert_eq!(v["kind"], "result");
    assert_eq!(v["proxy"], "1.2.3.4:80");
    assert_eq!(v["skipped"], serde_json::json!([]));
    assert_eq!(
        serde_json::to_value(CheckEvent::Done).unwrap(),
        serde_json::json!({ "kind": "done" })
    );
    assert_eq!(
        serde_json::to_value(CheckEvent::KeyOverHttp).unwrap(),
        serde_json::json!({ "kind": "keyOverHttp" })
    );
}

#[test]
fn site_lists_are_sanitized() {
    let cfg = CheckConfig {
        sites: vec![
            " https://www.youtube.com ".to_string(),
            "https://www.youtube.com".to_string(),
            "ftp://x".to_string(),
            "telegram.org".to_string(),
        ]
        .into_iter()
        .chain((0..20).map(|i| format!("https://s{i}.example.com")))
        .collect(),
        ..CheckConfig::default()
    }
    .sanitized();
    assert_eq!(cfg.sites[0], "https://www.youtube.com");
    assert_eq!(cfg.sites.len(), crate::commands::check::MAX_SITES);
    assert!(cfg.sites.iter().all(|s| s.starts_with("https://")));
}

#[test]
fn speed_test_reads_the_response_head() {
    use crate::commands::speed::{mbps, parse_head};
    let head = b"HTTP/1.1 200 OK\r\nContent-Type: x\r\nContent-Length: 5000000\r\n\r\nabc";
    assert_eq!(
        parse_head(head),
        Some((head.len() - 3, 200, Some(5_000_000)))
    );
    assert_eq!(parse_head(b"HTTP/1.1 200 OK\r\nX: y\r\n"), None);
    assert_eq!(
        parse_head(b"HTTP/1.1 403 Forbidden\r\n\r\n"),
        Some((26, 403, None))
    );
    assert_eq!(mbps(5_000_000, Duration::from_secs(4)), Some(10.0));
    assert_eq!(mbps(0, Duration::from_secs(1)), None);
}

#[test]
fn dnsbl_names_and_answers() {
    use crate::dnsbl::{interpret, query_name, Answer};
    use std::net::{IpAddr, Ipv4Addr};
    assert_eq!(
        query_name(Ipv4Addr::new(1, 2, 3, 4), "zen.spamhaus.org"),
        "4.3.2.1.zen.spamhaus.org"
    );
    let ip = |a, b, c, d| IpAddr::V4(Ipv4Addr::new(a, b, c, d));
    assert_eq!(interpret(&[ip(127, 0, 0, 2)]), Answer::Listed);
    assert_eq!(interpret(&[]), Answer::NotListed);
    // Spamhaus refuses queries from public resolvers with 127.255.255.x.
    assert_eq!(interpret(&[ip(127, 255, 255, 254)]), Answer::Unknown);
    assert_eq!(interpret(&[ip(10, 0, 0, 1)]), Answer::Unknown);
}

#[tokio::test]
async fn dead_results_keep_the_normalized_url() {
    // A closed local port: the precheck fails without any outside request.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let cfg = CheckConfig {
        test_url: "http://127.0.0.1:9/".to_string(),
        with_geo: false,
        with_anonymity: false,
        with_tamper: false,
        with_tls: false,
        ..CheckConfig::default()
    };
    let raw = format!("127.0.0.1:{port}");
    let out = crate::commands::check::run_checks(
        vec![raw.clone()],
        cfg,
        Vec::new(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        None,
    )
    .await
    .unwrap();
    assert_eq!(out.len(), 1);
    assert!(!out[0].alive);
    assert_eq!(out[0].proxy, raw);
    // The history keys alive results by this URL; dead ones must match.
    assert_eq!(out[0].url, format!("http://127.0.0.1:{port}"));
}

#[tokio::test]
async fn dnsbl_skips_ipv6_and_garbage() {
    assert!(crate::dnsbl::check_ip("2001:db8::1").await.is_none());
    assert!(crate::dnsbl::check_ip("not an ip").await.is_none());
}
