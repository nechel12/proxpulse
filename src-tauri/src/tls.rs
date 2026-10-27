use crate::http::short_error;
use crate::transport::{connect_via, parse_upstream_url, Upstream};
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

// ================= TLS fingerprint =================

static TLS_CFG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();

pub(crate) fn tls_client_config() -> Arc<rustls::ClientConfig> {
    TLS_CFG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            // NOTE: provider is pinned explicitly. Several rustls providers
            // (ring + aws-lc-rs) are compiled into the dependency tree, so
            // the feature-based auto-detect in ClientConfig::builder()
            // panics — see CryptoProvider docs.
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            Arc::new(
                rustls::ClientConfig::builder_with_provider(provider)
                    .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                    .expect("ring supports TLS 1.2/1.3")
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

fn cert_fp_hex(der: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(der);
    format!("{:x}", h.finalize())
}

async fn tls_handshake_fp<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    timeout: Duration,
) -> std::io::Result<(String, String)> {
    use rustls_pki_types::ServerName;
    let name = ServerName::try_from("example.com")
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "bad tls name"))?
        .to_owned();
    let connector = tokio_rustls::TlsConnector::from(tls_client_config());
    let tls = match tokio::time::timeout(timeout, connector.connect(name, stream)).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(Error::new(ErrorKind::TimedOut, "tls handshake timeout")),
    };
    let (_io, conn) = tls.get_ref();
    let certs = conn
        .peer_certificates()
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "no peer certificate"))?;
    let leaf = certs
        .first()
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "empty certificate chain"))?;
    let fp = cert_fp_hex(leaf.as_ref());
    let ver = match conn.protocol_version() {
        Some(rustls::ProtocolVersion::TLSv1_3) => "TLS 1.3",
        Some(rustls::ProtocolVersion::TLSv1_2) => "TLS 1.2",
        _ => "TLS ?",
    };
    let cipher = conn
        .negotiated_cipher_suite()
        .map(|s| format!("{:?}", s.suite()))
        .unwrap_or_else(|| "?".to_string());
    let short = fp.chars().take(12).collect::<String>();
    Ok((fp, format!("{ver} {cipher} {short}")))
}

pub(crate) fn is_cert_error(e: &std::io::Error) -> bool {
    let s = e.to_string().to_ascii_lowercase();
    s.contains("certificate") || s.contains("unknownissuer") || s.contains("invalidcertificate")
}

pub(crate) async fn tls_direct_fp() -> Option<String> {
    let timeout = Duration::from_millis(9000);
    let stream = match tokio::time::timeout(timeout, TcpStream::connect(("example.com", 443))).await
    {
        Ok(Ok(s)) => s,
        _ => return None,
    };
    tls_handshake_fp(stream, timeout)
        .await
        .ok()
        .map(|(fp, _)| fp)
}

pub(crate) async fn tls_via_proxy(
    proxy_url: &str,
    timeout: Duration,
    base_fp: Option<&str>,
) -> (Option<String>, Option<String>) {
    let (scheme, host, port, user, pass) = match parse_upstream_url(proxy_url) {
        Some(v) => v,
        None => return (None, None),
    };
    let up = Upstream {
        raw: proxy_url.to_string(),
        url: proxy_url.to_string(),
        scheme,
        host,
        port,
        user,
        pass,
        latency: None,
        country: None,
    };
    let stream = match connect_via(&up, "example.com", 443, timeout).await {
        Ok(s) => s,
        Err(_) => return (None, None),
    };
    match tls_handshake_fp(stream, timeout).await {
        Ok((fp, info)) => match base_fp {
            Some(b) if fp == b => (Some("ok".to_string()), Some(info)),
            Some(_) => (Some("modified".to_string()), Some(info)),
            None => (None, Some(info)),
        },
        Err(e) if is_cert_error(&e) => (
            Some("modified".to_string()),
            Some(format!(
                "certificate rejected: {}",
                short_error(&e.to_string())
            )),
        ),
        // A failed handshake is not proof of tampering (the proxy may block
        // example.com:443), but it must not look like a successful check.
        Err(e) => (
            Some("unverified".to_string()),
            Some(format!("handshake failed: {}", short_error(&e.to_string()))),
        ),
    }
}
