use crate::commands::dispatch::DispatchItem;
use crate::http::{read_body_capped, request_error, short_error, BODY_LIMIT_WEBHOOK};
use crate::netguard::build_guarded_client;
use crate::{epoch_secs, is_http_url, strip_credentials, WebhookResult};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Formats a webhook can be sent in.
pub(crate) const WEBHOOK_FORMATS: [&str; 6] =
    ["txt", "json", "csv", "telegram", "discord", "template"];
/// Longest custom body template accepted.
pub(crate) const TEMPLATE_MAX: usize = 4000;
/// Waits before the second and third attempt of a scheduled delivery.
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(5), Duration::from_secs(20)];
const TELEGRAM_MAX: usize = 4096;
const DISCORD_MAX: usize = 2000;

/// How a webhook body is built.
#[derive(Debug, Clone, Default)]
pub(crate) struct WebhookOptions {
    pub(crate) format: String,
    pub(crate) include_credentials: bool,
    /// Body for the "template" format.
    pub(crate) template: String,
}

/// Additions and removals since the last delivery to the same URL.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Changes {
    pub(crate) added: Vec<String>,
    pub(crate) removed: Vec<String>,
}

impl Changes {
    pub(crate) fn between(prev: &HashSet<String>, now: &[String]) -> Self {
        let current: HashSet<&String> = now.iter().collect();
        Self {
            added: now.iter().filter(|p| !prev.contains(*p)).cloned().collect(),
            removed: {
                let mut r: Vec<String> = prev
                    .iter()
                    .filter(|p| !current.contains(p))
                    .cloned()
                    .collect();
                r.sort();
                r
            },
        }
    }

    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// The lists last delivered to each webhook URL, for "changes only".
pub(crate) type SentLists = Mutex<HashMap<String, HashSet<String>>>;

fn webhook_proxies(items: &[DispatchItem], include_credentials: bool) -> Vec<String> {
    items
        .iter()
        .map(|i| {
            let source = i
                .url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(&i.raw);
            if include_credentials {
                source.to_string()
            } else {
                strip_credentials(source)
            }
        })
        .collect()
}

fn csv_cell(s: &str) -> String {
    let guarded = if s
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r'))
    {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

/// Chat message text that fits the service limit; the rest is counted.
fn chat_text(header: &str, lines: &[String], max: usize) -> String {
    let mut out = header.to_string();
    for (n, line) in lines.iter().enumerate() {
        let rest = lines.len() - n;
        // Room for the "… N more" tail.
        if out.len() + line.len() + 1 > max.saturating_sub(24) {
            out.push_str(&format!("\n… {rest} more"));
            break;
        }
        out.push('\n');
        out.push_str(line);
    }
    out
}

/// UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
fn utc_timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn json_list(items: &[String]) -> String {
    serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string())
}

/// The request body and its content type. With `changes`, list formats send
/// only what was added and removed since the last delivery.
pub(crate) fn build_body(
    opts: &WebhookOptions,
    items: &[DispatchItem],
    changes: Option<&Changes>,
    now: u64,
) -> Result<(String, &'static str), String> {
    let proxies = webhook_proxies(items, opts.include_credentials);
    let latency: HashMap<&str, Option<u64>> = proxies
        .iter()
        .zip(items)
        .map(|(p, i)| (p.as_str(), i.latency))
        .collect();
    let ms = |p: &str| latency.get(p).copied().flatten();
    let body = match (opts.format.as_str(), changes) {
        ("json", None) => {
            let arr: Vec<serde_json::Value> = proxies
                .iter()
                .map(|p| serde_json::json!({"proxy": p, "latency_ms": ms(p)}))
                .collect();
            (
                serde_json::to_string(&arr).map_err(|e| e.to_string())?,
                "application/json",
            )
        }
        ("json", Some(c)) => {
            let added: Vec<serde_json::Value> = c
                .added
                .iter()
                .map(|p| serde_json::json!({"proxy": p, "latency_ms": ms(p)}))
                .collect();
            let v = serde_json::json!({"added": added, "removed": c.removed});
            (v.to_string(), "application/json")
        }
        ("csv", None) => {
            let mut s = String::from("proxy,latency_ms\n");
            for p in &proxies {
                let l = ms(p).map(|v| v.to_string()).unwrap_or_default();
                s.push_str(&format!("{},{l}\n", csv_cell(p)));
            }
            (s, "text/csv")
        }
        ("csv", Some(c)) => {
            let mut s = String::from("change,proxy,latency_ms\n");
            for p in &c.added {
                let l = ms(p).map(|v| v.to_string()).unwrap_or_default();
                s.push_str(&format!("added,{},{l}\n", csv_cell(p)));
            }
            for p in &c.removed {
                s.push_str(&format!("removed,{},\n", csv_cell(p)));
            }
            (s, "text/csv")
        }
        ("telegram" | "discord", _) => {
            let (header, lines) = match changes {
                None => (
                    format!("ProxPulse: {} proxies", proxies.len()),
                    proxies.clone(),
                ),
                Some(c) => (
                    format!("ProxPulse: +{} / -{}", c.added.len(), c.removed.len()),
                    c.added
                        .iter()
                        .map(|p| format!("+ {p}"))
                        .chain(c.removed.iter().map(|p| format!("- {p}")))
                        .collect(),
                ),
            };
            let v = if opts.format == "telegram" {
                serde_json::json!({
                    "text": chat_text(&header, &lines, TELEGRAM_MAX),
                    "disable_web_page_preview": true,
                })
            } else {
                serde_json::json!({"content": chat_text(&header, &lines, DISCORD_MAX)})
            };
            (v.to_string(), "application/json")
        }
        ("template", _) => {
            let (added, removed) = match changes {
                Some(c) => (c.added.clone(), c.removed.clone()),
                None => (proxies.clone(), Vec::new()),
            };
            let body = opts
                .template
                .replace("{count}", &proxies.len().to_string())
                .replace("{time}", &utc_timestamp(now))
                .replace("{proxies_json}", &json_list(&proxies))
                .replace("{added_json}", &json_list(&added))
                .replace("{removed_json}", &json_list(&removed))
                .replace("{added_count}", &added.len().to_string())
                .replace("{removed_count}", &removed.len().to_string())
                .replace("{proxies}", &proxies.join("\n"))
                .replace("{added}", &added.join("\n"))
                .replace("{removed}", &removed.join("\n"));
            // JSON only when the filled-in body is JSON: "{count} proxies
            // online" starts with a brace too, and was refused as bad JSON.
            let ctype = if serde_json::from_str::<serde_json::Value>(&body).is_ok() {
                "application/json"
            } else {
                "text/plain"
            };
            (body, ctype)
        }
        (_, None) => (proxies.join("\n"), "text/plain"),
        (_, Some(c)) => {
            let lines: Vec<String> = c
                .added
                .iter()
                .map(|p| format!("+{p}"))
                .chain(c.removed.iter().map(|p| format!("-{p}")))
                .collect();
            (lines.join("\n"), "text/plain")
        }
    };
    Ok(body)
}

async fn send_webhook_request(
    client: &reqwest::Client,
    url: &str,
    opts: &WebhookOptions,
    items: &[DispatchItem],
    changes: Option<&Changes>,
) -> Result<String, String> {
    let (body, ctype) = build_body(opts, items, changes, epoch_secs())?;
    let r = client
        .post(url)
        .header("Content-Type", ctype)
        .body(body)
        .send()
        .await
        .map_err(|e| request_error(&e))?;
    let status = r.status();
    let txt = read_body_capped(r, BODY_LIMIT_WEBHOOK)
        .await
        .unwrap_or_default();
    let short: String = txt.chars().take(80).collect();
    let message = format!("http {} {short}", status.as_u16());
    // A 4xx/5xx (or an unfollowed redirect) means the receiver did not
    // accept the list; it must not be reported as delivered.
    if status.is_success() {
        Ok(message)
    } else {
        Err(message.trim_end().to_string())
    }
}

async fn send_webhook_one(
    url: &str,
    opts: &WebhookOptions,
    items: &[DispatchItem],
    changes: Option<&Changes>,
) -> Result<String, String> {
    let url = url.trim().to_string();
    if !is_http_url(&url) {
        return Err("bad webhook url".to_string());
    }
    let parsed = reqwest::Url::parse(&url).map_err(|_| "bad webhook url".to_string())?;
    if opts.include_credentials && parsed.scheme() == "http" {
        return Err("https required when sending proxy credentials".to_string());
    }
    // Webhook URLs are user-supplied but must not reach local services.
    let client = build_guarded_client(&parsed, Duration::from_secs(15)).await?;
    send_webhook_request(&client, &url, opts, items, changes).await
}

/// Server errors, rate limits and network failures are worth another try;
/// a rejected request or a bad URL is not.
pub(crate) fn retryable(err: &str) -> bool {
    if let Some(code) = err
        .strip_prefix("http ")
        .and_then(|s| s.get(..3))
        .and_then(|c| c.parse::<u16>().ok())
    {
        return code == 429 || code >= 500;
    }
    !matches!(err, "bad webhook url" | "private address") && !err.starts_with("https required")
}

async fn send_with_retries(
    url: &str,
    opts: &WebhookOptions,
    items: &[DispatchItem],
    changes: Option<&Changes>,
    delays: &[Duration],
) -> Result<String, String> {
    let mut attempt = 0;
    loop {
        match send_webhook_one(url, opts, items, changes).await {
            Ok(m) if attempt == 0 => return Ok(m),
            Ok(m) => return Ok(format!("{m} (attempt {})", attempt + 1)),
            Err(e) if attempt < delays.len() && retryable(&e) => {
                tokio::time::sleep(delays[attempt]).await;
                attempt += 1;
            }
            Err(e) if attempt == 0 => return Err(e),
            Err(e) => return Err(format!("{e} (after {} attempts)", attempt + 1)),
        }
    }
}

/// Scheduled delivery: retries, and with `changes_only` sends only the
/// difference from the last list delivered to each URL.
pub(crate) struct Delivery<'a> {
    pub(crate) retry: bool,
    pub(crate) changes_only: bool,
    pub(crate) sent: Option<&'a SentLists>,
}

pub(crate) async fn send_webhooks_inner(
    urls: &[String],
    opts: &WebhookOptions,
    items: Arc<Vec<DispatchItem>>,
    delivery: Delivery<'_>,
) -> Vec<WebhookResult> {
    let current = webhook_proxies(&items, opts.include_credentials);
    let mut set = tokio::task::JoinSet::new();
    for (index, url) in urls.iter().take(10).cloned().enumerate() {
        let opts = opts.clone();
        let items = items.clone();
        let prev = match delivery.sent {
            Some(sent) => sent.lock().await.get(url.trim()).cloned(),
            None => None,
        };
        let changes = (delivery.changes_only)
            .then(|| Changes::between(&prev.clone().unwrap_or_default(), &current));
        let retry = delivery.retry;
        set.spawn(async move {
            if prev.is_some() && changes.as_ref().is_some_and(Changes::is_empty) {
                return (index, url, Ok("no changes".to_string()));
            }
            let delays: &[Duration] = if retry { &RETRY_DELAYS } else { &[] };
            let res = send_with_retries(&url, &opts, &items, changes.as_ref(), delays).await;
            (index, url, res)
        });
    }
    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        match r {
            Ok((index, url, res)) => {
                if res.is_ok() {
                    if let Some(sent) = delivery.sent {
                        let list = current.iter().cloned().collect();
                        sent.lock().await.insert(url.trim().to_string(), list);
                    }
                }
                let (ok, message) = match res {
                    Ok(m) => (true, m),
                    Err(m) => (false, m),
                };
                out.push(WebhookResult { index, ok, message });
            }
            Err(e) => out.push(WebhookResult {
                index: usize::MAX,
                ok: false,
                message: short_error(&e.to_string()),
            }),
        }
    }
    out.sort_by_key(|r| r.index);
    out
}

fn options(
    format: String,
    include_credentials: Option<bool>,
    template: Option<String>,
) -> WebhookOptions {
    let mut template = template.unwrap_or_default();
    crate::http::truncate_utf8(&mut template, TEMPLATE_MAX);
    WebhookOptions {
        format: if WEBHOOK_FORMATS.contains(&format.as_str()) {
            format
        } else {
            "txt".to_string()
        },
        include_credentials: include_credentials.unwrap_or(false),
        template,
    }
}

#[tauri::command]
pub(crate) async fn send_webhooks(
    urls: Vec<String>,
    format: String,
    include_credentials: Option<bool>,
    template: Option<String>,
    items: Vec<DispatchItem>,
) -> Result<Vec<WebhookResult>, String> {
    let opts = options(format, include_credentials, template);
    let delivery = Delivery {
        retry: false,
        changes_only: false,
        sent: None,
    };
    Ok(send_webhooks_inner(&urls, &opts, Arc::new(items), delivery).await)
}

#[tauri::command]
pub(crate) async fn send_webhook(
    url: String,
    format: String,
    include_credentials: Option<bool>,
    template: Option<String>,
    items: Vec<DispatchItem>,
) -> Result<String, String> {
    let opts = options(format, include_credentials, template);
    send_webhook_one(&url, &opts, &items, None).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netguard::guarded_client_builder;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn opts(format: &str) -> WebhookOptions {
        WebhookOptions {
            format: format.to_string(),
            ..WebhookOptions::default()
        }
    }

    fn item(url: &str, latency: Option<u64>) -> DispatchItem {
        DispatchItem {
            raw: url.to_string(),
            url: Some(url.to_string()),
            latency,
            country: None,
        }
    }

    #[test]
    fn webhook_bodies_for_each_format() {
        let items = vec![
            item("http://u:p@a:80", Some(10)),
            item("socks5://b:1080", None),
        ];
        let (txt, ctype) = build_body(&opts("txt"), &items, None, 0).unwrap();
        assert_eq!(
            (txt.as_str(), ctype),
            ("http://a:80\nsocks5://b:1080", "text/plain")
        );
        let (json, _) = build_body(&opts("json"), &items, None, 0).unwrap();
        assert_eq!(
            json,
            r#"[{"latency_ms":10,"proxy":"http://a:80"},{"latency_ms":null,"proxy":"socks5://b:1080"}]"#
        );

        let (tg, ctype) = build_body(&opts("telegram"), &items, None, 0).unwrap();
        assert_eq!(ctype, "application/json");
        let v: serde_json::Value = serde_json::from_str(&tg).unwrap();
        assert_eq!(
            v["text"],
            "ProxPulse: 2 proxies\nhttp://a:80\nsocks5://b:1080"
        );
        let (dc, _) = build_body(&opts("discord"), &items, None, 0).unwrap();
        assert!(
            dc.starts_with(r#"{"content":"ProxPulse: 2 proxies"#),
            "{dc}"
        );

        let template = WebhookOptions {
            format: "template".to_string(),
            template: r#"{"n":{count},"list":{proxies_json},"at":"{time}"}"#.to_string(),
            ..WebhookOptions::default()
        };
        let (body, ctype) = build_body(&template, &items, None, 86_400 * 365 + 61).unwrap();
        assert_eq!(ctype, "application/json");
        assert_eq!(
            body,
            r#"{"n":2,"list":["http://a:80","socks5://b:1080"],"at":"1971-01-01T00:01:01Z"}"#
        );
    }

    #[test]
    fn webhook_change_bodies() {
        let prev: HashSet<String> = ["http://old:1".to_string(), "http://a:80".to_string()].into();
        let now = vec!["http://a:80".to_string(), "http://new:2".to_string()];
        let c = Changes::between(&prev, &now);
        assert_eq!(c.added, vec!["http://new:2"]);
        assert_eq!(c.removed, vec!["http://old:1"]);

        let items = vec![item("http://a:80", Some(5)), item("http://new:2", Some(7))];
        let (txt, _) = build_body(&opts("txt"), &items, Some(&c), 0).unwrap();
        assert_eq!(txt, "+http://new:2\n-http://old:1");
        let (json, _) = build_body(&opts("json"), &items, Some(&c), 0).unwrap();
        assert_eq!(
            json,
            r#"{"added":[{"latency_ms":7,"proxy":"http://new:2"}],"removed":["http://old:1"]}"#
        );
        let (csv, _) = build_body(&opts("csv"), &items, Some(&c), 0).unwrap();
        assert_eq!(
            csv,
            "change,proxy,latency_ms\nadded,http://new:2,7\nremoved,http://old:1,\n"
        );
    }

    #[test]
    fn chat_messages_fit_the_service_limit() {
        let lines: Vec<String> = (0..500)
            .map(|i| format!("http://10.0.0.{i}:8080"))
            .collect();
        let text = chat_text("ProxPulse: 500 proxies", &lines, DISCORD_MAX);
        assert!(text.len() <= DISCORD_MAX, "{}", text.len());
        assert!(text.ends_with(" more"), "{text}");
    }

    #[test]
    fn only_transient_failures_are_retried() {
        assert!(retryable("http 503 busy"));
        assert!(retryable("http 429"));
        assert!(retryable("connection refused"));
        assert!(!retryable("http 400 bad"));
        assert!(!retryable("http 404"));
        assert!(!retryable("private address"));
        assert!(!retryable("https required when sending proxy credentials"));
    }

    #[tokio::test]
    async fn unchanged_lists_are_not_sent_again() {
        let sent = SentLists::default();
        sent.lock().await.insert(
            "http://127.0.0.1:9/hook".to_string(),
            ["http://a:80".to_string()].into(),
        );
        let delivery = Delivery {
            retry: false,
            changes_only: true,
            sent: Some(&sent),
        };
        let results = send_webhooks_inner(
            &["http://127.0.0.1:9/hook".to_string()],
            &opts("txt"),
            Arc::new(vec![item("http://a:80", Some(1))]),
            delivery,
        )
        .await;
        assert!(results[0].ok);
        assert_eq!(results[0].message, "no changes");
    }

    #[test]
    fn webhook_payload_redacts_credentials_by_default() {
        let items = vec![
            DispatchItem {
                raw: "u:p@host:8080".to_string(),
                url: Some("http://u:p@host:8080".to_string()),
                latency: Some(10),
                country: None,
            },
            DispatchItem {
                raw: "socks5://host:1080".to_string(),
                url: None,
                latency: None,
                country: None,
            },
        ];
        assert_eq!(
            webhook_proxies(&items, false),
            vec!["http://host:8080", "socks5://host:1080"]
        );
        assert_eq!(
            webhook_proxies(&items, true),
            vec!["http://u:p@host:8080", "socks5://host:1080"]
        );
    }

    #[test]
    fn csv_formula_injection_guard() {
        assert_eq!(csv_cell("=1+1"), "'=1+1");
        assert_eq!(csv_cell("+1"), "'+1");
        assert_eq!(csv_cell("-1"), "'-1");
        assert_eq!(csv_cell("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(csv_cell("=a,b"), "\"'=a,b\"");
    }

    #[test]
    fn csv_escaping() {
        assert_eq!(csv_cell("1.2.3.4:80"), "1.2.3.4:80");
        assert_eq!(csv_cell("user,pass"), "\"user,pass\"");
        assert_eq!(csv_cell("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_cell("line\nbreak"), "\"line\nbreak\"");
    }
    #[tokio::test]
    async fn webhook_redirects_are_not_followed() {
        let redirector = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let redirect_port = redirector.local_addr().unwrap().port();
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_port = target.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = redirector.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:{target_port}/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = s.write_all(response.as_bytes()).await;
            let _ = s.shutdown().await;
        });

        let items = vec![DispatchItem {
            raw: "u:p@host:8080".to_string(),
            url: Some("http://u:p@host:8080".to_string()),
            latency: Some(1),
            country: None,
        }];
        let client = guarded_client_builder(Duration::from_secs(15))
            .build()
            .unwrap();
        let result = send_webhook_request(
            &client,
            &format!("http://127.0.0.1:{redirect_port}/hook"),
            &opts("txt"),
            &items,
            None,
        )
        .await
        .unwrap_err();
        assert!(result.starts_with("http 307"), "{result}");
        assert!(
            tokio::time::timeout(Duration::from_millis(300), target.accept())
                .await
                .is_err(),
            "webhook client must not follow redirects"
        );
    }

    async fn one_shot_server(response: &'static str) -> u16 {
        let server = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = server.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = server.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(response.as_bytes()).await;
            let _ = s.shutdown().await;
        });
        port
    }

    #[tokio::test]
    async fn webhook_status_follows_http_code() {
        let items = vec![DispatchItem {
            raw: "host:8080".to_string(),
            url: None,
            latency: None,
            country: None,
        }];
        let client = guarded_client_builder(Duration::from_secs(5))
            .build()
            .unwrap();

        let port = one_shot_server(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\nConnection: close\r\n\r\noops",
        )
        .await;
        let err = send_webhook_request(
            &client,
            &format!("http://127.0.0.1:{port}/hook"),
            &opts("txt"),
            &items,
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err, "http 500 oops");

        let port = one_shot_server("HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n").await;
        let ok = send_webhook_request(
            &client,
            &format!("http://127.0.0.1:{port}/hook"),
            &opts("txt"),
            &items,
            None,
        )
        .await
        .unwrap();
        assert!(ok.starts_with("http 204"), "{ok}");
    }

    #[tokio::test]
    async fn webhook_rejects_private_targets() {
        let items = vec![DispatchItem {
            raw: "host:8080".to_string(),
            url: None,
            latency: None,
            country: None,
        }];
        let err = send_webhook_one("http://127.0.0.1:9/hook", &opts("txt"), &items, None)
            .await
            .unwrap_err();
        assert_eq!(err, "private address");
        let err = send_webhook_one("http://localhost:9/hook", &opts("txt"), &items, None)
            .await
            .unwrap_err();
        assert_eq!(err, "private address");
    }
}
