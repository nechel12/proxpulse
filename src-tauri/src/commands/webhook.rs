use crate::commands::dispatch::DispatchItem;
use crate::http::{read_body_capped, short_error, BODY_LIMIT_WEBHOOK};
use crate::netguard::{guarded_client_builder, resolve_public_target};
use crate::{is_http_url, strip_credentials, WebhookResult};
use std::sync::Arc;
use std::time::Duration;

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

async fn send_webhook_request(
    client: &reqwest::Client,
    url: &str,
    format: &str,
    include_credentials: bool,
    items: &[DispatchItem],
) -> Result<String, String> {
    let proxies = webhook_proxies(items, include_credentials);
    let (body, ctype) = match format {
        "json" => {
            let arr: Vec<serde_json::Value> = items
                .iter()
                .enumerate()
                .map(|(n, i)| {
                    serde_json::json!({"proxy": proxies[n].clone(), "latency_ms": i.latency})
                })
                .collect();
            (
                serde_json::to_string(&arr).map_err(|e| e.to_string())?,
                "application/json",
            )
        }
        "csv" => {
            let mut s = String::from("proxy,latency_ms\n");
            for (n, i) in items.iter().enumerate() {
                s.push_str(&format!(
                    "{},{}\n",
                    csv_cell(&proxies[n]),
                    i.latency.map(|v| v.to_string()).unwrap_or_default()
                ));
            }
            (s, "text/csv")
        }
        _ => (proxies.join("\n"), "text/plain"),
    };
    let r = client
        .post(url)
        .header("Content-Type", ctype)
        .body(body)
        .send()
        .await
        .map_err(|e| short_error(&e.to_string()))?;
    let code = r.status().as_u16();
    let txt = read_body_capped(r, BODY_LIMIT_WEBHOOK)
        .await
        .unwrap_or_default();
    let short: String = txt.chars().take(80).collect();
    Ok(format!("http {code} {short}"))
}

async fn send_webhook_one(
    url: &str,
    format: &str,
    include_credentials: bool,
    items: &[DispatchItem],
) -> Result<String, String> {
    let url = url.trim().to_string();
    if !is_http_url(&url) {
        return Err("bad webhook url".to_string());
    }
    let parsed = reqwest::Url::parse(&url).map_err(|_| "bad webhook url".to_string())?;
    if include_credentials && parsed.scheme() == "http" {
        return Err("https required when sending proxy credentials".to_string());
    }
    // Webhook URLs are user-supplied but must not reach local services.
    let mut builder = guarded_client_builder(Duration::from_secs(15));
    if let Some((host, addr)) = resolve_public_target(&parsed).await? {
        builder = builder.resolve(&host, addr);
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    send_webhook_request(&client, &url, format, include_credentials, items).await
}

pub(crate) async fn send_webhooks_inner(
    urls: &[String],
    format: &str,
    include_credentials: bool,
    items: Arc<Vec<DispatchItem>>,
) -> Vec<WebhookResult> {
    let mut set = tokio::task::JoinSet::new();
    for (index, url) in urls.iter().take(10).cloned().enumerate() {
        let format = format.to_string();
        let items = items.clone();
        set.spawn(async move {
            let res = send_webhook_one(&url, &format, include_credentials, &items).await;
            match res {
                Ok(message) => WebhookResult {
                    index,
                    ok: true,
                    message,
                },
                Err(message) => WebhookResult {
                    index,
                    ok: false,
                    message,
                },
            }
        });
    }
    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        match r {
            Ok(v) => out.push(v),
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

#[tauri::command]
pub(crate) async fn send_webhooks(
    urls: Vec<String>,
    format: String,
    include_credentials: Option<bool>,
    items: Vec<DispatchItem>,
) -> Result<Vec<WebhookResult>, String> {
    Ok(send_webhooks_inner(
        &urls,
        &format,
        include_credentials.unwrap_or(false),
        Arc::new(items),
    )
    .await)
}

#[tauri::command]
pub(crate) async fn send_webhook(
    url: String,
    format: String,
    include_credentials: Option<bool>,
    items: Vec<DispatchItem>,
) -> Result<String, String> {
    send_webhook_one(&url, &format, include_credentials.unwrap_or(false), &items).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

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
            "txt",
            false,
            &items,
        )
        .await
        .unwrap();
        assert!(result.starts_with("http 307"));
        assert!(
            tokio::time::timeout(Duration::from_millis(300), target.accept())
                .await
                .is_err(),
            "webhook client must not follow redirects"
        );
    }

    #[tokio::test]
    async fn webhook_rejects_private_targets() {
        let items = vec![DispatchItem {
            raw: "host:8080".to_string(),
            url: None,
            latency: None,
            country: None,
        }];
        let err = send_webhook_one("http://127.0.0.1:9/hook", "txt", false, &items)
            .await
            .unwrap_err();
        assert_eq!(err, "private address");
        let err = send_webhook_one("http://localhost:9/hook", "txt", false, &items)
            .await
            .unwrap_err();
        assert_eq!(err, "private address");
    }
}
