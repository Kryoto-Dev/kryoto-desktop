//! Add-ons: a game's extra files from kryo.to - a language pack, the Online
//! add-on - put into the installed game, and taken out again.
//!
//! An add-on is added from its game's Add-ons & DLC card (`addons_get`: the
//! link is minted for this PC and the row knows its game), or downloaded like
//! any release from the Store's download window. When the finished file is one
//! of the game's add-ons (`downloads::match_file`), it is unpacked into the
//! game's own folder instead of a new one, and every file it wrote is recorded. A file it
//! replaces is first moved aside, into a hidden folder beside the game's
//! (`.<game>.kryoto-addons`: same drive, and outside the game, where nothing
//! that looks through its files can take an original for the game's own).
//! Undo deletes exactly the files it wrote and puts those back, so taking a DLC
//! out leaves the game as it was before.

use crate::downloads::{self, Status};
use crate::library::{self, LibraryGame};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Runtime};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InstalledAddon {
    /// Its name on kryo.to, e.g. "Japanese language pack".
    pub label: String,
    /// The archive it came from.
    pub file: String,
    /// Every file it wrote, relative to the game's folder.
    pub files: Vec<String>,
    /// The game's files it replaced, kept under `backup_dir` for Undo.
    pub replaced: Vec<String>,
    pub installed_at: u64,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Where a game's add-ons keep the files they replaced: beside the game's
/// folder, one folder per add-on archive inside. Uninstalling deletes it too.
pub fn backups_of(root: &Path) -> PathBuf {
    let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    root.with_file_name(format!(".{name}.kryoto-addons"))
}

fn backup_dir(root: &Path, file: &str) -> PathBuf {
    backups_of(root).join(downloads::safe_name(file))
}

/// What applying records: the files written, and the game's files moved aside.
#[derive(Default)]
struct Applied {
    files: Vec<String>,
    replaced: Vec<String>,
}

/// Move `from` into `to`, overwriting, and note each file (relative to `root`).
/// A file already there is moved into `backup` first, unless it is in `own`
/// (this add-on's files from an earlier apply: its original is kept already).
fn move_recording(from: &Path, to: &Path, root: &Path, backup: &Path, own: &[String], done: &mut Applied) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            move_recording(&entry.path(), &target, root, backup, own, done)?;
            continue;
        }
        let rel = target.strip_prefix(root).map(|r| r.to_string_lossy().replace('\\', "/")).ok();
        if target.exists() {
            match rel.as_ref().filter(|r| !own.contains(r)) {
                Some(rel) => {
                    let keep = backup.join(rel);
                    if let Some(parent) = keep.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::rename(&target, &keep)?;
                    done.replaced.push(rel.clone());
                }
                None => std::fs::remove_file(&target)?,
            }
        }
        std::fs::rename(entry.path(), &target)?;
        if let Some(rel) = rel {
            done.files.push(rel);
        }
    }
    Ok(())
}

/// The game's own folder: the top folder in its library folder, or where it is.
fn game_root<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> PathBuf {
    let folders = crate::settings::all_folders(&crate::settings::load(app));
    crate::storage::game_folder(&folders, &game.install_dir).unwrap_or_else(|| PathBuf::from(&game.install_dir))
}

/// Unpack a downloaded add-on into its game (called by the download runner).
pub async fn install<R: Runtime>(app: &AppHandle<R>, id: &str, settings: &crate::settings::Settings) -> Result<(), String> {
    let item = downloads::snapshot(app, id).ok_or("The download is gone.")?;
    let label = item.addon.clone().unwrap_or_else(|| "Add-on".into());
    let games = library::load(app).unwrap_or_default();
    // The game it was added to from its card; else the one with its page.
    let game = item
        .game_id
        .as_ref()
        .and_then(|id| games.iter().find(|g| &g.id == id))
        .or_else(|| item.slug.as_ref().and_then(|slug| games.iter().find(|g| g.slug.as_deref() == Some(slug.as_str()))))
        .cloned()
        .ok_or_else(|| {
            // A row named from its file reads "Forts DLC - Firebird": the game comes first.
            let game = item.meta.title.split(" DLC - ").next().unwrap_or(&item.meta.title);
            format!("{label} is an add-on. Install {game} first, then download it again.")
        })?;
    // Its files are in use while it runs. The archive stays, so Retry is quick.
    if library::is_running(app, &game.id) {
        return Err(downloads::close_first(&game.title));
    }
    downloads::set_status_pub(app, id, Status::Extracting, None);
    // 7-Zip on hand, as for a game: faster, and it reads every filter.
    #[cfg(windows)]
    downloads::ensure_7zr(app).await;
    if item.repair {
        return install_fix(app, id, settings, &item, &game, &label).await;
    }

    let root = game_root(app, &game);
    let staging = root.with_file_name(format!(".{}.addon", downloads::safe_name(&game.title)));
    let archive = PathBuf::from(&item.archive_path);
    let (root_c, staging_c) = (root.clone(), staging.clone());
    let game_name = root.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let progress_app = app.clone();
    let pid = id.to_string();
    // Applied before (a DLC downloaded again): its files are its own, and the
    // originals they replaced are already kept.
    let before = game.addons.iter().find(|a| a.file == item.file_name).cloned().unwrap_or_default();
    let backup = backup_dir(&root, &item.file_name);
    let own = before.files.clone();
    let applied = tauri::async_runtime::spawn_blocking(move || -> Result<Applied, String> {
        let _ = std::fs::remove_dir_all(&staging_c);
        downloads::extract(&archive, &staging_c, &|done, total| downloads::extract_progress(&progress_app, &pid, done, total))?;
        // Only unwrap a single top folder when it is the game's own folder
        // name; an add-on made of one folder ("Binaries") keeps it.
        let top = downloads::single_child_dir(&staging_c)
            .filter(|d| d.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == game_name))
            .unwrap_or_else(|| staging_c.clone());
        let mut done = Applied::default();
        move_recording(&top, &root_c, &root_c, &backup, &own, &mut done).map_err(|e| format!("Applying the add-on failed: {e}"))?;
        let _ = std::fs::remove_dir_all(&staging_c);
        Ok(done)
    })
    .await
    .map_err(|e| e.to_string())??;

    let file = item.file_name.clone();
    let count = applied.files.len();
    let mut replaced = before.replaced;
    replaced.extend(applied.replaced);
    replaced.sort();
    replaced.dedup();
    let mut files = applied.files;
    files.extend(before.files);
    files.sort();
    files.dedup();
    library::update(app, |games| {
        if let Some(g) = games.iter_mut().find(|g| g.id == game.id) {
            g.addons.retain(|a| a.file != file);
            g.addons.push(InstalledAddon { label: label.clone(), file: file.clone(), files, replaced, installed_at: now() });
        }
        Ok(())
    })?;
    if settings.delete_archives {
        let _ = std::fs::remove_file(&item.archive_path);
    }
    downloads::finish_as(app, id, &game);
    crate::logging::info("addons", &format!("{label} applied to {} ({count} files)", game.title));
    let _ = app.emit("library-changed", ());
    let _ = app.emit(
        "notify",
        downloads::Notice::new(&format!("{label} applied"), &format!("Added to {}.", game.title), Some(game.id.clone())),
    );
    Ok(())
}

/// A game's add-ons on kryo.to (`addons` in `/api/games/<slug>/downloads`),
/// asked as the signed-in app so the answer names their files, which is how
/// an applied one is recognised. Signed out, kryo.to answers without them.
#[tauri::command]
pub async fn addons_list(app: AppHandle, slug: String) -> Result<serde_json::Value, String> {
    if slug.is_empty() || !slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("That is not a kryo.to game.".into());
    }
    let json = match crate::phone::session(&app) {
        Ok(session) => crate::phone::listing(&session, &slug).await?,
        Err(_) => {
            let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(&app));
            let client = reqwest::Client::builder()
                .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| e.to_string())?;
            client
                .get(format!("{endpoint}/api/games/{slug}/downloads"))
                .send()
                .await
                .map_err(|e| format!("Could not reach kryo.to: {e}"))?
                .json::<serde_json::Value>()
                .await
                .map_err(|e| e.to_string())?
        }
    };
    Ok(json.get("addons").cloned().unwrap_or_else(|| serde_json::json!([])))
}

/// What pressing Add on a game's card did.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonsGot {
    /// The download rows now carrying them, one per add-on queued.
    pub queued: Vec<String>,
    /// Some could not be fetched by the app itself (signed out, or only on a
    /// host Kryoto cannot download from): the game's download window can.
    pub store: bool,
}

/// "1.2 GB" as kryo.to writes a size, in bytes. Only for the progress bar
/// until the file's own length is known.
fn size_bytes(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace();
    let n: f64 = parts.next()?.replace(',', ".").parse().ok()?;
    let unit = match parts.next()?.to_ascii_uppercase().as_str() {
        "B" => 1.0,
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        "TB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    (n.is_finite() && n > 0.0).then_some((n * unit) as u64)
}

/// The rows to queue for `ids` from kryo.to's `addons` list. Ours (kryo.to's
/// own filehost) first; else a mirror Kryoto downloads from by itself; an
/// add-on with neither is left to the download window (`true`). Pure, for the tests.
fn requests_for(addons: &[serde_json::Value], ids: &[String]) -> (Vec<downloads::AddonRequest>, bool) {
    let id_of = |a: &serde_json::Value| match &a["id"] {
        serde_json::Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    let mut out = Vec::new();
    let mut store = false;
    for want in ids {
        let Some(addon) = addons.iter().find(|a| &id_of(a) == want) else { continue };
        let ours = addon["links"].as_array().is_some_and(|l| !l.is_empty());
        let mirror = if ours {
            None
        } else {
            let page = addon["pages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p.as_str())
                .find_map(|p| crate::resolvers::classify(p).map(|(_, host)| (p.to_string(), host.to_string())));
            match page {
                Some((page, host)) => Some(downloads::Mirror { page, host }),
                None => {
                    store = true;
                    continue;
                }
            }
        };
        out.push(downloads::AddonRequest {
            release: want.clone(),
            label: addon["label"].as_str().filter(|l| !l.trim().is_empty()).unwrap_or("Add-on").to_string(),
            repair: addon["kind"].as_str() == Some("repair"),
            mirror,
            total: addon["download_size"].as_str().and_then(size_bytes),
        });
    }
    (out, store)
}

/// Add-ons pressed on an installed game's card (one, or "Add all DLC"): each
/// is queued for this game and applies to its folder when it lands. Nothing
/// opens; the card follows the rows. Signed out there is no link to mint, so
/// the answer says to use the game's download window instead.
#[tauri::command]
pub async fn addons_get(app: AppHandle, game_id: String, ids: Vec<String>) -> Result<AddonsGot, String> {
    let game = library::load(&app)?.into_iter().find(|g| g.id == game_id).ok_or("That game is no longer in the library.")?;
    let slug = game.slug.clone().ok_or("This game is not from kryo.to, so it has no add-ons there.")?;
    if library::is_running(&app, &game.id) {
        return Err(format!("Close {} first: its files are in use.", game.title));
    }
    let Ok(session) = crate::phone::session(&app) else {
        return Ok(AddonsGot { queued: Vec::new(), store: true });
    };
    let listing = crate::phone::listing(&session, &slug).await?;
    let addons = listing["addons"].as_array().cloned().unwrap_or_default();
    let (requests, store) = requests_for(&addons, &ids);
    if requests.is_empty() && !store {
        return Err("kryo.to no longer has that add-on. Reopen the game's page to see what it has now.".into());
    }
    let mut queued = Vec::new();
    for req in requests {
        let label = req.label.clone();
        queued.push(downloads::enqueue_addon(&app, &game, req)?);
        crate::logging::info("addons", &format!("{label} queued for {} from its card", game.title));
    }
    Ok(AddonsGot { queued, store })
}

/// A repair fix (kryo.to kind `repair`: Online-Fix's generic fix, packed by
/// Forge) goes in as a repair rather than an add-on: checked against the game
/// (`kryoto_repair::fixpack`: its own app id, the game's folder layout, the
/// architecture) and copied under the repair journal, so a failure puts every
/// replaced file back and Properties > Repair can undo it, Steam's own dlls
/// included. A plain add-on's undo only deletes what it wrote, which would
/// leave the game without the dlls the fix replaced.
async fn install_fix<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    settings: &crate::settings::Settings,
    item: &downloads::Download,
    game: &LibraryGame,
    label: &str,
) -> Result<(), String> {
    use kryoto_repair::fixpack;
    let root = game_root(app, game);
    let staging = root.with_file_name(format!(".{}.fix", downloads::safe_name(&game.title)));
    let archive = PathBuf::from(&item.archive_path);
    let progress_app = app.clone();
    let pid = id.to_string();
    let (root_c, staging_c) = (root.clone(), staging.clone());
    let plan = tauri::async_runtime::spawn_blocking(move || -> Result<fixpack::Plan, String> {
        let _ = std::fs::remove_dir_all(&staging_c);
        let result = (|| {
            downloads::extract(&archive, &staging_c, &|done, total| downloads::extract_progress(&progress_app, &pid, done, total))?;
            let manifest = fixpack::read_manifest(&staging_c)
                .ok_or("This fix has no kryoto-fix.json, so Kryoto cannot tell which game it is for.")?;
            let scope = fixpack::scope_of_dir(&staging_c)?;
            let plan = fixpack::plan(&staging_c, &scope, &root_c, &manifest.steam_appid)?;
            fixpack::apply(&staging_c, &root_c, &plan, &manifest.package_sha256.chars().take(12).collect::<String>())?;
            Ok::<_, String>(plan)
        })();
        let _ = std::fs::remove_dir_all(&staging_c);
        result
    })
    .await
    .map_err(|e| e.to_string())?;
    let plan = plan.map_err(|e| {
        crate::logging::error("addons", &format!("{label} on {}: {e}", game.title));
        e
    })?;
    if settings.delete_archives {
        let _ = std::fs::remove_file(&item.archive_path);
    }
    downloads::finish_as(app, id, game);
    crate::logging::info("addons", &format!("{label} applied to {} as a repair ({} files)", game.title, plan.placements.len()));
    let _ = app.emit("library-changed", ());
    let _ = app.emit(
        "notify",
        downloads::Notice::new(
            &format!("{label} applied"),
            &format!("{} is set up for Online-Fix. Steam must be running. Undo it under Properties > Repair.", game.title),
            Some(game.id.clone()),
        ),
    );
    Ok(())
}

/// Take an add-on out again: delete every file it wrote, put back the game's
/// files it replaced, then remove any folders it left empty.
#[tauri::command(async)]
pub fn addon_undo(app: AppHandle, game_id: String, file: String) -> Result<LibraryGame, String> {
    let game = library::load(&app)?.into_iter().find(|g| g.id == game_id).ok_or("That game is no longer in the library.")?;
    let addon = game.addons.iter().find(|a| a.file == file).cloned().ok_or("That add-on is not applied.")?;
    // In use, some of its files would stay and others go.
    if library::is_running(&app, &game.id) {
        return Err("Close the game first.".into());
    }
    let root = game_root(&app, &game);
    remove(&root, &addon)?;
    crate::logging::info("addons", &format!("{} removed from {} ({} files put back)", addon.label, game.title, addon.replaced.len()));
    let updated = library::update(&app, |games| {
        let g = games.iter_mut().find(|g| g.id == game_id).ok_or("That game is no longer in the library.")?;
        g.addons.retain(|a| a.file != file);
        Ok(g.clone())
    })?;
    let _ = app.emit("library-changed", ());
    Ok(updated)
}

/// Undo on disk: the add-on's files out, the originals back, its empty folders gone.
fn remove(root: &Path, addon: &InstalledAddon) -> Result<(), String> {
    let backup = backup_dir(root, &addon.file);
    let mut dirs: Vec<PathBuf> = Vec::new();
    for rel in &addon.files {
        let path = crate::launch::inside(root, rel)?;
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Could not delete {rel}: {e}")),
        }
        let mut parent = path.parent().map(Path::to_path_buf);
        while let Some(p) = parent {
            if p == root || !p.starts_with(root) {
                break;
            }
            dirs.push(p.clone());
            parent = p.parent().map(Path::to_path_buf);
        }
    }
    // Deepest first; remove_dir only succeeds on an empty folder.
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    dirs.dedup();
    for d in dirs {
        let _ = std::fs::remove_dir(d);
    }
    for rel in &addon.replaced {
        let kept = crate::launch::inside(&backup, rel)?;
        let path = crate::launch::inside(root, rel)?;
        if !kept.is_file() {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Could not put back {rel}: {e}"))?;
        }
        std::fs::rename(&kept, &path).map_err(|e| format!("Could not put back {rel}: {e}"))?;
    }
    let _ = std::fs::remove_dir_all(&backup);
    let _ = std::fs::remove_dir(backups_of(root));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_addon_is_queued_from_our_copy_else_a_mirror_else_left_to_the_window() {
        let addons = vec![
            serde_json::json!({ "id": 11, "label": "DLC: Firebird", "kind": "addon", "download_size": "1.5 GB", "links": [{ "name": "Forts DLC - Firebird - Kryoto.7z" }], "pages": ["https://pixeldrain.com/u/abc"] }),
            serde_json::json!({ "id": "12", "label": "", "kind": "repair", "download_size": null, "links": [], "pages": ["https://pixeldrain.com/u/def"] }),
            serde_json::json!({ "id": 13, "label": "DLC: Torrent only", "links": [], "pages": ["https://example.com/thing.torrent"] }),
        ];
        let ids = ["11", "12", "13", "99"].map(String::from);
        let (reqs, store) = requests_for(&addons, &ids);
        assert!(store, "the add-on Kryoto cannot fetch goes to the download window");
        assert_eq!(reqs.len(), 2);
        assert_eq!((reqs[0].release.as_str(), reqs[0].label.as_str()), ("11", "DLC: Firebird"));
        assert!(reqs[0].mirror.is_none() && !reqs[0].repair);
        assert_eq!(reqs[0].total, Some(1_610_612_736));
        assert_eq!(reqs[1].label, "Add-on");
        assert!(reqs[1].repair);
        assert_eq!(reqs[1].mirror.as_ref().map(|m| m.page.as_str()), Some("https://pixeldrain.com/u/def"));
        assert_eq!(size_bytes("300 MB"), Some(314_572_800));
        assert_eq!(size_bytes("about"), None);
    }

    #[test]
    fn undo_deletes_what_it_added_and_puts_back_what_it_replaced() {
        let base = std::env::temp_dir().join("kryoto-addon-test");
        let _ = std::fs::remove_dir_all(&base);
        let game = base.join("Game");
        let addon = base.join("addon");
        std::fs::create_dir_all(game.join("bin")).unwrap();
        std::fs::write(game.join("bin/steam_api64.dll"), "original").unwrap();
        std::fs::write(game.join("save.dat"), "save").unwrap();
        std::fs::create_dir_all(addon.join("bin")).unwrap();
        std::fs::create_dir_all(addon.join("lang/ja")).unwrap();
        std::fs::write(addon.join("bin/steam_api64.dll"), "online").unwrap();
        std::fs::write(addon.join("lang/ja/text.pak"), "ja").unwrap();

        let backup = backup_dir(&game, "Game - DLC Japan - kryoto.7z");
        let mut done = Applied::default();
        move_recording(&addon, &game, &game, &backup, &[], &mut done).unwrap();
        done.files.sort();
        assert_eq!(done.files, ["bin/steam_api64.dll", "lang/ja/text.pak"]);
        assert_eq!(done.replaced, ["bin/steam_api64.dll"]);
        assert_eq!(std::fs::read_to_string(game.join("bin/steam_api64.dll")).unwrap(), "online");

        // Applied again (downloaded twice): the kept original is not overwritten.
        std::fs::create_dir_all(addon.join("bin")).unwrap();
        std::fs::write(addon.join("bin/steam_api64.dll"), "online 2").unwrap();
        let mut again = Applied::default();
        move_recording(&addon, &game, &game, &backup, &done.files, &mut again).unwrap();
        assert!(again.replaced.is_empty());

        let record = InstalledAddon { label: "DLC: Japan".into(), file: "Game - DLC Japan - kryoto.7z".into(), files: done.files, replaced: done.replaced, installed_at: 0 };
        remove(&game, &record).unwrap();
        assert_eq!(std::fs::read_to_string(game.join("bin/steam_api64.dll")).unwrap(), "original");
        assert!(!game.join("lang").exists());
        assert!(!backups_of(&game).exists());
        assert!(game.join("save.dat").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
