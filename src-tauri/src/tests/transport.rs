use crate::transport::*;
use std::net::IpAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::TcpStream;

#[test]
fn socks4_ipv6_policy() {
    assert!(build_socks4_request("2001:db8::1", 443, None, false).is_err());
    let req = build_socks4_request("2001:db8::1", 443, None, true).unwrap();
    assert_eq!(&req[..8], &[0x04, 0x01, 0x01, 0xbb, 0, 0, 0, 1]);
    assert!(req.ends_with(b"2001:db8::1\0"));

    let v4 = build_socks4_request("1.2.3.4", 8080, Some("u"), false).unwrap();
    assert_eq!(&v4[..9], &[0x04, 0x01, 0x1f, 0x90, 1, 2, 3, 4, b'u']);
    assert_eq!(v4.last(), Some(&0));
}

#[tokio::test]
async fn socks_local_dns_modes() {
    let timeout = Duration::from_secs(2);
    let local = socks_target_host("socks5", "localhost", 80, timeout)
        .await
        .unwrap();
    assert!(local.parse::<IpAddr>().unwrap().is_loopback());

    let remote = socks_target_host("socks5h", "localhost", 80, timeout)
        .await
        .unwrap();
    assert_eq!(remote, "localhost");

    assert_eq!(
        socks_target_host("socks4", "127.0.0.1", 80, timeout)
            .await
            .unwrap(),
        "127.0.0.1"
    );
}

#[test]
fn forward_request_adds_proxy_auth_once() {
    let up = Upstream {
        raw: "http://u:p@proxy:8080".into(),
        url: "http://u:p@proxy:8080".into(),
        scheme: "http".into(),
        host: "proxy".into(),
        port: 8080,
        user: Some("u".into()),
        pass: Some("p".into()),
        latency: None,
        country: None,
    };
    let body = b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\n\r\nBODY";
    let hdr_end = body
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap();
    let out = prepare_forward_request(body, hdr_end, &up);
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches("Proxy-Authorization: Basic ").count(), 1);
    assert!(text.contains("Basic dTpw"));
    assert!(text.ends_with("BODY"));

    let no_auth = Upstream {
        user: None,
        pass: None,
        ..up
    };
    assert_eq!(
        prepare_forward_request(body, hdr_end, &no_auth),
        body.to_vec()
    );
}

#[test]
fn forward_request_replaces_client_proxy_auth() {
    let up = Upstream {
        raw: "http://u:p@proxy:8080".into(),
        url: "http://u:p@proxy:8080".into(),
        scheme: "http".into(),
        host: "proxy".into(),
        port: 8080,
        user: Some("u".into()),
        pass: Some("p".into()),
        latency: None,
        country: None,
    };
    let body = b"GET http://example.com/ HTTP/1.1\r\nProxy-Authorization: Basic old\r\n\r\n";
    let hdr_end = body
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap();
    let text = String::from_utf8(prepare_forward_request(body, hdr_end, &up)).unwrap();
    assert_eq!(text.matches("Proxy-Authorization").count(), 1);
    assert!(text.contains("Basic dTpw"));
    assert!(!text.contains("Basic old"));
}

#[test]
fn forward_request_strips_client_auth_without_upstream_creds() {
    let up = Upstream {
        raw: "http://proxy:8080".into(),
        url: "http://proxy:8080".into(),
        scheme: "http".into(),
        host: "proxy".into(),
        port: 8080,
        user: None,
        pass: None,
        latency: None,
        country: None,
    };
    let body = b"GET http://example.com/ HTTP/1.1\r\nproxy-authorization: Basic old\r\nHost: example.com\r\n\r\nBODY";
    let hdr_end = body
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap();
    let out = strip_header(body, hdr_end, "proxy-authorization");
    let text = String::from_utf8(out.clone()).unwrap();
    assert!(!text.to_ascii_lowercase().contains("proxy-authorization"));
    assert!(text.ends_with("BODY"));
    assert_eq!(prepare_forward_request(body, hdr_end, &up), out);
}

#[test]
fn strip_header_keeps_other_headers_and_body() {
    let body = b"GET / HTTP/1.1\r\nX-Test: 1\r\nProxy-Authorization: x\r\n\r\nP";
    let end = find_header_end(body).unwrap();
    let stripped = strip_header(body, end, "Proxy-Authorization");
    let text = String::from_utf8(stripped).unwrap();
    assert!(text.contains("X-Test: 1"));
    assert!(!text.contains("Proxy-Authorization"));
    assert!(text.ends_with("P"));
}

#[tokio::test]
async fn http_connect_preserves_tunnel_bytes() {
    let mock = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = mock.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = mock.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 256];
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
        let _ = s
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\nEXTRA")
            .await;
    });
    let client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut stream = http_connect(
        ProxyStream::Plain(client),
        "example.com",
        80,
        None,
        None,
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    let mut extra = [0u8; 5];
    tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut extra))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&extra, b"EXTRA");
}
