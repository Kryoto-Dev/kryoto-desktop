//! The updater behind community updates: brings a player's own copy of a
//! gbe_fork game up to Steam's newer build, then hands it to the offer flow
//! (community_offer.rs) through its record, `.kryoto-update.json`.
//!
//! The build comes from kryo.to's shared source pool (kryo.to
//! lib/community-source.ts), which only answers Kryoto Desktop, only for a
//! game whose Kryoto release is gbe_fork, only when Steam's build is newer, and
//! within the player's and the pool's daily share. The files come from Steam's
//! CDN through DepotDownloaderMod, the way Forge fetches them, into a staging
//! folder beside the game; nothing in the game changes until every depot is in.
//!
//!   * Steam's own steam_api dlls are never put in: the release's emulator
//!     stays. When Steam changed its original dll, gbe_fork is set up again
//!     around it the way Forge does: the new original becomes the
//!     `.dll.kryoto` beside the emulator, and gbe_fork's own tool makes the
//!     interface list (`steam_settings/steam_interfaces.txt`) from it.
//!   * Every file that differs is merged with the same undoable journal as a
//!     Kryoto release update (game_update.rs), and each is written down as the
//!     updater's own, which is what lets the offer flow tell them from a file
//!     changed by anything else.
//!   * The copy is then kept at that build (`pinned_version`), so Kryoto's
//!     older release is never offered as an "update" over it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kryoto_repair::community::{self as cu, Role};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager, Runtime};

use crate::community_offer::{self as offer, Kryo, ReleaseRecord, UpdateRecord, RELEASE_RECORD};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// Show "Your copy can be updated: try it".
    pub available: bool,
    pub steam_build_id: Option<String>,
    pub kryoto_build_id: Option<String>,
    /// Why not, when kryo.to said why (shown only after the player asked).
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Depot {
    depot_id: String,
    manifest_id: String,
    key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tool {
    url: String,
    sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Build {
    appid: String,
    build_id: String,
    version: String,
    depots: Vec<Depot>,
    manifests: BTreeMap<String, String>,
    tool: Option<Tool>,
}

/// Is there a build to try? Free on kryo.to's side (no provider is called).
#[tauri::command(async)]
pub async fn community_update_check(app: AppHandle, id: String) -> Result<UpdateCheck, String> {
    let game = offer::find(&app, &id)?;
    let Some(slug) = game.slug.clone() else { return Ok(UpdateCheck::default()) };
    let install = PathBuf::from(&game.install_dir);
    // Already updated (or offered): the offer card takes over.
    if offer::read_update(&install).is_some() || cu::eligible_source(game.source.as_deref().unwrap_or_default()).is_none() {
        return Ok(UpdateCheck::default());
    }
    let Ok(kryo) = Kryo::new(&app) else { return Ok(UpdateCheck::default()) };
    let path = format!("/api/community/source?slug={}", url::form_urlencoded::byte_serialize(slug.as_bytes()).collect::<String>());
    let Some(v) = kryo.call(reqwest::Method::GET, &path, None).await.ok().flatten() else { return Ok(UpdateCheck::default()) };
    Ok(UpdateCheck {
        available: v["eligible"].as_bool() == Some(true),
        steam_build_id: v["steamBuildId"].as_str().map(String::from),
        kryoto_build_id: v["kryotoBuildId"].as_str().map(String::from),
        reason: v["reason"].as_str().map(String::from),
    })
}

fn sha256_hex(path: &Path) -> Option<String> {
    cu::fingerprint(path).ok().map(|f| f.0)
}

fn is_steam_api(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
    name == "steam_api.dll" || name == "steam_api64.dll"
}

/// Every file under `root`, `/`-separated, skipping DepotDownloader's own folder.
fn files(root: &Path) -> Vec<String> {
    walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| Some(e.path().strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/")))
        .filter(|rel| !rel.starts_with(".DepotDownloader"))
        .collect()
}

/// What staging holds that the game does not.
#[derive(Debug, Default, PartialEq)]
struct Changes {
    /// New and changed files, never a steam_api dll.
    files: Vec<String>,
    /// steam_api dlls Steam shipped a new original of (the release keeps its
    /// original beside the emulator as `<dll>.kryoto`).
    originals: Vec<String>,
}

fn changes(staged: &Path, install: &Path) -> Changes {
    let mut out = Changes::default();
    for rel in files(staged) {
        let new = staged.join(&rel);
        if is_steam_api(&rel) {
            let original = install.join(format!("{rel}.kryoto"));
            if !original.is_file() || sha256_hex(&new) != sha256_hex(&original) {
                out.originals.push(rel);
            }
            continue;
        }
        let old = install.join(&rel);
        if !old.is_file() || sha256_hex(&new) != sha256_hex(&old) {
            out.files.push(rel);
        }
    }
    out.files.sort();
    out.originals.sort();
    out
}

/// gbe_fork set up again around Steam's new originals, into `merge`: each new
/// original as `<dll>.kryoto`, and the interface list made from it. Returns
/// the files it wrote there.
async fn reapply_gbe<R: Runtime>(app: &AppHandle<R>, staged: &Path, merge: &Path, originals: &[String]) -> Result<Vec<String>, String> {
    if originals.is_empty() {
        return Ok(Vec::new());
    }
    let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("repair-tools");
    let client = kryoto_repair::release::client()?;
    let tools = kryoto_repair::release::install(&client, &cache, kryoto_repair::settings::Emulator::GbeFork, &|_: &str, _: Option<u8>| {}).await?;
    let mut written = Vec::new();
    for rel in originals {
        let backup = format!("{rel}.kryoto");
        let to = merge.join(&backup);
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::copy(staged.join(rel), &to).map_err(|e| e.to_string())?;
        written.push(backup);
        let dir = rel.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
        let settings_rel = format!("{dir}steam_settings");
        let is64 = rel.to_ascii_lowercase().ends_with("steam_api64.dll");
        let made = kryoto_repair::crack::interfaces_for_original(&tools.paths, &staged.join(rel), is64, &merge.join(&settings_rel))
            .await
            .map_err(|e| format!("Setting gbe_fork up again failed: {e}"))?;
        if !made {
            return Err("gbe_fork's interface tool did not run, so the update was not applied. Kryoto staff will update this game instead.".into());
        }
        written.push(format!("{settings_rel}/steam_interfaces.txt"));
    }
    Ok(written)
}

/// DepotDownloaderMod: Kryoto's packed copy (checked against its SHA-256),
/// unpacked once into the app's cache. A debug build can point at Forge's with
/// `KRYOTO_DEPOTDOWNLOADER_DLL`.
async fn depotdownloader<R: Runtime>(app: &AppHandle<R>, tool: Option<&Tool>) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        if let Some(p) = std::env::var_os("KRYOTO_DEPOTDOWNLOADER_DLL").map(PathBuf::from).filter(|p| p.is_file()) {
            return Ok(p);
        }
    }
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("depotdownloader");
    let dll = dir.join("DepotDownloaderMod.dll");
    let stamp = dir.join("sha256.txt");
    if let Some(tool) = tool {
        if dll.is_file() && std::fs::read_to_string(&stamp).ok().as_deref().map(str::trim) == Some(tool.sha256.as_str()) {
            return Ok(dll);
        }
        let bytes = offer::storage().get(&tool.url).send().await.map_err(|e| format!("Getting DepotDownloader failed: {e}"))?;
        let bytes = bytes.error_for_status().map_err(|e| format!("Getting DepotDownloader failed: {e}"))?.bytes().await.map_err(|e| e.to_string())?;
        let got: String = {
            use sha2::Digest;
            sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect()
        };
        if got != tool.sha256 {
            return Err("DepotDownloader did not arrive as Kryoto published it, so it was not used.".into());
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).map_err(|e| e.to_string())?;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
            let Some(rel) = f.enclosed_name() else { continue };
            // A packed `Release/net9.0/` folder or the files at the top: keep only the names.
            let Some(name) = rel.file_name() else { continue };
            if f.is_dir() {
                continue;
            }
            let mut out = std::fs::File::create(dir.join(name)).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
        }
        std::fs::write(&stamp, &tool.sha256).map_err(|e| e.to_string())?;
        if dll.is_file() {
            return Ok(dll);
        }
        return Err("Kryoto's DepotDownloader package has no DepotDownloaderMod.dll.".into());
    }
    if dll.is_file() {
        return Ok(dll);
    }
    Err("Kryoto cannot download game updates on this PC yet (no DepotDownloader to use).".into())
}

/// `dotnet`, with a runtime new enough for DepotDownloaderMod (9 or later).
fn dotnet() -> Result<PathBuf, String> {
    let mut candidates = vec![PathBuf::from("dotnet")];
    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\Program Files\dotnet\dotnet.exe"));
    } else {
        candidates.push(PathBuf::from("/usr/share/dotnet/dotnet"));
        candidates.push(PathBuf::from("/usr/lib/dotnet/dotnet"));
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(PathBuf::from(home).join(".dotnet/dotnet"));
        }
    }
    for c in candidates {
        let mut cmd = std::process::Command::new(&c);
        cmd.arg("--list-runtimes");
        no_window(&mut cmd);
        let Ok(out) = cmd.output() else { continue };
        let text = String::from_utf8_lossy(&out.stdout);
        let ok = text.lines().any(|l| {
            l.strip_prefix("Microsoft.NETCore.App ")
                .and_then(|v| v.split('.').next())
                .and_then(|m| m.parse::<u32>().ok())
                .is_some_and(|m| m >= 9)
        });
        if ok {
            return Ok(c);
        }
    }
    Err("Updating a game needs the .NET runtime, version 9 or later: get it from https://dotnet.microsoft.com/download/dotnet and try again.".into())
}

fn no_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Try Steam's newer build on this copy. On success the game is at that build
/// and the offer card takes over (playtest, screenshot, offer).
#[tauri::command(async)]
pub async fn community_update_try(app: AppHandle, id: String) -> Result<offer::OfferState, String> {
    let game = offer::find(&app, &id)?;
    let slug = game.slug.clone().ok_or("Only a kryo.to game can be updated this way.")?;
    let source = cu::eligible_source(game.source.as_deref().unwrap_or_default())
        .ok_or_else(|| cu::ineligible_reason(game.source.as_deref().unwrap_or_default()).to_string())?;
    if source.steamless {
        return Err("Kryoto cannot update Steamless releases on your PC yet; staff update these.".into());
    }
    let install = PathBuf::from(&game.install_dir);
    let _: ReleaseRecord = offer::read_json(&install.join(RELEASE_RECORD))
        .ok_or("Kryoto has no record of the release installed here. Reinstall the game from Kryoto once, then try again.")?;

    offer::progress(&app, &id, "Asking kryo.to for the newer build", 0, 0);
    let kryo = Kryo::new(&app)?;
    let got = kryo
        .call(reqwest::Method::POST, "/api/community/source", Some(json!({ "slug": slug })))
        .await?
        .ok_or("Community updates are not on for your account.")?;
    let build: Build = serde_json::from_value(got["build"].clone()).map_err(|_| "kryo.to's answer could not be read.".to_string())?;
    let dll = depotdownloader(&app, build.tool.as_ref()).await?;
    let dotnet = tauri::async_runtime::spawn_blocking(dotnet).await.map_err(|e| e.to_string())??;

    // Staging beside the game, on the same disk, so the merge is a copy within it.
    let staging = install.parent().unwrap_or(&install).join(format!(".kryoto-update-{}", offer::safe(&slug)));
    let files_dir = staging.join("files");
    let work = staging.join("work");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&files_dir).map_err(|e| format!("Making the update folder failed: {e}"))?;
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = run(&app, &id, &game, &install, &staging, &files_dir, &work, &build, &dll, &dotnet).await;
    let _ = std::fs::remove_dir_all(&staging);
    result?;
    let record = offer::read_update(&install).ok_or("The update did not finish.")?;
    let game = offer::find(&app, &id)?;
    Ok(offer::OfferState { shown: true, to_version: record.to_version, kryoto_version: game.version.clone(), need_seconds: offer::MIN_PLAY_SECS, auto_shots: cfg!(windows), ..Default::default() })
}

#[allow(clippy::too_many_arguments)]
async fn run<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    game: &crate::library::LibraryGame,
    install: &Path,
    staging: &Path,
    files_dir: &Path,
    work: &Path,
    build: &Build,
    dll: &Path,
    dotnet: &Path,
) -> Result<(), String> {
    use base64::Engine as _;
    let keys = work.join("depotkeys.txt");
    let mut text = String::new();
    for d in &build.depots {
        if !d.key.chars().all(|c| c.is_ascii_hexdigit()) || !d.depot_id.chars().all(|c| c.is_ascii_digit()) || !d.manifest_id.chars().all(|c| c.is_ascii_digit()) {
            return Err("kryo.to's answer had a depot it should not.".into());
        }
        text.push_str(&format!("{};{}\n", d.depot_id, d.key));
    }
    std::fs::write(&keys, text).map_err(|e| e.to_string())?;
    for (i, d) in build.depots.iter().enumerate() {
        offer::progress(app, id, &format!("Downloading depot {} of {} from Steam", i + 1, build.depots.len()), i as u64, build.depots.len() as u64);
        let name = format!("{}_{}.manifest", d.depot_id, d.manifest_id);
        let data = build.manifests.get(&name).ok_or("kryo.to's answer is missing a manifest.")?;
        let file = work.join(&name);
        std::fs::write(&file, base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let mut cmd = std::process::Command::new(dotnet);
        cmd.arg(dll)
            .arg("-dir")
            .arg(files_dir)
            .args(["-max-downloads", "4"])
            .arg("-depotkeys")
            .arg(&keys)
            .args(["-app", &build.appid, "-depot", &d.depot_id, "-manifest", &d.manifest_id])
            .arg("-manifestfile")
            .arg(&file)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        no_window(&mut cmd);
        let out = tauri::async_runtime::spawn_blocking(move || cmd.output()).await.map_err(|e| e.to_string())?.map_err(|e| format!("DepotDownloader did not start: {e}"))?;
        if !out.status.success() {
            let tail: String = String::from_utf8_lossy(&out.stdout).lines().chain(String::from_utf8_lossy(&out.stderr).lines()).rev().take(3).collect::<Vec<_>>().join(" / ");
            crate::logging::warn("community", &format!("depot {}: {tail}", d.depot_id));
            return Err(format!("Steam's files for depot {} did not download ({tail}).", d.depot_id));
        }
    }

    offer::progress(app, id, "Comparing with your copy", 0, 0);
    let (staged_dir, install_dir) = (files_dir.to_path_buf(), install.to_path_buf());
    let found = tauri::async_runtime::spawn_blocking(move || changes(&staged_dir, &install_dir)).await.map_err(|e| e.to_string())?;
    if found.files.is_empty() && found.originals.is_empty() {
        return Err("Steam's newer build has no file your copy does not already have.".into());
    }
    // Only the changed files in the tree that is merged, and gbe_fork set up again around new originals.
    let merge = staging.join("merge");
    for rel in &found.files {
        let to = merge.join(rel);
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::rename(files_dir.join(rel), &to).map_err(|e| e.to_string())?;
    }
    offer::progress(app, id, "Setting gbe_fork up again", 0, 0);
    let mut changed = found.files.clone();
    for rel in reapply_gbe(app, files_dir, &merge, &found.originals).await? {
        // The same bytes as the copy already has is no change.
        if sha256_hex(&merge.join(&rel)) != sha256_hex(&install.join(&rel)) {
            changed.push(rel);
        } else {
            let _ = std::fs::remove_file(merge.join(&rel));
        }
    }
    changed.sort();
    if changed.is_empty() {
        return Err("Steam's newer build has no file your copy does not already have.".into());
    }

    offer::progress(app, id, "Updating your copy", 0, changed.len() as u64);
    let settings = crate::settings::load(app);
    let backup_root = if settings.update_backup_dir.trim().is_empty() {
        app.path().app_data_dir().map_err(|e| e.to_string())?.join("update-backups")
    } else {
        PathBuf::from(&settings.update_backup_dir)
    };
    let title = game.title.clone();
    let mut journal = crate::game_update::prepare(crate::game_update::Prepare {
        slug: game.slug.as_deref().unwrap_or(&game.id),
        title: &title,
        staging: &merge,
        dest: install,
        backup_root: &backup_root,
        from_version: game.version.clone(),
        to_version: Some(build.version.clone()),
    })?;
    for (i, rel) in changed.iter().enumerate() {
        let to = install.join(rel);
        let copied = to.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::copy(merge.join(rel), &to).map(|_| ()));
        if let Err(e) = copied {
            let _ = crate::game_update::restore(&mut journal);
            return Err(format!("Updating {rel} failed, so your copy was put back as it was: {e}"));
        }
        offer::progress(app, id, "Updating your copy", i as u64 + 1, changed.len() as u64);
    }
    journal.state = crate::game_update::State::Applied;
    journal.save()?;
    journal.previous = crate::game_update::remember(&crate::game_update::data_dir(app), &journal);
    crate::game_update::drop_previous(&journal);

    // The updater's record: the offer flow's only way in.
    let written: BTreeMap<String, Role> = changed.iter().map(|r| (r.clone(), Role::Update)).collect();
    offer::write_update(install, &UpdateRecord { to_version: build.version.clone(), build_id: Some(build.build_id.clone()), written, at: offer::now(), ..Default::default() })?;
    // Kept at this build: Kryoto's older release is never put over it.
    let version = build.version.clone();
    crate::library::update(app, |games| {
        if let Some(g) = games.iter_mut().find(|g| g.id == game.id) {
            g.version = Some(version.clone());
            g.pinned_version = Some(version.clone());
        }
        Ok(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn changes_keep_the_emulator_and_note_when_steams_own_dll_moved() {
        let base = std::env::temp_dir().join(format!("kry-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (install, staged) = (base.join("game"), base.join("staged"));
        put(&install, "Game.exe", b"old");
        put(&install, "data/a.pak", b"same");
        put(&install, "steam_api64.dll", b"gbe_fork");
        put(&install, "steam_api64.dll.kryoto", b"valve 1");
        put(&staged, "Game.exe", b"new");
        put(&staged, "data/a.pak", b"same");
        put(&staged, "data/b.pak", b"added");
        put(&staged, "steam_api64.dll", b"valve 1");
        put(&staged, ".DepotDownloader/depot.config", b"x");
        assert_eq!(changes(&staged, &install), Changes { files: vec!["Game.exe".into(), "data/b.pak".into()], originals: vec![] });

        // Steam's own dll moved on: the emulator is still not touched, the original is noted for gbe_fork.
        put(&staged, "steam_api64.dll", b"valve 2");
        assert_eq!(changes(&staged, &install).originals, ["steam_api64.dll"]);
        assert!(!changes(&staged, &install).files.iter().any(|f| f.contains("steam_api")));
        let _ = std::fs::remove_dir_all(&base);
    }
}
