//! Windows mirror pages have their own profile and the official, unmodified
//! uBlock Origin extension. Wait for its engine before visiting a file host.

use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Manager, Runtime, WebviewWindow};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use windows::core::{Interface, HSTRING};

type Reply = Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<Value, String>>>>>;

fn reply(sender: &Reply, value: Result<Value, String>) {
    if let Ok(mut sender) = sender.lock() {
        if let Some(sender) = sender.take() {
            let _ = sender.send(value);
        }
    }
}

async fn receive(receiver: tokio::sync::oneshot::Receiver<Result<Value, String>>) -> Result<Value, String> {
    tokio::time::timeout(Duration::from_secs(20), receiver)
        .await
        .map_err(|_| "uBlock took too long to start".to_string())?
        .map_err(|_| "the download window closed".to_string())?
}

async fn protocol<R: Runtime>(window: &WebviewWindow<R>, method: &str, params: Value, session: Option<String>) -> Result<Value, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender: Reply = Arc::new(Mutex::new(Some(sender)));
    let method = HSTRING::from(method);
    let params = HSTRING::from(params.to_string());
    window.with_webview(move |view| unsafe {
        let complete = sender.clone();
        let result = (|| -> windows::core::Result<()> {
            let core = view.controller().CoreWebView2()?;
            let handler = webview2_com::CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, response| {
                let result = status.map_err(|e| e.to_string()).and_then(|_| serde_json::from_str(&response).map_err(|e| e.to_string()));
                reply(&complete, result);
                Ok(())
            }));
            if let Some(session) = session {
                core.cast::<ICoreWebView2_21>()?.CallDevToolsProtocolMethodForSession(&HSTRING::from(session), &method, &params, &handler)
            } else {
                core.CallDevToolsProtocolMethod(&method, &params, &handler)
            }
        })();
        if let Err(error) = result {
            reply(&sender, Err(error.to_string()));
        }
    }).map_err(|e| e.to_string())?;
    receive(receiver).await
}

pub async fn install<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    let path = window.app_handle().path().resolve("extensions/ublock", tauri::path::BaseDirectory::Resource).map_err(|e| e.to_string())?;
    #[cfg(debug_assertions)]
    let path = if path.join("manifest.json").is_file() { path } else { std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/ublock") };
    if !path.join("manifest.json").is_file() {
        return Err("the bundled uBlock extension is missing".into());
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender: Reply = Arc::new(Mutex::new(Some(sender)));
    window.with_webview(move |view| unsafe {
        let complete = sender.clone();
        let result = (|| -> windows::core::Result<()> {
            let core = view.controller().CoreWebView2()?;
            let profile = core.cast::<ICoreWebView2_13>()?.Profile()?.cast::<ICoreWebView2Profile7>()?;
            let handler = webview2_com::ProfileAddBrowserExtensionCompletedHandler::create(Box::new(move |status, extension| {
                let result = (|| -> Result<Value, String> {
                    status.map_err(|e| e.to_string())?;
                    let extension = extension.ok_or("uBlock was not installed")?;
                    let mut enabled = windows::core::BOOL::default();
                    extension.IsEnabled(&mut enabled).map_err(|e| e.to_string())?;
                    if !enabled.as_bool() { return Err("uBlock is disabled".into()) }
                    let mut id = windows::core::PWSTR::null();
                    extension.Id(&mut id).map_err(|e| e.to_string())?;
                    Ok(Value::String(webview2_com::take_pwstr(id)))
                })();
                reply(&complete, result);
                Ok(())
            }));
            profile.AddBrowserExtension(&HSTRING::from(path.as_path()), &handler)
        })();
        if let Err(error) = result { reply(&sender, Err(error.to_string())) }
    }).map_err(|e| e.to_string())?;
    let extension = receive(receiver).await?;
    let url = format!("chrome-extension://{}/background.html", extension.as_str().ok_or("uBlock returned no extension ID")?);
    let started = Instant::now();
    let session = loop {
        let targets = protocol(window, "Target.getTargets", json!({}), None).await?;
        if let Some(target) = targets["targetInfos"].as_array().and_then(|targets| targets.iter().find(|t| t["url"] == url && t["type"] == "background_page")) {
            let attached = protocol(window, "Target.attachToTarget", json!({"targetId": target["targetId"], "flatten": true}), None).await?;
            break attached["sessionId"].as_str().ok_or("uBlock has no background session")?.to_string();
        }
        if started.elapsed() > Duration::from_secs(20) { return Err("uBlock's background page did not start".into()) }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let outcome = loop {
        let result = protocol(window, "Runtime.evaluate", json!({"expression": "globalThis.µBlock?.readyToFilter === true", "returnByValue": true}), Some(session.clone())).await?;
        if result["result"]["value"] == true { break Ok(()) }
        if started.elapsed() > Duration::from_secs(20) { break Err("uBlock's filters did not become ready".into()) }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let _ = protocol(window, "Target.detachFromTarget", json!({"sessionId": session}), None).await;
    outcome
}
