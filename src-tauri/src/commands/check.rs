use crate::tls::{tls_direct_fp, tls_via_proxy};
use crate::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::State;

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
    }
}

#[tauri::command]
pub(crate) fn cancel_check(state: State<'_, CheckState>) -> Result<(), String> {
    state.cancel.store(true, Ordering::SeqCst);
    Ok(())
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
    flags.precheck_timeout + timeout * (repeats as u32 + 1) + deep + Duration::from_secs(5)
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
    match tokio::time::timeout(
        budget,
        check_candidate_inner(
            raw, url, proto, test_url, timeout, repeats, flags, baseline, geo_cache, cancel,
        ),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => dead_result(raw_fb, proto_fb, 0, "timeout".to_string()),
    }
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
                    Ok(Err(e)) => short_error(&e.to_string()),
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

    // judge mode: pass our direct IP so the judge can detect leaks
    let req_url = if flags.judge {
        match baseline.direct_ip.as_deref() {
            Some(dip) if !dip.is_empty() => {
                let sep = if test_url.contains('?') { '&' } else { '?' };
                format!("{test_url}{sep}direct_ip={dip}")
            }
            _ => test_url.clone(),
        }
    } else {
        test_url.clone()
    };

    let mut latencies: Vec<u64> = Vec::new();
    let mut first_body: Option<String> = None;
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
                        first_body = read_body_capped(r, BODY_LIMIT_PAGE).await.ok();
                    }
                    last_err = None;
                } else {
                    last_err = Some(format!("http {}", status.as_u16()));
                }
            }
            Err(e) => {
                last_err = Some(short_error(&e.to_string()));
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
    };

    if let Some(b) = first_body {
        if flags.judge {
            let proxy_sha = try_parse_judge_body(&mut res, &b);
            res.tamper = match (&baseline.judge_content_sha, &proxy_sha) {
                (Some(a), Some(c)) => Some(if a == c {
                    "ok".to_string()
                } else {
                    "modified".to_string()
                }),
                _ => None,
            };
        } else {
            try_parse_test_body(&mut res, &test_url, &b);
        }
    }
    // Reuse previously seen geo. When the test URL does not reveal the exit
    // IP, the frontend-provided cache (TTL-filtered) is the only way to skip
    // a geo lookup for the default generate_204 URL.
    if let Some(entry) = geo_cache.get(&strip_credentials(&raw)) {
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
        let (geo_r, anon_r, tamper_r, tls_r) = tokio::join!(
            async {
                if !need_geo {
                    return None;
                }
                // Skip geo instead of waiting: waiting would eat the
                // per-proxy check budget and turn live proxies into timeouts.
                let _permit = geo_semaphore().try_acquire().ok()?;
                fetch_geo(&client, flags.ipapi_key.as_deref()).await
            },
            async {
                if !want_anon {
                    return None;
                }
                let _permit = deep_semaphore(&ANON_SEM).try_acquire().ok()?;
                detect_anonymity(&client, direct_ip.as_deref()).await
            },
            async {
                if !want_tamper {
                    return None;
                }
                let _permit = deep_semaphore(&TAMPER_SEM).try_acquire().ok()?;
                detect_tamper(&client, ex_len, ex_hash).await
            },
            async {
                if !want_tls {
                    return None;
                }
                let _permit = deep_semaphore(&TLS_SEM).try_acquire().ok()?;
                Some(tls_via_proxy(&url_c, timeout, cert_fp.as_deref()).await)
            },
        );
        if let Some(g) = geo_r {
            apply_ipapi(&mut res, &g);
        }
        if anon_r.is_some() {
            res.anonymity = anon_r;
        }
        if tamper_r.is_some() {
            res.tamper = tamper_r;
        }
        if let Some((v, info)) = tls_r {
            res.tls = v;
            res.tls_info = info;
        }
    }
    res
}

async fn fetch_baseline(flags: CheckFlags) -> Baseline {
    let timeout = Duration::from_millis(9000);
    let want_ip = flags.judge || flags.anon;
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
        let r = c.get("https://example.com").send().await.ok()?;
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
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_checks(
    proxies: Vec<String>,
    test_url: String,
    timeout: Duration,
    conc: usize,
    repeats: usize,
    default_proto: &str,
    flags: CheckFlags,
    geo_cache: Vec<GeoCacheEntry>,
    cancel: Arc<AtomicBool>,
    on_result: Option<&Channel<ProxyResult>>,
) -> Result<Vec<ProxyResult>, String> {
    let test_url = test_url.trim().to_string();
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

    // Direct judge content hash (for tamper check in judge mode). The fast
    // mode returns an empty 204 body, so the hash would never be used; skip
    // the extra /judge request entirely.
    let judge_url = if flags.judge && !test_url.trim_end_matches('/').ends_with("/generate_204") {
        judge_direct_url(&test_url)
    } else {
        None
    };
    let (baseline_res, judge_sha) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(20), fetch_baseline(flags.clone())),
        async {
            match &judge_url {
                Some(u) => fetch_direct_judge_sha(u, timeout).await,
                None => None,
            }
        }
    );
    let mut baseline_inner = baseline_res.unwrap_or(Baseline {
        direct_ip: None,
        example_len: None,
        example_hash: None,
        example_cert_fp: None,
        judge_content_sha: None,
    });
    baseline_inner.judge_content_sha = judge_sha;
    let baseline = Arc::new(baseline_inner);

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
                dead_result(raw, "HTTP/SOCKS5".to_string(), r1.attempts, combined)
            }
        });
        task_raw.insert(handle.id(), raw_key);
    }

    let mut out = Vec::new();
    loop {
        // Stop must abort in-flight requests immediately instead of waiting
        // for the first task to finish its own timeout.
        tokio::select! {
            maybe = set.join_next_with_id() => match maybe {
                Some(Ok((id, v))) => {
                    task_raw.remove(&id);
                    if let Some(ch) = on_result {
                        let _ = ch.send(v.clone());
                    }
                    out.push(v);
                }
                Some(Err(e)) => {
                    let id = e.id();
                    let raw = task_raw.remove(&id).unwrap_or_else(|| "?".to_string());
                    if e.is_cancelled() {
                        continue;
                    }
                    out.push(dead_result(
                        raw,
                        "?".to_string(),
                        0,
                        short_error(&e.to_string()),
                    ));
                }
                None => break,
            },
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
#[allow(clippy::too_many_arguments)]
pub(crate) async fn check_proxies(
    proxies: Vec<String>,
    test_url: String,
    timeout_ms: u64,
    concurrency: usize,
    default_proto: String,
    repeats: Option<usize>,
    with_geo: Option<bool>,
    with_anonymity: Option<bool>,
    with_tamper: Option<bool>,
    with_tls: Option<bool>,
    precheck: Option<bool>,
    precheck_timeout_ms: Option<u64>,
    judge_mode: Option<bool>,
    ipapi_key: Option<String>,
    geo_cache: Option<Vec<GeoCacheEntry>>,
    state: State<'_, CheckState>,
    on_result: Channel<ProxyResult>,
) -> Result<(), String> {
    let busy = state.busy.clone();
    if busy.swap(true, Ordering::SeqCst) {
        return Err(CHECK_BUSY.to_string());
    }
    let _guard = BusyGuard(busy);
    state.cancel.store(false, Ordering::SeqCst);

    let timeout = Duration::from_millis(timeout_ms.clamp(500, 60_000));
    let conc = concurrency.clamp(1, 500);
    let repeats = repeats.unwrap_or(1).clamp(1, 5);
    let flags = CheckFlags {
        geo: with_geo.unwrap_or(true),
        anon: with_anonymity.unwrap_or(true),
        tamper: with_tamper.unwrap_or(true),
        tls: with_tls.unwrap_or(true),
        precheck: precheck.unwrap_or(true),
        precheck_timeout: Duration::from_millis(
            precheck_timeout_ms.unwrap_or(1500).clamp(300, 10_000),
        ),
        judge: judge_mode.unwrap_or(false),
        ipapi_key: ipapi_key
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| Arc::from(s.as_str())),
    };

    run_checks(
        proxies,
        test_url,
        timeout,
        conc,
        repeats,
        &default_proto,
        flags,
        geo_cache.unwrap_or_default(),
        state.cancel.clone(),
        Some(&on_result),
    )
    .await?;
    // The channel carries the authoritative result list; this marker tells
    // the UI that the channel is drained. Returning the Vec as well duplicated
    // the whole payload over IPC.
    let _ = on_result.send(dead_result(
        String::new(),
        "__done__".to_string(),
        0,
        String::new(),
    ));
    Ok(())
}

#[tauri::command]
pub(crate) fn check_busy(state: State<'_, CheckState>) -> bool {
    state.busy.load(Ordering::SeqCst)
}

#[tauri::command]
pub(crate) async fn check_direct(test_url: String, timeout_ms: u64) -> Result<u64, String> {
    let test_url = test_url.trim().to_string();
    if !is_http_url(&test_url) {
        return Err("test url must start with http(s)://".to_string());
    }
    let timeout = Duration::from_millis(timeout_ms.clamp(500, 30_000));
    let client = build_client(None, timeout, true).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let r = client
        .get(test_url)
        .send()
        .await
        .map_err(|e| short_error(&e.to_string()))?;
    if !r.status().is_success() {
        return Err(format!("http {}", r.status().as_u16()));
    }
    Ok(start.elapsed().as_millis() as u64)
}
