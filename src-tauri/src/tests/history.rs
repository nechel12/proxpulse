use crate::commands::check::dead_result;
use crate::commands::history::*;
use std::collections::HashMap;

fn alive(url: &str, ms: u64) -> crate::ProxyResult {
    let mut r = dead_result(url.into(), "HTTP".into(), 1, String::new());
    r.alive = true;
    r.error = None;
    r.latency_ms = Some(ms);
    r
}

#[test]
fn history_counts_checks_and_smooths_ping() {
    let mut map = HashMap::new();
    record(&mut map, &[alive("http://u:p@1.2.3.4:80", 100)], 10);
    record(&mut map, &[alive("http://1.2.3.4:80", 200)], 20);
    let dead = dead_result(
        "http://1.2.3.4:80".into(),
        "HTTP".into(),
        1,
        "timeout".into(),
    );
    record(&mut map, &[dead], 30);
    let cancelled = dead_result(
        "http://1.2.3.4:80".into(),
        "HTTP".into(),
        0,
        "cancelled".into(),
    );
    record(&mut map, &[cancelled], 40);

    // Credentials are not part of the key, and a stopped check is not counted.
    let e = &map["http://1.2.3.4:80"];
    assert_eq!(map.len(), 1);
    assert_eq!((e.checks, e.alive), (3, 2));
    assert_eq!((e.last_alive, e.last_check), (20, 30));
    assert_eq!(e.avg_ms, Some(125));
}

#[test]
fn history_drops_the_oldest_entries_beyond_the_cap() {
    let mut map = HashMap::new();
    for i in 0..(HISTORY_MAX + 5) {
        map.insert(
            format!("http://h{i}:80"),
            HistEntry {
                checks: 1,
                last_check: i as u64,
                ..HistEntry::default()
            },
        );
    }
    record(&mut map, &[alive("http://new:80", 1)], 1_000_000);
    assert_eq!(map.len(), HISTORY_MAX);
    assert!(map.contains_key("http://new:80"));
    assert!(!map.contains_key("http://h0:80"));
}

#[test]
fn history_file_round_trip() {
    let path = std::env::temp_dir().join(format!(
        "proxpulse-test-{}-history.json",
        std::process::id()
    ));
    let mut map = HashMap::new();
    record(&mut map, &[alive("http://a:80", 50)], 5);
    std::fs::write(&path, serde_json::to_vec(&map).unwrap()).unwrap();
    assert_eq!(read_history_file(&path), map);
    std::fs::write(&path, b"not json").unwrap();
    assert!(read_history_file(&path).is_empty());
    let _ = std::fs::remove_file(&path);
}
