//! Local pool API: a small HTTP server on 127.0.0.1 that hands live proxies
//! to scripts, e.g. `GET /random?country=DE&pool=work`.

use crate::commands::dispatch::{basic_auth_ok, country_matches, DispatchInner, DispatchState};
use crate::commands::pools::{PoolMeta, PoolsState};
use crate::commands::prefs::{update_prefs, AppPrefsUpdate, PrefsState};
use crate::transport::Upstream;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Semaphore};

pub(crate) const DEFAULT_API_PORT: u16 = 1090;
const HEAD_MAX: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CLIENTS: usize = 64;
/// Same cap as the pool.
const LIMIT_MAX: usize = 20_000;

const HELP: &str = "ProxPulse pool API\n\
GET /random    one live proxy\n\
GET /fastest   the proxy with the lowest ping\n\
GET /list      every matching proxy\n\
GET /pools     the pools and their sizes\n\
Parameters: pool=<id or name> country=DE,NL proto=http,socks5 limit=N format=txt|json\n";

/// The API's state in the preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct PoolApiPrefs {
    /// Runs whenever the app runs.
    pub(crate) enabled: bool,
    pub(crate) port: u16,
}

impl Default for PoolApiPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_API_PORT,
        }
    }
}

impl PoolApiPrefs {
    pub(crate) fn sanitized(mut self) -> Self {
        if self.port == 0 {
            self.port = DEFAULT_API_PORT;
        }
        self
    }
}

#[derive(Default)]
pub(crate) struct PoolApiState {
    server: Mutex<Option<(u16, tokio::task::JoinHandle<()>)>>,
    last_error: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PoolApiStatus {
    running: bool,
    port: u16,
    last_error: Option<String>,
}

/// Every pool with its name, read for each request.
pub(crate) type PoolSource = Arc<dyn Fn() -> Vec<(PoolMeta, Arc<DispatchInner>)> + Send + Sync>;

#[derive(Debug, Default, PartialEq)]
pub(crate) struct ApiQuery {
    pub(crate) pool: Option<String>,
    pub(crate) countries: Vec<String>,
    pub(crate) protos: Vec<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) json: bool,
}

/// Comma- or semicolon-separated; spaces stay inside an entry, since country
/// names such as "United States" contain them.
fn split_list(v: &str) -> Vec<String> {
    v.split([',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(50)
        .collect()
}

/// The path and parameters of a request target (`/random?country=DE`).
pub(crate) fn parse_target(target: &str) -> Result<(String, ApiQuery), String> {
    let url = reqwest::Url::parse(&format!("http://localhost{target}"))
        .map_err(|_| "bad request target".to_string())?;
    let mut q = ApiQuery::default();
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "pool" => q.pool = Some(v.trim().to_string()).filter(|s| !s.is_empty()),
            "country" => q.countries = split_list(&v),
            "proto" => {
                q.protos = split_list(&v)
                    .iter()
                    .map(|p| p.to_ascii_lowercase())
                    .collect()
            }
            "limit" => {
                let n: usize = v
                    .trim()
                    .parse()
                    .map_err(|_| "limit must be a number".to_string())?;
                q.limit = Some(n.clamp(1, LIMIT_MAX));
            }
            "format" => match v.as_ref() {
                "json" => q.json = true,
                "txt" | "text" => q.json = false,
                _ => return Err("format must be txt or json".to_string()),
            },
            _ => {}
        }
    }
    Ok((url.path().to_string(), q))
}

/// `socks5h` and `socks4a` count as `socks5` and `socks4`; `socks` is either.
fn proto_matches(up: &Upstream, protos: &[String]) -> bool {
    let scheme = up.scheme.to_ascii_lowercase();
    let base = match scheme.as_str() {
        "socks5h" => "socks5",
        "socks4a" => "socks4",
        s => s,
    };
    protos.is_empty()
        || protos
            .iter()
            .any(|p| p == base || (p == "socks" && base.starts_with("socks")))
}

/// The matching proxies, without those in cooldown after a failure unless
/// no other is left.
pub(crate) fn select(pool: &[Upstream], q: &ApiQuery, cooling: &HashSet<String>) -> Vec<Upstream> {
    let allowed: Vec<&Upstream> = pool
        .iter()
        .filter(|u| country_matches(u, &q.countries) && proto_matches(u, &q.protos))
        .collect();
    let ready: Vec<&Upstream> = allowed
        .iter()
        .copied()
        .filter(|u| !cooling.contains(&u.url))
        .collect();
    let list = if ready.is_empty() { allowed } else { ready };
    list.into_iter().cloned().collect()
}

/// A pool by id or by name (any case); the main pool without `pool=`.
pub(crate) fn find_pool<'a, T>(
    pools: &'a [(PoolMeta, T)],
    key: Option<&str>,
) -> Option<&'a (PoolMeta, T)> {
    let key = key.unwrap_or(crate::commands::dispatch::MAIN_POOL);
    pools.iter().find(|(m, _)| m.id == key).or_else(|| {
        let key = key.to_lowercase();
        pools
            .iter()
            .find(|(m, _)| !m.name.is_empty() && m.name.to_lowercase() == key)
    })
}

/// Only local names: a page that rebinds its own domain to 127.0.0.1 must
/// not read the pool.
pub(crate) fn host_allowed(head: &str) -> bool {
    let Some(host) = header(head, "host") else {
        return true; // HTTP/1.0 clients; browsers always send Host
    };
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']')
            .next()
            .map(|h| format!("[{h}]"))
            .unwrap_or_default()
    } else {
        host.split(':').next().unwrap_or("").to_string()
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "[::1]"
    )
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.split("\r\n").skip(1).find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

fn random_index(len: usize) -> usize {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 8];
    if ring::rand::SystemRandom::new().fill(&mut b).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or_default();
        return nanos as usize % len;
    }
    (u64::from_le_bytes(b) % len as u64) as usize
}

#[derive(Serialize)]
struct ProxyOut<'a> {
    url: &'a str,
    scheme: &'a str,
    host: &'a str,
    port: u16,
    country: Option<&'a str>,
    latency: Option<u64>,
}

fn proxy_json(up: &Upstream) -> serde_json::Value {
    serde_json::to_value(ProxyOut {
        url: &up.url,
        scheme: &up.scheme,
        host: &up.host,
        port: up.port,
        country: up.country.as_deref(),
        latency: up.latency,
    })
    .unwrap_or_default()
}

struct Reply {
    status: &'static str,
    content_type: &'static str,
    body: String,
    extra: &'static str,
}

impl Reply {
    fn text(status: &'static str, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.into(),
            extra: "",
        }
    }

    fn error(status: &'static str, msg: &str, json: bool) -> Self {
        if json {
            Self {
                content_type: "application/json",
                ..Self::text(status, serde_json::json!({ "error": msg }).to_string())
            }
        } else {
            Self::text(status, format!("{msg}\n"))
        }
    }

    fn json(value: serde_json::Value) -> Self {
        Self {
            content_type: "application/json",
            ..Self::text("200 OK", value.to_string())
        }
    }

    fn bytes(&self) -> Vec<u8> {
        format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n{}Connection: close\r\n\r\n{}",
            self.status,
            self.content_type,
            self.body.len(),
            self.extra,
            self.body
        )
        .into_bytes()
    }
}

async fn cooling(inner: &DispatchInner) -> HashSet<String> {
    let now = Instant::now();
    let mut c = inner.cooldown.lock().await;
    c.retain(|_, until| *until > now);
    c.keys().cloned().collect()
}

/// A pool with a dispatcher login is listed and hands out its proxies only
/// with the same login.
async fn may_read(head: &str, inner: &DispatchInner) -> bool {
    let auth = inner.options.lock().await;
    auth.auth_user.is_empty()
        || basic_auth_ok(head, "authorization", &auth.auth_user, &auth.auth_pass)
}

/// The answer to one request head.
pub(crate) async fn answer(head: &str, source: &PoolSource) -> Vec<u8> {
    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    if !host_allowed(head) {
        return Reply::text("403 Forbidden", "only 127.0.0.1 or localhost\n").bytes();
    }
    if method != "GET" {
        return Reply::text("405 Method Not Allowed", "use GET\n").bytes();
    }
    let (path, q) = match parse_target(target) {
        Ok(v) => v,
        Err(e) => return Reply::error("400 Bad Request", &e, false).bytes(),
    };
    let pools = source();
    if path == "/" {
        return Reply::text("200 OK", HELP).bytes();
    }
    if path == "/pools" {
        let mut rows = Vec::new();
        for (meta, inner) in &pools {
            if may_read(head, inner).await {
                rows.push((meta.clone(), inner.pool.lock().await.len()));
            }
        }
        return if q.json {
            Reply::json(serde_json::Value::Array(
                rows.iter()
                    .map(|(m, n)| serde_json::json!({ "id": m.id, "name": m.name, "count": n }))
                    .collect(),
            ))
        } else {
            Reply::text(
                "200 OK",
                rows.iter()
                    .map(|(m, n)| format!("{}\t{}\t{}\n", m.id, m.name, n))
                    .collect::<String>(),
            )
        }
        .bytes();
    }
    if !matches!(path.as_str(), "/random" | "/fastest" | "/list") {
        return Reply::error("404 Not Found", "unknown path; see /", q.json).bytes();
    }
    let Some((_, inner)) = find_pool(&pools, q.pool.as_deref()) else {
        return Reply::error("404 Not Found", "unknown pool", q.json).bytes();
    };
    if !may_read(head, inner).await {
        let mut r = Reply::error("401 Unauthorized", "login required", q.json);
        r.extra = "WWW-Authenticate: Basic realm=\"ProxPulse\"\r\n";
        return r.bytes();
    }
    let cooling = cooling(inner).await;
    let mut list = select(&inner.pool.lock().await, &q, &cooling);
    if list.is_empty() {
        return Reply::error("404 Not Found", "no proxy matches", q.json).bytes();
    }
    match path.as_str() {
        "/random" => {
            let up = list.swap_remove(random_index(list.len()));
            list = vec![up];
        }
        "/fastest" => {
            let up = list
                .iter()
                .min_by_key(|u| u.latency.unwrap_or(u64::MAX))
                .cloned()
                .unwrap_or_else(|| list[0].clone());
            list = vec![up];
        }
        _ => list.truncate(q.limit.unwrap_or(LIMIT_MAX)),
    }
    if q.json {
        if path == "/list" {
            Reply::json(serde_json::Value::Array(
                list.iter().map(proxy_json).collect(),
            ))
        } else {
            Reply::json(proxy_json(&list[0]))
        }
    } else {
        Reply::text(
            "200 OK",
            list.iter()
                .map(|u| format!("{}\n", u.url))
                .collect::<String>(),
        )
    }
    .bytes()
}

async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        if buf.len() > HEAD_MAX {
            return None;
        }
        let n = stream.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

async fn handle(mut stream: TcpStream, source: PoolSource) {
    let Ok(Some(head)) = tokio::time::timeout(READ_TIMEOUT, read_head(&mut stream)).await else {
        return;
    };
    let reply = answer(&head, &source).await;
    let _ = stream.write_all(&reply).await;
    let _ = stream.shutdown().await;
}

/// Serve until the task is aborted; dropping the set ends open requests.
pub(crate) async fn serve(listener: TcpListener, source: PoolSource) {
    let sem = Arc::new(Semaphore::new(MAX_CLIENTS));
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        let stream = match listener.accept().await {
            Ok((s, _)) => s,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let Ok(permit) = sem.clone().try_acquire_owned() else {
            continue; // busy: drop the connection
        };
        let source = source.clone();
        tasks.spawn(async move {
            let _permit = permit;
            handle(stream, source).await;
        });
        while tasks.try_join_next().is_some() {}
    }
}

fn app_source(app: &AppHandle) -> PoolSource {
    let app = app.clone();
    Arc::new(move || {
        let metas = app.state::<PoolsState>().snapshot();
        let disp = app.state::<DispatchState>();
        metas
            .into_iter()
            .filter_map(|m| disp.pool(Some(&m.id)).ok().map(|p| (m, p)))
            .collect()
    })
}

async fn start_api(app: &AppHandle, port: u16) -> Result<u16, String> {
    let state = app.state::<PoolApiState>();
    let mut server = state.server.lock().await;
    if let Some((_, h)) = server.take() {
        h.abort();
        // Wait until the old listener is gone, or restarting on the same
        // port fails with "address in use".
        let _ = h.await;
    }
    let result = TcpListener::bind(("127.0.0.1", port)).await;
    let listener = match result {
        Ok(l) => l,
        Err(e) => {
            let msg = format!("127.0.0.1:{port}: {e}");
            *state.last_error.lock().await = Some(msg.clone());
            return Err(msg);
        }
    };
    let bound = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    *server = Some((bound, tokio::spawn(serve(listener, app_source(app)))));
    *state.last_error.lock().await = None;
    Ok(bound)
}

async fn remember(app: &AppHandle, enabled: bool, port: u16) {
    let update = AppPrefsUpdate {
        pool_api: Some(PoolApiPrefs { enabled, port }),
        ..AppPrefsUpdate::default()
    };
    let _ = update_prefs(app, update).await;
}

/// Start the API at launch when it was on.
pub(crate) async fn start_saved_pool_api(app: AppHandle) {
    let prefs = app.state::<PrefsState>().get().pool_api;
    if prefs.enabled {
        let _ = start_api(&app, prefs.port).await;
    }
}

#[tauri::command]
pub(crate) async fn start_pool_api(app: AppHandle, port: u16) -> Result<String, String> {
    if port == 0 {
        return Err("port must be 1-65535".to_string());
    }
    let bound = start_api(&app, port).await?;
    remember(&app, true, bound).await;
    Ok(format!("127.0.0.1:{bound}"))
}

#[tauri::command]
pub(crate) async fn stop_pool_api(app: AppHandle) -> Result<(), String> {
    let state = app.state::<PoolApiState>();
    let old = state.server.lock().await.take();
    if let Some((_, h)) = old {
        h.abort();
        let _ = h.await;
    }
    let port = app.state::<PrefsState>().get().pool_api.port;
    remember(&app, false, port).await;
    Ok(())
}

#[tauri::command]
pub(crate) async fn pool_api_status(
    state: State<'_, PoolApiState>,
    prefs: State<'_, PrefsState>,
) -> Result<PoolApiStatus, String> {
    let server = state.server.lock().await;
    Ok(PoolApiStatus {
        running: server.is_some(),
        port: server
            .as_ref()
            .map(|(p, _)| *p)
            .unwrap_or_else(|| prefs.get().pool_api.port),
        last_error: state.last_error.lock().await.clone(),
    })
}
