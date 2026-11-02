//! Named pools: each has its own proxies, schedule and dispatcher. The
//! registry (`pools.json`) holds their ids and names; the main pool is always
//! present and cannot be deleted.

use crate::commands::auto::AutoState;
use crate::commands::dispatch::{
    delete_pool_file, load_pool, stop_proxy_inner, write_private_file, DispatchState, MAIN_POOL,
};
use crate::commands::prefs::forget_pool;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tauri::{AppHandle, Manager, State};

const POOLS_FILE: &str = "pools.json";
/// The main pool plus nine named ones.
pub(crate) const MAX_POOLS: usize = 10;
const NAME_MAX: usize = 40;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PoolMeta {
    pub(crate) id: String,
    /// Empty for the main pool: the UI shows its own label.
    pub(crate) name: String,
}

#[derive(Default)]
pub(crate) struct PoolsState {
    list: Mutex<Vec<PoolMeta>>,
}

impl PoolsState {
    pub(crate) fn snapshot(&self) -> Vec<PoolMeta> {
        lock(&self.list).clone()
    }
}

fn lock(m: &Mutex<Vec<PoolMeta>>) -> MutexGuard<'_, Vec<PoolMeta>> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ids are short lowercase letters and digits: they appear in file names.
pub(crate) fn valid_pool_id(id: &str) -> bool {
    (1..=16).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

pub(crate) fn clean_name(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(NAME_MAX)
        .collect()
}

/// The registry as stored, repaired: the main pool first, valid unique ids,
/// at most [`MAX_POOLS`] entries.
pub(crate) fn normalize(list: Vec<PoolMeta>) -> Vec<PoolMeta> {
    let main_name = list
        .iter()
        .find(|p| p.id == MAIN_POOL)
        .map(|p| clean_name(&p.name))
        .unwrap_or_default();
    let mut out = vec![PoolMeta {
        id: MAIN_POOL.to_string(),
        name: main_name,
    }];
    for p in list {
        if out.len() >= MAX_POOLS {
            break;
        }
        if p.id == MAIN_POOL || !valid_pool_id(&p.id) || out.iter().any(|o| o.id == p.id) {
            continue;
        }
        out.push(PoolMeta {
            name: clean_name(&p.name),
            id: p.id,
        });
    }
    out
}

fn pools_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(POOLS_FILE))
}

pub(crate) fn read_pools_file(path: &Path) -> Vec<PoolMeta> {
    let list = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    normalize(list)
}

fn save(app: &AppHandle, list: &[PoolMeta]) -> Result<(), String> {
    let Some(path) = pools_path(app) else {
        return Ok(());
    };
    let json = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    write_private_file(&path, &json).map_err(|e| e.to_string())
}

/// Read the registry and create the runtime of every named pool, with its
/// saved proxies; called once during setup.
pub(crate) async fn load_pools(app: &AppHandle) {
    let list = pools_path(app)
        .map(|p| read_pools_file(&p))
        .unwrap_or_else(|| normalize(Vec::new()));
    for meta in list.iter().filter(|p| p.id != MAIN_POOL) {
        add_runtime(app, &meta.id).await;
    }
    *lock(&app.state::<PoolsState>().list) = list;
}

async fn add_runtime(app: &AppHandle, id: &str) {
    let inner = app.state::<DispatchState>().add(id);
    app.state::<AutoState>().add(id);
    load_pool(app, &inner).await;
}

fn new_id(taken: &[PoolMeta]) -> String {
    loop {
        let id = format!("p{}", &crate::judge_sig::new_nonce()[..6]);
        if !taken.iter().any(|p| p.id == id) {
            return id;
        }
    }
}

#[tauri::command]
pub(crate) fn list_pools(state: State<'_, PoolsState>) -> Vec<PoolMeta> {
    lock(&state.list).clone()
}

#[tauri::command]
pub(crate) async fn create_pool(app: AppHandle, name: String) -> Result<PoolMeta, String> {
    let meta = {
        let state = app.state::<PoolsState>();
        let mut list = lock(&state.list);
        if list.len() >= MAX_POOLS {
            return Err(format!("at most {MAX_POOLS} pools"));
        }
        let meta = PoolMeta {
            id: new_id(&list),
            name: clean_name(&name),
        };
        list.push(meta.clone());
        save(&app, &list)?;
        meta
    };
    add_runtime(&app, &meta.id).await;
    Ok(meta)
}

#[tauri::command]
pub(crate) fn rename_pool(app: AppHandle, id: String, name: String) -> Result<PoolMeta, String> {
    let state = app.state::<PoolsState>();
    let mut list = lock(&state.list);
    let meta = list
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| "unknown pool".to_string())?;
    meta.name = clean_name(&name);
    let out = meta.clone();
    save(&app, &list)?;
    Ok(out)
}

/// Delete a named pool: its dispatcher stops, its schedule ends and its
/// saved proxies are removed.
#[tauri::command]
pub(crate) async fn delete_pool(app: AppHandle, id: String) -> Result<(), String> {
    if id == MAIN_POOL {
        return Err("the main pool cannot be deleted".to_string());
    }
    {
        let state = app.state::<PoolsState>();
        let mut list = lock(&state.list);
        let before = list.len();
        list.retain(|p| p.id != id);
        if list.len() == before {
            return Err("unknown pool".to_string());
        }
        save(&app, &list)?;
    }
    if let Some(inner) = app.state::<DispatchState>().remove(&id) {
        stop_proxy_inner(&inner).await;
    }
    app.state::<AutoState>().remove(&id);
    delete_pool_file(&app, &id);
    forget_pool(&app, &id).await;
    Ok(())
}

/// Merge the named pools of a settings file into the registry: known pools
/// take the file's names, new ones are added while there is room. The main
/// pool keeps its name, since settings files leave it out. Returns the ids
/// of the added pools.
pub(crate) fn merge_pools(list: &mut Vec<PoolMeta>, incoming: Vec<PoolMeta>) -> Vec<String> {
    let mut added = Vec::new();
    for p in normalize(incoming)
        .into_iter()
        .filter(|p| p.id != MAIN_POOL)
    {
        if let Some(i) = list.iter().position(|o| o.id == p.id) {
            list[i].name = p.name;
        } else if list.len() < MAX_POOLS {
            added.push(p.id.clone());
            list.push(p);
        }
    }
    added
}

/// Settings import: add the pools a settings file names and take their
/// names; pools missing from the file are kept with their proxies.
#[tauri::command]
pub(crate) async fn restore_pools(
    app: AppHandle,
    pools: Vec<PoolMeta>,
) -> Result<Vec<PoolMeta>, String> {
    let (list, added) = {
        let state = app.state::<PoolsState>();
        let mut list = lock(&state.list);
        let added = merge_pools(&mut list, pools);
        save(&app, &list)?;
        (list.clone(), added)
    };
    for id in added {
        add_runtime(&app, &id).await;
    }
    Ok(list)
}
