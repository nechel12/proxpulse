use crate::commands::check::{run_checks, BusyGuard, CheckFlags, CheckState};
use crate::commands::dispatch::{
    pool_items, store_pool, DispatchItem, DispatchPoolItem, DispatchState,
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
fn merge_auto_items(
    results: &[ProxyResult],
    current: &HashMap<String, (String, Option<u64>, Option<String>)>,
    checked: &HashSet<String>,
    max_ping_ms: u64,
) -> (Vec<DispatchItem>, Option<u64>) {
    let mut items = Vec::new();
    let mut avg_sum = 0u64;
    let mut avg_n = 0u64;
    for r in results {
        if !r.alive {
            continue;
        }
        let Some((raw, _, old_country)) = current.get(&r.url) else {
            continue;
        };
        if max_ping_ms > 0 && r.latency_ms.is_some_and(|ms| ms > max_ping_ms) {
            continue;
        }
        // The average describes the pool that was actually kept, so it is
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
        if checked.contains(url) {
            continue;
        }
        items.push(DispatchItem {
            raw: raw.clone(),
            url: Some(url.clone()),
            latency: *latency,
            country: country.clone(),
        });
    }
    (items, avg_sum.checked_div(avg_n))
}

async fn run_auto_cycle(app: &tauri::AppHandle, kind: AutoKind) {
    let auto: State<'_, AutoState> = app.state();
    if auto.inner.running.swap(true, Ordering::SeqCst) {
        emit_auto_status(app, kind, "skipped", 0, 0, None, None);
        return;
    }
    let _run_guard = BusyGuard(auto.inner.running.clone());

    let settings = auto.inner.settings.lock().await.clone();
    let disp: State<'_, DispatchState> = app.state();
    let (urls, checked) = {
        let pool = disp.inner.pool.lock().await;
        let urls: Vec<String> = pool.iter().map(|u| u.url.clone()).collect();
        let checked: HashSet<String> = urls.iter().cloned().collect();
        (urls, checked)
    };
    let input_total = urls.len();
    if input_total == 0 {
        emit_auto_status(app, kind, "empty", 0, 0, None, None);
        return;
    }

    let check: State<'_, CheckState> = app.state();
    if check.busy.swap(true, Ordering::SeqCst) {
        emit_auto_status(app, kind, "skipped", 0, input_total, None, None);
        return;
    }
    let _busy_guard = BusyGuard(check.busy.clone());
    check.cancel.store(false, Ordering::SeqCst);
    emit_auto_status(app, kind, "running", 0, input_total, None, None);

    let timeout = Duration::from_millis(settings.timeout_ms.clamp(500, 60_000));
    let flags = CheckFlags {
        geo: settings.with_geo,
        anon: settings.with_anonymity,
        tamper: settings.with_tamper,
        tls: settings.with_tls,
        precheck: settings.precheck,
        precheck_timeout: Duration::from_millis(settings.precheck_timeout_ms.clamp(300, 10_000)),
        judge: settings.judge_mode,
        ipapi_key: settings
            .ipapi_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(Arc::from),
    };
    let result = run_checks(
        urls,
        settings.test_url.clone(),
        timeout,
        settings.concurrency.clamp(1, 500),
        settings.repeats.clamp(1, 5),
        &settings.default_proto,
        flags,
        Vec::new(),
        check.cancel.clone(),
        None,
    )
    .await;
    let canceled = check.cancel.load(Ordering::Relaxed);

    match result {
        Err(e) => emit_auto_status(app, kind, "error", 0, input_total, None, Some(e)),
        Ok(results) => {
            if canceled {
                let alive = results.iter().filter(|r| r.alive).count();
                emit_auto_status(app, kind, "stopped", alive, results.len(), None, None);
                return;
            }
            let total = results.len();
            let current: HashMap<String, (String, Option<u64>, Option<String>)> = {
                let pool = disp.inner.pool.lock().await;
                pool.iter()
                    .map(|u| (u.url.clone(), (u.raw.clone(), u.latency, u.country.clone())))
                    .collect()
            };
            let (items, avg) = merge_auto_items(&results, &current, &checked, settings.max_ping_ms);
            let kept = store_pool(&disp.inner, items).await;
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
            emit_auto_status(app, kind, "done", kept, total, avg, None);

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
        }
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
        run_auto_cycle(&app, kind).await;
        let now = epoch_secs();
        let settings = auto.inner.settings.lock().await.clone();
        match kind {
            AutoKind::Dispatch => auto.inner.disp_next.store(
                if settings.disp_enabled {
                    now + settings.disp_interval_min.clamp(1, 1440) * 60
                } else {
                    0
                },
                Ordering::SeqCst,
            ),
            AutoKind::Auto => auto.inner.auto_next.store(
                if settings.auto_enabled {
                    now + settings.auto_interval_min.clamp(1, 1440) * 60
                } else {
                    0
                },
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
        run_auto_cycle(&app, kind).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::check::dead_result;
    use std::collections::{HashMap, HashSet};

    fn alive_result(url: &str, latency: u64, country: &str) -> ProxyResult {
        let mut r = dead_result(url.into(), "HTTP".into(), 1, String::new());
        r.alive = true;
        r.url = url.into();
        r.latency_ms = Some(latency);
        r.country = Some(country.into());
        r.country_code = Some(country.into());
        r
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
        let checked = HashSet::from(["http://a:80".to_string(), "http://b:80".to_string()]);
        let results = vec![
            alive_result("http://a:80", 10, "FR"),
            alive_result("http://b:80", 999, "FR"),
        ];
        let (items, avg) = merge_auto_items(&results, &current, &checked, 100);
        let urls: Vec<&str> = items.iter().map(|i| i.url.as_deref().unwrap()).collect();
        assert_eq!(urls, vec!["http://a:80", "http://c:80"]);
        assert_eq!(items[0].raw, "raw-a");
        assert_eq!(items[0].latency, Some(10));
        assert_eq!(items[0].country.as_deref(), Some("FR"));
        assert_eq!(items[1].raw, "raw-c");
        assert_eq!(avg, Some(10));

        // A proxy deleted while the check was running is not resurrected.
        let current = HashMap::from([(
            "http://b:80".to_string(),
            ("raw-b".to_string(), Some(200), None),
        )]);
        let results = vec![alive_result("http://a:80", 10, "FR")];
        let (items, _) = merge_auto_items(&results, &current, &checked, 0);
        assert!(items.is_empty());
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
