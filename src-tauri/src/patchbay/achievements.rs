//! Achievements on the player's own server: the exchange the Store page does
//! with kryo.to (achievements_cloud.rs), done from here with that server
//! instead. Same rounds (a few seconds after start, then every half minute,
//! or at once when Settings changes the server), same requests, a bearer
//! token instead of a cookie.

use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;

use super::Own;
use crate::achievements_cloud::{self, Report};

static WAKE: Notify = Notify::const_new();

/// Run a round now (Settings changed the server or its token).
pub fn wake() {
    WAKE.notify_one();
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(8)).await;
        loop {
            match achievements_cloud::target(&app) {
                achievements_cloud::Target::Own(own) => {
                    if let Err(e) = round(&app, &own).await {
                        crate::logging::warn("achievements", &format!("{}: {e}", own.host));
                    }
                }
                // Kept on this PC: the same round with a file, off the UI thread.
                achievements_cloud::Target::Local(dir) => {
                    let a = app.clone();
                    let done = tauri::async_runtime::spawn_blocking(move || achievements_cloud::local_round(&a, &dir)).await;
                    if let Ok(Err(e)) = done {
                        crate::logging::warn("achievements", &e);
                    }
                }
                achievements_cloud::Target::Account => {}
            }
            let _ = tokio::time::timeout(Duration::from_secs(30), WAKE.notified()).await;
        }
    });
}

fn path(own: &Own, rest: &str) -> String {
    format!("{}/api/achievements/cloud{rest}", own.base)
}

async fn answer(res: Result<reqwest::Response, reqwest::Error>, own: &Own) -> Result<serde_json::Value, String> {
    let res = res.map_err(|e| format!("Could not reach {}: {e}", own.host))?;
    match res.status().as_u16() {
        200..=299 => res.json().await.map_err(|e| format!("{} answered oddly: {e}", own.host)),
        401 | 403 => Err(format!("{} refused the token. Check it in Settings > Patchbay.", own.host)),
        404 => Err(format!("{} has no achievements at this address. Check it in Settings > Patchbay.", own.host)),
        s => Err(format!("{} answered HTTP {s}.", own.host)),
    }
}

async fn round(app: &AppHandle, own: &Own) -> Result<(), String> {
    let account = Some(format!("patchbay:{}", super::ledger_key(&own.base)));
    let a = app.clone();
    let pending = tauri::async_runtime::spawn_blocking(move || achievements_cloud::pending_for(&a, account))
        .await
        .map_err(|e| e.to_string())??;
    let http = super::client();
    let mut report = Report::default();
    if !pending.games.is_empty() {
        let req = super::authorize(http.post(path(own, "")), own).json(&json!({ "games": &pending.games }));
        let j = answer(req.send().await, own).await?;
        // A server of your own has no perk to ask for: only a plain `false` stops it.
        report.perk = Some(j.get("perk").and_then(|p| p.as_bool()).unwrap_or(true));
        if let Some(saved) = j.get("games").and_then(|g| g.as_array()) {
            report.saved = saved.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
            report.sent = pending.games;
        }
    }
    if !pending.fetch.is_empty() {
        let req = super::authorize(http.get(path(own, "")).query(&[("appids", pending.fetch.join(","))]), own);
        let j = answer(req.send().await, own).await?;
        if report.perk.is_none() {
            report.perk = Some(j.get("perk").and_then(|p| p.as_bool()).unwrap_or(true));
        }
        let games = j.get("games").cloned().unwrap_or_default();
        for id in &pending.fetch {
            let list = games.get(id).cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            report.fetched.insert(id.clone(), list);
        }
    }
    if report.perk.is_none() && report.saved.is_empty() && report.fetched.is_empty() {
        return Ok(());
    }
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let running = a.state::<crate::library::Running>();
        achievements_cloud::apply(&a, &running, report).map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// For Settings' test button: how many games the server has achievements for.
pub async fn check(own: &Own) -> Result<usize, String> {
    let req = super::authorize(super::client().get(path(own, "")).query(&[("appids", "")]), own);
    let j = answer(req.send().await, own).await?;
    Ok(j.get("games").and_then(|g| g.as_object()).map(|g| g.len()).unwrap_or(0))
}
