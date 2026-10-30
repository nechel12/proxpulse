use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

#[cfg(windows)]
fn detect_install_kind() -> String {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    fn uninstall_kind(root: RegKey, path: &str) -> Option<&'static str> {
        let key = root.open_subkey_with_flags(path, KEY_READ).ok()?;
        for name in key.enum_keys().flatten() {
            let Ok(sub) = key.open_subkey(&name) else {
                continue;
            };
            let Ok(display) = sub.get_value::<String, _>("DisplayName") else {
                continue;
            };
            if !display.to_ascii_lowercase().contains("proxpulse") {
                continue;
            }
            let uninstall: String = sub.get_value("UninstallString").unwrap_or_default();
            let low = uninstall.to_ascii_lowercase();
            if low.contains("msiexec") {
                return Some("msi");
            }
            if low.contains("uninstall") {
                return Some("nsis");
            }
        }
        None
    }

    let path = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    if let Some(kind) = uninstall_kind(RegKey::predef(HKEY_LOCAL_MACHINE), path)
        .or_else(|| uninstall_kind(RegKey::predef(HKEY_CURRENT_USER), path))
    {
        return kind.to_string();
    }
    if let Ok(exe) = std::env::current_exe() {
        let exe = exe.to_string_lossy().to_ascii_lowercase();
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let base = format!("{}\\programs", local.to_ascii_lowercase());
            if exe.starts_with(&base) {
                return "nsis".to_string();
            }
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Ok(base) = std::env::var(var) {
                if exe.starts_with(&base.to_ascii_lowercase()) {
                    return "msi".to_string();
                }
            }
        }
    }
    "portable".to_string()
}

#[cfg(target_os = "linux")]
fn detect_install_kind() -> String {
    if std::env::var_os("APPIMAGE").is_some() {
        "auto".to_string()
    } else {
        "package".to_string()
    }
}

#[cfg(all(not(windows), not(target_os = "linux")))]
fn detect_install_kind() -> String {
    "auto".to_string()
}

#[tauri::command]
pub(crate) fn install_kind() -> String {
    detect_install_kind()
}

#[tauri::command]
pub(crate) fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[derive(Serialize)]
pub(crate) struct UpdateInfo {
    available: bool,
    version: Option<String>,
    current: String,
    notes: Option<String>,
}

#[tauri::command]
pub(crate) async fn check_app_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let updater = app.updater_builder().build().map_err(|e| e.to_string())?;
    match updater.check().await.map_err(|e| e.to_string())? {
        Some(u) => Ok(UpdateInfo {
            available: true,
            version: Some(u.version.clone()),
            current,
            notes: u.body.clone(),
        }),
        None => Ok(UpdateInfo {
            available: false,
            version: None,
            current,
            notes: None,
        }),
    }
}

#[tauri::command]
pub(crate) async fn install_app_update(
    app: AppHandle,
    expected_version: String,
) -> Result<(), String> {
    let updater = app.updater_builder().build().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("no update".to_string());
    };
    // The update is re-fetched here, so make sure the user is still installing
    // the version they confirmed in the UI.
    if update.version != expected_version {
        return Err("update version changed; check again".to_string());
    }
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn hide_to_tray(app: AppHandle) -> Result<(), String> {
    if app.tray_by_id("main").is_none() {
        return Err("tray is unavailable".to_string());
    }
    app.get_webview_window("main")
        .ok_or_else(|| "no window".to_string())
        .and_then(|w| w.hide().map_err(|e| e.to_string()))
}
