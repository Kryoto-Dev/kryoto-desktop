//! Games kryo.to no longer has: their page answered 404 or 410 (the game was
//! taken off the site, like persona-4-golden or bloodborne).
//!
//! An installed game keeps its link to the page, and everything that asks
//! kryo.to about installed games (its page in the Library, update checks, the
//! Steam branch at start, achievements) kept asking for the removed ones: tens
//! of thousands of 404s a week on `/api/games/<slug>` (2026-11-21). Each such
//! answer is remembered here, per slug, and that game is not asked again for a
//! day. The game itself stays in the Library and plays as before; a page that
//! comes back is found again the next day, or at once when the player links it
//! by hand (Properties > kryo.to, Add a game).
//!
//! Kept in `catalog-gone.json` in the app's data folder, so a restart does not
//! start the asking over.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, Runtime};

/// How long a removed game is left alone before it is asked about again.
const RETRY_SECS: u64 = 24 * 60 * 60;
const FILE: &str = "catalog-gone.json";

/// When each removed game last answered "gone", in seconds since 1970.
static GONE: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Gone, and said so less than a day ago.
fn still_gone(map: &HashMap<String, u64>, slug: &str, at: u64) -> bool {
    map.get(slug).is_some_and(|&when| at.saturating_sub(when) < RETRY_SECS)
}

/// Forget the ones that are due to be asked again.
fn prune(map: &mut HashMap<String, u64>, at: u64) {
    map.retain(|_, when| at.saturating_sub(*when) < RETRY_SECS);
}

fn with<R: Runtime, T>(app: &AppHandle<R>, f: impl FnOnce(&mut HashMap<String, u64>) -> T) -> T {
    let mut guard = GONE.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(|| {
        app.path()
            .app_data_dir()
            .ok()
            .and_then(|dir| std::fs::read(dir.join(FILE)).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    });
    f(map)
}

fn save<R: Runtime>(app: &AppHandle<R>, map: &HashMap<String, u64>) {
    let Ok(dir) = app.path().app_data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(bytes) = serde_json::to_vec(map) {
        let tmp = dir.join(format!("{FILE}.tmp"));
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join(FILE));
        }
    }
}

/// Whether kryo.to said this game is gone less than a day ago: then it is not asked.
pub fn is_gone<R: Runtime>(app: &AppHandle<R>, slug: &str) -> bool {
    with(app, |map| still_gone(map, slug, now()))
}

/// Remember (`true`) or forget (`false`) that kryo.to has no page for this game.
pub fn set<R: Runtime>(app: &AppHandle<R>, slug: &str, gone: bool) {
    with(app, |map| {
        let at = now();
        let changed = if gone {
            map.insert(slug.to_string(), at);
            true
        } else {
            map.remove(slug).is_some()
        };
        if changed {
            prune(map, at);
            save(app, map);
        }
    });
}

/// Whether an answer from `/api/games/<slug>` means the game is gone from kryo.to.
pub fn says_gone(status: u16) -> bool {
    status == 404 || status == 410
}

/// The page's side of the same memory (lib/library.ts `fetchCatalogGame`):
/// with `gone`, remember or forget; without, ask. Answers whether the game is
/// to be left alone now.
#[tauri::command(async)]
pub fn catalog_gone(app: AppHandle, slug: String, gone: Option<bool>) -> bool {
    let slug = slug.trim().to_ascii_lowercase();
    if slug.is_empty() || slug.len() > 160 {
        return false;
    }
    if let Some(gone) = gone {
        set(&app, &slug, gone);
    }
    is_gone(&app, &slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removed_game_is_left_alone_for_a_day_then_asked_again() {
        let mut map = HashMap::new();
        map.insert("bloodborne".to_string(), 1_000);
        assert!(still_gone(&map, "bloodborne", 1_000));
        assert!(still_gone(&map, "bloodborne", 1_000 + RETRY_SECS - 1));
        assert!(!still_gone(&map, "bloodborne", 1_000 + RETRY_SECS));
        assert!(!still_gone(&map, "hades", 1_000));
        prune(&mut map, 1_000 + RETRY_SECS);
        assert!(map.is_empty());
    }

    #[test]
    fn only_404_and_410_mean_gone() {
        assert!(says_gone(404));
        assert!(says_gone(410));
        // A site having trouble is not a game taken off it.
        for status in [200, 401, 403, 429, 500, 502, 503] {
            assert!(!says_gone(status));
        }
    }
}
