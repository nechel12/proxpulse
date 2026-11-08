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
fn raw_lines_never_key_the_history() {
    // A raw `host:port:user:pass` line would split the proxy's history and
    // put its password into the key; only normalized URLs are recorded.
    let mut map = HashMap::new();
    let raw = dead_result(
        "1.2.3.4:8080:user:secret".into(),
        "HTTP/SOCKS5".into(),
        1,
        "timeout".into(),
    );
    record(&mut map, &[raw], 10);
    assert!(map.is_empty());
    let mut keyed = dead_result(
        "1.2.3.4:8080:user:secret".into(),
        "HTTP/SOCKS5".into(),
        1,
        "timeout".into(),
    );
    keyed.url = "http://user:secret@1.2.3.4:8080".into();
    record(&mut map, &[keyed], 20);
    assert_eq!(map.keys().collect::<Vec<_>>(), vec!["http://1.2.3.4:8080"]);
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
    let garbage = dead_result("not a proxy".into(), "BAD".into(), 0, "bad format".into());
    record(&mut map, &[garbage], 50);

    // Credentials are not part of the key; a stopped check and a line that
    // is not a proxy are not counted.
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
    // Written the way the app writes it: a temporary file renamed over the
    // old one, which must also work when the file already exists.
    let write = |map: &HashMap<String, HistEntry>| {
        crate::commands::dispatch::write_private_file(&path, &serde_json::to_vec(map).unwrap())
            .unwrap();
    };
    let mut map = HashMap::new();
    record(&mut map, &[alive("http://a:80", 50)], 5);
    write(&map);
    assert_eq!(read_history_file(&path), map);
    record(&mut map, &[alive("http://b:80", 70)], 6);
    write(&map);
    assert_eq!(read_history_file(&path), map);
    assert!(!path.with_extension("json.tmp").exists());
    std::fs::write(&path, b"not json").unwrap();
    assert!(read_history_file(&path).is_empty());
    let _ = std::fs::remove_file(&path);
}
