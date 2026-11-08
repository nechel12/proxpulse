use crate::transport::{host_for_connect, parse_upstream_url};
use crate::*;

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

/// Lines as they come from real lists (sellers, free-proxy sites, configs),
/// and the URL each must become; `None` where a line is ambiguous.
#[test]
fn real_list_fixtures() {
    let url = |raw: &str| norm(raw, "http").map(|(u, _)| u);
    for (raw, want) in [
        // Passwords keep their special characters, encoded for the URL.
        (
            "1.2.3.4:8080:user:pa/ss",
            Some("http://user:pa%2Fss@1.2.3.4:8080"),
        ),
        (
            "1.2.3.4:8080:user:pa?s#s",
            Some("http://user:pa%3Fs%23s@1.2.3.4:8080"),
        ),
        (
            "1.2.3.4:8080:user:pass!",
            Some("http://user:pass!@1.2.3.4:8080"),
        ),
        (
            "1.2.3.4:8080:user:pa;ss",
            Some("http://user:pa;ss@1.2.3.4:8080"),
        ),
        (
            "user:p/ss@1.2.3.4:8080",
            Some("http://user:p%2Fss@1.2.3.4:8080"),
        ),
        (
            "user:p@ss@1.2.3.4:8080",
            Some("http://user:p%40ss@1.2.3.4:8080"),
        ),
        (
            "http://user:p%40ss@1.2.3.4:8080/",
            Some("http://user:p%40ss@1.2.3.4:8080"),
        ),
        (
            "user:pässword@1.2.3.4:8080",
            Some("http://user:p%C3%A4ssword@1.2.3.4:8080"),
        ),
        // The address first, the login after the @.
        (
            "1.2.3.4:8080@user:pass",
            Some("http://user:pass@1.2.3.4:8080"),
        ),
        // A login after a host:port.
        (
            "1.2.3.4:8080 user pass",
            Some("http://user:pass@1.2.3.4:8080"),
        ),
        // Table rows: country and anonymity columns are not a login.
        ("1.2.3.4 8080 US elite", Some("http://1.2.3.4:8080")),
        (
            "1.2.3.4 8080 user pass",
            Some("http://user:pass@1.2.3.4:8080"),
        ),
        // Bare IPv6 with a port is ambiguous: brackets are required.
        ("fe80::1:8080", None),
        ("[fe80::1]:8080", Some("http://[fe80::1]:8080")),
    ] {
        assert_eq!(url(raw).as_deref(), want, "{raw}");
    }
    // The dispatcher reads the encoded password back as it was.
    let (_, host, port, user, pass) =
        parse_upstream_url("http://user:pa%2Fss@1.2.3.4:8080").unwrap();
    assert_eq!(
        (host.as_str(), port, user.as_deref(), pass.as_deref()),
        ("1.2.3.4", 8080, Some("user"), Some("pa/ss"))
    );
}
