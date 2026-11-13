use crate::commands::auto::*;
use crate::commands::check::{dead_result, CheckConfig};
use crate::commands::dispatch::{DispatchPoolItem, MAIN_POOL};
use crate::ProxyResult;
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
