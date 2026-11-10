//! The Report button in Kryoto Desktop Preview (src/shell/PreviewReport.tsx;
//! kryo.to docs/preview.md): a picture of the window taken when it is pressed,
//! and the report sent to the Kryoto Preview as the account signed in to the
//! Store, with its files.
//!
//! Through here rather than from the page because the shell cannot reach
//! preview.kryo.to with the Store's sign-in, and the picture has to include the
//! Store, which is a web view of its own that only the window's own drawing has.

use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::json;
use tauri::AppHandle;
#[cfg(windows)]
use tauri::Manager;

use crate::community_offer::Kryo;

const MAX_FILE: usize = 25 * 1024 * 1024;

/// The window as it is now, as a base64 PNG. Windows only (`PrintWindow`, which
/// draws the Store's web view too); elsewhere `None` and the reporter adds their own.
#[tauri::command(async)]
pub fn preview_report_screenshot(app: AppHandle) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        let Some(window) = app.get_window("main") else { return Ok(None) };
        let hwnd = window.hwnd().map_err(|e| e.to_string())?;
        Ok(kryoto_repair::window_capture::capture_window_png(hwnd.0 as usize).map(|png| base64::engine::general_purpose::STANDARD.encode(png)))
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Ok(None)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportFile {
    name: String,
    mime: String,
    /// The file, base64.
    data: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewReport {
    kind: String,
    what: String,
    steps: Option<String>,
    /// Where in the app it happened: the tab, or the Store page.
    page: Option<String>,
    files: Vec<ReportFile>,
}

/// Send a report: each file up first (kryo.to hands out an upload address,
/// as for a comment's attachments), then the report naming them.
#[tauri::command(async)]
pub async fn preview_report_send(app: AppHandle, report: NewReport) -> Result<serde_json::Value, String> {
    let kryo = Kryo::new(&app)?;
    let put = reqwest::Client::builder().timeout(Duration::from_secs(120)).build().map_err(|e| e.to_string())?;
    let mut ids = Vec::new();
    for f in report.files.iter().take(5) {
        let bytes = base64::engine::general_purpose::STANDARD.decode(f.data.as_bytes()).map_err(|_| format!("{} could not be read.", f.name))?;
        if bytes.is_empty() || bytes.len() > MAX_FILE {
            return Err(format!("{} is larger than 25 MB.", f.name));
        }
        let prepared = kryo
            .call(reqwest::Method::POST, "/api/preview/reports/attachments", Some(json!({ "name": f.name, "type": f.mime, "size": bytes.len() })))
            .await?
            .ok_or("Reports are not open on this site.")?;
        let (Some(id), Some(url)) = (prepared["id"].as_str(), prepared["uploadUrl"].as_str()) else {
            return Err("kryo.to did not give an upload address.".into());
        };
        let res = put
            .put(url)
            .header("content-type", f.mime.as_str())
            .body(bytes)
            .send()
            .await
            .map_err(|e| format!("Uploading {} failed: {e}", f.name))?;
        if !res.status().is_success() {
            return Err(format!("Uploading {} failed ({}).", f.name, res.status()));
        }
        ids.push(id.to_string());
    }
    let context = json!({
        "app": "desktop",
        "version": env!("CARGO_PKG_VERSION"),
        "os": match std::env::consts::OS { "windows" => "Windows", "linux" => "Linux", "macos" => "macOS", other => other },
        "page": report.page.unwrap_or_default(),
        "build": option_env!("KRYOTO_BUILD").unwrap_or(""),
    });
    kryo.call(
        reqwest::Method::POST,
        "/api/preview/reports",
        Some(json!({ "kind": report.kind, "what": report.what, "steps": report.steps.unwrap_or_default(), "attachments": ids, "context": context })),
    )
    .await?
    .ok_or_else(|| "Reports are not open on this site.".to_string())
}

/// What this account sent and what staff decided, for the Yours tab.
#[tauri::command(async)]
pub async fn preview_reports_mine(app: AppHandle) -> Result<serde_json::Value, String> {
    let kryo = Kryo::new(&app)?;
    Ok(kryo.call(reqwest::Method::GET, "/api/preview/reports", None).await?.unwrap_or(json!({ "reports": [] })))
}
