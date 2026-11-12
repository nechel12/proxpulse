use crate::commands::dispatch::{DispatchOptions, DispatchState, MAIN_POOL};
use crate::commands::prefs::*;
use std::time::{Duration, Instant};

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("proxpulse-test-{}-{name}", std::process::id()))
}

#[test]
fn missing_or_broken_prefs_fall_back_to_defaults() {
    let path = temp_path("missing.json");
    let _ = std::fs::remove_file(&path);
    assert_eq!(read_prefs_file(&path), AppPrefs::default());

    std::fs::write(&path, b"{not json").unwrap();
    assert_eq!(read_prefs_file(&path), AppPrefs::default());
    let _ = std::fs::remove_file(&path);

    let d = AppPrefs::default();
    assert!(!d.close_to_tray && d.start_minimized && !d.start_dispatcher);
    assert_eq!(d.dispatcher.port, 1080);
}

#[test]
fn prefs_round_trip_and_sanitize_the_dispatcher() {
    let path = temp_path("roundtrip.json");
    let mut prefs = AppPrefs::default();
    prefs.apply(AppPrefsUpdate {
        close_to_tray: Some(true),
        start_dispatcher: Some(true),
        ..AppPrefsUpdate::default()
    });
    write_prefs_file(&path, &prefs).unwrap();
    assert_eq!(read_prefs_file(&path), prefs);

    std::fs::write(
        &path,
        br#"{"closeToTray":true,"dispatcher":{"port":0,"mode":"evil","listener":"x"}}"#,
    )
    .unwrap();
    let read = read_prefs_file(&path);
    let _ = std::fs::remove_file(&path);
    assert!(read.close_to_tray);
    assert!(read.start_minimized, "missing fields keep their defaults");
    assert_eq!(read.dispatcher, DispatcherPrefs::default());
}

#[test]
fn partial_updates_change_only_the_given_fields() {
    let mut prefs = AppPrefs::default();
    prefs.apply(AppPrefsUpdate {
        start_minimized: Some(false),
        ..AppPrefsUpdate::default()
    });
    assert!(!prefs.start_minimized);
    assert!(!prefs.close_to_tray);

    prefs.apply(AppPrefsUpdate {
        dispatcher: Some(DispatcherPrefs {
            port: 3128,
            mode: "failover".to_string(),
            listener: "http".to_string(),
            ..DispatcherPrefs::default()
        }),
        ..AppPrefsUpdate::default()
    });
    assert_eq!(prefs.dispatcher.port, 3128);
    assert_eq!(prefs.dispatcher.mode, "failover");
    assert_eq!(prefs.dispatcher.listener, "http");
    assert!(!prefs.start_minimized);
}

#[test]
fn only_started_named_dispatchers_start_at_launch() {
    let mut prefs = AppPrefs::default();
    // Options edited on a pool whose dispatcher never ran.
    prefs
        .pool_dispatchers
        .insert("p1".into(), DispatcherPrefs::default());
    prefs.pool_dispatchers.insert(
        "p2".into(),
        DispatcherPrefs {
            started: true,
            ..DispatcherPrefs::default()
        },
    );
    assert!(starts_at_launch(&prefs, MAIN_POOL));
    assert!(!starts_at_launch(&prefs, "p1"));
    assert!(starts_at_launch(&prefs, "p2"));
    assert!(!starts_at_launch(&prefs, "p3"));

    // Entries saved without the flag read as never started.
    let old: DispatcherPrefs =
        serde_json::from_str(r#"{"port":1081,"mode":"fastest","listener":"both"}"#).unwrap();
    assert!(!old.started);
}

#[tokio::test]
async fn saved_options_reach_running_dispatchers() {
    let disp = DispatchState::default();
    let named = disp.add("p1");
    *named.options.lock().await = DispatchOptions {
        sticky_min: 5,
        ..DispatchOptions::default()
    };
    named.sticky.lock().await.insert(
        "example.com".into(),
        (
            "http://1.2.3.4:80".into(),
            Instant::now() + Duration::from_secs(60),
        ),
    );
    // An imported file with a login for the main pool only.
    let login = DispatchOptions {
        auth_user: "u".into(),
        auth_pass: "p".into(),
        ..DispatchOptions::default()
    };
    let mut prefs = AppPrefs::default();
    prefs.dispatcher.options = login.clone();
    apply_dispatch_options(&disp, &prefs).await;

    assert_eq!(*disp.inner.options.lock().await, login);
    // A pool the preferences do not name gets the defaults, and its sticky
    // bindings made under the old options are dropped.
    assert_eq!(*named.options.lock().await, DispatchOptions::default());
    assert!(named.sticky.lock().await.is_empty());
}

#[tokio::test]
async fn settings_without_a_login_stop_a_running_dispatcher_that_had_one() {
    use std::sync::atomic::Ordering;
    let disp = DispatchState::default();
    let login = DispatchOptions {
        auth_user: "u".into(),
        auth_pass: "p".into(),
        ..DispatchOptions::default()
    };
    *disp.inner.options.lock().await = login.clone();
    disp.inner.running.store(true, Ordering::SeqCst);
    let named = disp.add("p1");
    *named.options.lock().await = login;

    // A reset, or an import whose file has no dispatcher settings.
    apply_dispatch_options(&disp, &AppPrefs::default()).await;

    // The running one is stopped, not left open to every local process.
    assert!(!disp.inner.running.load(Ordering::SeqCst));
    assert!(disp.inner.last_error.lock().await.is_some());
    // One that is not running just takes the new settings.
    assert!(named.options.lock().await.auth_user.is_empty());
    assert!(named.last_error.lock().await.is_none());
}
