//! A ProxPulse Judge's answers: where to ask it directly, how to read its
//! body, and whether an answer that came through a proxy can be believed.

use crate::baseline::Baseline;
use crate::commands::check::{answered_in_clear, is_tls_target};
use crate::http::{build_client, read_body_capped, BODY_LIMIT_PAGE};
use crate::{judge_sig, ProxyResult};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize, Default)]
struct JudgeGeo {
    country: Option<String>,
    country_code: Option<String>,
    city: Option<String>,
    asn: Option<u64>,
    aso: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct JudgeBody {
    ip: Option<String>,
    anonymity: Option<String>,
    ip_type: Option<String>,
    geo: Option<JudgeGeo>,
    content_sha256: Option<String>,
}

pub(crate) fn try_parse_judge_body(res: &mut ProxyResult, body: &str) -> Option<String> {
    let jb: JudgeBody = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return None,
    };
    if let Some(ip) = jb.ip {
        let t = ip.trim().to_string();
        if !t.is_empty() {
            res.ip = Some(t);
        }
    }
    res.anonymity = jb.anonymity;
    res.ip_type = jb.ip_type;
    if let Some(g) = jb.geo {
        res.country = g.country;
        res.country_code = g.country_code;
        res.city = g.city;
        res.asn = g.asn.map(|n| n.to_string());
        res.org = g.aso;
    }
    jb.content_sha256
}

/// Direct judge endpoint used for the content hash baseline. For a custom
/// path (e.g. `https://host/svc`) the URL is used as-is: the same endpoint
/// is what the checked proxy requests.
pub(crate) fn judge_direct_url(test_url: &str) -> Option<String> {
    let t = test_url.trim_end_matches('/');
    if t.is_empty() {
        return None;
    }
    if t.ends_with("/judge") {
        return Some(t.to_string());
    }
    if let Some(base) = t.strip_suffix("/generate_204") {
        if !base.is_empty() {
            return Some(format!("{base}/judge"));
        }
    }
    Some(t.to_string())
}

pub(crate) async fn fetch_direct_judge_sha(judge_url: &str, timeout: Duration) -> Option<String> {
    let client = build_client(None, timeout, true).ok()?;
    let r = client.get(judge_url).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let body = read_body_capped(r, BODY_LIMIT_PAGE).await.ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    v.get("content_sha256")?.as_str().map(|s| s.to_string())
}

pub(crate) fn signature_header(r: &reqwest::Response) -> Option<String> {
    r.headers()
        .get(judge_sig::SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Where the run takes the judge's content hash and key from: the judge
/// answer that goes over plain HTTP needs both. That is the HTTP judge
/// whenever there is one, whatever the main address (an HTTPS /generate_204
/// included), else a plain-HTTP main /judge. An HTTPS body cannot be modified
/// by the proxy and /generate_204 is empty, so those need neither.
pub(crate) fn judge_baseline_url(
    judge: bool,
    judge_http: Option<&str>,
    test_url: &str,
) -> Option<String> {
    if !judge {
        return None;
    }
    if let Some(http) = judge_http {
        return judge_direct_url(http);
    }
    if is_tls_target(test_url) || test_url.trim_end_matches('/').ends_with("/generate_204") {
        return None;
    }
    judge_direct_url(test_url)
}

/// Drop everything a rewritten judge answer said: its exit IP and geo would
/// otherwise reach the pool's countries, the dispatcher's country filter,
/// DNSBL and the geo cache.
pub(crate) fn forget_judge_answer(res: &mut ProxyResult) {
    res.ip = None;
    res.country = None;
    res.country_code = None;
    res.city = None;
    res.asn = None;
    res.org = None;
    res.ip_type = None;
    res.anonymity = None;
}

/// Anonymity and integrity from a judge answer reached over HTTP. A judge
/// that signs decides integrity by the signature; an answer that fails it
/// was rewritten on the way, so its anonymity verdict is dropped too.
pub(crate) fn apply_judge_integrity(
    res: &mut ProxyResult,
    baseline: &Baseline,
    check: judge_sig::SignatureCheck,
    proxy_sha: Option<&str>,
) {
    res.tamper = judge_sig::judge_tamper(check, baseline.judge_content_sha.as_deref(), proxy_sha);
    // Without our direct IP the judge cannot see a leak: its "elite" or
    // "anonymous" may hide "transparent", so there is no verdict, as without
    // a judge.
    if check == judge_sig::SignatureCheck::Invalid || baseline.direct_ip.is_none() {
        res.anonymity = None;
    }
}

/// The signature check of a judge answer. All checks of a run share one more
/// `/key` request, which a failed check starts. A key that came over HTTPS is
/// only replaced over HTTPS: anyone on the direct path could force plain HTTP.
pub(crate) async fn check_answer_signature(
    baseline: &Baseline,
    challenge: &str,
    direct_ip: &str,
    body: &[u8],
    signature: Option<&str>,
) -> judge_sig::SignatureCheck {
    let https_only = matches!(&baseline.judge_key, judge_sig::KeyLookup::Key(key) if key.over_tls);
    let refetch = baseline.judge_key_retry.get_or_init(|| async {
        match &baseline.judge_key_source {
            Some((url, timeout)) => judge_sig::fetch_key(url, *timeout, https_only).await,
            None => judge_sig::KeyLookup::NotSigned,
        }
    });
    judge_sig::check_signature(
        &baseline.judge_key,
        refetch,
        challenge,
        direct_ip,
        body,
        signature,
    )
    .await
}

/// With both schemes on, the HTTPS answer gives liveness and geo; this
/// plain-HTTP request through the same proxy gives anonymity and integrity.
/// A proxy limited to port 443, or a judge without plain HTTP, leaves them
/// empty.
pub(crate) async fn judge_over_http(
    client: &reqwest::Client,
    http_url: &str,
    nonce: &str,
    direct_ip: &str,
    baseline: &Baseline,
    res: &mut ProxyResult,
) {
    let url = judge_sig::judge_query(http_url, Some(direct_ip), nonce);
    let Ok(r) = client.get(&url).send().await else {
        return;
    };
    if !r.status().is_success() || !answered_in_clear(&url, r.url().as_str()) {
        return;
    }
    let sig = signature_header(&r);
    let Ok(body) = read_body_capped(r, BODY_LIMIT_PAGE).await else {
        return;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
        return;
    };
    res.anonymity = v
        .get("anonymity")
        .and_then(|a| a.as_str())
        .map(str::to_string);
    let proxy_sha = v.get("content_sha256").and_then(|s| s.as_str());
    let check =
        check_answer_signature(baseline, nonce, direct_ip, body.as_bytes(), sig.as_deref()).await;
    apply_judge_integrity(res, baseline, check, proxy_sha);
}

/// `https://host/ip` next to a judge address: the judge tells a direct client
/// its address too. HTTPS only, since a wrong address would hide leaks.
pub(crate) fn judge_ip_url(judge_url: &str) -> Option<String> {
    let key = judge_sig::key_url(judge_url)?;
    let base = key.strip_suffix("/key")?;
    let rest = base
        .strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))?;
    Some(format!("https://{rest}/ip"))
}
