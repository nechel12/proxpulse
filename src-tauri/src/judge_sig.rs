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
/// over HTTPS when the judge serves it. None: the judge does not sign.
pub(crate) async fn fetch_key(judge_url: &str, timeout: Duration) -> Option<JudgeKey> {
    let url = key_url(judge_url)?;
    let mut tries = Vec::new();
    if let Some(rest) = url.strip_prefix("http://") {
        tries.push(format!("https://{rest}"));
    }
    tries.push(url);
    let client = build_client(None, timeout, true).ok()?;
    for u in tries {
        let Ok(r) = client.get(&u).send().await else {
            continue;
        };
        if !r.status().is_success() {
            continue;
        }
        if let Some(key) = read_body_capped(r, BODY_LIMIT_SMALL)
            .await
            .ok()
            .and_then(|b| parse_key(&b))
        {
            return Some(key);
        }
    }
    None
}

/// Integrity of a judge answer: a signature decides when the judge signs;
/// otherwise the content hash is compared with the direct answer.
pub(crate) fn judge_tamper(
    signed: Option<bool>,
    base_sha: Option<&str>,
    proxy_sha: Option<&str>,
) -> Option<String> {
    let verdict = match (signed, base_sha, proxy_sha) {
        (Some(ok), _, _) => ok,
        (None, Some(a), Some(b)) => a == b,
        _ => return None,
    };
    Some(if verdict { "ok" } else { "modified" }.to_string())
}
