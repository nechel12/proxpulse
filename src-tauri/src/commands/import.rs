use crate::http::{read_body_capped, short_error, BODY_LIMIT_IMPORT};
use crate::netguard::build_guarded_client;
use std::time::Duration;

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
