use crate::commands::dispatch::DispatchState;
use crate::{epoch_secs, AutoState};
use std::sync::atomic::Ordering;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{Manager, State};
use tokio::sync::Mutex;

// ================= tray status =================

#[derive(Debug, Clone)]
struct TrayInfo {
    webhook_on: bool,
    webhook_url: String,
    lang: String,
}

impl Default for TrayInfo {
    fn default() -> Self {
        Self {
            webhook_on: false,
            webhook_url: String::new(),
            // Placeholder until the webview reports its language (saved
            // choice or system language) right after startup.
            lang: "ru".to_string(),
        }
    }
}

#[derive(Default)]
pub(crate) struct TrayState {
    info: Mutex<TrayInfo>,
    last_menu: Mutex<String>,
}

fn fmt_countdown(s: Option<u64>, ru: bool) -> String {
    match s {
        None => {
            if ru {
                "выкл".to_string()
            } else {
                "off".to_string()
            }
        }
        Some(x) if x >= 3600 => format!("{}h {:02}m", x / 3600, (x % 3600) / 60),
        Some(x) if x >= 60 => format!("{}m", x / 60),
        Some(_) => {
            if ru {
                "скоро".to_string()
            } else {
                "soon".to_string()
            }
        }
    }
}

/// Only the host is shown in the tray: webhook URLs may contain secrets.
fn url_host(u: &str) -> String {
    reqwest::Url::parse(u)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

pub(crate) async fn refresh_tray_menu(app: &tauri::AppHandle) {
    let disp: State<DispatchState> = app.state();
    let tray: State<TrayState> = app.state();
    let running = disp.inner.running.load(Ordering::SeqCst);
    let port = *disp.inner.port.lock().await;
    let info = tray.info.lock().await.clone();
    let auto: State<AutoState> = app.state();
    let now = epoch_secs();
    let remaining = |v: u64| {
        if v == 0 {
            None
        } else {
            Some(v.saturating_sub(now))
        }
    };
    let disp_next = remaining(auto.inner.disp_next.load(Ordering::SeqCst));
    let auto_next = remaining(auto.inner.auto_next.load(Ordering::SeqCst));
    let ru = info.lang == "ru";
    let dispatch_txt = if running {
        if ru {
            format!("Диспетчер: ВКЛ :{port}")
        } else {
            format!("Dispatcher: ON :{port}")
        }
    } else if ru {
        "Диспетчер: ВЫКЛ".to_string()
    } else {
        "Dispatcher: OFF".to_string()
    };
    let next_disp_txt = format!(
        "{} {}",
        if ru {
            "След. диспетчер:"
        } else {
            "Next dispatch:"
        },
        fmt_countdown(disp_next, ru)
    );
    let next_auto_txt = format!(
        "{} {}",
        if ru {
            "След. авто:"
        } else {
            "Next auto:"
        },
        fmt_countdown(auto_next, ru)
    );
    let sig = format!(
        "{dispatch_txt}|{next_disp_txt}|{next_auto_txt}|{}|{}|{}",
        info.webhook_on, info.webhook_url, info.lang
    );
    {
        let last = tray.last_menu.lock().await;
        if *last == sig {
            return;
        }
    }
    let build = || -> tauri::Result<()> {
        let show = MenuItem::with_id(
            app,
            "show",
            if ru { "Показать" } else { "Show" },
            true,
            None::<&str>,
        )?;
        let hide = MenuItem::with_id(
            app,
            "hide",
            if ru { "Скрыть" } else { "Hide" },
            true,
            None::<&str>,
        )?;
        let dispatch =
            MenuItem::with_id(app, "dispatch_toggle", &dispatch_txt, true, None::<&str>)?;
        let next_disp = MenuItem::with_id(app, "next_disp", &next_disp_txt, false, None::<&str>)?;
        let next_auto = MenuItem::with_id(app, "next_auto", &next_auto_txt, false, None::<&str>)?;
        let webhook_txt = if info.webhook_on && !info.webhook_url.trim().is_empty() {
            let label = if ru {
                "Вебхук: вкл"
            } else {
                "Webhook: on"
            };
            let host = url_host(info.webhook_url.trim());
            if host.is_empty() {
                label.to_string()
            } else {
                format!("{label} {host}")
            }
        } else if ru {
            "Вебхук: выкл".to_string()
        } else {
            "Webhook: off".to_string()
        };
        let webhook = MenuItem::with_id(app, "webhook_info", &webhook_txt, false, None::<&str>)?;
        let quit = MenuItem::with_id(
            app,
            "quit",
            if ru { "Выход" } else { "Quit" },
            true,
            None::<&str>,
        )?;
        let sep1 = PredefinedMenuItem::separator(app)?;
        let sep2 = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(
            app,
            &[
                &show, &hide, &sep1, &dispatch, &next_disp, &next_auto, &webhook, &sep2, &quit,
            ],
        )?;
        if let Some(tray_icon) = app.tray_by_id("main") {
            tray_icon.set_menu(Some(menu))?;
        }
        Ok(())
    };
    if build().is_ok() {
        *tray.last_menu.lock().await = sig;
    }
}

#[tauri::command]
pub(crate) async fn update_tray_status(
    app: tauri::AppHandle,
    tray: State<'_, TrayState>,
    webhook_on: bool,
    webhook_url: String,
    lang: Option<String>,
) -> Result<(), String> {
    let lang = match lang.as_deref() {
        Some("ru") => "ru",
        _ => "en",
    };
    *tray.info.lock().await = TrayInfo {
        webhook_on,
        webhook_url,
        lang: lang.to_string(),
    };
    refresh_tray_menu(&app).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_countdown_is_localized() {
        assert_eq!(fmt_countdown(None, true), "выкл");
        assert_eq!(fmt_countdown(None, false), "off");
        assert_eq!(fmt_countdown(Some(59), true), "скоро");
        assert_eq!(fmt_countdown(Some(59), false), "soon");
        assert_eq!(fmt_countdown(Some(60), true), "1m");
        assert_eq!(fmt_countdown(Some(3660), false), "1h 01m");
    }
}
