//! Backing saves up and putting them back.
//!
//! - **When a game closes** (a little after, as games write on the way out):
//!   what the tracer saw is learned from, then the saves are backed up if they
//!   changed since this PC last synced.
//! - **Before a game starts**: if another PC backed up since, and nothing
//!   changed here, its saves are put here; if both changed, the player picks.
//!   Offline, or slow: the game starts as it is.
//! - **Every restore** first packs what is on this PC into a local backup, so
//!   nothing a restore replaces is lost.
//!
//! kryo.to checks every backup: the account's perk and space, the bytes'
//! size and MD5, and that this PC had the newest version as its parent
//! (otherwise it answers with a conflict rather than overwrite another PC's).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use kryoto_saves::bundle::{self, BundleFile, Manifest, RegValue};
use kryoto_saves::decide::{decide, Action, Base};
use kryoto_saves::scan::{collect, Collected, HashCache, MAX_FILES};
use kryoto_saves::Rule;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::api::{self, Api, ApiError};
use super::{account, game_data, identity, load_rules, load_state, locations, now, roots, save_rules, save_state, update_account, GameState};
use crate::library::LibraryGame;

/// A backup's size limit before kryo.to has said the account's.
pub const DEFAULT_MAX_BACKUP: u64 = 100 << 20;
/// How long Play waits to hear from kryo.to before starting the game anyway.
const LAUNCH_CHECK: Duration = Duration::from_secs(6);
/// How long a question waits for the player before the game starts as it is.
const ASK_FOR: Duration = Duration::from_secs(600);
/// How long community rules are kept before asking kryo.to again.
const COMMUNITY_FRESH: u64 = 7 * 24 * 3600;

/// Games busy backing up or restoring: one thing at a time each.
static BUSY: Mutex<Option<HashSet<String>>> = Mutex::new(None);

struct Busy(String);

impl Busy {
    fn take(id: &str) -> Result<Busy, String> {
        let mut set = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        let set = set.get_or_insert_with(HashSet::new);
        if !set.insert(id.to_string()) {
            return Err("This game's saves are already being synced.".into());
        }
        Ok(Busy(id.to_string()))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        if let Some(set) = BUSY.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            set.remove(&self.0);
        }
    }
}

/// How far a backup or restore is, for the game page (`cloud-saves-progress`):
/// the phase (`scanning`, `packing`, `uploading`, `checking`, `downloading`,
/// `restoring`, then `done`) and bytes done of bytes in all, 0 of 0 when it
/// cannot be counted. A big backup takes minutes; without this it looked stuck.
/// Sent at most about eight times a second, at once when the phase changes,
/// and `done` when it is dropped, however the work ended.
pub(crate) struct Progress<R: Runtime> {
    app: AppHandle<R>,
    id: String,
    last: std::sync::Arc<Mutex<(&'static str, Option<std::time::Instant>)>>,
    owner: bool,
}

impl<R: Runtime> Progress<R> {
    fn new(app: &AppHandle<R>, id: &str) -> Self {
        Progress { app: app.clone(), id: id.to_string(), last: Default::default(), owner: true }
    }

    /// A copy to hand to other threads; only the first says `done`.
    fn share(&self) -> Self {
        Progress { app: self.app.clone(), id: self.id.clone(), last: self.last.clone(), owner: false }
    }

    fn report(&self, phase: &'static str, done: u64, total: u64) {
        let now = std::time::Instant::now();
        {
            let Ok(mut last) = self.last.lock() else { return };
            let due = last.0 != phase || (total > 0 && done >= total) || last.1.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(125));
            if !due {
                return;
            }
            *last = (phase, Some(now));
        }
        let _ = self.app.emit("cloud-saves-progress", json!({ "gameId": self.id, "phase": phase, "done": done, "total": total }));
    }
}

impl<R: Runtime> Drop for Progress<R> {
    fn drop(&mut self) {
        if self.owner {
            let _ = self.app.emit("cloud-saves-progress", json!({ "gameId": self.id, "phase": "done", "done": 0, "total": 0 }));
        }
    }
}

fn game<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<LibraryGame, String> {
    crate::library::load(app)?.into_iter().find(|g| g.id == id).ok_or_else(|| "That game is no longer in the library.".to_string())
}

fn running<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> bool {
    let ours = app.try_state::<crate::library::Running>().is_some_and(|r| r.0.lock().map(|m| m.contains_key(&game.id)).unwrap_or(false));
    ours || crate::handoff::find_in_folder(Path::new(&game.install_dir), 0, &HashSet::new()).is_some()
}

fn notice<R: Runtime>(app: &AppHandle<R>, title: &str, body: &str, game: &LibraryGame) {
    let _ = app.emit("notify", crate::downloads::Notice::new(title, body, Some(game.id.clone())));
}

/* ── this PC's saves ────────────────────────────────────────────────── */

pub(crate) struct Local {
    pub rules: Vec<Rule>,
    pub found: Collected,
    pub files: Vec<BundleFile>,
    pub registry: Vec<RegValue>,
    /// None when the game has no saves here.
    pub hash: Option<String>,
}

/// What the game's saves are on this PC right now. Reads files (hashing only
/// changed ones), so call it off the main thread.
pub(crate) fn local<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, roots: &kryoto_saves::SaveRoots) -> Local {
    let (rules, _) = locations(app, game, roots);
    let found = collect(&rules, roots, MAX_FILES);
    let cache_file = game_data(app, &game.id).map(|d| d.join("hashes.json"));
    let mut cache: HashCache = cache_file.as_ref().map(|f| super::read_json(f)).unwrap_or_default();
    let files: Vec<BundleFile> = found
        .files
        .iter()
        .filter_map(|f| Some(BundleFile { root: f.root, path: f.path.clone(), size: f.size, sha256: cache.hash(f).ok()?, mtime: f.mtime, rule: f.rule.clone() }))
        .collect();
    cache.retain(&found.files);
    if let Some(f) = &cache_file {
        let _ = super::write_json(f, &cache);
    }
    #[cfg(windows)]
    let registry = kryoto_saves::registry::read(&rules);
    #[cfg(not(windows))]
    let registry: Vec<RegValue> = Vec::new();
    let hash = (!files.is_empty() || !registry.is_empty()).then(|| bundle::content_hash(&files, &registry));
    Local { rules, found, files, registry, hash }
}

fn machine() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "This PC".into()).chars().take(60).collect()
}

fn template(game: &LibraryGame, local: &Local, parent: Option<String>) -> Manifest {
    let ident = identity(game);
    Manifest {
        game_key: ident.key,
        aliases: ident.aliases,
        title: game.title.clone(),
        rules: local.rules.clone(),
        registry: local.registry.clone(),
        steam_id: None,
        machine: machine(),
        created_at: now(),
        parent_version: parent,
        ..Default::default()
    }
}

/* ── the account ─────────────────────────────────────────────────────── */

pub(crate) fn remember_list<R: Runtime>(app: &AppHandle<R>, list: &api::List) {
    update_account(app, |a| {
        a.perk = Some(list.perk);
        a.perk_checked = now();
        a.allowance = Some(list.allowance.clone());
        a.usage = Some(list.usage.clone());
    });
}

pub(crate) async fn refresh_account<R: Runtime>(app: &AppHandle<R>) -> Result<(), ApiError> {
    let api = Api::new(app)?;
    match api.list().await {
        Ok(list) => {
            remember_list(app, &list);
            Ok(())
        }
        Err(ApiError::Off) => {
            update_account(app, |a| a.feature = false);
            Err(ApiError::Off)
        }
        Err(e) => Err(e),
    }
}

/// What kryo.to's answer means for this game's state and the account.
fn note_error<R: Runtime>(app: &AppHandle<R>, state: &mut GameState, e: &ApiError) {
    match e {
        ApiError::NoPerk => update_account(app, |a| {
            a.perk = Some(false);
            a.perk_checked = now();
        }),
        ApiError::Off => update_account(app, |a| a.feature = false),
        ApiError::Conflict(head) => {
            state.conflict = true;
            if let Some(h) = head {
                state.cloud_head = Some((**h).clone());
            }
        }
        ApiError::TooBig(_) => state.too_big = true,
        _ => {}
    }
    state.error = Some(e.to_string());
}

/* ── backing up ──────────────────────────────────────────────────────── */

/// Back a game's saves up. `manual` is the player pressing Back up now (it
/// always asks kryo.to); otherwise unchanged saves are left alone.
pub async fn backup<R: Runtime>(app: &AppHandle<R>, id: &str, manual: bool) -> Result<String, String> {
    if !super::enabled(app) {
        return Err("Cloud saves are off.".into());
    }
    let _busy = Busy::take(id)?;
    let game = game(app, id)?;
    let progress = Progress::new(app, id);
    progress.report("scanning", 0, 0);
    let roots = roots(app, &game);
    let app2 = app.clone();
    let game2 = game.clone();
    let roots2 = roots.clone();
    let local = tauri::async_runtime::spawn_blocking(move || local(&app2, &game2, &roots2)).await.map_err(|e| e.to_string())?;
    let mut state = load_state(app, id);
    let Some(hash) = local.hash.clone() else {
        return Ok("No saves found for this game yet.".into());
    };
    if local.found.truncated {
        state.too_big = true;
        state.error = Some(format!("More than {MAX_FILES} save files. Take off a location that is not saves."));
        save_state(app, id, &state);
        return Err(state.error.clone().unwrap_or_default());
    }
    if !manual && state.base_hash.as_deref() == Some(hash.as_str()) && state.base_version.is_some() {
        return Ok("Already backed up.".into());
    }
    let result = upload(app, &game, &local, &mut state, &progress).await;
    match &result {
        Ok(_) => {
            state.error = None;
            state.too_big = false;
            state.conflict = false;
        }
        Err(e) => {
            note_error(app, &mut state, e);
            if matches!(e, ApiError::Conflict(_)) {
                notice(app, "Saves differ from your account", &format!("{} has newer saves from another PC. Pick which to keep on its page.", game.title), &game);
            }
        }
    }
    save_state(app, id, &state);
    let _ = app.emit("cloud-saves", super::status_of(app, &account(app)));
    match result {
        Ok(()) => {
            if manual {
                notice(app, "Saves backed up", &format!("{}'s saves are on your kryo.to account.", game.title), &game);
            }
            Ok("Backed up.".into())
        }
        Err(e) => Err(e.to_string()),
    }
}

async fn upload<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, local: &Local, state: &mut GameState, progress: &Progress<R>) -> Result<(), ApiError> {
    let api = Api::new(app)?;
    refresh_community(app, game, &api).await;
    let dir = game_data(app, &game.id).ok_or_else(|| ApiError::Other("No app data folder.".into()))?;
    let out = dir.join("out.ksave");
    let tpl = template(game, local, state.base_version.clone());
    let files = local.found.files.clone();
    let out2 = out.clone();
    let packing = progress.share();
    let (manifest, written) = tauri::async_runtime::spawn_blocking(move || bundle::write_with(&tpl, &files, &out2, &mut |done, total| packing.report("packing", done, total)))
        .await
        .map_err(|e| ApiError::Other(e.to_string()))?
        .map_err(ApiError::Other)?;
    let max = account(app).allowance.map(|a| a.max_backup_bytes).filter(|m| *m > 0).unwrap_or(DEFAULT_MAX_BACKUP);
    if written.size > max {
        let _ = std::fs::remove_file(&out);
        return Err(ApiError::TooBig(format!("This game's saves are {} MB packed, over the {} MB a backup can be.", written.size.div_ceil(1 << 20), max >> 20)));
    }
    let body = json!({
        "gameKey": manifest.game_key,
        "aliases": manifest.aliases,
        "slug": game.slug,
        "title": game.title,
        "size": written.size,
        "md5": written.md5,
        "sha256": written.sha256,
        "manifest": {
            "files": manifest.files.iter().map(|f| json!({ "root": f.root, "path": f.path, "size": f.size })).collect::<Vec<_>>(),
            "registry": manifest.registry.iter().map(|r| json!({ "root": "HKCU", "path": r.path, "name": r.name, "size": r.data.len() + r.name.len() + r.path.len() + 64 })).collect::<Vec<_>>(),
            "rules": manifest.rules,
            "contentHash": manifest.content_hash,
        },
        "machine": manifest.machine,
        "parentVersion": state.base_version,
    });
    progress.report("uploading", 0, written.size);
    let prepared = api.prepare(body).await?;
    let (uploading, size) = (progress.share(), written.size);
    let sent = api::upload(&prepared.url, &prepared.headers, &out, written.size, move |done| uploading.report("uploading", done, size)).await;
    let _ = std::fs::remove_file(&out);
    sent.map_err(ApiError::Other)?;
    progress.report("checking", 0, 0);
    let version = api.claim(&prepared.upload_id).await?;
    state.base_version = Some(version.id.clone());
    state.base_hash = Some(manifest.content_hash.clone());
    state.last_backup_at = Some(now());
    state.cloud_head = Some(version);
    update_account(app, |a| {
        a.perk = Some(true);
        a.last_sync = Some(now());
    });
    crate::logging::info("saves", &format!("{}: backed up ({} files, {} bytes)", game.title, manifest.files.len(), written.size));
    Ok(())
}

/* ── putting saves back ──────────────────────────────────────────────── */

/// Pack what is on this PC now into a local backup (kept `cloud_saves_keep_local`).
fn keep_local<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, local: &Local) -> Result<(), String> {
    if local.hash.is_none() {
        return Ok(());
    }
    let dir = game_data(app, &game.id).ok_or("No app data folder.")?.join("backups");
    let out = dir.join(format!("{}.ksave", now()));
    bundle::write(&template(game, local, None), &local.found.files, &out)?;
    super::prune_backups(app, &game.id, crate::settings::load(app).cloud_saves_keep_local as usize);
    Ok(())
}

/// Put a backup file's saves on this PC. Blocking.
fn put_back<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, file: &Path) -> Result<Manifest, String> {
    let manifest = bundle::verify(file)?;
    let roots = roots(app, game);
    // The rules it came with, so the next backup from here covers them too.
    let mut stored = load_rules(app, &game.id);
    for r in bundle::file_rules(&manifest).into_iter().chain(manifest.rules.iter().filter(|r| r.kind == kryoto_saves::Kind::Registry).cloned()) {
        if r.problem().is_none() && !stored.carried.iter().any(|c| c.key() == r.key()) && !stored.ignored.contains(&r.key()) {
            stored.carried.push(r);
        }
    }
    save_rules(app, &game.id, &stored);
    let now_here = local(app, game, &roots);
    keep_local(app, game, &now_here)?;
    let done = bundle::restore(file, &manifest, &roots, &now_here.found.files)?;
    crate::logging::info("saves", &format!("{}: restored {} files ({} removed, {} registry values)", game.title, done.written, done.removed, done.registry));
    Ok(manifest)
}

/// Download one of the account's backups and put it here.
async fn fetch_and_put<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, version_id: &str, progress: &Progress<R>) -> Result<Manifest, String> {
    progress.report("downloading", 0, 0);
    let api = Api::new(app).map_err(|e| e.to_string())?;
    let link = api.link(version_id).await.map_err(|e| e.to_string())?;
    let dir = game_data(app, &game.id).ok_or("No app data folder.")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join("in.ksave");
    let max = account(app).allowance.map(|a| a.max_backup_bytes).filter(|m| *m > 0).unwrap_or(DEFAULT_MAX_BACKUP).max(link.size) + (1 << 20);
    let size = link.size;
    api::download(&link.url, &file, max, |done, total| progress.report("downloading", done, if size > 0 { size } else { total })).await?;
    progress.report("checking", 0, 0);
    let got = bundle::digest(&file)?;
    if !link.sha256.is_empty() && got.sha256 != link.sha256 {
        let _ = std::fs::remove_file(&file);
        return Err("The downloaded backup is not the one on your account. Nothing was changed.".into());
    }
    progress.report("restoring", 0, 0);
    let (app2, game2, file2) = (app.clone(), game.clone(), file.clone());
    let result = tauri::async_runtime::spawn_blocking(move || put_back(&app2, &game2, &file2)).await.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&file);
    result
}

pub async fn restore_version<R: Runtime>(app: &AppHandle<R>, id: &str, version_id: &str) -> Result<String, String> {
    restore_checked(app, id, version_id, false).await
}

/// `prelaunch`: Play itself is asking, before the game starts. Play has
/// already marked the game as running then, so only a copy started outside
/// Kryoto counts.
async fn restore_checked<R: Runtime>(app: &AppHandle<R>, id: &str, version_id: &str, prelaunch: bool) -> Result<String, String> {
    if !super::account(app).feature {
        return Err("Cloud saves are not available on your account yet.".into());
    }
    let _busy = Busy::take(id)?;
    let game = game(app, id)?;
    let busy = if prelaunch { crate::handoff::find_in_folder(Path::new(&game.install_dir), 0, &HashSet::new()).is_some() } else { running(app, &game) };
    if busy {
        return Err("Close the game first.".into());
    }
    let progress = Progress::new(app, id);
    let manifest = fetch_and_put(app, &game, version_id, &progress).await?;
    let mut state = load_state(app, id);
    state.base_version = Some(version_id.to_string());
    state.base_hash = Some(manifest.content_hash);
    state.last_restore_at = Some(now());
    state.conflict = false;
    state.error = None;
    save_state(app, id, &state);
    Ok("Saves restored. What was here before is kept under Versions, On this PC.".into())
}

pub async fn restore_local<R: Runtime>(app: &AppHandle<R>, id: &str, file: &str) -> Result<String, String> {
    let _busy = Busy::take(id)?;
    let game = game(app, id)?;
    if running(app, &game) {
        return Err("Close the game first.".into());
    }
    if file.contains(['/', '\\']) || !file.ends_with(".ksave") {
        return Err("That is not one of this game's local backups.".into());
    }
    let path = game_data(app, id).ok_or("No app data folder.")?.join("backups").join(file);
    if !path.is_file() {
        return Err("That local backup is gone.".into());
    }
    let progress = Progress::new(app, id);
    progress.report("restoring", 0, 0);
    // Keep it out of the pruning a restore does, then put it back.
    let keep = path.with_extension("restoring");
    std::fs::copy(&path, &keep).map_err(|e| e.to_string())?;
    let (app2, game2, keep2) = (app.clone(), game.clone(), keep.clone());
    let result = tauri::async_runtime::spawn_blocking(move || put_back(&app2, &game2, &keep2)).await.map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&keep);
    result?;
    let _ = app.emit("cloud-saves-game", json!({ "gameId": id }));
    Ok("Saves put back from this PC's backup.".into())
}

/* ── questions for the player ────────────────────────────────────────── */

static PROMPTS: Mutex<Option<HashMap<String, mpsc::Sender<String>>>> = Mutex::new(None);

pub fn answer(id: &str, choice: &str) -> Result<(), String> {
    if !matches!(choice, "cloud" | "local" | "later") {
        return Err("Unknown choice.".into());
    }
    let tx = PROMPTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(id)).ok_or("That question was already answered.")?;
    tx.send(choice.to_string()).map_err(|_| "That question was already answered.".to_string())
}

/// Ask the player (the shell shows it) and wait for the answer.
fn ask<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, kind: &str, head: &api::Version, local: &Local, state: &GameState) -> String {
    let (tx, rx) = mpsc::channel();
    PROMPTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(game.id.clone(), tx);
    let _ = app.emit(
        "cloud-saves-prompt",
        json!({
            "gameId": game.id,
            "title": game.title,
            "kind": kind,
            "cloud": { "machine": head.machine, "createdAt": head.created_at, "size": head.size, "files": head.file_count },
            "local": { "files": local.files.len(), "bytes": local.found.bytes, "lastBackupAt": state.last_backup_at, "changedAt": local.files.iter().map(|f| f.mtime).max() },
        }),
    );
    if let Some(w) = app.get_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    let choice = rx.recv_timeout(ASK_FOR).unwrap_or_else(|_| "later".into());
    if let Some(m) = PROMPTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(&game.id);
    }
    let _ = app.emit("cloud-saves-prompt-done", json!({ "gameId": game.id }));
    choice
}

/* ── before a game starts ────────────────────────────────────────────── */

/// Give gbe_fork the account's Steam id where the game has none set, so a game
/// that ties saves to the player's id finds them on every PC.
fn pin_steam_id(game_dir: &Path, steam_id: &str) {
    for dir in kryoto_saves::emulator::steam_settings_dirs(game_dir) {
        let file = dir.join("configs.user.ini");
        let text = std::fs::read(&file).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        if kryoto_saves::emulator::ini_get(&text, "user::general", "account_steamid").is_some() {
            continue;
        }
        let next = crate::player_name::set_ini(&text, Some("user::general"), "account_steamid", steam_id, true);
        let _ = std::fs::write(&file, next);
    }
}

async fn refresh_community<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, api: &Api) {
    let mut stored = load_rules(app, &game.id);
    if now().saturating_sub(stored.community_at) < COMMUNITY_FRESH {
        return;
    }
    let ident = identity(game);
    if let Ok(map) = api.rules(&ident.aliases).await {
        stored.community = map.into_values().flatten().filter(|r| r.problem().is_none() && matches!(r.source, kryoto_saves::Source::Community | kryoto_saves::Source::Staff)).collect();
        stored.community_at = now();
        save_rules(app, &game.id, &stored);
    }
}

/// Run on Play, before the game starts (blocking; the launch thread waits).
pub fn before_launch<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) {
    if !super::enabled(app) {
        return;
    }
    if let Some(user) = account(app).username {
        pin_steam_id(Path::new(&game.install_dir), &super::steam_id_for(&user));
    }
    let checked = tauri::async_runtime::block_on(async { tokio::time::timeout(LAUNCH_CHECK, check(app, game)).await });
    let (head, local) = match checked {
        Ok(Ok(Some(found))) => found,
        Ok(Ok(None)) => return,
        Ok(Err(e)) => {
            if !matches!(e, ApiError::SignedOut | ApiError::Off | ApiError::NoPerk) {
                crate::logging::warn("saves", &format!("{}: could not check the cloud: {e}", game.title));
            }
            return;
        }
        Err(_) => {
            crate::logging::warn("saves", &format!("{}: kryo.to was too slow; starting with this PC's saves", game.title));
            return;
        }
    };
    let mut state = load_state(app, &game.id);
    let base = Base { version: state.base_version.clone(), hash: state.base_hash.clone() };
    let action = decide(local.hash.as_deref(), &base, Some(&head.id), head.content_hash().as_deref());
    state.cloud_head = Some(head.clone());
    let choice = match action {
        Action::Nothing => {
            // The same saves on both sides: remember the account's version.
            if state.base_version.as_deref() != Some(head.id.as_str()) && local.hash.is_some() && local.hash == head.content_hash() {
                state.base_version = Some(head.id.clone());
                state.base_hash = local.hash.clone();
            }
            save_state(app, &game.id, &state);
            return;
        }
        Action::Upload => {
            save_state(app, &game.id, &state);
            return;
        }
        Action::FastForward if crate::settings::load(app).cloud_saves_restore != "ask" => "cloud".to_string(),
        Action::FastForward => ask(app, game, "restore", &head, &local, &state),
        Action::Conflict => {
            state.conflict = true;
            save_state(app, &game.id, &state);
            ask(app, game, "conflict", &head, &local, &state)
        }
    };
    match choice.as_str() {
        "cloud" => {
            let result = tauri::async_runtime::block_on(restore_checked(app, &game.id, &head.id, true));
            match result {
                Ok(_) => notice(app, "Saves restored", &format!("{} has the saves from {}.", game.title, head.machine.as_deref().unwrap_or("your other PC")), game),
                Err(e) => {
                    let mut s = load_state(app, &game.id);
                    s.error = Some(e.clone());
                    save_state(app, &game.id, &s);
                    notice(app, "Saves could not be restored", &format!("{}: {e}", game.title), game);
                }
            }
        }
        "local" => {
            // This PC's become the newest: the next backup goes on top of the account's.
            let mut s = load_state(app, &game.id);
            s.base_version = Some(head.id.clone());
            s.conflict = false;
            save_state(app, &game.id, &s);
            let app2 = app.clone();
            let id = game.id.clone();
            tauri::async_runtime::spawn(async move {
                let _ = backup(&app2, &id, false).await;
            });
        }
        _ => {}
    }
}

/// The account's newest backup and this PC's saves, or None when the
/// account has none and there is nothing to compare.
async fn check<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Result<Option<(api::Version, Local)>, ApiError> {
    let api = Api::new(app)?;
    let ident = identity(game);
    let head = match api.head(&ident.key).await {
        Ok(h) => h,
        Err(ApiError::Off) => {
            update_account(app, |a| a.feature = false);
            return Err(ApiError::Off);
        }
        Err(e) => return Err(e),
    };
    update_account(app, |a| {
        a.perk = Some(head.perk);
        a.perk_checked = now();
        a.allowance = Some(head.allowance.clone());
    });
    refresh_community(app, game, &api).await;
    let Some(version) = head.head else { return Ok(None) };
    let roots = roots(app, game);
    let (app2, game2) = (app.clone(), game.clone());
    let local = tauri::async_runtime::spawn_blocking(move || local(&app2, &game2, &roots)).await.map_err(|e| ApiError::Other(e.to_string()))?;
    Ok(Some((version, local)))
}

/* ── after a game closes ─────────────────────────────────────────────── */

/// What the game's own files were found to mention, kept so the same folder
/// names are not searched for again until the game changes.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Mentions {
    fingerprint: String,
    found: Vec<String>,
    missing: Vec<String>,
}

/// Which of the folder names the session wrote in are named in the game's
/// own files (kryoto_saves::mentions): what ties a folder to the game when
/// nothing else does, with no administrator needed.
fn mentioned<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, names: Vec<String>) -> HashSet<String> {
    let files = kryoto_saves::mentions::game_files(Path::new(&game.install_dir));
    let fingerprint = {
        let newest = files.iter().filter_map(|(p, _)| std::fs::metadata(p).ok()?.modified().ok()).max().map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)).unwrap_or(0);
        format!("{}:{}:{newest}", files.len(), files.iter().map(|(_, s)| s).sum::<u64>())
    };
    let cache_file = super::game_data(app, &game.id).map(|d| d.join("mentions.json"));
    let mut cache: Mentions = cache_file.as_ref().map(|f| super::read_json(f)).unwrap_or_default();
    if cache.fingerprint != fingerprint {
        cache = Mentions { fingerprint, ..Default::default() };
    }
    let unknown: Vec<String> = names.iter().filter(|n| !cache.found.contains(n) && !cache.missing.contains(n)).cloned().collect();
    if !unknown.is_empty() {
        let found = kryoto_saves::mentions::find(&files, &unknown, Duration::from_secs(25));
        for n in unknown {
            if found.contains(&n) {
                cache.found.push(n);
            } else {
                cache.missing.push(n);
            }
        }
        // Bounded: the oldest misses go first.
        let extra = cache.missing.len().saturating_sub(5_000);
        cache.missing.drain(..extra);
        if let Some(f) = &cache_file {
            let _ = super::write_json(f, &cache);
        }
    }
    let found: Vec<String> = names.into_iter().filter(|n| cache.found.contains(n)).collect();
    specific(app, game, found).into_iter().collect()
}

/// For each folder name, which other installed games' files were searched for
/// it and which of them mention it too. Kept for the PC: a name is looked up
/// once, not every session.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Baseline {
    names: BTreeMap<String, Seen>,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Seen {
    checked: Vec<String>,
    mentioned_by: Vec<String>,
}

/// Other games searched per name, and how many of them may mention it before
/// it is a common word, not one game's folder.
const CONTROL_GAMES: usize = 8;
const COMMON_IN: usize = 2;

/// The names that belong to this game: a word other games' files contain
/// too (`client`, `cache`, `signal`) is in every game and says nothing.
fn specific<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, names: Vec<String>) -> Vec<String> {
    if names.is_empty() {
        return names;
    }
    let file = super::base_dir(app).map(|d| d.join("mention-baseline.json"));
    let mut base: Baseline = file.as_ref().map(|f| super::read_json(f)).unwrap_or_default();
    let others: Vec<LibraryGame> =
        crate::library::load(app).unwrap_or_default().into_iter().filter(|g| g.id != game.id && !g.install_dir.is_empty() && Path::new(&g.install_dir).is_dir()).collect();
    let mut changed = false;
    for other in others.iter().take(CONTROL_GAMES * 2) {
        let todo: Vec<String> = names
            .iter()
            .filter(|n| base.names.get(*n).is_none_or(|s| s.checked.len() < CONTROL_GAMES && !s.checked.contains(&other.id) && s.mentioned_by.len() < COMMON_IN))
            .cloned()
            .collect();
        if todo.is_empty() {
            continue;
        }
        let files = kryoto_saves::mentions::game_files(Path::new(&other.install_dir));
        let found = kryoto_saves::mentions::find(&files, &todo, Duration::from_secs(6));
        for n in todo {
            let seen = base.names.entry(n.clone()).or_default();
            seen.checked.push(other.id.clone());
            if found.contains(&n) {
                seen.mentioned_by.push(other.id.clone());
            }
        }
        changed = true;
    }
    if changed {
        while base.names.len() > 20_000 {
            let Some(k) = base.names.keys().next().cloned() else { break };
            base.names.remove(&k);
        }
        if let Some(f) = &file {
            let _ = super::write_json(f, &base);
        }
    }
    names.into_iter().filter(|n| base.names.get(n).is_none_or(|s| s.mentioned_by.len() < COMMON_IN)).collect()
}

/// Learn where the game saved from what the tracer saw.
fn learn<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, obs: &kryoto_saves::trace::Observation, roots: &kryoto_saves::SaveRoots) {
    let ident = identity(game);
    let hints = super::hints(game, ident.appid.clone());
    let ignore = super::ignore_dirs(app, game);
    let precise = obs.traced.is_some();
    let mut obs = obs.clone();
    if !precise {
        let names = kryoto_saves::trace::names_to_check(&obs, roots, &hints, &ignore);
        if !names.is_empty() {
            obs.mentioned = mentioned(app, game, names);
        }
    }
    let obs = &obs;
    let raw = kryoto_saves::trace::candidates(obs, roots, &hints, &ignore, &|_| false);
    let noise_path = super::noise_file(app);
    let mut noise: super::Noise = noise_path.as_ref().map(|p| super::read_json(p)).unwrap_or_default();
    for c in &raw {
        let ids = noise.0.entry(c.rule.key()).or_default();
        if !ids.contains(&game.id) {
            ids.push(game.id.clone());
        }
    }
    while noise.0.len() > 20_000 {
        let Some(k) = noise.0.keys().next().cloned() else { break };
        noise.0.remove(&k);
    }
    if let Some(p) = &noise_path {
        let _ = super::write_json(p, &noise);
    }
    let noisy = |r: &Rule| noise.0.get(&r.key()).is_some_and(|ids| ids.iter().any(|g| *g != game.id));
    let mut stored = load_rules(app, &game.id);
    // A location Windows saw this game write is its own, whatever other
    // games did: noise only ever drops what the smart tracer guessed.
    stored.detected.retain(|d| !noisy(&d.rule) || d.accepted || d.traced);
    let t = now();
    let kept: Vec<_> = raw.into_iter().filter(|c| (precise && c.rule.kind == kryoto_saves::Kind::Files) || !noisy(&c.rule)).collect();
    if precise {
        stored.precise_sessions += 1;
        stored.precise_next = false;
        stored.precise_found |= kept.iter().any(|c| c.rule.kind == kryoto_saves::Kind::Files);
    }
    for c in kept {
        let reasons: Vec<String> = c.reasons.iter().map(|r| r.to_string()).collect();
        let traced = precise && c.rule.kind == kryoto_saves::Kind::Files;
        match stored.detected.iter_mut().find(|d| d.rule.key() == c.rule.key()) {
            Some(d) => {
                d.traced |= traced;
                d.sessions += 1;
                // Seen again in another session: more sure each time, up to twice.
                d.score = d.score.max(c.score) + if d.sessions <= 3 { 10 } else { 0 };
                d.files = c.files;
                d.last_seen = t;
                for r in reasons {
                    if !d.reasons.contains(&r) {
                        d.reasons.push(r);
                    }
                }
            }
            None => stored.detected.push(super::Detected { rule: c.rule, score: c.score, sessions: 1, files: c.files, last_seen: t, reasons, accepted: false, reported: false, traced }),
        }
    }
    stored.detected.sort_by_key(|d| std::cmp::Reverse((d.traced, d.score)));
    stored.detected.truncate(40);
    let fresh: Vec<(Rule, bool)> =
        stored.detected.iter().filter(|d| d.trusted() && !d.reported && kryoto_saves::trace::shareable(&d.rule)).map(|d| (d.rule.clone(), d.traced)).collect();
    save_rules(app, &game.id, &stored);
    crate::logging::info(
        "saves",
        &format!("{}: {} save location(s) seen this session{}", game.title, stored.detected.iter().filter(|d| d.last_seen == t).count(), if precise { " (precise)" } else { "" }),
    );
    if !fresh.is_empty() && crate::settings::load(app).cloud_saves_share {
        let app2 = app.clone();
        let id = game.id.clone();
        let key = ident.key.clone();
        tauri::async_runtime::spawn(async move {
            let Ok(api) = Api::new(&app2) else { return };
            // Found precisely counts for more on kryo.to: sent on its own.
            let mut sent: Vec<String> = Vec::new();
            for traced in [true, false] {
                let rules: Vec<Rule> = fresh.iter().filter(|(_, t)| *t == traced).map(|(r, _)| r.clone()).collect();
                if !rules.is_empty() && api.report_rules(&key, &rules, traced).await.is_ok() {
                    sent.extend(rules.iter().map(Rule::key));
                }
            }
            let mut stored = load_rules(&app2, &id);
            for d in stored.detected.iter_mut() {
                if sent.contains(&d.rule.key()) {
                    d.reported = true;
                }
            }
            save_rules(&app2, &id, &stored);
        });
    }
}

/// Run when a game has closed: learn from the session, then back up.
pub fn after_exit<R: Runtime>(app: AppHandle<R>, game_id: String, tracer: Option<super::tracer::Tracer>) {
    std::thread::spawn(move || {
        let Ok(game) = game(&app, &game_id) else { return };
        match tracer {
            Some(t) => {
                let roots = t.roots.clone();
                let obs = t.finish(Duration::from_secs(20));
                learn(&app, &game, &obs, &roots);
            }
            // Games often write their save on the way out.
            None => std::thread::sleep(Duration::from_secs(5)),
        }
        let _ = app.emit("cloud-saves-game", json!({ "gameId": game.id }));
        if !super::enabled(&app) || !crate::settings::load(&app).cloud_saves_on_exit || account(&app).perk == Some(false) {
            return;
        }
        // Started again in the meantime: the next close backs it up.
        if running(&app, &game) {
            return;
        }
        if let Err(e) = tauri::async_runtime::block_on(backup(&app, &game.id, false)) {
            crate::logging::warn("saves", &format!("{}: {e}", game.title));
        }
    });
}
