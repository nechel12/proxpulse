//! App preferences the backend needs before the window loads: closing to the
//! tray, starting hidden from autostart, and starting the dispatcher.

use crate::commands::dispatch::{
    normalize_listener, normalize_mode, start_proxy_inner, write_private_file, DispatchOptions,
    DispatchState, MAIN_POOL,
};
use crate::commands::pool_api::PoolApiPrefs;
use crate::commands::tray::refresh_tray_menu;
use crate::http::truncate_utf8;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;

const PREFS_FILE: &str = "prefs.json";
const PREFS_FILE_MAX: u64 = 64 * 1024;

/// Passed by the autostart entry; the window then starts hidden in the tray
/// when `start_minimized` is on.
pub(crate) const MINIMIZED_ARG: &str = "--minimized";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct DispatcherPrefs {
    pub(crate) port: u16,
    pub(crate) mode: String,
    pub(crate) listener: String,
    pub(crate) options: DispatchOptions,
    /// The dispatcher was started at least once. A named pool's dispatcher
    /// starts at launch only then; the main pool's always does.
    pub(crate) started: bool,
}

impl Default for DispatcherPrefs {
    fn default() -> Self {
        Self {
            port: 1080,
            mode: "round-robin".to_string(),
            listener: "both".to_string(),
            options: DispatchOptions::default(),
            started: false,
        }
    }
}

impl DispatcherPrefs {
    /// Reduce stored or imported values to the ones the dispatcher accepts.
    pub(crate) fn sanitized(self) -> Self {
        Self {
            port: if self.port == 0 { 1080 } else { self.port },
            mode: normalize_mode(&self.mode),
            listener: normalize_listener(&self.listener),
            options: self.options.sanitized(),
            started: self.started,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct AppPrefs {
    pub(crate) close_to_tray: bool,
    pub(crate) start_minimized: bool,
    pub(crate) start_dispatcher: bool,
    /// The last started dispatcher configuration.
    pub(crate) dispatcher: DispatcherPrefs,
    /// The same for every named pool, by pool id.
    pub(crate) pool_dispatchers: HashMap<String, DispatcherPrefs>,
    pub(crate) pool_api: PoolApiPrefs,
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            close_to_tray: false,
            start_minimized: true,
            start_dispatcher: false,
            dispatcher: DispatcherPrefs::default(),
            pool_dispatchers: HashMap::new(),
            pool_api: PoolApiPrefs::default(),
        }
    }
}

/// A partial update from the settings tab or a settings import.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct AppPrefsUpdate {
    pub(crate) close_to_tray: Option<bool>,
    pub(crate) start_minimized: Option<bool>,
    pub(crate) start_dispatcher: Option<bool>,
    pub(crate) dispatcher: Option<DispatcherPrefs>,
    /// Named pools' dispatchers; replaces the whole map when given.
    pub(crate) pool_dispatchers: Option<HashMap<String, DispatcherPrefs>>,
    pub(crate) pool_api: Option<PoolApiPrefs>,
}

impl AppPrefs {
    pub(crate) fn apply(&mut self, update: AppPrefsUpdate) {
        if let Some(v) = update.close_to_tray {
            self.close_to_tray = v;
        }
        if let Some(v) = update.start_minimized {
            self.start_minimized = v;
        }
        if let Some(v) = update.start_dispatcher {
            self.start_dispatcher = v;
        }
        if let Some(d) = update.dispatcher {
            self.dispatcher = d.sanitized();
        }
        if let Some(map) = update.pool_dispatchers {
            self.pool_dispatchers = map
                .into_iter()
                .filter(|(id, _)| crate::commands::pools::valid_pool_id(id) && id != MAIN_POOL)
                .map(|(id, d)| (id, d.sanitized()))
                .collect();
        }
        if let Some(api) = update.pool_api {
            self.pool_api = api.sanitized();
        }
    }
}

#[derive(Default)]
pub(crate) struct PrefsState {
    prefs: Mutex<AppPrefs>,
    /// Serializes file writes so the last write holds the latest snapshot.
    save_lock: Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl PrefsState {
    pub(crate) fn get(&self) -> AppPrefs {
        lock(&self.prefs).clone()
    }
}

fn prefs_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(PREFS_FILE))
}

pub(crate) fn read_prefs_file(path: &Path) -> AppPrefs {
    let Ok(meta) = std::fs::metadata(path) else {
        return AppPrefs::default();
    };
    if meta.len() > PREFS_FILE_MAX {
        return AppPrefs::default();
    }
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AppPrefs>(&bytes).ok())
        .map(|mut p| {
            p.dispatcher = p.dispatcher.sanitized();
            p.pool_dispatchers = std::mem::take(&mut p.pool_dispatchers)
                .into_iter()
                .map(|(id, d)| (id, d.sanitized()))
                .collect();
            p.pool_api = std::mem::take(&mut p.pool_api).sanitized();
            p
        })
        .unwrap_or_default()
}

pub(crate) fn write_prefs_file(path: &Path, prefs: &AppPrefs) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(prefs).map_err(std::io::Error::other)?;
    write_private_file(path, &json)
}

/// Read `prefs.json` into the managed state; called once during setup.
pub(crate) fn load_prefs(app: &AppHandle) -> AppPrefs {
    let prefs = prefs_path(app)
        .map(|p| read_prefs_file(&p))
        .unwrap_or_default();
    *lock(&app.state::<PrefsState>().prefs) = prefs.clone();
    prefs
}

pub(crate) async fn update_prefs(
    app: &AppHandle,
    update: AppPrefsUpdate,
) -> Result<AppPrefs, String> {
    let prefs = {
        let state = app.state::<PrefsState>();
        let mut prefs = lock(&state.prefs);
        prefs.apply(update);
        prefs.clone()
    };
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<PrefsState>();
        let _guard = lock(&state.save_lock);
        let snapshot = state.get();
        match prefs_path(&handle) {
            Some(path) => write_prefs_file(&path, &snapshot),
            None => Ok(()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(prefs)
}

/// The remembered dispatcher of a pool; defaults for one never started.
pub(crate) fn pool_dispatcher(prefs: &AppPrefs, pool: &str) -> DispatcherPrefs {
    if pool == MAIN_POOL {
        prefs.dispatcher.clone()
    } else {
        prefs
            .pool_dispatchers
            .get(pool)
            .cloned()
            .unwrap_or_default()
    }
}

pub(crate) async fn remember_dispatcher(app: &AppHandle, pool: &str, dispatcher: DispatcherPrefs) {
    let update = if pool == MAIN_POOL {
        AppPrefsUpdate {
            dispatcher: Some(dispatcher),
            ..AppPrefsUpdate::default()
        }
    } else {
        let mut map = app.state::<PrefsState>().get().pool_dispatchers;
        map.insert(pool.to_string(), dispatcher);
        AppPrefsUpdate {
            pool_dispatchers: Some(map),
            ..AppPrefsUpdate::default()
        }
    };
    let _ = update_prefs(app, update).await;
}

pub(crate) async fn remember_dispatch_options(
    app: &AppHandle,
    pool: &str,
    options: DispatchOptions,
) {
    let mut dispatcher = pool_dispatcher(&app.state::<PrefsState>().get(), pool);
    dispatcher.options = options;
    remember_dispatcher(app, pool, dispatcher).await;
}

/// Drop what a deleted pool left in the preferences.
pub(crate) async fn forget_pool(app: &AppHandle, pool: &str) {
    let mut map = app.state::<PrefsState>().get().pool_dispatchers;
    if map.remove(pool).is_some() {
        let update = AppPrefsUpdate {
            pool_dispatchers: Some(map),
            ..AppPrefsUpdate::default()
        };
        let _ = update_prefs(app, update).await;
    }
}

/// Closing the main window hides it instead when the user chose so and the
/// tray icon exists to bring it back.
pub(crate) fn close_to_tray(app: &AppHandle) -> bool {
    app.state::<PrefsState>().get().close_to_tray && app.tray_by_id("main").is_some()
}

/// Whether this launch came from autostart and should stay in the tray.
pub(crate) fn start_hidden(app: &AppHandle, prefs: &AppPrefs) -> bool {
    prefs.start_minimized
        && std::env::args().any(|a| a == MINIMIZED_ARG)
        && app.tray_by_id("main").is_some()
}

/// Whether the pool's dispatcher starts at launch when the user enabled it:
/// the main pool's always, a named pool's once it was started. Editing the
/// options of a pool that never ran keeps them without starting it.
pub(crate) fn starts_at_launch(prefs: &AppPrefs, pool: &str) -> bool {
    pool == MAIN_POOL || pool_dispatcher(prefs, pool).started
}

/// Restore every pool's dispatcher configuration and, when the user
/// enabled it, start the dispatchers of the main pool and of every named
/// pool that was started before.
pub(crate) async fn start_saved_dispatcher(app: AppHandle) {
    let prefs = app.state::<PrefsState>().get();
    let disp: State<DispatchState> = app.state();
    for inner in disp.all() {
        let d = pool_dispatcher(&prefs, &inner.id);
        *inner.port.lock().await = d.port;
        *inner.mode.lock().await = d.mode.clone();
        *inner.listener.lock().await = d.listener.clone();
        *inner.options.lock().await = d.options.clone();
        if !prefs.start_dispatcher
            || !starts_at_launch(&prefs, &inner.id)
            || inner.pool.lock().await.is_empty()
        {
            continue;
        }
        if let Err(e) = start_proxy_inner(&inner, d.port, d.mode, d.listener).await {
            *inner.last_error.lock().await = Some(format!("start at launch: {e}"));
        }
    }
    refresh_tray_menu(&app).await;
}

#[tauri::command]
pub(crate) fn get_app_prefs(state: State<'_, PrefsState>) -> AppPrefs {
    state.get()
}

#[tauri::command]
pub(crate) async fn set_app_prefs(
    app: AppHandle,
    prefs: AppPrefsUpdate,
) -> Result<AppPrefs, String> {
    update_prefs(&app, prefs).await
}

#[tauri::command]
pub(crate) fn autostart_enabled(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    if manager.is_enabled().map_err(|e| e.to_string())? != enabled {
        if enabled {
            manager.enable()
        } else {
            manager.disable()
        }
        .map_err(|e| e.to_string())?;
    }
    manager.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn notify(app: AppHandle, title: String, body: String) -> Result<(), String> {
    let (mut title, mut body) = (title, body);
    truncate_utf8(&mut title, 120);
    truncate_utf8(&mut body, 400);
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}
