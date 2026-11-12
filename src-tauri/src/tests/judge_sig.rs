use crate::baseline::{wants_direct_ip, Baseline};
use crate::commands::check::{answered_in_clear, candidate_budget, CheckConfig};
use crate::judge::check_answer_signature;
use crate::judge_sig::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

/// The direct IP the judge's test vector was signed with.
const DIRECT_IP: &str = "203.0.113.7";

/// A key that did not sign the test vector: the judge's before a restart.
fn old_key() -> JudgeKey {
    JudgeKey {
        id: "old".to_string(),
        public: vec![7; 32],
        over_tls: false,
    }
}

fn key_json() -> String {
    format!(r#"{{"alg":"ed25519","key_id":"56475aa7","public_key":"{PUBLIC}"}}"#)
}

/// A local server that gives every HTTP request one answer and counts them.
/// The HTTPS attempt that comes first fails, as on a judge without TLS.
async fn key_server(status: &'static str, body: String) -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    tokio::spawn(async move {
        while let Ok((mut conn, _)) = listener.accept().await {
            let (counter, body) = (counter.clone(), body.clone());
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let n = conn.read(&mut buf).await.unwrap_or(0);
                if !buf[..n].starts_with(b"GET /key") {
                    return;
                }
                counter.fetch_add(1, Ordering::SeqCst);
                let answer = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = conn.write_all(answer.as_bytes()).await;
            });
        }
    });
    (format!("http://127.0.0.1:{port}/judge"), requests)
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
    use SignatureCheck::*;
    assert_eq!(
        judge_tamper(Valid, Some("a"), Some("b")).as_deref(),
        Some("ok")
    );
    assert_eq!(
        judge_tamper(Invalid, Some("a"), Some("a")).as_deref(),
        Some("modified")
    );
    assert_eq!(
        judge_tamper(NotSigned, Some("a"), Some("a")).as_deref(),
        Some("ok")
    );
    assert_eq!(
        judge_tamper(NotSigned, Some("a"), Some("b")).as_deref(),
        Some("modified")
    );
    assert_eq!(judge_tamper(NotSigned, None, Some("b")), None);
}

#[test]
fn unchecked_signature_never_reports_ok() {
    use SignatureCheck::Unchecked;
    // A proxy can keep the hash and rewrite the verdict.
    assert_eq!(
        judge_tamper(Unchecked, Some("a"), Some("a")).as_deref(),
        Some("unverified")
    );
    assert_eq!(
        judge_tamper(Unchecked, Some("a"), Some("b")).as_deref(),
        Some("modified")
    );
    assert_eq!(
        judge_tamper(Unchecked, None, Some("b")).as_deref(),
        Some("unverified")
    );
}

#[tokio::test]
async fn signature_check_retries_only_when_needed() {
    async fn no_second_request<'a>() -> &'a KeyLookup {
        panic!("must not fetch the key again")
    }
    async fn run(first: KeyLookup) -> SignatureCheck {
        check_signature(
            &first,
            no_second_request(),
            NONCE,
            DIRECT_IP,
            BODY,
            Some(SIG),
        )
        .await
    }
    assert_eq!(run(KeyLookup::Key(key())).await, SignatureCheck::Valid);
    assert_eq!(run(KeyLookup::NotSigned).await, SignatureCheck::NotSigned);
}

#[tokio::test]
async fn signature_check_after_a_second_key_request() {
    use KeyLookup::{Key, NotSigned, Unavailable};
    use SignatureCheck as S;

    async fn run(first: KeyLookup, second: KeyLookup, body: &[u8], sig: Option<&str>) -> S {
        check_signature(&first, async { &second }, NONCE, DIRECT_IP, body, sig).await
    }
    // The judge restarted with a new key.
    assert_eq!(
        run(Key(old_key()), Key(key()), BODY, Some(SIG)).await,
        S::Valid
    );
    // The key is the same: rewritten, or the signature was stripped.
    assert_eq!(
        run(Key(key()), Key(key()), b"{}", Some(SIG)).await,
        S::Invalid
    );
    assert_eq!(run(Key(key()), Key(key()), BODY, None).await, S::Invalid);
    // This judge had a key: losing `/key` afterwards does not excuse an answer.
    assert_eq!(
        run(Key(old_key()), Unavailable, BODY, Some(SIG)).await,
        S::Invalid
    );
    assert_eq!(
        run(Key(old_key()), NotSigned, BODY, Some(SIG)).await,
        S::Invalid
    );
    // No key at the start of the run, for example a judge still waking up.
    assert_eq!(
        run(Unavailable, Key(key()), BODY, Some(SIG)).await,
        S::Valid
    );
    assert_eq!(
        run(Unavailable, Key(key()), b"{}", Some(SIG)).await,
        S::Invalid
    );
    assert_eq!(run(Unavailable, NotSigned, BODY, None).await, S::NotSigned);
    assert_eq!(
        run(Unavailable, Unavailable, BODY, Some(SIG)).await,
        S::Unchecked
    );
}

#[tokio::test]
async fn key_lookup_tells_no_key_from_no_answer() {
    let t = Duration::from_secs(5);
    let lookup = |status, body: &str| {
        let body = body.to_string();
        async move {
            let (url, _) = key_server(status, body).await;
            fetch_key(&url, t, false).await
        }
    };
    assert_eq!(lookup("200 OK", &key_json()).await, KeyLookup::Key(key()));
    // These servers speak plain HTTP: "no key" from them is only as good as
    // the direct path, and the user is told.
    assert_eq!(
        lookup("404 Not Found", "").await,
        KeyLookup::NotSignedOverHttp
    );
    assert_eq!(
        lookup("200 OK", "<html>").await,
        KeyLookup::NotSignedOverHttp
    );
    assert!(key_over_http(&KeyLookup::NotSignedOverHttp));
    assert!(!key_over_http(&KeyLookup::NotSigned));
    assert_eq!(
        lookup("500 Internal Server Error", "").await,
        KeyLookup::Unavailable
    );
    assert_eq!(
        lookup("429 Too Many Requests", "").await,
        KeyLookup::Unavailable
    );

    // A closed port.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    assert_eq!(
        fetch_key(&format!("http://127.0.0.1:{port}/judge"), t, false).await,
        KeyLookup::Unavailable
    );
}

#[tokio::test]
async fn the_key_lookup_does_not_follow_redirects() {
    // A redirect could hand the lookup to another host and make its key look
    // delivered over TLS.
    let (elsewhere, served) = key_server("200 OK", key_json()).await;
    let target = elsewhere.replace("/judge", "/key");
    let status: &'static str = Box::leak(
        format!(
            "302 Found
Location: {target}"
        )
        .into_boxed_str(),
    );
    let (url, asked) = key_server(status, String::new()).await;
    let found = fetch_key(&url, Duration::from_secs(5), false).await;
    assert_eq!(found, KeyLookup::Unavailable);
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(served.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_run_makes_one_more_key_request_at_most() {
    let (url, requests) = key_server("200 OK", key_json()).await;
    let baseline = Baseline {
        judge_key: KeyLookup::Key(old_key()),
        judge_key_source: Some((url, Duration::from_secs(5))),
        ..Baseline::default()
    };
    let check = |body, sig| check_answer_signature(&baseline, NONCE, DIRECT_IP, body, sig);
    assert_eq!(check(BODY, Some(SIG)).await, SignatureCheck::Valid);
    assert_eq!(check(b"{}", Some(SIG)).await, SignatureCheck::Invalid);
    assert_eq!(check(BODY, None).await, SignatureCheck::Invalid);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_key_from_https_is_not_replaced_over_http() {
    let (url, requests) = key_server("200 OK", key_json()).await;
    let t = Duration::from_secs(5);
    assert!(key_over_http(&fetch_key(&url, t, false).await));
    assert_eq!(requests.swap(0, Ordering::SeqCst), 1);

    assert_eq!(fetch_key(&url, t, true).await, KeyLookup::Unavailable);
    assert_eq!(requests.load(Ordering::SeqCst), 0);

    let baseline = Baseline {
        judge_key: KeyLookup::Key(JudgeKey {
            over_tls: true,
            ..old_key()
        }),
        judge_key_source: Some((url, t)),
        ..Baseline::default()
    };
    // The answer really was signed by the key on offer, but that key only came
    // over plain HTTP, so it cannot replace one that TLS vouched for.
    assert_eq!(
        check_answer_signature(&baseline, NONCE, DIRECT_IP, BODY, Some(SIG)).await,
        SignatureCheck::Invalid
    );
    assert_eq!(requests.load(Ordering::SeqCst), 0);
}

#[test]
fn a_redirect_to_https_hides_the_request() {
    assert!(answered_in_clear(
        "http://j/judge?a=1",
        "http://j/judge?a=1"
    ));
    assert!(!answered_in_clear("http://j/judge", "https://j/judge"));
    assert!(!answered_in_clear("http://j/judge", "HTTPS://j/judge"));
    assert!(!answered_in_clear("https://j/judge", "https://j/judge"));
    assert!(!answered_in_clear("https://j/judge", "http://j/judge"));
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
