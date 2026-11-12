//! What a run measures directly, without a proxy, before it starts: our own
//! address and the reference page and certificate that answers through a
//! proxy are compared with.

use crate::commands::check::{is_tls_target, CheckFlags};
use crate::http::{
    build_client, looks_like_ip, read_body_capped, BODY_LIMIT_PAGE, BODY_LIMIT_SMALL, TAMPER_URL,
};
use crate::tls::tls_direct_fp;
use crate::{body_hash, judge_sig};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// What a run measured directly, without a proxy, to compare answers with.
/// The default knows nothing and expects no signature.
pub(crate) struct Baseline {
    pub(crate) direct_ip: Option<String>,
    pub(crate) example_len: Option<usize>,
    pub(crate) example_hash: Option<u64>,
    pub(crate) example_cert_fp: Option<String>,
    pub(crate) judge_content_sha: Option<String>,
    /// The judge's key at the start of the run.
    pub(crate) judge_key: judge_sig::KeyLookup,
    /// Where `judge_key` came from: the judge URL and the request timeout.
    pub(crate) judge_key_source: Option<(String, Duration)>,
    /// The one further `/key` request of a run, shared by all its checks.
    pub(crate) judge_key_retry: tokio::sync::OnceCell<judge_sig::KeyLookup>,
}

impl Default for Baseline {
    fn default() -> Self {
        Self {
            direct_ip: None,
            example_len: None,
            example_hash: None,
            example_cert_fp: None,
            judge_content_sha: None,
            judge_key: judge_sig::KeyLookup::NotSigned,
            judge_key_source: None,
            judge_key_retry: tokio::sync::OnceCell::new(),
        }
    }
}

#[derive(Default)]
struct BaselineCache {
    direct_ip: Option<(String, Instant)>,
    example: Option<(usize, u64, Instant)>,
    cert: Option<(String, Instant)>,
}

static BASELINE_CACHE: std::sync::OnceLock<Mutex<BaselineCache>> = std::sync::OnceLock::new();

const BASELINE_IP_TTL: Duration = Duration::from_secs(300);

const BASELINE_PAGE_TTL: Duration = Duration::from_secs(1800);

fn baseline_cache() -> &'static Mutex<BaselineCache> {
    BASELINE_CACHE.get_or_init(|| Mutex::const_new(BaselineCache::default()))
}

pub(crate) fn cache_fresh(ts: Instant, ttl: Duration) -> bool {
    ts.elapsed() < ttl
}

/// Our direct IP is needed only where a leak can be seen: the anonymity
/// probe, or a judge reached over plain HTTP.
pub(crate) fn wants_direct_ip(flags: &CheckFlags, test_url: &str) -> bool {
    if flags.judge {
        !is_tls_target(test_url) || flags.judge_http.is_some()
    } else {
        flags.anon
    }
}

/// Our address as one of these services sees it: api.ipify.org answers with
/// the bare address, a judge's `/ip` with `{"ip": ...}`.
async fn lookup_direct_ip(client: &reqwest::Client, url: &str) -> Option<String> {
    let r = client.get(url).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let body = read_body_capped(r, BODY_LIMIT_SMALL).await.ok()?;
    let body = body.trim();
    let ip = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("ip")?.as_str().map(str::to_string))
        .unwrap_or_else(|| body.to_string());
    looks_like_ip(&ip).then_some(ip)
}

/// `judge_ip`: the judge's own `/ip` over HTTPS, asked when api.ipify.org
/// does not answer.
pub(crate) async fn fetch_baseline(
    flags: CheckFlags,
    want_ip: bool,
    judge_ip: Option<String>,
) -> Baseline {
    let timeout = Duration::from_millis(9000);
    let direct_ip = async {
        if !want_ip {
            return None;
        }
        {
            let c = baseline_cache().lock().await;
            if let Some((v, ts)) = &c.direct_ip {
                if cache_fresh(*ts, BASELINE_IP_TTL) {
                    return Some(v.clone());
                }
            }
        }
        let c = build_client(None, timeout, true).ok()?;
        let mut found = lookup_direct_ip(&c, "https://api.ipify.org").await;
        if found.is_none() {
            if let Some(url) = &judge_ip {
                found = lookup_direct_ip(&c, url).await;
            }
        }
        let t = found?;
        baseline_cache().lock().await.direct_ip = Some((t.clone(), Instant::now()));
        Some(t)
    };
    let example = async {
        if !flags.tamper {
            return None;
        }
        {
            let c = baseline_cache().lock().await;
            if let Some((len, hash, ts)) = c.example {
                if cache_fresh(ts, BASELINE_PAGE_TTL) {
                    return Some((len, hash));
                }
            }
        }
        let c = build_client(None, timeout, true).ok()?;
        let r = c.get(TAMPER_URL).send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        let t = read_body_capped(r, BODY_LIMIT_PAGE).await.ok()?;
        let val = (t.len(), body_hash(&t));
        baseline_cache().lock().await.example = Some((val.0, val.1, Instant::now()));
        Some(val)
    };
    let cert = async {
        if !flags.tls {
            return None;
        }
        {
            let c = baseline_cache().lock().await;
            if let Some((v, ts)) = &c.cert {
                if cache_fresh(*ts, BASELINE_PAGE_TTL) {
                    return Some(v.clone());
                }
            }
        }
        let fp = tls_direct_fp().await;
        if let Some(v) = &fp {
            baseline_cache().lock().await.cert = Some((v.clone(), Instant::now()));
        }
        fp
    };
    let (ip, ex, cert) = tokio::join!(direct_ip, example, cert);
    let (len, hash) = ex.unzip();
    Baseline {
        direct_ip: ip,
        example_len: len,
        example_hash: hash,
        example_cert_fp: cert,
        ..Baseline::default()
    }
}
