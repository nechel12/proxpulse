use crate::commands::dispatch::{
    is_listener_port, is_self_upstream_async, pool_file_name, start_proxy_inner, stop_proxy_inner,
    DispatchState, MAIN_POOL,
};
use crate::commands::pools::{
    clean_name, merge_pools, normalize, read_pools_file, valid_pool_id, PoolMeta, MAX_POOLS,
};
use crate::transport::Upstream;
use std::collections::HashMap;
use tokio::sync::Mutex;

fn meta(id: &str, name: &str) -> PoolMeta {
    PoolMeta {
        id: id.to_string(),
        name: name.to_string(),
    }
}

#[test]
fn pool_registry_is_repaired() {
    assert!(valid_pool_id("p1a2b3"));
    assert!(!valid_pool_id("Bad-Id"));
    assert!(!valid_pool_id(""));
    assert!(!valid_pool_id("../pool"));
    assert_eq!(clean_name("  Скрапинг\n "), "Скрапинг");
    assert_eq!(clean_name(&"x".repeat(100)).chars().count(), 40);

    let list = normalize(vec![
        meta("p1", "Work"),
        meta("p1", "Duplicate"),
        meta("BAD", "Bad"),
        meta(MAIN_POOL, "Main"),
    ]);
    assert_eq!(list, vec![meta(MAIN_POOL, "Main"), meta("p1", "Work")]);
    // A missing or broken file still yields the main pool.
    let path =
        std::env::temp_dir().join(format!("proxpulse-test-{}-pools.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    assert_eq!(read_pools_file(&path), vec![meta(MAIN_POOL, "")]);
    let many: Vec<PoolMeta> = (0..20).map(|i| meta(&format!("p{i}"), "")).collect();
    assert_eq!(normalize(many).len(), MAX_POOLS);
}

#[test]
fn imported_pools_keep_the_main_name_and_the_limit() {
    let mut list = vec![meta(MAIN_POOL, "Home"), meta("p1", "Old")];
    let added = merge_pools(&mut list, vec![meta("p1", "Work"), meta("p2", "New")]);
    assert_eq!(added, vec!["p2"]);
    assert_eq!(
        list,
        vec![
            meta(MAIN_POOL, "Home"),
            meta("p1", "Work"),
            meta("p2", "New")
        ]
    );

    // A full registry takes no more pools.
    let mut full = normalize((1..MAX_POOLS).map(|i| meta(&format!("p{i}"), "")).collect());
    assert!(merge_pools(&mut full, vec![meta("extra", "Extra")]).is_empty());
    assert_eq!(full.len(), MAX_POOLS);
}

#[test]
fn pools_are_looked_up_by_id() {
    let st = DispatchState::default();
    assert_eq!(st.pool(None).unwrap().id, MAIN_POOL);
    assert!(st.pool(Some("p1")).is_err());
    st.add("p1");
    assert_eq!(st.pool(Some("p1")).unwrap().id, "p1");
    assert_eq!(
        st.all().iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        vec![MAIN_POOL, "p1"]
    );
    assert!(st.remove("p1").is_some());
    assert!(st.pool(Some("p1")).is_err());
    assert_eq!(pool_file_name(MAIN_POOL), "pool.json");
    assert_eq!(pool_file_name("p1"), "pool-p1.json");
}

#[tokio::test]
async fn another_pools_listener_is_not_an_upstream() {
    let st = DispatchState::default();
    let named = st.add("p2");
    *named.pool.lock().await = vec![Upstream {
        raw: "http://10.255.255.1:3128".into(),
        url: "http://10.255.255.1:3128".into(),
        scheme: "http".into(),
        host: "10.255.255.1".into(),
        port: 3128,
        user: None,
        pass: None,
        latency: None,
        country: None,
    }];
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    start_proxy_inner(&named, port, "round-robin".into(), "both".into())
        .await
        .unwrap();
    let bound = *named.port.lock().await;
    assert!(is_listener_port(bound));

    // The main pool's dispatcher on another port must skip it.
    let looping = Upstream {
        raw: format!("http://127.0.0.1:{bound}"),
        url: format!("http://127.0.0.1:{bound}"),
        scheme: "http".into(),
        host: "127.0.0.1".into(),
        port: bound,
        user: None,
        pass: None,
        latency: None,
        country: None,
    };
    let cache = Mutex::new(HashMap::new());
    assert!(is_self_upstream_async(&looping, bound.wrapping_add(1), &cache).await);

    stop_proxy_inner(&named).await;
    assert!(!is_listener_port(bound));
}
