//! Checks against local stub proxies: real reqwest errors and whole runs,
//! with no request leaving the machine.

use crate::commands::check::{run_checks, CheckConfig};
use crate::*;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

/// A server that answers each connection with `script`: it reads what the
/// client sent, writes the next reply, and closes after the last one.
async fn stub(script: &'static [&'static [u8]]) -> (SocketAddr, JoinHandle<()>) {
    let (addr, task, _) = counting_stub(script).await;
    (addr, task)
}

/// [`stub`] that also counts the requests it answered (a bare connect, as
/// the port precheck makes, sends nothing and is not counted).
async fn counting_stub(
    script: &'static [&'static [u8]],
) -> (SocketAddr, JoinHandle<()>, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let counter = count.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut conn, _)) = listener.accept().await {
            let counter = counter.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                for reply in script {
                    if conn.read(&mut buf).await.unwrap_or(0) == 0 {
                        return;
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    if conn.write_all(reply).await.is_err() {
                        return;
                    }
                }
                let _ = conn.shutdown().await;
            });
        }
    });
    (addr, task, count)
}

async fn error_through(proxy: &str, target: &str) -> String {
    let client = build_client(Some(proxy), Duration::from_secs(5), false).unwrap();
    let err = client.get(target).send().await.unwrap_err();
    request_error(&err)
}

// A documentation address: nothing is resolved, and the stubs never connect on.
const TARGET_HTTPS: &str = "https://192.0.2.1/";
const TARGET_HTTP: &str = "http://192.0.2.1/";

#[tokio::test]
async fn a_407_to_connect_is_proxy_auth_required() {
    let (addr, task) = stub(&[
        b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic\r\nContent-Length: 0\r\n\r\n",
    ])
    .await;
    assert_eq!(
        error_through(&format!("http://{addr}"), TARGET_HTTPS).await,
        "proxy auth required"
    );
    task.abort();
}

#[tokio::test]
async fn a_refused_connect_is_a_proxy_error() {
    let (addr, task) = stub(&[b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n"]).await;
    assert_eq!(
        error_through(&format!("http://{addr}"), TARGET_HTTPS).await,
        "proxy error"
    );
    task.abort();
}

#[tokio::test]
async fn a_socks5_refusal_is_reported_as_such() {
    // No authentication, then reply code 5: connection refused.
    let (addr, task) = stub(&[b"\x05\x00", b"\x05\x05\x00\x01\x00\x00\x00\x00\x00\x00"]).await;
    assert_eq!(
        error_through(&format!("socks5://{addr}"), TARGET_HTTP).await,
        "connection refused"
    );
    task.abort();
}

#[tokio::test]
async fn a_server_that_is_not_socks_is_a_socks_error() {
    let (addr, task) = stub(&[b"HTTP/1.1 400 Bad Request\r\n\r\n"]).await;
    assert_eq!(
        error_through(&format!("socks5://{addr}"), TARGET_HTTP).await,
        "socks error"
    );
    task.abort();
}

/// An HTTP proxy that answers every request itself, as a working proxy
/// would relay the test page.
const OK_PAGE: &[&[u8]] = &[b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"];

fn plain_config(test_url: String) -> CheckConfig {
    CheckConfig {
        test_url,
        timeout_ms: 3000,
        repeats: 2,
        with_geo: false,
        with_anonymity: false,
        with_tamper: false,
        with_tls: false,
        ..CheckConfig::default()
    }
}

#[tokio::test]
async fn a_working_http_proxy_is_alive() {
    let (addr, task) = stub(OK_PAGE).await;
    let out = run_checks(
        vec![addr.to_string()],
        plain_config(TARGET_HTTP.to_string()),
        Vec::new(),
        Arc::new(AtomicBool::new(false)),
        None,
    )
    .await
    .unwrap();
    task.abort();
    assert_eq!(out.len(), 1);
    let r = &out[0];
    assert!(r.alive, "{:?}", r.error);
    assert_eq!(r.proto, "HTTP");
    assert_eq!(r.url, format!("http://{addr}"));
    assert_eq!(r.attempts, 2);
    assert!((r.success_rate - 1.0).abs() < f32::EPSILON);
    assert!(r.latency_ms.is_some());
    assert_eq!(r.error, None);
}

#[tokio::test]
async fn http_and_socks5_mode_finds_the_protocol_that_works() {
    let (addr, task) = stub(OK_PAGE).await;
    let cfg = CheckConfig {
        default_proto: "http+socks5".to_string(),
        ..plain_config(TARGET_HTTP.to_string())
    };
    let out = run_checks(
        vec![addr.to_string()],
        cfg,
        Vec::new(),
        Arc::new(AtomicBool::new(false)),
        None,
    )
    .await
    .unwrap();
    task.abort();
    assert!(out[0].alive, "{:?}", out[0].error);
    assert_eq!(out[0].proto, "HTTP");
}

#[tokio::test]
async fn a_proxy_that_answers_with_an_error_page_is_dead() {
    let (addr, task) =
        stub(&[b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"])
            .await;
    let out = run_checks(
        vec![addr.to_string()],
        plain_config(TARGET_HTTP.to_string()),
        Vec::new(),
        Arc::new(AtomicBool::new(false)),
        None,
    )
    .await
    .unwrap();
    task.abort();
    assert!(!out[0].alive);
    assert_eq!(out[0].error.as_deref(), Some("http 502"));
}

#[tokio::test]
async fn stop_ends_a_run_at_once() {
    // A proxy that accepts and never answers: without Stop, each check would
    // wait for the 3 s timeout.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = silent.local_addr().unwrap();
    let hold = tokio::spawn(async move {
        let mut open = Vec::new();
        while let Ok((conn, _)) = silent.accept().await {
            open.push(conn);
        }
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let started = std::time::Instant::now();
    let out = run_checks(
        vec![addr.to_string(); 3],
        plain_config(TARGET_HTTP.to_string()),
        Vec::new(),
        cancel,
        None,
    )
    .await
    .unwrap();
    hold.abort();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    // Proxies stopped mid-check report nothing rather than a false "dead".
    assert!(out.iter().all(|r| !r.alive));
}

#[tokio::test]
async fn a_cached_country_saves_the_geo_request() {
    let (addr, task, requests) = counting_stub(OK_PAGE).await;
    let url = format!("http://{addr}");
    let cached = GeoCacheEntry {
        proxy: geo_key(&url),
        ip: None,
        country: Some("Testland".into()),
        country_code: Some("TL".into()),
        city: None,
        isp: None,
        org: None,
        asn: None,
        ip_type: None,
    };
    let cfg = CheckConfig {
        with_geo: true,
        ..plain_config(TARGET_HTTP.to_string())
    };
    let out = run_checks(
        vec![addr.to_string()],
        cfg,
        vec![cached],
        Arc::new(AtomicBool::new(false)),
        None,
    )
    .await
    .unwrap();
    task.abort();
    let r = &out[0];
    assert!(r.alive, "{:?}", r.error);
    assert_eq!(r.country_code.as_deref(), Some("TL"));
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    // The two measured requests and no geo lookup through the proxy.
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}
