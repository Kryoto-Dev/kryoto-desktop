//! Cloud saves: a game's saves on the player's kryo.to account, put back on
//! any PC they play on. A supporter perk, next to cloud achievements
//! (achievements_cloud.rs), behind kryo.to's `cloud_saves` feature flag.
//!
//! Where a game saves is worked out by `kryoto_saves` from exact sources (the
//! emulator's config, the engine, the Ludusavi database, what other players'
//! PCs saw) and from what this PC saw the game write while it ran
//! (`tracer.rs`). `sync.rs` backs the saves up when the game closes and, before
//! it starts, puts back saves another PC made since. Everything for one game
//! lives in `<app data>/saves/<game id>/`: its rules, its sync state, a hash
//! cache, and local backups of whatever a restore replaced.

pub mod api;
mod known;
pub mod precise;
mod sync;
pub mod tracer;
#[cfg(windows)]
mod win;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kryoto_saves::rules::{merge, GameHints};
use kryoto_saves::{Kind, Rule, SaveRoots, Source};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::library::LibraryGame;

pub use sync::{after_exit, before_launch};

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/* ── where things are kept ─────────────────────────────────────────── */

pub(crate) fn base_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    Some(app.path().app_data_dir().ok()?.join("saves"))
}

/// A game's own folder of cloud-save state. Ids are the library's, which are
/// slugs or generated; anything else is replaced so it stays one folder.
pub(crate) fn game_data<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<PathBuf> {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(80).collect();
    Some(base_dir(app)?.join(safe))
}

pub(crate) fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Write JSON through a temporary file, so a crash leaves the old file or the
/// new one.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/* ── the account ───────────────────────────────────────────────────── */

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub username: Option<String>,
    /// kryo.to's `cloud_saves` feature flag is on for this account.
    pub feature: bool,
    /// kryo.to's last answer on the supporter perk (None: not asked yet).
    pub perk: Option<bool>,
    pub perk_checked: u64,
    pub allowance: Option<api::Allowance>,
    pub usage: Option<api::Usage>,
    pub last_sync: Option<u64>,
}

static ACCOUNT: Mutex<Option<Account>> = Mutex::new(None);

fn account_file<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    Some(base_dir(app)?.join("account.json"))
}

pub(crate) fn account<R: Runtime>(app: &AppHandle<R>) -> Account {
    let mut slot = ACCOUNT.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(account_file(app).map(|f| read_json(&f)).unwrap_or_default());
    }
    slot.clone().unwrap_or_default()
}

pub(crate) fn update_account<R: Runtime>(app: &AppHandle<R>, f: impl FnOnce(&mut Account)) {
    let mut a = account(app);
    let before = a.clone();
    f(&mut a);
    if a == before {
        return;
    }
    if let Ok(mut slot) = ACCOUNT.lock() {
        *slot = Some(a.clone());
    }
    if let Some(file) = account_file(app) {
        let _ = write_json(&file, &a);
    }
    let _ = app.emit("cloud-saves", status_of(app, &a));
}

/// kryo.to said who is signed in and whether cloud saves are on for them.
/// Another account signing in starts from scratch.
pub fn set_account<R: Runtime>(app: &AppHandle<R>, username: Option<&str>, feature: bool) {
    update_account(app, |a| {
        if a.username.as_deref() != username {
            *a = Account { username: username.map(str::to_string), ..Default::default() };
        }
        a.feature = feature && username.is_some();
    });
}

/// Cloud saves can run: the account has the feature and Settings has it on.
pub(crate) fn enabled<R: Runtime>(app: &AppHandle<R>) -> bool {
    let a = account(app);
    a.feature && a.username.is_some() && crate::settings::load(app).cloud_saves
}

/// Save locations are being looked for while games run.
pub(crate) fn detecting<R: Runtime>(app: &AppHandle<R>) -> bool {
    enabled(app) && crate::settings::load(app).cloud_saves_detect && account(app).perk != Some(false)
}

/// A precise session is how many at most, per game, until one finds its saves.
const PRECISE_TRIES: u32 = 3;

/// Whether this launch should find the game's saves precisely: Settings says
/// so (or the player asked for it on the game's page), and it has not found
/// them that way yet, nor been told no.
fn wants_precise(mode: &str, r: &GameRules) -> bool {
    r.precise_next || (mode == "precise" && !r.precise_declined && !r.precise_found && r.precise_sessions < PRECISE_TRIES)
}

/// Before a game starts: when this session should find its saves precisely,
/// ask Windows for administrator and start the helper (precise.rs). Declined,
/// or anything going wrong, leaves the smart tracer to it.
pub fn precise_before_launch<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Option<precise::Precise> {
    if !cfg!(windows) || !detecting(app) {
        return None;
    }
    let mut stored = load_rules(app, &game.id);
    if !wants_precise(&crate::settings::load(app).cloud_saves_find, &stored) {
        return None;
    }
    let dir = game_data(app, &game.id)?.join("trace");
    match precise::begin(&dir, Path::new(&game.install_dir)) {
        precise::Begin::Running(p) => Some(p),
        precise::Begin::Declined => {
            crate::logging::info("saves", &format!("{}: administrator declined, smart detection instead", game.title));
            stored.precise_declined = true;
            stored.precise_next = false;
            save_rules(app, &game.id, &stored);
            None
        }
        precise::Begin::Failed(e) => {
            crate::logging::warn("saves", &format!("{}: precise finding did not start: {e}", game.title));
            None
        }
    }
}

/// The Steam id the emulator uses for this account on every PC, so a game
/// that ties its saves to the player's id finds them on the next PC too.
pub(crate) fn steam_id_for(username: &str) -> String {
    use sha2::Digest;
    let h = sha2::Sha256::digest(username.to_lowercase().as_bytes());
    let n = u32::from_le_bytes([h[0], h[1], h[2], h[3]]) % 0x7fff_ffff + 1;
    (76_561_197_960_265_728u64 + n as u64).to_string()
}

/* ── a game's identity and folders ─────────────────────────────────── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    /// What the account knows it by: `steam:<appid>`, `slug:<slug>` or
    /// `custom:<hash>`.
    pub key: String,
    pub aliases: Vec<String>,
    pub appid: Option<String>,
}

fn exe_stem(game: &LibraryGame) -> String {
    Path::new(&game.executable.replace('\\', "/")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

pub(crate) fn identity(game: &LibraryGame) -> Identity {
    let dir = Path::new(&game.install_dir);
    let appid = crate::achievements::appid_of(dir).or_else(|| kryoto_saves::emulator::online_appid(dir));
    let custom = {
        use sha2::Digest;
        let seed = format!("{}|{}", kryoto_saves::rules::squash(&game.title), exe_stem(game).to_lowercase());
        format!("custom:{}", kryoto_saves::scan::hex(&sha2::Sha256::digest(seed.as_bytes())))
    };
    let mut aliases = Vec::new();
    if let Some(a) = &appid {
        aliases.push(format!("steam:{a}"));
    }
    if let Some(s) = &game.slug {
        aliases.push(format!("slug:{s}"));
    }
    aliases.push(custom.clone());
    // A game on kryo.to goes by its page (kryo.to checks it is one), with its
    // AppID as an alias; one that is not on kryo.to by its name.
    let key = match &game.slug {
        Some(s) => format!("slug:{s}"),
        None => custom,
    };
    Identity { key, aliases, appid }
}

pub(crate) fn hints(game: &LibraryGame, appid: Option<String>) -> GameHints {
    GameHints { title: game.title.clone(), exe_stem: exe_stem(game), slug: game.slug.clone(), appid, developer: game.developer.clone() }
}

/// The folders behind each root for this game: this PC's own, or the Wine or
/// Proton prefix it runs in.
pub(crate) fn roots<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> SaveRoots {
    let game_dir = Some(PathBuf::from(&game.install_dir));
    #[cfg(windows)]
    let mut r = {
        let _ = app;
        win::roots(game_dir)
    };
    #[cfg(not(windows))]
    let mut r = {
        let prefix = crate::library::plan_for(app, game, None).ok().and_then(|(plan, _)| {
            plan.env.iter().find_map(|(k, v)| match k.as_str() {
                "WINEPREFIX" => Some(PathBuf::from(v)),
                "STEAM_COMPAT_DATA_PATH" => Some(PathBuf::from(v).join("pfx")),
                _ => None,
            })
        });
        match prefix {
            Some(p) => SaveRoots::for_prefix(&p, game_dir),
            None => SaveRoots { game_dir, ..Default::default() },
        }
    };
    r.steam_id = account(app).username.as_deref().map(steam_id_for);
    r
}

/* ── a game's rules ─────────────────────────────────────────────────── */

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    pub rule: Rule,
    pub score: i32,
    /// Sessions it was seen in.
    pub sessions: u32,
    pub files: usize,
    pub last_seen: u64,
    #[serde(default)]
    pub reasons: Vec<String>,
    /// The player kept it.
    #[serde(default)]
    pub accepted: bool,
    /// Sent to kryo.to's community rules.
    #[serde(default)]
    pub reported: bool,
    /// Found precisely: Windows saw the game write it.
    #[serde(default)]
    pub traced: bool,
}

impl Detected {
    pub fn trusted(&self) -> bool {
        self.accepted || self.score >= kryoto_saves::trace::CORROBORATED
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameRules {
    /// Folders the player added.
    pub player: Vec<Rule>,
    /// What this PC saw the game write.
    pub detected: Vec<Detected>,
    /// Locations the player took off (by [`Rule::key`]).
    pub ignored: Vec<String>,
    /// Locations a restored backup came with (another PC's).
    pub carried: Vec<Rule>,
    /// Kryoto's community rules, and when they were fetched.
    pub community: Vec<Rule>,
    pub community_at: u64,
    /// Folders the player left out of a location, by the location's
    /// [`Rule::key`] (relative patterns, as in [`Rule::excludes`]).
    pub excludes: BTreeMap<String, Vec<String>>,
    /// The [`DETECTOR`] that found `detected`.
    pub detector: u32,
    /// Precise sessions run (precise.rs), and whether one found saves.
    pub precise_sessions: u32,
    pub precise_found: bool,
    /// The player said no to administrator for this game: smart only.
    pub precise_declined: bool,
    /// The player asked for the next session to be precise.
    pub precise_next: bool,
}

/// Raised when the tracer's rules change enough that what an older one found
/// cannot be trusted. 2: a location needs evidence of the game itself (its
/// open files, its name), never only timing, so a compile or a git checkout
/// beside the game is no longer a save.
pub(crate) const DETECTOR: u32 = 2;

pub(crate) fn rules_file<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<PathBuf> {
    Some(game_data(app, id)?.join("rules.json"))
}

pub(crate) fn load_rules<R: Runtime>(app: &AppHandle<R>, id: &str) -> GameRules {
    let Some(file) = rules_file(app, id) else { return GameRules::default() };
    let mut rules: GameRules = read_json(&file);
    if rules.detector < DETECTOR {
        // An older detector's finds are forgotten, unless the player kept
        // them; the next session finds the real ones again.
        let before = rules.detected.len();
        rules.detected.retain(|d| d.accepted);
        rules.detector = DETECTOR;
        if rules.detected.len() != before {
            let _ = write_json(&file, &rules);
        }
    }
    rules
}

pub(crate) fn save_rules<R: Runtime>(app: &AppHandle<R>, id: &str, rules: &GameRules) {
    if let Some(f) = rules_file(app, id) {
        let _ = write_json(&f, rules);
    }
}

/// Every save location for a game, most trusted first, with what each one
/// is; and the detected locations still waiting for the player.
pub(crate) fn locations<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, roots: &SaveRoots) -> (Vec<Rule>, Vec<Detected>) {
    let (mut all, stored) = every_location(app, game, roots);
    all.retain(|r| !stored.ignored.contains(&r.key()));
    // Nothing is suggested in or around a location already backed up: inside
    // it is backed up already, and around it is more than the game's saves.
    let suggestions = stored
        .detected
        .iter()
        .filter(|d| !d.trusted() && !stored.ignored.contains(&d.rule.key()) && !all.iter().any(|r| r.contains(&d.rule) || d.rule.contains(r)))
        .cloned()
        .collect();
    (all, suggestions)
}

/// Every location from every source, the ones the player took off included,
/// with the player's left-out folders applied; and the game's stored rules.
fn every_location<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, roots: &SaveRoots) -> (Vec<Rule>, GameRules) {
    let id = identity(game);
    let stored = load_rules(app, &game.id);
    let dir = Path::new(&game.install_dir);
    let emulator = kryoto_saves::emulator::rules(dir, id.appid.as_deref());
    let engine = kryoto_saves::engine::rules(dir, roots, &hints(game, id.appid.clone()));
    let known = known::rules_for(app, id.appid.as_deref(), &game.title);
    let detected: Vec<Rule> = stored.detected.iter().filter(|d| d.trusted()).map(|d| d.rule.clone()).collect();
    let mut all = merge(&[&stored.player, &emulator, &engine, &known, &stored.carried, &stored.community, &detected]);
    for r in all.iter_mut() {
        if let Some(extra) = stored.excludes.get(&r.key()) {
            r.excludes.extend(extra.iter().cloned());
        }
    }
    (all, stored)
}

/* ── per-game sync state ────────────────────────────────────────────── */

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct GameState {
    /// The account this state is for: another account starts afresh.
    pub account: Option<String>,
    pub base_version: Option<String>,
    pub base_hash: Option<String>,
    pub last_backup_at: Option<u64>,
    pub last_restore_at: Option<u64>,
    /// The account's newest backup, as last seen.
    pub cloud_head: Option<api::Version>,
    /// Both this PC and the account changed since they last agreed.
    pub conflict: bool,
    /// The last thing that went wrong, for the game page.
    pub error: Option<String>,
    pub too_big: bool,
}

pub(crate) fn state_file<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<PathBuf> {
    Some(game_data(app, id)?.join("state.json"))
}

pub(crate) fn load_state<R: Runtime>(app: &AppHandle<R>, id: &str) -> GameState {
    let s: GameState = state_file(app, id).map(|f| read_json(&f)).unwrap_or_default();
    let who = account(app).username;
    if s.account != who {
        return GameState { account: who, ..Default::default() };
    }
    s
}

pub(crate) fn save_state<R: Runtime>(app: &AppHandle<R>, id: &str, s: &GameState) {
    if let Some(f) = state_file(app, id) {
        let _ = write_json(&f, s);
    }
    let _ = app.emit("cloud-saves-game", serde_json::json!({ "gameId": id }));
}

/* ── status for the shell ───────────────────────────────────────────── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// kryo.to has cloud saves on for this account.
    pub feature: bool,
    /// Settings has them on.
    pub on: bool,
    pub perk: Option<bool>,
    pub allowance: Option<api::Allowance>,
    pub usage: Option<api::Usage>,
    pub last_sync: Option<u64>,
    /// Games with a conflict waiting.
    pub conflicts: Vec<String>,
}

pub(crate) fn status_of<R: Runtime>(app: &AppHandle<R>, a: &Account) -> Status {
    let conflicts = crate::library::load(app)
        .unwrap_or_default()
        .into_iter()
        .filter(|g| load_state(app, &g.id).conflict)
        .map(|g| g.id)
        .collect();
    Status {
        feature: a.feature,
        on: crate::settings::load(app).cloud_saves,
        perk: a.perk,
        allowance: a.allowance.clone(),
        usage: a.usage.clone(),
        last_sync: a.last_sync,
        conflicts,
    }
}

#[tauri::command(async)]
pub fn cloud_saves_status(app: AppHandle) -> Status {
    status_of(&app, &account(&app))
}

/// Ask kryo.to again for the perk, the allowance and what is stored.
#[tauri::command]
pub async fn cloud_saves_refresh(app: AppHandle) -> Result<Status, String> {
    if account(&app).feature {
        sync::refresh_account(&app).await.map_err(|e| e.to_string())?;
    }
    Ok(status_of(&app, &account(&app)))
}

/* ── one game, for its page ─────────────────────────────────────────── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub key: String,
    pub kind: Kind,
    pub root: String,
    /// How a player reads it: `Documents\My Games\Skyrim`.
    pub label: String,
    pub source: Source,
    pub source_label: &'static str,
    /// Where it is on this PC (None for the registry, or not there).
    pub path: Option<String>,
    pub exists: bool,
    pub files: usize,
    pub bytes: u64,
    /// Registry values under a registry location.
    pub values: usize,
    pub score: Option<i32>,
    pub reasons: Vec<String>,
    /// Folders the player left out of it.
    pub left_out: Vec<String>,
}

/// A big folder inside a location, offered to leave out when the saves are
/// too big to back up (downloaded maps, mods, replays).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Heavy {
    pub location: String,
    /// The folder, relative to the location (what leaving it out adds).
    pub folder: String,
    pub label: String,
    pub files: usize,
    pub bytes: u64,
}

/// How this game's saves are being found, for its page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Settings: `precise` or `smart`.
    pub mode: String,
    /// Precise finding is possible on this PC (Windows).
    pub can_precise: bool,
    /// The next session asks for administrator to find them precisely.
    pub precise_next: bool,
    pub precise_found: bool,
    pub precise_declined: bool,
    pub precise_sessions: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSaves {
    pub identity: Identity,
    pub feature: bool,
    pub on: bool,
    pub perk: Option<bool>,
    pub finding: Finding,
    pub locations: Vec<Location>,
    pub suggestions: Vec<Location>,
    /// Locations the player took off, to put back.
    pub ignored: Vec<Location>,
    /// The biggest folders inside the locations, largest first.
    pub heavy: Vec<Heavy>,
    pub files: usize,
    pub bytes: u64,
    /// Registry values among the saves.
    pub registry_values: usize,
    pub truncated: bool,
    pub state: GameState,
    pub max_backup_bytes: u64,
    pub local_backups: Vec<LocalBackup>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackup {
    pub file: String,
    pub created_at: u64,
    pub size: u64,
}

pub(crate) fn label_of(rule: &Rule) -> String {
    let path = rule.path.replace('/', "\\");
    match rule.kind {
        Kind::Registry => format!("HKEY_CURRENT_USER\\{path}"),
        Kind::Files => format!("{}\\{path}", rule.root.label()),
    }
}

fn registry_values(rule: &Rule) -> usize {
    #[cfg(windows)]
    {
        if rule.kind == Kind::Registry {
            return kryoto_saves::registry::read(std::slice::from_ref(rule)).len();
        }
    }
    let _ = rule;
    0
}

fn location(rule: &Rule, roots: &SaveRoots, per_rule: &HashMap<String, (usize, u64)>, detected: Option<&Detected>, left_out: &[String]) -> Location {
    let base = (rule.kind == Kind::Files).then(|| rule.base(roots)).flatten();
    let found = kryoto_saves::rules::expand(rule, roots);
    let (files, bytes) = per_rule.get(&rule.key()).copied().unwrap_or_default();
    let values = registry_values(rule);
    Location {
        key: rule.key(),
        kind: rule.kind,
        root: rule.root.name().into(),
        label: label_of(rule),
        source: rule.source,
        // Windows saw the game write it: more than "seen while it ran".
        source_label: if detected.is_some_and(|d| d.traced) { "Precise" } else { rule.source.label() },
        path: found.first().cloned().or(base).map(|p| p.to_string_lossy().to_string()),
        exists: !found.is_empty() || values > 0,
        files,
        bytes,
        values,
        score: detected.map(|d| d.score),
        reasons: detected.map(|d| d.reasons.clone()).unwrap_or_default(),
        left_out: left_out.to_vec(),
    }
}

/// The biggest top-level folders inside each location, from what a scan found.
fn heavy_folders(rules: &[Rule], found: &kryoto_saves::scan::Collected) -> Vec<Heavy> {
    let mut sizes: BTreeMap<(String, String), (usize, u64, String)> = BTreeMap::new();
    for f in &found.files {
        let Some(rule) = rules.iter().find(|r| r.key() == f.rule) else { continue };
        if rule.path.contains('*') || rule.path.contains('<') {
            continue;
        }
        // Compared without changing the bytes: a lowercased name (Cyrillic,
        // say) need not keep its length.
        let prefix = format!("{}/", rule.path);
        let Some(rel) = f.path.get(..prefix.len()).filter(|p| p.eq_ignore_ascii_case(&prefix)).map(|_| &f.path[prefix.len()..]) else { continue };
        let Some((folder, _)) = rel.split_once('/') else { continue };
        let e = sizes.entry((rule.key(), folder.to_lowercase())).or_insert((0, 0, folder.to_string()));
        e.0 += 1;
        e.1 += f.size;
    }
    let mut out: Vec<Heavy> = sizes
        .into_iter()
        .filter(|(_, (_, bytes, _))| *bytes >= 1 << 20)
        .filter_map(|((key, _), (files, bytes, folder))| {
            let rule = rules.iter().find(|r| r.key() == key)?;
            Some(Heavy { label: format!("{}\\{}", label_of(rule), folder), location: key, folder, files, bytes })
        })
        .collect();
    out.sort_by_key(|h| std::cmp::Reverse(h.bytes));
    out.truncate(6);
    out
}

pub(crate) fn local_backups<R: Runtime>(app: &AppHandle<R>, id: &str) -> Vec<LocalBackup> {
    let Some(dir) = game_data(app, id).map(|d| d.join("backups")) else { return Vec::new() };
    let mut out: Vec<LocalBackup> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "ksave"))
                .filter_map(|e| {
                    let m = e.metadata().ok()?;
                    let created_at = e.file_name().to_string_lossy().split('.').next()?.parse().ok()?;
                    Some(LocalBackup { file: e.file_name().to_string_lossy().to_string(), created_at, size: m.len() })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    out
}

fn game_by_id<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<LibraryGame, String> {
    crate::library::load(app)?.into_iter().find(|g| g.id == id).ok_or_else(|| "That game is no longer in the library.".to_string())
}

#[tauri::command(async)]
pub fn cloud_saves_game(app: AppHandle, id: String) -> Result<GameSaves, String> {
    let game = game_by_id(&app, &id)?;
    let roots = roots(&app, &game);
    let (rules, suggestions) = locations(&app, &game, &roots);
    let found = kryoto_saves::scan::collect(&rules, &roots, kryoto_saves::scan::MAX_FILES);
    let per_rule = found.per_rule();
    let (everything, stored) = every_location(&app, &game, &roots);
    let detected = |r: &Rule| stored.detected.iter().find(|d| d.rule.key() == r.key());
    let left_out = |r: &Rule| stored.excludes.get(&r.key()).cloned().unwrap_or_default();
    let mut ignored: Vec<Location> = everything
        .iter()
        .filter(|r| stored.ignored.contains(&r.key()))
        .map(|r| location(r, &roots, &HashMap::new(), detected(r), &left_out(r)))
        .collect();
    // Suggestions the player dismissed are taken off too.
    for d in stored.detected.iter().filter(|d| stored.ignored.contains(&d.rule.key())) {
        if !ignored.iter().any(|l| l.key == d.rule.key()) {
            ignored.push(location(&d.rule, &roots, &HashMap::new(), Some(d), &[]));
        }
    }
    let locations: Vec<Location> = rules.iter().map(|r| location(r, &roots, &per_rule, detected(r), &left_out(r))).collect();
    let registry_values = locations.iter().map(|l| l.values).sum();
    // What each suggestion holds now, so its size can be judged before Add;
    // one this PC no longer has is not offered.
    let suggested: Vec<Rule> = suggestions.iter().map(|d| d.rule.clone()).collect();
    let in_suggested = kryoto_saves::scan::collect(&suggested, &roots, kryoto_saves::scan::MAX_FILES).per_rule();
    let suggestions: Vec<Location> = suggestions.iter().map(|d| location(&d.rule, &roots, &in_suggested, Some(d), &[])).filter(|l| l.exists).collect();
    let a = account(&app);
    let settings = crate::settings::load(&app);
    let finding = Finding {
        precise_next: wants_precise(&settings.cloud_saves_find, &stored) && cfg!(windows) && settings.cloud_saves_detect,
        mode: settings.cloud_saves_find.clone(),
        can_precise: cfg!(windows),
        precise_found: stored.precise_found,
        precise_declined: stored.precise_declined,
        precise_sessions: stored.precise_sessions,
    };
    Ok(GameSaves {
        identity: identity(&game),
        feature: a.feature,
        on: settings.cloud_saves,
        perk: a.perk,
        finding,
        suggestions,
        heavy: heavy_folders(&rules, &found),
        locations,
        ignored,
        files: found.files.len(),
        bytes: found.bytes,
        registry_values,
        truncated: found.truncated,
        state: load_state(&app, &id),
        max_backup_bytes: a.allowance.as_ref().map(|x| x.max_backup_bytes).filter(|b| *b > 0).unwrap_or(sync::DEFAULT_MAX_BACKUP),
        local_backups: local_backups(&app, &id),
    })
}

/// Keep a suggested location, or take one off ("ignore"), or put back one the
/// player took off ("restore").
#[tauri::command(async)]
pub fn cloud_saves_location(app: AppHandle, id: String, key: String, action: String) -> Result<(), String> {
    let game = game_by_id(&app, &id)?;
    let mut stored = load_rules(&app, &id);
    match action.as_str() {
        "keep" => {
            let d = stored.detected.iter_mut().find(|d| d.rule.key() == key).ok_or("That location is no longer suggested.")?;
            d.accepted = true;
            stored.ignored.retain(|k| *k != key);
        }
        // Taken off, not forgotten: a folder the player added stays listed
        // under the taken-off locations, so it can be put back.
        "ignore" => {
            if !stored.ignored.contains(&key) {
                stored.ignored.push(key);
            }
        }
        "restore" => stored.ignored.retain(|k| *k != key),
        _ => return Err("Unknown action.".into()),
    }
    save_rules(&app, &id, &stored);
    let _ = app.emit("cloud-saves-game", serde_json::json!({ "gameId": game.id }));
    Ok(())
}

/// Leave a folder inside a location out of the backup (`on`), or take it back
/// in. `folder` is relative to the location.
#[tauri::command(async)]
pub fn cloud_saves_leave_out(app: AppHandle, id: String, key: String, folder: String, on: bool) -> Result<(), String> {
    let game = game_by_id(&app, &id)?;
    let folder = kryoto_saves::rules::normalize(&folder);
    if kryoto_saves::rules::path_problem(&folder).is_some() {
        return Err("That folder cannot be left out.".into());
    }
    let mut stored = load_rules(&app, &id);
    let list = stored.excludes.entry(key.clone()).or_default();
    list.retain(|f| !f.eq_ignore_ascii_case(&folder));
    if on {
        list.push(folder);
    }
    if list.is_empty() {
        stored.excludes.remove(&key);
    }
    save_rules(&app, &id, &stored);
    // Too big was about the old size; the next backup says again if it still is.
    let mut s = load_state(&app, &id);
    if s.too_big {
        s.too_big = false;
        s.error = None;
        save_state(&app, &id, &s);
    }
    let _ = app.emit("cloud-saves-game", serde_json::json!({ "gameId": game.id }));
    Ok(())
}

/// Find this game's saves precisely the next time it is played (one
/// administrator prompt then), or not.
#[tauri::command(async)]
pub fn cloud_saves_find_precisely(app: AppHandle, id: String, on: bool) -> Result<(), String> {
    let game = game_by_id(&app, &id)?;
    let mut stored = load_rules(&app, &id);
    stored.precise_next = on;
    if on {
        stored.precise_declined = false;
    } else if crate::settings::load(&app).cloud_saves_find == "precise" && !stored.precise_found {
        // "Not now" in precise mode means this game stays smart.
        stored.precise_declined = true;
    }
    save_rules(&app, &id, &stored);
    let _ = app.emit("cloud-saves-game", serde_json::json!({ "gameId": game.id }));
    Ok(())
}

/// Add a folder (or file) the player picked as a save location.
#[tauri::command(async)]
pub fn cloud_saves_add_folder(app: AppHandle, id: String, path: String) -> Result<(), String> {
    let game = game_by_id(&app, &id)?;
    let roots = roots(&app, &game);
    let (root, rel) = roots
        .placeholder_of(Path::new(&path))
        .ok_or("Pick a folder in the game's folder, your user folder (Documents, AppData, Saved Games), Public Documents or ProgramData.")?;
    let rule = Rule::files(root, rel, Source::Player);
    if let Some(p) = rule.problem() {
        return Err(p.into());
    }
    let mut stored = load_rules(&app, &id);
    stored.ignored.retain(|k| *k != rule.key());
    if !stored.player.iter().any(|r| r.key() == rule.key()) {
        stored.player.push(rule);
    }
    save_rules(&app, &id, &stored);
    let _ = app.emit("cloud-saves-game", serde_json::json!({ "gameId": game.id }));
    Ok(())
}

/// Back the game's saves up now. `over` settles a conflict for this PC: its
/// saves go on top of the account's newest instead of being refused.
#[tauri::command]
pub async fn cloud_saves_backup(app: AppHandle, id: String, over: Option<bool>) -> Result<String, String> {
    if over == Some(true) {
        let mut s = load_state(&app, &id);
        if let Some(head) = s.cloud_head.clone() {
            s.base_version = Some(head.id);
            s.conflict = false;
            save_state(&app, &id, &s);
        }
    }
    sync::backup(&app, &id, true).await
}

/// The account's backups of this game, newest first.
#[tauri::command]
pub async fn cloud_saves_versions(app: AppHandle, id: String) -> Result<Vec<api::Version>, String> {
    let game = game_by_id(&app, &id)?;
    let ident = identity(&game);
    let api = api::Api::new(&app).map_err(|e| e.to_string())?;
    let list = api.list().await.map_err(|e| e.to_string())?;
    sync::remember_list(&app, &list);
    let mut versions: Vec<api::Version> = list
        .games
        .into_iter()
        .filter(|g| g.game_key == ident.key || ident.aliases.contains(&g.game_key) || g.aliases.iter().any(|a| ident.aliases.contains(a)))
        .flat_map(|g| g.versions)
        .collect();
    versions.sort_by_key(|v| std::cmp::Reverse(v.created_at));
    Ok(versions)
}

/// Put one of the account's backups on this PC.
#[tauri::command]
pub async fn cloud_saves_restore(app: AppHandle, id: String, version_id: String) -> Result<String, String> {
    sync::restore_version(&app, &id, &version_id).await
}

/// Put back what a restore replaced on this PC.
#[tauri::command]
pub async fn cloud_saves_restore_local(app: AppHandle, id: String, file: String) -> Result<String, String> {
    sync::restore_local(&app, &id, &file).await
}

/// The player's answer to a conflict or a "restore these?" question.
#[tauri::command]
pub fn cloud_saves_answer(id: String, choice: String) -> Result<(), String> {
    sync::answer(&id, &choice)
}

/// Saves of every game in the library, for Settings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub id: String,
    pub title: String,
    pub last_backup_at: Option<u64>,
    pub conflict: bool,
    pub error: Option<String>,
    pub too_big: bool,
}

#[tauri::command(async)]
pub fn cloud_saves_library(app: AppHandle) -> Vec<LibraryEntry> {
    let mut out: Vec<LibraryEntry> = crate::library::load(&app)
        .unwrap_or_default()
        .into_iter()
        .map(|g| {
            let s = load_state(&app, &g.id);
            LibraryEntry { id: g.id, title: g.title, last_backup_at: s.last_backup_at, conflict: s.conflict, error: s.error, too_big: s.too_big }
        })
        .collect();
    out.sort_by(|a, b| b.conflict.cmp(&a.conflict).then(b.last_backup_at.cmp(&a.last_backup_at)).then(a.title.cmp(&b.title)));
    out
}

/// Prune local backups beyond `keep`, oldest first.
pub(crate) fn prune_backups<R: Runtime>(app: &AppHandle<R>, id: &str, keep: usize) {
    let Some(dir) = game_data(app, id).map(|d| d.join("backups")) else { return };
    for b in local_backups(app, id).into_iter().skip(keep.max(1)) {
        let _ = std::fs::remove_file(dir.join(b.file));
    }
}

/// Folders a tracer must never take for a game's: Kryoto's own, and every
/// other game in the library.
pub(crate) fn ignore_dirs<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in [app.path().app_data_dir(), app.path().app_local_data_dir(), app.path().app_cache_dir()].into_iter().flatten() {
        out.push(p);
    }
    for g in crate::library::load(app).unwrap_or_default() {
        if g.id != game.id && !g.install_dir.is_empty() && !Path::new(&game.install_dir).starts_with(&g.install_dir) {
            out.push(PathBuf::from(g.install_dir));
        }
    }
    out
}

/// Which games each detected location was seen for (the noise log): a
/// location more than one game writes to belongs to none of them.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Noise(pub BTreeMap<String, Vec<String>>);

pub(crate) fn noise_file<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    Some(base_dir(app)?.join("noise.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steam_id_is_stable_and_valid() {
        let a = steam_id_for("Mira");
        assert_eq!(a, steam_id_for("mira"));
        assert_ne!(a, steam_id_for("neo"));
        let n: u64 = a.parse().unwrap();
        // An individual account in the public universe: 0x0110000100000000 + a 32-bit id.
        assert!(n > 76_561_197_960_265_728 && n < 76_561_197_960_265_728 + (1u64 << 32));
    }

    #[test]
    fn identity_is_the_kryoto_page_with_the_appid_as_an_alias() {
        let dir = std::env::temp_dir().join(format!("kryo-saves-ident-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_appid.txt"), "620").unwrap();
        let mut g = LibraryGame { title: "Portal 2".into(), slug: Some("portal-2".into()), install_dir: dir.to_string_lossy().into(), executable: "portal2.exe".into(), ..Default::default() };
        let i = identity(&g);
        assert_eq!(i.key, "slug:portal-2");
        assert!(i.aliases.contains(&"steam:620".to_string()) && i.aliases.iter().any(|a| a.starts_with("custom:")));
        g.slug = None;
        assert!(identity(&g).key.starts_with("custom:"), "a game not on kryo.to goes by its name");
        std::fs::remove_file(dir.join("steam_appid.txt")).unwrap();
        g.slug = Some("portal-2".into());
        assert_eq!(identity(&g).key, "slug:portal-2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What real installed games would back up. Opt-in, read-only:
    /// `KRYOTO_SAVES_GAMES="<game dir>|<appid>;..." cargo test real_games_saves -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn real_games_saves() {
        let list = std::env::var("KRYOTO_SAVES_GAMES").expect("KRYOTO_SAVES_GAMES");
        for spec in list.split(';').filter(|s| !s.is_empty()) {
            let (dir, appid) = spec.split_once('|').unwrap_or((spec, ""));
            let dir = Path::new(dir);
            let roots = win::roots(Some(dir.to_path_buf()));
            let title = dir.file_name().unwrap().to_string_lossy().to_string();
            let hints = GameHints { title, ..Default::default() };
            let appid = (!appid.is_empty()).then_some(appid);
            let rules = merge(&[&kryoto_saves::emulator::rules(dir, appid), &kryoto_saves::engine::rules(dir, &roots, &hints)]);
            let found = kryoto_saves::scan::collect(&rules, &roots, kryoto_saves::scan::MAX_FILES);
            let values = kryoto_saves::registry::read(&rules).len();
            println!("== {} : {} files, {:.1} MB, {} registry values, {} left out", dir.display(), found.files.len(), found.bytes as f64 / 1048576.0, values, found.skipped);
            for (key, (n, b)) in found.per_rule() {
                println!("   {key}: {n} files, {:.1} MB", b as f64 / 1048576.0);
            }
            for f in found.files.iter().take(8) {
                println!("     {}/{}", f.root.name(), f.path);
            }
        }
    }

    #[test]
    fn labels_read_like_windows() {
        assert_eq!(label_of(&Rule::files(kryoto_saves::Root::Documents, "My Games/X", Source::Known)), "Documents\\My Games\\X");
        assert_eq!(label_of(&Rule::registry("Software/A/B", Source::Engine)), "HKEY_CURRENT_USER\\Software\\A\\B");
    }
}
