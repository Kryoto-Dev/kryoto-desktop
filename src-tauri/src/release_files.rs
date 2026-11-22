//! A release's own file list, and what it lets Kryoto say and do about an
//! installed game.
//!
//! Forge lists every file of a release as it packs it: path, size, SHA-256 and
//! what each file is (`role`): the game's own, added or replaced by the Steam
//! emulator, a fix (Steamless, online-fix), or Kryoto's own shortcut files.
//! kryo.to serves it at `GET /api/games/<slug>/releases/<id>/files` (kryo.to
//! `lib/release-files-format.ts`), only for releases made by the new Forge; any
//! other answers 404 `no_file_list`.
//!
//! With it:
//!
//! - **Check files** names exactly which file is missing or broken (an
//!   antivirus took `steam_api64.dll`), cheaply: existence and size for every
//!   file, a hash only for emulator files changed since the install, and every
//!   file hashed only when the player asks for it ([`check_tree`]).
//! - **Get the files back** without downloading the game again when the bytes
//!   are on this PC: another listed file with the same hash (the shipped
//!   `.dll.kryoto` backup of an original), the emulator builds Repair already
//!   keeps, or the download itself when it was kept (Settings > Storage)
//!   ([`game_files_restore`]).
//! - **A clean emulator swap**: the release's own emulator taken out by role
//!   (originals back from their shipped backup, verified by hash; what the
//!   emulator added set aside; fixes and the game left alone) before Repair puts
//!   the new one in, and put back exactly by "Back to the release's own files"
//!   ([`clean_for_switch`], [`unclean`]).
//!
//! Nothing here runs on the hot path. The list is fetched lazily (after an
//! install, on opening Properties, after a launch failure), kept in the app's
//! data folder by release id, and never fetched again once kept (a list never
//! changes); a release without one is asked about again after a week. Every
//! check runs off the UI thread and gives the disk room between reads (D: is a
//! hard disk on many PCs, see `achievements_cloud.rs`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kryoto_repair::community::{self as cu, Stamp};
use kryoto_repair::journal;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::community_offer::{read_json, ReleaseRecord, RELEASE_RECORD};
use crate::library::LibraryGame;

/// A release with no list (or a game whose release could not be told) is
/// asked about again after this long.
const NO_LIST_RETRY_SECS: u64 = 7 * 24 * 60 * 60;
/// kryo.to did not answer (offline, 5xx, 429): not before this long.
const FAILED_RETRY_SECS: u64 = 10 * 60;
/// The biggest list read (kryo.to allows 300,000 files; ~60 MB of JSON).
const MAX_BODY: usize = 96 * 1024 * 1024;
/// Changed files named in a report; the rest are only counted.
const CHANGED_SAMPLE: usize = 40;
/// Where Kryoto sets the release's own emulator files aside during a swap.
pub const SWAP_RECORD: &str = ".kryoto-repair/release-emulator.json";
const SWAP_DIR: &str = ".kryoto-repair/release-emulator";

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What a listed file is (kryo.to `ROLES`). An unknown one reads as the game's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    #[default]
    Game,
    EmulatorAdded,
    EmulatorReplaced,
    Fix,
    Kryoto,
    #[serde(other)]
    Other,
}

impl Role {
    fn emulator(self) -> bool {
        matches!(self, Role::EmulatorAdded | Role::EmulatorReplaced)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListedFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Emu {
    pub name: String,
    pub label: String,
    pub variant: Option<String>,
    pub version: Option<String>,
    pub dll_sha256: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Removed {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// kryo.to's answer, as stored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileList {
    pub schema: u32,
    pub release_id: String,
    pub slug: String,
    pub applied_by_forge: bool,
    pub emulator_off: bool,
    /// Steam files the uploader kept as the game shipped them.
    pub excluded: Vec<String>,
    pub emulator: Option<Emu>,
    pub files: Option<Vec<ListedFile>>,
    pub removed: Vec<Removed>,
}

fn sha_ok(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl FileList {
    /// Only what is safe to act on: relative paths inside the game, real hashes.
    pub fn cleaned(mut self) -> Self {
        if let Some(files) = self.files.as_mut() {
            files.retain(|f| cu::safe_path(&f.path) && sha_ok(&f.sha256));
            for f in files.iter_mut() {
                f.sha256 = f.sha256.to_ascii_lowercase();
                f.original_sha256 = f.original_sha256.take().filter(|s| sha_ok(s)).map(|s| s.to_ascii_lowercase());
                f.backup = f.backup.take().filter(|b| cu::safe_path(b));
            }
        }
        self.excluded.retain(|p| cu::safe_path(p));
        self
    }

    pub fn listed(&self) -> &[ListedFile] {
        self.files.as_deref().unwrap_or_default()
    }

    /// "gbe_fork (experimental)": the release's emulator in a player's words.
    pub fn emulator_words(&self) -> Option<String> {
        let e = self.emulator.as_ref()?;
        let label = if e.label.trim().is_empty() { e.name.trim() } else { e.label.trim() };
        if label.is_empty() {
            return None;
        }
        match e.variant.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            Some(v) if !label.to_ascii_lowercase().contains(&v.to_ascii_lowercase()) => Some(format!("{label} ({v})")),
            _ => Some(label.to_string()),
        }
    }
}

/// Paths are compared ignoring case: Windows sees `A.dll` and `a.dll` as one.
pub fn key(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// A program, library or script: what an antivirus takes, and what a game
/// cannot start without (antivirus.rs `WATCHED`, plus Linux's `.so`).
fn is_program(path: &str) -> bool {
    crate::antivirus::watched(path) || path.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("so"))
}

/* ── Which release is installed ─────────────────────────────────────────── */

/// The release of a `/api/games/<slug>/downloads` answer the installed build
/// is, and what kryo.to says about its list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Picked {
    pub id: String,
    /// kryo.to has a list for it: `Some(false)` when it says there is none,
    /// `None` from a kryo.to too old to say.
    pub has_list: Option<bool>,
    /// "12.3 GB", as kryo.to lists the download.
    pub download_size: Option<String>,
}

/// By the archive's SHA-256 when the install kept it (a re-upload of the same
/// version is another release), else by version, the main release first. A
/// known hash that matches nothing is never matched by version to a release
/// that lists other hashes: that would be another upload's list.
pub fn pick_release(json: &serde_json::Value, build_sha: Option<&str>, version: Option<&str>) -> Option<Picked> {
    let releases = json["releases"].as_array()?;
    let hashes = |r: &serde_json::Value| -> Vec<String> {
        r["hashes"].as_array().into_iter().flatten().filter_map(|h| h["sha256"].as_str()).map(str::to_ascii_lowercase).collect()
    };
    let sha = build_sha.filter(|s| sha_ok(s)).map(str::to_ascii_lowercase);
    let by_sha = sha.as_ref().and_then(|s| releases.iter().find(|r| hashes(r).contains(s)));
    let r = match by_sha {
        Some(r) => r,
        None => {
            let v = version.map(str::trim).filter(|v| !v.is_empty())?;
            let same: Vec<&serde_json::Value> = releases.iter().filter(|r| r["version"].as_str().map(str::trim) == Some(v)).collect();
            let r = same.iter().find(|r| r["primary"].as_bool() == Some(true)).or(same.first()).copied()?;
            if sha.is_some() && !hashes(r).is_empty() {
                return None;
            }
            r
        }
    };
    let id = match &r["id"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if id.is_empty() || id.len() > 19 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let has_list = match r.get("files") {
        None => None,
        Some(serde_json::Value::Null) => Some(false),
        Some(f) => f["has_list"].as_bool(),
    };
    let download_size = match &r["download_size"] {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    Some(Picked { id, has_list, download_size })
}

/// A release with no list, or a failed question, is due to be asked again.
pub fn due(asked_at: Option<u64>, wait: u64, at: u64) -> bool {
    asked_at.is_none_or(|t| at.saturating_sub(t) >= wait)
}

/* ── The cache ──────────────────────────────────────────────────────────── */

/// One release's list as kept on disk, by release id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Kept {
    pub etag: Option<String>,
    pub fetched_at: u64,
    /// kryo.to said it has no list for this release.
    pub none_at: Option<u64>,
    /// kryo.to could not be asked (offline, busy).
    pub failed_at: Option<u64>,
    pub list: Option<FileList>,
}

/// What this PC knows about one installed game's release.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GameEntry {
    /// The install this is about: a new one (another build) starts over.
    pub slug: String,
    pub version: Option<String>,
    pub build_sha: Option<String>,
    pub release_id: Option<String>,
    pub download_size: Option<String>,
    /// When the release was looked for (found or not), and when that failed.
    pub resolved_at: Option<u64>,
    pub failed_at: Option<u64>,
    pub report: Option<CheckReport>,
}

impl GameEntry {
    fn same_install(&self, game: &LibraryGame) -> bool {
        Some(self.slug.as_str()) == game.slug.as_deref() && self.version == game.version && self.build_sha == game.build_sha
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    games: HashMap<String, GameEntry>,
}

static STORE: Mutex<()> = Mutex::new(());

fn dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    Some(app.path().app_data_dir().ok()?.join("release-files"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// The kept list of a release, by id (digits only).
pub fn read_kept(dir: &Path, release_id: &str) -> Kept {
    if !release_id.bytes().all(|b| b.is_ascii_digit()) || release_id.is_empty() {
        return Kept::default();
    }
    read_json(&dir.join(format!("{release_id}.json"))).unwrap_or_default()
}

pub fn write_kept(dir: &Path, release_id: &str, kept: &Kept) -> Result<(), String> {
    if !release_id.bytes().all(|b| b.is_ascii_digit()) || release_id.is_empty() {
        return Err("not a release id".into());
    }
    write_atomic(&dir.join(format!("{release_id}.json")), &serde_json::to_vec(kept).map_err(|e| e.to_string())?)
}

fn with_entry<R: Runtime, T>(app: &AppHandle<R>, game: &LibraryGame, f: impl FnOnce(&mut GameEntry) -> T) -> Option<T> {
    let dir = dir(app)?;
    let _guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join("games.json");
    let mut store: Store = read_json(&path).unwrap_or_default();
    let entry = store.games.entry(game.id.clone()).or_default();
    if !entry.same_install(game) {
        *entry = GameEntry { slug: game.slug.clone().unwrap_or_default(), version: game.version.clone(), build_sha: game.build_sha.clone(), ..Default::default() };
    }
    let out = f(entry);
    let _ = write_atomic(&path, &serde_json::to_vec(&store).unwrap_or_default());
    Some(out)
}

/// What is kept for this install, read only (another build of it is nothing yet).
fn entry<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> GameEntry {
    let Some(dir) = dir(app) else { return GameEntry::default() };
    let _guard = STORE.lock().unwrap_or_else(|e| e.into_inner());
    read_json::<Store>(&dir.join("games.json"))
        .and_then(|mut s| s.games.remove(&game.id))
        .filter(|e| e.same_install(game))
        .unwrap_or_default()
}

/// The list kept for this game's release, without asking kryo.to.
pub fn cached_list<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Option<FileList> {
    let id = entry(app, game).release_id?;
    read_kept(&dir(app)?, &id).list
}

/// What kryo.to answered for a list.
#[derive(Debug, PartialEq)]
pub enum Answer {
    List(Box<FileList>, Option<String>),
    Unchanged,
    NoList,
    Failed,
}

/// The answer to `GET .../files`, from its status and body. Pure, for tests.
pub fn read_answer(status: u16, etag: Option<String>, body: &[u8]) -> Answer {
    match status {
        200 => match serde_json::from_slice::<FileList>(body) {
            Ok(list) if list.files.is_some() => Answer::List(Box::new(list.cleaned()), etag),
            Ok(_) => Answer::NoList,
            Err(_) => Answer::Failed,
        },
        304 => Answer::Unchanged,
        // `no_file_list`, or `not_found` for a release no longer public: none either way.
        404 | 410 => Answer::NoList,
        _ => Answer::Failed,
    }
}

fn client<R: Runtime>(app: &AppHandle<R>) -> Option<(reqwest::Client, String)> {
    let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(app));
    let client = crate::net::kryoto_tls(&endpoint, crate::net::builder().user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION"))))
        .timeout(Duration::from_secs(30))
        .build()
        .ok()?;
    Some((client, endpoint))
}

/// Which release this game is, asked of kryo.to when due.
async fn resolve<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Option<String> {
    let e = entry(app, game);
    if e.release_id.is_some() {
        return e.release_id;
    }
    let at = now();
    if !due(e.resolved_at, NO_LIST_RETRY_SECS, at) || !due(e.failed_at, FAILED_RETRY_SECS, at) {
        return None;
    }
    let slug = game.slug.as_deref().filter(|s| !s.is_empty() && s.len() <= 160 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))?;
    if crate::catalog_gone::is_gone(app, slug) {
        return None;
    }
    let (client, endpoint) = client(app)?;
    let answer = match client.get(format!("{endpoint}/api/games/{slug}/downloads")).send().await {
        Ok(res) => {
            let status = res.status().as_u16();
            if crate::catalog_gone::says_gone(status) {
                crate::catalog_gone::set(app, slug, true);
            }
            // The downloads perk withholds the links (401) but still lists the releases.
            res.json::<serde_json::Value>().await.ok().filter(|j| j["releases"].is_array())
        }
        Err(_) => None,
    };
    let Some(json) = answer else {
        with_entry(app, game, |e| e.failed_at = Some(at));
        return None;
    };
    let picked = pick_release(&json, game.build_sha.as_deref(), game.pinned_version.as_deref().or(game.version.as_deref()));
    with_entry(app, game, |e| {
        e.resolved_at = Some(at);
        e.failed_at = None;
        e.release_id = picked.as_ref().map(|p| p.id.clone());
        e.download_size = picked.as_ref().and_then(|p| p.download_size.clone());
    });
    let picked = picked?;
    // kryo.to already says there is none: no second question.
    if picked.has_list == Some(false) {
        if let Some(d) = dir(app) {
            let mut kept = read_kept(&d, &picked.id);
            if kept.list.is_none() {
                kept.none_at = Some(at);
                let _ = write_kept(&d, &picked.id, &kept);
            }
        }
    }
    Some(picked.id)
}

/// This game's list: kept, or fetched once when `network` and due. Never
/// fails: no list is the answer for every older release and when offline.
pub async fn ensure_list<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, network: bool) -> Option<FileList> {
    if game.slug.is_none() || game.launcher.is_some() {
        return None;
    }
    let id = if network { resolve(app, game).await? } else { entry(app, game).release_id? };
    let d = dir(app)?;
    let mut kept = read_kept(&d, &id);
    if kept.list.is_some() || !network {
        return kept.list;
    }
    let at = now();
    if !due(kept.none_at, NO_LIST_RETRY_SECS, at) || !due(kept.failed_at, FAILED_RETRY_SECS, at) {
        return None;
    }
    let slug = game.slug.clone()?;
    let (client, endpoint) = client(app)?;
    let mut req = client.get(format!("{endpoint}/api/games/{slug}/releases/{id}/files"));
    if let Some(tag) = &kept.etag {
        req = req.header("If-None-Match", tag);
    }
    let answer = match req.send().await {
        Ok(res) => {
            let status = res.status().as_u16();
            let etag = res.headers().get("etag").and_then(|v| v.to_str().ok()).map(String::from);
            let too_big = res.content_length().is_some_and(|n| n as usize > MAX_BODY);
            match res.bytes().await {
                Ok(body) if !too_big && body.len() <= MAX_BODY => read_answer(status, etag, &body),
                _ => Answer::Failed,
            }
        }
        Err(_) => Answer::Failed,
    };
    match answer {
        Answer::List(list, etag) => {
            kept = Kept { etag, fetched_at: at, none_at: None, failed_at: None, list: Some(*list) };
        }
        Answer::Unchanged => kept.fetched_at = at,
        Answer::NoList => {
            kept.none_at = Some(at);
            kept.failed_at = None;
        }
        Answer::Failed => kept.failed_at = Some(at),
    }
    let _ = write_kept(&d, &id, &kept);
    if kept.list.is_some() {
        crate::logging::info("files", &format!("{}: release {id} lists {} files", game.title, kept.list.as_ref().map(|l| l.listed().len()).unwrap_or(0)));
        // Properties' Emulator row says what the release came with now.
        let _ = app.emit("files-checked", &game.id);
    }
    kept.list
}

/* ── Checking an install ────────────────────────────────────────────────── */

/// What to do with one listed file, from what is on disk. Pure, for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// There, the right size, and nothing says it changed.
    Fine,
    Missing,
    /// There, another size: it is not the release's file.
    Differs,
    /// The right size; its hash says whether it is the same file.
    Hash,
    /// Changed since the install by its time, not worth a hash now.
    Unverified,
}

/// `now`: the file on disk; `installed`: its stamp right after the install
/// (`.kryoto-release.json`). A quick check hashes only what is `suspect`
/// (emulator files, small) and only when its time moved; a full check hashes
/// every file the right size.
pub fn decide(listed: u64, now: Option<Stamp>, installed: Option<Stamp>, full: bool, suspect: bool) -> Step {
    let Some(now) = now else { return Step::Missing };
    if now.size != listed {
        return Step::Differs;
    }
    if full {
        return Step::Hash;
    }
    match installed {
        Some(i) if i.size == now.size && i.modified_ms == now.modified_ms => Step::Fine,
        Some(_) if suspect => Step::Hash,
        Some(_) => Step::Unverified,
        None => Step::Fine,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Problem {
    /// Gone from the folder.
    Missing,
    /// An emulator program that is not the release's: the game will not start with it.
    Broken,
    /// Not the release's file, said quietly: a mod, a config the game wrote.
    Changed,
}

/// How much a file that is not the release's matters. Only missing files and
/// broken emulator programs raise the card; a game file the player's mods
/// changed never does. Kryoto's own files (its shortcut, its record) only
/// count when a program among them is gone.
pub fn problem(role: Role, path: &str, missing: bool) -> Option<Problem> {
    if role == Role::Kryoto {
        return (missing && is_program(path)).then_some(Problem::Missing);
    }
    if missing {
        return Some(Problem::Missing);
    }
    Some(if role.emulator() && is_program(path) { Problem::Broken } else { Problem::Changed })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub path: String,
    pub role: Role,
    pub problem: Problem,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CheckReport {
    pub release_id: String,
    pub at: u64,
    /// Every file hashed, not only the suspect ones.
    pub full: bool,
    pub files: usize,
    pub hashed: usize,
    /// Left out: Kryoto changed them itself (a switch, an add-on).
    pub skipped: usize,
    pub missing: Vec<Finding>,
    pub broken: Vec<Finding>,
    pub changed: usize,
    pub changed_sample: Vec<Finding>,
    pub cancelled: bool,
}

impl CheckReport {
    pub fn alarm(&self) -> bool {
        !self.missing.is_empty() || !self.broken.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckProgress {
    pub game_id: String,
    pub done: usize,
    pub total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Finds listed paths on disk ignoring case: exact first (all Windows needs),
/// then folder by folder, each folder read once.
pub struct Finder<'a> {
    root: &'a Path,
    folders: HashMap<PathBuf, Option<HashMap<String, std::ffi::OsString>>>,
}

impl<'a> Finder<'a> {
    pub fn new(root: &'a Path) -> Self {
        Self { root, folders: HashMap::new() }
    }

    /// The file's path, when it is there (a folder is not a file).
    pub fn find(&mut self, rel: &str) -> Option<PathBuf> {
        let exact = self.root.join(rel);
        if exact.is_file() {
            return Some(exact);
        }
        let mut at = self.root.to_path_buf();
        for part in rel.split('/') {
            let names = self.folders.entry(at.clone()).or_insert_with(|| {
                std::fs::read_dir(&at).ok().map(|d| d.flatten().map(|e| (e.file_name().to_string_lossy().to_lowercase(), e.file_name())).collect())
            });
            let name = names.as_ref()?.get(&part.to_lowercase())?.clone();
            at.push(name);
        }
        at.is_file().then_some(at)
    }

    /// `rel` as it is on disk.
    pub fn rel_of(&self, path: &Path) -> String {
        path.strip_prefix(self.root).unwrap_or(path).to_string_lossy().replace('\\', "/")
    }
}

/// SHA-256 of a file, stopping when asked, and leaving the disk room between
/// reads: a pause a quarter as long as each read took, so a check over a game
/// on a hard disk never starves a download or the game next to it.
pub fn hash_file(path: &Path, stop: &AtomicBool, read: &mut dyn FnMut(u64)) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if stop.load(Ordering::SeqCst) {
            return None;
        }
        let started = Instant::now();
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
        read(n as u64);
        let spent = started.elapsed();
        if spent > Duration::from_millis(2) {
            std::thread::sleep(spent / 4);
        }
    }
    Some(format!("{:x}", sha.finalize()))
}

/// Everything a check needs to know besides the folder and the list.
#[derive(Default)]
pub struct CheckWith {
    /// `.kryoto-release.json`'s stamps, by `key`.
    pub stamps: HashMap<String, Stamp>,
    /// Paths Kryoto changed itself (repair, add-ons, Kryoto Online), by `key`.
    pub skip: BTreeSet<String>,
    /// Kryoto switched the emulator: the release's emulator files are not expected.
    pub skip_emulator: bool,
    pub full: bool,
}

/// Check `root` against `list`. Off the UI thread only.
pub fn check_tree(root: &Path, list: &FileList, with: &CheckWith, stop: &AtomicBool, progress: &mut dyn FnMut(usize, usize, u64, u64)) -> CheckReport {
    let files = list.listed();
    let mut report = CheckReport { release_id: list.release_id.clone(), at: now(), full: with.full, files: files.len(), ..Default::default() };
    let bytes_total: u64 = if with.full { files.iter().map(|f| f.size).sum() } else { 0 };
    let mut bytes_done = 0u64;
    let mut finder = Finder::new(root);
    let mut last = Instant::now();
    for (i, f) in files.iter().enumerate() {
        if stop.load(Ordering::SeqCst) {
            report.cancelled = true;
            break;
        }
        let k = key(&f.path);
        if with.skip.contains(&k) || (with.skip_emulator && f.role.emulator()) {
            report.skipped += 1;
            bytes_done += f.size;
            continue;
        }
        let found = finder.find(&f.path);
        let stamp = found.as_deref().and_then(cu::stamp);
        let step = decide(f.size, stamp, with.stamps.get(&k).copied(), with.full, f.role.emulator());
        let mismatch = match step {
            Step::Fine | Step::Unverified => {
                bytes_done += f.size;
                None
            }
            Step::Missing => {
                bytes_done += f.size;
                Some(true)
            }
            Step::Differs => {
                bytes_done += f.size;
                Some(false)
            }
            Step::Hash => {
                let path = found.clone().unwrap_or_default();
                let before = bytes_done;
                let sha = hash_file(&path, stop, &mut |n| bytes_done += n);
                bytes_done = before + f.size;
                report.hashed += 1;
                match sha {
                    None if stop.load(Ordering::SeqCst) => {
                        report.cancelled = true;
                        break;
                    }
                    // Unreadable now (locked by the running game): not a finding.
                    None => None,
                    Some(sha) => (sha != f.sha256).then_some(false),
                }
            }
        };
        if let Some(missing) = mismatch {
            let finding = |p| Finding { path: f.path.clone(), role: f.role, problem: p };
            match problem(f.role, &f.path, missing) {
                Some(Problem::Missing) => report.missing.push(finding(Problem::Missing)),
                Some(Problem::Broken) => report.broken.push(finding(Problem::Broken)),
                Some(Problem::Changed) => {
                    report.changed += 1;
                    if report.changed_sample.len() < CHANGED_SAMPLE {
                        report.changed_sample.push(finding(Problem::Changed));
                    }
                }
                None => {}
            }
        }
        if last.elapsed() > Duration::from_millis(150) {
            progress(i + 1, files.len(), bytes_done, bytes_total);
            last = Instant::now();
        }
    }
    if !report.cancelled {
        progress(files.len(), files.len(), bytes_total, bytes_total);
    }
    report
}

/// What Kryoto itself changed in this game, so a check never blames the
/// player's antivirus for it: the repair journal, the swap record, add-ons
/// and Kryoto Online. `true` with it when the emulator was switched.
fn kryoto_changes(game: &LibraryGame, root: &Path) -> (BTreeSet<String>, bool) {
    let mut skip = BTreeSet::new();
    let journal = journal::load(root).ok().flatten().filter(|j| j.state != "undone");
    if let Some(j) = &journal {
        skip.extend(j.entries.iter().map(|e| key(&e.path)));
    }
    let swapped = swapped_paths(root);
    let switched = journal.is_some() || !swapped.is_empty() || game.repair.is_some() || game.online.is_some();
    skip.extend(swapped);
    for a in &game.addons {
        skip.extend(a.files.iter().chain(a.replaced.iter()).map(|p| key(p)));
    }
    if let Some(o) = &game.online {
        skip.extend(o.added.iter().map(|p| key(p)));
        skip.extend(o.saved.iter().map(|(p, _)| key(p)));
    }
    (skip, switched)
}

fn stamps_of(root: &Path) -> HashMap<String, Stamp> {
    read_json::<ReleaseRecord>(&root.join(RELEASE_RECORD)).map(|r| r.files.into_iter().map(|(p, s)| (key(&p), s)).collect()).unwrap_or_default()
}

/// The stored report, with what has come back since left out (a player who
/// put a file back by hand): cheap, a look at the few files it names.
/// A broken file stays named until the next check or Get the files back:
/// being there again proves nothing about its bytes.
fn current(report: Option<CheckReport>, root: &Path) -> Option<CheckReport> {
    let mut r = report?;
    let mut finder = Finder::new(root);
    r.missing.retain(|f| finder.find(&f.path).is_none());
    Some(r)
}

static CHECKING: Mutex<Vec<(String, Arc<AtomicBool>)>> = Mutex::new(Vec::new());

fn game<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<LibraryGame, String> {
    crate::library::load(app)?.into_iter().find(|g| g.id == id).ok_or_else(|| "That game is no longer in the library.".into())
}

/// Check one game against its list and keep the result. `None` when it has
/// no list. Waits for nothing on the UI thread.
pub async fn check<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, full: bool, network: bool) -> Result<Option<CheckReport>, String> {
    if game.install_dir.is_empty() || game.launcher.is_some() {
        return Ok(None);
    }
    if game.slug.as_deref().is_some_and(|s| crate::downloads::unpacking(app, s)) {
        return Err("The game is being installed. Check its files once that is done.".into());
    }
    let Some(list) = ensure_list(app, game, network).await else { return Ok(None) };
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut running = CHECKING.lock().unwrap_or_else(|e| e.into_inner());
        if running.iter().any(|(id, _)| id == &game.id) {
            return Err("Its files are being checked already.".into());
        }
        running.push((game.id.clone(), stop.clone()));
    }
    let root = PathBuf::from(&game.install_dir);
    let (skip, skip_emulator) = kryoto_changes(game, &root);
    let (app2, id) = (app.clone(), game.id.clone());
    let stop2 = stop.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        let with = CheckWith { stamps: stamps_of(&root), skip, skip_emulator, full };
        let mut emit = |done, total, bytes_done, bytes_total| {
            let _ = app2.emit("files-check-progress", CheckProgress { game_id: id.clone(), done, total, bytes_done, bytes_total });
        };
        check_tree(&root, &list, &with, &stop2, &mut emit)
    })
    .await;
    CHECKING.lock().unwrap_or_else(|e| e.into_inner()).retain(|(id, _)| id != &game.id);
    let report = report.map_err(|e| e.to_string())?;
    if !report.cancelled {
        with_entry(app, game, |e| e.report = Some(report.clone()));
        let names: Vec<&str> = report.missing.iter().chain(report.broken.iter()).take(6).map(|f| f.path.as_str()).collect();
        let line = format!(
            "{}: checked {} files ({} hashed{}): {} missing, {} broken, {} changed{}",
            game.title,
            report.files,
            report.hashed,
            if full { ", all" } else { "" },
            report.missing.len(),
            report.broken.len(),
            report.changed,
            if names.is_empty() { String::new() } else { format!(": {}", names.join(", ")) }
        );
        if report.alarm() {
            crate::logging::warn("files", &line);
        } else {
            crate::logging::info("files", &line);
        }
        let _ = app.emit("files-checked", &game.id);
    }
    Ok(Some(report))
}

/// After an install: the list fetched and a quick look (existence and size),
/// so a file an antivirus took while it unpacked is named on the game's page.
/// A swap record from the build before goes: its files are gone with it.
pub fn after_install<R: Runtime>(app: &AppHandle<R>, game_id: &str) {
    let (app, id) = (app.clone(), game_id.to_string());
    tauri::async_runtime::spawn(async move {
        let Ok(game) = game(&app, &id) else { return };
        let root = PathBuf::from(&game.install_dir);
        if root.join(SWAP_RECORD).is_file() && journal::load(&root).ok().flatten().is_none_or(|j| j.state == "undone") {
            let _ = std::fs::remove_file(root.join(SWAP_RECORD));
            let _ = std::fs::remove_dir_all(root.join(SWAP_DIR));
        }
        // The release record is written on its own thread: give it a moment.
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Err(e) = check(&app, &game, false, true).await {
            crate::logging::warn("files", &format!("{}: no check after the install: {e}", game.title));
        }
    });
}

/// A game's list, its findings, and what the Emulator row says about it.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesState {
    /// `yes`: a list is kept; `none`: kryo.to has none for this release;
    /// `unknown`: not asked yet, or this game is not a kryo.to release.
    pub list: &'static str,
    pub files: usize,
    pub emulator: Option<String>,
    pub emulator_off: bool,
    /// Steam files the uploader kept as the game shipped them.
    pub kept_own: Vec<String>,
    pub report: Option<CheckReport>,
    pub checking: bool,
    /// The download is still on this PC: missing files come from it.
    pub archive: bool,
    /// "12.3 GB": what downloading it again takes.
    pub download_size: Option<String>,
}

fn state_of<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> FilesState {
    let e = entry(app, game);
    let kept = e.release_id.as_deref().and_then(|id| dir(app).map(|d| read_kept(&d, id))).unwrap_or_default();
    let root = PathBuf::from(&game.install_dir);
    let list = kept.list.as_ref();
    FilesState {
        list: if list.is_some() {
            "yes"
        } else if kept.none_at.is_some() || (e.resolved_at.is_some() && e.release_id.is_none()) {
            "none"
        } else {
            "unknown"
        },
        files: list.map(|l| l.listed().len()).unwrap_or(0),
        emulator: list.and_then(FileList::emulator_words),
        emulator_off: list.is_some_and(|l| l.emulator_off),
        kept_own: list.map(|l| l.excluded.clone()).unwrap_or_default(),
        report: current(e.report.filter(|_| list.is_some()), &root),
        checking: CHECKING.lock().map(|c| c.iter().any(|(id, _)| id == &game.id)).unwrap_or(false),
        archive: kept_archive(app, game).is_some(),
        download_size: e.download_size,
    }
}

/// What is known, with no question to kryo.to: Properties and the game page.
#[tauri::command(async)]
pub fn game_files_state(app: AppHandle, game_id: String) -> Result<FilesState, String> {
    Ok(state_of(&app, &game(&app, &game_id)?))
}

/// The list fetched if it is due (on opening Properties), then the state.
#[tauri::command]
pub async fn game_files_fetch(app: AppHandle, game_id: String) -> Result<FilesState, String> {
    let game = game(&app, &game_id)?;
    ensure_list(&app, &game, true).await;
    Ok(state_of(&app, &game))
}

/// Check files: `full` hashes every file (with progress, and Cancel).
/// Without a list, what Kryoto installed is looked at instead (antivirus.rs).
#[tauri::command]
pub async fn game_files_check(app: AppHandle, game_id: String, full: bool) -> Result<FilesState, String> {
    let game = game(&app, &game_id)?;
    let report = check(&app, &game, full, true).await?;
    let mut state = state_of(&app, &game);
    // Stopped halfway: said so, and the last whole check stays the one kept.
    if let Some(r) = report.filter(|r| r.cancelled) {
        state.report = Some(r);
    }
    Ok(state)
}

#[tauri::command]
pub fn game_files_check_cancel(game_id: String) {
    if let Ok(list) = CHECKING.lock() {
        for (id, stop) in list.iter() {
            if id == &game_id {
                stop.store(true, Ordering::SeqCst);
            }
        }
    }
}

/// For the game page's antivirus card: the files the last check found
/// missing or broken, with what each is, when the release has a list.
pub fn alarms<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Option<Vec<Finding>> {
    cached_list(app, game)?;
    let report = current(entry(app, game).report, Path::new(&game.install_dir))?;
    Some(report.missing.into_iter().chain(report.broken).collect())
}

/* ── Getting files back ─────────────────────────────────────────────────── */

/// The download this game was installed from, when it is still on this PC:
/// the same archive by hash when both are known, else the same build.
fn kept_archive<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Option<PathBuf> {
    let state = app.try_state::<crate::downloads::Downloads>()?;
    crate::downloads::downloads_list(state)
        .into_iter()
        .filter(|d| d.addon.is_none() && d.status == crate::downloads::Status::Installed)
        .filter(|d| d.game_id.as_deref() == Some(game.id.as_str()) || (d.slug.is_some() && d.slug == game.slug))
        .filter(|d| match (game.build_sha.as_deref(), d.sha256.as_deref()) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => d.meta.version == game.version,
        })
        .map(|d| PathBuf::from(d.archive_path))
        .find(|p| p.is_file())
}

/// The folder inside the archive the game's files sit under ("" when at the
/// top): the one most wanted files agree on, the shortest on a tie. Pure.
pub fn archive_prefix(entries: &[String], wanted: &[String]) -> Option<String> {
    let lower: Vec<String> = entries.iter().map(|e| e.replace('\\', "/").to_lowercase()).collect();
    let mut votes: BTreeMap<String, usize> = BTreeMap::new();
    for w in wanted {
        let w = key(w);
        for e in &lower {
            if *e == w {
                *votes.entry(String::new()).or_default() += 1;
            } else if e.len() > w.len() && e.ends_with(&w) && e.as_bytes()[e.len() - w.len() - 1] == b'/' {
                *votes.entry(e[..e.len() - w.len()].to_string()).or_default() += 1;
            }
        }
    }
    votes.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.len().cmp(&a.0.len()))).map(|(p, _)| p)
}

/// Each wanted path's name in the archive, under `prefix`. Pure.
pub fn archive_names(entries: &[String], prefix: &str, wanted: &[String]) -> Vec<(String, String)> {
    let by_key: HashMap<String, &String> = entries.iter().map(|e| (e.replace('\\', "/").to_lowercase(), e)).collect();
    wanted.iter().filter_map(|w| by_key.get(&format!("{prefix}{}", key(w))).map(|e| (w.clone(), (*e).clone()))).collect()
}

fn archive_entries(archive: &Path) -> Option<Vec<String>> {
    let ext = archive.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "7z" => Some(sevenz_rust::Archive::open(archive).ok()?.files.iter().filter(|f| !f.is_directory()).map(|f| f.name().replace('\\', "/")).collect()),
        "zip" => {
            let zip = zip::ZipArchive::new(std::fs::File::open(archive).ok()?).ok()?;
            Some(zip.file_names().filter(|n| !n.ends_with('/')).map(String::from).collect())
        }
        _ => None,
    }
}

/// `names` out of a .7z without 7-Zip on the PC: the built-in reader, which
/// goes through the whole archive (one solid block) to reach them.
fn extract_7z_named(archive: &Path, dest: &Path, names: &BTreeSet<String>, stop: &AtomicU8, progress: &dyn Fn(u64, Option<u64>)) -> Result<(), String> {
    let total: u64 = sevenz_rust::Archive::open(archive).map_err(|e| e.to_string())?.files.iter().map(|f| f.size()).sum();
    let mut done = 0u64;
    sevenz_rust::decompress_file_with_extract_fn(archive, dest, |entry, reader, _| {
        if stop.load(Ordering::SeqCst) != 0 {
            return Err(sevenz_rust::Error::other("stopped"));
        }
        let name = entry.name().replace('\\', "/");
        let wanted = !entry.is_directory() && names.contains(&name) && cu::safe_path(&name);
        let mut sink: Box<dyn std::io::Write> = if wanted {
            let path = dest.join(&name);
            std::fs::create_dir_all(path.parent().unwrap_or(dest)).map_err(sevenz_rust::Error::io)?;
            Box::new(std::fs::File::create(path).map_err(sevenz_rust::Error::io)?)
        } else {
            Box::new(std::io::sink())
        };
        let n = std::io::copy(reader, &mut sink).map_err(sevenz_rust::Error::io)?;
        done += n;
        progress(done, Some(total));
        Ok(true)
    })
    .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Restored {
    /// Put back, each checked against its listed hash.
    pub restored: Vec<String>,
    /// From the download Kryoto kept.
    pub from_archive: usize,
    /// Not on this PC: only downloading the game again brings them.
    pub left: Vec<String>,
    /// "12.3 GB".
    pub download_size: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RestoreProgress {
    game_id: String,
    text: String,
    percent: Option<u8>,
}

/// Put `from` at `to` (inside `root`) once its hash is `sha`. A copy, so a
/// source inside the game stays where it is.
fn place(root: &Path, from: &Path, rel: &str, sha: &str) -> Result<(), String> {
    if journal::digest(from)? != sha {
        return Err(format!("{rel}: the copy found is not the release's file"));
    }
    let to = journal::safe_path(root, rel)?;
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = to.with_file_name(format!(".{}.kryoto-restore", to.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
    std::fs::copy(from, &tmp).map_err(|e| format!("{rel}: {e}"))?;
    if to.exists() {
        journal::make_writable(&to)?;
    }
    std::fs::rename(&tmp, &to).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{rel}: {e}")
    })
}

/// Files on this PC with the same bytes as a wanted one: another listed file
/// (a shipped `.dll.kryoto` is the original's twin) or an emulator build
/// Repair keeps. Matched by name and size first, then proven by hash.
fn local_twin(finder: &mut Finder, list: &FileList, want: &ListedFile, tools: &[(String, u64, PathBuf)]) -> Option<PathBuf> {
    for other in list.listed().iter().filter(|o| o.sha256 == want.sha256 && key(&o.path) != key(&want.path)) {
        if let Some(p) = finder.find(&other.path).filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() == want.size)) {
            if journal::digest(&p).ok().as_deref() == Some(want.sha256.as_str()) {
                return Some(p);
            }
        }
    }
    let name = want.path.rsplit('/').next().unwrap_or(&want.path).to_lowercase();
    tools
        .iter()
        .filter(|(n, size, _)| *n == name && *size == want.size)
        .find(|(_, _, p)| journal::digest(p).ok().as_deref() == Some(want.sha256.as_str()))
        .map(|(_, _, p)| p.clone())
}

/// The emulator builds Repair keeps (`repair-tools` in the app's cache), by
/// lowercase name and size. Small: a few builds of a few files each.
fn tool_files<R: Runtime>(app: &AppHandle<R>) -> Vec<(String, u64, PathBuf)> {
    let Ok(base) = app.path().app_cache_dir() else { return Vec::new() };
    let mut out = Vec::new();
    let mut stack = vec![(base.join("repair-tools"), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for e in read.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() && depth < 6 {
                stack.push((e.path(), depth + 1));
            } else if t.is_file() {
                if let Ok(m) = e.metadata() {
                    out.push((e.file_name().to_string_lossy().to_lowercase(), m.len(), e.path()));
                }
            }
            if out.len() > 20_000 {
                return out;
            }
        }
    }
    out
}

/// Get the files back: every missing or broken file the last check named,
/// from this PC when its bytes are here, else said plainly (with the
/// download's size) so the game's page offers downloading it again.
#[tauri::command]
pub async fn game_files_restore(app: AppHandle, game_id: String) -> Result<Restored, String> {
    let game = game(&app, &game_id)?;
    if crate::library::is_running(&app, &game.id) {
        return Err("Close the game before getting its files back.".into());
    }
    let list = cached_list(&app, &game).ok_or("Kryoto has no list of this release's files.")?;
    let root = PathBuf::from(&game.install_dir);
    let e = entry(&app, &game);
    let wanted: Vec<ListedFile> = {
        let report = current(e.report.clone(), &root).unwrap_or_default();
        let names: BTreeSet<String> = report.missing.iter().chain(report.broken.iter()).map(|f| key(&f.path)).collect();
        list.listed().iter().filter(|f| names.contains(&key(&f.path))).cloned().collect()
    };
    let archive = kept_archive(&app, &game);
    #[cfg(windows)]
    if archive.as_ref().is_some_and(|a| a.extension().is_some_and(|x| x.eq_ignore_ascii_case("7z"))) {
        crate::downloads::ensure_7zr(&app).await;
    }
    let tools = tool_files(&app);
    let (app2, id) = (app.clone(), game.id.clone());
    let say = move |text: &str, percent: Option<u8>| {
        let _ = app2.emit("files-restore-progress", RestoreProgress { game_id: id.clone(), text: text.into(), percent });
    };
    let mut out = tauri::async_runtime::spawn_blocking(move || -> Result<Restored, String> {
        let _guard = journal::lock(&root)?;
        let mut out = Restored::default();
        let mut finder = Finder::new(&root);
        let mut left: Vec<ListedFile> = Vec::new();
        say("Looking for the files on this PC", None);
        for f in &wanted {
            match local_twin(&mut finder, &list, f, &tools) {
                Some(from) => match place(&root, &from, &f.path, &f.sha256) {
                    Ok(()) => out.restored.push(f.path.clone()),
                    Err(e) => {
                        crate::logging::warn("files", &e);
                        left.push(f.clone());
                    }
                },
                None => left.push(f.clone()),
            }
        }
        if let (false, Some(archive)) = (left.is_empty(), archive) {
            let entries = archive_entries(&archive).unwrap_or_default();
            let paths: Vec<String> = left.iter().map(|f| f.path.clone()).collect();
            let prefix = archive_prefix(&entries, &paths).unwrap_or_default();
            let names = archive_names(&entries, &prefix, &paths);
            if !names.is_empty() {
                let staging = root.with_file_name(format!(".{}.kryoto-restore", root.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
                let _ = std::fs::remove_dir_all(&staging);
                let count = names.len();
                let label = format!("Taking {count} file{} from the download Kryoto kept", if count == 1 { "" } else { "s" });
                say(&label, Some(0));
                let progress = |done: u64, total: Option<u64>| {
                    let pct = total.filter(|t| *t > 0).map(|t| (done.min(t) * 100 / t) as u8);
                    say(&label, pct);
                };
                let stop = AtomicU8::new(0);
                let list_names: Vec<String> = names.iter().map(|(_, n)| n.clone()).collect();
                let unpacked = match crate::downloads::extract_named(&archive, &staging, &list_names, &progress, &stop) {
                    Ok(true) => Ok(()),
                    Ok(false) if archive.extension().is_some_and(|x| x.eq_ignore_ascii_case("7z")) => {
                        extract_7z_named(&archive, &staging, &list_names.iter().cloned().collect(), &stop, &progress)
                    }
                    Ok(false) => Err("no 7-Zip to read it".to_string()),
                    Err(e) => Err(e),
                };
                if let Err(e) = &unpacked {
                    crate::logging::warn("files", &format!("could not take files from {}: {e}", archive.display()));
                }
                if unpacked.is_ok() {
                    for (rel, name) in &names {
                        let Some(f) = left.iter().find(|f| &f.path == rel) else { continue };
                        let from = staging.join(name);
                        if from.is_file() && place(&root, &from, &f.path, &f.sha256).is_ok() {
                            out.restored.push(f.path.clone());
                            out.from_archive += 1;
                        }
                    }
                }
                let _ = std::fs::remove_dir_all(&staging);
                let _ = std::fs::remove_file(staging.with_extension("names"));
                left.retain(|f| !out.restored.contains(&f.path));
            }
        }
        out.left = left.into_iter().map(|f| f.path).collect();
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())??;
    out.download_size = e.download_size;
    // What came back is no longer a finding.
    let back: BTreeSet<String> = out.restored.iter().map(|p| key(p)).collect();
    with_entry(&app, &game, |e| {
        if let Some(r) = e.report.as_mut() {
            r.missing.retain(|f| !back.contains(&key(&f.path)));
            r.broken.retain(|f| !back.contains(&key(&f.path)));
        }
    });
    crate::logging::info("files", &format!("{}: {} file(s) back ({} from the kept download), {} not on this PC", game.title, out.restored.len(), out.from_archive, out.left.len()));
    let _ = app.emit("files-checked", &game.id);
    let _ = app.emit("library-changed", ());
    Ok(out)
}

/* ── A clean emulator swap ──────────────────────────────────────────────── */

/// One original to put back from the backup the release ships.
#[derive(Debug, Clone, PartialEq)]
pub struct Restore {
    pub path: String,
    pub backup: String,
    pub original_sha256: String,
}

/// What taking the release's own emulator out does, by role: originals back
/// from their shipped backup (`restore`), what the emulator added set aside
/// (`remove`), replaced files with no usable backup left in place for Repair
/// to deal with as it always has (`keep`). Fixes, Kryoto's files and the game
/// are never in it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SwapPlan {
    pub restore: Vec<Restore>,
    pub remove: Vec<String>,
    pub keep: Vec<String>,
}

/// The plan for `list`, with `usable` saying whether a backup is on disk with
/// the original's hash. A backup that is not usable stays, with what it backs
/// up: Repair reads the original from it. Pure, for tests.
pub fn swap_plan(list: &FileList, usable: &mut dyn FnMut(&Restore) -> bool) -> SwapPlan {
    let mut plan = SwapPlan::default();
    if list.emulator_off {
        return plan;
    }
    let mut needed = BTreeSet::new();
    for f in list.listed().iter().filter(|f| f.role == Role::EmulatorReplaced) {
        let candidate = match (&f.backup, &f.original_sha256) {
            (Some(b), Some(o)) => Some(Restore { path: f.path.clone(), backup: b.clone(), original_sha256: o.clone() }),
            _ => None,
        };
        match candidate {
            Some(r) if usable(&r) => plan.restore.push(r),
            other => {
                if let Some(r) = other {
                    needed.insert(key(&r.backup));
                }
                plan.keep.push(f.path.clone());
            }
        }
    }
    let restored: BTreeSet<String> = plan.restore.iter().map(|r| key(&r.path)).collect();
    plan.remove = list
        .listed()
        .iter()
        .filter(|f| f.role == Role::EmulatorAdded && !needed.contains(&key(&f.path)) && !restored.contains(&key(&f.path)))
        .map(|f| f.path.clone())
        .collect();
    plan
}

/// What a swap set aside, so "Back to the release's own files" is exact.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SwapRecord {
    pub release_id: String,
    pub at: u64,
    pub entries: Vec<SwapEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SwapEntry {
    /// As it is on disk, relative to the game.
    pub path: String,
    /// Where the release's file went (under `SWAP_DIR`), and its hash.
    pub stash: Option<String>,
    pub sha256: Option<String>,
    /// Kryoto wrote the original here from this backup: Undo takes it out.
    pub restored_from: Option<String>,
}

fn read_swap(root: &Path) -> Option<SwapRecord> {
    read_json(&root.join(SWAP_RECORD))
}

fn save_swap(root: &Path, record: &SwapRecord) -> Result<(), String> {
    write_atomic(&journal::safe_path(root, SWAP_RECORD)?, &serde_json::to_vec_pretty(record).map_err(|e| e.to_string())?)
}

/// The paths a swap took out or changed, by `key`.
pub fn swapped_paths(root: &Path) -> BTreeSet<String> {
    read_swap(root).map(|r| r.entries.iter().map(|e| key(&e.path)).collect()).unwrap_or_default()
}

pub fn has_swap(root: &Path) -> bool {
    root.join(SWAP_RECORD).is_file()
}

/// Move `rel` aside into the swap folder, noting it first, so a crash halfway
/// leaves a record that still names it.
fn stash(root: &Path, record: &mut SwapRecord, rel: &str, restored_from: Option<String>) -> Result<(), String> {
    let live = journal::safe_path(root, rel)?;
    let n = record.entries.len();
    if !live.is_file() {
        record.entries.push(SwapEntry { path: rel.into(), stash: None, sha256: None, restored_from });
        return save_swap(root, record);
    }
    let name = format!("{SWAP_DIR}/{n}");
    let sha = journal::digest(&live)?;
    record.entries.push(SwapEntry { path: rel.into(), stash: Some(name.clone()), sha256: Some(sha), restored_from });
    save_swap(root, record)?;
    let target = journal::safe_path(root, &name)?;
    std::fs::create_dir_all(target.parent().unwrap_or(root)).map_err(|e| e.to_string())?;
    journal::make_writable(&live)?;
    if std::fs::rename(&live, &target).is_err() {
        std::fs::copy(&live, &target).map_err(|e| format!("{rel}: {e}"))?;
        std::fs::remove_file(&live).map_err(|e| format!("{rel}: {e}"))?;
    }
    Ok(())
}

/// Before a switch: the release's own emulator out, by the list's roles.
/// `Ok(None)` when there is nothing to take out (no emulator in the list, or
/// `emulator_off`). Undone by [`unclean`], and by itself if it stops halfway.
pub fn clean_for_switch(root: &Path, list: &FileList) -> Result<Option<(usize, usize)>, String> {
    if has_swap(root) {
        return Ok(None);
    }
    let _guard = journal::lock(root)?;
    let mut finder = Finder::new(root);
    let mut usable = |r: &Restore| finder.find(&r.backup).is_some_and(|p| journal::digest(&p).ok().as_deref() == Some(r.original_sha256.as_str()));
    let plan = swap_plan(list, &mut usable);
    if plan.restore.is_empty() && plan.remove.is_empty() {
        return Ok(None);
    }
    let mut record = SwapRecord { release_id: list.release_id.clone(), at: now(), entries: Vec::new() };
    match clean_steps(root, &plan, &mut record) {
        Ok(counts) => Ok(Some(counts)),
        Err(e) => {
            drop(_guard);
            match unclean(root) {
                Ok(_) => Err(format!("{e} Nothing was changed.")),
                Err(u) => Err(format!("{e} Putting the files back failed too: {u}. Keep the .kryoto-repair folder and contact support.")),
            }
        }
    }
}

/// `clean_for_switch`'s work, each step noted in `record` before it is done.
fn clean_steps(root: &Path, plan: &SwapPlan, record: &mut SwapRecord) -> Result<(usize, usize), String> {
    let mut finder = Finder::new(root);
    for r in &plan.restore {
        let backup = finder.find(&r.backup).ok_or_else(|| format!("{} is gone", r.backup))?;
        let rel = finder.find(&r.path).map(|p| finder.rel_of(&p)).unwrap_or_else(|| r.path.clone());
        // Read once more: what is copied is what was checked.
        let bytes = std::fs::read(&backup).map_err(|e| e.to_string())?;
        if format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes)) != r.original_sha256 {
            return Err(format!("{} is not the original it backs up", r.backup));
        }
        stash(root, record, &rel, Some(finder.rel_of(&backup)))?;
        let live = journal::safe_path(root, &rel)?;
        std::fs::create_dir_all(live.parent().unwrap_or(root)).map_err(|e| e.to_string())?;
        std::fs::write(&live, &bytes).map_err(|e| format!("{rel}: {e}"))?;
    }
    let mut removed = 0;
    for p in &plan.remove {
        if let Some(found) = finder.find(p) {
            let rel = finder.rel_of(&found);
            stash(root, record, &rel, None)?;
            removed += 1;
            // Folders the emulator made and that are empty now go too.
            let mut dir = found.parent().map(Path::to_path_buf);
            while let Some(d) = dir.filter(|d| d.starts_with(root) && d != root) {
                if std::fs::remove_dir(&d).is_err() {
                    break;
                }
                dir = d.parent().map(Path::to_path_buf);
            }
        }
    }
    Ok((plan.restore.len(), removed))
}

/// Put back what [`clean_for_switch`] set aside. `Ok(false)` when there was
/// nothing. Every copy is checked by hash before any file is touched.
pub fn unclean(root: &Path) -> Result<bool, String> {
    let Some(record) = read_swap(root) else { return Ok(false) };
    let _guard = journal::lock(root)?;
    for e in &record.entries {
        journal::safe_path(root, &e.path)?;
        if let Some(s) = &e.stash {
            if !s.starts_with(&format!("{SWAP_DIR}/")) {
                return Err("The swap record is invalid. Keep the .kryoto-repair folder and contact support.".into());
            }
            let p = journal::safe_path(root, s)?;
            // A file never moved (a crash right after it was noted) is still where it was.
            if p.is_file() && journal::digest(&p).ok() != e.sha256 {
                return Err("A copy of the release's own emulator files changed. Keep the .kryoto-repair folder and contact support.".into());
            }
        }
    }
    for e in record.entries.iter().rev() {
        let live = journal::safe_path(root, &e.path)?;
        let stashed = e.stash.as_deref().map(|s| journal::safe_path(root, s)).transpose()?.filter(|p| p.is_file());
        if (e.restored_from.is_some() || stashed.is_some()) && live.is_file() {
            journal::make_writable(&live)?;
            std::fs::remove_file(&live).map_err(|err| format!("{}: {err}", e.path))?;
        }
        if let Some(from) = stashed {
            std::fs::create_dir_all(live.parent().unwrap_or(root)).map_err(|err| err.to_string())?;
            if std::fs::rename(&from, &live).is_err() {
                std::fs::copy(&from, &live).map_err(|err| format!("{}: {err}", e.path))?;
            }
        }
    }
    let _ = std::fs::remove_dir_all(root.join(SWAP_DIR));
    let _ = std::fs::remove_file(root.join(SWAP_RECORD));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(bytes))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kryoto-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file(path: &str, bytes: &[u8], role: Role) -> ListedFile {
        ListedFile { path: path.into(), size: bytes.len() as u64, sha256: sha(bytes), role, ..Default::default() }
    }

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn the_installed_release_is_found_by_its_hash_then_its_version() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let json = serde_json::json!({ "releases": [
            { "id": "20", "version": "1.1", "primary": true, "download_size": "3.2 GB", "hashes": [{ "sha256": a }], "files": { "has_list": true } },
            { "id": "19", "version": "1.0", "primary": false, "hashes": [{ "sha256": b }], "files": null },
            { "id": 18, "version": "0.9", "primary": false, "hashes": [] }
        ]});
        let p = pick_release(&json, Some(&b.to_uppercase()), Some("1.1")).unwrap();
        assert_eq!((p.id.as_str(), p.has_list), ("19", Some(false)), "the hash wins over the version");
        let p = pick_release(&json, None, Some("1.1")).unwrap();
        assert_eq!((p.id.as_str(), p.has_list, p.download_size.as_deref()), ("20", Some(true), Some("3.2 GB")));
        // A kryo.to too old to say anything about lists.
        assert_eq!(pick_release(&json, None, Some("0.9")).unwrap(), Picked { id: "18".into(), has_list: None, download_size: None });
        // Another upload of the same version is not this install's list.
        assert_eq!(pick_release(&json, Some(&"c".repeat(64)), Some("1.1")), None);
        assert_eq!(pick_release(&json, None, None), None);
        assert_eq!(pick_release(&serde_json::json!({ "error": "x" }), None, Some("1.1")), None);
    }

    #[test]
    fn a_missing_list_and_an_offline_kryo_to_are_asked_again_only_later() {
        assert!(due(None, NO_LIST_RETRY_SECS, 1_000));
        assert!(!due(Some(1_000), NO_LIST_RETRY_SECS, 1_000 + 3600));
        assert!(due(Some(1_000), NO_LIST_RETRY_SECS, 1_000 + NO_LIST_RETRY_SECS));
        assert!(!due(Some(1_000), FAILED_RETRY_SECS, 1_000 + 60));
        let body = br#"{"schema":1,"release_id":"20","slug":"g","files":[{"path":"a\\b.dll","size":1,"sha256":"x","role":"game"},{"path":"ok.exe","size":2,"sha256":"ABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB","role":"emulator-added"},{"path":"../up.dll","size":2,"sha256":"abababababababababababababababababababababababababababababababab","role":"game"}]}"#;
        match read_answer(200, Some("\"e1\"".into()), body) {
            Answer::List(list, etag) => {
                assert_eq!(etag.as_deref(), Some("\"e1\""));
                assert_eq!(list.listed().len(), 1, "bad hashes and paths outside the game are dropped");
                assert_eq!(list.listed()[0].sha256, "ab".repeat(32));
                assert_eq!(list.listed()[0].role, Role::EmulatorAdded);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(read_answer(404, None, br#"{"error":"no_file_list"}"#), Answer::NoList);
        assert_eq!(read_answer(304, None, b""), Answer::Unchanged);
        assert_eq!(read_answer(429, None, b""), Answer::Failed);
        assert_eq!(read_answer(503, None, b"<html>"), Answer::Failed);
        assert_eq!(read_answer(200, None, b"not json"), Answer::Failed);
        assert_eq!(read_answer(200, None, br#"{"schema":1,"files":null}"#), Answer::NoList);
        // An unknown role from a newer Forge reads, as the game's.
        let newer = br#"{"files":[{"path":"a.dll","size":1,"sha256":"abababababababababababababababababababababababababababababababab","role":"shader-cache"}]}"#;
        assert!(matches!(read_answer(200, None, newer), Answer::List(l, _) if l.listed()[0].role == Role::Other));
    }

    #[test]
    fn the_cache_keeps_a_list_by_release_id() {
        let dir = scratch("cache");
        let list = FileList { release_id: "42".into(), files: Some(vec![file("a.exe", b"a", Role::Game)]), ..Default::default() };
        let kept = Kept { etag: Some("\"t\"".into()), fetched_at: 5, list: Some(list.clone()), ..Default::default() };
        write_kept(&dir, "42", &kept).unwrap();
        let back = read_kept(&dir, "42");
        assert_eq!(back.list, Some(list));
        assert_eq!(back.etag.as_deref(), Some("\"t\""));
        assert!(read_kept(&dir, "43").list.is_none());
        assert!(write_kept(&dir, "../x", &kept).is_err() && read_kept(&dir, "../x").list.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_decide_and_only_suspect_files_are_hashed() {
        let st = |size, ms| Some(Stamp { size, modified_ms: ms });
        assert_eq!(decide(10, None, st(10, 1), false, true), Step::Missing);
        assert_eq!(decide(10, st(11, 1), st(10, 1), false, false), Step::Differs);
        assert_eq!(decide(10, st(10, 1), st(10, 1), false, true), Step::Fine, "untouched since the install");
        assert_eq!(decide(10, st(10, 2), st(10, 1), false, true), Step::Hash, "an emulator file whose time moved");
        assert_eq!(decide(10, st(10, 2), st(10, 1), false, false), Step::Unverified, "a game file is not hashed on a quick check");
        assert_eq!(decide(10, st(10, 2), None, false, true), Step::Fine, "no stamp from the install");
        assert_eq!(decide(10, st(10, 1), st(10, 1), true, false), Step::Hash, "Check every file");
    }

    #[test]
    fn only_missing_files_and_broken_emulator_programs_raise_the_card() {
        assert_eq!(problem(Role::Game, "PlayGTAV.exe", true), Some(Problem::Missing));
        assert_eq!(problem(Role::Game, "data.pak", false), Some(Problem::Changed), "a mod");
        assert_eq!(problem(Role::EmulatorReplaced, "bin/steam_api64.dll", false), Some(Problem::Broken));
        assert_eq!(problem(Role::EmulatorAdded, "steam_settings/configs.user.ini", false), Some(Problem::Changed), "Kryoto writes the player's name there");
        assert_eq!(problem(Role::EmulatorAdded, "lib/libsteam_api.so", false), Some(Problem::Broken));
        assert_eq!(problem(Role::Kryoto, ".kryoto-release.json", true), None);
        assert_eq!(problem(Role::Kryoto, "RunMe.bat", true), Some(Problem::Missing));
        assert_eq!(problem(Role::Kryoto, "RunMe.bat", false), None);
        assert_eq!(problem(Role::Fix, "Game.exe", false), Some(Problem::Changed));
    }

    #[test]
    fn paths_match_ignoring_case() {
        let dir = scratch("case");
        write(&dir, "Bin/Steam_Api64.DLL", b"x");
        let mut f = Finder::new(&dir);
        let found = f.find("bin/steam_api64.dll").unwrap();
        assert_eq!(f.rel_of(&found).to_lowercase(), "bin/steam_api64.dll");
        assert!(f.find("bin/other.dll").is_none());
        assert!(f.find("bin").is_none(), "a folder is not a file");
        assert_eq!(key("Bin\\Steam_Api64.DLL"), "bin/steam_api64.dll");
        assert_eq!(archive_prefix(&["Game/Bin/Steam_API64.dll".into(), "Game/Game.exe".into(), "Game/readme.txt".into()], &["bin/steam_api64.dll".into(), "game.exe".into()]).as_deref(), Some("game/"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_check_names_what_is_missing_broken_and_changed() {
        let dir = scratch("check");
        write(&dir, "Game.exe", b"game");
        write(&dir, "data.pak", b"modded by the player");
        write(&dir, "bin/steam_api64.dll", b"quarantine stub");
        write(&dir, "steam_settings/configs.user.ini", b"[user]\nname=me");
        let list = FileList {
            release_id: "7".into(),
            files: Some(vec![
                file("Game.exe", b"game", Role::Game),
                file("PlayGTAV.exe", b"launcher", Role::Game),
                file("data.pak", b"vanilla", Role::Game),
                file("bin/steam_api64.dll", b"gbe", Role::EmulatorReplaced),
                file("steam_settings/configs.user.ini", b"[user]\nname=x", Role::EmulatorAdded),
                file("steam_settings/steam_appid.txt", b"480", Role::EmulatorAdded),
                file(".kryoto-release.json", b"{}", Role::Kryoto),
            ]),
            ..Default::default()
        };
        let stop = AtomicBool::new(false);
        let r = check_tree(&dir, &list, &CheckWith::default(), &stop, &mut |_, _, _, _| {});
        let names = |v: &[Finding]| v.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
        assert_eq!(names(&r.missing), ["PlayGTAV.exe", "steam_settings/steam_appid.txt"]);
        assert_eq!(names(&r.broken), ["bin/steam_api64.dll"], "a size that differs needs no hash");
        assert_eq!(r.changed, 2);
        assert_eq!(r.hashed, 0, "a quick check with no install stamps hashes nothing");
        assert!(r.alarm());
        // An emulator file the right size whose time moved since the install: hashed, and broken.
        write(&dir, "bin/steam_api64.dll", b"gbx");
        let with = CheckWith { stamps: HashMap::from([(key("bin/steam_api64.dll"), Stamp { size: 3, modified_ms: 1 })]), ..Default::default() };
        let r = check_tree(&dir, &list, &with, &stop, &mut |_, _, _, _| {});
        assert_eq!((names(&r.broken), r.hashed), (vec!["bin/steam_api64.dll".to_string()], 1));
        // Same size, other bytes: only a full check sees it.
        write(&dir, "Game.exe", b"gam3");
        let full = check_tree(&dir, &list, &CheckWith { full: true, ..Default::default() }, &stop, &mut |_, _, _, _| {});
        assert!(full.changed_sample.iter().any(|f| f.path == "Game.exe"));
        // After a switch, the release's emulator files are not expected.
        let switched = check_tree(&dir, &list, &CheckWith { skip_emulator: true, ..Default::default() }, &stop, &mut |_, _, _, _| {});
        assert!(switched.broken.is_empty() && switched.skipped == 3);
        stop.store(true, Ordering::SeqCst);
        assert!(check_tree(&dir, &list, &CheckWith::default(), &stop, &mut |_, _, _, _| {}).cancelled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn swap_list() -> FileList {
        let mut dll = file("bin/steam_api64.dll", b"gbe", Role::EmulatorReplaced);
        dll.original_sha256 = Some(sha(b"valve"));
        dll.backup = Some("bin/steam_api64.dll.kryoto".into());
        let mut other = file("steam_api.dll", b"gbe32", Role::EmulatorReplaced);
        other.original_sha256 = Some(sha(b"valve32"));
        other.backup = Some("steam_api.dll.kryoto".into());
        FileList {
            release_id: "9".into(),
            applied_by_forge: true,
            files: Some(vec![
                dll,
                other,
                file("bin/steam_api64.dll.kryoto", b"valve", Role::EmulatorAdded),
                file("steam_api.dll.kryoto", b"broken", Role::EmulatorAdded),
                file("bin/steam_settings/steam_appid.txt", b"480", Role::EmulatorAdded),
                file("Game.exe", b"steamless", Role::Fix),
                file("RunMe.bat", b"start", Role::Kryoto),
                file("data.pak", b"d", Role::Game),
            ]),
            ..Default::default()
        }
    }

    #[test]
    fn the_swap_plan_follows_the_roles() {
        let list = swap_list();
        let plan = swap_plan(&list, &mut |r| r.path == "bin/steam_api64.dll");
        assert_eq!(plan.restore.len(), 1);
        assert_eq!(plan.restore[0].backup, "bin/steam_api64.dll.kryoto");
        assert_eq!(plan.keep, ["steam_api.dll"], "no usable backup: left for Repair");
        assert_eq!(plan.remove, ["bin/steam_api64.dll.kryoto", "bin/steam_settings/steam_appid.txt"], "the unusable backup stays with what it backs up");
        let off = FileList { emulator_off: true, ..list };
        assert_eq!(swap_plan(&off, &mut |_| true), SwapPlan::default());
    }

    #[test]
    fn a_swap_is_undone_exactly() {
        let dir = scratch("swap");
        for (p, b) in [
            ("bin/steam_api64.dll", &b"gbe"[..]),
            ("bin/steam_api64.dll.kryoto", b"valve"),
            ("steam_api.dll", b"gbe32"),
            ("steam_api.dll.kryoto", b"broken"),
            ("bin/steam_settings/steam_appid.txt", b"480"),
            ("Game.exe", b"steamless"),
            ("RunMe.bat", b"start"),
            ("data.pak", b"d"),
        ] {
            write(&dir, p, b);
        }
        let (restored, removed) = clean_for_switch(&dir, &swap_list()).unwrap().unwrap();
        assert_eq!((restored, removed), (1, 2));
        assert_eq!(std::fs::read(dir.join("bin/steam_api64.dll")).unwrap(), b"valve");
        assert!(!dir.join("bin/steam_settings").exists(), "the emulator's empty folder goes");
        assert!(!dir.join("bin/steam_api64.dll.kryoto").exists());
        assert_eq!(std::fs::read(dir.join("steam_api.dll")).unwrap(), b"gbe32", "kept for Repair");
        assert!(dir.join("steam_api.dll.kryoto").exists());
        for keep in ["Game.exe", "RunMe.bat", "data.pak"] {
            assert!(dir.join(keep).is_file(), "{keep} is left alone");
        }
        assert_eq!(swapped_paths(&dir).len(), 3);
        assert_eq!(clean_for_switch(&dir, &swap_list()).unwrap(), None, "a second clean does nothing");
        // A new emulator went in over it.
        std::fs::write(dir.join("bin/steam_api64.dll"), b"rune").unwrap();
        assert!(unclean(&dir).unwrap());
        assert_eq!(std::fs::read(dir.join("bin/steam_api64.dll")).unwrap(), b"gbe");
        assert_eq!(std::fs::read(dir.join("bin/steam_api64.dll.kryoto")).unwrap(), b"valve");
        assert_eq!(std::fs::read(dir.join("bin/steam_settings/steam_appid.txt")).unwrap(), b"480");
        assert!(!has_swap(&dir) && !dir.join(SWAP_DIR).exists());
        assert!(!unclean(&dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_set_aside_copy_stops_the_undo() {
        let dir = scratch("swap-tamper");
        write(&dir, "bin/steam_api64.dll", b"gbe");
        write(&dir, "bin/steam_api64.dll.kryoto", b"valve");
        write(&dir, "bin/steam_settings/steam_appid.txt", b"480");
        write(&dir, "steam_api.dll", b"gbe32");
        write(&dir, "steam_api.dll.kryoto", b"broken");
        clean_for_switch(&dir, &swap_list()).unwrap();
        std::fs::write(dir.join(SWAP_DIR).join("0"), b"evil").unwrap();
        assert!(unclean(&dir).is_err());
        assert_eq!(std::fs::read(dir.join("bin/steam_api64.dll")).unwrap(), b"valve", "nothing touched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_comes_back_from_its_twin_proven_by_hash() {
        let dir = scratch("twin");
        let list = swap_list();
        write(&dir, "bin/steam_api64.dll.kryoto", b"valve");
        let want = ListedFile { path: "bin/original.dll".into(), size: 5, sha256: sha(b"valve"), ..Default::default() };
        let mut finder = Finder::new(&dir);
        let twin = local_twin(&mut finder, &list, &want, &[]).unwrap();
        place(&dir, &twin, &want.path, &want.sha256).unwrap();
        assert_eq!(std::fs::read(dir.join("bin/original.dll")).unwrap(), b"valve");
        assert!(dir.join("bin/steam_api64.dll.kryoto").is_file(), "the source stays");
        // A tool file of the same name and size but other bytes is not used.
        let tools_dir = dir.join("tools");
        write(&tools_dir, "steam_api64.dll", b"nope");
        let want = ListedFile { path: "bin/steam_api64.dll".into(), size: 4, sha256: sha(b"gbe!"), ..Default::default() };
        let tools = vec![("steam_api64.dll".to_string(), 4u64, tools_dir.join("steam_api64.dll"))];
        assert!(local_twin(&mut finder, &list, &want, &tools).is_none());
        write(&tools_dir, "steam_api64.dll", b"gbe!");
        assert!(local_twin(&mut finder, &list, &want, &tools).is_some());
        assert!(place(&dir, &tools_dir.join("steam_api64.dll"), "../escape.dll", &want.sha256).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_inside_an_archive_follow_its_folder() {
        let entries: Vec<String> = ["Hades/Hades.exe", "Hades/x64/steam_api64.dll", "Hades/readme.txt"].map(String::from).to_vec();
        let wanted: Vec<String> = ["x64/Steam_Api64.dll", "Hades.exe", "gone.dll"].map(String::from).to_vec();
        let prefix = archive_prefix(&entries, &wanted).unwrap();
        assert_eq!(prefix, "hades/");
        let names = archive_names(&entries, &prefix, &wanted);
        assert_eq!(names, vec![("x64/Steam_Api64.dll".to_string(), "Hades/x64/steam_api64.dll".to_string()), ("Hades.exe".to_string(), "Hades/Hades.exe".to_string())]);
        let flat: Vec<String> = ["a.exe".to_string()].to_vec();
        assert_eq!(archive_prefix(&flat, &["A.exe".to_string()]).as_deref(), Some(""));
    }

    /// Packs a small game with this PC's 7-Zip, then takes one file back out
    /// through the install's own 7-Zip (`extract_named`) and through the
    /// built-in reader. Skipped where there is no 7-Zip.
    #[test]
    fn one_file_comes_out_of_a_kept_archive() {
        let dir = scratch("archive");
        write(&dir, "src/Game/bin/steam_api64.dll", b"gbe");
        write(&dir, "src/Game/Game.exe", b"game");
        let archive = dir.join("Game - Kryoto.7z");
        let tools = [r"C:\Program Files\7-Zip\7z.exe", "7zz", "7z", "7za"];
        let packed = tools.iter().any(|t| {
            std::process::Command::new(t).current_dir(dir.join("src")).arg("a").arg(&archive).arg("Game").output().is_ok_and(|o| o.status.success())
        });
        if !packed {
            eprintln!("no 7-Zip here; skipped");
            return;
        }
        let entries = archive_entries(&archive).unwrap();
        let wanted = vec!["BIN/steam_api64.dll".to_string()];
        let prefix = archive_prefix(&entries, &wanted).unwrap();
        let names: Vec<String> = archive_names(&entries, &prefix, &wanted).into_iter().map(|(_, n)| n).collect();
        assert_eq!(names, ["Game/bin/steam_api64.dll"]);
        let out = dir.join("out");
        if crate::downloads::extract_named(&archive, &out, &names, &|_, _| {}, &AtomicU8::new(0)).unwrap() {
            assert_eq!(std::fs::read(out.join("Game/bin/steam_api64.dll")).unwrap(), b"gbe");
            assert!(!out.join("Game/Game.exe").exists(), "only the file asked for");
        }
        let out = dir.join("out2");
        extract_7z_named(&archive, &out, &names.iter().cloned().collect(), &AtomicU8::new(0), &|_, _| {}).unwrap();
        assert_eq!(std::fs::read(out.join("Game/bin/steam_api64.dll")).unwrap(), b"gbe");
        assert!(!out.join("Game/Game.exe").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_release_emulator_is_said_in_a_players_words() {
        let mut list = FileList { emulator: Some(Emu { name: "gbe_fork".into(), label: "gbe_fork".into(), variant: Some("experimental".into()), ..Default::default() }), ..Default::default() };
        assert_eq!(list.emulator_words().as_deref(), Some("gbe_fork (experimental)"));
        list.emulator = Some(Emu { name: "rune".into(), label: "RUNE".into(), variant: Some("rune".into()), ..Default::default() });
        assert_eq!(list.emulator_words().as_deref(), Some("RUNE"));
        list.emulator = None;
        assert_eq!(list.emulator_words(), None);
    }
}
