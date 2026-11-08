//! Per-proxy history across checks: how often a proxy was alive, when it was
//! last seen alive and its typical ping. Kept in `history.json` keyed by the
//! proxy URL without credentials.

use crate::commands::dispatch::write_private_file;
use crate::{epoch_secs, strip_credentials, ProxyResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

const HISTORY_FILE: &str = "history.json";
const HISTORY_FILE_MAX: u64 = 32 * 1024 * 1024;
/// Entries kept; the ones checked longest ago are dropped first.
pub(crate) const HISTORY_MAX: usize = 50_000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct HistEntry {
    pub(crate) checks: u32,
    pub(crate) alive: u32,
    /// Unix seconds; 0 = never.
    pub(crate) last_check: u64,
    pub(crate) last_alive: u64,
    /// Smoothed ping of the alive checks.
    pub(crate) avg_ms: Option<u64>,
}

/// What the UI shows for a proxy.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistSummary {
    checks: u32,
    alive: u32,
    /// Share of alive checks, 0..1.
    uptime: f64,
    /// Uptime that trusts few checks less (Laplace smoothing), for sorting.
    score: f64,
    last_alive: Option<u64>,
    last_check: u64,
    avg_ms: Option<u64>,
}

impl From<&HistEntry> for HistSummary {
    fn from(e: &HistEntry) -> Self {
        let checks = e.checks.max(1) as f64;
        Self {
            checks: e.checks,
            alive: e.alive,
            uptime: (e.alive as f64 / checks * 1000.0).round() / 1000.0,
            score: ((e.alive as f64 + 1.0) / (e.checks as f64 + 2.0) * 1000.0).round() / 1000.0,
            last_alive: (e.last_alive > 0).then_some(e.last_alive),
            last_check: e.last_check,
            avg_ms: e.avg_ms,
        }
    }
}

#[derive(Default)]
pub(crate) struct HistoryState {
    pub(crate) map: Mutex<HashMap<String, HistEntry>>,
    save_lock: Mutex<()>,
}

pub(crate) fn history_key(url: &str) -> String {
    strip_credentials(url)
}

/// Count every finished check; stopped checks and lines that are not proxies
/// are not results.
pub(crate) fn record(map: &mut HashMap<String, HistEntry>, results: &[ProxyResult], now: u64) {
    for r in results {
        if r.error.as_deref() == Some("cancelled") || r.proto == "BAD" {
            continue;
        }
        // Only normalized URLs: a raw line such as `host:port:user:pass`
        // would split a proxy's history and put its password into the key.
        if !r.url.contains("://") {
            continue;
        }
        let e = map.entry(history_key(&r.url)).or_default();
        e.checks = e.checks.saturating_add(1);
        e.last_check = now;
        if r.alive {
            e.alive = e.alive.saturating_add(1);
            e.last_alive = now;
            if let Some(ms) = r.latency_ms {
                // Recent pings weigh more than old ones.
                e.avg_ms = Some(e.avg_ms.map_or(ms, |a| (a * 3 + ms) / 4));
            }
        }
    }
    if map.len() > HISTORY_MAX {
        let mut by_age: Vec<(u64, String)> =
            map.iter().map(|(k, e)| (e.last_check, k.clone())).collect();
        by_age.sort();
        for (_, k) in by_age.into_iter().take(map.len() - HISTORY_MAX) {
            map.remove(&k);
        }
    }
}

fn history_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(HISTORY_FILE))
}

pub(crate) fn read_history_file(path: &Path) -> HashMap<String, HistEntry> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() <= HISTORY_FILE_MAX => {}
        _ => return HashMap::new(),
    }
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(crate) async fn load_history(app: &AppHandle) {
    let Some(path) = history_path(app) else {
        return;
    };
    let map = tauri::async_runtime::spawn_blocking(move || read_history_file(&path))
        .await
        .unwrap_or_default();
    *app.state::<HistoryState>().map.lock().await = map;
}

async fn save_history(app: &AppHandle) {
    let Some(path) = history_path(app) else {
        return;
    };
    let state = app.state::<HistoryState>();
    let _guard = state.save_lock.lock().await;
    let Ok(json) = serde_json::to_vec(&*state.map.lock().await) else {
        return;
    };
    let _ = tauri::async_runtime::spawn_blocking(move || write_private_file(&path, &json)).await;
}

/// Record a finished check and persist the history.
pub(crate) async fn record_results(app: &AppHandle, results: &[ProxyResult]) {
    {
        let state = app.state::<HistoryState>();
        let mut map = state.map.lock().await;
        record(&mut map, results, epoch_secs());
    }
    save_history(app).await;
}

/// History of the given proxies, keyed by the URLs as passed.
#[tauri::command]
pub(crate) async fn proxy_history(
    urls: Vec<String>,
    state: State<'_, HistoryState>,
) -> Result<HashMap<String, HistSummary>, String> {
    let map = state.map.lock().await;
    Ok(urls
        .into_iter()
        .filter_map(|u| {
            let e = map.get(&history_key(&u))?;
            Some((u, HistSummary::from(e)))
        })
        .collect())
}

#[tauri::command]
pub(crate) async fn history_size(state: State<'_, HistoryState>) -> Result<usize, String> {
    Ok(state.map.lock().await.len())
}

#[tauri::command]
pub(crate) async fn clear_history(app: AppHandle) -> Result<(), String> {
    app.state::<HistoryState>().map.lock().await.clear();
    save_history(&app).await;
    Ok(())
}
