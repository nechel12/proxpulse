use crate::extract::*;

fn extract(text: &str) -> Vec<String> {
    extract_proxies(text, usize::MAX, "http")
}

#[test]
fn build_from_parts_rules() {
    assert_eq!(
        build_from_parts("1.2.3.4", "8080", None, None, None).as_deref(),
        Some("1.2.3.4:8080")
    );
    for port in ["0", "99999", "abc"] {
        assert_eq!(build_from_parts("1.2.3.4", port, None, None, None), None);
    }
    assert_eq!(build_from_parts("", "8080", None, None, None), None);
    assert_eq!(
        build_from_parts("1.2.3.4", "8080", Some("u"), Some("p"), Some("socks5")).as_deref(),
        Some("socks5://u:p@1.2.3.4:8080")
    );
    assert_eq!(
        build_from_parts("1.2.3.4", "8080", Some("u"), None, None).as_deref(),
        Some("u@1.2.3.4:8080")
    );
    assert_eq!(
        build_from_parts("2001:db8::1", "8080", None, None, None).as_deref(),
        Some("[2001:db8::1]:8080")
    );
    assert_eq!(
        build_from_parts("[2001:db8::1]", "8080", None, None, None).as_deref(),
        Some("[2001:db8::1]:8080")
    );
}

#[test]
fn csv_lines() {
    let csv = |s: &str| try_csv_line(s);
    assert_eq!(csv("1.2.3.4,8080").as_deref(), Some("1.2.3.4:8080"));
    assert_eq!(csv("1.2.3.4,8080,u,p").as_deref(), Some("u:p@1.2.3.4:8080"));
    assert_eq!(csv("u,p,1.2.3.4,8080").as_deref(), Some("u:p@1.2.3.4:8080"));
    assert_eq!(csv("1.2.3.4:8080:socks5"), None, "no delimiter, not csv");
    assert_eq!(
        csv("1.2.3.4,8080,socks5").as_deref(),
        Some("socks5://1.2.3.4:8080")
    );
    assert_eq!(
        csv("socks5,1.2.3.4,8080").as_deref(),
        Some("socks5://1.2.3.4:8080")
    );
    assert_eq!(csv("1.2.3.4 8080 u p").as_deref(), Some("u:p@1.2.3.4:8080"));
    assert_eq!(
        csv("socks5,1.2.3.4,8080,u,p").as_deref(),
        Some("socks5://u:p@1.2.3.4:8080")
    );
    // A numeric password keeps the documented column order.
    assert_eq!(
        csv("user,1234,1.2.3.4,9090").as_deref(),
        Some("user:1234@1.2.3.4:9090")
    );
    assert_eq!(
        csv("1.2.3.4,9090,user,1234").as_deref(),
        Some("user:1234@1.2.3.4:9090")
    );
    assert_eq!(
        csv("user,1234,proxy.example.com,9090").as_deref(),
        Some("user:1234@proxy.example.com:9090")
    );
    assert_eq!(
        csv("proxy.example.com,9090,user,1234").as_deref(),
        Some("user:1234@proxy.example.com:9090")
    );
    assert_eq!(
        csv("2001:db8::1,8080").as_deref(),
        Some("[2001:db8::1]:8080")
    );
    assert_eq!(csv("hello world foo"), None);
    assert_eq!(csv("single"), None);
}

#[test]
fn tails_and_scheme_urls() {
    assert_eq!(clean_tail("1.2.3.4:8080,"), "1.2.3.4:8080");
    assert_eq!(clean_tail("(1.2.3.4:8080)"), "1.2.3.4:8080");
    assert_eq!(
        scheme_urls("use socks5://9.9.9.9:1080, or HTTP://1.1.1.1:80)"),
        vec!["socks5://9.9.9.9:1080", "HTTP://1.1.1.1:80"]
    );
    // The scheme must be a whole word.
    assert!(scheme_urls("xhttp://1.1.1.1:80 ftp://x").is_empty());
    assert_eq!(
        scheme_urls("foo://socks5://1.2.3.4:80"),
        vec!["socks5://1.2.3.4:80"]
    );
}

#[test]
fn json_inputs() {
    let parse = |s: &str| {
        let mut out = Vec::new();
        proxies_from_json(&serde_json::from_str(s).unwrap(), &mut out, usize::MAX);
        out
    };
    assert_eq!(
        parse(r#"{"host":"1.2.3.4","port":8080,"username":"u","password":"p"}"#),
        vec!["u:p@1.2.3.4:8080"]
    );
    assert_eq!(
        parse(r#"{"data":{"proxies":["1.2.3.4:8080","5.6.7.8:3128"]}}"#),
        vec!["1.2.3.4:8080", "5.6.7.8:3128"]
    );
    assert_eq!(
        parse(r#"[{"IP":"1.2.3.4","Port":"8080","Type":"SOCKS5"}]"#),
        vec!["socks5://1.2.3.4:8080"]
    );
    assert_eq!(
        extract(r#"[{"ip":"1.2.3.4","port":8080}]"#),
        vec!["1.2.3.4:8080"]
    );
    assert!(extract(r#"{"version":"1.0.0"}"#).is_empty());
}

#[test]
fn text_lists() {
    assert_eq!(
        extract("# comment\n1) 1.2.3.4:8080\n- user:pass@5.6.7.8:3128 # home\n\n// xx"),
        vec!["1.2.3.4:8080", "user:pass@5.6.7.8:3128"]
    );
    assert_eq!(
        extract("[x] 1.2.3.4:8080\r\n• 5.6.7.8:3128\r\n"),
        vec!["1.2.3.4:8080", "5.6.7.8:3128"]
    );
    assert_eq!(extract("1.2.3.4:8080\n1.2.3.4:8080"), vec!["1.2.3.4:8080"]);
    assert!(extract("use socks5://9.9.9.9:1080 please").contains(&"socks5://9.9.9.9:1080".into()));
    assert!(extract("   \n # nothing\n").is_empty());
    assert!(extract("ab").is_empty());
    assert_eq!(
        extract("1.2.3.4:8080, comment\n5.6.7.8:3128; note\nuser:pass@9.9.9.9:1080 here"),
        vec!["1.2.3.4:8080", "5.6.7.8:3128", "user:pass@9.9.9.9:1080"]
    );
    assert!(extract("1.2.3.4:0\n1.2.3.4:65536\n://host:80").is_empty());
    assert!(extract("hello world").is_empty());
    assert_eq!(
        extract("1.2.3.4:8080:user:pass\nuser2:pass:1.2.3.4:8080\n1.2.3.4:8080:socks5"),
        vec![
            "1.2.3.4:8080:user:pass",
            "user2:pass:1.2.3.4:8080",
            "1.2.3.4:8080:socks5"
        ]
    );
    // Both column orders describe the same proxy: one check, not two.
    assert_eq!(
        extract("1.2.3.4:8080:user:pass\nuser:pass:1.2.3.4:8080"),
        vec!["1.2.3.4:8080:user:pass"]
    );
    assert_eq!(extract("use 1.2.3.4:8080 here"), vec!["1.2.3.4:8080"]);
    assert_eq!(
        extract("x 11.2.3.45:80a 1.2.3.4:8080."),
        vec!["1.2.3.4:8080"]
    );
}

#[test]
fn only_checkable_entries_are_counted() {
    // Rejected by the checker's normalization, so not counted either.
    assert!(extract("http://:8080").is_empty());
    // Bare IPv6 is ambiguous and unsupported; brackets work.
    assert!(extract("2001:db8::1:8080").is_empty());
    assert_eq!(extract("[2001:db8::1]:8080"), vec!["[2001:db8::1]:8080"]);
    // One proxy written two ways is one check.
    assert_eq!(
        extract("1.2.3.4:8080\nhttp://1.2.3.4:8080\nsocks5://1.2.3.4:8080"),
        vec!["1.2.3.4:8080", "socks5://1.2.3.4:8080"]
    );
    // The default type decides what a scheme-less entry means.
    assert_eq!(
        extract_proxies("1.2.3.4:8080\nsocks5://1.2.3.4:8080", 10, "socks5"),
        vec!["1.2.3.4:8080"]
    );
}

#[test]
fn stops_at_the_limit() {
    let text = "1.1.1.1:80\n2.2.2.2:80\n3.3.3.3:80";
    assert_eq!(
        extract_proxies(text, 2, "http"),
        vec!["1.1.1.1:80", "2.2.2.2:80"]
    );
    assert!(extract_proxies(text, 0, "http").is_empty());
}
