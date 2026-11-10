//! The Library: games on this PC, how each one starts, and how long it ran.
//!
//! Kept in one JSON file in the app's data folder, written whole through a
//! temporary file so a crash mid-save leaves the previous library rather than
//! half of one. Every command reloads it before changing it, so a playtime
//! written by a game that just closed is never overwritten by a stale copy.

use crate::launch::{self, LaunchEntry, LaunchPlan};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LibraryGame {
    pub id: String,
    pub title: String,
    /// The Steam branch its kryo.to listing builds from (`public`, or a beta
    /// such as GoreBox27's `gb27`). Two listings of one Steam game share the
    /// same art, so the Library tags a beta's; None until kryo.to has said.
    pub steam_branch: Option<String>,
    /// The kryo.to page this is linked to, if any.
    pub slug: Option<String>,
    pub cover: Option<String>,
    pub hero: Option<String>,
    /// Player-selected pictures, preserved independently of refreshed catalog art.
    pub custom_cover: Option<String>,
    pub custom_hero: Option<String>,
    /// The transparent title logo drawn over the hero, when Steam has one.
    pub logo: Option<String>,
    /// The wide store header, for rows and cards.
    pub header: Option<String>,
    /// Absolute folder the game lives in.
    pub install_dir: String,
    /// Exe to start when no Steam entry is picked, relative to `install_dir`.
    pub executable: String,
    /// The release's chosen arguments on kryo.to, used with `executable`.
    pub default_args: String,
    /// Steam's launch entries (Windows ones), from kryo.to.
    pub entries: Vec<LaunchEntry>,
    /// Release source label, e.g. "Steam + Kryoto Online".
    pub source: Option<String>,
    /// Which entry Play starts. `None` is the game's own exe (`executable`).
    pub preferred_entry: Option<usize>,
    /// Play asks how to start the game each time, whatever `preferred_entry` says.
    pub ask_each_time: bool,
    /// The Steam-style LAUNCH OPTIONS line.
    pub launch_options: String,
    /// Wine or Proton, for a non-Windows host.
    pub compat_tool: Option<String>,
    /// Add the release's WINEDLLOVERRIDES automatically under Wine/Proton.
    pub apply_overrides: bool,
    /// Windows: Play starts it elevated, with Windows' own prompt.
    pub run_as_admin: bool,
    /// The player changed its files (a mod, an online fix) and wants them left
    /// alone: no update is offered or fetched for it, and a repair left
    /// unfinished no longer stops Play. Reinstalling it clears this.
    pub keep_files: bool,
    pub playtime_seconds: u64,
    pub last_played: Option<u64>,
    pub added_at: u64,
    /// The kryo.to release version installed, when it came from a download -
    /// what "Update available" compares against.
    pub version: Option<String>,
    /// The SHA-256 of the archive it was installed from, when kryo.to listed
    /// one and the download matched it. A download replaced on kryo.to under
    /// the same build number, or a new upload with none, has another: that
    /// is an update too, where the version alone said nothing had changed.
    pub build_sha: Option<String>,
    /// A build picked under Versions rather than the current one: while it is
    /// the one installed, no update is offered. Like choosing a branch in
    /// Steam's Betas.
    pub pinned_version: Option<String>,
    /// A few words from kryo.to for the game page.
    pub short: Option<String>,
    pub developer: Option<String>,
    /// kryo.to marks it an adult game: its art is blurred unless Settings
    /// says to show adult art, as on the site.
    pub nsfw: bool,
    /// Add-ons put into it (language packs, the Online add-on), with the files
    /// each wrote, for Undo.
    pub addons: Vec<crate::addons::InstalledAddon>,
    /// Kryoto Online set up on this PC (not from a kryo.to add-on), for Undo.
    pub online: Option<crate::online::LocalOnline>,
    pub repair: Option<crate::repair::LocalRepair>,
    /// This game's in-game name, over Settings'; None follows Settings.
    pub player_name_mode: Option<crate::player_name::Mode>,
    pub player_name: String,
    /// This game's language, over Settings'. None follows Settings, empty
    /// leaves the build's own.
    pub game_language: Option<String>,
    /// Play time found in the game's folder when it was installed again (the
    /// old `.kryoto-game.json`), not counted yet: Play offers to restore it
    /// (`playtime_offer`). Cleared once answered.
    pub found_playtime: Option<u64>,
    /// The player's answer to "restore your playtime?": `never` stops the
    /// question for this game, `restored` after a yes. None: not asked, or
    /// "later".
    pub playtime_prompt: Option<String>,
    /// kryo.to's VR flag for the game: `vr_supported`, `vr_only`, or None.
    pub vr: Option<String>,
    /// Another launcher's game this came from: `hydra:<shop>:<objectId>`,
    /// `steam:<appid>`.
    pub origin: Option<String>,
    /// The game's Steam AppID when kryo.to does not list it, kept on this PC
    /// only, to recognise it when kryo.to does (bridge `offers`).
    pub steam_appid: Option<String>,
    /// The launcher that keeps this game (`steam`): its files are that
    /// launcher's and Play opens `launch_url`. Kryoto only lists it.
    pub launcher: Option<String>,
    pub launch_url: Option<String>,
    /// What the player chose for this game on Discord, where it differs from
    /// Settings: never shown, or an adult game shown anyway (discord_rpc/).
    pub discord_show: Option<bool>,
    /// The player keeps this to themselves: it sits in the Library's Hidden
    /// group, and nobody else sees it. It runs and updates as usual.
    pub hidden: bool,
    /// Changed here and not yet confirmed by the account (kryo.to's list wins
    /// otherwise). A game with no kryo.to page never leaves this PC.
    pub hidden_unsent: bool,
    /// Play time kryo.to has not had: Hydra's hours, and every session while
    /// the game is not linked to a kryo.to page. Never sent while unlinked.
    pub carry_seconds: u64,
    /// What linking it is sending to the account, once each (`carry_sent`).
    pub carry_sends: Vec<CarrySend>,
    /// kryo.to pages the player said this is not ("No" to linking it).
    pub merge_declined: Vec<String>,
}

/// Play time from before a game was linked, on its way to kryo.to under a key
/// of its own, so a send repeated after a lost answer is counted once.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CarrySend {
    pub key: String,
    pub seconds: u64,
}

#[derive(Default)]
pub struct Running(pub Mutex<HashMap<String, Arc<crate::handoff::Session>>>);

/// Whether this game is running, or on its way to it.
pub(crate) fn is_running<R: Runtime>(app: &AppHandle<R>, id: &str) -> bool {
    app.try_state::<Running>().is_some_and(|r| r.0.lock().map(|m| m.contains_key(id)).unwrap_or(false))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameEvent {
    id: String,
    running: bool,
    /// How long that session lasted, on exit.
    seconds: Option<u64>,
    /// Exit code, when there is one.
    code: Option<i32>,
    stopped: bool,
}

/// A Steam branch's name the way people say it ("Public beta", "GB27"), or None
/// for the public branch and for a title that already names it. The same rule
/// as the Library's BranchTag (src/library/BranchTag.tsx) and kryo.to's
/// `lib/steam-branch.ts`.
pub fn branch_label(branch: Option<&str>, title: &str) -> Option<String> {
    let name = branch.map(str::trim).filter(|b| !b.is_empty() && !b.eq_ignore_ascii_case("public"))?;
    let plain = |s: &str| {
        let words: Vec<String> = s.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect();
        format!(" {} ", words.join(" "))
    };
    if plain(title).contains(&plain(name)) {
        return None;
    }
    let words: Vec<String> = name
        .split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .enumerate()
        .map(|(i, w)| {
            if w.chars().any(|c| c.is_ascii_digit()) && w.chars().count() <= 6 {
                w.to_uppercase()
            } else if i == 0 {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect()).unwrap_or_default()
            } else {
                w.to_lowercase()
            }
        })
        .collect();
    Some(words.join(" "))
}

/// "GoreBox (Public beta branch)": the name with its branch, for notifications
/// and Discord, where no tag can be drawn.
pub fn title_with_branch(title: &str, branch: Option<&str>) -> String {
    match branch_label(branch, title) {
        Some(label) => format!("{title} ({label} branch)"),
        None => title.to_string(),
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn data_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// The library: `library.json`, or, when that cannot be read, what
/// [`recover`] makes of it. A damaged list never stops the app: before, every
/// later save failed on it too ("library.json is damaged"), so not even a new
/// download could be added.
pub(crate) fn load<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<LibraryGame>, String> {
    let file = data_dir(app)?.join("library.json");
    match std::fs::read(&file) {
        Ok(bytes) => match parse(&bytes) {
            Some(mut games) => {
                // An update used to record the folder in its canonical spelling.
                for g in games.iter_mut().filter(|g| g.install_dir.starts_with(r"\\?\")) {
                    g.install_dir = dunce::simplified(Path::new(&g.install_dir)).to_string_lossy().into_owned();
                }
                games.iter_mut().for_each(drop_unstartable_entries);
                keep_private(&games);
                Ok(games)
            }
            None => Ok(recover(app)),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Steam entries Play cannot start (KB-0263: a DLC's `index.html`), saved by
/// an older version, go; the entry picked in Properties keeps pointing at the
/// same one, or goes back to the game's own exe when it was one of those.
fn drop_unstartable_entries(g: &mut LibraryGame) {
    if g.entries.iter().all(LaunchEntry::is_windows) {
        return;
    }
    let picked = g.preferred_entry.and_then(|i| g.entries.get(i)).filter(|e| e.is_windows()).cloned();
    g.entries.retain(LaunchEntry::is_windows);
    g.preferred_entry = picked.and_then(|p| g.entries.iter().position(|e| *e == p));
}

/// The names, folders and exes of games kryo.to does not list, for the error
/// reports to leave out (logging.rs `keep_private`), and their pictures, which
/// load from where they are and not through kryo.to (art.rs `load_directly`).
fn keep_private(games: &[LibraryGame]) {
    let mut values = Vec::new();
    for g in games.iter().filter(|g| g.slug.is_none()) {
        crate::art::load_directly([&g.cover, &g.hero, &g.logo, &g.header, &g.custom_cover, &g.custom_hero].into_iter().flatten().cloned());
        if let Some(id) = &g.steam_appid {
            crate::art::load_app_directly(id);
        }
        values.push(g.title.clone());
        values.push(g.install_dir.clone());
        if let Some(name) = Path::new(&g.install_dir).file_name() {
            values.push(name.to_string_lossy().into_owned());
        }
        if let Some(exe) = Path::new(&g.executable.replace('\\', "/")).file_name() {
            values.push(exe.to_string_lossy().into_owned());
        }
    }
    crate::logging::keep_private(values);
}

/// Link a game to its kryo.to page: kryo.to's art, details and launch
/// entries over what it had, keeping where it is installed, its play time
/// and the player's own settings. What it was played for before is queued
/// for the account, once (`carry_sends`).
pub(crate) fn apply_meta(g: &mut LibraryGame, slug: &str, meta: &crate::downloads::CatalogMeta) {
    g.slug = Some(slug.to_string());
    if !meta.title.is_empty() {
        g.title = meta.title.clone();
    }
    g.cover = meta.cover.clone().or(g.cover.take());
    g.hero = meta.hero.clone().or(g.hero.take());
    g.logo = meta.logo.clone().or(g.logo.take());
    g.header = meta.header.clone().or(g.header.take());
    g.short = meta.short.clone();
    g.developer = meta.developer.clone();
    g.nsfw = meta.nsfw;
    g.steam_branch = meta.steam_branch.clone();
    g.vr = meta.vr.clone();
    g.source = meta.source.clone();
    g.default_args = meta.default_args.clone();
    g.entries = meta.entries.iter().filter(|e| e.is_windows()).cloned().collect();
    if g.preferred_entry.is_some_and(|i| i >= g.entries.len()) {
        g.preferred_entry = None;
    }
    if !meta.executable.is_empty() && !g.install_dir.is_empty() {
        let (root, exe) = crate::downloads::locate(Path::new(&g.install_dir), &meta.executable);
        if exe.to_ascii_lowercase().ends_with(&meta.executable.replace('\\', "/").to_ascii_lowercase()) {
            g.install_dir = root.to_string_lossy().into_owned();
            g.executable = exe;
        }
    }
    queue_carry(g);
}

/// Once linked, the time counted while it was not goes to the account under
/// a key of its own.
pub(crate) fn queue_carry(g: &mut LibraryGame) {
    let Some(slug) = g.slug.as_deref() else { return };
    if g.carry_seconds == 0 {
        return;
    }
    let short: String = slug.chars().take(40).collect();
    let key = format!("carry-{short}-{}-{}", now(), g.carry_sends.len());
    g.carry_sends.push(CarrySend { key, seconds: g.carry_seconds });
    g.carry_seconds = 0;
}

/// kryo.to has the hours sent under `key` (lib.rs `store_carry_done`): they
/// are not sent again.
pub(crate) fn carry_sent<R: Runtime>(app: &AppHandle<R>, id: &str, key: &str) -> Result<(), String> {
    update(app, |games| {
        if let Some(g) = games.iter_mut().find(|g| g.id == id) {
            g.carry_sends.retain(|c| c.key != key);
        }
        Ok(())
    })?;
    let _ = app.emit("library-changed", ());
    Ok(())
}

/// A library list, or `None` when the bytes are not one: empty, cut short,
/// or the run of zero bytes a file becomes when Windows lost power after
/// recording its size but before writing its data.
fn parse(bytes: &[u8]) -> Option<Vec<LibraryGame>> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.trim_matches(|c: char| c.is_whitespace() || c == '\0').is_empty() {
        return None;
    }
    serde_json::from_str(text).ok()
}

/// Write the list: to a temporary file, flushed to the disk, then swapped in,
/// so a crash leaves the old list or the new one, never half of one. The list
/// being replaced is kept as `library.json.bak` (when it was readable), which
/// is what [`recover`] goes back to first.
fn save<R: Runtime>(app: &AppHandle<R>, games: &[LibraryGame]) -> Result<(), String> {
    let dir = data_dir(app)?;
    let tmp = dir.join("library.json.tmp");
    let json = serde_json::to_string_pretty(games).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let current = dir.join("library.json");
    if let Ok(old) = std::fs::read(&current) {
        if parse(&old).is_some_and(|g| !g.is_empty()) {
            let _ = write_synced(&dir.join("library.json.bak"), &old);
        }
    }
    std::fs::rename(&tmp, current).map_err(|e| e.to_string())
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Each installed game also keeps its own entry in its folder
/// (`.kryoto-game.json`), so a lost list can be rebuilt exactly from the
/// folders themselves. Best effort: a read-only folder just has none.
pub(crate) fn write_marker(game: &LibraryGame) {
    // Another launcher's folder is that launcher's: nothing of Kryoto's goes in.
    if game.install_dir.is_empty() || game.launcher.is_some() {
        return;
    }
    let dir = Path::new(&game.install_dir);
    if !dir.is_dir() {
        return;
    }
    if let Ok(json) = serde_json::to_vec_pretty(game) {
        let _ = write_synced(&dir.join(MARKER), &json);
    }
}

const MARKER: &str = ".kryoto-game.json";

/// The entry a game folder keeps of itself, if it has one.
pub(crate) fn read_marker(dir: &Path) -> Option<LibraryGame> {
    if dir.as_os_str().is_empty() {
        return None;
    }
    std::fs::read(dir.join(MARKER)).ok().and_then(|b| serde_json::from_slice::<LibraryGame>(&b).ok())
}

/// A launcher link (`link2ea://...`) where a file should be: never stored as
/// the exe (launch.rs `is_url_scheme`).
fn is_url_like(exe: &str) -> bool {
    launch::is_url_scheme(exe)
}

/// Play time Play can offer back, and where it was found.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaytimeOffer {
    pub seconds: u64,
    /// `folder` (the game's own marker), `account` (kryo.to), or both.
    pub source: &'static str,
}

/// The offer for one game: the most either the folder or the account
/// remembers, when that is more than a minute above what this PC counts and
/// the player has not said no. Pure, for the tests.
pub fn offer_for(game: &LibraryGame, folder: Option<u64>, account: Option<u64>) -> Option<PlaytimeOffer> {
    if game.playtime_prompt.as_deref() == Some("never") {
        return None;
    }
    let local = game.playtime_seconds;
    let folder = folder.filter(|s| *s > local + 60);
    let account = account.filter(|s| *s > local + 60);
    let (seconds, source) = match (folder, account) {
        (Some(f), Some(a)) => (f.max(a), "both"),
        (Some(f), None) => (f, "folder"),
        (None, Some(a)) => (a, "account"),
        (None, None) => return None,
    };
    Some(PlaytimeOffer { seconds, source })
}

/// Before Play: is there play time to give back? `account_seconds` is what
/// kryo.to counts for this game on the signed-in account (none for a guest,
/// or when kryo.to did not answer). Also, for a game nothing knows the hours
/// of, the emulator's achievement times say whether it was played before:
/// then "Last played" shows when, though no hours can be restored.
#[tauri::command(async)]
pub fn playtime_offer(app: AppHandle, id: String, account_seconds: Option<u64>) -> Result<Option<PlaytimeOffer>, String> {
    let game = load(&app)?.into_iter().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
    let marker = read_marker(Path::new(&game.install_dir)).filter(|m| m.slug == game.slug).map(|m| m.playtime_seconds);
    // `found_playtime` is what the folder held when the game was installed
    // again; sessions since then count on top of it, and the marker on disk
    // has been rewritten with only those since the first of them.
    let found = game.found_playtime.map(|f| f.saturating_add(game.playtime_seconds));
    let folder = found.max(marker);
    let offer = offer_for(&game, folder, account_seconds);
    if offer.is_none() && game.last_played.is_none() && game.playtime_seconds == 0 {
        if let Some(at) = crate::achievements::last_unlock(&app, &game) {
            let _ = update(&app, |games| {
                if let Some(g) = games.iter_mut().find(|g| g.id == id) {
                    g.last_played = g.last_played.or(Some(at));
                }
                Ok(())
            });
            let _ = app.emit("library-changed", ());
        }
    }
    Ok(offer)
}

/// The answer to "restore your playtime?". `yes` takes the larger of the two
/// (sessions from here on keep adding, and kryo.to counts each session once,
/// so nothing is counted twice); `later` asks again next time; `no` never asks
/// again for this game.
#[tauri::command(async)]
pub fn playtime_answer(app: AppHandle, id: String, answer: String, seconds: u64) -> Result<LibraryGame, String> {
    let game = update(&app, |games| {
        let g = games.iter_mut().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
        match answer.as_str() {
            "yes" => {
                g.playtime_seconds = g.playtime_seconds.max(seconds.min(100 * 365 * 86_400));
                g.found_playtime = None;
                g.playtime_prompt = Some("restored".into());
            }
            "no" => {
                g.found_playtime = None;
                g.playtime_prompt = Some("never".into());
            }
            "later" => {}
            _ => return Err("Not an answer.".to_string()),
        }
        Ok(g.clone())
    })?;
    write_marker(&game);
    Ok(game)
}

/// One recovery at a time; the others wait and read its result.
static RECOVER: Mutex<()> = Mutex::new(());

/// `library.json` cannot be read. Keep it (renamed, for support), then:
///
/// 1. the list before the last change (`library.json.bak`), or else
/// 2. the game folders themselves: each one's `.kryoto-game.json`, else a
///    Forge release's `.kryoto-release.json`, else any folder with a game in
///    it, by its folder name. The ones not yet linked to kryo.to are looked
///    up there by name afterwards, for their art and launch options.
///
/// The result is saved, and the app says what happened.
fn recover<R: Runtime>(app: &AppHandle<R>) -> Vec<LibraryGame> {
    let Ok(_guard) = RECOVER.lock() else { return Vec::new() };
    let Ok(dir) = data_dir(app) else { return Vec::new() };
    let file = dir.join("library.json");
    // Another call got here first and already put a list back.
    if let Some(games) = std::fs::read(&file).ok().and_then(|b| parse(&b)) {
        return games;
    }
    let damaged = dir.join(format!("library.json.damaged-{}", now()));
    let _ = std::fs::rename(&file, &damaged);
    let (games, how) = match std::fs::read(dir.join("library.json.bak")).ok().and_then(|b| parse(&b)) {
        Some(games) if !games.is_empty() => (games, "its last good copy"),
        _ => (rebuild_from_folders(app), "the game folders"),
    };
    let _ = save(app, &games);
    crate::logging::error("library", &format!("library.json was damaged ({}); restored {} game(s) from {how}", damaged.display(), games.len()));
    let body = if games.is_empty() {
        "Kryoto could not find your games again. Add them with Add a game, or ask support: the damaged list is kept.".to_string()
    } else {
        format!("Kryoto put {} game(s) back from {how}. Nothing was deleted.", games.len())
    };
    let _ = app.emit("notify", crate::downloads::Notice::new("Your library list was damaged", &body, None));
    if games.iter().any(|g| g.slug.is_none()) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { relink(&app).await });
    }
    games
}

/// Every game folder in the library folders, as library entries.
fn rebuild_from_folders<R: Runtime>(app: &AppHandle<R>) -> Vec<LibraryGame> {
    let roots = library_roots(&crate::settings::load(app));
    let mut games: Vec<LibraryGame> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for root in roots {
        let Ok(read) = std::fs::read_dir(&root) else { continue };
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            // `.Title.installing` staging folders and other hidden ones.
            if !path.is_dir() || name.starts_with('.') || !seen.insert(path.clone()) {
                continue;
            }
            if let Some(game) = from_folder(&path, &name) {
                games.push(game);
            }
        }
    }
    // Unique ids, as `upsert_installed` makes them.
    let mut ids = std::collections::HashSet::new();
    for g in games.iter_mut() {
        let base = if g.id.is_empty() { g.slug.clone().unwrap_or_else(|| slugify(&g.title)) } else { g.id.clone() };
        let mut id = base.clone();
        let mut n = 2;
        while !ids.insert(id.clone()) {
            id = format!("{base}-{n}");
            n += 1;
        }
        g.id = id;
    }
    games
}

fn from_folder(path: &Path, name: &str) -> Option<LibraryGame> {
    let install_dir = path.to_string_lossy().into_owned();
    // Our own entry, written at install.
    if let Some(mut game) = std::fs::read(path.join(MARKER)).ok().and_then(|b| serde_json::from_slice::<LibraryGame>(&b).ok()) {
        game.install_dir = install_dir;
        return Some(game);
    }
    let release: Option<serde_json::Value> = std::fs::read(path.join(".kryoto-release.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let (root, executable) = crate::downloads::locate(path, "");
    if executable.is_empty() {
        return None; // not a game folder
    }
    let slug = release
        .as_ref()
        .and_then(|r| r["slug"].as_str())
        .filter(|s| !s.is_empty() && s.len() <= 160 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .map(String::from);
    let title = release.as_ref().and_then(|r| r["title"].as_str()).map(String::from).unwrap_or_else(|| name.to_string());
    Some(LibraryGame {
        title,
        slug,
        install_dir: root.to_string_lossy().into_owned(),
        executable,
        apply_overrides: true,
        added_at: now(),
        ..Default::default()
    })
}

fn library_roots(settings: &crate::settings::Settings) -> Vec<PathBuf> {
    crate::settings::all_folders(settings).into_iter().filter(|f| !f.trim().is_empty()).map(PathBuf::from).collect()
}

/// Games Kryoto installed that `games` does not list (suggestion #58): the
/// `.kryoto-game.json` each install leaves in its folder, in the library
/// folders and one level below them (a game whose files sit in a subfolder).
/// Only Kryoto's own markers, so nothing else on the drive is guessed at.
/// Reads one folder listing per game: run off the UI thread, on request.
fn marked_games(games: &[LibraryGame], roots: &[PathBuf]) -> Vec<LibraryGame> {
    // A game on the list whose folder has gone does not hold its place: what
    // is found under its name is that game, moved.
    let listed = |dir: &str, slug: Option<&str>| {
        games.iter().any(|g| crate::settings::same_path(&g.install_dir, dir) || (slug.is_some() && g.slug.as_deref() == slug && !folder_gone(g)))
    };
    let subdirs = |dir: &Path| -> Vec<PathBuf> {
        let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
        read.flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.') && e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect()
    };
    let mut found: Vec<LibraryGame> = Vec::new();
    for root in roots {
        for dir in subdirs(root) {
            // `_downloads` and the like hold no games.
            if dir.file_name().is_some_and(|n| n.to_string_lossy().starts_with('_')) {
                continue;
            }
            let hit = read_marker(&dir).map(|g| (dir.clone(), g)).or_else(|| subdirs(&dir).into_iter().find_map(|d| read_marker(&d).map(|g| (d, g))));
            let Some((dir, mut game)) = hit else { continue };
            game.install_dir = dir.to_string_lossy().into_owned();
            if listed(&game.install_dir, game.slug.as_deref()) || found.iter().any(|f| f.install_dir == game.install_dir || (game.slug.is_some() && f.slug == game.slug)) {
                continue;
            }
            found.push(game);
        }
    }
    found.sort_by_key(|g| g.title.to_lowercase());
    found
}

/// The game is on the list, but its folder is not where the list says.
fn folder_gone(g: &LibraryGame) -> bool {
    !g.install_dir.is_empty() && !Path::new(&g.install_dir).is_dir()
}

/// The listed game a found one is, when the listed one's folder has gone.
fn moved_from<'a>(games: &'a mut [LibraryGame], found: &LibraryGame) -> Option<&'a mut LibraryGame> {
    games.iter_mut().find(|g| folder_gone(g) && ((found.slug.is_some() && g.slug == found.slug) || (!found.id.is_empty() && g.id == found.id)))
}

/// Play on a game moved between library folders by hand: its marker says where
/// it went, so nobody has to go and point at it. Only looked for once the folder
/// on record has gone, off the UI thread.
fn find_moved<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Option<LibraryGame> {
    let games = load(app).ok()?;
    let roots = library_roots(&crate::settings::load(app));
    let found = marked_games(&games, &roots).into_iter().find(|f| (f.slug.is_some() && f.slug == game.slug) || f.id == game.id)?;
    let moved = update(app, |games| {
        let g = moved_from(games, &found).ok_or("The game has been found already.")?;
        g.install_dir = found.install_dir.clone();
        Ok(g.clone())
    })
    .ok()?;
    crate::logging::info("library", &format!("{} was moved; found it again at {}", moved.title, moved.install_dir));
    let _ = app.emit("library-changed", ());
    Some(moved)
}

/// One game the scan found, for the list the player picks from.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundGame {
    title: String,
    install_dir: String,
    cover: Option<String>,
    nsfw: bool,
    playtime_seconds: u64,
    /// Already on the list: this is where it went, not a game to add.
    moved: bool,
}

/// Settings > Storage's "Find games Kryoto installed", and the check after an
/// update: the games in the library folders that the library has lost.
#[tauri::command(async)]
pub fn library_find_kryoto_games(app: AppHandle) -> Result<Vec<FoundGame>, String> {
    let mut games = load(&app)?;
    let roots = library_roots(&crate::settings::load(&app));
    let found = marked_games(&games, &roots);
    Ok(found
        .into_iter()
        .map(|g| {
            let moved = moved_from(&mut games, &g).is_some();
            FoundGame { title: g.title, install_dir: g.install_dir, cover: g.custom_cover.or(g.cover), nsfw: g.nsfw, playtime_seconds: g.playtime_seconds, moved }
        })
        .collect())
}

/// Put the picked ones back, each as its own folder remembers it (play time,
/// launch options, art). Found again here rather than taken from the page,
/// so only a real marker in a library folder is ever added.
#[tauri::command(async)]
pub fn library_restore_found(app: AppHandle, dirs: Vec<String>) -> Result<usize, String> {
    let roots = library_roots(&crate::settings::load(&app));
    let added = update(&app, |games| {
        let picked: Vec<LibraryGame> = marked_games(games, &roots).into_iter().filter(|g| dirs.iter().any(|d| crate::settings::same_path(d, &g.install_dir))).collect();
        for mut game in picked.iter().cloned() {
            // Moved, not lost: the entry keeps its hours and settings, and only
            // its folder changes.
            if let Some(listed) = moved_from(games, &game) {
                listed.install_dir = game.install_dir.clone();
                continue;
            }
            let base = if game.id.is_empty() { game.slug.clone().unwrap_or_else(|| slugify(&game.title)) } else { game.id.clone() };
            let mut id = base.clone();
            let mut n = 2;
            while games.iter().any(|g| g.id == id) {
                id = format!("{base}-{n}");
                n += 1;
            }
            game.id = id;
            games.push(game);
        }
        Ok(picked.len())
    })?;
    if added > 0 {
        crate::logging::info("library", &format!("put {added} game(s) back from their folders"));
        let _ = app.emit("library-changed", ());
        if load(&app)?.iter().any(|g| g.slug.is_none()) {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { relink(&app).await });
        }
    }
    Ok(added)
}

/// Rebuilt entries with no kryo.to link: matched on this PC against Kryoto's
/// catalog index by name and exe (the folder is named after the game; its
/// name is never sent to be searched), then filled in with its art, launch
/// entries and details the way an install does. Only sure matches.
async fn relink<R: Runtime>(app: &AppHandle<R>) {
    let settings = crate::settings::load(app);
    let endpoint = crate::settings::catalog_endpoint(&settings);
    let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build() else { return };
    let Ok(games) = load(app) else { return };
    let Some(index) = crate::bridge::match_index(app, crate::bridge::INDEX_MAX_AGE).await else { return };
    let mut found: Vec<(String, String, crate::downloads::CatalogMeta)> = Vec::new();
    for g in games.iter().filter(|g| g.slug.is_none()) {
        let Some(slug) = crate::bridge::sure_match(&index, g) else { continue };
        if let Some(meta) = crate::downloads::fetch_meta(&client, &endpoint, &slug).await {
            found.push((g.id.clone(), slug, meta));
        }
    }
    if found.is_empty() {
        return;
    }
    let _ = update(app, |games| {
        for (id, slug, meta) in &found {
            let Some(g) = games.iter_mut().find(|g| &g.id == id) else { continue };
            apply_meta(g, slug, meta);
            write_marker(g);
        }
        Ok(())
    });
    let _ = app.emit("library-changed", ());
}

/// Ask kryo.to once which Steam branch each linked game builds from, for the
/// games installed before Desktop kept it. Off the main thread, one request
/// per such game, and only those: a game that has its branch is never asked.
pub fn fill_branches<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(games) = load(&app) else { return };
        let missing: Vec<(String, String)> = games
            .iter()
            .filter(|g| g.steam_branch.is_none())
            .filter_map(|g| Some((g.id.clone(), g.slug.clone()?)))
            .collect();
        if missing.is_empty() {
            return;
        }
        let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(&app));
        let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build() else { return };
        let mut found: Vec<(String, String)> = Vec::new();
        for (id, slug) in missing {
            if let Some(branch) = crate::downloads::fetch_meta(&client, &endpoint, &slug).await.and_then(|m| m.steam_branch) {
                found.push((id, branch));
            }
        }
        if found.is_empty() {
            return;
        }
        let _ = update(&app, |games| {
            for (id, branch) in &found {
                if let Some(g) = games.iter_mut().find(|g| &g.id == id) {
                    g.steam_branch = Some(branch.clone());
                }
            }
            Ok(())
        });
        let _ = app.emit("library-changed", ());
    });
}

/// Library writes are serialised: a game closing while Properties saves must
/// not lose either change.
static WRITE: Mutex<()> = Mutex::new(());

pub(crate) fn update<R: Runtime, T>(app: &AppHandle<R>, f: impl FnOnce(&mut Vec<LibraryGame>) -> Result<T, String>) -> Result<T, String> {
    let _guard = WRITE.lock().map_err(|_| "library lock poisoned")?;
    let mut games = load(app)?;
    let out = f(&mut games)?;
    save(app, &games)?;
    Ok(out)
}

fn slugify(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "game".into() } else { s }
}

#[tauri::command(async)]
pub fn library_list(app: AppHandle) -> Result<Vec<LibraryGame>, String> {
    load(&app)
}

/// Add a game from an exe on disk. `game` carries whatever the caller already
/// knows (title, kryo.to link, Steam entries); the folder is worked out here.
#[tauri::command(async)]
pub fn library_add(app: AppHandle, exe_path: String, mut game: LibraryGame) -> Result<LibraryGame, String> {
    let exe = PathBuf::from(&exe_path);
    if !exe.is_file() {
        return Err(format!("{exe_path} is not a file."));
    }
    let (root, rel) = added_place(&exe, &game.executable);
    game.install_dir = root.to_string_lossy().into_owned();
    game.executable = rel;
    if game.title.trim().is_empty() {
        game.title = exe.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Game".into());
    }
    game.entries.retain(LaunchEntry::is_windows);
    game.apply_overrides = true;
    game.added_at = now();
    update(&app, |games| {
        let base = game.slug.clone().unwrap_or_else(|| slugify(&game.title));
        let mut id = base.clone();
        let mut n = 2;
        while games.iter().any(|g| g.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        game.id = id;
        games.push(game.clone());
        Ok(game)
    })
}

/// "Locate": the game is somewhere else now, and this is the file that starts
/// it. Only where it is changes: its play time, launch settings, Wine prefix and
/// everything else stay on the entry. Picked at the path the release names, the
/// folder above it is the root, so a game's Steam entries still resolve.
#[tauri::command(async)]
pub fn library_relocate(app: AppHandle, id: String, exe_path: String) -> Result<LibraryGame, String> {
    let exe = PathBuf::from(&exe_path);
    if !exe.is_file() {
        return Err(format!("{exe_path} is not a file."));
    }
    let game = update(&app, |games| {
        let g = games.iter_mut().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
        let (root, rel) = added_place(&exe, &g.executable);
        // Another file than the one on record: the game's other ways to start
        // were worked out from the old layout.
        if !rel.eq_ignore_ascii_case(&g.executable.replace('\\', "/")) {
            g.preferred_entry = None;
            g.ask_each_time = false;
        }
        g.install_dir = root.to_string_lossy().into_owned();
        g.executable = rel;
        Ok(g.clone())
    })?;
    write_marker(&game);
    let _ = app.emit("library-changed", ());
    Ok(game)
}

/// Manage > Rename folder: the game's folder in plain letters (see
/// `downloads::plain_name`), for a game installed before new installs got
/// them, or one added from a folder of the player's own. Some games will not
/// start from a path with `’` or `ö` in it. Only the name changes: the folder
/// stays where it is, and play time, settings and saves come along.
#[tauri::command(async)]
pub fn library_plain_folder(app: AppHandle, id: String) -> Result<LibraryGame, String> {
    if is_running(&app, &id) {
        return Err("Close the game first: its files are in use.".into());
    }
    let game = load(&app)?.into_iter().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
    if game.launcher.is_some() {
        return Err("This game's folder belongs to its own launcher.".into());
    }
    let install = dunce::canonicalize(&game.install_dir).map_err(|_| "The game's folder is not there any more.".to_string())?;
    let folders = crate::settings::all_folders(&crate::settings::load(&app));
    // The folder the game is in at the top of a library folder, so a game
    // whose root is a level down is renamed where the name was given.
    let top = crate::storage::game_folder(&folders, &game.install_dir).map(|d| dunce::simplified(&d).to_path_buf()).unwrap_or_else(|| install.clone());
    let (Some(parent), Some(name)) = (top.parent(), top.file_name().map(|n| n.to_string_lossy().into_owned())) else {
        return Err("This game's folder cannot be renamed.".into());
    };
    let plain = crate::downloads::plain_name(&name, game.slug.as_deref());
    if plain == name {
        let above = install.to_string_lossy();
        return Err(if above.is_ascii() {
            "The folder's name is already plain letters.".into()
        } else {
            format!("The game's own folder is plain already; a folder above it is not ({above}). Move the game to another library folder in Settings > Storage.")
        });
    }
    let to = parent.join(&plain);
    if to.exists() {
        return Err(format!("There is already a folder called {plain} beside it. Rename or remove that one first."));
    }
    std::fs::rename(&top, &to).map_err(|e| match e.raw_os_error() {
        // Sharing violation and access denied: something has a file in it open.
        Some(5) | Some(32) => format!("Windows would not rename {name}: something has a file in it open. Close the game, its launcher and any folder window showing it, then try again."),
        _ => format!("Could not rename {name}: {e}"),
    })?;
    let rest = install.strip_prefix(&top).map(Path::to_path_buf).unwrap_or_default();
    let moved = update(&app, |games| {
        let g = games.iter_mut().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
        g.install_dir = if rest.as_os_str().is_empty() { to.clone() } else { to.join(&rest) }.to_string_lossy().into_owned();
        Ok(g.clone())
    });
    let game = match moved {
        Ok(g) => g,
        Err(e) => {
            // The list could not take the new place: put the old name back.
            let _ = std::fs::rename(&to, &top);
            return Err(e);
        }
    };
    crate::logging::info("library", &format!("renamed {name} to {plain}"));
    write_marker(&game);
    let _ = app.emit("library-changed", ());
    Ok(game)
}

/// Is this folder there? For the game page, which says so before Play does.
#[tauri::command(async)]
pub fn folder_exists(path: String) -> bool {
    Path::new(&path).is_dir()
}

/// Where Add a game puts a picked exe: its game root and the exe relative to it.
///
/// A linked release names its exe relative to the game's root, e.g.
/// `bin/win64/Game.exe`. When the picked file ends with that path, the root
/// is the folder above it, so Steam's entries resolve; otherwise the exe's
/// own folder is the root (any game of your own, Windows .exe or not).
fn added_place(exe: &Path, executable: &str) -> (PathBuf, String) {
    let wanted = executable.replace('\\', "/").to_ascii_lowercase();
    let picked = exe.to_string_lossy().replace('\\', "/");
    if !wanted.is_empty() && picked.to_ascii_lowercase().ends_with(&format!("/{wanted}")) {
        let cut = picked.len() - wanted.len() - 1;
        (PathBuf::from(&picked[..cut]), executable.replace('\\', "/"))
    } else {
        let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (exe.parent().map(Path::to_path_buf).unwrap_or_default(), name)
    }
}

/// Background art resolution must not overwrite a player's newer Properties edits.
#[tauri::command(async)]
pub fn library_artwork(
    app: AppHandle,
    id: String,
    cover: Option<String>,
    hero: Option<String>,
    logo: Option<String>,
    header: Option<String>,
) -> Result<LibraryGame, String> {
    update(&app, |games| {
        let game = games.iter_mut().find(|g| g.id == id).ok_or("Game not found.")?;
        game.cover = cover.or(game.cover.take());
        game.hero = hero.or(game.hero.take());
        game.logo = logo.or(game.logo.take());
        game.header = header.or(game.header.take());
        Ok(game.clone())
    })
}

/// Put a game that a download just installed into the library: a new entry,
/// or the existing one for the same kryo.to page with its folder and version
/// moved on (an update), keeping playtime and the player's own settings.
pub fn upsert_installed<R: Runtime>(app: &AppHandle<R>, mut game: LibraryGame) -> Result<LibraryGame, String> {
    game.entries.retain(LaunchEntry::is_windows);
    if is_url_like(&game.executable) {
        game.executable.clear();
    }
    // What the folder remembers from an earlier install, read before this
    // install writes its own marker over it.
    let remembered = read_marker(Path::new(&game.install_dir));
    update(app, |games| {
        if let Some(slug) = game.slug.clone() {
            if let Some(slot) = games.iter_mut().find(|g| g.slug.as_deref() == Some(slug.as_str())) {
                slot.install_dir = game.install_dir.clone();
                slot.executable = game.executable.clone();
                slot.default_args = game.default_args.clone();
                slot.entries = game.entries.clone();
                slot.source = game.source.clone();
                // Repair metadata describes the previous installed files.
                slot.repair = None;
                // These are Kryoto's files again.
                slot.keep_files = false;
                slot.version = game.version.clone();
                slot.build_sha = game.build_sha.clone();
                slot.pinned_version = game.pinned_version.clone();
                slot.logo = game.logo.clone().or(slot.logo.take());
                slot.header = game.header.clone().or(slot.header.take());
                slot.cover = game.cover.clone().or(slot.cover.take());
                slot.hero = game.hero.clone().or(slot.hero.take());
                slot.short = game.short.clone().or(slot.short.take());
                slot.developer = game.developer.clone().or(slot.developer.take());
                slot.nsfw = game.nsfw;
                slot.vr = game.vr.clone().or(slot.vr.take());
                slot.steam_branch = game.steam_branch.clone().or(slot.steam_branch.take());
                if slot.preferred_entry.is_some_and(|i| i >= slot.entries.len()) {
                    slot.preferred_entry = None;
                }
                return Ok(slot.clone());
            }
        }
        let base = game.slug.clone().unwrap_or_else(|| slugify(&game.title));
        let mut id = base.clone();
        let mut n = 2;
        while games.iter().any(|g| g.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        game.id = id;
        game.apply_overrides = true;
        game.added_at = now();
        // Installed again into a folder that remembers hours played: offered
        // back on the first Play, never added by itself.
        if let Some(old) = remembered.as_ref().filter(|m| m.slug.is_some() && m.slug == game.slug) {
            game.found_playtime = (old.playtime_seconds > 0).then_some(old.playtime_seconds);
            game.last_played = game.last_played.or(old.last_played);
        }
        games.push(game.clone());
        Ok(game)
    })
    .inspect(write_marker)
    .inspect(|game| {
        #[cfg(target_os = "linux")]
        if let Err(e) = sync_proton_script(app, game) { crate::logging::error("proton-script", &e); }
        #[cfg(not(target_os = "linux"))]
        let _ = game;
    })
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn sync_proton_script<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Result<Option<PathBuf>, String> {
    if !game.executable.to_ascii_lowercase().ends_with(".exe") { return Ok(None); }
    let prefix = data_dir(app)?.join("prefixes").join(&game.id);
    let managed = crate::compat::managed_dir(app);
    let umu = crate::compat::umu_run(managed.as_deref());
    let default_plan = plan_for(app, game, None).ok().map(|p| p.0);
    let env = default_plan.as_ref().map(|p| p.env.as_slice()).unwrap_or(&[]);
    let args = default_plan.as_ref().map(|p| p.game_args.as_str()).unwrap_or(&game.default_args);
    crate::proton_script::write(Path::new(&game.install_dir), &game.executable, args, &prefix, env, umu.as_deref()).map(Some)
}

/// Why Windows would not start a program, in words that say what to do. Windows'
/// own text arrives in the PC's language and names no way out.
fn start_failure(program: &Path, e: &std::io::Error) -> String {
    let name = program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| program.display().to_string());
    let appimage = program.extension().is_some_and(|x| x.eq_ignore_ascii_case("appimage"));
    match e.raw_os_error() {
        // ERROR_VIRUS_INFECTED
        Some(225) if cfg!(windows) => format!("Your antivirus blocked {name} as a threat. Exclude the game's folder, then get the file back: both are on the game's page."),
        // ERROR_SXS_CANT_GEN_ACTCTX: an older Visual C++ runtime is missing.
        Some(14001) if cfg!(windows) => format!("{name} needs an older Visual C++ runtime. Install everything under Settings > Game runtimes, then press Play again."),
        // ERROR_BAD_EXE_FORMAT
        Some(193) if cfg!(windows) && appimage => format!("{name} is the Linux version. Download the Windows version from the game's page."),
        _ => format!("Could not start {}: {e}", program.display()),
    }
}

pub(crate) fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => total += entry.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

/// Where a game's Wine prefix lives, whether or not it has one yet. An id is
/// checked, because this path gets deleted.
fn prefix_root<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<PathBuf, String> {
    if !is_prefix_name(id) {
        return Err("Not a game.".into());
    }
    Ok(data_dir(app)?.join("prefixes").join(id))
}

fn is_prefix_name(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A Wine prefix no game in the library uses any more.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeftoverPrefix {
    id: String,
    path: String,
    bytes: u64,
    /// Unix seconds it last changed.
    modified: Option<u64>,
}

/// The folders under `prefixes` that are not one of `games`' ids.
fn leftover_prefix_dirs(root: &Path, games: &[LibraryGame]) -> Vec<(String, PathBuf)> {
    let Ok(read) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = read
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(name, _)| is_prefix_name(name) && !games.iter().any(|g| &g.id == name))
        .collect();
    out.sort();
    out
}

/// Settings > Storage: prefixes left behind by games that were uninstalled.
#[tauri::command]
pub async fn storage_leftover_prefixes(app: AppHandle) -> Result<Vec<LeftoverPrefix>, String> {
    if cfg!(windows) {
        return Ok(Vec::new());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let games = load(&app)?;
        let root = data_dir(&app)?.join("prefixes");
        Ok(leftover_prefix_dirs(&root, &games)
            .into_iter()
            .map(|(id, dir)| LeftoverPrefix {
                modified: std::fs::metadata(&dir).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()),
                bytes: dir_size(&dir),
                path: dir.to_string_lossy().into_owned(),
                id,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Delete one of those. Refused for a prefix a game in the library still uses,
/// whatever the page asked.
#[tauri::command]
pub async fn storage_delete_prefix(app: AppHandle, id: String) -> Result<(), String> {
    if cfg!(windows) {
        return Ok(());
    }
    let dir = prefix_root(&app, &id)?;
    if load(&app)?.iter().any(|g| g.id == id) {
        return Err("A game in your library still uses that prefix.".into());
    }
    crate::logging::info("storage", &format!("deleting the leftover prefix {id}"));
    tauri::async_runtime::spawn_blocking(move || if dir.is_dir() { std::fs::remove_dir_all(dir) } else { Ok(()) })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Could not delete the prefix: {e}"))
}

/// Bytes the game's folder takes, for Installed Files.
/// A game's Wine prefix, for "Browse Wine prefix" in its Manage menu: the
/// folder Wine or Proton keeps its Windows in (the C: drive, the registry,
/// most saves and crash dumps). `None` until the game has been started once,
/// and always on Windows, where there is none.
#[tauri::command]
pub fn library_prefix_folder(app: AppHandle, id: String) -> Result<Option<String>, String> {
    if cfg!(windows) {
        return Ok(None);
    }
    let root = prefix_root(&app, &id)?;
    if !root.is_dir() {
        return Ok(None);
    }
    // Proton keeps the prefix proper one level down, beside its own files.
    let pfx = root.join("pfx");
    Ok(Some((if pfx.is_dir() { pfx } else { root }).to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn game_disk_size(install_dir: String) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || dir_size(Path::new(&install_dir)))
        .await
        .map_err(|e| e.to_string())
}

/// Save the settings the Properties window edits. Playtime is the app's own
/// record and is kept from disk, never taken from the window.
#[tauri::command(async)]
pub fn library_save(app: AppHandle, game: LibraryGame) -> Result<LibraryGame, String> {
    update(&app, |games| {
        let slot = games.iter_mut().find(|g| g.id == game.id).ok_or("That game is no longer in the library.")?;
        launch::inside(Path::new(&game.install_dir), &game.executable)?;
        let mut next = game;
        next.entries.retain(LaunchEntry::is_windows);
        next.playtime_seconds = slot.playtime_seconds;
        next.last_played = slot.last_played;
        next.added_at = slot.added_at;
        // Answered through playtime_answer only, never the Properties window.
        next.found_playtime = slot.found_playtime;
        next.playtime_prompt = slot.playtime_prompt.clone();
        // Where it came from and what it still owes the account (bridge).
        next.origin = slot.origin.clone();
        next.steam_appid = slot.steam_appid.clone();
        next.launcher = slot.launcher.clone();
        // Set by discord_game_set, at once, wherever the player flips it.
        next.discord_show = slot.discord_show;
        // Likewise library_set_hidden.
        next.hidden = slot.hidden;
        next.hidden_unsent = slot.hidden_unsent;
        next.launch_url = slot.launch_url.clone();
        next.carry_seconds = slot.carry_seconds;
        next.carry_sends = slot.carry_sends.clone();
        next.merge_declined = slot.merge_declined.clone();
        if launch::is_url_scheme(&next.executable) {
            return Err("That is a launcher link, not a file. Pick the game's own .exe.".into());
        }
        // Only applying and undoing change these, never the Properties window.
        next.addons = slot.addons.clone();
        next.online = slot.online.clone();
        let had_repair = next.repair.is_some() || slot.repair.is_some();
        next.repair = slot.repair.clone();
        if had_repair { next.source = slot.source.clone(); next.apply_overrides = slot.apply_overrides; }
        next.player_name = crate::player_name::clean(&next.player_name).unwrap_or_default();
        next.game_language = next.game_language.map(|l| crate::player_name::clean_language(&l).unwrap_or_default());
        if next.preferred_entry.is_some_and(|i| i >= next.entries.len()) {
            next.preferred_entry = None;
        }
        *slot = next.clone();
        Ok(next)
    })
    .inspect(write_marker)
    .inspect(|_| crate::discord_rpc::refresh(&app))
}

/// Hide a game, or show it again. A "show anyway" on Discord was for the game
/// as it was: hiding starts from nothing on Discord, whatever came before.
fn set_hidden(g: &mut LibraryGame, hidden: bool) {
    if g.hidden != hidden && g.discord_show == Some(true) {
        g.discord_show = None;
    }
    g.hidden = hidden;
}

/// Hide games or show them again: here at once, and on the account when the
/// Store can reach it. `ids` are games on this PC; `slugs` are kryo.to games
/// that are not; `adult` also takes every adult game on the account.
#[tauri::command(async)]
pub fn library_set_hidden(app: AppHandle, ids: Vec<String>, slugs: Vec<String>, hidden: bool, adult: bool) -> Result<(), String> {
    let mut send = slugs;
    if !ids.is_empty() {
        let changed = update(&app, |games| {
            let mut changed = Vec::new();
            for g in games.iter_mut().filter(|g| ids.contains(&g.id)) {
                set_hidden(g, hidden);
                g.hidden_unsent = g.slug.is_some();
                changed.extend(g.slug.clone());
            }
            Ok(changed)
        })?;
        send.extend(changed);
        let _ = app.emit("library-changed", ());
        crate::discord_rpc::refresh(&app);
    }
    if send.is_empty() && !adult {
        return Ok(());
    }
    // Games on this PC go later if the Store cannot take them now (`account_hidden`).
    crate::store_send_hidden(&app, &send, hidden, adult, true).or_else(|e| if ids.is_empty() { Err(e) } else { Ok(()) })
}

/// What should change on a library so it follows the account's hidden list:
/// the games to set here, and the ones whose change here has not reached the
/// account yet. Pure, for the tests.
fn hidden_differences(games: &[LibraryGame], account: &HashSet<String>) -> (Vec<(String, bool)>, Vec<String>, Vec<String>) {
    let (mut set, mut hide, mut show) = (Vec::new(), Vec::new(), Vec::new());
    for g in games {
        let Some(slug) = &g.slug else { continue };
        let there = account.contains(slug);
        if g.hidden_unsent && there != g.hidden {
            if g.hidden { hide.push(slug.clone()) } else { show.push(slug.clone()) }
        } else if g.hidden != there || g.hidden_unsent {
            set.push((g.id.clone(), there));
        }
    }
    (set, hide, show)
}

/// The account's hidden list, as the Store's page read it: what the player
/// changed here and has not reached it goes up, and everything else follows it.
pub(crate) fn account_hidden<R: Runtime>(app: &AppHandle<R>, account: &[String]) -> Result<(), String> {
    let account: HashSet<String> = account.iter().cloned().collect();
    let (set, hide, show) = hidden_differences(&load(app)?, &account);
    if !set.is_empty() {
        update(app, |games| {
            for (id, hidden) in &set {
                if let Some(g) = games.iter_mut().find(|g| &g.id == id) {
                    set_hidden(g, *hidden);
                    g.hidden_unsent = false;
                }
            }
            Ok(())
        })?;
        let _ = app.emit("library-changed", ());
        crate::discord_rpc::refresh(app);
    }
    // No refresh after these: a site that keeps refusing must not become a loop.
    if !hide.is_empty() {
        let _ = crate::store_send_hidden(app, &hide, true, false, false);
    }
    if !show.is_empty() {
        let _ = crate::store_send_hidden(app, &show, false, false, false);
    }
    Ok(())
}

/// Take it off the list, and with `delete_files` uninstall it too.
///
/// Files are only ever deleted from inside one of the library folders in
/// Settings > Storage - folders this client installs into - and then the
/// game's whole folder there goes, not just the one holding the exe. A game
/// added from elsewhere on the PC is the player's folder, not ours, and
/// removing it only unlists it.
///
/// `delete_prefix` (Linux) also takes the game's Wine prefix, with whatever
/// the game kept in it. Left behind otherwise, and listed in Settings > Storage.
#[tauri::command]
pub async fn library_remove(app: AppHandle, id: String, delete_files: bool, delete_prefix: Option<bool>) -> Result<bool, String> {
    // Its files are in use: the delete would stop partway, leaving half a game.
    if is_running(&app, &id) {
        return Err("Stop the game first.".into());
    }
    let game = load(&app)?.into_iter().find(|g| g.id == id);
    if delete_prefix.unwrap_or(false) && game.is_some() {
        let dir = prefix_root(&app, &id)?;
        tauri::async_runtime::spawn_blocking(move || if dir.is_dir() { std::fs::remove_dir_all(dir) } else { Ok(()) })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("Could not delete the game's Wine prefix: {e}"))?;
    }
    let mut deleted = false;
    // Another launcher's game is only ever unlisted here.
    if let (Some(game), true) = (game.as_ref().filter(|g| g.launcher.is_none()), delete_files) {
        let folders = crate::settings::all_folders(&crate::settings::load(&app));
        if let Some(dir) = crate::storage::game_folder(&folders, &game.install_dir) {
            crate::logging::info("library", &format!("uninstalling {} from {}", game.title, dir.display()));
            tauri::async_runtime::spawn_blocking(move || {
                // The originals its add-ons replaced go with it.
                let _ = std::fs::remove_dir_all(crate::addons::backups_of(&dir));
                std::fs::remove_dir_all(dir)
            })
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("Could not delete the game's files: {e}"))?;
            deleted = true;
            if let Some(slug) = game.slug.clone() {
                let data = crate::game_update::data_dir(&app);
                tauri::async_runtime::spawn_blocking(move || crate::game_update::forget(&data, &slug));
            }
        }
    }
    update(&app, |games| {
        games.retain(|g| g.id != id);
        Ok(())
    })?;
    crate::downloads::forget_game(&app, &id);
    crate::pad_games::forget(&app, &id);
    if let Some(game) = &game {
        crate::shortcuts::remove(&app, game);
    }
    Ok(deleted)
}

/// The entry a plain Play starts, for a start with nobody to ask (kryo.to's
/// remote, the phone): the one picked in Properties, else the game's own exe
/// (`None`), or Steam's first entry for a game with no exe on record. Matches
/// `startEntry` in `src/lib/library.ts`.
pub(crate) fn default_entry<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<usize> {
    let games = load(app).ok()?;
    let game = games.iter().find(|g| g.id == id)?;
    match game.preferred_entry {
        Some(i) if i < game.entries.len() => Some(i),
        _ if game.executable.trim().is_empty() && !game.entries.is_empty() => Some(0),
        _ => None,
    }
}

/// The plan, and the compatibility tool it uses (for the game's log).
pub(crate) fn plan_for<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, entry: Option<usize>) -> Result<(LaunchPlan, Option<PathBuf>), String> {
    let entry = match entry {
        Some(i) => Some(game.entries.get(i).filter(|e| e.is_windows()).ok_or("That launch entry no longer exists.")?),
        None => None,
    };
    let prefix = data_dir(app)?.join("prefixes").join(&game.id);
    // The game's own pick, else the one in Settings, else the best one found
    // on this computer (newest Proton, then umu-run, then Wine).
    let settings = crate::settings::load(app);
    let managed = crate::compat::managed_dir(app);
    let umu = crate::compat::umu_run(managed.as_deref());
    // Settings > Compatibility's switches, only when the tool is really there.
    let mut wrappers = Vec::new();
    let mut env = Vec::new();
    if !cfg!(windows) {
        if settings.linux_gamemode {
            wrappers.extend(crate::compat::on_path("gamemoderun").map(|p| p.to_string_lossy().into_owned()));
        }
        if settings.linux_mangohud {
            wrappers.extend(crate::compat::on_path("mangohud").map(|p| p.to_string_lossy().into_owned()));
        }
        if settings.linux_fsr {
            env.push(("WINE_FULLSCREEN_FSR".to_string(), "1".to_string()));
        }
        // Settings > Controller's remaps, for SDL, Wine and Proton.
        env.extend(crate::pad::game_env(app, &game.id));
    }
    let tool_path = tool_for(app, game, &settings)?;
    let tool = tool_path.as_deref();
    let plan = launch::plan(&launch::PlanInput {
        install_dir: Path::new(&game.install_dir),
        executable: &game.executable,
        default_args: &game.default_args,
        entry,
        launch_options: &game.launch_options,
        compat_tool: tool,
        prefix_dir: &prefix,
        source: game.source.as_deref(),
        apply_overrides: game.apply_overrides,
        windows_host: cfg!(windows),
        umu: umu.as_deref(),
        wrappers,
        env,
    })?;
    Ok((plan, tool_path))
}

fn tool_for<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, settings: &crate::settings::Settings) -> Result<Option<PathBuf>, String> {
    // A start script is this system's own program, with no Wine to find.
    if cfg!(windows) || launch::is_shell_script(&game.executable) {
        return Ok(None);
    }
    let saved = game.compat_tool.as_deref().filter(|t| !t.trim().is_empty()).or(settings.default_compat_tool.as_deref().filter(|t| !t.trim().is_empty()));
    crate::compat::resolve_tool(saved, &crate::compat::detect_in(crate::compat::managed_dir(app).as_deref()))
}

/// The exact line Play would run, for the Properties window.
#[tauri::command(async)]
pub fn game_launch_preview(app: AppHandle, game: LibraryGame, entry: Option<usize>) -> Result<String, String> {
    let (mut plan, tool) = plan_for(&app, &game, entry)?;
    if cfg!(target_os = "linux") && tool.as_deref().is_some_and(|p| crate::compat::kind_of(p) == "proton") {
        if let Some(script) = sync_proton_script(&app, &game)? {
            crate::proton_script::use_script(&mut plan, &script)?;
        }
    }
    Ok(plan.display())
}

#[tauri::command]
pub fn game_running(running: State<'_, Running>) -> Vec<String> {
    running.0.lock().map(|m| m.iter().filter(|(_, s)| !s.stopped()).map(|(id, _)| id.clone()).collect()).unwrap_or_default()
}

/// Start a game. `entry` is the Steam launch entry picked, or `None` for the
/// release default.
#[tauri::command]
pub async fn game_launch(app: AppHandle, running: State<'_, Running>, id: String, entry: Option<usize>, join_lobby: Option<String>) -> Result<(), String> {
    launch_with(app, running, id, entry, join_lobby, false).await
}

/// "Play in VR" for a game that also plays flat (kryo.to `vr_supported`):
/// the same start, with the headset's runtime made ready first (vr.rs).
#[tauri::command]
pub async fn game_launch_vr(app: AppHandle, running: State<'_, Running>, id: String, entry: Option<usize>) -> Result<(), String> {
    launch_with(app, running, id, entry, None, true).await
}

async fn launch_with(app: AppHandle, running: State<'_, Running>, id: String, entry: Option<usize>, join_lobby: Option<String>, vr: bool) -> Result<(), String> {
    // A game another launcher keeps: that launcher starts it, and has its hours.
    if let Some(link) = load(&app)?.into_iter().find(|g| g.id == id).and_then(|g| g.launch_url) {
        // Off the async workers: an opener gets a moment to fail (linux_open.rs).
        prepare_launch(move || crate::bridge::open_launcher_link(&link)).await?;
        let played = update(&app, |games| {
            let g = games.iter_mut().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
            g.last_played = Some(now());
            Ok(())
        });
        let _ = app.emit("library-changed", ());
        return played;
    }
    // Its new files are about to go over the old ones (downloads.rs `install`).
    if load(&app)?.iter().find(|g| g.id == id).and_then(|g| g.slug.as_deref()).is_some_and(|slug| crate::downloads::unpacking(&app, slug)) {
        return Err("It is being updated. Press Play once that is done.".into());
    }
    let session = Arc::new(crate::handoff::Session::default());
    {
        let mut all = running.0.lock().map_err(|_| "state lock poisoned")?;
        if let Some(s) = all.get(&id) {
            if !s.stopped() {
                return Err("It is already running.".into());
            }
            if s.pid().is_some() {
                return Err("It is still closing. Press Play again in a moment.".into());
            }
        }
        all.insert(id.clone(), session.clone());
    }
    let task_id = id.clone();
    let task_session = session.clone();
    // Launch preparation includes synchronous file/process work and an async
    // achievement fetch driven by block_on. It must run outside Tokio's async
    // workers: nesting block_on there panics before any game is spawned.
    let result = prepare_launch(move || launch_reserved(app, task_id, entry, join_lobby, task_session, vr)).await;
    if result.is_err() {
        if let Ok(mut all) = running.0.lock() {
            if all.get(&id).is_some_and(|s| Arc::ptr_eq(s, &session)) {
                all.remove(&id);
            }
        }
    }
    result
}

async fn prepare_launch(task: impl FnOnce() -> Result<(), String> + Send + 'static) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(task).await.map_err(|e| format!("Could not prepare the game launch: {e}")).and_then(|result| result)
}

fn launch_reserved(app: AppHandle, id: String, entry: Option<usize>, join_lobby: Option<String>, session: Arc<crate::handoff::Session>, vr: bool) -> Result<(), String> {
    let mut game = load(&app)?.into_iter().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
    if folder_gone(&game) {
        if let Some(moved) = find_moved(&app, &game) {
            game = moved;
        }
    }
    if !cfg!(windows) {
        let settings = crate::settings::load(&app);
        let managed = crate::compat::managed_dir(&app);
        // Recover the broken ARM install left by earlier releases if it is
        // Kryoto's own tool and there is no usable replacement on the PC.
        let mut tool = tool_for(&app, &game, &settings);
        let saved = game.compat_tool.as_deref().or(settings.default_compat_tool.as_deref());
        if tool.is_err() && saved.is_some_and(|p| managed.as_ref().is_some_and(|m| Path::new(p).starts_with(m))) {
            tauri::async_runtime::block_on(crate::compat::compat_install(app.clone()))?;
            tool = tool_for(&app, &game, &settings);
        }
        if tool?.is_some_and(|p| crate::compat::kind_of(&p) == "proton") && crate::compat::umu_run(managed.as_deref()).is_none() {
            tauri::async_runtime::block_on(crate::compat::ensure_runtime(&app))?;
        }
    }
    if session.stopped() {
        return Err("Launch cancelled.".into());
    }
    let (mut plan, tool) = plan_for(&app, &game, entry)?;
    crate::repair::can_launch(&app, &game)?;
    // From a game invite: start straight into the host's Steam lobby.
    if let Some(arg) = join_lobby.as_deref().and_then(crate::lobbies::connect_arg) {
        plan.game_args = format!("{} {arg}", plan.game_args).trim().to_string();
    }
    if !plan.exe.is_file() {
        // The exe moved inside the folder (an update re-laid it out, or the
        // archive unpacked one level deeper than the release says). Look for a
        // file of the same name before giving up, and keep what was found.
        let found = plan
            .exe
            .file_name()
            .and_then(|name| find_by_name(Path::new(&game.install_dir), name, 5));
        let Some(found) = found else {
            // Kryoto installed it, and it has gone: not a wrong setting.
            let rel = plan.exe.strip_prefix(&game.install_dir).map(|r| r.to_string_lossy().into_owned()).unwrap_or_default();
            if !rel.is_empty() && crate::antivirus::was_installed(Path::new(&game.install_dir), &rel) {
                let name = plan.exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(rel);
                return Err(format!("{name} is gone from the game's folder: it was there when Kryoto installed the game. The game's page says what removed it, if Windows Security did, and gets it back."));
            }
            return Err(format!(
                "{} is not there any more. Point Properties at the game's .exe.",
                plan.exe.display()
            ));
        };
        let old = plan.exe.to_string_lossy().into_owned();
        if plan.program == plan.exe {
            plan.program = found.clone();
        }
        for arg in plan.lead_args.iter_mut() {
            if *arg == old {
                *arg = found.to_string_lossy().into_owned();
            }
        }
        if !plan.cwd.is_dir() || plan.exe.parent() == Some(plan.cwd.as_path()) {
            plan.cwd = found.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(&game.install_dir));
        }
        if entry.is_none() {
            if let Ok(rel) = found.strip_prefix(&game.install_dir) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                let _ = update(&app, |games| {
                    if let Some(g) = games.iter_mut().find(|g| g.id == id) {
                        g.executable = rel.clone();
                    }
                    Ok(())
                });
            }
        }
        crate::logging::info("library", &format!("{old} was gone; found it at {}", found.display()));
        plan.exe = found;
    }
    // A working folder that is not there (os error 267, "The directory name is
    // invalid") would refuse the start outright: the exe's own folder instead.
    if !plan.cwd.is_dir() {
        plan.cwd = plan.exe.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(&game.install_dir));
    }
    if let Some(prefix) = plan.env.iter().find(|(k, _)| k == "WINEPREFIX" || k == "STEAM_COMPAT_DATA_PATH") {
        std::fs::create_dir_all(&prefix.1).map_err(|e| format!("Could not create the game's compatibility prefix: {e}"))?;
    }
    // Online-Fix and Kryoto Online under Proton: Steam's client files (compat.rs).
    let steam_note = if cfg!(target_os = "linux") && tool.as_deref().is_some_and(|p| crate::compat::kind_of(p) == "proton") {
        std::env::var_os("HOME").and_then(|home| crate::compat::steam_client_for(&mut plan.env, Path::new(&home)))
    } else {
        None
    };

    // Newer saves another PC backed up, put here before the game reads them.
    crate::saves::before_launch(&app, &game);
    // The name the player picked, into the emulator's files, before it reads them.
    crate::player_name::apply_for_launch(&app, &game);
    // The achievement list gbe_fork needs, into a build made without one.
    crate::achievements::before_launch(&game);
    // A VR start: the headset's runtime running (SteamVR started first, as
    // Steam does) and, when Settings picked one, the game pointed at it.
    let picked_entry = entry.and_then(|i| game.entries.get(i));
    let vr_note = crate::vr::wants_vr(game.vr.as_deref(), picked_entry, vr).then(|| {
        let (env, note) = crate::vr::prepare(&app, &crate::settings::load(&app));
        for (k, v) in env {
            if !plan.env.iter().any(|(key, _)| *key == k) {
                plan.env.push((k, v));
            }
        }
        crate::logging::info("vr", &format!("{}: {note}", game.title));
        note
    });

    if cfg!(target_os = "linux") && tool.as_deref().is_some_and(|p| crate::compat::kind_of(p) == "proton") {
        match sync_proton_script(&app, &game) {
            Ok(Some(script)) => crate::proton_script::use_script(&mut plan, &script)?,
            Ok(None) => {}
            // A folder that cannot be written (a read-only drive): the script is
            // a convenience, and the game still starts without it.
            Err(e) => crate::logging::warn("library", &e),
        }
    }
    // Finding this game's saves precisely: Windows asks for administrator
    // once, before the game starts, so its first writes are seen too.
    let precise = crate::saves::precise_before_launch(&app, &game);
    if session.stopped() {
        return Err("Launch cancelled.".into());
    }
    let mut cmd = launch::command(&plan);
    // Everything the game (and Wine/Proton) prints goes into its log (game_logs.rs).
    cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let log = crate::game_logs::begin(&app, &game, &plan, tool.as_deref());
    if let (Some(log), Some(note)) = (&log, vr_note.as_deref().filter(|n| !n.is_empty())) {
        log.note(note);
    }
    if let (Some(log), Some(note)) = (&log, steam_note.as_deref()) {
        log.note(note);
    }
    let before = crate::handoff::existing();
    session.track(Path::new(&game.install_dir), &before);
    // Properties > General: this game always starts elevated.
    #[cfg(windows)]
    let spawned = if game.run_as_admin {
        if let Some(log) = &log {
            log.note("Started as administrator, as set in Properties. Its output is not in this log.");
        }
        session.spawn(&mut elevated(&plan))
    } else {
        session.spawn(&mut cmd)
    };
    #[cfg(not(windows))]
    let spawned = session.spawn(&mut cmd);
    let mut child = match spawned {
        Ok(child) => child,
        // ERROR_ELEVATION_REQUIRED: the exe's manifest asks for administrator
        // rights, which a plain start cannot give. Ask Windows to elevate it
        // (the UAC prompt), through PowerShell so there is still a process to
        // wait on for "Playing" and the playtime.
        #[cfg(windows)]
        Err(e) if e.raw_os_error() == Some(740) => {
            if let Some(log) = &log {
                log.note("The game asks for administrator rights: started through Windows' prompt. Its output is not in this log.");
            }
            session.spawn(&mut elevated(&plan)).map_err(|e| {
                let msg = format!("Could not start {} as administrator: {e}", plan.program.display());
                if let Some(log) = &log {
                    log.note(&msg);
                    log.finish(None, 0, None);
                }
                msg
            })?
        }
        Err(e) => {
            let msg = start_failure(&plan.program, &e);
            if let Some(log) = &log {
                log.note(&msg);
                log.finish(None, 0, None);
            }
            return Err(msg);
        }
    };
    if let Some(log) = &log { log.capture(&mut child); }
    let pid = child.id();
    let started = now();
    let started_at = std::time::Instant::now();
    let _ = update(&app, |games| {
        if let Some(g) = games.iter_mut().find(|g| g.id == id) {
            g.last_played = Some(started);
        }
        Ok(())
    });
    let open_session = OpenSession::start(&app, &id, started_at);
    crate::presence::beat(&app);
    crate::discord_rpc::started(&app, &id, started);
    let _ = app.emit("game-state", GameEvent { id: id.clone(), running: true, seconds: None, code: None, stopped: false });
    crate::achievements::watch(app.clone(), &game, crate::achievements::roots_for(&plan.env).with_game(Path::new(&game.install_dir)));
    // A community update's playtest: its screenshots, taken from the game itself.
    crate::community_offer::during_play(&app, &game, pid);
    // Where it writes while it runs, to find its saves (saves/tracer.rs).
    let tracer = crate::saves::tracer::start(&app, &game, session.clone(), precise);
    if crate::settings::load(&app).minimize_on_play {
        if let Some(w) = app.get_window("main") {
            let _ = w.minimize();
        }
    }

    let install_dir = PathBuf::from(&game.install_dir);
    std::thread::spawn(move || {
        let status = child.wait().ok();
        let exited_after = started_at.elapsed().as_secs();
        let mut code = status.and_then(|s| s.code());
        let mut followed = None;
        // A quick exit is often a hand-over (a launcher, a game restarting
        // itself), not a crash: follow the process now running from the game's
        // folder, so Stop, "Playing" and the playtime all carry on with it.
        if !session.stopped() {
            // Failed starts should report immediately, without a 20s search
            // concealing the error. Already-running children still count.
            let next = crate::handoff::find_in_folder(&install_dir, pid, &before).or_else(|| {
                status
                    .filter(|s| s.success() && exited_after < crate::handoff::HANDOFF_WITHIN_SECS)
                    .and_then(|_| crate::handoff::look_for(&install_dir, pid, &before, &session))
            });
            if let Some(next) = next {
                if let Err(e) = session.follow(next) {
                    crate::logging::error("library", &e);
                }
                crate::logging::info("library", &format!("{id}: followed the game to process {next}"));
                followed = Some(crate::handoff::follow_chain(&session, &install_dir, next, &before));
                code = None;
            }
        }
        let seconds = if followed.is_some() { started_at.elapsed().as_secs() } else { exited_after };
        if let Some(log) = &log {
            log.finish(code, seconds, followed);
        }
        if let Some(state) = app.try_state::<Running>() {
            if let Ok(mut m) = state.0.lock() {
                if m.get(&id).is_some_and(|s| Arc::ptr_eq(s, &session)) {
                    m.remove(&id);
                }
            }
        }
        open_session.close();
        let counted = update(&app, |games| {
            if let Some(g) = games.iter_mut().find(|g| g.id == id) {
                add_playtime(g, seconds);
            }
            Ok(())
        });
        if let Err(e) = counted {
            crate::logging::error("library", &format!("{id}: {seconds}s of play could not be saved: {e}"));
        }
        crate::community_offer::after_play(&install_dir, seconds, code);
        crate::presence::beat(&app);
        crate::discord_rpc::ended(&app, &id);
        let _ = app.emit("game-state", GameEvent { id: id.clone(), running: false, seconds: Some(seconds), code, stopped: session.stopped() });
        // Learn where it saved, then back the saves up.
        crate::saves::after_exit(app.clone(), id, tracer);
    });
    Ok(())
}

fn add_playtime(g: &mut LibraryGame, seconds: u64) {
    g.playtime_seconds += seconds;
    // A game kryo.to does not list: the hours stay here, and go
    // to the account only if it is linked one day.
    if g.slug.is_none() {
        g.carry_seconds += seconds;
    }
    // The folder keeps its own copy of the hours, so a reinstall
    // into it (or a rebuilt library) can give them back without a
    // server (`playtime_offer`).
    write_marker(g);
}

/// How long a running game has been going, written down each minute. A play
/// session only reached the library when the game closed, so Kryoto crashing or
/// being closed during one lost all of it; `recover_sessions` adds what this left.
struct OpenSession {
    running: Arc<std::sync::atomic::AtomicBool>,
    file: Option<PathBuf>,
}

impl OpenSession {
    fn start<R: Runtime>(app: &AppHandle<R>, id: &str, started: std::time::Instant) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        let running = Arc::new(AtomicBool::new(true));
        let dir = data_dir(app).ok().filter(|_| is_prefix_name(id)).map(|d| d.join("open-sessions"));
        let file = dir.as_ref().map(|d| d.join(id));
        if let (Some(dir), Some(file)) = (dir, file.clone()) {
            let running = running.clone();
            std::thread::spawn(move || {
                let _ = std::fs::create_dir_all(&dir);
                let mut ticks = 0u32;
                while running.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    ticks += 1;
                    if ticks.is_multiple_of(60) && running.load(Ordering::SeqCst) {
                        let _ = std::fs::write(&file, started.elapsed().as_secs().to_string());
                    }
                }
            });
        }
        Self { running, file }
    }

    /// The game closed and its time is about to be counted the normal way.
    fn close(self) {
        self.running.store(false, std::sync::atomic::Ordering::SeqCst);
        if let Some(file) = self.file {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// The sessions in `dir` and how long each had run, emptying it.
fn take_open_sessions(dir: &Path) -> Vec<(String, u64)> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found = Vec::new();
    for entry in read.flatten() {
        let seconds = std::fs::read_to_string(entry.path()).ok().and_then(|s| s.trim().parse::<u64>().ok());
        if let (Some(seconds), Ok(id)) = (seconds, entry.file_name().into_string()) {
            found.push((id, seconds));
        }
        let _ = std::fs::remove_file(entry.path());
    }
    found
}

/// At start: count the play time of sessions Kryoto did not live to see end.
pub fn recover_sessions<R: Runtime>(app: &AppHandle<R>) {
    let Ok(dir) = data_dir(app).map(|d| d.join("open-sessions")) else { return };
    for (id, seconds) in take_open_sessions(&dir) {
        crate::logging::info("library", &format!("{id}: kept {seconds}s of play from before Kryoto closed"));
        let _ = update(app, |games| {
            if let Some(g) = games.iter_mut().find(|g| g.id == id) {
                add_playtime(g, seconds);
            }
            Ok(())
        });
    }
}

/// The first file called `name` under `dir`, shallowest first, at most `depth` deep.
fn find_by_name(dir: &Path, name: &std::ffi::OsStr, depth: usize) -> Option<PathBuf> {
    let mut level = vec![dir.to_path_buf()];
    for _ in 0..=depth {
        let mut next = Vec::new();
        for d in level {
            let Ok(read) = std::fs::read_dir(&d) else { continue };
            for e in read.flatten() {
                let path = e.path();
                if path.is_dir() {
                    next.push(path);
                } else if path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&name.to_string_lossy()))
                {
                    return Some(path);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    None
}

/// PowerShell's `Start-Process -Verb RunAs`, waiting for the game to end.
#[cfg(windows)]
fn elevated(plan: &launch::LaunchPlan) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let mut script = "$ErrorActionPreference='Stop'; ".to_string();
    for (key, value) in &plan.env {
        script.push_str(&format!("[Environment]::SetEnvironmentVariable({}, {}, 'Process'); ", quote(key), quote(value)));
    }
    script.push_str(&format!(
        "try {{ $game = Start-Process -Verb RunAs -Wait -PassThru -FilePath {} -WorkingDirectory {}",
        quote(&plan.program.to_string_lossy()),
        quote(&plan.cwd.to_string_lossy()),
    ));
    let args: Vec<String> = plan.lead_args.iter().cloned().chain((!plan.game_args.trim().is_empty()).then(|| plan.game_args.clone())).collect();
    if !args.is_empty() {
        script.push_str(&format!(" -ArgumentList {}", quote(&args.join(" "))));
    }
    // 1223, Windows' own "cancelled": the prompt was turned down, and the shell
    // tells that from a game that failed (`quickExit`).
    script.push_str("; exit $game.ExitCode } catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1223 }");
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script]);
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    cmd
}

pub(crate) const NOT_RUNNING: &str = "It is not running.";

/// Close a running game and everything it started.
#[tauri::command(async)]
pub fn game_stop(running: State<'_, Running>, id: String) -> Result<(), String> {
    let session = running.0.lock().map_err(|_| "state lock poisoned")?.get(&id).cloned().ok_or(NOT_RUNNING)?;
    session.stop()
}

/// Show a folder in the file manager.
///
/// Never fails on a folder that is not there. "C:\\Users\\...\\Kryoto Games is
/// not a folder" was one of the most reported errors: a library folder that
/// had never been created yet (nothing installed into it), or a game folder
/// somebody deleted by hand, and the press threw instead of showing anything.
/// `create` makes the folder (a library folder, which is meant to exist);
/// otherwise the nearest folder above it that does exist is shown.
#[tauri::command(async)]
pub fn open_folder(path: String, create: Option<bool>) -> Result<(), String> {
    let mut dir = PathBuf::from(&path);
    if !dir.is_dir() && create.unwrap_or(false) && !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }
    while !dir.is_dir() {
        match dir.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => dir = parent.to_path_buf(),
            _ => return Err(format!("{path} is not there any more.")),
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    return crate::linux_open::open(&dir.to_string_lossy());
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        std::process::Command::new(program).arg(&dir).spawn().map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{added_place, drop_unstartable_entries, from_folder, hidden_differences, parse, prepare_launch, set_hidden, slugify, title_with_branch, LibraryGame, MARKER};
    use crate::launch::LaunchEntry;

    /// KB-0263: a library saved before the fix keeps the DLC page as an
    /// entry; it goes on load, and the picked entry follows its own.
    #[test]
    fn a_saved_page_entry_goes_and_the_pick_follows() {
        let e = |exe: &str| LaunchEntry { executable: exe.into(), ..Default::default() };
        let mut g = LibraryGame { entries: vec![e("Game.exe"), e("Cosplay DLC/index.html"), e("Editor.exe")], preferred_entry: Some(2), ..Default::default() };
        drop_unstartable_entries(&mut g);
        assert_eq!(g.entries, vec![e("Game.exe"), e("Editor.exe")]);
        assert_eq!(g.preferred_entry, Some(1), "still Editor.exe");
        let mut picked_page = LibraryGame { entries: vec![e("Game.exe"), e("Cosplay DLC/index.html")], preferred_entry: Some(1), ..Default::default() };
        drop_unstartable_entries(&mut picked_page);
        assert_eq!(picked_page.preferred_entry, None, "back to the game's own exe");
    }

    #[test]
    fn a_beta_branch_is_named_where_no_tag_can_be_drawn() {
        assert_eq!(title_with_branch("GoreBox", Some("public-beta")), "GoreBox (Public beta branch)");
        assert_eq!(title_with_branch("GoreBox", Some("gb27")), "GoreBox (GB27 branch)");
        assert_eq!(title_with_branch("GoreBox", Some("public")), "GoreBox");
        assert_eq!(title_with_branch("GoreBox", None), "GoreBox");
        // A title that already names its branch is left alone.
        assert_eq!(title_with_branch("GoreBox GB27", Some("gb27")), "GoreBox GB27");
    }

    #[test]
    fn a_hidden_game_follows_the_account_unless_it_changed_here_first() {
        let game = |id: &str, hidden: bool, unsent: bool| LibraryGame { id: id.into(), slug: Some(id.into()), hidden, hidden_unsent: unsent, ..Default::default() };
        let account = ["on-site".to_string(), "here-and-there".to_string()].into_iter().collect();
        let games = [
            // Hidden on the site or on another PC: follows it.
            game("on-site", false, false),
            // Shown again on the site: follows that too.
            game("shown-on-site", true, false),
            // Hidden here, by a guest or with the Store away: goes up, stays hidden.
            game("hid-here", true, true),
            // Shown here while the account still hides it: goes up.
            game("here-and-there", false, true),
            // Nothing to do.
            game("plain", false, false),
            // A game with no kryo.to page is only this PC's business.
            LibraryGame { id: "own".into(), hidden: true, ..Default::default() },
        ];
        let (set, hide, show) = hidden_differences(&games, &account);
        assert_eq!(set, [("on-site".to_string(), true), ("shown-on-site".to_string(), false)]);
        assert_eq!(hide, ["hid-here"]);
        assert_eq!(show, ["here-and-there"]);

        // The account answered with what was sent: nothing left to do but mark it settled.
        let settled = [game("on-site", true, true)];
        let (set, hide, show) = hidden_differences(&settled, &account);
        assert_eq!(set, [("on-site".to_string(), true)]);
        assert!(hide.is_empty() && show.is_empty());
    }

    #[test]
    fn hiding_starts_from_nothing_on_discord() {
        let mut g = LibraryGame { discord_show: Some(true), ..Default::default() };
        set_hidden(&mut g, true);
        assert_eq!((g.hidden, g.discord_show), (true, None));
        // A game kept off Discord stays off it.
        let mut g = LibraryGame { discord_show: Some(false), ..Default::default() };
        set_hidden(&mut g, true);
        assert_eq!(g.discord_show, Some(false));
    }

    #[test]
    fn a_game_of_your_own_is_rooted_at_its_exe_and_a_release_at_its_root() {
        let (root, rel) = added_place(std::path::Path::new("/home/moo/VN/Mighty Moo.exe"), "");
        assert_eq!(root, std::path::PathBuf::from("/home/moo/VN"));
        assert_eq!(rel, "Mighty Moo.exe");
        let (root, rel) = added_place(std::path::Path::new("/games/Hades/x64/Hades.exe"), "x64\\Hades.exe");
        assert_eq!(root, std::path::PathBuf::from("/games/Hades"));
        assert_eq!(rel, "x64/Hades.exe");
    }

    /// Suggestion #64: a Windows game Kryoto does not list, added with Add a
    /// game and started with Play on Linux, through the Wine or Proton on this
    /// PC. Run by hand on each distro with a stand-in that writes its folder:
    ///
    /// `KRYOTO_ADDED_EXE=<path to a Windows .exe> KRYOTO_COMPAT=<wine or proton path> [KRYOTO_PREFIX=<booted prefix>] cargo test --lib -- --ignored added_windows_exe`
    #[test]
    #[ignore]
    fn an_added_windows_exe_starts_through_the_compat_tool() {
        let exe = std::path::PathBuf::from(std::env::var("KRYOTO_ADDED_EXE").expect("KRYOTO_ADDED_EXE"));
        let tool = std::path::PathBuf::from(std::env::var("KRYOTO_COMPAT").expect("KRYOTO_COMPAT"));
        // What library_add stores for a game of your own.
        let (root, rel) = added_place(&exe, "");
        let log = root.join("launch-log.txt");
        let _ = std::fs::remove_file(&log);
        // A prefix booted beforehand (`wineboot -u`) keeps the first start quick.
        let own_prefix = std::env::var("KRYOTO_PREFIX").ok();
        let prefix = own_prefix.clone().map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("kryoto-added-prefix-{}", std::process::id())));
        // What plan_for passes for it: no kryo.to source, overrides on, the
        // compat tool in place of the Settings default.
        let plan = crate::launch::plan(&crate::launch::PlanInput {
            install_dir: &root,
            executable: &rel,
            default_args: "",
            entry: None,
            launch_options: "",
            compat_tool: Some(&tool),
            prefix_dir: &prefix,
            source: None,
            apply_overrides: true,
            windows_host: cfg!(windows),
            umu: None,
            wrappers: Vec::new(),
            env: Vec::new(),
        })
        .unwrap();
        eprintln!("{}", plan.display());
        let mut child = crate::launch::command(&plan).spawn().unwrap();
        // A first Wine start builds its prefix, which takes a while.
        for _ in 0..3000 {
            if std::fs::read_to_string(&log).map(|s| s.contains("os=windows-exe")).unwrap_or(false) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        let text = std::fs::read_to_string(&log).expect("the game never started: no launch-log.txt");
        let _ = std::fs::remove_file(&log);
        if own_prefix.is_none() {
            let _ = std::fs::remove_dir_all(&prefix);
        }
        assert!(text.contains("os=windows-exe"), "{text}");
    }

    #[test]
    fn achievement_setup_in_an_async_launch_reaches_the_game_process() {
        let root = std::env::temp_dir().join(format!("kryoto-async-launch-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("steam_appid.txt"), "620").unwrap();
        let game = LibraryGame { install_dir: root.to_string_lossy().into_owned(), ..Default::default() };
        let cwd = root.clone();
        let result = tauri::async_runtime::block_on(async move {
            prepare_launch(move || {
                // This used to nest block_on on an async worker, panic, and
                // leave Play waiting forever. No Steam request is needed here.
                crate::achievements::before_launch(&game);
                let plan = crate::launch::LaunchPlan {
                    program: std::path::PathBuf::from(if cfg!(windows) { "cmd" } else { "true" }),
                    game_args: if cfg!(windows) { "/C exit 0".into() } else { String::new() },
                    lead_args: Vec::new(),
                    env: Vec::new(),
                    cwd,
                    exe: std::path::PathBuf::new(),
                };
                let status = crate::launch::command(&plan).status().map_err(|e| e.to_string())?;
                if status.success() {
                    Ok(())
                } else {
                    Err("The stand-in game did not start.".into())
                }
            })
            .await
        });
        std::fs::remove_dir_all(root).unwrap();
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn preparation_panics_reject_the_launch_instead_of_losing_its_ipc_reply() {
        let result = tauri::async_runtime::block_on(prepare_launch(|| panic!("synthetic launch failure")));
        assert!(result.unwrap_err().contains("Could not prepare the game launch"));
    }

    #[test]
    fn a_zeroed_or_empty_list_is_damaged_not_empty() {
        // What a player sent: 23,597 zero bytes, a file Windows had sized but
        // never written.
        assert!(parse(&vec![0u8; 23_597]).is_none());
        assert!(parse(b"").is_none());
        assert!(parse(b"  
").is_none());
        assert!(parse(b"[{\"id\":").is_none());
        assert_eq!(parse(b"[]").map(|g| g.len()), Some(0));
        assert_eq!(parse(br#"[{"id":"a","title":"A"}]"#).map(|g| g[0].title.clone()), Some("A".into()));
    }

    #[test]
    fn folders_come_back_as_games() {
        let root = std::env::temp_dir().join(format!("kryoto-rebuild-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // An install with its marker: back exactly as it was.
        let marked = root.join("Celeste");
        std::fs::create_dir_all(marked.join("bin")).unwrap();
        std::fs::write(marked.join("bin/Celeste.exe"), b"MZ").unwrap();
        let entry = LibraryGame { id: "celeste".into(), title: "Celeste".into(), slug: Some("celeste".into()), executable: "bin/Celeste.exe".into(), playtime_seconds: 3600, ..Default::default() };
        std::fs::write(marked.join(MARKER), serde_json::to_vec(&entry).unwrap()).unwrap();
        let g = from_folder(&marked, "Celeste").unwrap();
        assert_eq!((g.slug.as_deref(), g.playtime_seconds, g.install_dir.as_str()), (Some("celeste"), 3600, marked.to_string_lossy().as_ref()));
        // An older install: named after its folder, its game found inside.
        let plain = root.join("Hades II");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("Hades2.exe"), b"MZ").unwrap();
        std::fs::write(plain.join("unins000.exe"), b"MZ").unwrap();
        let g = from_folder(&plain, "Hades II").unwrap();
        assert_eq!((g.title.as_str(), g.executable.as_str(), g.slug), ("Hades II", "Hades2.exe", None));
        // Not a game.
        let empty = root.join("notes");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(from_folder(&empty, "notes").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_scan_finds_only_lost_kryoto_installs() {
        use super::marked_games;
        let root = std::env::temp_dir().join(format!("kryoto-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mark = |dir: &std::path::Path, slug: &str, title: &str| {
            std::fs::create_dir_all(dir).unwrap();
            let entry = LibraryGame { id: slug.into(), title: title.into(), slug: Some(slug.into()), playtime_seconds: 60, ..Default::default() };
            std::fs::write(dir.join(MARKER), serde_json::to_vec(&entry).unwrap()).unwrap();
        };
        mark(&root.join("Celeste"), "celeste", "Celeste");
        // Its files one level down, as some archives unpack.
        mark(&root.join("Hollow Knight").join("Hollow Knight"), "hollow-knight", "Hollow Knight");
        // Already in the library, by folder and by page.
        mark(&root.join("Hades"), "hades", "Hades");
        mark(&root.join("Moved Tunic"), "tunic", "Tunic");
        // Not ours, and not game folders.
        std::fs::create_dir_all(root.join("Other").join("x")).unwrap();
        mark(&root.join("_downloads"), "partial", "Partial");
        mark(&root.join(".Celeste.installing"), "staging", "Staging");
        let library = vec![
            LibraryGame { id: "hades".into(), install_dir: root.join("Hades").to_string_lossy().into(), ..Default::default() },
            // Somewhere that exists: with its folder gone, this copy would be it, moved.
            LibraryGame { id: "tunic".into(), slug: Some("tunic".into()), install_dir: std::env::temp_dir().to_string_lossy().into(), ..Default::default() },
        ];
        let found = marked_games(&library, &[root.clone(), root.join("missing")]);
        let titles: Vec<&str> = found.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["Celeste", "Hollow Knight"]);
        assert!(found[1].install_dir.ends_with("Hollow Knight") && found[1].install_dir.contains(&format!("Hollow Knight{}Hollow Knight", std::path::MAIN_SEPARATOR)));
        assert_eq!(found[0].playtime_seconds, 60);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn playtime_is_offered_back_once_and_only_when_more() {
        use super::{offer_for, PlaytimeOffer};
        let mut g = LibraryGame { playtime_seconds: 120, ..Default::default() };
        // Nothing more anywhere, or only a few seconds more: nothing to ask.
        assert_eq!(offer_for(&g, None, None), None);
        assert_eq!(offer_for(&g, Some(150), Some(170)), None);
        // The folder remembers more (an uninstall that kept the files).
        assert_eq!(offer_for(&g, Some(7_200), None), Some(PlaytimeOffer { seconds: 7_200, source: "folder" }));
        // The account counts more (played on another PC too): the larger wins.
        assert_eq!(offer_for(&g, Some(7_200), Some(9_000)), Some(PlaytimeOffer { seconds: 9_000, source: "both" }));
        assert_eq!(offer_for(&g, None, Some(9_000)), Some(PlaytimeOffer { seconds: 9_000, source: "account" }));
        // "No" is for good.
        g.playtime_prompt = Some("never".into());
        assert_eq!(offer_for(&g, Some(7_200), Some(9_000)), None);
    }

    #[test]
    fn a_reinstall_into_the_old_folder_finds_its_hours() {
        use super::{read_marker, take_open_sessions};
        let open =std::env::temp_dir().join(format!("kryoto-open-sessions-{}", std::process::id()));
        std::fs::create_dir_all(&open).unwrap();
        std::fs::write(open.join("celeste"), "1800").unwrap();
        std::fs::write(open.join("junk"), "not a number").unwrap();
        assert_eq!(take_open_sessions(&open), vec![("celeste".to_string(), 1800)]);
        assert!(take_open_sessions(&open).is_empty(), "a session is counted once");
        let _ = std::fs::remove_dir_all(&open);
        let root = std::env::temp_dir().join(format!("kryoto-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(read_marker(&root).is_none());
        let old = LibraryGame { slug: Some("celeste".into()), playtime_seconds: 5_400, last_played: Some(1_700_000_000), install_dir: root.to_string_lossy().into(), ..Default::default() };
        super::write_marker(&old);
        let back = read_marker(&root).unwrap();
        assert_eq!((back.playtime_seconds, back.last_played, back.slug.as_deref()), (5_400, Some(1_700_000_000), Some("celeste")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_game_moved_by_hand_is_found_as_itself_with_its_hours() {
        use super::{folder_gone, marked_games, moved_from, write_marker};
        let root = std::env::temp_dir().join(format!("kryoto-moved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (library, new_home) = (root.join("Library"), root.join("Library/Celeste 2"));
        std::fs::create_dir_all(&new_home).unwrap();
        // What the folder took with it, and the list that still has the old place.
        let on_list = LibraryGame { id: "celeste".into(), slug: Some("celeste".into()), playtime_seconds: 5_400, install_dir: root.join("Old/Celeste").to_string_lossy().into(), ..Default::default() };
        write_marker(&LibraryGame { install_dir: new_home.to_string_lossy().into(), ..on_list.clone() });
        let mut games = vec![on_list];
        assert!(folder_gone(&games[0]));
        let found = marked_games(&games, std::slice::from_ref(&library));
        assert_eq!(found.len(), 1);
        assert_eq!(std::path::Path::new(&found[0].install_dir), new_home);
        moved_from(&mut games, &found[0]).expect("the listed game").install_dir = found[0].install_dir.clone();
        assert_eq!(games[0].playtime_seconds, 5_400);
        // Back in place, it is listed: nothing to find, and nothing is moved.
        assert!(!folder_gone(&games[0]));
        assert!(marked_games(&games, std::slice::from_ref(&library)).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_of_a_game_still_in_the_library_is_never_a_leftover() {
        use super::{is_prefix_name, leftover_prefix_dirs};
        let root = std::env::temp_dir().join(format!("kryoto-prefixes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("celeste/pfx")).unwrap();
        std::fs::create_dir_all(root.join("gone-game/pfx")).unwrap();
        std::fs::write(root.join("a-file"), b"x").unwrap();
        let games = vec![LibraryGame { id: "celeste".into(), ..Default::default() }];
        let left = leftover_prefix_dirs(&root, &games);
        assert_eq!(left.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["gone-game"]);
        assert!(leftover_prefix_dirs(&root.join("nothing-here"), &games).is_empty());
        // An id is part of a path that gets deleted.
        for bad in ["", "..", "../home", "a/b", "a\\b", "."] {
            assert!(!is_prefix_name(bad), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ids_are_readable() {
        assert_eq!(slugify("Captain Hardcore"), "captain-hardcore");
        assert_eq!(slugify("  ?? "), "game");
    }
}
