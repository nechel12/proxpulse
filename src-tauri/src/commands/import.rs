use crate::extract::extract_proxies;
use crate::http::{read_body_capped, short_error, BODY_LIMIT_IMPORT};
use crate::netguard::build_guarded_client;
use serde::Serialize;
use std::time::Duration;

/// Upper bound for one parse; the UI asks for its own cap + 1.
const PARSE_LIMIT_MAX: usize = 100_000;

#[derive(Debug, Serialize)]
pub(crate) struct ParsedList {
    count: usize,
    truncated: bool,
    list: Option<Vec<String>>,
}

/// Extract proxies from pasted, imported or downloaded text. The UI counts
/// and checks exactly this list, so the count matches what gets checked.
#[tauri::command]
pub(crate) async fn parse_proxy_text(
    text: String,
    limit: usize,
    want_list: bool,
    default_proto: Option<String>,
) -> Result<ParsedList, String> {
    let limit = limit.min(PARSE_LIMIT_MAX);
    let proto = default_proto.unwrap_or_else(|| "http".to_string());
    tauri::async_runtime::spawn_blocking(move || {
        let list = extract_proxies(&text, limit, &proto);
        ParsedList {
            count: list.len(),
            truncated: list.len() >= limit,
            list: want_list.then_some(list),
        }
    })
    .await
    .map_err(|e| short_error(&e.to_string()))
}

#[tauri::command]
pub(crate) async fn fetch_url_text(url: String) -> Result<String, String> {
    let mut current = reqwest::Url::parse(url.trim()).map_err(|_| "bad url".to_string())?;
    for _ in 0..6 {
        let client = build_guarded_client(&current, Duration::from_secs(20)).await?;
        let r = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| short_error(&e.to_string()))?;
        if r.status().is_redirection() {
            let location = r
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "bad redirect".to_string())?;
            current = current
                .join(location)
                .map_err(|_| "bad redirect".to_string())?;
            continue;
        }
        if !r.status().is_success() {
            return Err(format!("http {}", r.status().as_u16()));
        }
        return read_body_capped(r, BODY_LIMIT_IMPORT).await;
    }
    Err("too many redirects".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn url_import_rejects_loopback_before_connecting() {
        let err = fetch_url_text("http://127.0.0.1:9/list".to_string())
            .await
            .unwrap_err();
        assert_eq!(err, "private address");
    }
}
