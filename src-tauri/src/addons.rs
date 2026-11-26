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
    /// Its files the game's last update wrote over, taken off the two lists
    /// above: they are the new build's now. Go back to the previous build
    /// puts this add-on's copies back, and these with them (`take_back`).
    #[serde(skip_serializing_if = "WrittenOver::is_empty")]
    pub updated: WrittenOver,
}

/// The paths a game update took off an add-on's `files` and `replaced`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WrittenOver {
    pub files: Vec<String>,
    pub replaced: Vec<String>,
}

impl WrittenOver {
    fn is_empty(&self) -> bool {
        self.files.is_empty() && self.replaced.is_empty()
    }
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

/// The game's folder `from` is now `to` (moved or renamed): its add-on
/// backups go along, under the new name, or Undo would find none of the
/// originals they keep. A rename when it can, else a copy and the old one gone.
/// Once the copy is whole it is the one Undo reads, so an old file that will
/// not go is only left behind, not a failure.
pub(crate) fn carry_backups(from: &Path, to: &Path) -> Result<(), String> {
    let (old, new) = (backups_of(from), backups_of(to));
    if !old.is_dir() {
        return Ok(());
    }
    if std::fs::rename(&old, &new).is_ok() {
        return Ok(());
    }
    if let Err(e) = crate::storage::copy_tree(&old, &new, &mut |_| {}) {
        let _ = std::fs::remove_dir_all(&new);
        return Err(format!("Could not move its add-ons' kept files: {e}"));
    }
    if let Err(e) = std::fs::remove_dir_all(&old) {
        crate::logging::warn("addons", &format!("Could not remove the old copy of its add-ons' kept files at {}: {e}", old.display()));
    }
    Ok(())
}

/// The files of `addons` a game update is about to write over: `staging` is
/// the unpacked update, merged into `dest`. Relative to `root`, as recorded.
pub(crate) fn shipped(addons: &[InstalledAddon], root: &Path, staging: &Path, dest: &Path) -> Vec<String> {
    let (root, dest) = (dunce::simplified(root), dunce::simplified(dest));
    let in_update = |rel: &String| {
        crate::launch::inside(root, rel).ok().and_then(|p| p.strip_prefix(dest).ok().map(|r| staging.join(r))).is_some_and(|p| p.is_file())
    };
    let mut out: Vec<String> = addons.iter().flat_map(|a| a.files.iter()).filter(|rel| in_update(rel)).cloned().collect();
    out.sort();
    out.dedup();
    out
}

/// A game update wrote its own files over `paths` (from `shipped`): they are
/// the new build's now. Each add-on lets go of them, so Undo neither deletes
/// the new build's file nor puts the old build's back over it. What it let go
/// is noted in `updated`, and the originals it kept stay (Undo clears them
/// with the rest): Go back to the previous build makes them its own again.
/// Run on every update, even one that writes over none, so `updated` is only
/// ever the last update's.
pub(crate) fn let_go(addons: &mut [InstalledAddon], paths: &[String]) {
    for a in addons.iter_mut() {
        a.updated = WrittenOver {
            files: a.files.iter().filter(|r| paths.contains(r)).cloned().collect(),
            replaced: a.replaced.iter().filter(|r| paths.contains(r)).cloned().collect(),
        };
        a.files.retain(|r| !paths.contains(r));
        a.replaced.retain(|r| !paths.contains(r));
    }
}

/// The game's last update was undone, or rolled back after it was cut off:
/// its backup put back the files it had replaced (`restored`, relative to
/// `dest`), the add-ons' copies among them. Those are the add-ons' again, so
/// Undo takes them out and puts the game's originals back.
pub(crate) fn take_back(addons: &mut [InstalledAddon], root: &Path, dest: &Path, restored: &[String]) {
    let (root, dest) = (dunce::simplified(root), dunce::simplified(dest));
    let put_back = |rel: &String| {
        crate::launch::inside(root, rel)
            .ok()
            .and_then(|p| p.strip_prefix(dest).ok().map(|r| r.to_string_lossy().replace('\\', "/")))
            .is_some_and(|r| restored.contains(&r))
    };
    for a in addons.iter_mut() {
        let over = std::mem::take(&mut a.updated);
        a.files.extend(over.files.into_iter().filter(|r| put_back(r)));
        a.replaced.extend(over.replaced.into_iter().filter(|r| put_back(r)));
        for list in [&mut a.files, &mut a.replaced] {
            list.sort();
            list.dedup();
        }
    }
}

/// `take_back` for the game a game update's journal belongs to.
pub(crate) fn update_undone<R: Runtime>(app: &AppHandle<R>, journal: &crate::game_update::Journal) {
    let dest = PathBuf::from(&journal.dest);
    let _ = library::update(app, |games| {
        if let Some(g) = games.iter_mut().find(|g| g.slug.as_deref() == Some(journal.slug.as_str())) {
            let root = game_root(app, g);
            take_back(&mut g.addons, &root, &dest, &journal.replaced);
        }
        Ok(())
    });
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
/// A file in `under` goes to the path given with it instead (see `under`).
fn move_recording(from: &Path, to: &Path, root: &Path, backup: &Path, own: &[String], under: &[(String, PathBuf)], done: &mut Applied) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            move_recording(&entry.path(), &target, root, backup, own, under, done)?;
            continue;
        }
        let rel = target.strip_prefix(root).map(|r| r.to_string_lossy().replace('\\', "/")).ok();
        if let Some((rel, kept)) = under.iter().find(|(r, _)| Some(r) == rel.as_ref()) {
            if let Some(parent) = kept.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(entry.path(), kept)?;
            done.files.push(rel.clone());
            continue;
        }
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

/// An add-on applied again: of its files (`own`), those an add-on applied
/// after it (`later`, in order) wrote over, with where that one keeps what it
/// replaced. The new copy goes there, under it, as when they first went on in
/// that order: the later one stays in the game, and its Undo shows this one's.
fn under(root: &Path, later: &[InstalledAddon], own: &[String]) -> Vec<(String, PathBuf)> {
    own.iter()
        .filter_map(|rel| {
            let above = later.iter().find(|a| a.files.contains(rel))?;
            Some((rel.clone(), crate::launch::inside(&backup_dir(root, &above.file), rel).ok()?))
        })
        .collect()
}

/// The game's own folder: the top folder in its library folder, or where it is.
pub(crate) fn game_root<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> PathBuf {
    let folders = crate::settings::all_folders(&crate::settings::load(app));
    crate::storage::game_folder(&folders, game).unwrap_or_else(|| PathBuf::from(&game.install_dir))
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
            // A row named from its file reads "Forts DLC - Firebird" or
            // "Forts - DLC Firebird": the game comes first.
            let game = downloads::dlc_parts(&item.meta.title).map(|(game, _)| game).unwrap_or_else(|| item.meta.title.clone());
            if item.from_file {
                format!("{label} is an add-on for {game}. Install {game} first, then press Retry here. Your file stays where it is.")
            } else {
                format!("{label} is an add-on. Install {game} first, then download it again.")
            }
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
    let at = game.addons.iter().position(|a| a.file == item.file_name);
    let before = at.map(|i| game.addons[i].clone()).unwrap_or_default();
    let backup = backup_dir(&root, &item.file_name);
    let own = before.files.clone();
    let under = at.map(|i| under(&root, &game.addons[i + 1..], &own)).unwrap_or_default();
    let applied = tauri::async_runtime::spawn_blocking(move || -> Result<Applied, String> {
        let _ = std::fs::remove_dir_all(&staging_c);
        downloads::extract(&archive, &staging_c, &|done, total| downloads::extract_progress(&progress_app, &pid, done, total))?;
        // Only unwrap a single top folder when it is the game's own folder
        // name; an add-on made of one folder ("Binaries") keeps it.
        let top = downloads::single_child_dir(&staging_c)
            .filter(|d| d.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == game_name))
            .unwrap_or_else(|| staging_c.clone());
        let mut done = Applied::default();
        move_recording(&top, &root_c, &root_c, &backup, &own, &under, &mut done).map_err(|e| format!("Applying the add-on failed: {}", crate::logging::why(&e)))?;
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
            let addon = InstalledAddon { label: label.clone(), file: file.clone(), files, replaced, installed_at: now(), updated: before.updated.clone() };
            // Applied again, it keeps its place: the list is the order the
            // add-ons first went on, which is how their kept originals stack.
            match g.addons.iter_mut().find(|a| a.file == file) {
                Some(slot) => *slot = addon,
                None => g.addons.push(addon),
            }
        }
        Ok(())
    })?;
    if let Some(path) = downloads::own_archive(&item).filter(|_| settings.delete_archives) {
        let _ = std::fs::remove_file(path);
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
    // A game kryo.to took off has no add-ons to list, and is not asked again
    // for a day (catalog_gone.rs): its Library page asked on every visit.
    let none = || serde_json::json!([]);
    if crate::catalog_gone::is_gone(&app, &slug) {
        return Ok(none());
    }
    let json = match crate::phone::session(&app) {
        Ok(session) => match crate::phone::listing(&session, &slug).await {
            Ok(json) => json,
            Err(e) if crate::phone::listing_says_gone(&e) => {
                crate::catalog_gone::set(&app, &slug, true);
                return Ok(none());
            }
            Err(e) => return Err(e),
        },
        Err(_) => {
            let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(&app));
            let client = crate::net::builder()
                .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| e.to_string())?;
            let res = client
                .get(format!("{endpoint}/api/games/{slug}/downloads"))
                .send()
                .await
                .map_err(|e| format!("Could not reach kryo.to: {}", crate::logging::why(&e)))?;
            if crate::catalog_gone::says_gone(res.status().as_u16()) {
                crate::catalog_gone::set(&app, &slug, true);
                return Ok(none());
            }
            res.json::<serde_json::Value>().await.map_err(|e| e.to_string())?
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

/// "1.2 GB" as kryo.to writes a size, in bytes: the progress bar until the
/// file's own length is known, and a game's install size (downloads.rs).
pub(crate) fn size_bytes(text: &str) -> Option<u64> {
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
    crate::required::refuse()?;
    let game = library::load(&app)?.into_iter().find(|g| g.id == game_id).ok_or("That game is no longer in the library.")?;
    let slug = game.slug.clone().ok_or("This game is not from kryo.to, so it has no add-ons there.")?;
    if library::is_running(&app, &game.id) {
        return Err(format!("Close {} first: its files are in use.", game.title));
    }
    // Only a signed-in app can mint them (KB-0326): a guest gets the window.
    let Some(session) = crate::phone::session(&app).ok().filter(|s| s.signed_in()) else {
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
    if let Some(path) = downloads::own_archive(item).filter(|_| settings.delete_archives) {
        let _ = std::fs::remove_file(path);
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
    let at = game.addons.iter().position(|a| a.file == file).ok_or("That add-on is not applied.")?;
    let addon = game.addons[at].clone();
    // In use, some of its files would stay and others go.
    if library::is_running(&app, &game.id) {
        return Err("Close the game first.".into());
    }
    let root = game_root(&app, &game);
    let mut left = addon.clone();
    let mut later = game.addons[at + 1..].to_vec();
    let undone = remove(&root, &mut left, &mut later);
    // Saved whether or not it got to the end: what it did is done, and the
    // record says what is left, so Undo again carries on from there.
    let updated = library::update(&app, |games| {
        let g = games.iter_mut().find(|g| g.id == game_id).ok_or("That game is no longer in the library.")?;
        for a in g.addons.iter_mut() {
            if a.file == file {
                (a.files, a.replaced) = (left.files.clone(), left.replaced.clone());
            } else if let Some(after) = later.iter().find(|l| l.file == a.file) {
                // The add-ons on top of it now keep what it kept.
                a.replaced = after.replaced.clone();
            }
        }
        if undone.is_ok() {
            g.addons.retain(|a| a.file != file);
        }
        Ok(g.clone())
    })?;
    let _ = app.emit("library-changed", ());
    let Undone { put_back, missing } = undone?;
    crate::logging::info("addons", &format!("{} removed from {} ({put_back} files put back)", addon.label, game.title));
    // Said, not skipped quietly: the game is missing these files now.
    if !missing.is_empty() {
        crate::logging::warn("addons", &format!("{}: the game's own {} not found to put back", addon.label, missing.join(", ")));
        let _ = app.emit(
            "notify",
            downloads::Notice::new(
                &format!("{} removed, with files missing", addon.label),
                &format!(
                    "Kryoto could not find the game's own {} to put back. Get the files back under Properties > Emulator & fixes > Files.",
                    missing.join(", ")
                ),
                Some(game.id.clone()),
            ),
        );
    }
    Ok(updated)
}

/// What an Undo did with the game's files it had replaced.
#[derive(Debug, Default)]
struct Undone {
    put_back: usize,
    /// Originals it had no kept copy of (see `remove`).
    missing: Vec<String>,
}

/// Undo on disk: the add-on's files out, the originals back, its empty folders gone.
///
/// `later` are the add-ons applied after it, in order. One that wrote over a
/// file of this one kept this one's copy as its original, so that file stays:
/// the first such add-on is given this one's original instead (or none, when
/// this one added the file), and its own Undo then puts the game's file back.
///
/// Each step is struck off `addon` as it is done. When one fails (a file in
/// use), the caller saves what is left, and Undo again carries on from there
/// instead of doing a step twice: handing over an original it already handed
/// over, or deleting one it already put back.
///
/// Says which originals it should put back and found no copy of (their kept
/// files were lost, as a Move or Rename folder used to leave them), so the
/// player hears which of the game's files are missing.
fn remove(root: &Path, addon: &mut InstalledAddon, later: &mut [InstalledAddon]) -> Result<Undone, String> {
    let backup = backup_dir(root, &addon.file);
    let mut dirs: Vec<PathBuf> = Vec::new();
    let taken_out = take_out(root, &backup, addon, later, &mut dirs);
    // Deepest first; remove_dir only succeeds on an empty folder.
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    dirs.dedup();
    for d in dirs {
        let _ = std::fs::remove_dir(d);
    }
    taken_out?;
    let mut undone = Undone::default();
    for rel in addon.replaced.clone() {
        let kept = crate::launch::inside(&backup, &rel)?;
        let path = crate::launch::inside(root, &rel)?;
        if !kept.is_file() {
            // Stays on the list: said again if this Undo has to be retried.
            undone.missing.push(rel);
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Could not put back {rel}: {e}"))?;
        }
        std::fs::rename(&kept, &path).map_err(|e| format!("Could not put back {rel}: {e}"))?;
        addon.replaced.retain(|r| r != &rel);
        undone.put_back += 1;
    }
    let _ = std::fs::remove_dir_all(&backup);
    let _ = std::fs::remove_dir(backups_of(root));
    Ok(undone)
}

/// The add-on's files out of the game, or handed over to the add-on above
/// them; the folders they were in go into `dirs`, to remove when empty.
fn take_out(root: &Path, backup: &Path, addon: &mut InstalledAddon, later: &mut [InstalledAddon], dirs: &mut Vec<PathBuf>) -> Result<(), String> {
    for rel in addon.files.clone() {
        if let Some(above) = later.iter_mut().find(|a| a.files.contains(&rel)) {
            hand_over(root, backup, addon.replaced.contains(&rel), above, &rel)?;
            addon.replaced.retain(|r| r != &rel);
        } else {
            let path = crate::launch::inside(root, &rel)?;
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
        addon.files.retain(|r| r != &rel);
    }
    Ok(())
}

/// `rel` stays as `above` wrote it. What `above` kept for it was this
/// add-on's copy; it keeps the game's original instead, or nothing when this
/// add-on added the file (`replaced` false), so its Undo deletes it. An
/// original that was lost stays on `above`'s list with nothing kept, so its
/// Undo says the game's file is missing rather than putting this one's back.
fn hand_over(root: &Path, backup: &Path, replaced: bool, above: &mut InstalledAddon, rel: &String) -> Result<(), String> {
    let theirs = crate::launch::inside(&backup_dir(root, &above.file), rel)?;
    let kept = crate::launch::inside(backup, rel)?;
    if replaced && kept.is_file() {
        if let Some(parent) = theirs.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Could not keep {rel}: {e}"))?;
        }
        std::fs::rename(&kept, &theirs).map_err(|e| format!("Could not keep {rel}: {e}"))?;
    } else {
        match std::fs::remove_file(&theirs) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Could not let go of {rel}: {e}")),
        }
    }
    if !replaced {
        above.replaced.retain(|r| r != rel);
    } else if !above.replaced.contains(rel) {
        above.replaced.push(rel.clone());
        above.replaced.sort();
    }
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
        move_recording(&addon, &game, &game, &backup, &[], &[], &mut done).unwrap();
        done.files.sort();
        assert_eq!(done.files, ["bin/steam_api64.dll", "lang/ja/text.pak"]);
        assert_eq!(done.replaced, ["bin/steam_api64.dll"]);
        assert_eq!(std::fs::read_to_string(game.join("bin/steam_api64.dll")).unwrap(), "online");

        // Applied again (downloaded twice): the kept original is not overwritten.
        std::fs::create_dir_all(addon.join("bin")).unwrap();
        std::fs::write(addon.join("bin/steam_api64.dll"), "online 2").unwrap();
        let mut again = Applied::default();
        move_recording(&addon, &game, &game, &backup, &done.files, &[], &mut again).unwrap();
        assert!(again.replaced.is_empty());

        let mut record = InstalledAddon { label: "DLC: Japan".into(), file: "Game - DLC Japan - kryoto.7z".into(), files: done.files, replaced: done.replaced, ..Default::default() };
        assert!(remove(&game, &mut record, &mut []).unwrap().missing.is_empty());
        assert_eq!(std::fs::read_to_string(game.join("bin/steam_api64.dll")).unwrap(), "original");
        assert!(!game.join("lang").exists());
        assert!(!backups_of(&game).exists());
        assert!(game.join("save.dat").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Every file under `dir` with its text, relative and with `/`.
    fn tree(dir: &Path) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        let mut todo = vec![dir.to_path_buf()];
        while let Some(d) = todo.pop() {
            for entry in std::fs::read_dir(&d).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    todo.push(path);
                } else {
                    let rel = path.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
                    out.insert(rel, std::fs::read_to_string(&path).unwrap());
                }
            }
        }
        out
    }

    /// Apply an add-on of `files` to `game` as `install` does, and its record.
    fn apply(game: &Path, label: &str, file: &str, files: &[(&str, &str)]) -> InstalledAddon {
        let src = game.with_file_name(format!("{file}.src"));
        for (rel, text) in files {
            std::fs::create_dir_all(src.join(rel).parent().unwrap()).unwrap();
            std::fs::write(src.join(rel), text).unwrap();
        }
        let mut done = Applied::default();
        move_recording(&src, game, game, &backup_dir(game, file), &[], &[], &mut done).unwrap();
        std::fs::remove_dir_all(&src).unwrap();
        done.files.sort();
        InstalledAddon { label: label.into(), file: file.into(), files: done.files, replaced: done.replaced, ..Default::default() }
    }

    /// The add-on `file` in `list` downloaded again with `files`, as `install` does.
    fn apply_again(game: &Path, list: &mut [InstalledAddon], file: &str, files: &[(&str, &str)]) {
        let at = list.iter().position(|a| a.file == file).unwrap();
        let src = game.with_file_name(format!("{file}.src"));
        for (rel, text) in files {
            std::fs::create_dir_all(src.join(rel).parent().unwrap()).unwrap();
            std::fs::write(src.join(rel), text).unwrap();
        }
        let own = list[at].files.clone();
        let mut done = Applied::default();
        move_recording(&src, game, game, &backup_dir(game, file), &own, &under(game, &list[at + 1..], &own), &mut done).unwrap();
        std::fs::remove_dir_all(&src).unwrap();
        list[at].files.extend(done.files);
        list[at].replaced.extend(done.replaced);
        for l in [&mut list[at].files, &mut list[at].replaced] {
            l.sort();
            l.dedup();
        }
    }

    /// `addon_undo` on disk and in the list: what is left of it is kept when
    /// it fails, and it is gone from the list when it is done.
    fn try_undo(game: &Path, list: &mut Vec<InstalledAddon>, file: &str) -> Result<Undone, String> {
        let at = list.iter().position(|a| a.file == file).unwrap();
        let mut left = list[at].clone();
        let undone = remove(game, &mut left, &mut list[at + 1..]);
        list[at] = left;
        if undone.is_ok() {
            list.remove(at);
        }
        undone
    }

    fn undo(game: &Path, list: &mut Vec<InstalledAddon>, file: &str) {
        assert!(try_undo(game, list, file).unwrap().missing.is_empty());
    }

    #[test]
    fn two_addons_on_one_file_undo_to_the_original_in_either_order() {
        let base = std::env::temp_dir().join("kryoto-addon-stack-test");
        for order in ["a then b", "b then a", "a only", "language packs"] {
            let _ = std::fs::remove_dir_all(&base);
            let game = base.join("Game");
            std::fs::create_dir_all(game.join("lang")).unwrap();
            std::fs::write(game.join("lang/text.pak"), "game").unwrap();
            std::fs::write(game.join("game.exe"), "exe").unwrap();
            let vanilla = tree(&game);
            let (a, b) = if order == "language packs" { ("Language: Japanese", "Language: French") } else { ("DLC: A", "DLC: B") };
            // Both replace the game's text.pak, and both write a file the game does not have.
            let mut list = vec![apply(&game, a, "a.7z", &[("lang/text.pak", "A"), ("lang/extra.pak", "A")])];
            list.push(apply(&game, b, "b.7z", &[("lang/text.pak", "B"), ("lang/extra.pak", "B")]));
            assert_eq!(tree(&game)["lang/text.pak"], "B");
            match order {
                "b then a" => {
                    undo(&game, &mut list, "b.7z");
                    assert_eq!(tree(&game)["lang/text.pak"], "A");
                    undo(&game, &mut list, "a.7z");
                }
                "a only" => {
                    undo(&game, &mut list, "a.7z");
                    let now = tree(&game);
                    assert_eq!((now["lang/text.pak"].as_str(), now["lang/extra.pak"].as_str()), ("B", "B"), "B's files stay");
                    // B keeps the game's own text.pak now, and nothing for the file A added.
                    assert_eq!(list[0].replaced, ["lang/text.pak"]);
                    undo(&game, &mut list, "b.7z");
                }
                _ => {
                    undo(&game, &mut list, "a.7z");
                    undo(&game, &mut list, "b.7z");
                }
            }
            assert_eq!(tree(&game), vanilla, "{order}: the game's files as they were");
            assert!(!backups_of(&game).exists(), "{order}: no backups left");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_undo_that_stops_halfway_carries_on_without_losing_the_original() {
        let base = std::env::temp_dir().join("kryoto-addon-retry-test");
        for then in ["the same again", "the other first"] {
            let _ = std::fs::remove_dir_all(&base);
            let game = base.join("Game");
            std::fs::create_dir_all(game.join("lang")).unwrap();
            std::fs::write(game.join("lang/text.pak"), "game").unwrap();
            std::fs::write(game.join("game.exe"), "exe").unwrap();
            let vanilla = tree(&game);
            let mut list = vec![apply(&game, "Language: Japanese", "a.7z", &[("lang/text.pak", "A"), ("lang/z.pak", "A")])];
            list.push(apply(&game, "Language: French", "b.7z", &[("lang/text.pak", "B")]));
            // Something has z.pak open: here a folder in its place, which no file delete takes.
            std::fs::remove_file(game.join("lang/z.pak")).unwrap();
            std::fs::create_dir_all(game.join("lang/z.pak/open")).unwrap();
            assert!(try_undo(&game, &mut list, "a.7z").is_err());
            assert_eq!(list.len(), 2, "{then}: still applied");
            assert_eq!(tree(&game)["lang/text.pak"], "B");
            std::fs::remove_dir_all(game.join("lang/z.pak")).unwrap();
            std::fs::write(game.join("lang/z.pak"), "A").unwrap();
            if then == "the other first" {
                undo(&game, &mut list, "b.7z");
                assert_eq!(tree(&game)["lang/text.pak"], "game");
                undo(&game, &mut list, "a.7z");
            } else {
                undo(&game, &mut list, "a.7z");
                assert_eq!(tree(&game)["lang/text.pak"], "B");
                undo(&game, &mut list, "b.7z");
            }
            assert_eq!(tree(&game), vanilla, "{then}: the game's files as they were");
            assert!(!backups_of(&game).exists(), "{then}: no backups left");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn one_downloaded_again_under_another_stays_under_it() {
        let base = std::env::temp_dir().join("kryoto-addon-again-test");
        for order in ["a then b", "b then a"] {
            let _ = std::fs::remove_dir_all(&base);
            let game = base.join("Game");
            std::fs::create_dir_all(game.join("lang")).unwrap();
            std::fs::write(game.join("lang/text.pak"), "game").unwrap();
            let vanilla = tree(&game);
            let mut list = vec![apply(&game, "Language: Japanese", "a.7z", &[("lang/text.pak", "A")])];
            list.push(apply(&game, "Language: French", "b.7z", &[("lang/text.pak", "B")]));
            apply_again(&game, &mut list, "a.7z", &[("lang/text.pak", "A2")]);
            assert_eq!(tree(&game)["lang/text.pak"], "B", "the one applied last stays in the game");
            if order == "a then b" {
                undo(&game, &mut list, "a.7z");
                assert_eq!(tree(&game)["lang/text.pak"], "B", "French stays when Japanese goes");
                undo(&game, &mut list, "b.7z");
            } else {
                undo(&game, &mut list, "b.7z");
                assert_eq!(tree(&game)["lang/text.pak"], "A2");
                undo(&game, &mut list, "a.7z");
            }
            assert_eq!(tree(&game), vanilla, "{order}: the game's files as they were");
            assert!(!backups_of(&game).exists(), "{order}: no backups left");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Unpack a game update in `staging` over `game` as `downloads::install`
    /// does, with its backup in `backups` for Go back: what it wrote over.
    fn update(game: &Path, list: &mut [InstalledAddon], staging: &Path, backups: &Path) -> crate::game_update::Journal {
        let journal = crate::game_update::prepare(crate::game_update::Prepare {
            slug: "game",
            title: "Game",
            staging,
            dest: game,
            backup_root: backups,
            from_version: None,
            to_version: None,
        })
        .unwrap();
        let written = shipped(list, game, staging, game);
        for rel in journal.replaced.iter().chain(&journal.added) {
            std::fs::rename(staging.join(rel), game.join(rel)).unwrap();
        }
        let_go(list, &written);
        journal
    }

    #[test]
    fn undo_after_an_update_keeps_the_new_builds_files() {
        let base = std::env::temp_dir().join("kryoto-addon-update-test");
        for then in ["kept", "gone back", "back one of two"] {
            let _ = std::fs::remove_dir_all(&base);
            let game = base.join("Game");
            std::fs::create_dir_all(game.join("bin")).unwrap();
            std::fs::write(game.join("bin/text.pak"), "old build").unwrap();
            let vanilla = tree(&game);
            let mut list = vec![apply(&game, "Language: Japanese", "ja.7z", &[("bin/text.pak", "ja"), ("bin/ja.pak", "ja"), ("bin/font.ttf", "ja")])];
            let applied = list[0].clone();
            // The update ships text.pak and, new in this build, ja.pak; not font.ttf.
            let staging = base.join(".Game.installing");
            std::fs::create_dir_all(staging.join("bin")).unwrap();
            std::fs::write(staging.join("bin/text.pak"), "new build").unwrap();
            std::fs::write(staging.join("bin/ja.pak"), "new build").unwrap();
            assert_eq!(shipped(&list, &game, &staging, &game), ["bin/ja.pak", "bin/text.pak"]);
            let mut journal = update(&game, &mut list, &staging, &base.join("Backups"));
            assert_eq!(list[0].files, ["bin/font.ttf"]);
            assert!(list[0].replaced.is_empty(), "the old build's text.pak is not put back over the new one");
            match then {
                "kept" => {
                    undo(&game, &mut list, "ja.7z");
                    let now = tree(&game);
                    assert_eq!(now.len(), 2, "the add-on's own font is gone: {now:?}");
                    assert_eq!((now["bin/text.pak"].as_str(), now["bin/ja.pak"].as_str()), ("new build", "new build"));
                }
                "gone back" => {
                    // Go back to the previous build (or the rollback, at the next start, of an update cut off).
                    crate::game_update::restore(&mut journal).unwrap();
                    take_back(&mut list, &game, &game, &journal.replaced);
                    assert_eq!((&list[0].files, &list[0].replaced), (&applied.files, &applied.replaced));
                    undo(&game, &mut list, "ja.7z");
                    assert_eq!(tree(&game), vanilla, "the old build's own files, the add-on's gone");
                }
                _ => {
                    // A second update, of none of the add-on's files, then Go back: one step only.
                    std::fs::create_dir_all(staging.join("bin")).unwrap();
                    std::fs::write(staging.join("bin/text.pak"), "newer build").unwrap();
                    let mut second = update(&game, &mut list, &staging, &base.join("Backups 2"));
                    crate::game_update::restore(&mut second).unwrap();
                    take_back(&mut list, &game, &game, &second.replaced);
                    assert_eq!(list[0].files, ["bin/font.ttf"], "the first update's files stay the game's");
                    undo(&game, &mut list, "ja.7z");
                    let now = tree(&game);
                    assert_eq!((now.len(), now["bin/text.pak"].as_str()), (2, "new build"));
                }
            }
            assert!(!backups_of(&game).exists(), "{then}: no backups left");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn undo_after_a_move_or_rename_finds_the_originals() {
        let base = std::env::temp_dir().join("kryoto-addon-move-test");
        let _ = std::fs::remove_dir_all(&base);
        let game = base.join("Game’s Folder");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("text.pak"), "game").unwrap();
        let mut record = apply(&game, "Language: Japanese", "ja.7z", &[("text.pak", "ja")]);
        // Rename folder; a move to another library folder is the same, a level over.
        let renamed = base.join("Library 2").join("Games Folder");
        std::fs::create_dir_all(renamed.parent().unwrap()).unwrap();
        std::fs::rename(&game, &renamed).unwrap();
        carry_backups(&game, &renamed).unwrap();
        assert!(!backups_of(&game).exists() && backups_of(&renamed).is_dir());
        assert!(remove(&renamed, &mut record, &mut []).unwrap().missing.is_empty());
        assert_eq!(std::fs::read_to_string(renamed.join("text.pak")).unwrap(), "game");
        assert!(!backups_of(&renamed).exists());

        // Kept files that are gone are said, not skipped quietly.
        let mut record = apply(&renamed, "Language: Japanese", "ja.7z", &[("text.pak", "ja")]);
        std::fs::remove_dir_all(backups_of(&renamed)).unwrap();
        assert_eq!(remove(&renamed, &mut record, &mut []).unwrap().missing, ["text.pak"]);

        // Two on the file: the one under it lets go, the last one says so.
        std::fs::write(renamed.join("text.pak"), "game").unwrap();
        let mut list = vec![apply(&renamed, "Language: Japanese", "ja.7z", &[("text.pak", "ja")])];
        list.push(apply(&renamed, "Language: French", "fr.7z", &[("text.pak", "fr")]));
        std::fs::remove_file(backup_dir(&renamed, "ja.7z").join("text.pak")).unwrap();
        undo(&renamed, &mut list, "ja.7z");
        assert_eq!(std::fs::read_to_string(renamed.join("text.pak")).unwrap(), "fr");
        assert_eq!(try_undo(&renamed, &mut list, "fr.7z").unwrap().missing, ["text.pak"]);
        assert!(!renamed.join("text.pak").exists(), "not the Japanese pack's copy put back");
        let _ = std::fs::remove_dir_all(&base);
    }
}
