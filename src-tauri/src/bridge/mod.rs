//! Bridge: Kryoto Desktop and other launchers. Games come in from them, and
//! go out to them, on one screen (`src/library/Bridge.tsx`).
//!
//! A launcher is one file in this folder implementing [`Provider`], listed in
//! [`PROVIDERS`]: where it keeps its data, whether it is open, and what games
//! it has; and, when games can go out to it, [`Export`]. Everything else
//! is here and shared: the review before anything is written, the merge rules,
//! remembered answers, reports. Adding Epic, GOG, Heroic or Playnite is a new
//! file and a line in that list.
//!
//! Two kinds of launcher. One that hands its games over (Hydra): each game is
//! matched on this PC against Kryoto's match index
//! (`/api/desktop/match-index`, downloaded whole and kept for a few hours, so
//! nothing about the player's games is sent anywhere to be looked up), a
//! matched game becomes a normal Kryoto entry, and one kryo.to does not list
//! keeps what the launcher had. The answer for each game is remembered
//! (`<provider>-links.json`), so it is never asked twice. And one that keeps
//! them (Steam, [`Provider::keeps_games`]): its games show in the Library and
//! start through it, stay the player's own, are never linked to kryo.to, and
//! their hours are that launcher's.
//!
//! A launcher's files are only read, unless the player sends games out.
//!
//! Later, when kryo.to lists a game the player has as their own (the same
//! Steam AppID, or a sure match by exe or title), `library_merge_offers`
//! names it and the player says yes or no once.

mod hydra;
mod steam;

use crate::library::{self, LibraryGame};
use kryoto_import::matcher::{CatalogGame, Candidate, How, Index, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const INDEX_FILE: &str = "match-index.json";
/// A remembered "this is my own game" answer.
const OWN: &str = "own";
/// How old the downloaded index may be before it is fetched again.
pub(crate) const INDEX_MAX_AGE: u64 = 6 * 3600;

/// Every launcher Bridge knows.
static PROVIDERS: &[&dyn Provider] = &[&hydra::Hydra, &steam::Steam];

/* ── a launcher ────────────────────────────────────────────────────── */

/// One game as a launcher has it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Incoming {
    /// The launcher's own key for it: what a remembered link is kept by.
    pub key: String,
    pub title: String,
    pub steam_appid: Option<String>,
    /// Absolute path of its exe, when the launcher has one.
    pub executable: Option<String>,
    /// Where it is installed, for a game with no exe on record.
    pub folder: Option<String>,
    pub playtime_seconds: u64,
    /// Unix seconds.
    pub last_played: Option<u64>,
    pub added_at: Option<u64>,
    pub launch_options: Option<String>,
    pub proton_path: Option<String>,
    pub cover: Option<String>,
    pub hero: Option<String>,
    pub logo: Option<String>,
    pub header: Option<String>,
    /// Pictures the player picked: web addresses or files on this PC.
    pub custom_cover: Option<String>,
    pub custom_hero: Option<String>,
    pub developer: Option<String>,
    pub year: Option<u32>,
}

pub(crate) enum ReadProblem {
    /// The launcher has its files open: it has to close first.
    Open,
    Other(String),
}

pub(crate) trait Provider: Sync {
    /// Short and stable: names its files, and starts each game's `origin`.
    fn id(&self) -> &'static str;
    /// As the player reads it.
    fn name(&self) -> &'static str;
    /// Where it keeps its data on this PC: `picked` when the player chose a
    /// folder and it is one, else where it normally is.
    fn locate(&self, picked: Option<&str>) -> Option<PathBuf>;
    /// It is open.
    fn running(&self) -> bool;
    /// Its games. Only ever reads.
    fn read(&self, dir: &Path) -> Result<Vec<Incoming>, ReadProblem>;
    /// Its games stay in it, and its files can be read while it runs. False
    /// for a launcher that hands its games over.
    fn keeps_games(&self) -> bool {
        false
    }
    /// For a launcher that keeps its games: the link Play opens for one.
    fn launch_url(&self, _game: &Incoming) -> Option<String> {
        None
    }
    /// Games just came over (their library ids and keys): anything more to bring.
    /// True when more is on its way to them (saves).
    fn brought_over(&self, _app: &AppHandle, _dir: &Path, _games: Vec<BroughtOver>) -> bool {
        false
    }
    /// Games can also go out to it.
    fn export(&self) -> Option<&dyn Export> {
        None
    }
}

/// A game that just came over.
pub(crate) struct BroughtOver {
    pub id: String,
    pub title: String,
    pub key: String,
}

/// A launcher games can be sent to. Runs on a worker thread; pictures are
/// already fetched by then.
pub(crate) trait Export: Sync {
    /// Who and what could be sent, and how it stands.
    fn list(&self, app: &AppHandle, dir: &Path, games: &[LibraryGame]) -> Result<ExportList, String>;
    /// Send these games to `user`'s library.
    fn send(&self, app: &AppHandle, dir: &Path, user: &str, games: Vec<Outgoing>) -> Result<Sent, String>;
    /// Take back everything sent.
    fn undo(&self, app: &AppHandle, dir: &Path) -> Result<usize, String>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportUser {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportGame {
    pub id: String,
    pub title: String,
    pub cover: Option<String>,
    pub nsfw: bool,
    /// Already there, from an earlier send.
    pub sent: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportList {
    pub users: Vec<ExportUser>,
    pub games: Vec<ExportGame>,
    /// Something the player should know before sending.
    pub note: Option<String>,
    /// How many games an undo would take back.
    pub undoable: usize,
}

/// A game on its way out, with its pictures' bytes.
pub(crate) struct Outgoing {
    pub game: LibraryGame,
    pub cover: Option<Vec<u8>>,
    pub hero: Option<Vec<u8>>,
    pub logo: Option<Vec<u8>>,
    pub header: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sent {
    pub added: usize,
    pub updated: usize,
    /// Pictures written.
    pub pictures: usize,
    /// The copy of the file from before, for the report.
    pub backup: Option<String>,
}

fn provider(id: &str) -> Result<&'static dyn Provider, String> {
    PROVIDERS.iter().copied().find(|p| p.id() == id).ok_or_else(|| "Bridge does not know that launcher.".to_string())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn data_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub(crate) fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Is a process with one of these names (lower case) running? `exe` gets a
/// second look at each process's path, for names too plain to go by alone.
pub(crate) fn process_running(names: &[&str], exe: impl Fn(&str, &str) -> bool) -> bool {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};
    let sys = System::new_with_specifics(RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet)));
    sys.processes().values().any(|p| {
        let name = p.name().to_string_lossy().to_ascii_lowercase();
        let path = p.exe().map(|e| e.to_string_lossy().to_ascii_lowercase().replace('\\', "/")).unwrap_or_default();
        names.contains(&name.as_str()) || exe(&name, &path)
    })
}

/* ── Kryoto's catalog, on this PC ──────────────────────────────────── */

#[derive(Default, Serialize, Deserialize)]
struct CachedIndex {
    at: u64,
    games: Vec<CatalogGame>,
}

/// Kryoto's match index: the copy on disk while it is younger than
/// `max_age`, else a fresh one from kryo.to, else the old copy (offline).
pub(crate) async fn match_index<R: Runtime>(app: &AppHandle<R>, max_age: u64) -> Option<Index> {
    let path = data_dir(app).ok()?.join(INDEX_FILE);
    let cached: Option<CachedIndex> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    if let Some(c) = cached.as_ref().filter(|c| now().saturating_sub(c.at) < max_age && !c.games.is_empty()) {
        return Some(Index::new(c.games.clone()));
    }
    match fetch_index(app).await {
        Ok(games) => {
            let fresh = CachedIndex { at: now(), games };
            if let Ok(json) = serde_json::to_vec(&fresh) {
                let _ = std::fs::write(&path, json);
            }
            Some(Index::new(fresh.games))
        }
        Err(e) => {
            crate::logging::warn("match-index", &e);
            cached.map(|c| Index::new(c.games))
        }
    }
}

async fn fetch_index<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<CatalogGame>, String> {
    #[derive(Deserialize)]
    struct Body {
        games: Vec<CatalogGame>,
    }
    let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(app));
    let client = reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client.get(format!("{endpoint}/api/desktop/match-index")).send().await.map_err(|e| format!("kryo.to could not be reached: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("kryo.to answered {} for the match index", res.status()));
    }
    Ok(res.json::<Body>().await.map_err(|e| e.to_string())?.games)
}

fn candidate<'a>(title: &'a str, appid: Option<&'a str>, exe: Option<&'a str>, developer: Option<&'a str>, year: Option<u32>) -> Candidate<'a> {
    Candidate { title, steam_appid: appid, executable: exe, developer, year }
}

/// A library game's full exe path, for matching by its file and folder name.
fn exe_path(g: &LibraryGame) -> Option<String> {
    (!g.executable.is_empty() && !g.install_dir.is_empty()).then(|| Path::new(&g.install_dir).join(&g.executable).to_string_lossy().into_owned())
}

/// The kryo.to game this unlinked library entry surely is, if any. A game that
/// stays in its own launcher is never one.
pub(crate) fn sure_match(index: &Index, g: &LibraryGame) -> Option<String> {
    if g.launcher.is_some() {
        return None;
    }
    let exe = exe_path(g);
    let appid = g.steam_appid.clone().or_else(|| crate::achievements::appid_of(Path::new(&g.install_dir)));
    match index.judge(&candidate(&g.title, appid.as_deref(), exe.as_deref(), g.developer.as_deref(), None)) {
        Verdict::Linked { slug, .. } => Some(slug),
        _ => None,
    }
}

/* ── Remembered answers ────────────────────────────────────────────── */

type Links = BTreeMap<String, String>;

fn links_file<R: Runtime>(app: &AppHandle<R>, provider: &str) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join(format!("{provider}-links.json")))
}

fn load_links<R: Runtime>(app: &AppHandle<R>, provider: &str) -> Links {
    links_file(app, provider).ok().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_links<R: Runtime>(app: &AppHandle<R>, provider: &str, links: &Links) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(links).map_err(|e| e.to_string())?;
    std::fs::write(links_file(app, provider)?, json).map_err(|e| e.to_string())
}

/* ── What Bridge can reach ─────────────────────────────────────────── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Launcher {
    pub id: &'static str,
    pub name: &'static str,
    /// Its data folder on this PC, when one was found.
    pub dir: Option<String>,
    /// It is open.
    pub running: bool,
    /// Its games stay in it, and it can be read while it runs.
    pub keeps: bool,
    /// Games can go out to it too.
    pub exports: bool,
}

fn launcher(p: &dyn Provider, picked: Option<&str>) -> Launcher {
    Launcher { id: p.id(), name: p.name(), dir: p.locate(picked).map(|d| d.to_string_lossy().into_owned()), running: p.running(), keeps: p.keeps_games(), exports: p.export().is_some() }
}

/// Every launcher, found here or not.
#[tauri::command(async)]
pub fn bridge_launchers() -> Vec<Launcher> {
    PROVIDERS.iter().map(|p| launcher(*p, None)).collect()
}

/// One launcher again: with a folder the player picked, or to see if it closed.
#[tauri::command(async)]
pub fn bridge_detect(launcher_id: String, dir: Option<String>) -> Result<Launcher, String> {
    Ok(launcher(provider(&launcher_id)?, dir.as_deref()))
}

/* ── Looking before anything is written ────────────────────────────── */

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub slug: String,
    pub title: String,
    pub developer: Option<String>,
    pub year: Option<u32>,
    pub nsfw: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// The launcher's key for it.
    pub key: String,
    pub title: String,
    pub playtime_seconds: u64,
    pub last_played: Option<u64>,
    pub executable: Option<String>,
    pub cover: Option<String>,
    /// `matched` (kryo.to has it), `own` (it does not), `ask` (maybe: the
    /// player picks), `missing` (not installed on this PC), `imported`
    /// (brought over before) or `stays` (remains in its own launcher).
    pub kind: &'static str,
    /// For `matched`, the game; for `ask`, the possibilities, best first.
    pub options: Vec<Choice>,
    /// What made the match sure.
    pub how: Option<How>,
    /// A game already in the library that this one joins (same folder, or
    /// the same kryo.to game), or for `stays`, has the same name: its title.
    pub joins: Option<String>,
    /// The answer came from an earlier import.
    pub remembered: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub dir: Option<String>,
    pub running: bool,
    /// Kryoto's catalog could be read (from kryo.to or the copy on this PC):
    /// without it, everything comes over as the player's own, to be linked later.
    pub catalog: bool,
    pub rows: Vec<Row>,
}

fn choice(index: &Index, slug: &str) -> Option<Choice> {
    index.get(slug).map(|g| Choice { slug: g.slug.clone(), title: g.title.clone(), developer: g.developer.clone(), year: g.year, nsfw: g.nsfw })
}

fn installed(h: &Incoming) -> bool {
    h.executable.as_deref().is_some_and(|e| Path::new(e).is_file()) || h.folder.as_deref().is_some_and(|d| Path::new(d).is_dir())
}

/// The library entry installed in the same place as the launcher's copy.
fn same_install<'a>(games: &'a [LibraryGame], h: &Incoming) -> Option<&'a LibraryGame> {
    let at = h.executable.as_deref().or(h.folder.as_deref())?;
    let norm = |p: &str| {
        let s = p.replace('\\', "/").trim_end_matches('/').to_string();
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    let at = norm(at);
    games.iter().filter(|g| g.install_dir.len() > 3).find(|g| {
        let dir = norm(&g.install_dir);
        at == dir || at.starts_with(&format!("{dir}/"))
    })
}

fn origin(provider: &str, key: &str) -> String {
    format!("{provider}:{key}")
}

/// A row for a game that stays in its own launcher: no catalog involved, so
/// nothing of it is compared with kryo.to. Pure, for the tests.
pub(crate) fn judge_kept(provider: &str, h: &Incoming, games: &[LibraryGame]) -> Row {
    let mut row = blank_row(h);
    let from = origin(provider, &h.key);
    if let Some(g) = games.iter().find(|g| g.origin.as_deref() == Some(from.as_str())) {
        row.kind = "imported";
        row.joins = Some(g.title.clone());
        return row;
    }
    row.kind = if installed(h) { "stays" } else { "missing" };
    let name = |t: &str| kryoto_import::matcher::squash(&kryoto_import::matcher::normalize(t));
    row.joins = games.iter().find(|g| name(&g.title) == name(&h.title)).map(|g| g.title.clone());
    row
}

fn blank_row(h: &Incoming) -> Row {
    Row {
        key: h.key.clone(),
        title: h.title.clone(),
        playtime_seconds: h.playtime_seconds,
        last_played: h.last_played,
        executable: h.executable.clone().or_else(|| h.folder.clone()),
        cover: h.cover.clone().or_else(|| h.custom_cover.clone().filter(|c| c.starts_with("https://"))),
        kind: "own",
        options: Vec::new(),
        how: None,
        joins: None,
        remembered: false,
    }
}

/// One game from a launcher that hands its games over: what it is, as the
/// review shows it. Pure, for the tests.
pub(crate) fn judge(provider: &str, h: &Incoming, index: Option<&Index>, links: &Links, games: &[LibraryGame]) -> Row {
    let mut row = blank_row(h);
    let from = origin(provider, &h.key);
    if let Some(g) = games.iter().find(|g| g.origin.as_deref() == Some(from.as_str())) {
        row.kind = "imported";
        row.joins = Some(g.title.clone());
        return row;
    }
    if !installed(h) {
        row.kind = "missing";
        return row;
    }
    let here = same_install(games, h);
    let remembered = links.get(&h.key).cloned();
    let verdict = match (&remembered, index) {
        (Some(v), _) if v == OWN => Verdict::Own,
        (Some(slug), Some(index)) if index.get(slug).is_some() => Verdict::Linked { slug: slug.clone(), how: How::Title },
        (_, Some(index)) => {
            // The copy already in the library says which game it is.
            if let Some(slug) = here.and_then(|g| g.slug.clone()).filter(|s| index.get(s).is_some()) {
                Verdict::Linked { slug, how: How::Folder }
            } else {
                index.judge(&candidate(&h.title, h.steam_appid.as_deref(), h.executable.as_deref(), h.developer.as_deref(), h.year))
            }
        }
        (_, None) => Verdict::Own,
    };
    row.remembered = remembered.is_some();
    match verdict {
        Verdict::Linked { slug, how } => {
            row.kind = "matched";
            row.how = (!row.remembered).then_some(how);
            row.options = index.and_then(|i| choice(i, &slug)).into_iter().collect();
            row.joins = games.iter().find(|g| g.slug.as_deref() == Some(slug.as_str())).or(here).map(|g| g.title.clone());
        }
        Verdict::Ask { options } => {
            row.kind = "ask";
            row.options = options.iter().filter_map(|s| index.and_then(|i| choice(i, s))).collect();
        }
        Verdict::Own => {
            row.joins = here.filter(|g| g.slug.is_none()).map(|g| g.title.clone());
        }
    }
    row
}

async fn read_games(p: &'static dyn Provider, dir: PathBuf) -> Result<Result<Vec<Incoming>, ReadProblem>, String> {
    tauri::async_runtime::spawn_blocking(move || p.read(&dir)).await.map_err(|e| e.to_string())
}

/// What importing would do, before anything is written.
#[tauri::command]
pub async fn bridge_scan(app: AppHandle, launcher_id: String, dir: Option<String>) -> Result<Scan, String> {
    let p = provider(&launcher_id)?;
    let Some(root) = p.locate(dir.as_deref()) else {
        return Ok(Scan { dir: None, running: p.running(), catalog: false, rows: Vec::new() });
    };
    let shown = Some(root.to_string_lossy().into_owned());
    let open = || Scan { dir: shown.clone(), running: true, catalog: false, rows: Vec::new() };
    if !p.keeps_games() && p.running() {
        return Ok(open());
    }
    let incoming = match read_games(p, root).await? {
        Ok(games) => games,
        Err(ReadProblem::Open) => return Ok(open()),
        Err(ReadProblem::Other(e)) => return Err(e),
    };
    let games = library::load(&app)?;
    let (rows, catalog): (Vec<Row>, bool) = if p.keeps_games() {
        (incoming.iter().map(|h| judge_kept(p.id(), h, &games)).collect(), true)
    } else {
        let index = match_index(&app, INDEX_MAX_AGE).await;
        let links = load_links(&app, p.id());
        (incoming.iter().map(|h| judge(p.id(), h, index.as_ref(), &links, &games)).collect(), index.is_some())
    };
    // These pictures are of games kryo.to may not list: straight from where
    // they are, not through kryo.to's image proxy.
    crate::art::load_directly(rows.iter().filter_map(|r| r.cover.clone()));
    Ok(Scan { dir: shown, running: false, catalog, rows })
}

/* ── Bringing them over ────────────────────────────────────────────── */

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub key: String,
    /// The kryo.to game it is; None to keep it as the player's own.
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    /// Linked to kryo.to games.
    pub linked: usize,
    /// Kept as the player's own games.
    pub own: usize,
    /// Joined to a game already in the library.
    pub joined: usize,
    /// Play time brought over.
    pub seconds: u64,
    /// Save versions are on, so the games' saves are being kept as versions
    /// now (the launcher's own backup too, when it left one).
    pub saves: bool,
}

/// A library entry from the launcher's copy, before any kryo.to link.
fn from_incoming(provider: &str, h: &Incoming, custom_cover: Option<String>, custom_hero: Option<String>) -> Option<LibraryGame> {
    let exe = PathBuf::from(h.executable.as_deref()?);
    let dir = exe.parent()?.to_string_lossy().into_owned();
    let name = exe.file_name()?.to_string_lossy().into_owned();
    let proton = h.proton_path.clone().filter(|p| !cfg!(windows) && Path::new(p).exists());
    Some(LibraryGame {
        title: h.title.clone(),
        install_dir: dir,
        executable: name,
        launch_options: h.launch_options.clone().unwrap_or_default(),
        compat_tool: proton,
        apply_overrides: true,
        cover: h.cover.clone(),
        hero: h.hero.clone(),
        logo: h.logo.clone(),
        custom_cover,
        custom_hero,
        developer: h.developer.clone(),
        playtime_seconds: h.playtime_seconds,
        last_played: h.last_played,
        steam_appid: h.steam_appid.clone(),
        origin: Some(origin(provider, &h.key)),
        // These hours have never been sent to kryo.to: they go when it is linked.
        carry_seconds: h.playtime_seconds,
        ..Default::default()
    })
}

/// An entry for a game that stays in its launcher: Play opens `link`. Its
/// hours are the launcher's, so nothing is owed to any account, and it is
/// the player's own for good.
fn kept_entry(provider: &str, h: &Incoming, link: String) -> LibraryGame {
    LibraryGame {
        title: h.title.clone(),
        install_dir: h.folder.clone().unwrap_or_default(),
        cover: h.cover.clone(),
        hero: h.hero.clone(),
        logo: h.logo.clone(),
        header: h.header.clone(),
        playtime_seconds: h.playtime_seconds,
        last_played: h.last_played,
        steam_appid: h.steam_appid.clone(),
        origin: Some(origin(provider, &h.key)),
        launcher: Some(provider.to_string()),
        launch_url: Some(link),
        ..Default::default()
    }
}

/// The launcher's copy into an entry already in the library: the higher play
/// time (never the sum: the same hours may be in both), the later last-played,
/// its launch options when the entry had none. Only what it adds is owed to
/// the account.
fn join(provider: &str, into: &mut LibraryGame, h: &Incoming) {
    let more = h.playtime_seconds.saturating_sub(into.playtime_seconds);
    into.playtime_seconds += more;
    into.carry_seconds += more;
    into.last_played = into.last_played.max(h.last_played);
    if into.launch_options.trim().is_empty() {
        into.launch_options = h.launch_options.clone().unwrap_or_default();
    }
    if into.steam_appid.is_none() && into.slug.is_none() {
        into.steam_appid = h.steam_appid.clone();
    }
    into.origin = into.origin.take().or_else(|| Some(origin(provider, &h.key)));
}

fn unique_id(games: &[LibraryGame], base: &str) -> String {
    let base: String = base.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let base = base.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let base = if base.is_empty() { "game".to_string() } else { base };
    let mut id = base.clone();
    let mut n = 2;
    while games.iter().any(|g| g.id == id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Bring the chosen games over. Each decision names a game and what it is;
/// games not named stay out. The launcher's files are only read.
#[tauri::command]
pub async fn bridge_import(app: AppHandle, launcher_id: String, dir: Option<String>, decisions: Vec<Decision>) -> Result<Imported, String> {
    let p = provider(&launcher_id)?;
    let root = p.locate(dir.as_deref()).ok_or_else(|| format!("No {} library found.", p.name()))?;
    let closed_first = || format!("Close {} first: it keeps its library open while it runs.", p.name());
    if !p.keeps_games() && p.running() {
        return Err(closed_first());
    }
    let incoming = match read_games(p, root.clone()).await? {
        Ok(games) => games,
        Err(ReadProblem::Open) => return Err(closed_first()),
        Err(ReadProblem::Other(e)) => return Err(e),
    };
    if p.keeps_games() {
        return import_kept(&app, p, incoming, decisions).await;
    }
    let index = match_index(&app, INDEX_MAX_AGE).await;
    let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(&app));
    let client = reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    // kryo.to's details for each game being linked, and the player's own
    // pictures copied into Kryoto, before the library is touched.
    let mut metas: BTreeMap<String, crate::downloads::CatalogMeta> = BTreeMap::new();
    let mut pictures: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();
    for d in &decisions {
        let Some(h) = incoming.iter().find(|h| h.key == d.key) else { continue };
        match &d.slug {
            Some(slug) => {
                if index.as_ref().is_none_or(|i| i.get(slug).is_none()) {
                    return Err(format!("{} is not on kryo.to any more. Scan again.", h.title));
                }
                if !metas.contains_key(slug) {
                    let meta = crate::downloads::fetch_meta(&client, &endpoint, slug).await.ok_or_else(|| format!("kryo.to did not answer for {}. Try again.", h.title))?;
                    metas.insert(slug.clone(), meta);
                }
            }
            None => {
                let local = |p: &Option<String>| p.clone().filter(|p| !p.starts_with("http") && Path::new(p).is_file());
                let copy = |p: Option<String>| p.and_then(|p| crate::art::art_import(app.clone(), p).ok());
                let web = |p: &Option<String>| p.clone().filter(|p| p.starts_with("https://"));
                let cover = copy(local(&h.custom_cover)).or_else(|| web(&h.custom_cover));
                let hero = copy(local(&h.custom_hero)).or_else(|| web(&h.custom_hero));
                pictures.insert(h.key.clone(), (cover, hero));
            }
        }
    }

    let mut links = load_links(&app, p.id());
    let mut out = Imported::default();
    let mut came: Vec<(String, String)> = Vec::new();
    let changed = library::update(&app, |games| {
        let mut changed: Vec<String> = Vec::new();
        for d in &decisions {
            let Some(h) = incoming.iter().find(|h| h.key == d.key) else { continue };
            let from = origin(p.id(), &h.key);
            if !installed(h) || games.iter().any(|g| g.origin.as_deref() == Some(from.as_str())) {
                continue;
            }
            let here = same_install(games, h).map(|g| g.id.clone());
            let id = match &d.slug {
                Some(slug) => {
                    let meta = &metas[slug];
                    let existing = games.iter().position(|g| g.slug.as_deref() == Some(slug.as_str())).or_else(|| here.as_ref().and_then(|id| games.iter().position(|g| &g.id == id)));
                    let id = if let Some(i) = existing {
                        let g = &mut games[i];
                        let before = g.playtime_seconds;
                        join(p.id(), g, h);
                        if g.slug.is_none() {
                            library::apply_meta(g, slug, meta);
                        } else {
                            library::queue_carry(g);
                        }
                        out.joined += 1;
                        out.seconds += g.playtime_seconds - before;
                        g.id.clone()
                    } else {
                        let Some(mut g) = from_incoming(p.id(), h, None, None) else { continue };
                        // kryo.to's art, not the launcher's: its pictures go.
                        g.cover = None;
                        g.hero = None;
                        g.logo = None;
                        g.header = None;
                        g.steam_appid = None;
                        library::apply_meta(&mut g, slug, meta);
                        g.id = unique_id(games, slug);
                        g.added_at = h.added_at.unwrap_or_else(now);
                        out.linked += 1;
                        out.seconds += g.playtime_seconds;
                        let id = g.id.clone();
                        games.push(g);
                        id
                    };
                    links.insert(h.key.clone(), slug.clone());
                    id
                }
                None => {
                    let id = if let Some(i) = here.as_ref().and_then(|id| games.iter().position(|g| &g.id == id)) {
                        let g = &mut games[i];
                        let before = g.playtime_seconds;
                        join(p.id(), g, h);
                        library::queue_carry(g);
                        out.joined += 1;
                        out.seconds += g.playtime_seconds - before;
                        g.id.clone()
                    } else {
                        let (cover, hero) = pictures.remove(&h.key).unwrap_or_default();
                        let Some(mut g) = from_incoming(p.id(), h, cover, hero) else { continue };
                        g.id = unique_id(games, &h.title);
                        g.added_at = h.added_at.unwrap_or_else(now);
                        out.own += 1;
                        out.seconds += g.playtime_seconds;
                        let id = g.id.clone();
                        games.push(g);
                        id
                    };
                    links.insert(h.key.clone(), OWN.to_string());
                    id
                }
            };
            came.push((id.clone(), h.key.clone()));
            changed.push(id);
        }
        Ok(changed)
    })?;
    let _ = save_links(&app, p.id(), &links);
    let games = library::load(&app).unwrap_or_default();
    for g in games.iter().filter(|g| changed.contains(&g.id)) {
        library::write_marker(g);
    }
    crate::logging::info("bridge", &format!("{}: imported {} linked, {} own, {} joined", p.id(), out.linked, out.own, out.joined));
    let _ = app.emit("library-changed", ());
    let brought = came
        .into_iter()
        .filter_map(|(id, key)| games.iter().find(|g| g.id == id).map(|g| BroughtOver { id, title: g.title.clone(), key }))
        .collect();
    out.saves = p.brought_over(&app, &root, brought);
    Ok(out)
}

/// A launcher that keeps its games: each chosen one becomes an entry that
/// starts through it, with its pictures from Steam's own store API. Nothing
/// goes to kryo.to, and nothing is matched.
async fn import_kept(app: &AppHandle, p: &'static dyn Provider, incoming: Vec<Incoming>, decisions: Vec<Decision>) -> Result<Imported, String> {
    let chosen: Vec<&Incoming> = decisions.iter().filter_map(|d| incoming.iter().find(|h| h.key == d.key)).collect();
    let ids: Vec<String> = chosen.iter().filter_map(|h| h.steam_appid.clone()).collect();
    let art = crate::steam_art::many(&ids).await;
    let mut out = Imported::default();
    let changed = library::update(app, |games| {
        let mut changed = Vec::new();
        for h in &chosen {
            let from = origin(p.id(), &h.key);
            let Some(link) = p.launch_url(h) else { continue };
            if !installed(h) || games.iter().any(|g| g.origin.as_deref() == Some(from.as_str())) {
                continue;
            }
            let mut g = kept_entry(p.id(), h, link);
            if let Some(a) = h.steam_appid.as_ref().and_then(|id| art.get(id)) {
                g.cover = a.capsule.clone().or(g.cover);
                g.hero = a.hero.clone().or(g.hero);
                g.logo = a.logo.clone().or(g.logo);
                g.header = a.header.clone().or(g.header);
            }
            g.id = unique_id(games, &h.title);
            g.added_at = now();
            out.own += 1;
            out.seconds += g.playtime_seconds;
            changed.push(g.id.clone());
            games.push(g);
        }
        Ok(changed)
    })?;
    crate::logging::info("bridge", &format!("{}: {} games added", p.id(), changed.len()));
    let _ = app.emit("library-changed", ());
    Ok(out)
}

/// Play for a game another launcher keeps. Only that launcher's own "start
/// this game" links are ever opened.
pub(crate) fn open_launcher_link(link: &str) -> Result<(), String> {
    if !launcher_link(link) {
        return Err("That is not a link to a game in its launcher.".into());
    }
    // Debug builds, against a fake Steam: noted, not opened.
    #[cfg(debug_assertions)]
    if let Some(dir) = env_path("KRYOTO_DEV_STEAM_DIR") {
        return std::fs::write(dir.join("launched.txt"), link).map_err(|e| e.to_string());
    }
    crate::open_with_system(link)
}

fn launcher_link(link: &str) -> bool {
    link.strip_prefix("steam://rungameid/").is_some_and(|id| !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()))
}

/* ── Sending them out ──────────────────────────────────────────────── */

fn exporter(id: &str) -> Result<(&'static dyn Provider, &'static dyn Export), String> {
    let p = provider(id)?;
    let e = p.export().ok_or_else(|| format!("Games cannot go out to {} yet.", p.name()))?;
    Ok((p, e))
}

/// What could be sent, before anything is written.
#[tauri::command(async)]
pub fn bridge_export_list(app: AppHandle, launcher_id: String, dir: Option<String>) -> Result<ExportList, String> {
    let (p, e) = exporter(&launcher_id)?;
    let root = p.locate(dir.as_deref()).ok_or_else(|| format!("{} was not found on this PC.", p.name()))?;
    let games = library::load(&app)?;
    e.list(&app, &root, &games)
}

/// Send these library games out, with their pictures. `adult_art`: the player
/// shows adult art; an adult game's pictures stay behind otherwise.
#[tauri::command]
pub async fn bridge_export(app: AppHandle, launcher_id: String, dir: Option<String>, user: String, ids: Vec<String>, adult_art: bool) -> Result<Sent, String> {
    let (p, e) = exporter(&launcher_id)?;
    let root = p.locate(dir.as_deref()).ok_or_else(|| format!("{} was not found on this PC.", p.name()))?;
    if p.running() {
        return Err(format!("Close {0} first: it would write over the change when it closes.", p.name()));
    }
    let games = library::load(&app)?;
    let mut going = Vec::new();
    for g in games.into_iter().filter(|g| ids.contains(&g.id) && g.launcher.is_none()) {
        let art = !g.nsfw || adult_art;
        let get = |picks: Vec<Option<String>>| {
            let app = app.clone();
            async move {
                for url in picks.into_iter().flatten() {
                    if let Some(bytes) = crate::art::bytes_of(&app, &url).await {
                        return Some(bytes);
                    }
                }
                None
            }
        };
        let (cover, hero, logo, header) = if art {
            (get(vec![g.custom_cover.clone(), g.cover.clone()]).await, get(vec![g.custom_hero.clone(), g.hero.clone()]).await, get(vec![g.logo.clone()]).await, get(vec![g.header.clone()]).await)
        } else {
            (None, None, None, None)
        };
        going.push(Outgoing { game: g, cover, hero, logo, header });
    }
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || e.send(&a, &root, &user, going)).await.map_err(|e| e.to_string())?
}

/// Take back everything sent to this launcher.
#[tauri::command(async)]
pub fn bridge_export_undo(app: AppHandle, launcher_id: String, dir: Option<String>) -> Result<usize, String> {
    let (p, e) = exporter(&launcher_id)?;
    let root = p.locate(dir.as_deref()).ok_or_else(|| format!("{} was not found on this PC.", p.name()))?;
    if p.running() {
        return Err(format!("Close {0} first: it would write over the change when it closes.", p.name()));
    }
    e.undo(&app, &root)
}

/* ── Later: kryo.to lists one of the player's own games ────────────── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeOffer {
    /// The library entry.
    pub id: String,
    pub title: String,
    /// The kryo.to game it is.
    pub slug: String,
    pub kryo_title: String,
}

/// The player's own games kryo.to now lists, by the same Steam AppID or a
/// sure match, except the ones they said no to. Pure, for the tests.
pub(crate) fn offers(index: &Index, games: &[LibraryGame]) -> Vec<MergeOffer> {
    games
        .iter()
        .filter(|g| g.slug.is_none() && !g.install_dir.is_empty())
        .filter_map(|g| {
            let slug = sure_match(index, g)?;
            // Never two entries for one kryo.to game.
            if g.merge_declined.contains(&slug) || games.iter().any(|o| o.slug.as_deref() == Some(slug.as_str())) {
                return None;
            }
            let kryo_title = index.get(&slug)?.title.clone();
            Some(MergeOffer { id: g.id.clone(), title: g.title.clone(), slug, kryo_title })
        })
        .collect()
}

#[tauri::command]
pub async fn library_merge_offers(app: AppHandle) -> Result<Vec<MergeOffer>, String> {
    let games = library::load(&app)?;
    if !games.iter().any(|g| g.slug.is_none() && g.launcher.is_none()) {
        return Ok(Vec::new());
    }
    let Some(index) = match_index(&app, INDEX_MAX_AGE).await else { return Ok(Vec::new()) };
    Ok(offers(&index, &games))
}

/// The player's answer. Yes links it: kryo.to's art and details, the same
/// folder and play time, and the hours counted while it was the player's own
/// go to the account once. No is remembered for this game and that page.
#[tauri::command]
pub async fn library_merge_answer(app: AppHandle, id: String, slug: String, yes: bool) -> Result<LibraryGame, String> {
    let meta = if yes {
        let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(&app));
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
        Some(crate::downloads::fetch_meta(&client, &endpoint, &slug).await.ok_or("kryo.to did not answer. Try again.")?)
    } else {
        None
    };
    let game = library::update(&app, |games| {
        if yes && games.iter().any(|g| g.slug.as_deref() == Some(slug.as_str())) {
            return Err("That game is already in your library.".to_string());
        }
        let g = games.iter_mut().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
        match &meta {
            Some(meta) => library::apply_meta(g, &slug, meta),
            None => {
                if !g.merge_declined.contains(&slug) {
                    g.merge_declined.push(slug.clone());
                }
            }
        }
        Ok(g.clone())
    })?;
    if yes {
        if let Some((who, key)) = game.origin.as_deref().and_then(|o| o.split_once(':')) {
            let mut links = load_links(&app, who);
            links.insert(key.to_string(), slug.clone());
            let _ = save_links(&app, who, &links);
        }
        library::write_marker(&game);
        let _ = app.emit("library-changed", ());
    }
    Ok(game)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> Index {
        let g = |slug: &str, title: &str, appid: Option<&str>, exes: &[&str]| CatalogGame {
            slug: slug.into(),
            title: title.into(),
            appid: appid.map(String::from),
            exes: exes.iter().map(|e| e.to_string()).collect(),
            ..Default::default()
        };
        Index::new(vec![
            g("hades", "Hades", Some("1145360"), &["hades.exe"]),
            g("stardew-valley", "Stardew Valley", Some("413150"), &[]),
            g("hollow-knight", "Hollow Knight", Some("367520"), &[]),
            g("lantern-rite", "Lantern Rite", Some("3999991"), &[]),
        ])
    }

    /// `test` keeps each test's folder its own: they run at once.
    fn temp_exe(test: &str, name: &str) -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("kryoto-bridge-{}-{test}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(format!("{name}.exe"));
        std::fs::write(&exe, b"MZ").unwrap();
        (dir, exe.to_string_lossy().into_owned())
    }

    fn hydra(key: &str, title: &str, exe: Option<&str>, appid: Option<&str>, seconds: u64) -> Incoming {
        Incoming { key: key.into(), title: title.into(), executable: exe.map(String::from), steam_appid: appid.map(String::from), playtime_seconds: seconds, ..Default::default() }
    }

    #[test]
    fn every_launcher_has_a_name_of_its_own() {
        let mut ids: Vec<_> = PROVIDERS.iter().map(|p| p.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PROVIDERS.len());
        assert!(provider("hydra").is_ok() && provider("steam").is_ok() && provider("../x").is_err());
        assert!(provider("steam").unwrap().export().is_some() && provider("hydra").unwrap().export().is_none());
    }

    #[test]
    fn rows_say_what_each_game_will_become() {
        let (dir, exe) = temp_exe("rows", "Hades");
        let mut links = Links::new();
        let games: Vec<LibraryGame> = Vec::new();
        let i = index();
        // By AppID.
        let r = judge("hydra", &hydra("steam:1145360", "Hades", Some(&exe), Some("1145360"), 60), Some(&i), &links, &games);
        assert_eq!((r.kind, r.how, r.options[0].slug.as_str()), ("matched", Some(How::SteamAppid), "hades"));
        // Close: asked.
        let r = judge("hydra", &hydra("custom:1", "Hollow Knight Godmaster", Some(&exe), None, 0), Some(&i), &links, &games);
        assert_eq!((r.kind, r.options.len()), ("ask", 1));
        // Not on Kryoto.
        let (mod_dir, mod_exe) = temp_exe("rows-mod", "SomeMod");
        let r = judge("hydra", &hydra("custom:2", "Some Mod", Some(&mod_exe), None, 0), Some(&i), &links, &games);
        assert_eq!(r.kind, "own");
        let _ = std::fs::remove_dir_all(mod_dir);
        // Hades' exe under a name that is not Hades: asked.
        let r = judge("hydra", &hydra("custom:4", "Some Mod", Some(&exe), None, 0), Some(&i), &links, &games);
        assert_eq!((r.kind, r.options[0].slug.as_str()), ("ask", "hades"));
        // Not installed here.
        let r = judge("hydra", &hydra("steam:504230", "Celeste", Some(r"Z:\nowhere\Celeste.exe"), None, 0), Some(&i), &links, &games);
        assert_eq!(r.kind, "missing");
        // An answer given before is not asked again.
        links.insert("custom:1".into(), "hollow-knight".into());
        links.insert("custom:3".into(), OWN.into());
        let r = judge("hydra", &hydra("custom:1", "Hollow Knight Godmaster", Some(&exe), None, 0), Some(&i), &links, &games);
        assert_eq!((r.kind, r.remembered), ("matched", true));
        let r = judge("hydra", &hydra("custom:3", "Hades", Some(&exe), Some("1145360"), 0), Some(&i), &links, &games);
        assert_eq!((r.kind, r.remembered), ("own", true));
        // Without the catalog, everything is the player's own for now.
        assert_eq!(judge("hydra", &hydra("steam:1145360", "Hades", Some(&exe), Some("1145360"), 0), None, &Links::new(), &games).kind, "own");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_same_install_joins_the_entry_already_there() {
        let (dir, exe) = temp_exe("join", "Hades");
        let mut kryoto = LibraryGame { id: "hades".into(), title: "Hades".into(), slug: Some("hades".into()), install_dir: dir.to_string_lossy().into(), executable: "Hades.exe".into(), playtime_seconds: 3_000, ..Default::default() };
        let h = hydra("steam:1145360", "Hades", Some(&exe), Some("1145360"), 5_000);
        let r = judge("hydra", &h, Some(&index()), &Links::new(), std::slice::from_ref(&kryoto));
        assert_eq!((r.kind, r.joins.as_deref()), ("matched", Some("Hades")));
        // The higher of the two, never the sum, and only Hydra's extra is owed.
        join("hydra", &mut kryoto, &h);
        assert_eq!((kryoto.playtime_seconds, kryoto.carry_seconds), (5_000, 2_000));
        let mut lower = LibraryGame { playtime_seconds: 9_000, ..kryoto.clone() };
        lower.carry_seconds = 0;
        join("hydra", &mut lower, &h);
        assert_eq!((lower.playtime_seconds, lower.carry_seconds), (9_000, 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn own_games_keep_hydras_data_and_owe_its_hours() {
        let (dir, exe) = temp_exe("own", "LanternRite");
        let mut h = hydra("custom:c7", "Lantern Rite Test", Some(&exe), Some("3999991"), 5_400);
        h.launch_options = Some("-windowed".into());
        h.hero = Some("https://cdn.example/hero.jpg".into());
        let g = from_incoming("hydra", &h, None, None).unwrap();
        assert_eq!((g.slug, g.executable.as_str(), g.launch_options.as_str()), (None, "LanternRite.exe", "-windowed"));
        assert_eq!((g.playtime_seconds, g.carry_seconds, g.steam_appid.as_deref()), (5_400, 5_400, Some("3999991")));
        assert_eq!((g.hero.as_deref(), g.origin.as_deref()), (Some("https://cdn.example/hero.jpg"), Some("hydra:custom:c7")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_steam_game_stays_steams_and_never_meets_kryoto() {
        let dir = std::env::temp_dir().join(format!("kryoto-bridge-{}-kept", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let h = Incoming { key: "1145360".into(), title: "Hades".into(), steam_appid: Some("1145360".into()), folder: Some(dir.to_string_lossy().into()), playtime_seconds: 5_400, ..Default::default() };
        let g = kept_entry("steam", &h, "steam://rungameid/1145360".into());
        assert_eq!((g.slug.as_deref(), g.launcher.as_deref(), g.launch_url.as_deref()), (None, Some("steam"), Some("steam://rungameid/1145360")));
        // The hours are Steam's: shown, and never owed to any account.
        assert_eq!((g.playtime_seconds, g.carry_seconds), (5_400, 0));
        // Kryoto lists Hades, but this entry is never offered a link to it.
        let i = index();
        assert_eq!(sure_match(&i, &g), None);
        assert!(offers(&i, std::slice::from_ref(&g)).is_empty());
        // Already in the library under that name: said so. Brought once only.
        let kryoto = LibraryGame { title: "HADES".into(), slug: Some("hades".into()), ..Default::default() };
        let r = judge_kept("steam", &h, std::slice::from_ref(&kryoto));
        assert_eq!((r.kind, r.joins.as_deref()), ("stays", Some("HADES")));
        assert_eq!(judge_kept("steam", &h, std::slice::from_ref(&g)).kind, "imported");
        let gone = Incoming { folder: Some(r"Z:\gone".into()), ..h };
        assert_eq!(judge_kept("steam", &gone, &[]).kind, "missing");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_a_launchers_own_game_links_open() {
        assert!(launcher_link("steam://rungameid/1145360"));
        for bad in ["steam://rungameid/", "steam://rungameid/1;calc", "steam://install/1", "https://evil.example", "file:///etc/passwd", "steam://rungameid/1/../x"] {
            assert!(!launcher_link(bad), "{bad}");
        }
    }

    #[test]
    fn later_offers_follow_the_appid_once_and_respect_no() {
        let mut own = LibraryGame { id: "lantern".into(), title: "Lantern Rite Test".into(), install_dir: r"D:\Games\Lantern".into(), executable: "LanternRite.exe".into(), steam_appid: Some("3999991".into()), ..Default::default() };
        let i = index();
        let o = offers(&i, std::slice::from_ref(&own));
        assert_eq!((o.len(), o[0].slug.as_str(), o[0].kryo_title.as_str()), (1, "lantern-rite", "Lantern Rite"));
        own.merge_declined.push("lantern-rite".into());
        assert!(offers(&i, std::slice::from_ref(&own)).is_empty(), "No is remembered");
        // A name nothing like any listing: never offered.
        let other = LibraryGame { id: "x".into(), title: "My Mod".into(), install_dir: r"D:\Mods\x".into(), executable: "mod.exe".into(), ..Default::default() };
        assert!(offers(&i, &[other]).is_empty());
    }

    #[test]
    fn linking_queues_the_unsent_hours_once() {
        let mut g = LibraryGame { title: "Lantern".into(), carry_seconds: 7_200, ..Default::default() };
        let meta = crate::downloads::CatalogMeta { title: "Lantern Rite".into(), cover: Some("https://kryo.to/c.jpg".into()), ..Default::default() };
        library::apply_meta(&mut g, "lantern-rite", &meta);
        assert_eq!((g.slug.as_deref(), g.title.as_str(), g.cover.as_deref()), (Some("lantern-rite"), "Lantern Rite", Some("https://kryo.to/c.jpg")));
        assert_eq!((g.carry_seconds, g.carry_sends.len(), g.carry_sends[0].seconds), (0, 1, 7_200));
        assert!(g.carry_sends[0].key.starts_with("carry-lantern-rite-"));
        // Nothing more to send: no second entry.
        library::queue_carry(&mut g);
        assert_eq!(g.carry_sends.len(), 1);
    }
}
