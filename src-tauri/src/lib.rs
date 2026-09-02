use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyResult {
    pub proxy: String,
    pub proto: String,
    pub alive: bool,
    pub latency_ms: Option<u64>,
    pub ip: Option<String>,
    pub error: Option<String>,
}

fn proto_label(scheme: &str) -> String {
    match scheme.to_lowercase().as_str() {
        "http" | "https" => "HTTP".to_string(),
        "socks5" | "socks5h" => "SOCKS5".to_string(),
        "socks4" | "socks4a" => "SOCKS4".to_string(),
        other => other.to_uppercase(),
    }
}

/// Normalize user input into (full_url, proto_label).
/// Supports:
/// - 1.2.3.4:8080
/// - user:pass@1.2.3.4:8080
/// - http://1.2.3.4:8080, socks5://..., socks4://...
/// - 1.2.3.4:8080:user:pass (seller format) -> user:pass@host:port
fn normalize_proxy(raw: &str, default_proto: &str) -> Option<(String, String)> {
    let mut s = raw.trim().to_string();
    if s.is_empty() || s.starts_with('#') || s.starts_with("//") {
        return None;
    }
    // strip surrounding quotes/brackets noise
    s = s.trim_matches(|c| c == '"' || c == '\'' || c == ',' || c == ';').to_string();
    if s.is_empty() {
        return None;
    }

    let def = match default_proto.to_lowercase().as_str() {
        "socks5" => "socks5",
        "socks4" => "socks4",
        _ => "http",
    };

    // seller format host:port:user:pass (4 parts, no @, no ://)
    if !s.contains("://") && !s.contains('@') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 4 && !parts[0].is_empty() && !parts[1].is_empty() {
            let (host, port, user, pass) = (parts[0], parts[1], parts[2], parts[3]);
            if port.chars().all(|c| c.is_ascii_digit()) {
                let url = format!("{def}://{user}:{pass}@{host}:{port}");
                return Some((url, proto_label(def)));
            }
        }
    }

    let with_scheme = if s.contains("://") {
        s.clone()
    } else {
        format!("{def}://{s}")
    };

    let scheme = with_scheme
        .split("://")
        .next()
        .unwrap_or(def)
        .to_lowercase();
    // basic validation: must have host:port after scheme
    let after = with_scheme.split("://").nth(1).unwrap_or("");
    // strip userinfo for host check
    let hostport = after.rsplit('@').next().unwrap_or(after);
    if !hostport.contains(':') || hostport.len() < 3 {
        return None;
    }
    Some((with_scheme, proto_label(&scheme)))
}

fn short_error(e: &str) -> String {
    let low = e.to_lowercase();
    if low.contains("timed out") || low.contains("timeout") || low.contains("deadline") {
        return "timeout".to_string();
    }
    if low.contains("proxy authentication") || low.contains("407") {
        return "proxy auth required".to_string();
    }
    if low.contains("connection refused") {
        return "connection refused".to_string();
    }
    if low.contains("failed to lookup")
        || low.contains("dns")
        || low.contains("could not resolve")
    {
        return "dns error".to_string();
    }
    if low.contains("connection closed")
        || low.contains("connection reset")
        || low.contains("broken pipe")
    {
        return "connection reset".to_string();
    }
    // trim long reqwest debug strings
    let mut s = e.replace('\n', " ");
    if s.len() > 120 {
        s.truncate(120);
    }
    s.trim().to_string()
}

async fn check_one(
    raw: String,
    url: String,
    proto: String,
    test_url: String,
    timeout: Duration,
) -> ProxyResult {
    let proxy = match reqwest::Proxy::all(&url) {
        Ok(p) => p,
        Err(e) => {
            return ProxyResult {
                proxy: raw,
                proto,
                alive: false,
                latency_ms: None,
                ip: None,
                error: Some(short_error(&e.to_string())),
            }
        }
    };

    let client = match reqwest::Client::builder()
        .proxy(proxy)
        .timeout(timeout)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) ProxPulse/0.1")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ProxyResult {
                proxy: raw,
                proto,
                alive: false,
                latency_ms: None,
                ip: None,
                error: Some(short_error(&e.to_string())),
            }
        }
    };

    let start = Instant::now();
    let resp = client.get(&test_url).send().await;
    let latency = start.elapsed().as_millis() as u64;

    match resp {
        Ok(r) => {
            let status = r.status();
            if status.is_success() {
                let ip = match r.text().await {
                    Ok(t) => {
                        let t = t.trim().to_string();
                        if t.is_empty() || t.len() > 128 || t.contains('<') {
                            None
                        } else {
                            Some(t)
                        }
                    }
                    Err(_) => None,
                };
                ProxyResult {
                    proxy: raw,
                    proto,
                    alive: true,
                    latency_ms: Some(latency),
                    ip,
                    error: None,
                }
            } else {
                ProxyResult {
                    proxy: raw,
                    proto,
                    alive: false,
                    latency_ms: Some(latency),
                    ip: None,
                    error: Some(format!("http {}", status.as_u16())),
                }
            }
        }
        Err(e) => ProxyResult {
            proxy: raw,
            proto,
            alive: false,
            latency_ms: None,
            ip: None,
            error: Some(short_error(&e.to_string())),
        },
    }
}

#[tauri::command]
async fn check_proxies(
    proxies: Vec<String>,
    test_url: String,
    timeout_ms: u64,
    concurrency: usize,
    default_proto: String,
) -> Result<Vec<ProxyResult>, String> {
    let test_url = test_url.trim().to_string();
    if test_url.is_empty() {
        return Err("empty test url".to_string());
    }
    if !(test_url.starts_with("http://") || test_url.starts_with("https://")) {
        return Err("test url must start with http(s)://".to_string());
    }

    let timeout = Duration::from_millis(timeout_ms.clamp(500, 60_000));
    let conc = concurrency.clamp(1, 500);

    // normalize + dedup keep order
    let mut jobs: Vec<(String, String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in proxies {
        let t = raw.trim().to_string();
        if t.is_empty() || !seen.insert(t.clone()) {
            continue;
        }
        if let Some((url, proto)) = normalize_proxy(&t, &default_proto) {
            jobs.push((t, url, proto));
        } else {
            // invalid format -> immediate dead entry handled below
            jobs.push((t, String::new(), "BAD".to_string()));
        }
        if jobs.len() >= 20_000 {
            break;
        }
    }

    let sem = Arc::new(Semaphore::new(conc));
    let mut set = tokio::task::JoinSet::new();

    for (raw, url, proto) in jobs {
        if url.is_empty() {
            let r = ProxyResult {
                proxy: raw,
                proto: "BAD".to_string(),
                alive: false,
                latency_ms: None,
                ip: None,
                error: Some("bad format".to_string()),
            };
            // push directly via completed task
            set.spawn(async move { r });
            continue;
        }
        let permit_sem = sem.clone();
        let tu = test_url.clone();
        set.spawn(async move {
            let _p = permit_sem.acquire_owned().await.unwrap();
            check_one(raw, url, proto, tu, timeout).await
        });
    }

    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        match r {
            Ok(v) => out.push(v),
            Err(e) => out.push(ProxyResult {
                proxy: "?".to_string(),
                proto: "?".to_string(),
                alive: false,
                latency_ms: None,
                ip: None,
                error: Some(short_error(&e.to_string())),
            }),
        }
    }
    // sort: alive first, then by latency
    out.sort_by(|a, b| {
        b.alive
            .cmp(&a.alive)
            .then(a.latency_ms.unwrap_or(u64::MAX).cmp(&b.latency_ms.unwrap_or(u64::MAX)))
    });
    Ok(out)
}

#[tauri::command]
async fn check_direct(test_url: String, timeout_ms: u64) -> Result<u64, String> {
    let timeout = Duration::from_millis(timeout_ms.clamp(500, 30_000));
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) ProxPulse/0.1")
        .build()
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let r = client
        .get(test_url.trim())
        .send()
        .await
        .map_err(|e| short_error(&e.to_string()))?;
    if !r.status().is_success() {
        return Err(format!("http {}", r.status().as_u16()));
    }
    Ok(start.elapsed().as_millis() as u64)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![check_proxies, check_direct])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
