use crate::commands::check::{candidate_budget, wants_direct_ip, CheckConfig};
use crate::judge_sig::*;
use std::time::Duration;

/// The judge's own test vector (proxpulse-judge `sign::tests::known_vector`):
/// a signature made by its Ed25519 implementation must verify here.
const PUBLIC: &str = "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=";
const SIG: &str =
    "bKJP4sGEk2X3TrUbVqV0thXJWq21c5/D/Dg+mP3Tv+BJ6mbj7nJ5r9oOUXwFITETF1oT8/78dJSUh32a964eAA==";
const NONCE: &str = "0123456789abcdef01234567";
const BODY: &[u8] = b"{\"anonymity\":\"elite\"}";

fn key() -> JudgeKey {
    parse_key(&format!(
        r#"{{"alg":"ed25519","key_id":"56475aa7","public_key":"{PUBLIC}"}}"#
    ))
    .unwrap()
}

#[test]
fn judge_signatures_verify_across_implementations() {
    let k = key();
    assert_eq!(k.id, "56475aa7");
    assert!(verify(&k, NONCE, "203.0.113.7", BODY, Some(SIG)));
    // Another nonce (replay), direct IP or body (rewrite) fails, and so does
    // an answer whose signature header was stripped.
    assert!(!verify(
        &k,
        "ffffffffffffffffffffffff",
        "203.0.113.7",
        BODY,
        Some(SIG)
    ));
    assert!(!verify(&k, NONCE, "198.51.100.1", BODY, Some(SIG)));
    assert!(!verify(
        &k,
        NONCE,
        "203.0.113.7",
        b"{\"anonymity\":\"transparent\"}",
        Some(SIG)
    ));
    assert!(!verify(&k, NONCE, "203.0.113.7", BODY, None));
    assert!(!verify(&k, NONCE, "203.0.113.7", BODY, Some("not base64")));
}

#[test]
fn judge_keys_and_urls() {
    assert!(parse_key(r#"{"alg":"rsa","public_key":"AAAA"}"#).is_none());
    assert!(parse_key(r#"{"alg":"ed25519","public_key":"AAAA"}"#).is_none());
    assert_eq!(
        key_url("http://judge.example.com/judge").as_deref(),
        Some("http://judge.example.com/key")
    );
    assert_eq!(
        key_url("https://host/svc/judge?x=1").as_deref(),
        Some("https://host/svc/key")
    );
    assert_eq!(
        judge_query("http://h/judge", Some("1.2.3.4"), "n0nce123"),
        "http://h/judge?direct_ip=1.2.3.4&nonce=n0nce123"
    );
    assert_eq!(
        judge_query("http://h/judge?x=1", Some(""), "n0nce123"),
        "http://h/judge?x=1&nonce=n0nce123"
    );
    let n = new_nonce();
    assert_eq!(n.len(), 24);
    assert_ne!(n, new_nonce());
}

#[test]
fn judge_integrity_prefers_the_signature() {
    assert_eq!(
        judge_tamper(Some(true), Some("a"), Some("b")).as_deref(),
        Some("ok")
    );
    assert_eq!(
        judge_tamper(Some(false), Some("a"), Some("a")).as_deref(),
        Some("modified")
    );
    assert_eq!(
        judge_tamper(None, Some("a"), Some("a")).as_deref(),
        Some("ok")
    );
    assert_eq!(
        judge_tamper(None, Some("a"), Some("b")).as_deref(),
        Some("modified")
    );
    assert_eq!(judge_tamper(None, None, Some("b")), None);
}

#[test]
fn judge_http_url_only_in_judge_mode() {
    let cfg = |judge: bool, url: &str| {
        CheckConfig {
            judge_mode: judge,
            test_url: "https://judge.example.com/judge".to_string(),
            judge_http_url: Some(url.to_string()),
            ..CheckConfig::default()
        }
        .sanitized()
    };
    assert_eq!(
        cfg(true, " http://judge.example.com/judge ")
            .judge_http_url
            .as_deref(),
        Some("http://judge.example.com/judge")
    );
    assert_eq!(
        cfg(true, "https://judge.example.com/judge").judge_http_url,
        None
    );
    assert_eq!(
        cfg(false, "http://judge.example.com/judge").judge_http_url,
        None
    );

    // The extra HTTP request needs our direct IP and one more timeout.
    let flags = cfg(true, "http://judge.example.com/judge").flags();
    assert!(wants_direct_ip(&flags, "https://judge.example.com/judge"));
    let without = cfg(true, "").flags();
    assert!(!wants_direct_ip(
        &without,
        "https://judge.example.com/judge"
    ));
    let t = Duration::from_secs(5);
    assert_eq!(
        candidate_budget(t, 1, &flags),
        candidate_budget(t, 1, &without) + t
    );
}
