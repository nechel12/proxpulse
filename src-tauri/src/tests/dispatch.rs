use crate::commands::dispatch::{
    abort_connections, handle_http, handle_socks5, http_auth_ok, is_self_literal, is_self_upstream,
    is_self_upstream_async, parse_socks_auth, parse_target, pick_order, plan_upstreams, pool_items,
    read_pool_file, store_pool, upstream_from_item, write_pool_file, CountingIo, DispatchItem,
    DispatchOptions, DispatchPoolItem,
};
use crate::transport::Upstream;
use crate::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn mk_upstream(host: &str, port: u16) -> Upstream {
    Upstream {
        raw: String::new(),
        url: format!("http://{host}:{port}"),
        scheme: "http".into(),
        host: host.into(),
        port,
        user: None,
        pass: None,
        latency: None,
        country: None,
    }
}

#[test]
fn target_parsing() {
    assert_eq!(
        parse_target("CONNECT", "example.com:443"),
        Some(("example.com".into(), 443))
    );
    assert_eq!(
        parse_target("CONNECT", "[::1]:443"),
        Some(("::1".into(), 443))
    );
    assert_eq!(
        parse_target("GET", "http://example.com:8080/path?q=1"),
        Some(("example.com".into(), 8080))
    );
    assert_eq!(
        parse_target("GET", "http://example.com/path"),
        Some(("example.com".into(), 80))
    );
    assert_eq!(
        parse_target("GET", "HTTP://example.com:8080/path"),
        Some(("example.com".into(), 8080))
    );
    assert_eq!(
        parse_target("GET", "http://example.com?x=1"),
        Some(("example.com".into(), 80))
    );
    assert_eq!(
        parse_target("GET", "http://example.com#frag"),
        Some(("example.com".into(), 80))
    );
    assert_eq!(parse_target("GET", "/relative"), None);
}

#[test]
fn order_modes() {
    let rr = AtomicUsize::new(0);
    // set_dispatch_pool keeps the pool sorted by latency.
    assert_eq!(pick_order(3, "fastest", &rr, 3), vec![0]);
    assert_eq!(pick_order(3, "failover", &rr, 2), vec![0, 1]);
    assert_eq!(pick_order(0, "fastest", &rr, 3), Vec::<usize>::new());
    let a = pick_order(3, "round-robin", &rr, 3);
    let b = pick_order(3, "round-robin", &rr, 3);
    assert_eq!(a, vec![0, 1, 2]);
    assert_eq!(b, vec![1, 2, 0]);
}

#[test]
fn self_loop_detection() {
    assert!(is_self_upstream(&mk_upstream("127.0.0.1", 1080), 1080));
    assert!(is_self_upstream(&mk_upstream("127.0.0.2", 1080), 1080));
    assert!(is_self_upstream(&mk_upstream("localhost", 1080), 1080));
    assert!(is_self_upstream(&mk_upstream("[::1]", 1080), 1080));
    assert!(is_self_upstream(&mk_upstream("0.0.0.0", 1080), 1080));
    assert!(is_self_upstream(&mk_upstream("::", 1080), 1080));
    assert!(!is_self_upstream(&mk_upstream("127.0.0.1", 1081), 1080));
    assert!(!is_self_upstream(&mk_upstream("1.2.3.4", 1080), 1080));
    assert!(is_self_literal("localhost."));
    assert!(is_self_literal("[::1]"));
    assert!(!is_self_literal("example.com"));
}

#[tokio::test]
async fn self_loop_hostname_check_is_port_scoped() {
    let cache = Mutex::new(HashMap::new());
    let up = mk_upstream("localhost", 1080);
    assert!(is_self_upstream_async(&up, 1080, &cache).await);
    assert!(!is_self_upstream_async(&up, 1081, &cache).await);
}

#[tokio::test]
async fn abort_connections_drains_active_tasks() {
    let st = DispatchState::default();
    {
        let mut conns = st.inner.connections.lock().await;
        conns.spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
    }
    abort_connections(&st.inner).await;
    assert_eq!(st.inner.connections.lock().await.len(), 0);
}

#[tokio::test]
async fn counting_io_tracks_written_bytes() {
    let (client, mut peer) = tokio::io::duplex(64);
    let written = Arc::new(AtomicU64::new(0));
    let mut counted = CountingIo::new(client, written.clone());
    counted.write_all(b"hello").await.unwrap();
    assert_eq!(written.load(Ordering::Relaxed), 5);
    let mut got = [0u8; 5];
    peer.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"hello");
}

#[test]
fn dispatch_item_preserves_normalized_scheme() {
    let (_, up) = upstream_from_item(
        "1.2.3.4:1080",
        Some("socks5://1.2.3.4:1080"),
        Some(42),
        Some("DE".into()),
    )
    .unwrap();
    assert_eq!(up.scheme, "socks5");
    assert_eq!(up.host, "1.2.3.4");
    assert_eq!(up.port, 1080);
    assert_eq!(up.raw, "1.2.3.4:1080");
    assert_eq!(up.url, "socks5://1.2.3.4:1080");
    assert_eq!(up.latency, Some(42));
    assert_eq!(up.country.as_deref(), Some("DE"));

    let (_, explicit) = upstream_from_item("socks5://1.2.3.4:1080", None, None, None).unwrap();
    assert_eq!(explicit.scheme, "socks5");
    assert_eq!(explicit.url, "socks5://1.2.3.4:1080");
}

#[tokio::test]
async fn store_pool_keeps_normalized_url_and_country() {
    let st = DispatchState::default();
    let items = vec![
        DispatchItem {
            raw: "u:p@1.2.3.4:8080".into(),
            url: Some("socks5://u:p@1.2.3.4:8080".into()),
            latency: Some(42),
            country: Some("DE".into()),
        },
        DispatchItem {
            raw: "1.2.3.4:8080".into(),
            url: Some("socks5://u:p@1.2.3.4:8080".into()),
            latency: Some(10),
            country: Some("NL".into()),
        },
    ];
    assert_eq!(store_pool(&st.inner, items, true).await, 1);
    let pool = pool_items(&st.inner).await;
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].url, "socks5://u:p@1.2.3.4:8080");
    assert_eq!(pool[0].raw, "u:p@1.2.3.4:8080");
    assert_eq!(pool[0].latency, Some(42));
    assert_eq!(pool[0].country.as_deref(), Some("DE"));
}

fn pool_item(url: &str) -> DispatchItem {
    DispatchItem {
        raw: url.into(),
        url: Some(url.into()),
        latency: Some(10),
        country: None,
    }
}

#[tokio::test]
async fn automatic_prune_keeps_the_listener_running() {
    let st = DispatchState::default();
    store_pool(&st.inner, vec![pool_item("http://1.2.3.4:8080")], true).await;
    // Port 0 lets the OS pick a free port for the test listener.
    start_proxy_inner(&st.inner, 0, "round-robin".into(), "both".into())
        .await
        .unwrap();
    assert!(st.inner.running.load(Ordering::SeqCst));

    // Automatic pruning to an empty pool must not stop the dispatcher.
    store_pool(&st.inner, Vec::new(), false).await;
    assert!(st.inner.running.load(Ordering::SeqCst));

    // Clearing the pool is an explicit user action and stops it.
    store_pool(&st.inner, Vec::new(), true).await;
    assert!(!st.inner.running.load(Ordering::SeqCst));
}

#[tokio::test]
async fn store_pool_forgets_failures_of_removed_entries() {
    let st = DispatchState::default();
    let items = vec![
        pool_item("http://1.2.3.4:8080"),
        pool_item("http://5.6.7.8:8080"),
    ];
    store_pool(&st.inner, items, true).await;
    *st.inner.fails.lock().await = HashMap::from([
        ("http://1.2.3.4:8080".to_string(), 1),
        ("http://5.6.7.8:8080".to_string(), 1),
    ]);
    store_pool(&st.inner, vec![pool_item("http://1.2.3.4:8080")], true).await;
    let fails = st.inner.fails.lock().await.clone();
    assert_eq!(fails.len(), 1);
    assert_eq!(fails.get("http://1.2.3.4:8080"), Some(&1));
}

#[test]
fn pool_file_round_trip() {
    let dir = std::env::temp_dir().join(format!("proxpulse-pool-{}", std::process::id()));
    let path = dir.join("pool.json");
    assert!(
        read_pool_file(&path).is_empty(),
        "missing file reads as empty"
    );
    let items = vec![DispatchPoolItem {
        raw: "u:p@1.2.3.4:8080".into(),
        url: "http://u:p@1.2.3.4:8080".into(),
        latency: Some(42),
        country: Some("DE".into()),
    }];
    write_pool_file(&path, &items).unwrap();
    let back = read_pool_file(&path);
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].raw, "u:p@1.2.3.4:8080");
    assert_eq!(back[0].url.as_deref(), Some("http://u:p@1.2.3.4:8080"));
    assert_eq!(back[0].latency, Some(42));
    assert_eq!(back[0].country.as_deref(), Some("DE"));

    std::fs::write(&path, b"not json").unwrap();
    assert!(
        read_pool_file(&path).is_empty(),
        "corrupt file reads as empty"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn socks5_dispatcher_tunnels_via_http_proxy() {
    let mock = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let mock_port = mock.local_addr().unwrap().port();
    let (connect_tx, connect_rx) = tokio::sync::oneshot::channel::<String>();
    let (ping_tx, ping_rx) = tokio::sync::oneshot::channel::<Vec<u8>>();
    tokio::spawn(async move {
        let (mut s, _) = mock.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            let n = s.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let _ = connect_tx.send(String::from_utf8_lossy(&buf).to_string());
        let _ = s
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await;
        let mut ping = [0u8; 4];
        let _ = s.read_exact(&mut ping).await;
        let _ = ping_tx.send(ping.to_vec());
        let _ = s.write_all(b"pong").await;
        let _ = s.shutdown().await;
    });

    let proxy = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let proxy_port = proxy.local_addr().unwrap().port();
    let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    let (server, _) = proxy.accept().await.unwrap();
    let st = DispatchState::default();
    *st.inner.port.lock().await = proxy_port;
    *st.inner.mode.lock().await = "failover".to_string();
    *st.inner.pool.lock().await = vec![Upstream {
        raw: format!("http://127.0.0.1:{mock_port}"),
        url: format!("http://127.0.0.1:{mock_port}"),
        scheme: "http".to_string(),
        host: "127.0.0.1".to_string(),
        port: mock_port,
        user: None,
        pass: None,
        latency: None,
        country: None,
    }];
    let inner = st.inner.clone();
    let h = tokio::spawn(async move {
        handle_socks5(server, inner).await;
    });

    client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
    let mut method = [0u8; 2];
    client.read_exact(&mut method).await.unwrap();
    assert_eq!(method, [0x05, 0x00]);

    let mut connect = vec![0x05, 0x01, 0x00, 0x03, 11];
    connect.extend_from_slice(b"example.com");
    connect.extend_from_slice(&80u16.to_be_bytes());
    client.write_all(&connect).await.unwrap();
    let mut reply = [0u8; 10];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply[..2], &[0x05, 0x00]);

    client.write_all(b"ping").await.unwrap();
    let mut pong = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut pong))
        .await
        .expect("pong timeout")
        .unwrap();
    assert_eq!(&pong, b"pong");
    let _ = client.shutdown().await;
    tokio::time::timeout(Duration::from_secs(5), h)
        .await
        .expect("handle_socks5 should finish")
        .unwrap();

    let captured = tokio::time::timeout(Duration::from_secs(2), connect_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(captured.starts_with("CONNECT example.com:80 "));
    let ping = tokio::time::timeout(Duration::from_secs(2), ping_rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ping, b"ping");
}

#[tokio::test]
async fn http_forward_uses_absolute_form_and_proxy_auth() {
    let mock = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let mock_port = mock.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        let (mut s, _) = mock.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            let n = s.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&buf).to_string());
        let _ = s
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await;
        let _ = s.shutdown().await;
    });

    let proxy = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let proxy_port = proxy.local_addr().unwrap().port();
    let mut client = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    let (server, _) = proxy.accept().await.unwrap();

    let st = DispatchState::default();
    *st.inner.port.lock().await = proxy_port;
    *st.inner.mode.lock().await = "failover".to_string();
    *st.inner.pool.lock().await = vec![Upstream {
        raw: format!("http://u:p@127.0.0.1:{mock_port}"),
        url: format!("http://u:p@127.0.0.1:{mock_port}"),
        scheme: "http".to_string(),
        host: "127.0.0.1".to_string(),
        port: mock_port,
        user: Some("u".to_string()),
        pass: Some("p".to_string()),
        latency: None,
        country: None,
    }];
    let inner = st.inner.clone();
    let h = tokio::spawn(async move {
        handle_http(server, inner).await;
    });

    client
        .write_all(b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n")
        .await
        .unwrap();
    // Half-close the request side so copy_bidirectional can finish once
    // the mock proxy has sent its response and closed the upstream.
    let _ = client.shutdown().await;
    let mut resp = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut resp)).await;
    tokio::time::timeout(Duration::from_secs(5), h)
        .await
        .expect("handle_http should finish")
        .unwrap();

    let captured = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .unwrap()
        .unwrap();
    assert!(captured.starts_with("GET http://example.com/ HTTP/1.1"));
    assert!(captured.contains("Proxy-Authorization: Basic dTpw"));
    // Only this request is rewritten for the upstream, so the connection
    // must not be reused for the next one.
    assert!(captured.contains("Connection: close\r\n"));
    assert!(String::from_utf8_lossy(&resp).contains("200 OK"));
}

fn with_country(host: &str, country: &str) -> Upstream {
    Upstream {
        country: Some(country.to_string()),
        ..mk_upstream(host, 80)
    }
}

fn hosts(v: &[Upstream]) -> Vec<&str> {
    v.iter().map(|u| u.host.as_str()).collect()
}

#[test]
fn upstream_plan_filters_countries_cools_down_and_sticks() {
    let pool = vec![
        with_country("a", "DE"),
        with_country("b", "NL"),
        with_country("c", "de"),
        mk_upstream("d", 80),
    ];
    let rr = AtomicUsize::new(0);
    let none = HashSet::new();
    let de = vec!["DE".to_string()];
    assert_eq!(
        hosts(&plan_upstreams(&pool, "failover", &rr, &de, &none, None, 5)),
        vec!["a", "c"]
    );

    // An upstream in cooldown is skipped while another one is left.
    let cooling: HashSet<String> = ["http://a:80".to_string()].into();
    assert_eq!(
        hosts(&plan_upstreams(
            &pool, "failover", &rr, &de, &cooling, None, 5
        )),
        vec!["c"]
    );
    let all: HashSet<String> = ["http://a:80".to_string(), "http://c:80".to_string()].into();
    assert_eq!(
        hosts(&plan_upstreams(&pool, "failover", &rr, &de, &all, None, 5)),
        vec!["a", "c"]
    );

    // The sticky upstream of the host comes first, unless it is cooling down.
    let plan = plan_upstreams(&pool, "failover", &rr, &[], &none, Some("http://d:80"), 2);
    assert_eq!(hosts(&plan), vec!["d", "a"]);
    let cooling_d: HashSet<String> = ["http://d:80".to_string()].into();
    let plan = plan_upstreams(
        &pool,
        "failover",
        &rr,
        &[],
        &cooling_d,
        Some("http://d:80"),
        2,
    );
    assert_eq!(hosts(&plan), vec!["a", "b"]);

    let fr = vec!["FR".to_string()];
    assert!(plan_upstreams(&pool, "failover", &rr, &fr, &none, None, 5).is_empty());
}

#[test]
fn dispatcher_login_checks() {
    let head = "GET http://x/ HTTP/1.1\r\nHost: x\r\nProxy-Authorization: Basic dTpwYXNz\r\n\r\n";
    assert!(http_auth_ok(head, "u", "pass"));
    assert!(!http_auth_ok(head, "u", "other"));
    assert!(!http_auth_ok(
        "GET http://x/ HTTP/1.1\r\nHost: x\r\n\r\n",
        "u",
        "pass"
    ));
    assert!(!http_auth_ok(
        "GET http://x/ HTTP/1.1\r\nProxy-Authorization: Bearer dTpwYXNz\r\n\r\n",
        "u",
        "pass"
    ));

    assert_eq!(
        parse_socks_auth(&[1, b'u', 2, b'p', b'w']),
        Some((&b"u"[..], &b"pw"[..]))
    );
    assert_eq!(parse_socks_auth(&[3, b'u']), None);

    let o = DispatchOptions {
        countries: vec![" DE ".into(), String::new()],
        sticky_min: 5000,
        auth_user: "  ".into(),
        auth_pass: "secret".into(),
    }
    .sanitized();
    assert_eq!(o.countries, vec!["DE"]);
    assert_eq!(o.sticky_min, 1440);
    assert!(
        o.auth_pass.is_empty(),
        "a password without a login is dropped"
    );
}

#[tokio::test]
async fn dispatcher_requires_the_configured_login() {
    let st = DispatchState::default();
    *st.inner.options.lock().await = DispatchOptions {
        auth_user: "u".into(),
        auth_pass: "pass".into(),
        ..DispatchOptions::default()
    };

    // HTTP without credentials gets 407.
    let proxy = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = proxy.local_addr().unwrap().port();
    let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (server, _) = proxy.accept().await.unwrap();
    let inner = st.inner.clone();
    tokio::spawn(async move { handle_http(server, inner).await });
    client
        .write_all(b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut buf)).await;
    assert!(String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 407"));

    // SOCKS5 offers only "no auth": refused; a wrong password: rejected.
    let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (server, _) = proxy.accept().await.unwrap();
    let inner = st.inner.clone();
    tokio::spawn(async move { handle_socks5(server, inner).await });
    client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
    let mut reply = [0u8; 2];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply, [0x05, 0xFF]);

    let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (server, _) = proxy.accept().await.unwrap();
    let inner = st.inner.clone();
    tokio::spawn(async move { handle_socks5(server, inner).await });
    client.write_all(&[0x05, 0x02, 0x00, 0x02]).await.unwrap();
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply, [0x05, 0x02]);
    client
        .write_all(&[0x01, 1, b'u', 3, b'b', b'a', b'd'])
        .await
        .unwrap();
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply, [0x01, 0x01]);
}
