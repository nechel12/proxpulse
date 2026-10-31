use crate::commands::check::{direct_reachable, run_checks, BusyGuard, CheckConfig, CheckState};
use crate::commands::dispatch::{
    pool_items, save_pool, store_pool, DispatchItem, DispatchPoolItem, DispatchState,
};
use crate::commands::webhook::send_webhooks_inner;
use crate::{epoch_secs, strip_credentials, GeoCacheEntry, ProxyResult, WebhookResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;

// ================= scheduled checks =================

/// One schedule for the shared pool. It used to be two independent timers
/// (dispatcher tab and auto tab) re-checking the same pool; now webhook
/// delivery is an option of the single schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct AutoSettings {
    enabled: bool,
    interval_min: u64,
    /// POST the pool to the webhooks after every scheduled check.
    send_webhooks: bool,
    max_ping_ms: u64,
    webhook_urls: Vec<String>,
    webhook_format: String,
    webhook_include_credentials: bool,
    check: CheckConfig,
}

impl Default for AutoSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_min: 30,
            send_webhooks: false,
            max_ping_ms: 0,
            webhook_urls: Vec::new(),
            webhook_format: "txt".to_string(),
            webhook_include_credentials: false,
            check: CheckConfig::default(),
        }
    }
}

pub(crate) struct AutoInner {
    pub(crate) settings: Mutex<AutoSettings>,
    /// Unix time of the next scheduled check, 0 when the schedule is off.
    pub(crate) next: AtomicU64,
    pub(crate) running: Arc<AtomicBool>,
}

pub(crate) struct AutoState {
    pub(crate) inner: Arc<AutoInner>,
}

impl Default for AutoState {
    fn default() -> Self {
        Self {
            inner: Arc::new(AutoInner {
                settings: Mutex::new(AutoSettings::default()),
                next: AtomicU64::new(0),
                running: Arc::new(AtomicBool::new(false)),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoStatus {
    state: String,
    alive: usize,
    total: usize,
    avg_ms: Option<u64>,
    error: Option<String>,
    at: u64,
}

#[derive(Debug, Clone, Serialize)]
struct AutoPoolPayload {
    items: Vec<DispatchPoolItem>,
}

#[derive(Debug, Clone, Serialize)]
struct AutoWebhookPayload {
    results: Vec<WebhookResult>,
}

fn emit_auto_status(
    app: &tauri::AppHandle,
    state: &str,
    alive: usize,
    total: usize,
    avg_ms: Option<u64>,
    error: Option<String>,
) {
    let _ = app.emit(
        "auto://status",
        AutoStatus {
            state: state.to_string(),
            alive,
            total,
            avg_ms,
            error,
            at: epoch_secs(),
        },
    );
}

/// A proxy that fails this many scheduled checks in a row leaves the pool.
pub(crate) const AUTO_DROP_AFTER: u32 = 2;
/// Retry delay after a cycle that could not reach the check server directly.
const OFFLINE_RETRY_SECS: u64 = 60;

type PoolEntry = (String, Option<u64>, Option<String>);

#[derive(Debug)]
pub(crate) struct AutoMerge {
    pub(crate) items: Vec<DispatchItem>,
    pub(crate) fails: HashMap<String, u32>,
    pub(crate) alive: usize,
    pub(crate) avg: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoOutcome {
    Finished,
    Offline,
}

/// Scheduled checks only keep the pool fresh: liveness, ping and country.
/// The merge discards anonymity, integrity and TLS results, so their extra
/// requests through every proxy (and third-party quota) are skipped.
pub(crate) fn pool_check_config(check: &CheckConfig) -> CheckConfig {
    CheckConfig {
        with_anonymity: false,
        with_tamper: false,
        with_tls: false,
        ..check.clone()
    }
}

/// Countries already known for pool entries, passed as a geo cache, so a
/// scheduled check looks geo up only for entries that have none.
pub(crate) fn pool_geo_cache(pool: &[DispatchPoolItem]) -> Vec<GeoCacheEntry> {
    pool.iter()
        .filter_map(|p| {
            let country = p.country.clone().filter(|c| !c.trim().is_empty())?;
            let is_code = country.len() == 2 && country.chars().all(|c| c.is_ascii_uppercase());
            Some(GeoCacheEntry {
                proxy: strip_credentials(&p.url),
                ip: None,
                country: (!is_code).then(|| country.clone()),
                country_code: is_code.then_some(country),
                city: None,
                isp: None,
                org: None,
                asn: None,
                ip_type: None,
            })
        })
        .collect()
}

/// Merge check results into the pool as it is *now*: entries deleted while
/// the check ran are not resurrected, and entries without a result (added
/// during the run) are kept unchanged.
pub(crate) fn merge_auto_items(
    results: &[ProxyResult],
    current: &HashMap<String, PoolEntry>,
    max_ping_ms: u64,
    prev_fails: &HashMap<String, u32>,
) -> AutoMerge {
    let mut items = Vec::new();
    let mut fails = HashMap::new();
    let mut seen = HashSet::new();
    let mut alive = 0;
    let mut avg_sum = 0u64;
    let mut avg_n = 0u64;
    for r in results {
        let Some((raw, latency, old_country)) = current.get(&r.url) else {
            continue;
        };
        seen.insert(r.url.as_str());
        if !r.alive {
            // One miss may be a hiccup on our side; drop only after several
            // consecutive failures.
            let n = prev_fails.get(&r.url).copied().unwrap_or(0) + 1;
            if n < AUTO_DROP_AFTER {
                fails.insert(r.url.clone(), n);
                items.push(DispatchItem {
                    raw: raw.clone(),
                    url: Some(r.url.clone()),
                    latency: *latency,
                    country: old_country.clone(),
                });
            }
            continue;
        }
        if max_ping_ms > 0 && r.latency_ms.is_some_and(|ms| ms > max_ping_ms) {
            continue;
        }
        alive += 1;
        // The average describes the alive part of the kept pool, so it is
        // accumulated only after the deletion and max-ping filters.
        if let Some(ms) = r.latency_ms {
            avg_sum += ms;
            avg_n += 1;
        }
        let country = r
            .country_code
            .clone()
            .or_else(|| r.country.clone())
            .or_else(|| old_country.clone());
        items.push(DispatchItem {
            raw: raw.clone(),
            url: Some(r.url.clone()),
            latency: r.latency_ms,
            country,
        });
    }
    for (url, (raw, latency, country)) in current {
        if seen.contains(url.as_str()) {
            continue;
        }
        if let Some(n) = prev_fails.get(url) {
            fails.insert(url.clone(), *n);
        }
        items.push(DispatchItem {
            raw: raw.clone(),
            url: Some(url.clone()),
            latency: *latency,
            country: country.clone(),
        });
    }
    AutoMerge {
        items,
        fails,
        alive,
        avg: avg_sum.checked_div(avg_n),
    }
}

async fn run_auto_cycle(app: &tauri::AppHandle) -> AutoOutcome {
    let auto: State<'_, AutoState> = app.state();
    if auto.inner.running.swap(true, Ordering::SeqCst) {
        emit_auto_status(app, "skipped", 0, 0, None, None);
        return AutoOutcome::Finished;
    }
    let _run_guard = BusyGuard(auto.inner.running.clone());

    let settings = auto.inner.settings.lock().await.clone();
    let disp: State<'_, DispatchState> = app.state();
    let pool_now = pool_items(&disp.inner).await;
    let urls: Vec<String> = pool_now.iter().map(|p| p.url.clone()).collect();
    let input_total = urls.len();
    if input_total == 0 {
        emit_auto_status(app, "empty", 0, 0, None, None);
        return AutoOutcome::Finished;
    }

    let check: State<'_, CheckState> = app.state();
    if check.busy.swap(true, Ordering::SeqCst) {
        emit_auto_status(app, "skipped", 0, input_total, None, None);
        return AutoOutcome::Finished;
    }
    let _busy_guard = BusyGuard(check.busy.clone());
    check.cancel.store(false, Ordering::SeqCst);
    emit_auto_status(app, "running", 0, input_total, None, None);

    let cfg = pool_check_config(&settings.check).sanitized();
    let timeout = cfg.timeout();
    let test_url = cfg.test_url.clone();
    let result = run_checks(
        urls,
        cfg,
        pool_geo_cache(&pool_now),
        check.cancel.clone(),
        None,
    )
    .await;
    let canceled = check.cancel.load(Ordering::Relaxed);

    match result {
        Err(e) => {
            emit_auto_status(app, "error", 0, input_total, None, Some(e));
            AutoOutcome::Finished
        }
        Ok(results) => {
            if canceled {
                let alive = results.iter().filter(|r| r.alive).count();
                emit_auto_status(app, "stopped", alive, results.len(), None, None);
                return AutoOutcome::Finished;
            }
            let total = results.len();
            // Zero alive proxies usually means this machine is offline (for
            // example right after waking from sleep), not that the whole pool
            // died. Without direct access to the check server the results say
            // nothing about the proxies, so the pool is left untouched.
            if !results.iter().any(|r| r.alive) && !direct_reachable(&test_url, timeout).await {
                emit_auto_status(app, "offline", 0, total, None, None);
                return AutoOutcome::Offline;
            }
            let current: HashMap<String, PoolEntry> = {
                let pool = disp.inner.pool.lock().await;
                pool.iter()
                    .map(|u| (u.url.clone(), (u.raw.clone(), u.latency, u.country.clone())))
                    .collect()
            };
            let prev_fails = disp.inner.fails.lock().await.clone();
            let merged = merge_auto_items(&results, &current, settings.max_ping_ms, &prev_fails);
            *disp.inner.fails.lock().await = merged.fails;
            store_pool(&disp.inner, merged.items, false).await;
            save_pool(app, &disp.inner).await;
            let pool = pool_items(&disp.inner).await;
            let _ = app.emit(
                "auto://pool",
                AutoPoolPayload {
                    items: pool.clone(),
                },
            );
            // The check and pool update are done; only webhook delivery
            // remains, and it must not keep a manual check blocked.
            drop(_busy_guard);
            emit_auto_status(app, "done", merged.alive, total, merged.avg, None);

            if settings.send_webhooks && !settings.webhook_urls.is_empty() {
                let webhook_items: Vec<DispatchItem> = pool
                    .iter()
                    .map(|p| DispatchItem {
                        raw: p.raw.clone(),
                        url: Some(p.url.clone()),
                        latency: p.latency,
                        country: p.country.clone(),
                    })
                    .collect();
                let results = send_webhooks_inner(
                    &settings.webhook_urls,
                    &settings.webhook_format,
                    settings.webhook_include_credentials,
                    Arc::new(webhook_items),
                )
                .await;
                let _ = app.emit("auto://webhook", AutoWebhookPayload { results });
            }
            AutoOutcome::Finished
        }
    }
}

/// Next run time after a cycle: an offline cycle is retried soon instead of
/// waiting a whole interval with possibly stale results.
fn next_run(enabled: bool, interval_min: u64, outcome: AutoOutcome, now: u64) -> u64 {
    if !enabled {
        0
    } else if outcome == AutoOutcome::Offline {
        now + OFFLINE_RETRY_SECS
    } else {
        now + interval_min.clamp(1, 1440) * 60
    }
}

pub(crate) async fn auto_scheduler_loop(app: tauri::AppHandle) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let auto: State<'_, AutoState> = app.state();
        if auto.inner.running.load(Ordering::SeqCst) {
            continue;
        }
        let next = auto.inner.next.load(Ordering::SeqCst);
        let enabled = auto.inner.settings.lock().await.enabled;
        if !enabled || next == 0 || epoch_secs() < next {
            continue;
        }
        let outcome = run_auto_cycle(&app).await;
        let settings = auto.inner.settings.lock().await.clone();
        auto.inner.next.store(
            next_run(
                settings.enabled,
                settings.interval_min,
                outcome,
                epoch_secs(),
            ),
            Ordering::SeqCst,
        );
    }
}

fn sanitize_auto_settings(mut s: AutoSettings) -> AutoSettings {
    s.interval_min = s.interval_min.clamp(1, 1440);
    s.check = s.check.sanitized();
    if s.check.test_url.is_empty() {
        s.check.test_url = CheckConfig::default().test_url;
    }
    s.webhook_format = match s.webhook_format.as_str() {
        "json" => "json".to_string(),
        "csv" => "csv".to_string(),
        _ => "txt".to_string(),
    };
    s.webhook_urls = s
        .webhook_urls
        .into_iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .take(10)
        .collect();
    s
}

#[tauri::command]
pub(crate) async fn set_auto_config(
    settings: AutoSettings,
    state: State<'_, AutoState>,
) -> Result<(), String> {
    let s = sanitize_auto_settings(settings);

    let old = {
        let mut cur = state.inner.settings.lock().await;
        let old = cur.clone();
        *cur = s.clone();
        old
    };
    if !s.enabled {
        state.inner.next.store(0, Ordering::SeqCst);
    } else if !old.enabled
        || old.interval_min != s.interval_min
        || state.inner.next.load(Ordering::SeqCst) == 0
    {
        state
            .inner
            .next
            .store(epoch_secs() + s.interval_min * 60, Ordering::SeqCst);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn run_auto_now(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn(async move {
        let _ = run_auto_cycle(&app).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::check::dead_result;
    use std::collections::HashMap;

    fn alive_result(url: &str, latency: u64, country: &str) -> ProxyResult {
        let mut r = dead_result(url.into(), "HTTP".into(), 1, String::new());
        r.alive = true;
        r.url = url.into();
        r.latency_ms = Some(latency);
        r.country = Some(country.into());
        r.country_code = Some(country.into());
        r
    }

    fn dead(url: &str) -> ProxyResult {
        dead_result(url.into(), "HTTP".into(), 1, "timeout".into())
    }

    fn urls(m: &AutoMerge) -> Vec<&str> {
        m.items.iter().map(|i| i.url.as_deref().unwrap()).collect()
    }

    #[test]
    fn auto_pool_merge_respects_deletions_and_additions() {
        let current = HashMap::from([
            (
                "http://a:80".to_string(),
                ("raw-a".to_string(), Some(100), Some("DE".to_string())),
            ),
            (
                "http://b:80".to_string(),
                ("raw-b".to_string(), Some(200), Some("NL".to_string())),
            ),
            (
                "http://c:80".to_string(),
                ("raw-c".to_string(), Some(300), Some("US".to_string())),
            ),
        ]);
        let results = vec![
            alive_result("http://a:80", 10, "FR"),
            alive_result("http://b:80", 999, "FR"),
        ];
        let m = merge_auto_items(&results, &current, 100, &HashMap::new());
        assert_eq!(urls(&m), vec!["http://a:80", "http://c:80"]);
        assert_eq!(m.items[0].raw, "raw-a");
        assert_eq!(m.items[0].latency, Some(10));
        assert_eq!(m.items[0].country.as_deref(), Some("FR"));
        assert_eq!(m.items[1].raw, "raw-c");
        assert_eq!(m.avg, Some(10));
        assert_eq!(m.alive, 1);

        // A proxy deleted while the check was running is not resurrected,
        // and an entry without a result is kept as is.
        let current = HashMap::from([(
            "http://b:80".to_string(),
            ("raw-b".to_string(), Some(200), None),
        )]);
        let results = vec![alive_result("http://a:80", 10, "FR")];
        let m = merge_auto_items(&results, &current, 0, &HashMap::new());
        assert_eq!(urls(&m), vec!["http://b:80"]);
        assert_eq!(m.alive, 0);
    }

    #[test]
    fn auto_pool_drops_only_after_consecutive_failures() {
        let current = HashMap::from([
            (
                "http://a:80".to_string(),
                ("raw-a".to_string(), Some(100), Some("DE".to_string())),
            ),
            (
                "http://b:80".to_string(),
                ("raw-b".to_string(), Some(200), None),
            ),
        ]);
        let results = vec![dead("http://a:80"), alive_result("http://b:80", 50, "NL")];

        // First miss: kept with its previous latency and country.
        let first = merge_auto_items(&results, &current, 0, &HashMap::new());
        assert_eq!(urls(&first), vec!["http://a:80", "http://b:80"]);
        assert_eq!(first.items[0].latency, Some(100));
        assert_eq!(first.items[0].country.as_deref(), Some("DE"));
        assert_eq!(first.alive, 1);
        assert_eq!(first.fails.get("http://a:80"), Some(&1));
        assert!(!first.fails.contains_key("http://b:80"));

        // Second consecutive miss: dropped.
        let second = merge_auto_items(&results, &current, 0, &first.fails);
        assert_eq!(urls(&second), vec!["http://b:80"]);
        assert!(second.fails.is_empty());

        // A success in between resets the counter.
        let recovered = vec![alive_result("http://a:80", 70, "DE")];
        let m = merge_auto_items(&recovered, &current, 0, &first.fails);
        assert!(!m.fails.contains_key("http://a:80"));
    }

    #[test]
    fn offline_cycle_is_retried_soon() {
        assert_eq!(next_run(false, 30, AutoOutcome::Offline, 1000), 0);
        assert_eq!(
            next_run(true, 30, AutoOutcome::Offline, 1000),
            1000 + OFFLINE_RETRY_SECS
        );
        assert_eq!(next_run(true, 30, AutoOutcome::Finished, 1000), 1000 + 1800);
    }

    #[test]
    fn scheduled_checks_skip_discarded_deep_checks() {
        let user = CheckConfig {
            with_geo: true,
            with_anonymity: true,
            with_tamper: true,
            with_tls: true,
            repeats: 3,
            ..CheckConfig::default()
        };
        let cfg = pool_check_config(&user);
        assert!(cfg.with_geo, "the country is kept in the pool");
        assert!(!cfg.with_anonymity && !cfg.with_tamper && !cfg.with_tls);
        assert_eq!(cfg.repeats, 3, "liveness settings are the user's");
    }

    #[test]
    fn pool_countries_become_geo_cache_without_credentials() {
        let pool = vec![
            DispatchPoolItem {
                raw: "u:p@1.2.3.4:8080".into(),
                url: "http://u:p@1.2.3.4:8080".into(),
                latency: Some(10),
                country: Some("DE".into()),
            },
            DispatchPoolItem {
                raw: "5.6.7.8:1080".into(),
                url: "socks5://5.6.7.8:1080".into(),
                latency: None,
                country: Some("Testland".into()),
            },
            DispatchPoolItem {
                raw: "9.9.9.9:80".into(),
                url: "http://9.9.9.9:80".into(),
                latency: None,
                country: None,
            },
        ];
        let cache = pool_geo_cache(&pool);
        assert_eq!(
            cache.len(),
            2,
            "entries without a country still get a lookup"
        );
        assert_eq!(cache[0].proxy, "http://1.2.3.4:8080");
        assert_eq!(cache[0].country_code.as_deref(), Some("DE"));
        assert_eq!(cache[1].country.as_deref(), Some("Testland"));
        assert!(cache[1].country_code.is_none());
    }

    #[test]
    fn auto_settings_are_sanitized() {
        let s = AutoSettings {
            interval_min: 9999,
            webhook_format: "xml".into(),
            webhook_urls: vec![" ".into(), " https://hook ".into()],
            check: CheckConfig {
                timeout_ms: 1,
                concurrency: 0,
                repeats: 99,
                precheck_timeout_ms: 1,
                test_url: "   ".into(),
                ipapi_key: Some("   ".into()),
                ..CheckConfig::default()
            },
            ..AutoSettings::default()
        };
        let s = sanitize_auto_settings(s);
        assert_eq!(s.interval_min, 1440);
        assert_eq!(s.check.timeout_ms, 1000);
        assert_eq!(s.check.concurrency, 1);
        assert_eq!(s.check.repeats, 5);
        assert_eq!(s.check.precheck_timeout_ms, 300);
        assert_eq!(s.check.test_url, CheckConfig::default().test_url);
        assert_eq!(s.webhook_format, "txt");
        assert_eq!(s.webhook_urls, vec!["https://hook"]);
        assert!(s.check.ipapi_key.is_none());
    }

    #[test]
    fn settings_deserialize_from_the_frontend_shape() {
        let json = r#"{
            "enabled": true, "intervalMin": 15, "sendWebhooks": true,
            "maxPingMs": 800, "webhookUrls": ["https://hook"], "webhookFormat": "json",
            "webhookIncludeCredentials": false,
            "check": { "testUrl": "https://cp.cloudflare.com/generate_204", "timeoutMs": 4000,
                       "concurrency": 80, "repeats": 2, "defaultProto": "socks5",
                       "withGeo": false, "withAnonymity": true, "withTamper": false,
                       "withTls": false, "precheck": true, "precheckTimeoutMs": 900,
                       "judgeMode": false, "ipapiKey": null }
        }"#;
        let s: AutoSettings = serde_json::from_str(json).unwrap();
        assert!(s.enabled && s.send_webhooks);
        assert_eq!(s.interval_min, 15);
        assert_eq!(s.check.concurrency, 80);
        assert_eq!(s.check.default_proto, "socks5");
        // Missing fields fall back to defaults instead of failing.
        let s: AutoSettings = serde_json::from_str(r#"{"enabled": true}"#).unwrap();
        assert_eq!(s.interval_min, 30);
        assert_eq!(s.check.test_url, CheckConfig::default().test_url);
    }
}
