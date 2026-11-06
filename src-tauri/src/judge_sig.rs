//! Signed answers from ProxPulse Judge. Over plain HTTP the checked proxy
//! sees the judge request (the anonymity test needs that) and could rewrite
//! the answer; the judge signs the body together with our nonce and
//! direct IP, so a rewritten or replayed answer is caught here.

use crate::http::{build_client, read_body_capped, BODY_LIMIT_SMALL};
use base64::Engine;
use ring::rand::SecureRandom;
use ring::signature::{UnparsedPublicKey, ED25519};
use std::time::Duration;

/// Prefix of the signed message; matches the judge's `SIGNATURE_CONTEXT`.
const SIGNATURE_CONTEXT: &[u8] = b"proxpulse-judge-v1\n";
pub(crate) const SIGNATURE_HEADER: &str = "x-judge-signature";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JudgeKey {
    pub(crate) id: String,
    pub(crate) public: Vec<u8>,
    /// Delivered over HTTPS, so TLS vouches for it.
    pub(crate) over_tls: bool,
}

/// What a direct `/key` request found.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum KeyLookup {
    /// The judge signs with this key.
    Key(JudgeKey),
    /// A clear answer that there is no key: an older or third-party judge.
    NotSigned,
    /// No clear answer (network error, 429, 5xx): the judge may well sign.
    Unavailable,
}

/// What the signature says about a judge answer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SignatureCheck {
    Valid,
    /// Rewritten or replayed on the way.
    Invalid,
    /// The judge does not sign.
    NotSigned,
    /// The judge may sign, but its key could not be fetched.
    Unchecked,
}

/// The `/key` answer: `{"alg": "ed25519", "key_id": .., "public_key": base64}`.
pub(crate) fn parse_key(body: &str) -> Option<JudgeKey> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if v.get("alg")?.as_str()? != "ed25519" {
        return None;
    }
    let public = base64::engine::general_purpose::STANDARD
        .decode(v.get("public_key")?.as_str()?)
        .ok()
        .filter(|k| k.len() == 32)?;
    Some(JudgeKey {
        id: v
            .get("key_id")
            .and_then(|k| k.as_str())
            .unwrap_or("")
            .to_string(),
        public,
        over_tls: false,
    })
}

pub(crate) fn message(nonce: &str, direct_ip: &str, body: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(
        SIGNATURE_CONTEXT.len() + nonce.len() + direct_ip.len() + body.len() + 2,
    );
    m.extend_from_slice(SIGNATURE_CONTEXT);
    m.extend_from_slice(nonce.as_bytes());
    m.push(b'\n');
    m.extend_from_slice(direct_ip.as_bytes());
    m.push(b'\n');
    m.extend_from_slice(body);
    m
}

/// Whether `signature` (base64, from the response header) signs this answer.
pub(crate) fn verify(
    key: &JudgeKey,
    nonce: &str,
    direct_ip: &str,
    body: &[u8],
    signature: Option<&str>,
) -> bool {
    let Some(sig) = signature.and_then(|s| {
        base64::engine::general_purpose::STANDARD
            .decode(s.trim())
            .ok()
    }) else {
        return false;
    };
    UnparsedPublicKey::new(&ED25519, &key.public)
        .verify(&message(nonce, direct_ip, body), &sig)
        .is_ok()
}

/// A fresh random nonce (24 hex characters) for one judge request.
pub(crate) fn new_nonce() -> String {
    let mut bytes = [0u8; 12];
    if ring::rand::SystemRandom::new().fill(&mut bytes).is_err() {
        // Without randomness a replayed answer cannot be told apart; the
        // time still makes the nonce unique per request.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        bytes.copy_from_slice(&nanos.to_le_bytes()[..12]);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A judge URL with our direct IP (when known) and the nonce in the query.
pub(crate) fn judge_query(url: &str, direct_ip: Option<&str>, nonce: &str) -> String {
    let mut out = url.to_string();
    let mut sep = if url.contains('?') { '&' } else { '?' };
    if let Some(ip) = direct_ip.filter(|ip| !ip.is_empty()) {
        out.push_str(&format!("{sep}direct_ip={ip}"));
        sep = '&';
    }
    out.push_str(&format!("{sep}nonce={nonce}"));
    out
}

/// `/key` next to a `/judge` endpoint (custom paths keep their prefix).
pub(crate) fn key_url(judge_url: &str) -> Option<String> {
    let base = judge_url.split(['?', '#']).next()?.trim_end_matches('/');
    let base = base.strip_suffix("/judge").unwrap_or(base);
    (!base.is_empty()).then(|| format!("{base}/key"))
}

/// Fetch the judge's public key directly (never through a checked proxy),
/// over HTTPS when the judge serves it. Plain HTTP stays as a fallback for
/// judges without TLS, unless `https_only`; such a key is only as trustworthy
/// as the direct path.
pub(crate) async fn fetch_key(judge_url: &str, timeout: Duration, https_only: bool) -> KeyLookup {
    let Some(url) = key_url(judge_url) else {
        return KeyLookup::NotSigned;
    };
    let mut tries = Vec::new();
    if let Some(rest) = url.strip_prefix("http://") {
        tries.push(format!("https://{rest}"));
    }
    if !https_only || url.starts_with("https://") {
        tries.push(url);
    }
    let Ok(client) = build_client(None, timeout, true) else {
        return KeyLookup::Unavailable;
    };
    let mut found = KeyLookup::Unavailable;
    for u in tries {
        let Ok(r) = client.get(&u).send().await else {
            continue;
        };
        let over_tls = r.url().scheme() == "https";
        if https_only && !over_tls {
            continue;
        }
        let status = r.status();
        if status.is_success() {
            let Ok(body) = read_body_capped(r, BODY_LIMIT_SMALL).await else {
                continue;
            };
            match parse_key(&body) {
                Some(key) => return KeyLookup::Key(JudgeKey { over_tls, ..key }),
                None => found = KeyLookup::NotSigned,
            }
        } else if status == reqwest::StatusCode::NOT_FOUND {
            found = KeyLookup::NotSigned;
        }
    }
    found
}

/// Whether this lookup found a key that came over plain HTTP.
pub(crate) fn key_over_http(lookup: &KeyLookup) -> bool {
    matches!(lookup, KeyLookup::Key(key) if !key.over_tls)
}

/// Check an answer's signature with the key from the start of the run. When
/// that fails, or there was no key yet, `refetch` asks the judge once more: it
/// may have restarted with a new key. `X-Judge-Key-Id` plays no part in this,
/// as it is not signed and the proxy controls it.
pub(crate) async fn check_signature<'a>(
    first: &'a KeyLookup,
    refetch: impl std::future::Future<Output = &'a KeyLookup>,
    nonce: &str,
    direct_ip: &str,
    body: &[u8],
    signature: Option<&str>,
) -> SignatureCheck {
    let holds = |key: &JudgeKey| verify(key, nonce, direct_ip, body, signature);
    match first {
        KeyLookup::NotSigned => return SignatureCheck::NotSigned,
        KeyLookup::Key(key) if holds(key) => return SignatureCheck::Valid,
        _ => {}
    }
    match (first, refetch.await) {
        (_, KeyLookup::Key(key)) if holds(key) => SignatureCheck::Valid,
        // This judge had a key, and the answer does not match it.
        (_, KeyLookup::Key(_)) | (KeyLookup::Key(_), _) => SignatureCheck::Invalid,
        (_, KeyLookup::NotSigned) => SignatureCheck::NotSigned,
        (_, KeyLookup::Unavailable) => SignatureCheck::Unchecked,
    }
}

/// Integrity of a judge answer: a signature decides when the judge signs;
/// otherwise the content hash is compared with the direct answer.
pub(crate) fn judge_tamper(
    check: SignatureCheck,
    base_sha: Option<&str>,
    proxy_sha: Option<&str>,
) -> Option<String> {
    let same_hash = base_sha.zip(proxy_sha).map(|(a, b)| a == b);
    let verdict = match check {
        SignatureCheck::Valid => "ok",
        SignatureCheck::Invalid => "modified",
        SignatureCheck::NotSigned => {
            if same_hash? {
                "ok"
            } else {
                "modified"
            }
        }
        // A proxy can keep the hash and rewrite the verdict, so here a matching
        // hash proves nothing: only a mismatch is a result.
        SignatureCheck::Unchecked => {
            if same_hash == Some(false) {
                "modified"
            } else {
                "unverified"
            }
        }
    };
    Some(verdict.to_string())
}
