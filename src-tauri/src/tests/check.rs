use crate::commands::check::{
    candidate_budget, dead_result, is_tls_target, wants_direct_ip, CheckFlags,
};
use crate::tls::{is_cert_error, tls_client_config};
use crate::*;
use std::collections::HashMap;
use std::io::{Error, ErrorKind};
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
