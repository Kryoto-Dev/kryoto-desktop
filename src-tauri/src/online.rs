//! Kryoto Online on an installed game: whether the game page offers it,
//! and Undo.
//!
//! Setting it up goes through the repair engine, like every other emulator.
//! Games set up by older versions kept their own record here (`LocalOnline`)
//! of what was added and what was set aside: Undo deletes what was added and
//! puts every original back, so the game is exactly as it was.

use crate::library::{self, LibraryGame};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, Runtime};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalOnline {
    /// Kryoto Online's release, e.g. "v1.8.1".
    pub version: String,
    /// Files added, relative to the game's folder; deleted on undo.
    pub added: Vec<String>,
    /// Files replaced or set aside: (the file, its saved original).
    pub saved: Vec<(String, String)>,
    pub applied_at: u64,
}

fn game_root<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> PathBuf {
    let folders = crate::settings::all_folders(&crate::settings::load(app));
    crate::storage::game_folder(&folders, game).unwrap_or_else(|| PathBuf::from(&game.install_dir))
}

/// Why a game has no use for Kryoto Online, or `None` when it does.
///
/// It is for games Steam says play online, whose release does not already
/// bring its own way online: kryo.to marks those releases multiplayer, and
/// older ones say so in their source (OFME, Online-Fix, Kryoto Online).
pub fn online_unneeded(game: &serde_json::Value, local_source: Option<&str>) -> Option<&'static str> {
    if game["multiplayer"].as_bool() == Some(true) {
        return Some("This release already plays online.");
    }
    let has_fix = |s: &str| {
        let s = s.to_ascii_lowercase();
        s.contains("ofme")
            || s.contains("online-fix")
            || s.contains("onlinefix")
            || s.contains("online fix")
            || s.contains("kryoto online")
            || s.contains("kryotoo")
    };
    if [game["source"].as_str(), local_source].into_iter().flatten().any(has_fix) {
        return Some("This release already plays online.");
    }
    let features: Vec<&str> = game["features"].as_array().into_iter().flatten().filter_map(|f| f.as_str()).collect();
    let has = |name: &str| features.iter().any(|f| f.eq_ignore_ascii_case(name));
    let online = has("Online Co-op")
        || has("Online PvP")
        || has("MMO")
        || has("Cross-Platform Multiplayer")
        // Steam's plain "Multi-player" with no split screen listed is online.
        || (has("Multi-player") && !features.iter().any(|f| f.to_ascii_lowercase().contains("split screen")));
    if !online {
        return Some("Steam does not list online play for this game.");
    }
    None
}

async fn catalog_game<R: tauri::Runtime>(app: &AppHandle<R>, client: &reqwest::Client, slug: &str) -> Result<serde_json::Value, String> {
    // A game taken off kryo.to is not asked again for a day (catalog_gone.rs):
    // its Library page asked on every visit (the Add-ons card's online check).
    if crate::catalog_gone::is_gone(app, slug) {
        return Err("kryo.to no longer has this game.".into());
    }
    let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(app));
    let res = client.get(format!("{endpoint}/api/games/{slug}")).send().await.map_err(|e| e.to_string())?;
    if crate::catalog_gone::says_gone(res.status().as_u16()) {
        crate::catalog_gone::set(app, slug, true);
    }
    if !res.status().is_success() {
        return Err(format!("kryo.to answered {}.", res.status().as_u16()));
    }
    let json = res.json::<serde_json::Value>().await.map_err(|e| e.to_string())?;
    Ok(json["game"].clone())
}

fn client() -> Result<reqwest::Client, String> {
    crate::net::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// Whether the game page offers Kryoto Online: `None` when it does, else why not.
#[tauri::command]
pub async fn online_check(app: AppHandle, game_id: String) -> Result<Option<String>, String> {
    let game = library::load(&app)?.into_iter().find(|g| g.id == game_id).ok_or("That game is no longer in the library.")?;
    let Some(slug) = game.slug.clone() else { return Ok(Some("Not linked to a kryo.to page.".into())) };
    let catalog = catalog_game(&app, &client()?, &slug).await?;
    Ok(online_unneeded(&catalog, game.source.as_deref()).map(String::from))
}

#[tauri::command]
pub async fn online_apply(app: AppHandle, game_id: String) -> Result<LibraryGame, String> {
    crate::repair::repair_apply(app.clone(), game_id.clone(), kryoto_repair::settings::Emulator::Online).await?;
    library::load(&app)?.into_iter().find(|g| g.id == game_id).ok_or("That game is no longer in the library.".into())
}
fn undo_files(root: &Path, rec: &LocalOnline) {
    for f in &rec.added {
        if let Ok(p) = crate::launch::inside(root, f) {
            if p.is_dir() {
                // Only if still empty: plugins a player put in stay.
                let _ = std::fs::remove_dir(p);
            } else {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    for (file, backup) in &rec.saved {
        if let (Ok(file), Ok(backup)) = (crate::launch::inside(root, file), crate::launch::inside(root, backup)) {
            if backup.exists() {
                let _ = std::fs::remove_file(&file);
                let _ = std::fs::rename(&backup, &file);
            }
        }
    }
}

#[tauri::command(async)]
pub fn online_undo(app: AppHandle, game_id: String) -> Result<LibraryGame, String> {
    let game = library::load(&app)?.into_iter().find(|g| g.id == game_id).ok_or("That game is no longer in the library.")?;
    if game.repair.is_some() {
        crate::repair::repair_undo(app.clone(), game_id.clone())?;
        return library::load(&app)?.into_iter().find(|g| g.id == game_id).ok_or("That game is no longer in the library.".into());
    }
    if app.state::<library::Running>().0.lock().map_err(|e| e.to_string())?.contains_key(&game_id) { return Err("Close the game before Undo.".into()) }
    let rec = game.online.clone().ok_or("Kryoto Online is not set up for this game.")?;
    undo_files(&game_root(&app, &game), &rec);
    crate::logging::info("online", &format!("Kryoto Online removed from {}", game.title));
    let updated = library::update(&app, |games| {
        let g = games.iter_mut().find(|g| g.id == game_id).ok_or("That game is no longer in the library.")?;
        g.online = None;
        Ok(g.clone())
    })?;
    let _ = app.emit("library-changed", ());
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_is_offered_only_where_it_is_missing() {
        let g = |v: serde_json::Value| v;
        let online = serde_json::json!(["Single-player", "Multi-player", "Online Co-op"]);
        // Steam lists online play, the release has no way online: offered.
        assert_eq!(online_unneeded(&g(serde_json::json!({"features": online, "source": "Steam + gbe_fork"})), None), None);
        // Marked multiplayer on kryo.to: the release brings its own.
        assert!(online_unneeded(&g(serde_json::json!({"features": online, "multiplayer": true})), None).is_some());
        // Older releases only say so in their source.
        for source in ["OFME", "Steam + Online-Fix", "onlinefix", "Steam + Kryoto Online", "KryotoO"] {
            assert!(online_unneeded(&g(serde_json::json!({"features": online, "source": source})), None).is_some(), "{source}");
        }
        assert!(online_unneeded(&g(serde_json::json!({"features": online})), Some("OFME")).is_some());
        // Single-player, or split screen only: nothing to play online.
        assert!(online_unneeded(&g(serde_json::json!({"features": ["Single-player"]})), None).is_some());
        assert!(online_unneeded(&g(serde_json::json!({"features": ["Multi-player", "Shared/Split Screen Co-op"]})), None).is_some());
        assert_eq!(online_unneeded(&g(serde_json::json!({"features": ["Multi-player"]})), None), None);
        assert!(online_unneeded(&g(serde_json::json!({})), None).is_some());
    }

    /// A game set up by an older version, as it left it: the proxy in the
    /// dll's place, its core and ini beside it, the originals set aside.
    #[test]
    fn undo_leaves_the_game_as_it_was() {
        let base = std::env::temp_dir().join(format!("kryoto-online-undo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("Game");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("bin/steam_api64.dll"), "proxy").unwrap();
        std::fs::write(root.join("bin/steam_api64.dll.kryoto-original"), "valve").unwrap();
        std::fs::write(root.join("bin/kryotoO.dll"), "core").unwrap();
        std::fs::write(root.join("bin/kryoto-online.ini"), "ini").unwrap();
        std::fs::write(root.join("bin/steam_appid.txt.kryoto-original"), "12345").unwrap();
        let rec = LocalOnline {
            version: "v1.8.1".into(),
            added: vec!["bin/kryotoO.dll".into(), "bin/kryoto-online.ini".into()],
            saved: vec![
                ("bin/steam_api64.dll".into(), "bin/steam_api64.dll.kryoto-original".into()),
                ("bin/steam_appid.txt".into(), "bin/steam_appid.txt.kryoto-original".into()),
            ],
            applied_at: 1,
        };

        undo_files(&root, &rec);
        assert_eq!(std::fs::read_to_string(root.join("bin/steam_api64.dll")).unwrap(), "valve");
        assert_eq!(std::fs::read_to_string(root.join("bin/steam_appid.txt")).unwrap(), "12345");
        assert!(!root.join("bin/kryotoO.dll").exists());
        assert!(!root.join("bin/kryoto-online.ini").exists());
        assert!(!root.join("bin/steam_api64.dll.kryoto-original").exists());
        assert!(!root.join("bin/steam_appid.txt.kryoto-original").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
