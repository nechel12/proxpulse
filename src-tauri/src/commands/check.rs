use crate::tls::{tls_direct_fp, tls_via_proxy};
use crate::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::State;
use tokio::net::TcpStream;

#[derive(Debug, Default)]
pub(crate) struct CheckState {
    pub(crate) cancel: Arc<AtomicBool>,
    /// Prevents a manual check and an auto check (or two manual checks)
    /// from running at the same time.
    pub(crate) busy: Arc<AtomicBool>,
}

pub(crate) struct BusyGuard(pub(crate) Arc<AtomicBool>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

const CHECK_BUSY: &str = "another check is running";

/// Limits shared by manual checks, scheduled checks and the UI inputs
/// (`index.html`, `readCheckSettings`).
pub(crate) const TIMEOUT_MS: (u64, u64) = (1_000, 30_000);
pub(crate) const PRECHECK_TIMEOUT_MS: (u64, u64) = (300, 10_000);
pub(crate) const MAX_CONCURRENCY: usize = 500;
pub(crate) const MAX_REPEATS: usize = 5;
/// Most sites one check requests through each proxy.
pub(crate) const MAX_SITES: usize = 8;

/// Check parameters, one definition for a manual check and a scheduled pool
/// check. Mirrors `CheckSettings` in `src/check-client.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct CheckConfig {
    pub(crate) test_url: String,
    pub(crate) timeout_ms: u64,
    pub(crate) concurrency: usize,
    pub(crate) repeats: usize,
    pub(crate) default_proto: String,
    pub(crate) with_geo: bool,
    pub(crate) with_anonymity: bool,
    pub(crate) with_tamper: bool,
    pub(crate) with_tls: bool,
    pub(crate) precheck: bool,
    pub(crate) precheck_timeout_ms: u64,
    pub(crate) judge_mode: bool,
    pub(crate) ipapi_key: Option<String>,
    /// Sites requested through every alive proxy.
    pub(crate) sites: Vec<String>,
    /// Judge mode over HTTPS: the plain-HTTP `/judge` URL asked through every
    /// alive proxy for anonymity and integrity.
    pub(crate) judge_http_url: Option<String>,
    /// Look the exit IP up in DNS blocklists.
    pub(crate) with_dnsbl: bool,
}

impl Default for CheckConfig {
    fn default() -> Self {
        Self {
            test_url: "https://www.google.com/generate_204".to_string(),
            timeout_ms: 5000,
            concurrency: 50,
            repeats: 1,
            default_proto: "http".to_string(),
            with_geo: true,
            with_anonymity: true,
            with_tamper: true,
            with_tls: true,
            precheck: true,
            precheck_timeout_ms: 1500,
            judge_mode: false,
            ipapi_key: None,
            sites: Vec::new(),
            judge_http_url: None,
            with_dnsbl: false,
        }
    }
}

impl CheckConfig {
    pub(crate) fn sanitized(mut self) -> Self {
        self.test_url = self.test_url.trim().to_string();
        self.timeout_ms = self.timeout_ms.clamp(TIMEOUT_MS.0, TIMEOUT_MS.1);
        self.concurrency = self.concurrency.clamp(1, MAX_CONCURRENCY);
        self.repeats = self.repeats.clamp(1, MAX_REPEATS);
        self.precheck_timeout_ms = self
            .precheck_timeout_ms
            .clamp(PRECHECK_TIMEOUT_MS.0, PRECHECK_TIMEOUT_MS.1);
        self.ipapi_key = self
            .ipapi_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
        let mut sites: Vec<String> = Vec::new();
        for s in self.sites.iter().map(|s| s.trim()) {
            if is_http_url(s) && !sites.iter().any(|x| x == s) && sites.len() < MAX_SITES {
                sites.push(s.to_string());
            }
        }
        self.sites = sites;
        self.judge_http_url = self
            .judge_http_url
            .map(|u| u.trim().to_string())
            .filter(|u| {
                self.judge_mode
                    && u.get(..7)
                        .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
            });
        self
    }

    pub(crate) fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }

    pub(crate) fn flags(&self) -> CheckFlags {
        CheckFlags {
            geo: self.with_geo,
            anon: self.with_anonymity,
            tamper: self.with_tamper,
            tls: self.with_tls,
            precheck: self.precheck,
            precheck_timeout: Duration::from_millis(self.precheck_timeout_ms),
            judge: self.judge_mode,
            ipapi_key: self.ipapi_key.as_deref().map(Arc::from),
            sites: self.sites.iter().cloned().collect(),
            judge_http: self.judge_http_url.as_deref().map(Arc::from),
            dnsbl: self.with_dnsbl,
        }
    }
}

#[derive(Clone)]
pub(crate) struct CheckFlags {
    pub(crate) geo: bool,
    pub(crate) anon: bool,
    pub(crate) tamper: bool,
    pub(crate) tls: bool,
    pub(crate) precheck: bool,
    pub(crate) precheck_timeout: Duration,
    pub(crate) judge: bool,
    /// Optional ip-api Pro key. Used only for the separate geo lookup; the
    /// main test URL is built by the frontend. The key is visible to the
    /// checked proxy because the request originates from it; see README.
    pub(crate) ipapi_key: Option<Arc<str>>,
    pub(crate) sites: Arc<[String]>,
    pub(crate) judge_http: Option<Arc<str>>,
    pub(crate) dnsbl: bool,
}

// ================= checker =================

pub(crate) fn dead_result(raw: String, proto: String, attempts: u32, err: String) -> ProxyResult {
    ProxyResult {
        url: raw.clone(),
        proxy: raw,
        proto,
        alive: false,
        latency_ms: None,
        jitter_ms: None,
        success_rate: 0.0,
        attempts,
        ip: None,
        country: None,
        country_code: None,
        city: None,
        isp: None,
        org: None,
        asn: None,
        ip_type: None,
        anonymity: None,
        tamper: None,
        tls: None,
        tls_info: None,
        error: Some(err),
        skipped: Vec::new(),
        sites: Vec::new(),
        dnsbl: None,
    }
}

/// Messages on the check channel: one per finished proxy, then `done` once
/// the channel is drained (the command does not return the list again).
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum CheckEvent {
    Result(Box<ProxyResult>),
    /// The judge's key came over plain HTTP, sent at most once per run.
    KeyOverHttp,
    /// No proxy was alive and the check server did not answer directly
    /// either: the run says nothing about the proxies. Sent before `Done`.
    Unreachable,
    Done,
}

/// Outcome of one deep check: `Ran(None)` = not requested or failed.
pub(crate) enum Deep<T> {
    Ran(Option<T>),
    Skipped,
}

impl<T> Deep<T> {
    /// The value, recording `name` when the stage was skipped.
    pub(crate) fn value(self, name: &str, skipped: &mut Vec<String>) -> Option<T> {
        match self {
            Deep::Ran(v) => v,
            Deep::Skipped => {
                skipped.push(name.to_string());
                None
            }
        }
    }
}

/// Resolves once Stop is pressed, so a long phase can be abandoned at once.
async fn wait_cancelled(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tauri::command]
pub(crate) fn cancel_check(state: State<'_, CheckState>) -> Result<(), String> {
    state.cancel.store(true, Ordering::SeqCst);
    Ok(())
}

/// `https://` targets are reached through a CONNECT tunnel (or SOCKS), which
/// hides the request from the proxy: header and content checks see nothing.
pub(crate) fn is_tls_target(url: &str) -> bool {
    url.trim()
        .get(..8)
        .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
}

/// Whether the proxy could see a judge request and alter its answer. A
/// redirect to HTTPS (some hosts force it) hides them again, so the final URL
/// counts as well as the requested one.
pub(crate) fn answered_in_clear(requested: &str, final_url: &str) -> bool {
    !is_tls_target(requested) && !is_tls_target(final_url)
}

pub(crate) fn candidate_budget(timeout: Duration, repeats: usize, flags: &CheckFlags) -> Duration {
    // Main attempts: repeats × timeout. Deep stages run concurrently; the
    // slowest is TLS via an HTTPS proxy: TCP + CONNECT + handshake = 3×.
    // A local DNS lookup for socks4/socks5 adds one more timeout.
    // Without TLS, one timeout is enough for the whole deep group.
    let deep = if flags.judge {
        Duration::ZERO
    } else if flags.tls {
        timeout * 4
    } else {
        timeout
    };
    // Sites, and the judge's HTTP request, follow the main attempts.
    let sites = if flags.sites.is_empty() {
        Duration::ZERO
    } else {
        timeout
    };
    let judge_http = if flags.judge_http.is_some() {
        timeout
    } else {
        Duration::ZERO
    };
    // A check may wait for the run's second `/key` request: HTTPS, then HTTP.
    let judge_key = if flags.judge {
        timeout * 2
    } else {
        Duration::ZERO
    };
    // DNS lookups of all lists run together, four seconds at most.
    let dnsbl = if flags.dnsbl {
        Duration::from_secs(5)
    } else {
        Duration::ZERO
    };
    flags.precheck_timeout
        + timeout * (repeats as u32 + 1)
        + deep
        + sites
        + judge_http
        + judge_key
        + dnsbl
        + Duration::from_secs(5)
}

#[allow(clippy::too_many_arguments)]
async fn check_candidate(
    raw: String,
    url: String,
    proto: String,
    test_url: String,
    timeout: Duration,
    repeats: usize,
    flags: CheckFlags,
    baseline: Arc<Baseline>,
    geo_cache: Arc<HashMap<String, GeoCacheEntry>>,
    cancel: Arc<AtomicBool>,
) -> ProxyResult {
    // absolute ceiling: a single proxy may never outlive the sum of its own timeouts
    let budget = candidate_budget(timeout, repeats, &flags);
    let raw_fb = raw.clone();
    let proto_fb = proto.clone();
    let url_fb = url.clone();
    let mut r = match tokio::time::timeout(
        budget,
        check_candidate_inner(
            raw, url, proto, test_url, timeout, repeats, flags, baseline, geo_cache, cancel,
        ),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => dead_result(raw_fb, proto_fb, 0, "timeout".to_string()),
    };
    // Dead results keep the normalized URL as well: the history keys a proxy
    // by it, so `1.2.3.4:80` alive and dead must land on one entry.
    r.url = url_fb;
    r
}

#[allow(clippy::too_many_arguments)]
async fn check_candidate_inner(
    raw: String,
    url: String,
    proto: String,
    test_url: String,
    timeout: Duration,
    repeats: usize,
    flags: CheckFlags,
    baseline: Arc<Baseline>,
    geo_cache: Arc<HashMap<String, GeoCacheEntry>>,
    cancel: Arc<AtomicBool>,
) -> ProxyResult {
    // stage 1: fast tcp precheck
    if flags.precheck {
        if let Some((h, p)) = host_port_of_url(&url) {
            let r =
                tokio::time::timeout(flags.precheck_timeout, TcpStream::connect((h.as_str(), p)))
                    .await;
            let ok = matches!(r, Ok(Ok(_)));
            if !ok {
                let err = match r {
                    Ok(Err(e)) => io_error(&e),
                    _ => "timeout".to_string(),
                };
                return dead_result(raw, proto, 0, err);
            }
        }
    }

    let client = match build_client(Some(&url), timeout, false) {
        Ok(c) => c,
        Err(e) => return dead_result(raw, proto, 0, e),
    };

    // HTTP judge: pass our direct IP so the judge can detect leaks, and a
    // nonce the judge signs. An HTTPS judge cannot see proxy headers, so it
    // gets no reason to learn our IP.
    let nonce = judge_sig::new_nonce();
    let direct_ip = baseline.direct_ip.as_deref().unwrap_or("");
    let req_url = if flags.judge && !is_tls_target(&test_url) {
        judge_sig::judge_query(&test_url, Some(direct_ip), &nonce)
    } else {
        test_url.clone()
    };

    let mut latencies: Vec<u64> = Vec::new();
    let mut first_body: Option<String> = None;
    let mut first_sig: Option<String> = None;
    let mut first_in_clear = false;
    let mut last_err: Option<String> = None;

    for _ in 0..repeats {
        if cancel.load(Ordering::Relaxed) {
            last_err = Some("cancelled".to_string());
            break;
        }
        let start = Instant::now();
        match client.get(&req_url).send().await {
            Ok(r) => {
                let status = r.status();
                let ms = start.elapsed().as_millis() as u64;
                if status.is_success() {
                    latencies.push(ms);
                    if first_body.is_none() {
                        first_sig = signature_header(&r);
                        first_in_clear = answered_in_clear(&req_url, r.url().as_str());
                        first_body = read_body_capped(r, BODY_LIMIT_PAGE).await.ok();
                    }
                    last_err = None;
                } else {
                    last_err = Some(format!("http {}", status.as_u16()));
                }
            }
            Err(e) => {
                last_err = Some(request_error(&e));
            }
        }
    }

    let attempts = repeats as u32;
    let successes = latencies.len() as u32;
    if successes == 0 {
        return dead_result(
            raw,
            proto,
            attempts,
            last_err.unwrap_or_else(|| "failed".to_string()),
        );
    }

    let avg = latencies.iter().sum::<u64>() / latencies.len() as u64;
    let jitter = if latencies.len() > 1 {
        Some(latencies.iter().max().unwrap() - latencies.iter().min().unwrap())
    } else {
        Some(0)
    };
    let success_rate = successes as f32 / attempts as f32;

    let mut res = ProxyResult {
        proxy: raw.clone(),
        url: url.clone(),
        proto,
        alive: true,
        latency_ms: Some(avg),
        jitter_ms: jitter,
        success_rate,
        attempts,
        ip: None,
        country: None,
        country_code: None,
        city: None,
        isp: None,
        org: None,
        asn: None,
        ip_type: None,
        anonymity: None,
        tamper: None,
        tls: None,
        tls_info: None,
        error: None,
        skipped: Vec::new(),
        sites: Vec::new(),
        dnsbl: None,
    };

    if let Some(b) = first_body {
        if flags.judge {
            let proxy_sha = try_parse_judge_body(&mut res, &b);
            if first_in_clear {
                let check = check_answer_signature(
                    &baseline,
                    &nonce,
                    direct_ip,
                    b.as_bytes(),
                    first_sig.as_deref(),
                )
                .await;
                apply_judge_integrity(&mut res, &baseline, check, proxy_sha.as_deref());
                if check == judge_sig::SignatureCheck::Invalid {
                    forget_judge_answer(&mut res);
                }
            } else {
                // Inside a TLS tunnel the proxy can neither add headers nor
                // change the body: "elite" would be a guess, not a result.
                res.anonymity = None;
            }
        } else {
            try_parse_test_body(&mut res, &test_url, &b);
        }
    }
    // Reuse previously seen geo. When the test URL does not reveal the exit
    // IP, the frontend-provided cache (TTL-filtered) is the only way to skip
    // a geo lookup for the default generate_204 URL.
    // Keyed by the normalized URL without credentials: raw column formats
    // (`host:port:user:pass`) would put the password into the key.
    if let Some(entry) = geo_cache.get(&strip_credentials(&url)) {
        apply_cached_geo(&mut res, entry);
    }
    // deep stages run concurrently: worst case ~1 timeout instead of 4 in a row
    if !flags.judge && !cancel.load(Ordering::Relaxed) {
        let need_geo = flags.geo && res.country.is_none() && res.ip_type.is_none();
        let want_anon = flags.anon;
        let want_tamper = flags.tamper;
        let want_tls = flags.tls;
        let direct_ip = baseline.direct_ip.clone();
        let (ex_len, ex_hash) = (baseline.example_len, baseline.example_hash);
        let cert_fp = baseline.example_cert_fp.clone();
        let url_c = url.clone();
        // Stages are skipped instead of queued when their limit is reached:
        // waiting would eat the per-proxy budget and turn live proxies into
        // timeouts. A skip is reported, so an empty column is explained.
        let (geo_r, anon_r, tamper_r, tls_r) = tokio::join!(
            async {
                if !need_geo {
                    return Deep::Ran(None);
                }
                if geo_cooling_down().await {
                    return Deep::Skipped;
                }
                let Ok(_permit) = geo_semaphore().try_acquire() else {
                    return Deep::Skipped;
                };
                Deep::Ran(fetch_geo(&client, flags.ipapi_key.as_deref()).await)
            },
            async {
                if !want_anon {
                    return Deep::Ran(None);
                }
                let Ok(_permit) = deep_semaphore(&ANON_SEM).try_acquire() else {
                    return Deep::Skipped;
                };
                Deep::Ran(detect_anonymity(&client, direct_ip.as_deref()).await)
            },
            async {
                if !want_tamper {
                    return Deep::Ran(None);
                }
                let Ok(_permit) = deep_semaphore(&TAMPER_SEM).try_acquire() else {
                    return Deep::Skipped;
                };
                Deep::Ran(detect_tamper(&client, ex_len, ex_hash).await)
            },
            async {
                if !want_tls {
                    return Deep::Ran(None);
                }
                let Ok(_permit) = deep_semaphore(&TLS_SEM).try_acquire() else {
                    return Deep::Skipped;
                };
                Deep::Ran(Some(
                    tls_via_proxy(&url_c, timeout, cert_fp.as_deref()).await,
                ))
            },
        );
        let mut skipped = Vec::new();
        if let Some(g) = geo_r.value("geo", &mut skipped) {
            apply_ipapi(&mut res, &g);
        }
        if let Some(anon) = anon_r.value("anon", &mut skipped) {
            res.anonymity = Some(anon);
        }
        if let Some(tamper) = tamper_r.value("tamper", &mut skipped) {
            res.tamper = Some(tamper);
        }
        if let Some((v, info)) = tls_r.value("tls", &mut skipped) {
            res.tls = v;
            res.tls_info = info;
        }
        res.skipped = skipped;
    }
    if let Some(http_url) = flags.judge_http.as_deref() {
        if is_tls_target(&test_url) && !cancel.load(Ordering::Relaxed) {
            judge_over_http(&client, http_url, &nonce, direct_ip, &baseline, &mut res).await;
        }
    }
    if !flags.sites.is_empty() && !cancel.load(Ordering::Relaxed) {
        match deep_semaphore(&SITES_SEM).try_acquire() {
            Ok(_permit) => res.sites = probe_sites(&client, &flags.sites).await,
            Err(_) => res.skipped.push("sites".to_string()),
        }
    }
    if flags.dnsbl && !cancel.load(Ordering::Relaxed) {
        if let Some(ip) = res.ip.clone() {
            match deep_semaphore(&DNSBL_SEM).try_acquire() {
                Ok(_permit) => res.dnsbl = crate::dnsbl::check_ip(&ip).await,
                Err(_) => res.skipped.push("dnsbl".to_string()),
            }
        }
    }
    res
}

fn signature_header(r: &reqwest::Response) -> Option<String> {
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
    if check == judge_sig::SignatureCheck::Invalid {
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
async fn judge_over_http(
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

/// Request every site through the proxy at once. Any HTTP answer, even an
/// error page, shows the site is reachable through it.
pub(crate) async fn probe_sites(client: &reqwest::Client, sites: &[String]) -> Vec<SiteProbe> {
    let mut set = tokio::task::JoinSet::new();
    for (i, site) in sites.iter().cloned().enumerate() {
        let client = client.clone();
        set.spawn(async move {
            let start = Instant::now();
            let probe = match client.get(&site).send().await {
                Ok(r) => SiteProbe {
                    url: site,
                    ok: true,
                    ms: Some(start.elapsed().as_millis() as u64),
                    status: Some(r.status().as_u16()),
                    error: None,
                },
                Err(e) => SiteProbe {
                    url: site,
                    ok: false,
                    ms: None,
                    status: None,
                    error: Some(request_error(&e)),
                },
            };
            (i, probe)
        });
    }
    let mut out: Vec<(usize, SiteProbe)> = Vec::new();
    while let Some(Ok(v)) = set.join_next().await {
        out.push(v);
    }
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, p)| p).collect()
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

async fn fetch_baseline(flags: CheckFlags, want_ip: bool) -> Baseline {
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
        let r = c.get("https://api.ipify.org").send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        let t = read_body_capped(r, BODY_LIMIT_SMALL).await.ok()?;
        let t = t.trim().to_string();
        if looks_like_ip(&t) {
            baseline_cache().lock().await.direct_ip = Some((t.clone(), Instant::now()));
            Some(t)
        } else {
            None
        }
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
        judge_content_sha: None,
        judge_key: judge_sig::KeyLookup::NotSigned,
        judge_key_source: None,
        judge_key_retry: tokio::sync::OnceCell::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_checks(
    proxies: Vec<String>,
    cfg: CheckConfig,
    geo_cache: Vec<GeoCacheEntry>,
    cancel: Arc<AtomicBool>,
    on_result: Option<&Channel<CheckEvent>>,
) -> Result<Vec<ProxyResult>, String> {
    let cfg = cfg.sanitized();
    let flags = cfg.flags();
    let timeout = cfg.timeout();
    let (conc, repeats) = (cfg.concurrency, cfg.repeats);
    let default_proto = cfg.default_proto.as_str();
    let test_url = cfg.test_url.clone();
    if test_url.is_empty() {
        return Err("empty test url".to_string());
    }
    if !is_http_url(&test_url) {
        return Err("test url must start with http(s)://".to_string());
    }

    let mut jobs: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut seen = HashSet::new();
    let mut seen_urls = HashSet::new();
    for raw in proxies {
        let t = raw.trim().to_string();
        if t.is_empty() || !seen.insert(t.clone()) {
            continue;
        }
        let mut cands = normalize_candidates(&t, default_proto);
        let was_valid = !cands.is_empty();
        // `1.2.3.4:8080` and `http://1.2.3.4:8080` are the same proxy.
        cands.retain(|(url, _)| seen_urls.insert(url.clone()));
        if was_valid && cands.is_empty() {
            continue;
        }
        jobs.push((t, cands));
        if jobs.len() >= 20_000 {
            break;
        }
    }

    let geo_map: Arc<HashMap<String, GeoCacheEntry>> = Arc::new(
        geo_cache
            .into_iter()
            .map(|e| (e.proxy.clone(), e))
            .collect(),
    );

    let judge_url = judge_baseline_url(flags.judge, flags.judge_http.as_deref(), &test_url);
    let baseline_phase = async {
        tokio::join!(
            tokio::time::timeout(
                Duration::from_secs(20),
                fetch_baseline(flags.clone(), wants_direct_ip(&flags, &test_url))
            ),
            async {
                match &judge_url {
                    Some(u) => tokio::join!(
                        fetch_direct_judge_sha(u, timeout),
                        judge_sig::fetch_key(u, timeout, false)
                    ),
                    None => (None, judge_sig::KeyLookup::NotSigned),
                }
            }
        )
    };
    // The baseline can take up to 20 s; Stop must not wait for it.
    let (baseline_res, (judge_sha, judge_key)) = tokio::select! {
        r = baseline_phase => r,
        _ = wait_cancelled(&cancel) => return Ok(Vec::new()),
    };
    let mut baseline_inner = baseline_res.unwrap_or(Baseline {
        direct_ip: None,
        example_len: None,
        example_hash: None,
        example_cert_fp: None,
        judge_content_sha: None,
        judge_key: judge_sig::KeyLookup::NotSigned,
        judge_key_source: None,
        judge_key_retry: tokio::sync::OnceCell::new(),
    });
    baseline_inner.judge_content_sha = judge_sha;
    baseline_inner.judge_key = judge_key;
    baseline_inner.judge_key_source = judge_url.map(|u| (u, timeout));
    let baseline = Arc::new(baseline_inner);
    let warned_key_over_http = judge_sig::key_over_http(&baseline.judge_key);
    if warned_key_over_http {
        if let Some(ch) = on_result {
            let _ = ch.send(CheckEvent::KeyOverHttp);
        }
    }

    let sem = Arc::new(Semaphore::new(conc));
    let mut set = tokio::task::JoinSet::new();
    let mut task_raw: HashMap<tokio::task::Id, String> = HashMap::new();

    for (raw, cands) in jobs {
        if cands.is_empty() {
            let raw_key = raw.clone();
            let r = dead_result(raw, "BAD".to_string(), 0, "bad format".to_string());
            let handle = set.spawn(async move { r });
            task_raw.insert(handle.id(), raw_key);
            continue;
        }
        let sem_c = sem.clone();
        let tu = test_url.clone();
        let base = baseline.clone();
        let gc = geo_map.clone();
        let cl = cancel.clone();
        let fl = flags.clone();
        let raw_key = raw.clone();
        let handle = set.spawn(async move {
            let _p = sem_c.acquire_owned().await.unwrap();
            if cands.len() == 1 {
                let (url, proto) = &cands[0];
                check_candidate(
                    raw,
                    url.clone(),
                    proto.clone(),
                    tu,
                    timeout,
                    repeats,
                    fl,
                    base,
                    gc,
                    cl,
                )
                .await
            } else {
                let (u1, p1) = &cands[0];
                let r1 = check_candidate(
                    raw.clone(),
                    u1.clone(),
                    p1.clone(),
                    tu.clone(),
                    timeout,
                    repeats,
                    fl.clone(),
                    base.clone(),
                    gc.clone(),
                    cl.clone(),
                )
                .await;
                if r1.alive {
                    return r1;
                }
                if cl.load(Ordering::Relaxed) {
                    return r1;
                }
                let (u2, p2) = &cands[1];
                let r2 = check_candidate(
                    raw.clone(),
                    u2.clone(),
                    p2.clone(),
                    tu,
                    timeout,
                    repeats,
                    fl,
                    base,
                    gc,
                    cl,
                )
                .await;
                if r2.alive {
                    return r2;
                }
                let e1 = r1.error.unwrap_or_default();
                let e2 = r2.error.unwrap_or_default();
                let mut combined = format!("http: {e1} / socks5: {e2}");
                truncate_utf8(&mut combined, 120);
                let mut dead = dead_result(raw, "HTTP/SOCKS5".to_string(), r1.attempts, combined);
                // Keyed like the alive results, by a normalized URL: the raw
                // line would split the history and, as `host:port:user:pass`,
                // put the password into it.
                dead.url = r1.url;
                dead
            }
        });
        task_raw.insert(handle.id(), raw_key);
    }

    let mut out = Vec::new();
    loop {
        // Stop must abort in-flight requests immediately instead of waiting
        // for the first task to finish its own timeout.
        tokio::select! {
            maybe = set.join_next_with_id() => {
                let r = match maybe {
                    Some(Ok((id, v))) => {
                        task_raw.remove(&id);
                        v
                    }
                    Some(Err(e)) => {
                        let raw = task_raw.remove(&e.id()).unwrap_or_else(|| "?".to_string());
                        if e.is_cancelled() {
                            continue;
                        }
                        dead_result(raw, "?".to_string(), 0, short_error(&e.to_string()))
                    }
                    None => break,
                };
                if let Some(ch) = on_result {
                    let _ = ch.send(CheckEvent::Result(Box::new(r.clone())));
                }
                out.push(r);
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if cancel.load(Ordering::Relaxed) {
                    set.abort_all();
                    break;
                }
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        set.abort_all();
    }
    // The run's second `/key` request may have come back over plain HTTP.
    let late_key_over_http = baseline
        .judge_key_retry
        .get()
        .is_some_and(judge_sig::key_over_http);
    if late_key_over_http && !warned_key_over_http {
        if let Some(ch) = on_result {
            let _ = ch.send(CheckEvent::KeyOverHttp);
        }
    }
    out.sort_by(|a, b| {
        b.alive.cmp(&a.alive).then(
            a.latency_ms
                .unwrap_or(u64::MAX)
                .cmp(&b.latency_ms.unwrap_or(u64::MAX)),
        )
    });
    Ok(out)
}

#[tauri::command]
pub(crate) async fn check_proxies(
    app: tauri::AppHandle,
    proxies: Vec<String>,
    config: CheckConfig,
    geo_cache: Option<Vec<GeoCacheEntry>>,
    state: State<'_, CheckState>,
    on_result: Channel<CheckEvent>,
) -> Result<(), String> {
    let busy = state.busy.clone();
    if busy.swap(true, Ordering::SeqCst) {
        return Err(CHECK_BUSY.to_string());
    }
    let _guard = BusyGuard(busy);
    state.cancel.store(false, Ordering::SeqCst);
    let probe = config.clone().sanitized();

    let results = run_checks(
        proxies,
        config,
        geo_cache.unwrap_or_default(),
        state.cancel.clone(),
        Some(&on_result),
    )
    .await?;
    // Every proxy dead is more often this machine being offline, or the check
    // server being down, than the whole list dying. Then the results are not
    // history: they would mark every proxy down in Uptime.
    let stopped = state.cancel.load(Ordering::SeqCst);
    if !stopped
        && !results.is_empty()
        && !results.iter().any(|r| r.alive)
        && !direct_reachable(&probe.test_url, probe.timeout()).await
    {
        let _ = on_result.send(CheckEvent::Unreachable);
    } else {
        // Recorded before `Done`, so the UI reads the updated history.
        crate::commands::history::record_results(&app, &results).await;
    }
    // The channel carries the authoritative result list; `Done` tells the UI
    // that it is drained. Returning the Vec as well duplicated the whole
    // payload over IPC.
    let _ = on_result.send(CheckEvent::Done);
    Ok(())
}

#[tauri::command]
pub(crate) fn check_busy(state: State<'_, CheckState>) -> bool {
    state.busy.load(Ordering::SeqCst)
}

/// Whether the check server answers without a proxy. Tells "every proxy is
/// dead" apart from "this machine is offline".
pub(crate) async fn direct_reachable(test_url: &str, timeout: Duration) -> bool {
    let Ok(client) = build_client(None, timeout, true) else {
        return false;
    };
    matches!(client.get(test_url.trim()).send().await, Ok(r) if r.status().is_success())
}

#[tauri::command]
pub(crate) async fn check_direct(test_url: String, timeout_ms: u64) -> Result<u64, String> {
    let test_url = test_url.trim().to_string();
    if !is_http_url(&test_url) {
        return Err("test url must start with http(s)://".to_string());
    }
    let timeout = Duration::from_millis(timeout_ms.clamp(TIMEOUT_MS.0, TIMEOUT_MS.1));
    let client = build_client(None, timeout, true).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let r = client
        .get(test_url)
        .send()
        .await
        .map_err(|e| request_error(&e))?;
    if !r.status().is_success() {
        return Err(format!("http {}", r.status().as_u16()));
    }
    Ok(start.elapsed().as_millis() as u64)
}
