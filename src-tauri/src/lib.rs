use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, State,
};
use tokio::sync::{Mutex, Semaphore};

mod http;
mod netguard;
mod transport;
use http::*;
mod proxy;
mod tls;
use proxy::*;

mod commands;
use commands::app::{
    app_version, check_app_update, hide_to_tray, install_app_update, install_kind,
};
use commands::auto::{auto_scheduler_loop, run_auto_now, set_auto_config, AutoState};
use commands::check::{cancel_check, check_busy, check_direct, check_proxies, CheckState};
use commands::dispatch::{
    get_dispatch_pool, local_proxy_status, set_dispatch_pool, start_local_proxy, start_proxy_inner,
    stop_local_proxy, stop_proxy_inner, DispatchState,
};
use commands::export::export_text_file;
use commands::import::fetch_url_text;
use commands::tray::{refresh_tray_menu, update_tray_status, TrayState};
use commands::webhook::{send_webhook, send_webhooks};

// ================= results =================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyResult {
    pub proxy: String,
    /// Normalized URL actually used for the check (keeps the real scheme).
    pub url: String,
    pub proto: String,
    pub alive: bool,
    pub latency_ms: Option<u64>,
    pub jitter_ms: Option<u64>,
    pub success_rate: f32,
    pub attempts: u32,
    pub ip: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub city: Option<String>,
    pub isp: Option<String>,
    pub org: Option<String>,
    pub asn: Option<String>,
    pub ip_type: Option<String>,
    pub anonymity: Option<String>,
    pub tamper: Option<String>,
    pub tls: Option<String>,
    pub tls_info: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IpApi {
    status: Option<String>,
    query: Option<String>,
    country: Option<String>,
    #[serde(rename = "countryCode")]
    country_code: Option<String>,
    city: Option<String>,
    isp: Option<String>,
    org: Option<String>,
    #[serde(rename = "as")]
    asn: Option<String>,
    mobile: Option<bool>,
    proxy: Option<bool>,
    hosting: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct HttpbinIp {
    origin: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HttpbinHeaders {
    headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Default)]
struct JudgeGeo {
    country: Option<String>,
    country_code: Option<String>,
    city: Option<String>,
    asn: Option<u64>,
    aso: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct JudgeBody {
    ip: Option<String>,
    anonymity: Option<String>,
    ip_type: Option<String>,
    geo: Option<JudgeGeo>,
    content_sha256: Option<String>,
}

fn try_parse_judge_body(res: &mut ProxyResult, body: &str) -> Option<String> {
    let jb: JudgeBody = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return None,
    };
    if let Some(ip) = jb.ip {
        let t = ip.trim().to_string();
        if !t.is_empty() {
            res.ip = Some(t);
        }
    }
    res.anonymity = jb.anonymity;
    res.ip_type = jb.ip_type;
    if let Some(g) = jb.geo {
        res.country = g.country;
        res.country_code = g.country_code;
        res.city = g.city;
        res.asn = g.asn.map(|n| n.to_string());
        res.org = g.aso;
    }
    jb.content_sha256
}

#[derive(Debug, Clone, Deserialize)]
struct GeoCacheEntry {
    proxy: String,
    ip: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
    city: Option<String>,
    isp: Option<String>,
    org: Option<String>,
    asn: Option<String>,
    ip_type: Option<String>,
}

/// Direct judge endpoint used for the content hash baseline. For a custom
/// path (e.g. `https://host/svc`) the URL is used as-is: the same endpoint
/// is what the checked proxy requests.
fn judge_direct_url(test_url: &str) -> Option<String> {
    let t = test_url.trim_end_matches('/');
    if t.is_empty() {
        return None;
    }
    if t.ends_with("/judge") {
        return Some(t.to_string());
    }
    if let Some(base) = t.strip_suffix("/generate_204") {
        if !base.is_empty() {
            return Some(format!("{base}/judge"));
        }
    }
    Some(t.to_string())
}

async fn fetch_direct_judge_sha(judge_url: &str, timeout: Duration) -> Option<String> {
    let client = build_client(None, timeout, true).ok()?;
    let r = client.get(judge_url).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let body = read_body_capped(r, BODY_LIMIT_PAGE).await.ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    v.get("content_sha256")?.as_str().map(|s| s.to_string())
}

struct Baseline {
    direct_ip: Option<String>,
    example_len: Option<usize>,
    example_hash: Option<u64>,
    example_cert_fp: Option<String>,
    judge_content_sha: Option<String>,
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

/// The free ip-api tier is heavily rate-limited. Cap concurrent geo lookups
/// instead of firing one request per checked proxy at 500-way concurrency.
const GEO_CONCURRENCY: usize = 8;
const GEO_COOLDOWN: Duration = Duration::from_secs(60);
static GEO_SEM: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();
static GEO_COOLDOWN_UNTIL: Mutex<Option<Instant>> = Mutex::const_new(None);

/// Deep checks open one extra connection per stage. Without a cap, a
/// 500-way check can spawn ~2500 sockets at once. These limits are per stage
/// and skip (rather than queue) when saturated, so the per-proxy budget is
/// not consumed by waiting.
const DEEP_CONCURRENCY: usize = 64;
static ANON_SEM: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();
static TAMPER_SEM: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();
static TLS_SEM: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();

fn geo_semaphore() -> &'static Semaphore {
    GEO_SEM.get_or_init(|| Semaphore::new(GEO_CONCURRENCY))
}

fn deep_semaphore(slot: &'static std::sync::OnceLock<Semaphore>) -> &'static Semaphore {
    slot.get_or_init(|| Semaphore::new(DEEP_CONCURRENCY))
}

fn baseline_cache() -> &'static Mutex<BaselineCache> {
    BASELINE_CACHE.get_or_init(|| Mutex::const_new(BaselineCache::default()))
}

fn cache_fresh(ts: Instant, ttl: Duration) -> bool {
    ts.elapsed() < ttl
}

fn body_hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[derive(Debug, Clone, Serialize)]
struct WebhookResult {
    index: usize,
    ok: bool,
    message: String,
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Keep a single instance: two processes would share localStorage and
        // could send duplicate webhooks / run duplicate auto-checks.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(DispatchState::default())
        .manage(CheckState::default())
        .manage(TrayState::default())
        .manage(AutoState::default())
        .invoke_handler(tauri::generate_handler![
            check_proxies,
            check_busy,
            cancel_check,
            check_direct,
            export_text_file,
            install_kind,
            fetch_url_text,
            set_dispatch_pool,
            get_dispatch_pool,
            start_local_proxy,
            stop_local_proxy,
            local_proxy_status,
            send_webhook,
            send_webhooks,
            set_auto_config,
            run_auto_now,
            hide_to_tray,
            app_version,
            check_app_update,
            install_app_update,
            update_tray_status
        ])
        .setup(|app| {
            if let Some(icon) = app.default_window_icon().cloned() {
                TrayIconBuilder::with_id("main")
                    .icon(icon)
                    .tooltip("ProxPulse")
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "hide" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.hide();
                            }
                        }
                        "dispatch_toggle" => {
                            let ah = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let disp: State<DispatchState> = ah.state();
                                if disp.inner.running.load(Ordering::SeqCst) {
                                    stop_proxy_inner(&disp.inner).await;
                                } else {
                                    let port = *disp.inner.port.lock().await;
                                    let mode = disp.inner.mode.lock().await.clone();
                                    let listener = disp.inner.listener.lock().await.clone();
                                    if start_proxy_inner(&disp.inner, port, mode, listener)
                                        .await
                                        .is_err()
                                    {
                                        *disp.inner.last_error.lock().await = Some(
                                            "toggle failed: pool empty or ports busy".to_string(),
                                        );
                                    }
                                }
                                refresh_tray_menu(&ah).await;
                            });
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            if let Some(w) = app.get_webview_window("main") {
                                if w.is_visible().unwrap_or(true) {
                                    let _ = w.hide();
                                } else {
                                    let _ = w.show();
                                    let _ = w.set_focus();
                                }
                            }
                        }
                    })
                    .build(app)?;
                // The menu is built from the same localized code as later
                // refreshes instead of hardcoding English labels here.
                tauri::async_runtime::block_on(refresh_tray_menu(app.handle()));
            }
            // Scheduling and webhooks run in Rust, so hiding the window or
            // a throttled WebView does not stop scheduled checks.
            let auto_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                auto_scheduler_loop(auto_handle).await;
            });
            let tray_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    refresh_tray_menu(&tray_handle).await;
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests;
