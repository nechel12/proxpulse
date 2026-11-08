use crate::commands::check::{direct_reachable, run_checks, BusyGuard, CheckConfig, CheckState};
use crate::commands::dispatch::{
    backup_pool, pool_items, save_pool, store_pool, write_private_file, DispatchInner,
    DispatchItem, DispatchPoolItem, DispatchState, MAIN_POOL,
};
use crate::commands::import::fetch_url_text;
use crate::commands::webhook::{
    send_webhooks_inner, Delivery, SentLists, WebhookOptions, TEMPLATE_MAX, WEBHOOK_FORMATS,
};
use crate::extract::extract_proxies;
use crate::proxy::normalize_candidates;
use crate::{epoch_secs, is_http_url, GeoCacheEntry, ProxyResult, WebhookResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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
    /// Body for the "template" webhook format.
    webhook_template: String,
    /// Send only what changed since the last delivery to each URL.
    webhook_changes_only: bool,
    /// Proxy list URLs fetched before every scheduled check; new alive
    /// proxies from them join the pool.
    subscriptions: Vec<String>,
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
            webhook_template: String::new(),
            webhook_changes_only: false,
            subscriptions: Vec::new(),
            check: CheckConfig::default(),
        }
    }
}

pub(crate) struct AutoInner {
    pub(crate) settings: Mutex<AutoSettings>,
    /// Unix time of the next scheduled check, 0 when the schedule is off.
    pub(crate) next: AtomicU64,
    pub(crate) running: Arc<AtomicBool>,
    /// Lists last delivered to each webhook, for change-only delivery.
    pub(crate) sent: SentLists,
    /// Suspicious cycles in a row (see [`looks_like_local_trouble`]).
    suspicious: AtomicU32,
    /// Unix time the last cycle ran, kept across restarts in the schedule
    /// file; 0 when it never ran.
    pub(crate) last_run: AtomicU64,
    /// Offline cycles in a row: the retry waits longer after each.
    offline: AtomicU32,
}

impl AutoInner {
    fn new() -> Self {
        Self {
            settings: Mutex::new(AutoSettings::default()),
            next: AtomicU64::new(0),
            running: Arc::new(AtomicBool::new(false)),
            sent: SentLists::default(),
            suspicious: AtomicU32::new(0),
            last_run: AtomicU64::new(0),
            offline: AtomicU32::new(0),
        }
    }
}

/// One schedule per pool: the main one, reached directly by the tray, and
/// the named pools by id.
pub(crate) struct AutoState {
    pub(crate) inner: Arc<AutoInner>,
    extra: std::sync::RwLock<HashMap<String, Arc<AutoInner>>>,
}

impl Default for AutoState {
    fn default() -> Self {
        Self {
            inner: Arc::new(AutoInner::new()),
            extra: std::sync::RwLock::new(HashMap::new()),
        }
    }
}

impl AutoState {
    pub(crate) fn pool(&self, id: Option<&str>) -> Result<Arc<AutoInner>, String> {
        match id.unwrap_or(MAIN_POOL) {
            MAIN_POOL => Ok(self.inner.clone()),
            id => self
                .extra
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .get(id)
                .cloned()
                .ok_or_else(|| "unknown pool".to_string()),
        }
    }

    pub(crate) fn add(&self, id: &str) {
        self.extra
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(AutoInner::new()));
    }

    pub(crate) fn remove(&self, id: &str) {
        self.extra
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    fn all(&self) -> Vec<(String, Arc<AutoInner>)> {
        let mut out = vec![(MAIN_POOL.to_string(), self.inner.clone())];
        let extra = self.extra.read().unwrap_or_else(|e| e.into_inner());
        out.extend(extra.iter().map(|(id, a)| (id.clone(), a.clone())));
        out
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoStatus {
    pool: String,
    state: String,
    alive: usize,
    total: usize,
    avg_ms: Option<u64>,
    error: Option<String>,
    at: u64,
}

#[derive(Debug, Clone, Serialize)]
struct AutoPoolPayload {
    pool: String,
    items: Vec<DispatchPoolItem>,
}

#[derive(Debug, Clone, Serialize)]
struct AutoWebhookPayload {
    pool: String,
    results: Vec<WebhookResult>,
}

/// What one subscription delivered in a scheduled check.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubscriptionResult {
    index: usize,
    ok: bool,
    /// Proxies in the list.
    found: usize,
    /// Proxies not in the pool yet; they are checked with it.
    added: usize,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct AutoSubscriptionPayload {
    pool: String,
    results: Vec<SubscriptionResult>,
}

/// Most subscriptions, and most new proxies one cycle takes from them.
const SUBSCRIPTIONS_MAX: usize = 10;
const SUBSCRIPTION_NEW_MAX: usize = 20_000;

/// Entries of a fetched list that are not in the pool yet (by normalized
/// URL), up to `room`.
pub(crate) fn new_entries(
    entries: Vec<String>,
    default_proto: &str,
    known: &mut HashSet<String>,
    room: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    for e in entries {
        if out.len() >= room {
            break;
        }
        let urls: Vec<String> = normalize_candidates(&e, default_proto)
            .into_iter()
            .map(|(url, _)| url)
            .collect();
        if urls.is_empty() || urls.iter().any(|u| known.contains(u)) {
            continue;
        }
        known.extend(urls);
        out.push(e);
    }
    out
}

/// Whether a proxy line points into this machine or its local network. A
/// subscription is someone else's list: its `127.0.0.1:7890` or
/// `socks5://192.168.1.1:80` would get a connection and a request through it,
/// and an answer would put it into the pool, the dispatcher and webhooks. The
/// user's own lists are not limited.
pub(crate) fn points_inside(entry: &str, default_proto: &str) -> bool {
    normalize_candidates(entry, default_proto)
        .iter()
        .any(|(url, _)| {
            reqwest::Url::parse(url).map_or(true, |u| crate::netguard::is_disallowed_url_host(&u))
        })
}

async fn fetch_subscriptions(
    urls: &[String],
    default_proto: &str,
    known: &mut HashSet<String>,
) -> (Vec<String>, Vec<SubscriptionResult>) {
    let mut fresh = Vec::new();
    let mut results = Vec::new();
    for (index, url) in urls.iter().take(SUBSCRIPTIONS_MAX).enumerate() {
        let text = match fetch_url_text(url.clone()).await {
            Ok(t) => t,
            Err(e) => {
                results.push(SubscriptionResult {
                    index,
                    ok: false,
                    found: 0,
                    added: 0,
                    error: Some(e),
                });
                continue;
            }
        };
        let proto = default_proto.to_string();
        let mut entries = tauri::async_runtime::spawn_blocking(move || {
            extract_proxies(&text, SUBSCRIPTION_NEW_MAX, &proto)
        })
        .await
        .unwrap_or_default();
        let found = entries.len();
        entries.retain(|e| !points_inside(e, default_proto));
        let room = SUBSCRIPTION_NEW_MAX.saturating_sub(fresh.len());
        let added = new_entries(entries, default_proto, known, room);
        results.push(SubscriptionResult {
            index,
            ok: true,
            found,
            added: added.len(),
            error: None,
        });
        fresh.extend(added);
    }
    (fresh, results)
}

fn emit_auto_status(
    app: &tauri::AppHandle,
    pool: &str,
    state: &str,
    alive: usize,
    total: usize,
    avg_ms: Option<u64>,
    error: Option<String>,
) {
    let _ = app.emit(
        "auto://status",
        AutoStatus {
            pool: pool.to_string(),
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
/// Slower than the pool's max ping counts as a failure too.
pub(crate) const AUTO_DROP_AFTER: u32 = 2;
/// A cycle in which this many pool entries or more, and more than half of
/// the pool, missed (dead or over the max ping) is suspicious.
const LOCAL_TROUBLE_MIN: usize = 3;

/// Whether a cycle in which `missed` of `total` pool entries failed more
/// likely says something about this machine (a congested uplink, a VPN or
/// firewall blocking proxy ports) than about the proxies.
pub(crate) fn looks_like_local_trouble(total: usize, missed: usize) -> bool {
    missed >= LOCAL_TROUBLE_MIN && missed * 2 > total
}

/// Whether to apply a cycle, given whether it is suspicious and how many
/// suspicious cycles came right before it. The first suspicious cycle in a
/// row changes neither the pool nor the history; a second one in a row is
/// trusted, so a pool that really died is still pruned (two misses later,
/// with a backup), just one cycle later.
pub(crate) fn apply_cycle(suspicious: bool, streak_before: u32) -> bool {
    !suspicious || streak_before >= 1
}
/// Retry delay after a cycle that could not run usefully: the check server
/// was unreachable directly, or a manual check was in progress.
const RETRY_SOON_SECS: u64 = 60;

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
    /// A manual check held the checker; the pool was not checked.
    Busy,
}

/// Scheduled checks only keep the pool fresh: liveness, ping and country.
/// The merge discards anonymity, integrity and TLS results, so their extra
/// requests through every proxy (and third-party quota) are skipped.
pub(crate) fn pool_check_config(check: &CheckConfig) -> CheckConfig {
    CheckConfig {
        with_anonymity: false,
        with_tamper: false,
        with_tls: false,
        sites: Vec::new(),
        judge_http_url: None,
        with_dnsbl: false,
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
                proxy: crate::proxy::geo_key(&p.url),
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

/// Alive proxies from subscriptions join the pool after the regular merge.
pub(crate) fn add_new_items(
    merged: &mut AutoMerge,
    results: &[ProxyResult],
    fresh: &HashSet<String>,
    max_ping_ms: u64,
) {
    let mut have: HashSet<String> = merged.items.iter().filter_map(|i| i.url.clone()).collect();
    let mut sum = merged.avg.unwrap_or(0) * merged.alive as u64;
    let mut n = merged.alive as u64;
    for r in results {
        if !r.alive || !fresh.contains(&r.proxy) || !have.insert(r.url.clone()) {
            continue;
        }
        if max_ping_ms > 0 && r.latency_ms.is_some_and(|ms| ms > max_ping_ms) {
            continue;
        }
        merged.alive += 1;
        if let Some(ms) = r.latency_ms {
            sum += ms;
            n += 1;
        }
        merged.items.push(DispatchItem {
            raw: r.proxy.clone(),
            url: Some(r.url.clone()),
            latency: r.latency_ms,
            country: r.country_code.clone().or_else(|| r.country.clone()),
        });
    }
    merged.avg = sum.checked_div(n);
}

/// Alive, but slower than the pool's max ping (`0` means no limit).
fn is_slow(r: &ProxyResult, max_ping_ms: u64) -> bool {
    r.alive && max_ping_ms > 0 && r.latency_ms.is_some_and(|ms| ms > max_ping_ms)
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
        let Some((raw, _, old_country)) = current.get(&r.url) else {
            continue;
        };
        seen.insert(r.url.as_str());
        let slow = is_slow(r, max_ping_ms);
        if !r.alive || slow {
            // One miss may be a hiccup on our side (a congested uplink makes
            // every proxy slow); drop only after several consecutive misses.
            let n = prev_fails.get(&r.url).copied().unwrap_or(0) + 1;
            if n < AUTO_DROP_AFTER {
                fails.insert(r.url.clone(), n);
                items.push(DispatchItem {
                    raw: raw.clone(),
                    url: Some(r.url.clone()),
                    // A slow proxy keeps its new latency and a dead one has
                    // none, so both sort down: its old latency kept a dead
                    // proxy first in the dispatcher's "fastest" order.
                    latency: if slow { r.latency_ms } else { None },
                    country: old_country.clone(),
                });
            }
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

async fn run_auto_cycle(app: &tauri::AppHandle, pool: &str) -> AutoOutcome {
    let (Ok(auto), Ok(disp)) = (
        app.state::<AutoState>().pool(Some(pool)),
        app.state::<DispatchState>().pool(Some(pool)),
    ) else {
        return AutoOutcome::Finished;
    };
    // This pool's cycle is already running ("Check now" during it): the
    // interface shows that one; a "skipped" here would end it there.
    if auto.running.swap(true, Ordering::SeqCst) {
        return AutoOutcome::Busy;
    }
    let run_guard = BusyGuard(auto.running.clone());

    let settings = auto.settings.lock().await.clone();
    let pool_now = pool_items(&disp).await;
    // A manual check holds the checker: skip before fetching subscriptions.
    if app.state::<CheckState>().busy.load(Ordering::SeqCst) {
        emit_auto_status(app, pool, "skipped", 0, pool_now.len(), None, None);
        return AutoOutcome::Busy;
    }
    // Still offline after an offline cycle: one direct request says so,
    // without fetching subscriptions and checking the whole pool again.
    if auto.offline.load(Ordering::SeqCst) > 0 {
        let cfg = pool_check_config(&settings.check).sanitized();
        if !direct_reachable(&cfg.test_url, cfg.timeout()).await {
            emit_auto_status(app, pool, "offline", 0, pool_now.len(), None, None);
            return AutoOutcome::Offline;
        }
    }
    let mut urls: Vec<String> = pool_now.iter().map(|p| p.url.clone()).collect();
    let mut fresh = HashSet::new();
    if !settings.subscriptions.is_empty() {
        emit_auto_status(app, pool, "fetching", 0, urls.len(), None, None);
        let mut known: HashSet<String> = urls.iter().cloned().collect();
        let (new, results) = fetch_subscriptions(
            &settings.subscriptions,
            &settings.check.default_proto,
            &mut known,
        )
        .await;
        let _ = app.emit(
            "auto://subscriptions",
            AutoSubscriptionPayload {
                pool: pool.to_string(),
                results,
            },
        );
        fresh = new.iter().cloned().collect();
        urls.extend(new);
    }
    let input_total = urls.len();
    if input_total == 0 {
        emit_auto_status(app, pool, "empty", 0, 0, None, None);
        return AutoOutcome::Finished;
    }

    let check: State<'_, CheckState> = app.state();
    if check.busy.swap(true, Ordering::SeqCst) {
        emit_auto_status(app, pool, "skipped", 0, input_total, None, None);
        return AutoOutcome::Busy;
    }
    let _busy_guard = BusyGuard(check.busy.clone());
    check.cancel.store(false, Ordering::SeqCst);
    emit_auto_status(app, pool, "running", 0, input_total, None, None);

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
            emit_auto_status(app, pool, "error", 0, input_total, None, Some(e));
            AutoOutcome::Finished
        }
        Ok(results) => {
            if canceled {
                let alive = results.iter().filter(|r| r.alive).count();
                emit_auto_status(app, pool, "stopped", alive, results.len(), None, None);
                return AutoOutcome::Finished;
            }
            let total = results.len();
            // Zero alive proxies usually means this machine is offline (for
            // example right after waking from sleep), not that the whole pool
            // died. Without direct access to the check server the results say
            // nothing about the proxies, so the pool is left untouched.
            if !results.iter().any(|r| r.alive) && !direct_reachable(&test_url, timeout).await {
                emit_auto_status(app, pool, "offline", 0, total, None, None);
                return AutoOutcome::Offline;
            }
            let current: HashMap<String, PoolEntry> = {
                let pool = disp.pool.lock().await;
                pool.iter()
                    .map(|u| (u.url.clone(), (u.raw.clone(), u.latency, u.country.clone())))
                    .collect()
            };
            // Entries of the pool that answered (fast or slow) and that missed.
            let in_pool = |r: &&ProxyResult| current.contains_key(&r.url);
            let answered = results.iter().filter(in_pool).filter(|r| r.alive).count();
            let missed = results
                .iter()
                .filter(in_pool)
                .filter(|r| !r.alive || is_slow(r, settings.max_ping_ms))
                .count();
            let suspicious = looks_like_local_trouble(current.len(), missed);
            let streak_before = if suspicious {
                auto.suspicious.fetch_add(1, Ordering::SeqCst)
            } else {
                auto.suspicious.store(0, Ordering::SeqCst);
                0
            };
            if !apply_cycle(suspicious, streak_before) {
                // Neither the pool, its miss counts nor the history changes:
                // what the cycle saw was more likely a problem on this machine.
                emit_auto_status(app, pool, "suspect", answered, current.len(), None, None);
                return AutoOutcome::Finished;
            }
            let prev_fails = disp.fails.lock().await.clone();
            let mut merged =
                merge_auto_items(&results, &current, settings.max_ping_ms, &prev_fails);
            let removed = {
                let kept: HashSet<&str> = merged
                    .items
                    .iter()
                    .filter_map(|i| i.url.as_deref())
                    .collect();
                current
                    .keys()
                    .filter(|u| !kept.contains(u.as_str()))
                    .count()
            };
            crate::commands::history::record_results(app, &results).await;
            add_new_items(&mut merged, &results, &fresh, settings.max_ping_ms);
            *disp.fails.lock().await = merged.fails;
            if removed > 0 {
                backup_pool(app, &disp).await;
            }
            store_pool(&disp, merged.items, false).await;
            save_pool(app, &disp).await;
            let pool_id = pool.to_string();
            let pool_list = pool_items(&disp).await;
            let _ = app.emit(
                "auto://pool",
                AutoPoolPayload {
                    pool: pool_id.clone(),
                    items: pool_list.clone(),
                },
            );
            // The check and pool update are done; only webhook delivery
            // remains, and it must not keep a manual check blocked.
            drop(_busy_guard);
            emit_auto_status(app, pool, "done", merged.alive, total, merged.avg, None);

            if settings.send_webhooks && !settings.webhook_urls.is_empty() {
                let webhook_items: Vec<DispatchItem> = pool_list
                    .iter()
                    .map(|p| DispatchItem {
                        raw: p.raw.clone(),
                        url: Some(p.url.clone()),
                        latency: p.latency,
                        country: p.country.clone(),
                    })
                    .collect();
                // Delivery with retries can take a minute: in the background,
                // so other pools' schedules do not wait for it. This pool
                // stays running until it ends, so its next cycle cannot
                // deliver over it.
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _run_guard = run_guard;
                    let opts = WebhookOptions {
                        format: settings.webhook_format.clone(),
                        include_credentials: settings.webhook_include_credentials,
                        template: settings.webhook_template.clone(),
                    };
                    let delivery = Delivery {
                        retry: true,
                        changes_only: settings.webhook_changes_only,
                        sent: Some(&auto.sent),
                    };
                    let results = send_webhooks_inner(
                        &settings.webhook_urls,
                        &opts,
                        Arc::new(webhook_items),
                        delivery,
                    )
                    .await;
                    let _ = app.emit(
                        "auto://webhook",
                        AutoWebhookPayload {
                            pool: pool_id.clone(),
                            results,
                        },
                    );
                    // The lists just delivered, for change-only delivery
                    // after a restart.
                    if settings.webhook_changes_only {
                        save_schedule(&app, &pool_id, &auto, &disp).await;
                    }
                });
            }
            AutoOutcome::Finished
        }
    }
}

/// Next run time after a cycle. A crowded-out cycle is retried soon instead
/// of waiting a whole interval with stale results; an offline one too, but
/// each further offline cycle in a row waits twice as long, up to the
/// interval, so a machine that stays offline is not checked every minute.
fn next_run(enabled: bool, interval_min: u64, outcome: AutoOutcome, now: u64, offline: u32) -> u64 {
    let interval = interval_min.clamp(1, 1440) * 60;
    if !enabled {
        0
    } else if outcome == AutoOutcome::Busy {
        now + RETRY_SOON_SECS
    } else if outcome == AutoOutcome::Offline {
        now + (RETRY_SOON_SECS << offline.saturating_sub(1).min(6)).min(interval)
    } else {
        now + interval
    }
}

/// The first run after the schedule is set up: one interval after the last
/// run, which survives restarts, but not sooner than a minute from now. A
/// daily schedule on a machine restarted every day would otherwise never run.
fn first_run(interval_min: u64, last_run: u64, now: u64) -> u64 {
    let interval = interval_min.clamp(1, 1440) * 60;
    if last_run == 0 {
        now + interval
    } else {
        (last_run + interval).max(now + RETRY_SOON_SECS)
    }
}

/// What a schedule keeps across restarts: when it last ran, the miss counts
/// of its pool's entries, and the lists last delivered to each webhook (for
/// change-only delivery; they may hold credentials, like the pool file).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ScheduleFile {
    pub(crate) last_run: u64,
    pub(crate) fails: HashMap<String, u32>,
    pub(crate) sent: HashMap<String, Vec<String>>,
}

pub(crate) fn schedule_file_name(id: &str) -> String {
    if id == MAIN_POOL {
        "schedule.json".to_string()
    } else {
        format!("schedule-{id}.json")
    }
}

fn schedule_path(app: &tauri::AppHandle, id: &str) -> Option<std::path::PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join(schedule_file_name(id)))
}

/// Saves of schedule files one at a time: a cycle's book-keeping and its
/// background webhook delivery can save the same file at once, and both
/// write through one temporary file.
static SCHEDULE_SAVE: Mutex<()> = Mutex::const_new(());

async fn save_schedule(app: &tauri::AppHandle, id: &str, auto: &AutoInner, disp: &DispatchInner) {
    let Some(path) = schedule_path(app, id) else {
        return;
    };
    // Taken before reading the state, so the last save has the newest.
    let _saving = SCHEDULE_SAVE.lock().await;
    let file = ScheduleFile {
        last_run: auto.last_run.load(Ordering::SeqCst),
        fails: disp.fails.lock().await.clone(),
        sent: auto
            .sent
            .lock()
            .await
            .iter()
            .map(|(url, list)| (url.clone(), list.iter().cloned().collect()))
            .collect(),
    };
    let Ok(bytes) = serde_json::to_vec(&file) else {
        return;
    };
    let _ = tauri::async_runtime::spawn_blocking(move || write_private_file(&path, &bytes)).await;
}

/// Restore a pool's schedule state at startup.
pub(crate) async fn load_schedule(app: &tauri::AppHandle, id: &str) {
    let (Ok(auto), Ok(disp)) = (
        app.state::<AutoState>().pool(Some(id)),
        app.state::<DispatchState>().pool(Some(id)),
    ) else {
        return;
    };
    let Some(path) = schedule_path(app, id) else {
        return;
    };
    let Ok(Ok(bytes)) = tauri::async_runtime::spawn_blocking(move || std::fs::read(path)).await
    else {
        return;
    };
    let Ok(file) = serde_json::from_slice::<ScheduleFile>(&bytes) else {
        return;
    };
    auto.last_run.store(file.last_run, Ordering::SeqCst);
    *disp.fails.lock().await = file.fails;
    *auto.sent.lock().await = file
        .sent
        .into_iter()
        .map(|(url, list)| (url, list.into_iter().collect()))
        .collect();
}

/// Remove a deleted pool's schedule file.
pub(crate) fn delete_schedule(app: &tauri::AppHandle, id: &str) {
    if let Some(path) = schedule_path(app, id) {
        let _ = std::fs::remove_file(path);
    }
}

/// Book-keeping after a cycle, scheduled or "Check now": the offline streak,
/// the last run, the next run, and the schedule file.
async fn finish_cycle(app: &tauri::AppHandle, id: &str, auto: &AutoInner, outcome: AutoOutcome) {
    let offline = match outcome {
        AutoOutcome::Offline => auto.offline.fetch_add(1, Ordering::SeqCst) + 1,
        AutoOutcome::Finished => {
            auto.offline.store(0, Ordering::SeqCst);
            auto.last_run.store(epoch_secs(), Ordering::SeqCst);
            0
        }
        AutoOutcome::Busy => auto.offline.load(Ordering::SeqCst),
    };
    let settings = auto.settings.lock().await.clone();
    auto.next.store(
        next_run(
            settings.enabled,
            settings.interval_min,
            outcome,
            epoch_secs(),
            offline,
        ),
        Ordering::SeqCst,
    );
    if outcome == AutoOutcome::Finished {
        if let Ok(disp) = app.state::<DispatchState>().pool(Some(id)) {
            save_schedule(app, id, auto, &disp).await;
        }
    }
}

pub(crate) async fn auto_scheduler_loop(app: tauri::AppHandle) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        // Pools take turns; a cycle crowded out by another one is retried
        // a minute later like one crowded out by a manual check.
        let pools = app.state::<AutoState>().all();
        for (id, auto) in pools {
            if auto.running.load(Ordering::SeqCst) {
                continue;
            }
            let next = auto.next.load(Ordering::SeqCst);
            let enabled = auto.settings.lock().await.enabled;
            if !enabled || next == 0 || epoch_secs() < next {
                continue;
            }
            let outcome = run_auto_cycle(&app, &id).await;
            finish_cycle(&app, &id, &auto, outcome).await;
        }
    }
}

fn sanitize_auto_settings(mut s: AutoSettings) -> AutoSettings {
    s.interval_min = s.interval_min.clamp(1, 1440);
    s.check = s.check.sanitized();
    if s.check.test_url.is_empty() {
        s.check.test_url = CheckConfig::default().test_url;
    }
    if !WEBHOOK_FORMATS.contains(&s.webhook_format.as_str()) {
        s.webhook_format = "txt".to_string();
    }
    crate::http::truncate_utf8(&mut s.webhook_template, TEMPLATE_MAX);
    s.subscriptions = s
        .subscriptions
        .into_iter()
        .map(|u| u.trim().to_string())
        .filter(|u| is_http_url(u))
        .take(SUBSCRIPTIONS_MAX)
        .collect();
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
    pool: Option<String>,
    settings: AutoSettings,
    state: State<'_, AutoState>,
) -> Result<(), String> {
    let auto = state.pool(pool.as_deref())?;
    let s = sanitize_auto_settings(settings);

    let old = {
        let mut cur = auto.settings.lock().await;
        let old = cur.clone();
        *cur = s.clone();
        old
    };
    if !s.enabled {
        auto.next.store(0, Ordering::SeqCst);
    } else if !old.enabled
        || old.interval_min != s.interval_min
        || auto.next.load(Ordering::SeqCst) == 0
    {
        let last = auto.last_run.load(Ordering::SeqCst);
        auto.next.store(
            first_run(s.interval_min, last, epoch_secs()),
            Ordering::SeqCst,
        );
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn run_auto_now(
    app: tauri::AppHandle,
    pool: Option<String>,
) -> Result<(), String> {
    let pool = pool.unwrap_or_else(|| MAIN_POOL.to_string());
    let auto = app.state::<AutoState>().pool(Some(&pool))?;
    tauri::async_runtime::spawn(async move {
        // Like a scheduled run, it moves the schedule on: a run due seconds
        // later would otherwise check the pool and post the webhooks again.
        let outcome = run_auto_cycle(&app, &pool).await;
        finish_cycle(&app, &pool, &auto, outcome).await;
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
    fn subscriptions_cannot_reach_into_the_local_network() {
        for line in [
            "127.0.0.1:7890",
            "socks5://192.168.1.1:80",
            "10.0.0.1:3128:user:pass",
            "localhost:8080",
            "http://router.local:3128",
            "[::1]:1080",
            "socks5://[fe80::1]:1080",
            "0.0.0.0:3128",
        ] {
            assert!(points_inside(line, "http"), "{line}");
        }
        for line in [
            "1.2.3.4:8080",
            "socks5://proxy.example.com:1080",
            "8.8.8.8:3128:u:p",
        ] {
            assert!(!points_inside(line, "http"), "{line}");
        }
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
        // b is over the max ping: a first miss, kept with its new latency.
        assert_eq!(urls(&m), vec!["http://a:80", "http://b:80", "http://c:80"]);
        assert_eq!(m.items[0].raw, "raw-a");
        assert_eq!(m.items[0].latency, Some(10));
        assert_eq!(m.items[0].country.as_deref(), Some("FR"));
        assert_eq!(m.items[1].latency, Some(999));
        assert_eq!(m.fails.get("http://b:80"), Some(&1));
        assert_eq!(m.items[2].raw, "raw-c");
        assert_eq!(m.avg, Some(10));
        assert_eq!(m.alive, 1);
        // The second slow cycle in a row drops it.
        let m = merge_auto_items(&results, &current, 100, &m.fails);
        assert_eq!(urls(&m), vec!["http://a:80", "http://c:80"]);

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
    fn a_cycle_where_most_of_the_pool_fails_is_suspicious() {
        // Most of the pool at once, at least three entries.
        assert!(looks_like_local_trouble(10, 6));
        assert!(looks_like_local_trouble(4, 3));
        assert!(looks_like_local_trouble(3, 3));
        // Half or less, or a small pool losing one or two.
        assert!(!looks_like_local_trouble(10, 5));
        assert!(!looks_like_local_trouble(2, 2));
        assert!(!looks_like_local_trouble(3, 2));
        assert!(!looks_like_local_trouble(0, 0));
    }

    #[test]
    fn only_the_first_suspicious_cycle_in_a_row_is_held_back() {
        assert!(apply_cycle(false, 0));
        // A one-off: the pool and the history stay as they were.
        assert!(!apply_cycle(true, 0));
        // It repeats: the proxies really are failing, so it applies, and
        // removal still needs the usual two misses in a row.
        assert!(apply_cycle(true, 1));
        assert!(apply_cycle(true, 5));
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

        // First miss: kept with its country, without a latency, so it sorts last.
        let first = merge_auto_items(&results, &current, 0, &HashMap::new());
        assert_eq!(urls(&first), vec!["http://a:80", "http://b:80"]);
        assert_eq!(first.items[0].latency, None);
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
    fn offline_or_busy_cycle_is_retried_soon() {
        assert_eq!(next_run(false, 30, AutoOutcome::Offline, 1000, 1), 0);
        assert_eq!(
            next_run(true, 30, AutoOutcome::Busy, 1000, 0),
            1000 + RETRY_SOON_SECS
        );
        assert_eq!(
            next_run(true, 30, AutoOutcome::Finished, 1000, 0),
            1000 + 1800
        );
        // Offline: a minute, then twice as long each time, up to the interval.
        let wait = |n| next_run(true, 30, AutoOutcome::Offline, 1000, n) - 1000;
        assert_eq!((wait(1), wait(2), wait(3)), (60, 120, 240));
        assert_eq!(wait(5), 960);
        assert_eq!(wait(6), 1800);
        assert_eq!(wait(40), 1800);
    }

    #[test]
    fn the_schedule_resumes_from_the_last_run() {
        // Never ran: one interval from now.
        assert_eq!(first_run(30, 0, 10_000), 10_000 + 1800);
        // Restarted ten minutes after a run: twenty more minutes.
        assert_eq!(first_run(30, 10_000 - 600, 10_000), 10_000 + 1200);
        // A daily schedule restarted a day later: soon, not a day later.
        assert_eq!(
            first_run(1440, 100_000 - 90_000, 100_000),
            100_000 + RETRY_SOON_SECS
        );
        assert_eq!(schedule_file_name(MAIN_POOL), "schedule.json");
        assert_eq!(schedule_file_name("p1"), "schedule-p1.json");
        let file: ScheduleFile =
            serde_json::from_str(r#"{"lastRun":5,"sent":{"https://h":["a","b"]}}"#).unwrap();
        assert_eq!(file.last_run, 5);
        assert!(file.fails.is_empty());
        assert_eq!(file.sent["https://h"], vec!["a", "b"]);
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
        // The login's hash, never the password.
        assert_eq!(cache[0].proxy, "http://1.2.3.4:8080#uf00c3c10");
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
    #[test]
    fn subscriptions_add_only_unknown_proxies() {
        let mut known: HashSet<String> = ["http://1.1.1.1:80".to_string()].into();
        let entries = vec![
            "1.1.1.1:80".to_string(),
            "2.2.2.2:8080".to_string(),
            "http://2.2.2.2:8080".to_string(),
            "garbage".to_string(),
            "3.3.3.3:3128".to_string(),
        ];
        let new = new_entries(entries.clone(), "http", &mut known, 10);
        assert_eq!(new, vec!["2.2.2.2:8080", "3.3.3.3:3128"]);
        assert!(known.contains("http://3.3.3.3:3128"));

        let mut known = HashSet::new();
        assert_eq!(new_entries(entries, "http", &mut known, 1).len(), 1);
    }

    #[test]
    fn alive_subscription_proxies_join_the_pool() {
        let current = HashMap::from([(
            "http://a:80".to_string(),
            ("raw-a".to_string(), Some(100), None),
        )]);
        let mut fresh_alive = alive_result("http://n:80", 50, "NL");
        fresh_alive.proxy = "n:80".to_string();
        let mut fresh_slow = alive_result("http://s:80", 900, "NL");
        fresh_slow.proxy = "s:80".to_string();
        let mut fresh_dead = dead("http://d:80");
        fresh_dead.proxy = "d:80".to_string();
        let results = vec![
            alive_result("http://a:80", 10, "DE"),
            fresh_alive,
            fresh_slow,
            fresh_dead,
        ];
        let fresh: HashSet<String> = ["n:80", "s:80", "d:80"].map(String::from).into();
        let mut m = merge_auto_items(&results, &current, 500, &HashMap::new());
        add_new_items(&mut m, &results, &fresh, 500);
        assert_eq!(urls(&m), vec!["http://a:80", "http://n:80"]);
        assert_eq!(m.items[1].raw, "n:80");
        assert_eq!(m.items[1].country.as_deref(), Some("NL"));
        assert_eq!(m.alive, 2);
        assert_eq!(m.avg, Some(30));
    }

    #[test]
    fn subscription_urls_are_sanitized() {
        let s = sanitize_auto_settings(AutoSettings {
            subscriptions: vec![
                " https://lists.example.com/a.txt ".to_string(),
                "ftp://x".to_string(),
                String::new(),
            ],
            webhook_format: "discord".to_string(),
            ..AutoSettings::default()
        });
        assert_eq!(s.subscriptions, vec!["https://lists.example.com/a.txt"]);
        assert_eq!(s.webhook_format, "discord");
    }
}
