use crate::proxy::canonical_scheme;
use crate::tls::tls_client_config;
use base64::Engine as _;
use std::io::{Error, ErrorKind};
use std::net::IpAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

// ================= tcp via upstream (shared by TLS check + dispatcher) =================

#[derive(Debug, Clone)]
pub(crate) struct Upstream {
    pub(crate) raw: String,
    /// Normalized URL actually used for checks (keeps the real scheme).
    pub(crate) url: String,
    pub(crate) scheme: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) user: Option<String>,
    pub(crate) pass: Option<String>,
    pub(crate) latency: Option<u64>,
    pub(crate) country: Option<String>,
}

pub(crate) type UpstreamParts = (String, String, u16, Option<String>, Option<String>);

pub(crate) fn parse_upstream_url(url: &str) -> Option<UpstreamParts> {
    let (scheme, rest) = url.split_once("://")?;
    let after = rest.rsplit('@').next()?;
    let userinfo = rest.rsplit_once('@').map(|(u, _)| u.to_string());
    let (user, pass) = match userinfo {
        Some(u) => match u.split_once(':') {
            Some((a, b)) => (Some(a.to_string()), Some(b.to_string())),
            None => (Some(u), None),
        },
        None => (None, None),
    };
    let hp = after.split('/').next()?;
    let (host, port) = if let Some(stripped) = hp.strip_prefix('[') {
        let end = stripped.find(']')?;
        let host = stripped[..end].to_string();
        let port: u16 = stripped[end + 1..].strip_prefix(':')?.parse().ok()?;
        (host, port)
    } else {
        let (h, p) = hp.rsplit_once(':')?;
        (h.to_string(), p.parse().ok()?)
    };
    Some((canonical_scheme(scheme), host, port, user, pass))
}

pub(crate) fn host_for_connect(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

pub(crate) enum ProxyStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for ProxyStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ProxyStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ProxyStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ProxyStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_flush(cx),
            ProxyStream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            ProxyStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ProxyStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

pub(crate) async fn read_exact_t<S: AsyncRead + Unpin>(
    s: &mut S,
    buf: &mut [u8],
    d: Duration,
) -> std::io::Result<()> {
    match tokio::time::timeout(d, s.read_exact(buf)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "read timeout")),
    }
}

pub(crate) async fn write_all_t<S: AsyncWrite + Unpin>(
    s: &mut S,
    buf: &[u8],
    d: Duration,
) -> std::io::Result<()> {
    match tokio::time::timeout(d, s.write_all(buf)).await {
        Ok(r) => r,
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "write timeout")),
    }
}

pub(crate) async fn socks5_connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    t_host: &str,
    t_port: u16,
    user: Option<String>,
    pass: Option<String>,
    d: Duration,
) -> std::io::Result<S> {
    if user.is_some() {
        write_all_t(&mut s, &[0x05, 0x01, 0x02], d).await?;
    } else {
        write_all_t(&mut s, &[0x05, 0x01, 0x00], d).await?;
    }
    let mut resp = [0u8; 2];
    read_exact_t(&mut s, &mut resp, d).await?;
    if resp[0] != 0x05 {
        return Err(Error::new(ErrorKind::InvalidData, "bad socks5 version"));
    }
    if resp[1] == 0x02 {
        let (u, p) = (user.unwrap_or_default(), pass.unwrap_or_default());
        let ub = u.as_bytes();
        let pb = p.as_bytes();
        if ub.len() > 255 || pb.len() > 255 {
            return Err(Error::new(ErrorKind::InvalidInput, "socks5 creds too long"));
        }
        let mut req = vec![0x01, ub.len() as u8];
        req.extend_from_slice(ub);
        req.push(pb.len() as u8);
        req.extend_from_slice(pb);
        write_all_t(&mut s, &req, d).await?;
        read_exact_t(&mut s, &mut resp, d).await?;
        if resp[1] != 0x00 {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "socks5 auth failed",
            ));
        }
    } else if resp[1] != 0x00 {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "socks5 auth required",
        ));
    }
    let mut req = vec![0x05, 0x01, 0x00];
    if let Ok(ip) = t_host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(v) => {
                req.push(0x01);
                req.extend_from_slice(&v.octets());
            }
            IpAddr::V6(v) => {
                req.push(0x04);
                req.extend_from_slice(&v.octets());
            }
        }
    } else {
        let hb = t_host.as_bytes();
        if hb.len() > 255 {
            return Err(Error::new(ErrorKind::InvalidInput, "hostname too long"));
        }
        req.push(0x03);
        req.push(hb.len() as u8);
        req.extend_from_slice(hb);
    }
    req.extend_from_slice(&t_port.to_be_bytes());
    write_all_t(&mut s, &req, d).await?;
    let mut hdr = [0u8; 4];
    read_exact_t(&mut s, &mut hdr, d).await?;
    if hdr[0] != 0x05 || hdr[1] != 0x00 {
        return Err(Error::new(
            ErrorKind::ConnectionRefused,
            format!("socks5 connect failed: {:02x}", hdr[1]),
        ));
    }
    match hdr[3] {
        0x01 => {
            let mut b = [0u8; 6];
            read_exact_t(&mut s, &mut b, d).await?;
        }
        0x04 => {
            let mut b = [0u8; 18];
            read_exact_t(&mut s, &mut b, d).await?;
        }
        0x03 => {
            let mut l = [0u8; 1];
            read_exact_t(&mut s, &mut l, d).await?;
            let mut b = vec![0u8; l[0] as usize + 2];
            read_exact_t(&mut s, &mut b, d).await?;
        }
        _ => return Err(Error::new(ErrorKind::InvalidData, "bad socks5 atyp")),
    }
    Ok(s)
}

pub(crate) async fn socks4_connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    t_host: &str,
    t_port: u16,
    user: Option<String>,
    allow_ipv6_domain: bool,
    d: Duration,
) -> std::io::Result<S> {
    let req = build_socks4_request(t_host, t_port, user.as_deref(), allow_ipv6_domain)?;
    write_all_t(&mut s, &req, d).await?;
    let mut resp = [0u8; 8];
    read_exact_t(&mut s, &mut resp, d).await?;
    if resp[1] != 90 {
        return Err(Error::new(
            ErrorKind::ConnectionRefused,
            format!("socks4 connect failed: {}", resp[1]),
        ));
    }
    Ok(s)
}

pub(crate) fn build_socks4_request(
    t_host: &str,
    t_port: u16,
    user: Option<&str>,
    allow_ipv6_domain: bool,
) -> std::io::Result<Vec<u8>> {
    let mut req = vec![0x04, 0x01];
    req.extend_from_slice(&t_port.to_be_bytes());
    let domain = match t_host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v)) => {
            req.extend_from_slice(&v.octets());
            None
        }
        Ok(IpAddr::V6(_)) if allow_ipv6_domain => {
            req.extend_from_slice(&[0, 0, 0, 1]);
            Some(t_host)
        }
        Ok(IpAddr::V6(_)) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "socks4 does not support IPv6; use socks5 or socks4a",
            ))
        }
        Err(_) => {
            req.extend_from_slice(&[0, 0, 0, 1]);
            Some(t_host)
        }
    };
    let uid = user.unwrap_or("");
    if uid.len() > 255 {
        return Err(Error::new(ErrorKind::InvalidInput, "socks4 user too long"));
    }
    req.extend_from_slice(uid.as_bytes());
    req.push(0x00);
    if let Some(dm) = domain {
        if dm.is_empty() || dm.len() > 255 {
            return Err(Error::new(ErrorKind::InvalidInput, "socks4 host too long"));
        }
        req.extend_from_slice(dm.as_bytes());
        req.push(0x00);
    }
    Ok(req)
}

fn basic_proxy_auth(user: Option<&str>, pass: Option<&str>) -> Option<String> {
    user.map(|u| {
        base64::engine::general_purpose::STANDARD.encode(format!(
            "{}:{}",
            u,
            pass.unwrap_or_default()
        ))
    })
}

pub(crate) fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

/// Remove one header from the header block, preserving the body. The client's
/// local-proxy credentials must never be forwarded to an upstream proxy or
/// to the origin.
pub(crate) fn strip_header(buf: &[u8], hdr_end: usize, name: &str) -> Vec<u8> {
    let end = hdr_end.min(buf.len());
    let head = &buf[..end];
    let mut out = Vec::with_capacity(buf.len());
    let mut start = 0;
    for (i, &b) in head.iter().enumerate() {
        if b != b'\n' {
            continue;
        }
        let line = &head[start..=i];
        let is_target = String::from_utf8_lossy(line)
            .split_once(':')
            .map(|(k, _)| k.trim().eq_ignore_ascii_case(name))
            .unwrap_or(false);
        if !is_target {
            out.extend_from_slice(line);
        }
        start = i + 1;
    }
    out.extend_from_slice(&head[start..]);
    out.extend_from_slice(&buf[end..]);
    out
}

/// The dispatcher rewrites only the first request on a client connection
/// (absolute-form, upstream credentials, routing by its host). Later
/// keep-alive requests would reach the upstream unrewritten: without
/// credentials, or through a SOCKS tunnel even to the first request's host.
/// So the far end is asked to close after this response and the client opens
/// a new connection for the next request. Upgrade requests keep their
/// `Connection` header: an upgraded connection stays with one host anyway.
pub(crate) fn close_after_response(buf: &[u8], hdr_end: usize) -> Vec<u8> {
    let end = hdr_end.min(buf.len());
    let has_upgrade = String::from_utf8_lossy(&buf[..end])
        .lines()
        .skip(1)
        .any(|l| {
            l.split_once(':')
                .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case("upgrade"))
        });
    if has_upgrade {
        return buf.to_vec();
    }
    let stripped = strip_header(buf, end, "connection");
    let Some(end) = find_header_end(&stripped) else {
        return stripped;
    };
    let mut out = Vec::with_capacity(stripped.len() + 19);
    out.extend_from_slice(&stripped[..end - 2]);
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    out.extend_from_slice(&stripped[end..]);
    out
}

/// For plain HTTP through an HTTP(S) proxy, the request must be sent in
/// absolute-form with Proxy-Authorization. Unlike CONNECT, this works with
/// proxies that only allow CONNECT to port 443.
pub(crate) fn prepare_forward_request(buf: &[u8], hdr_end: usize, up: &Upstream) -> Vec<u8> {
    // Drop headers that are hop-by-hop between the client and this listener.
    // Proxy-Authorization is meant for the local listener, not the upstream;
    // Proxy-Connection and Keep-Alive are equally meaningless to the origin.
    // Connection/Upgrade/Transfer-Encoding stay intact so upgrades and body
    // framing keep working. Then add this upstream's own credentials.
    let stripped = strip_header(buf, hdr_end, "proxy-authorization");
    let end = find_header_end(&stripped).unwrap_or(stripped.len());
    let stripped = strip_header(&stripped, end, "proxy-connection");
    let end = find_header_end(&stripped).unwrap_or(stripped.len());
    let stripped = strip_header(&stripped, end, "keep-alive");
    let Some(auth) = basic_proxy_auth(up.user.as_deref(), up.pass.as_deref()) else {
        return stripped;
    };
    let end = find_header_end(&stripped).unwrap_or(stripped.len());
    let head_end = end.saturating_sub(4);
    let mut out = Vec::with_capacity(stripped.len() + auth.len() + 48);
    out.extend_from_slice(&stripped[..head_end]);
    out.extend_from_slice(b"\r\nProxy-Authorization: Basic ");
    out.extend_from_slice(auth.as_bytes());
    out.extend_from_slice(b"\r\n\r\n");
    out.extend_from_slice(&stripped[end..]);
    out
}

pub(crate) async fn http_connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    t_host: &str,
    t_port: u16,
    user: Option<String>,
    pass: Option<String>,
    d: Duration,
) -> std::io::Result<S> {
    let host = host_for_connect(t_host);
    let mut req = format!("CONNECT {host}:{t_port} HTTP/1.1\r\nHost: {host}:{t_port}\r\n");
    if let Some(enc) = basic_proxy_auth(user.as_deref(), pass.as_deref()) {
        req.push_str(&format!("Proxy-Authorization: Basic {enc}\r\n"));
    }
    req.push_str("\r\n");
    write_all_t(&mut s, req.as_bytes(), d).await?;
    let deadline = Instant::now() + d;
    let mut buf = Vec::new();
    // Read exactly up to the end of the header. Reading a larger chunk can
    // consume bytes that the proxy already sent for the tunnel and lose them.
    let mut one = [0u8; 1];
    loop {
        loop {
            if buf.len() > 32768 {
                return Err(Error::new(ErrorKind::InvalidData, "proxy header too big"));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::new(ErrorKind::TimedOut, "proxy read timeout"));
            }
            let n = match tokio::time::timeout(remaining, s.read(&mut one)).await {
                Ok(Ok(0)) => return Err(Error::new(ErrorKind::UnexpectedEof, "proxy closed")),
                Ok(Ok(n)) => n,
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(Error::new(ErrorKind::TimedOut, "proxy read timeout")),
            };
            buf.extend_from_slice(&one[..n]);
            if buf.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&buf);
        let code = head
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .unwrap_or("");
        // A proxy may send interim 1xx responses (for example 100 Continue)
        // before the final 200; skip them and read the next response head.
        if code.len() == 3 && code.starts_with('1') {
            buf.clear();
            continue;
        }
        if code != "200" {
            return Err(Error::new(
                ErrorKind::ConnectionRefused,
                format!("http proxy: {code}"),
            ));
        }
        return Ok(s);
    }
}

async fn tls_to_proxy(
    stream: TcpStream,
    host: &str,
    d: Duration,
) -> std::io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    use rustls_pki_types::ServerName;
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "bad proxy tls name"))?;
    let connector = tokio_rustls::TlsConnector::from(tls_client_config());
    match tokio::time::timeout(d, connector.connect(name, stream)).await {
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => Err(Error::other(format!("proxy tls: {e}"))),
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "proxy tls timeout")),
    }
}

pub(crate) async fn connect_to_http_proxy(
    up: &Upstream,
    timeout: Duration,
) -> std::io::Result<ProxyStream> {
    let addr = format!("{}:{}", host_for_connect(&up.host), up.port);
    let stream = match tokio::time::timeout(timeout, TcpStream::connect(addr.as_str())).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(Error::new(ErrorKind::TimedOut, "upstream timeout")),
    };
    stream.set_nodelay(true).ok();
    if up.scheme == "https" {
        Ok(ProxyStream::Tls(Box::new(
            tls_to_proxy(stream, &up.host, timeout).await?,
        )))
    } else {
        Ok(ProxyStream::Plain(stream))
    }
}

/// socks4/socks5 resolve the destination locally; socks4a/socks5h send the
/// hostname to the proxy. This matches the checker (reqwest) and strict
/// SOCKS4 servers that do not implement the SOCKS4a extension.
pub(crate) async fn socks_target_host(
    scheme: &str,
    t_host: &str,
    t_port: u16,
    timeout: Duration,
) -> std::io::Result<String> {
    let local = matches!(scheme, "socks4" | "socks5");
    if !local || t_host.parse::<IpAddr>().is_ok() {
        return Ok(t_host.to_string());
    }
    match tokio::time::timeout(timeout, tokio::net::lookup_host((t_host, t_port))).await {
        Ok(Ok(mut addrs)) => addrs
            .next()
            .map(|a| a.ip().to_string())
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "socks target did not resolve")),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "socks dns timeout")),
    }
}

pub(crate) async fn connect_via(
    up: &Upstream,
    t_host: &str,
    t_port: u16,
    timeout: Duration,
) -> std::io::Result<ProxyStream> {
    let target = socks_target_host(&up.scheme, t_host, t_port, timeout).await?;
    let target = target.as_str();
    let addr = format!("{}:{}", host_for_connect(&up.host), up.port);
    let stream = match tokio::time::timeout(timeout, TcpStream::connect(addr.as_str())).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(Error::new(ErrorKind::TimedOut, "upstream timeout")),
    };
    stream.set_nodelay(true).ok();
    match up.scheme.as_str() {
        "socks5" | "socks5h" => {
            socks5_connect(
                ProxyStream::Plain(stream),
                target,
                t_port,
                up.user.clone(),
                up.pass.clone(),
                timeout,
            )
            .await
        }
        "socks4" => {
            socks4_connect(
                ProxyStream::Plain(stream),
                target,
                t_port,
                up.user.clone(),
                false,
                timeout,
            )
            .await
        }
        "socks4a" => {
            socks4_connect(
                ProxyStream::Plain(stream),
                target,
                t_port,
                up.user.clone(),
                true,
                timeout,
            )
            .await
        }
        "https" => {
            let tls = tls_to_proxy(stream, &up.host, timeout).await?;
            http_connect(
                ProxyStream::Tls(Box::new(tls)),
                t_host,
                t_port,
                up.user.clone(),
                up.pass.clone(),
                timeout,
            )
            .await
        }
        _ => {
            http_connect(
                ProxyStream::Plain(stream),
                t_host,
                t_port,
                up.user.clone(),
                up.pass.clone(),
                timeout,
            )
            .await
        }
    }
}
