//! The Ludusavi database on this PC: downloaded from GitHub the way Ludusavi
//! itself does, turned into a small index (`known-index.json`), and checked
//! for a newer copy once a week.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kryoto_saves::known::{KnownIndex, MANIFEST_URL};
use kryoto_saves::Rule;
use tauri::{AppHandle, Runtime};

const REFRESH_SECS: u64 = 7 * 24 * 3600;

static INDEX: Mutex<Option<Arc<KnownIndex>>> = Mutex::new(None);
static REFRESHING: Mutex<bool> = Mutex::new(false);

fn file<R: Runtime>(app: &AppHandle<R>) -> Option<std::path::PathBuf> {
    Some(super::base_dir(app)?.join("known-index.json"))
}

fn loaded<R: Runtime>(app: &AppHandle<R>) -> Arc<KnownIndex> {
    let mut slot = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = slot.as_ref() {
        return i.clone();
    }
    let index = Arc::new(file(app).map(|f| super::read_json::<KnownIndex>(&f)).unwrap_or_default());
    *slot = Some(index.clone());
    index
}

/// The known save locations for a game. Starts a refresh in the background
/// when the index is missing or a week old.
pub fn rules_for<R: Runtime>(app: &AppHandle<R>, appid: Option<&str>, title: &str) -> Vec<Rule> {
    let index = loaded(app);
    if super::now().saturating_sub(index.built_at) > REFRESH_SECS {
        refresh(app.clone(), index.etag.clone());
    }
    index.rules_for(appid, title)
}

/// When a refresh last started: an offline PC tries again after a while,
/// not on every look at a game.
static LAST_TRY: Mutex<u64> = Mutex::new(0);

fn refresh<R: Runtime>(app: AppHandle<R>, etag: Option<String>) {
    {
        let mut busy = REFRESHING.lock().unwrap_or_else(|e| e.into_inner());
        let mut last = LAST_TRY.lock().unwrap_or_else(|e| e.into_inner());
        if *busy || super::now().saturating_sub(*last) < 1800 {
            return;
        }
        *busy = true;
        *last = super::now();
    }
    tauri::async_runtime::spawn(async move {
        let result = fetch(etag.clone()).await;
        match result {
            Ok(Some((yaml, etag))) => {
                let built = tauri::async_runtime::spawn_blocking(move || KnownIndex::build(&yaml, etag, super::now())).await;
                match built {
                    Ok(Ok(index)) => {
                        crate::logging::info("saves", &format!("save database: {} games", index.by_appid.len() + index.by_title.len()));
                        if let Some(f) = file(&app) {
                            let _ = super::write_json(&f, &index);
                        }
                        *INDEX.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(index));
                    }
                    Ok(Err(e)) => crate::logging::warn("saves", &e),
                    Err(e) => crate::logging::warn("saves", &e.to_string()),
                }
            }
            // Unchanged: only the date moves on.
            Ok(None) => {
                let current = loaded(&app);
                let next = KnownIndex { built_at: super::now(), etag: current.etag.clone(), by_appid: current.by_appid.clone(), by_title: current.by_title.clone() };
                if let Some(f) = file(&app) {
                    let _ = super::write_json(&f, &next);
                }
                *INDEX.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(next));
            }
            Err(e) => crate::logging::warn("saves", &format!("save database: {e}")),
        }
        *REFRESHING.lock().unwrap_or_else(|e| e.into_inner()) = false;
    });
}

/// The manifest, unless it is still the copy with `etag`.
async fn fetch(etag: Option<String>) -> Result<Option<(String, Option<String>)>, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(MANIFEST_URL);
    if let Some(e) = &etag {
        req = req.header("if-none-match", e);
    }
    let res = req.send().await.map_err(|e| e.to_string())?;
    if res.status().as_u16() == 304 {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(format!("GitHub answered {}", res.status()));
    }
    let etag = res.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
    let text = res.text().await.map_err(|e| e.to_string())?;
    Ok(Some((text, etag)))
}
