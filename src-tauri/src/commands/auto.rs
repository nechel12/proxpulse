use crate::commands::check::{direct_reachable, run_checks, BusyGuard, CheckConfig, CheckState};
use crate::commands::dispatch::{
    pool_items, save_pool, store_pool, DispatchItem, DispatchPoolItem, DispatchState,
};
use crate::commands::webhook::send_webhooks_inner;
use crate::{epoch_secs, ProxyResult, WebhookResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;

// ================= scheduled checks =================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AutoSettings {
    disp_enabled: bool,
    disp_interval_min: u64,
    auto_enabled: bool,
    auto_interval_min: u64,
    max_ping_ms: u64,
    webhook_urls: Vec<String>,
    webhook_format: String,
    webhook_include_credentials: bool,
    test_url: String,
    timeout_ms: u64,
    concurrency: usize,
    repeats: usize,
    default_proto: String,
    with_geo: bool,
    with_anonymity: bool,
    with_tamper: bool,
    with_tls: bool,
    precheck: bool,
    precheck_timeout_ms: u64,
    judge_mode: bool,
    ipapi_key: Option<String>,
}

impl Default for AutoSettings {
    fn default() -> Self {
        Self {
            disp_enabled: false,
            disp_interval_min: 30,
            auto_enabled: false,
            auto_interval_min: 30,
            max_ping_ms: 0,
            webhook_urls: Vec::new(),
            webhook_format: "txt".to_string(),
            webhook_include_credentials: false,
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
        }
    }
}

pub(crate) struct AutoInner {
    pub(crate) settings: Mutex<AutoSettings>,
    pub(crate) disp_next: AtomicU64,
    pub(crate) auto_next: AtomicU64,
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
                disp_next: AtomicU64::new(0),
                auto_next: AtomicU64::new(0),
                running: Arc::new(AtomicBool::new(false)),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum AutoKind {
    Dispatch,
    Auto,
}

impl AutoKind {
    fn schedule(self) -> &'static str {
        match self {
            AutoKind::Dispatch => "disp",
            AutoKind::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoStatus {
    schedule: String,
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
    kind: AutoKind,
    state: &str,
    alive: usize,
    total: usize,
    avg_ms: Option<u64>,
    error: Option<String>,
) {
    let _ = app.emit(
        "auto://status",
        AutoStatus {
            schedule: kind.schedule().to_string(),
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

async fn run_auto_cycle(app: &tauri::AppHandle, kind: AutoKind) -> AutoOutcome {
    let auto: State<'_, AutoState> = app.state();
    if auto.inner.running.swap(true, Ordering::SeqCst) {
        emit_auto_status(app, kind, "skipped", 0, 0, None, None);
        return AutoOutcome::Finished;
    }
    let _run_guard = BusyGuard(auto.inner.running.clone());

    let settings = auto.inner.settings.lock().await.clone();
    let disp: State<'_, DispatchState> = app.state();
    let urls: Vec<String> = {
        let pool = disp.inner.pool.lock().await;
        pool.iter().map(|u| u.url.clone()).collect()
    };
    let input_total = urls.len();
    if input_total == 0 {
        emit_auto_status(app, kind, "empty", 0, 0, None, None);
        return AutoOutcome::Finished;
    }

    let check: State<'_, CheckState> = app.state();
    if check.busy.swap(true, Ordering::SeqCst) {
        emit_auto_status(app, kind, "skipped", 0, input_total, None, None);
        return AutoOutcome::Finished;
    }
    let _busy_guard = BusyGuard(check.busy.clone());
    check.cancel.store(false, Ordering::SeqCst);
    emit_auto_status(app, kind, "running", 0, input_total, None, None);

    let cfg = CheckConfig {
        test_url: settings.test_url.clone(),
        timeout_ms: settings.timeout_ms,
        concurrency: settings.concurrency,
        repeats: settings.repeats,
        default_proto: settings.default_proto.clone(),
        with_geo: settings.with_geo,
        with_anonymity: settings.with_anonymity,
        with_tamper: settings.with_tamper,
        with_tls: settings.with_tls,
        precheck: settings.precheck,
        precheck_timeout_ms: settings.precheck_timeout_ms,
        judge_mode: settings.judge_mode,
        ipapi_key: settings.ipapi_key.clone(),
    }
    .sanitized();
    let timeout = cfg.timeout();
    let result = run_checks(urls, cfg, Vec::new(), check.cancel.clone(), None).await;
    let canceled = check.cancel.load(Ordering::Relaxed);

    match result {
        Err(e) => {
            emit_auto_status(app, kind, "error", 0, input_total, None, Some(e));
            AutoOutcome::Finished
        }
        Ok(results) => {
            if canceled {
                let alive = results.iter().filter(|r| r.alive).count();
                emit_auto_status(app, kind, "stopped", alive, results.len(), None, None);
                return AutoOutcome::Finished;
            }
            let total = results.len();
            // Zero alive proxies usually means this machine is offline (for
            // example right after waking from sleep), not that the whole pool
            // died. Without direct access to the check server the results say
            // nothing about the proxies, so the pool is left untouched.
            if !results.iter().any(|r| r.alive)
                && !direct_reachable(&settings.test_url, timeout).await
            {
                emit_auto_status(app, kind, "offline", 0, total, None, None);
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
            emit_auto_status(app, kind, "done", merged.alive, total, merged.avg, None);

            if matches!(kind, AutoKind::Auto) && !settings.webhook_urls.is_empty() {
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
        let now = epoch_secs();
        let settings = auto.inner.settings.lock().await.clone();
        let disp_next = auto.inner.disp_next.load(Ordering::SeqCst);
        let auto_next = auto.inner.auto_next.load(Ordering::SeqCst);
        let kind = if settings.disp_enabled && disp_next != 0 && now >= disp_next {
            Some(AutoKind::Dispatch)
        } else if settings.auto_enabled && auto_next != 0 && now >= auto_next {
            Some(AutoKind::Auto)
        } else {
            None
        };
        let Some(kind) = kind else {
            continue;
        };
        let outcome = run_auto_cycle(&app, kind).await;
        let now = epoch_secs();
        let settings = auto.inner.settings.lock().await.clone();
        match kind {
            AutoKind::Dispatch => auto.inner.disp_next.store(
                next_run(
                    settings.disp_enabled,
                    settings.disp_interval_min,
                    outcome,
                    now,
                ),
                Ordering::SeqCst,
            ),
            AutoKind::Auto => auto.inner.auto_next.store(
                next_run(
                    settings.auto_enabled,
                    settings.auto_interval_min,
                    outcome,
                    now,
                ),
                Ordering::SeqCst,
            ),
        }
    }
}

fn sanitize_auto_settings(mut s: AutoSettings) -> AutoSettings {
    s.disp_interval_min = s.disp_interval_min.clamp(1, 1440);
    s.auto_interval_min = s.auto_interval_min.clamp(1, 1440);
    s.timeout_ms = s.timeout_ms.clamp(500, 60_000);
    s.concurrency = s.concurrency.clamp(1, 500);
    s.repeats = s.repeats.clamp(1, 5);
    s.precheck_timeout_ms = s.precheck_timeout_ms.clamp(300, 10_000);
    s.test_url = s.test_url.trim().to_string();
    if s.test_url.is_empty() {
        s.test_url = AutoSettings::default().test_url;
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
    s.ipapi_key = s
        .ipapi_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
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
    let now = epoch_secs();
    if s.disp_enabled {
        if !old.disp_enabled
            || old.disp_interval_min != s.disp_interval_min
            || state.inner.disp_next.load(Ordering::SeqCst) == 0
        {
            state
                .inner
                .disp_next
                .store(now + s.disp_interval_min * 60, Ordering::SeqCst);
        }
    } else {
        state.inner.disp_next.store(0, Ordering::SeqCst);
    }
    if s.auto_enabled {
        if !old.auto_enabled
            || old.auto_interval_min != s.auto_interval_min
            || state.inner.auto_next.load(Ordering::SeqCst) == 0
        {
            state
                .inner
                .auto_next
                .store(now + s.auto_interval_min * 60, Ordering::SeqCst);
        }
    } else {
        state.inner.auto_next.store(0, Ordering::SeqCst);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn run_auto_now(app: tauri::AppHandle, schedule: String) -> Result<(), String> {
    let kind = if schedule == "disp" {
        AutoKind::Dispatch
    } else {
        AutoKind::Auto
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = run_auto_cycle(&app, kind).await;
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
    fn auto_settings_are_sanitized() {
        let s = AutoSettings {
            disp_interval_min: 0,
            auto_interval_min: 9999,
            timeout_ms: 1,
            concurrency: 0,
            repeats: 99,
            precheck_timeout_ms: 1,
            test_url: "   ".into(),
            webhook_format: "xml".into(),
            webhook_urls: vec![" ".into(), " https://hook ".into()],
            ipapi_key: Some("   ".into()),
            ..AutoSettings::default()
        };
        let s = sanitize_auto_settings(s);
        assert_eq!(s.disp_interval_min, 1);
        assert_eq!(s.auto_interval_min, 1440);
        assert_eq!(s.timeout_ms, 500);
        assert_eq!(s.concurrency, 1);
        assert_eq!(s.repeats, 5);
        assert_eq!(s.precheck_timeout_ms, 300);
        assert_eq!(s.test_url, AutoSettings::default().test_url);
        assert_eq!(s.webhook_format, "txt");
        assert_eq!(s.webhook_urls, vec!["https://hook"]);
        assert!(s.ipapi_key.is_none());
    }
}
