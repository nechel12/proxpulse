use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

fn validate_export_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() || path.contains('\0') {
        return Err("invalid path".to_string());
    }
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return Err("path must be absolute".to_string());
    }
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    if !matches!(ext.as_deref(), Some("txt" | "json" | "csv" | "log")) {
        return Err("unsupported file extension".to_string());
    }
    Ok(())
}

fn with_export_extension(mut path: std::path::PathBuf, ext: &str) -> std::path::PathBuf {
    let ext = ext.trim_start_matches('.');
    if !ext.is_empty() && path.extension().is_none() {
        path.set_extension(ext);
    }
    path
}

#[tauri::command]
pub(crate) async fn export_text_file(
    app: AppHandle,
    contents: String,
    default_name: String,
    filter_name: String,
    extensions: Vec<String>,
) -> Result<Option<String>, String> {
    let ext = extensions
        .first()
        .cloned()
        .unwrap_or_else(|| "txt".to_string());
    let default_name = with_export_extension(std::path::PathBuf::from(default_name), &ext)
        .to_string_lossy()
        .to_string();
    let extensions: Vec<String> = extensions
        .into_iter()
        .map(|e| e.trim_start_matches('.').to_string())
        .filter(|e| !e.is_empty())
        .collect();
    let filter_name = if filter_name.trim().is_empty() {
        "Text".to_string()
    } else {
        filter_name
    };
    let choice = tauri::async_runtime::spawn_blocking(move || {
        let refs: Vec<&str> = extensions.iter().map(String::as_str).collect();
        app.dialog()
            .file()
            .set_file_name(default_name)
            .add_filter(filter_name, &refs)
            .blocking_save_file()
    })
    .await
    .map_err(|e| crate::http::short_error(&e.to_string()))?;
    let Some(path) = choice else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|e| crate::http::short_error(&e.to_string()))?;
    let path = with_export_extension(path, &ext);
    validate_export_path(&path.to_string_lossy())?;
    let write_path = path.clone();
    tauri::async_runtime::spawn_blocking(move || std::fs::write(&write_path, contents))
        .await
        .map_err(|e| crate::http::short_error(&e.to_string()))?
        .map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_path_validation() {
        let ok = std::env::temp_dir().join("proxpulse-audit.txt");
        assert!(validate_export_path(&ok.to_string_lossy()).is_ok());
        assert!(validate_export_path("relative.txt").is_err());
        assert!(validate_export_path("C:\\temp\\x.exe").is_err());
        assert!(validate_export_path("bad\0path.txt").is_err());
        assert!(validate_export_path("").is_err());
    }
}
