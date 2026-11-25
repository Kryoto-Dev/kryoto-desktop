//! Storage: Steam's storage manager, for Kryoto's library folders.
//!
//! A library folder is somewhere games get installed, one folder each. The
//! first one in Settings is where new downloads go; more can be added on other
//! drives. For each folder this reports the drive (size, free), what every game
//! in it takes, and it moves a game from one folder to another - a rename on
//! the same drive, a copy-then-delete with progress across drives.

use crate::library::{self, LibraryGame};
use crate::settings::{self, same_path};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// Total and free bytes on the drive holding `path` (or its nearest existing
/// parent, for a folder that is not made yet).
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    disk_space_of(&p)
}

#[cfg(windows)]
fn disk_space_of(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let (mut free_to_us, mut total, mut free) = (0u64, 0u64, 0u64);
    // SAFETY: a NUL-terminated path and three out-pointers to live u64s.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_to_us, &mut total, &mut free) };
    (ok != 0).then_some((total, free_to_us))
}

#[cfg(unix)]
// The `statvfs` fields are u64 on 64-bit Linux but narrower elsewhere
// (`fsblkcnt_t` is 32 bits on macOS and on 32-bit Linux), so the casts are
// needed on some targets and redundant on this one - where clippy flags them.
#[allow(clippy::unnecessary_cast)]
fn disk_space_of(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and an out-pointer to a zeroed statvfs.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let block = s.f_frsize as u64;
    Some((s.f_blocks as u64 * block, s.f_bavail as u64 * block))
}

/// The drive a path is on, as the player knows it: `C:` on Windows, the
/// mount point it lives under elsewhere (`/`, `/home`, `/mnt/games`).
fn drive_of(path: &Path) -> String {
    if let Some(Component::Prefix(p)) = path.components().next() {
        // A canonical path spells its drive \\?\C:, which is no name for a person.
        return p.as_os_str().to_string_lossy().trim_start_matches(r"\\?\").trim_end_matches('\\').to_uppercase();
    }
    let full = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    mounts
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        // /proc/mounts writes a space in a path as \040.
        .map(|m| m.replace("\\040", " "))
        .filter(|m| full.starts_with(m))
        .max_by_key(|m| m.len())
        .unwrap_or_else(|| "/".into())
}

/// The game's own folder inside the innermost library folder that holds it:
/// for `D:\Kryoto Games\Hades\bin\Hades.exe`'s root `D:\Kryoto Games\Hades\bin`,
/// that is `D:\Kryoto Games\Hades`, even with `D:\` a library folder too.
/// `None` when the game is not in one of them - it was added from somewhere
/// else, and is not ours to move or delete - or when that folder holds another
/// library folder, as `D:\Games\Steam` does with `D:\Games\Steam\Library` added.
///
/// KB-0375: the first folder under the library folder is not the game's when
/// the game sits deeper (`X:\Games\Foo` with `X:\` a library folder, `Games`
/// holding other folders too). The game's folder is exact, so neither a move,
/// a rename, an update nor Repair takes anything else along:
/// - that first folder, when it is provably a wrapper of this game alone: it
///   carries the game's install record or marker at its top, or it is named
///   for the game and holds nothing but the way down to the game;
/// - else the highest folder down to the game carrying the game's own install
///   record (`.kryoto-release.json`, listing the game's files), else its own
///   marker (`.kryoto-game.json`);
/// - else the game's install folder itself.
pub fn game_folder(folders: &[String], game: &LibraryGame) -> Option<PathBuf> {
    let dir = PathBuf::from(&game.install_dir).canonicalize().ok()?;
    game_folder_in(&canonical_roots(folders), &dir, game).map(|(_, f)| f)
}

/// `game_folder` on paths already canonical, with which of `roots` it is in.
fn game_folder_in(roots: &[Option<PathBuf>], dir: &Path, game: &LibraryGame) -> Option<(usize, PathBuf)> {
    let (at, first) = own_folder(roots, dir)?;
    exact_folder(roots[at].as_ref()?, first, dir, game).map(|f| (at, f))
}

/// `game_folder`'s rules from `first`, the first folder under `library`.
fn exact_folder(library: &Path, first: PathBuf, dir: &Path, game: &LibraryGame) -> Option<PathBuf> {
    let dir = dir.to_path_buf();
    if first == dir {
        return Some(dir);
    }
    // From the game's folder up to the first folder under the library folder.
    let chain: Vec<&Path> = dir.ancestors().take_while(|a| a.starts_with(&first)).collect();
    if carries_own(game, &first, &dir) || crate::safe_delete::named_for(game, &first, library) && only_way_down(game, &first, &dir) {
        return Some(first);
    }
    let by_record = chain.iter().rev().find(|a| own_record(a, &dir));
    let by_marker = || chain.iter().rev().find(|a| own_marker(game, a));
    Some(by_record.or_else(by_marker).map_or(dir.clone(), |a| a.to_path_buf()))
}

/// `folder` carries this game's install record or marker at its top.
fn carries_own(game: &LibraryGame, folder: &Path, dir: &Path) -> bool {
    own_record(folder, dir) || own_marker(game, folder)
}

/// `folder` carries the game's install record: the installer wrote it where
/// the game is (`dir`), and one above lists the files down there.
fn own_record(folder: &Path, dir: &Path) -> bool {
    use crate::community_offer::{read_json, ReleaseRecord, RELEASE_RECORD};
    let Some(record) = read_json::<ReleaseRecord>(&folder.join(RELEASE_RECORD)).filter(|r| !r.files.is_empty()) else { return false };
    if folder == dir {
        return true;
    }
    let Ok(rel) = dir.strip_prefix(folder) else { return false };
    let down = format!("{}/", crate::safe_delete::key(&rel.to_string_lossy()));
    record.files.keys().any(|f| crate::safe_delete::key(f).starts_with(&down))
}

/// `folder` carries the marker Kryoto wrote for this game.
fn own_marker(game: &LibraryGame, folder: &Path) -> bool {
    crate::library::read_marker(folder).is_some_and(|m| crate::safe_delete::same_game(&m, game))
}

/// Every folder from `top` down to `dir` holds nothing but the way down, or
/// files and folders the game's install record lists.
fn only_way_down(game: &LibraryGame, top: &Path, dir: &Path) -> bool {
    use crate::community_offer::{read_json, ReleaseRecord, RELEASE_RECORD};
    let levels: Vec<&Path> = dir.ancestors().skip(1).take_while(|a| a.starts_with(top)).collect();
    levels.iter().all(|level| {
        let Ok(read) = std::fs::read_dir(level) else { return false };
        // A record above lists this level's other entries by their path from it.
        let records: Vec<(PathBuf, Vec<String>)> = level
            .ancestors()
            .take_while(|a| a.starts_with(top))
            .filter(|a| own_record(a, dir))
            .filter_map(|a| read_json::<ReleaseRecord>(&a.join(RELEASE_RECORD)).map(|r| (a.to_path_buf(), r.files.keys().map(|k| crate::safe_delete::key(k)).collect())))
            .collect();
        read.flatten().all(|e| {
            let path = e.path();
            if dir.starts_with(&path) {
                return true;
            }
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if (name == crate::library::MARKER && own_marker(game, level)) || (name == RELEASE_RECORD && own_record(level, dir)) {
                return true;
            }
            records.iter().any(|(at, files)| {
                let Ok(rel) = path.strip_prefix(at) else { return false };
                let rel = crate::safe_delete::key(&rel.to_string_lossy());
                let down = format!("{rel}/");
                files.iter().any(|f| *f == rel || f.starts_with(&down))
            })
        })
    })
}

/// The game's folder is its own by `game_folder`'s rules, for a folder found
/// some other way (`outside_root` climbing out of `bin`): the game's folder
/// itself, one carrying its record or marker, or one holding nothing but the
/// way down to it.
fn is_own_top(game: &LibraryGame, top: &Path, dir: &Path) -> bool {
    top == dir || carries_own(game, top, dir) || only_way_down(game, top, dir)
}

/// The first folder under the innermost library folder that holds the
/// canonical `dir`, with which of `roots` that is: where `game_folder` starts.
fn own_folder(roots: &[Option<PathBuf>], dir: &Path) -> Option<(usize, PathBuf)> {
    let at = innermost_root(roots, dir)?;
    let root = roots[at].as_ref()?;
    let Some(Component::Normal(name)) = dir.strip_prefix(root).ok()?.components().next() else {
        return None;
    };
    let folder = root.join(name);
    if roots.iter().flatten().any(|r| r.starts_with(&folder)) {
        return None;
    }
    Some((at, folder))
}

/// `own_folder` as the uninstall used it up to 0.3.12, for safe_delete's test
/// that reproduces KB-0375.
#[cfg(test)]
pub(crate) fn own_folder_for_tests(roots: &[Option<PathBuf>], dir: &Path) -> Option<(usize, PathBuf)> {
    own_folder(roots, dir)
}

/// The library folders as the disk spells them, in the same order: `None` for
/// one that is not there, such as a drive that is unplugged.
fn canonical_roots(folders: &[String]) -> Vec<Option<PathBuf>> {
    folders.iter().map(|f| PathBuf::from(f).canonicalize().ok()).collect()
}

/// Which of `roots` the canonical `dir` is in: the innermost, so with
/// `D:\Games` and `D:\Games\Kryoto` both library folders a game in the second
/// is in the second, whichever comes first.
fn innermost_root(roots: &[Option<PathBuf>], dir: &Path) -> Option<usize> {
    roots
        .iter()
        .enumerate()
        .filter_map(|(at, r)| r.as_ref().filter(|r| dir.starts_with(r)).map(|r| (at, r.components().count())))
        .max_by_key(|&(_, depth)| depth)
        .map(|(at, _)| at)
}

/// How many games Settings > Storage lists under `roots[at]`: those in their
/// own folder in it, and not in a library folder inside it.
fn games_in(roots: &[Option<PathBuf>], at: usize, games: &[LibraryGame]) -> usize {
    games
        .iter()
        .filter_map(|g| PathBuf::from(&g.install_dir).canonicalize().ok())
        .filter(|dir| own_folder(roots, dir).is_some_and(|(i, _)| i == at))
        .count()
}

/// Another game in the library is also in `folder`: deleting or moving it
/// would take that game along. `leaving` are the games going with it.
pub fn holds_other_games(games: &[LibraryGame], leaving: &[&str], folder: &Path) -> bool {
    holds_other_games_in(games, &game_dirs(games), leaving, folder)
}

/// Each game's folder as the disk spells it, in the library's order: `None`
/// for one with no folder or one that is not there.
fn game_dirs(games: &[LibraryGame]) -> Vec<Option<PathBuf>> {
    games.iter().map(|g| if g.install_dir.is_empty() { None } else { PathBuf::from(&g.install_dir).canonicalize().ok() }).collect()
}

/// `holds_other_games` with each game's folder from `game_dirs`.
fn holds_other_games_in(games: &[LibraryGame], dirs: &[Option<PathBuf>], leaving: &[&str], folder: &Path) -> bool {
    games.iter().zip(dirs).any(|(g, dir)| !leaving.contains(&g.id.as_str()) && dir.as_ref().is_some_and(|dir| dir.starts_with(folder)))
}

/// The player's home folder, canonical.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).and_then(|h| h.canonicalize().ok())
}

const NOT_OWN: &str = "Its folder holds more than this game, so Kryoto does not move it. Move it yourself, then point Properties at the new place.";

/// The folder of a game added from somewhere else (brought over from Hydra,
/// or picked in Add a game): its install folder, out of the subfolders
/// engines put exes in (`bin`, `x64`, `Binaries\Win64`). Refused when that
/// folder is not one game's own: a drive, the player's home or one of its
/// usual folders, a library folder, or a folder another game in the library
/// is also in. Moving it moves exactly that folder.
pub fn outside_root(games: &[LibraryGame], folders: &[String], game: &LibraryGame) -> Result<PathBuf, String> {
    outside_root_in(games, &game_dirs(games), &canonical_roots(folders), home_dir().as_deref(), game)
}

/// `outside_root` on what is worked out already: each game's folder from
/// `game_dirs`, the library folders from `canonical_roots`, and the home folder.
fn outside_root_in(games: &[LibraryGame], dirs: &[Option<PathBuf>], roots: &[Option<PathBuf>], home: Option<&Path>, game: &LibraryGame) -> Result<PathBuf, String> {
    const INNER: &[&str] = &["bin", "bin64", "bin32", "x64", "x86", "win64", "win32", "binaries", "retail", "_retail_", "release"];
    let dir = PathBuf::from(&game.install_dir).canonicalize().map_err(|_| "Its folder is not there any more.".to_string())?;
    let mut root = dir.clone();
    while root.file_name().is_some_and(|n| INNER.contains(&n.to_string_lossy().to_ascii_lowercase().as_str())) {
        match root.parent() {
            Some(p) if p.parent().is_some() => root = p.to_path_buf(),
            _ => break,
        }
    }
    let not_own = || Err(NOT_OWN.to_string());
    // A drive (C:\) or one level under it (C:\Games) is a place, not a game.
    if root.components().filter(|c| matches!(c, Component::Normal(_))).count() < 2 {
        return not_own();
    }
    if let Some(home) = home {
        let usual = ["", "Desktop", "Downloads", "Documents", "Games", "AppData", "AppData/Local", "AppData/Roaming", ".local/share"];
        if usual.iter().any(|u| root == home.join(u)) || !root.starts_with(home) && home.starts_with(&root) {
            return not_own();
        }
    }
    for f in roots.iter().flatten() {
        if root == *f || f.starts_with(&root) {
            return not_own();
        }
    }
    if holds_other_games_in(games, dirs, &[game.id.as_str()], &root) {
        return not_own();
    }
    // Out of `bin` only into a folder that is plainly the game's (KB-0375).
    if !is_own_top(game, &root, &dir) {
        return not_own();
    }
    Ok(root)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredGame {
    id: String,
    title: String,
    folder: String,
    last_played: Option<u64>,
    cover: Option<String>,
    nsfw: bool,
    /// Brought over from Hydra (whose own list still points at this folder).
    from_hydra: bool,
    /// For a game outside the library folders: why it cannot be moved in.
    blocked: Option<String>,
    /// The launcher that keeps it, for a game that lives there.
    launcher: Option<String>,
    version: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolder {
    path: String,
    drive: String,
    is_default: bool,
    exists: bool,
    total: u64,
    free: u64,
    games: Vec<StoredGame>,
    /// A whole drive or the home folder, added before Kryoto refused those.
    warning: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageOverview {
    folders: Vec<LibraryFolder>,
    /// Games added from elsewhere on the PC.
    elsewhere: Vec<StoredGame>,
}

/// Why a game another launcher keeps cannot move.
fn keeper(g: &LibraryGame) -> Option<String> {
    g.launcher.as_deref().map(|l| {
        let mut name = l.chars();
        let name: String = name.next().map(|c| c.to_uppercase().chain(name).collect()).unwrap_or_default();
        format!("{name} manages this game.")
    })
}

fn stored(g: &LibraryGame, folder: PathBuf, blocked: Option<String>) -> StoredGame {
    StoredGame {
        id: g.id.clone(),
        title: g.title.clone(),
        folder: dunce::simplified(&folder).to_string_lossy().into_owned(),
        last_played: g.last_played,
        cover: g.custom_cover.clone().or_else(|| g.cover.clone()),
        nsfw: g.nsfw,
        from_hydra: g.origin.as_deref().is_some_and(|o| o.starts_with("hydra:")),
        blocked,
        launcher: g.launcher.clone(),
        version: g.version.clone(),
    }
}

/// Forget library folders that are gone from the disk and hold no game: the
/// folder Kryoto made first, left behind when the default moved, sat in every
/// list beside the real one. One with games stays, whatever its state: a drive
/// that is unplugged comes back.
pub fn forget_gone_folders<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let mut s = settings::load(app);
    let gone: Vec<String> = s.library_folders.iter().filter(|f| !Path::new(f.as_str()).is_dir()).cloned().collect();
    if gone.is_empty() {
        return Ok(());
    }
    let games = library::load(app)?;
    let empty: Vec<String> = gone.into_iter().filter(|f| !games.iter().any(|g| path_is_in(&g.install_dir, f))).collect();
    if empty.is_empty() {
        return Ok(());
    }
    s.library_folders.retain(|f| !empty.iter().any(|e| same_path(e, f)));
    crate::logging::info("storage", &format!("forgot {} library folder(s) no longer on disk", empty.len()));
    settings::write(app, &s)
}

/// By the words alone: for folders that may not be there to ask.
fn path_is_in(path: &str, folder: &str) -> bool {
    let norm = |p: &str| {
        let p = p.trim().trim_end_matches(['\\', '/']).replace('\\', "/");
        if cfg!(windows) { p.to_lowercase() } else { p }
    };
    let (path, folder) = (norm(path), norm(folder));
    path == folder || path.starts_with(&format!("{folder}/"))
}

/// What is where, at once. Sizes are not in it: walking every game's folder
/// took the page tens of seconds on a hard disk, so the page asks for them one
/// game at a time afterwards (`game_disk_size`).
fn overview<R: Runtime>(app: &AppHandle<R>) -> Result<StorageOverview, String> {
    if !app.try_state::<Moving>().is_some_and(|moving| moving.0.load(Ordering::SeqCst)) {
        recover_move(app)?;
    }
    forget_gone_folders(app)?;
    let s = settings::load(app);
    let folders = settings::all_folders(&s);
    let games = library::load(app)?;
    let roots = canonical_roots(&folders);
    let mut out: Vec<LibraryFolder> = folders
        .iter()
        .map(|f| {
            let path = PathBuf::from(f);
            let (total, free) = disk_space(&path).unwrap_or((0, 0));
            LibraryFolder {
                path: f.clone(),
                drive: drive_of(&path),
                is_default: same_path(f, &s.library_dir),
                exists: path.is_dir(),
                total,
                free,
                games: Vec::new(),
                warning: wide_warning(&path),
            }
        })
        .collect();
    let mut elsewhere = Vec::new();
    // Each game's folder is looked up once: checking every game outside the
    // library folders against all the others asked the disk N x N times.
    let dirs = game_dirs(&games);
    let home = home_dir();
    for (g, dir) in games.iter().zip(&dirs) {
        // Another launcher's folder is that launcher's to manage and move.
        if let Some(who) = keeper(g) {
            elsewhere.push(stored(g, PathBuf::from(&g.install_dir), Some(who)));
            continue;
        }
        match dir.as_deref().and_then(|dir| game_folder_in(&roots, dir, g)) {
            // Listed under the innermost library folder that holds it, as game_folder finds it.
            Some((at, folder)) => out[at].games.push(stored(g, folder, None)),
            None => elsewhere.push(match outside_root_in(&games, &dirs, &roots, home.as_deref(), g) {
                Ok(root) => stored(g, root, None),
                Err(why) => stored(g, PathBuf::from(&g.install_dir), Some(why)),
            }),
        }
    }
    Ok(StorageOverview { folders: out, elsewhere })
}

#[tauri::command]
pub async fn storage_overview(app: AppHandle) -> Result<StorageOverview, String> {
    tauri::async_runtime::spawn_blocking(move || overview(&app)).await.map_err(|e| e.to_string())?
}

/// The library folders and their free space only, without sizing every game
/// (that walks whole drives): for the install prompt's folder choice.
#[tauri::command]
pub async fn storage_folders(app: AppHandle) -> Result<Vec<LibraryFolder>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _ = forget_gone_folders(&app);
        let s = settings::load(&app);
        settings::all_folders(&s)
            .iter()
            .map(|f| {
                let path = PathBuf::from(f);
                let (total, free) = disk_space(&path).unwrap_or((0, 0));
                LibraryFolder { path: f.clone(), drive: drive_of(&path), is_default: same_path(f, &s.library_dir), exists: path.is_dir(), total, free, games: Vec::new(), warning: wide_warning(&path) }
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// Why a folder makes a poor library folder: a whole drive or the home folder
/// holds much more than games (KB-0375). None when it is fine.
fn too_wide(path: &Path, home: Option<&Path>) -> Option<&'static str> {
    let full = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if full.parent().is_none() {
        return Some("a whole drive");
    }
    home.filter(|h| *h == full || h.starts_with(&full)).map(|_| "your home folder")
}

/// What Settings > Storage says under a library folder that is too wide.
fn wide_warning(path: &Path) -> Option<String> {
    too_wide(path, crate::safe_delete::home().as_deref()).map(|what| {
        format!("This is {what}. Uninstalling only ever deletes what Kryoto installed, but new games are safer in a folder of their own: add one inside it and make it the default.")
    })
}

/// A folder typed or pasted as well as picked (KB-0042): only a whole path,
/// and one without "..". The check for a drive or the home folder cannot
/// work that out while the folder is not there yet, so `/home/deck/New/..`
/// got the home folder in.
fn whole_path(path: &Path) -> Result<(), String> {
    let example = if cfg!(windows) { r"D:\Kryoto Games" } else { "/home/deck/Games" };
    if !path.is_absolute() {
        return Err(format!("Type the folder's whole path, such as {example}."));
    }
    if path.components().any(|c| c == Component::ParentDir) {
        return Err(format!("Type the folder's path without \"..\", such as {example}."));
    }
    Ok(())
}

/// Add a library folder. Made if it does not exist yet. Never a whole drive
/// or the home folder: a folder for games inside it instead.
#[tauri::command(async)]
pub fn storage_add_folder(app: AppHandle, path: String) -> Result<(), String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("Pick a folder.".into());
    }
    whole_path(Path::new(&path))?;
    if let Some(what) = too_wide(Path::new(&path), crate::safe_delete::home().as_deref()) {
        let inside = Path::new(&path).join("Kryoto Games");
        return Err(format!("{path} is {what}, which holds much more than games. Add a folder for games inside it instead, such as {}.", inside.display()));
    }
    std::fs::create_dir_all(&path).map_err(|e| format!("Cannot use {path}: {e}"))?;
    let mut s = settings::load(&app);
    if settings::all_folders(&s).iter().any(|f| settings::same_folder(f, &path)) {
        return Err("That folder is already a library folder.".into());
    }
    s.library_folders.push(path.clone());
    settings::write(&app, &s)?;
    crate::logging::info("storage", &format!("added library folder {path}"));
    Ok(())
}

/// Stop using a library folder. Refused while games are installed in it, and
/// for the default: those have to be moved or uninstalled first. The folder
/// itself stays on disk.
#[tauri::command(async)]
pub fn storage_remove_folder(app: AppHandle, path: String) -> Result<(), String> {
    let mut s = settings::load(&app);
    if same_path(&path, &s.library_dir) {
        return Err("New games install here. Make another folder the default first.".into());
    }
    // A game in a library folder inside this one stays where it is.
    let folders = settings::all_folders(&s);
    let inside = match folders.iter().position(|f| settings::same_folder(f, &path)) {
        Some(at) => games_in(&canonical_roots(&folders), at, &library::load(&app)?),
        None => 0,
    };
    if inside > 0 {
        return Err(format!(
            "{inside} game{} installed here. Move {} to another folder or uninstall {} first.",
            if inside == 1 { " is" } else { "s are" },
            if inside == 1 { "it" } else { "them" },
            if inside == 1 { "it" } else { "them" },
        ));
    }
    s.library_folders.retain(|f| !same_path(f, &path));
    settings::write(&app, &s)
}

/// Make a folder the one new games install into.
#[tauri::command(async)]
pub fn storage_set_default(app: AppHandle, path: String) -> Result<(), String> {
    let mut s = settings::load(&app);
    if !settings::all_folders(&s).iter().any(|f| same_path(f, &path)) {
        return Err("That is not one of your library folders.".into());
    }
    let old = std::mem::replace(&mut s.library_dir, path.clone());
    s.library_folders.retain(|f| !same_path(f, &path));
    if !s.library_folders.iter().any(|f| same_path(f, &old)) {
        s.library_folders.insert(0, old);
    }
    settings::write(&app, &s)
}

/// One move at a time; a second waits for the answer "already moving".
#[derive(Default)]
pub struct Moving(AtomicBool);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MoveProgress {
    id: String,
    copied: u64,
    total: u64,
    done: bool,
    error: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct MoveJournal {
    id: String,
    from: PathBuf,
    dest: PathBuf,
    old_dir: String,
    new_dir: String,
}

fn journal_file<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("storage-move.json"))
}

fn save_journal(path: &Path, journal: &MoveJournal) -> Result<(), String> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(journal).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}

/// Recover metadata before the downloader or Library can use an interrupted move.
/// Never delete either copy during recovery: a partial copy may contain user edits.
pub fn recover_move<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let path = journal_file(app)?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Could not read the interrupted move: {e}")),
    };
    let journal: MoveJournal = serde_json::from_str(&text).map_err(|e| format!("The move recovery record is damaged: {e}"))?;
    let games = library::load(app)?;
    // With the game gone from the library, or moved again since, there is
    // nothing left to put back: keep the record and both folders, but stop it
    // blocking Storage and every move for good (it survived reinstalls).
    let Some(game) = games.iter().find(|g| g.id == journal.id) else {
        return set_aside(&path, &journal, "its game is no longer in the library");
    };
    if !same_path(&game.install_dir, &journal.old_dir) && !same_path(&game.install_dir, &journal.new_dir) {
        return set_aside(&path, &journal, "the game's library entry changed since");
    }
    recover_paths(&journal, &game.install_dir)?;
    if journal.from.exists() && journal.dest.exists() {
        let extra = if same_path(&game.install_dir, &journal.new_dir) { &journal.from } else { &journal.dest };
        crate::logging::warn("storage", &format!("An interrupted move kept a second folder at {}. Inspect it before removing it.", extra.display()));
    }
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

/// Rename a recovery record that can no longer be acted on to `storage-move-<time>.kept.json`, deleting nothing.
fn set_aside(path: &Path, journal: &MoveJournal, why: &str) -> Result<(), String> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let kept = path.with_file_name(format!("storage-move-{stamp}.kept.json"));
    std::fs::rename(path, &kept).map_err(|e| format!("Could not set the interrupted move's record aside: {e}"))?;
    crate::logging::warn(
        "storage",
        &format!("An interrupted move was not finished because {why}. Nothing was deleted: look in {} and {}. Record kept at {}.", journal.from.display(), journal.dest.display(), kept.display()),
    );
    Ok(())
}

fn recover_paths(journal: &MoveJournal, install_dir: &str) -> Result<(), String> {
    if same_path(install_dir, &journal.new_dir) && !Path::new(&journal.new_dir).is_dir() {
        return Err("The moved install folder could not be found. Both folders and the recovery record have been kept.".into());
    }
    if same_path(install_dir, &journal.old_dir) && !journal.from.exists() {
        // A rename happened, but metadata did not commit. Restore the original.
        if !journal.dest.exists() { return Err("Neither folder for the interrupted move could be found. The recovery record has been kept.".into()); }
        std::fs::rename(&journal.dest, &journal.from).map_err(|e| format!("Could not restore the interrupted move; its files are at {}: {e}", journal.dest.display()))?;
    } else if !same_path(install_dir, &journal.old_dir) && !same_path(install_dir, &journal.new_dir) {
        return Err("The library entry changed during an interrupted move. Both folders and the recovery record have been kept.".into());
    }
    Ok(())
}

fn commit_move(journal: &MoveJournal, renamed: bool, save: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    if let Err(e) = save() {
        if renamed {
            if journal.from.exists() {
                return Err(format!("Could not save the move ({e}); the original path now exists. The game is at {} and its recovery record has been kept.", journal.dest.display()));
            }
            std::fs::rename(&journal.dest, &journal.from).map_err(|rollback| format!("Could not save the move ({e}) or restore its original folder ({rollback}). The game is at {} and its recovery record has been kept.", journal.dest.display()))?;
        }
        return Err(format!("Could not save the move. The original game folder has been kept: {e}"));
    }
    Ok(())
}

fn install_suffix(from: &Path, install: &Path) -> Result<PathBuf, String> {
    let install = install.canonicalize().map_err(|e| format!("Could not find the game's install folder: {e}"))?;
    install.strip_prefix(from).map(Path::to_path_buf).map_err(|_| "The install folder is outside the game folder.".into())
}

fn copy_tree(from: &Path, to: &Path, progress: &mut dyn FnMut(u64)) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        #[cfg(unix)]
        if kind.is_symlink() {
            // Linux games ship links (libraries, launch scripts); keep them links.
            std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, &target)?;
            continue;
        }
        if kind.is_dir() {
            copy_tree(&entry.path(), &target, progress)?;
        } else if kind.is_file() {
            let n = std::fs::copy(entry.path(), &target)?;
            progress(n);
        }
    }
    Ok(())
}

/// Move a game to another library folder. Its launch settings, playtime and
/// everything else in the Library move with it; only the folder changes.
#[tauri::command]
pub async fn storage_move(
    app: AppHandle,
    moving: State<'_, Moving>,
    running: State<'_, library::Running>,
    id: String,
    to: String,
) -> Result<(), String> {
    crate::required::refuse()?;
    if running.0.lock().map(|m| m.contains_key(&id)).unwrap_or(false) {
        return Err("Close the game first.".into());
    }
    if moving.0.swap(true, Ordering::SeqCst) {
        return Err("Another game is being moved. Wait for it to finish.".into());
    }
    let result = move_game(&app, &id, &to).await;
    moving.0.store(false, Ordering::SeqCst);
    if let Err(e) = &result {
        crate::logging::error("storage", &format!("moving {id}: {e}"));
        let _ = app.emit("storage-move", MoveProgress { id, copied: 0, total: 0, done: true, error: Some(e.clone()) });
    }
    result
}

async fn move_game<R: Runtime>(app: &AppHandle<R>, id: &str, to: &str) -> Result<(), String> {
    recover_move(app)?;
    let s = settings::load(app);
    let folders = settings::all_folders(&s);
    if !folders.iter().any(|f| same_path(f, to)) {
        return Err("That is not one of your library folders.".into());
    }
    let games = library::load(app)?;
    let game = games.iter().find(|g| g.id == id).cloned().ok_or("That game is no longer in the library.")?;
    if let Some(who) = keeper(&game) {
        return Err(who);
    }
    // A game added from elsewhere moves in with its whole own folder.
    let from = match game_folder(&folders, &game) {
        Some(from) if holds_other_games(&games, &[game.id.as_str()], &from) => return Err(NOT_OWN.into()),
        Some(from) => from,
        None => outside_root(&games, &folders, &game)?,
    };
    // Whatever found it, only a folder that is the game's own moves (KB-0375):
    // `guard` below says nothing else of the library's is in it, this says
    // nothing else at all is.
    match PathBuf::from(&game.install_dir).canonicalize() {
        Ok(dir) if is_own_top(&game, &from, &dir) => {}
        _ => return Err(NOT_OWN.into()),
    }
    let name = from.file_name().ok_or("That game folder has no name.")?.to_owned();
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let target_root = PathBuf::from(to).canonicalize().map_err(|e| e.to_string())?;
    let dest = target_root.join(&name);
    let rest = install_suffix(&from, Path::new(&game.install_dir))?;
    // The game's own folder itself, or a folder inside it; never a trailing separator.
    let new_dir = dunce::simplified(&if rest.as_os_str().is_empty() { dest.clone() } else { dest.join(rest) }).to_string_lossy().into_owned();
    if target_root.canonicalize().ok().as_deref() == from.parent() {
        return Err("It is already in that folder.".into());
    }
    if dest.exists() {
        return Err(format!("{} already exists. Rename or remove it first.", dunce::simplified(&dest).display()));
    }
    // The old folder is deleted once copied: only one that holds this game
    // alone (safe_delete.rs, the same checks as an uninstall; KB-0375).
    let known = crate::safe_delete::Known { games: &games, folders: &folders, home: crate::safe_delete::home(), release_list: None, downloaded: Vec::new() };
    let from_disk = dunce::canonicalize(&from).map_err(|e| e.to_string())?;
    let found = crate::safe_delete::guard(&known, &game, &[game.id.as_str()], &from_disk).map_err(|why| why.replace("so Kryoto deletes nothing in it.", "so Kryoto does not move it."))?;
    let total: u64 = found.iter().map(|(_, size)| size).sum();
    if let Some((_, free)) = disk_space(&target_root) {
        if free < total && drive_of(&target_root) != drive_of(&from) {
            return Err(format!(
                "Not enough space: {} needs {}, {} has {} free.",
                game.title,
                human(total),
                drive_of(&target_root),
                human(free)
            ));
        }
    }
    crate::logging::info("storage", &format!("moving {} from {} to {}", game.title, from.display(), dest.display()));

    let journal_path = journal_file(app)?;
    let journal = MoveJournal {
        id: id.into(), from: from.clone(), dest: dest.clone(), old_dir: game.install_dir.clone(), new_dir: new_dir.clone(),
    };
    save_journal(&journal_path, &journal)?;

    let progress_app = app.clone();
    let pid = id.to_string();
    let (from_c, dest_c) = (from.clone(), dest.clone());
    let renamed = tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
        std::fs::create_dir_all(dest_c.parent().unwrap_or(&dest_c)).map_err(|e| e.to_string())?;
        // Same drive: a rename, done at once.
        if std::fs::rename(&from_c, &dest_c).is_ok() {
            return Ok(true);
        }
        let mut copied = 0u64;
        let mut last = std::time::Instant::now();
        let copy = copy_tree(&from_c, &dest_c, &mut |n| {
            copied += n;
            if last.elapsed().as_millis() > 200 {
                last = std::time::Instant::now();
                let _ = progress_app.emit(
                    "storage-move",
                    MoveProgress { id: pid.clone(), copied, total, done: false, error: None },
                );
            }
        });
        if let Err(e) = copy {
            // Leave the original exactly as it was.
            let _ = std::fs::remove_dir_all(&dest_c);
            return Err(format!("Copying failed, nothing was moved: {e}"));
        }
        Ok(false)
    })
    .await
    .map_err(|e| e.to_string())??;

    commit_move(&journal, renamed, || library::update(app, |games| {
        let g = games.iter_mut().find(|g| g.id == id).ok_or("The game was removed while moving. Its folders have been kept.")?;
        if !same_path(&g.install_dir, &game.install_dir) { return Err("The install folder changed while moving. Its folders have been kept.".into()); }
        g.install_dir = new_dir.clone();
        Ok(())
    }))?;
    // Metadata now points at the verified complete destination. Only now is
    // removing the original safe; an interrupted cleanup keeps a usable game.
    let from_c = from.clone();
    let leftover = if renamed { None } else { tauri::async_runtime::spawn_blocking(move || -> Option<String> {
        // The copy is complete, so the game now lives at the new place whatever
        // happens next. A file in the old folder can still be held open for a
        // moment (antivirus scanning it, a launcher that has not quit: os error
        // 32), so the removal is retried; if it still fails, the move counts and
        // the old folder is left for the player to delete, rather than leaving
        // the library pointing at a copy it was about to stop using.
        let mut last = None;
        for wait in [0u64, 1, 3, 6] {
            std::thread::sleep(std::time::Duration::from_secs(wait));
            match std::fs::remove_dir_all(&from_c) {
                Ok(()) => return None,
                Err(e) if !from_c.exists() => {
                    let _ = e;
                    return None;
                }
                Err(e) => last = Some(e),
            }
        }
        Some(format!(
            "Moved. The old folder {} could not be removed ({}): something still has a file in it open. Delete it yourself once that is closed.",
            from_c.display(),
            last.map(|e| e.to_string()).unwrap_or_default(),
        ))
    })
    .await
    .map_err(|e| e.to_string())? };
    if let Some(note) = &leftover {
        crate::logging::warn("storage", &format!("moving {id}: {note}"));
    }

    std::fs::remove_file(journal_path).map_err(|e| e.to_string())?;
    let _ = app.emit("storage-move", MoveProgress { id: id.to_string(), copied: total, total, done: true, error: leftover });
    let _ = app.emit("library-changed", ());
    Ok(())
}

/// A size in the unit that reads best: "66848.2 GB" was a drive's worth of
/// terabytes, and a small add-on read "0 MB".
fn human(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b >= K * K * K * K {
        format!("{:.1} TB", b / (K * K * K * K))
    } else if b >= K * K * K {
        format!("{:.1} GB", b / (K * K * K))
    } else if b >= K * K {
        format!("{:.0} MB", b / (K * K))
    } else if b >= K {
        format!("{:.0} KB", b / K)
    } else {
        format!("{bytes} B")
    }
}

/// `human`, for the other modules' messages and logs.
pub fn human_size(bytes: u64) -> String {
    human(bytes)
}

/// Both folders are on one drive.
pub fn same_drive(a: &Path, b: &Path) -> bool {
    drive_of(a) == drive_of(b)
}

/// Before a download starts: room for what is left of the archive where it is
/// written, and for about as much again unpacked in the library. With a download
/// folder on another drive, each drive only needs its own share.
///
/// `unpacked` is the game's install size when kryo.to lists it; without it the
/// game is taken to be as big as its archive.
///
/// On one drive, `in_place` (HatchZip can unpack this one, hatchzip.rs) asks
/// for the game, not the game and its archive: the unpack eats the archive as
/// the game appears, so the drive holds the rest of the archive, what the game
/// adds beyond it, and a little extra. downloads.rs forces it when the drive is
/// short for both. A solid archive that gains nothing from it is checked again
/// before 7-Zip unpacks it, with the archive kept for Retry.
pub fn check_download_room(archive_dir: &Path, library: &Path, archive_left: u64, archive_total: u64, unpacked: Option<u64>, in_place: bool, what: &str) -> Result<(), String> {
    // A listed size under half the archive is a wrong listing, not a game.
    let listed = unpacked.filter(|&u| u >= archive_total / 2);
    let unpacked = listed.unwrap_or(archive_total);
    if same_drive(archive_dir, library) {
        let more = if in_place { unpacked.saturating_sub(archive_total) + in_place_extra(archive_total) } else { unpacked };
        let needed = archive_left + more;
        crate::logging::info(
            "download",
            &format!(
                "{what}: room on {}: {} free, needs {} (archive {} of which {} still to come, game {}{}, unpacked in place: {in_place})",
                drive_of(library),
                disk_space(library).map(|(_, f)| human(f)).unwrap_or_else(|| "?".into()),
                human(needed),
                human(archive_total),
                human(archive_left),
                human(unpacked),
                if listed.is_some() { " as kryo.to lists it" } else { ", taken as the archive size" },
            ),
        );
        // Said plainly when the download and the game must sit side by side:
        // a player with "500 GB free" read the bare total as a wrong reading
        // (KB-0329).
        return check_room(library, needed, what).map_err(|e| if in_place { e } else { side_by_side(library, needed, archive_total, unpacked, what) });
    }
    check_room(archive_dir, archive_left, what)?;
    check_room(library, unpacked, what)
}

/// What an unpack in place needs beyond the game itself: the 500 MB HatchZip
/// keeps free, a 1 GB margin, and 2% of the archive for the block being
/// unpacked while it is still on the disk (Forge's archives are many small
/// solid blocks: 82 in a 3.6 GB one at its default level).
fn in_place_extra(archive: u64) -> u64 {
    (3 << 29) + archive / 50
}

/// "Not enough space" for a game unpacked the usual way on the drive that
/// holds its download: 7-Zip writes the game beside the download, so the
/// drive needs room for both at once. Names the drive and both sizes.
fn side_by_side(folder: &Path, needed: u64, archive: u64, game: u64, what: &str) -> String {
    let free = disk_space(folder).map(|(_, f)| human(f)).unwrap_or_else(|| "less".into());
    format!(
        "Not enough space on {drive} for {what}: it is unpacked with 7-Zip (the standard unpacker), which needs the download ({}) and the game ({}) on {drive} at the same time, about {} in all, and {free} is free. Free some space or choose another folder, then press Retry.",
        human(archive),
        human(game),
        human(needed),
        drive = drive_of(folder),
    )
}

/// Before 7-Zip unpacks a download that is already on `folder`'s drive (the
/// usual way, after unpacking in place stood aside): room for the game beside
/// it. The download is kept, so Retry needs no new download.
pub fn check_room_beside(folder: &Path, game: u64, archive: u64, what: &str) -> Result<(), String> {
    check_room(folder, game, what).map_err(|_| {
        let free = disk_space(folder).map(|(_, f)| human(f)).unwrap_or_else(|| "less".into());
        format!(
            "Not enough space on {drive} for {what}: 7-Zip (the standard unpacker) writes the game ({}) beside its download ({}), which stays on {drive} until it is done, and {free} is free. The download is kept: free some space or choose another folder, then press Retry.",
            human(game),
            human(archive),
            drive = drive_of(folder),
        )
    })
}

/// Before a download lands: is there room for the archive and what it unpacks
/// to? `needed` is the caller's estimate. An error names both numbers.
pub fn check_room(folder: &Path, needed: u64, what: &str) -> Result<(), String> {
    let Some((_, free)) = disk_space(folder) else { return Ok(()) };
    if free >= needed {
        return Ok(());
    }
    Err(format!(
        "Not enough space on {} for {what}: it needs about {}, and {} is free. Free some up, or add a library folder on another drive in Settings > Storage.",
        drive_of(folder),
        human(needed),
        human(free),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_the_unit_that_fits() {
        assert_eq!(human(140), "140 B");
        assert_eq!(human(3 * 1024), "3 KB");
        assert_eq!(human(25 * 1024 * 1024), "25 MB");
        assert_eq!(human(3 * 1024 * 1024 * 1024 / 2), "1.5 GB");
        // 64 TiB claimed by a host: it read "65536.0 GB".
        assert_eq!(human(64 * 1024u64.pow(4)), "64.0 TB");
    }

    #[test]
    fn unpacking_in_place_needs_room_for_the_game_not_twice() {
        // KB-0319: 159 GB on 219 GB free was refused with HatchZip on.
        let dir = std::env::temp_dir();
        let Some((_, free)) = disk_space(&dir) else { return };
        if free < 8 << 30 {
            return;
        }
        let archive = free / 10 * 6;
        assert!(check_download_room(&dir, &dir, archive, archive, None, false, "a game").is_err());
        assert!(check_download_room(&dir, &dir, archive, archive, None, true, "a game").is_ok());
        assert!(check_download_room(&dir, &dir, free, free, None, true, "a game").is_err());
        // A game listed bigger than its archive needs that much more (Wukong:
        // 130 GB archive, 150 GB game), and half done needs only the rest.
        let bigger = Some(archive / 10 * 17);
        assert!(check_download_room(&dir, &dir, archive, archive, bigger, true, "a game").is_err());
        assert!(check_download_room(&dir, &dir, archive / 4, archive, Some(archive / 10 * 11), true, "a game").is_ok());
        // A listing far under the archive is ignored.
        assert!(check_download_room(&dir, &dir, archive, archive, Some(1), true, "a game").is_ok());
    }

    /// KB-0329: when the download and the game must sit side by side, the
    /// message says so, names the drive and both sizes, and says what 7-Zip is.
    #[test]
    fn room_for_both_says_both_and_the_drive() {
        let dir = std::env::temp_dir();
        let Some((_, free)) = disk_space(&dir) else { return };
        let archive = free / 10 * 6;
        let e = check_download_room(&dir, &dir, archive, archive, None, false, "a game").unwrap_err();
        assert!(e.starts_with(&format!("Not enough space on {}", drive_of(&dir))), "{e}");
        assert!(e.contains("7-Zip (the standard unpacker)") && e.contains("at the same time") && e.contains(&human(archive)), "{e}");
        let e = check_room_beside(&dir, free + (1 << 30), archive, "a game").unwrap_err();
        assert!(e.starts_with("Not enough space on") && e.contains("beside its download") && e.contains("The download is kept"), "{e}");
        assert!(check_room_beside(&dir, 1, archive, "a game").is_ok());
    }

    #[test]
    fn an_outside_game_moves_with_its_own_folder_only() {
        let base = std::env::temp_dir().join(format!("kryoto-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let hades = base.join("Hydra Games").join("Hades");
        std::fs::create_dir_all(hades.join("x64")).unwrap();
        std::fs::create_dir_all(base.join("Hydra Games").join("Loose")).unwrap();
        let lib = base.join("Kryoto Games");
        std::fs::create_dir_all(&lib).unwrap();
        let game = |id: &str, dir: &Path| LibraryGame { id: id.into(), title: id.into(), install_dir: dir.to_string_lossy().into(), ..Default::default() };
        let folders = vec![lib.to_string_lossy().into_owned()];
        // Out of the engine's x64 folder, to the game's own.
        let h = game("hades", &hades.join("x64"));
        let games = vec![h.clone()];
        assert_eq!(outside_root(&games, &folders, &h).unwrap(), hades.canonicalize().unwrap());
        // A folder that also holds another game is not this game's to move.
        let loose = game("loose", &base.join("Hydra Games"));
        let both = vec![h.clone(), loose.clone()];
        assert!(outside_root(&both, &folders, &loose).is_err());
        // Nor a library folder, nor a drive.
        assert!(outside_root(&games, &folders, &game("lib", &lib)).is_err());
        let drive = Path::new(&std::env::temp_dir()).ancestors().last().unwrap().to_path_buf();
        assert!(outside_root(&games, &folders, &game("drive", &drive)).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    fn fixture() -> (PathBuf, MoveJournal) {
        let root = std::env::temp_dir().join(format!("kryoto-move-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&root).unwrap();
        let from = root.join("old");
        let dest = root.join("new");
        std::fs::create_dir_all(from.join("bin")).unwrap();
        std::fs::write(from.join("bin/game.exe"), "fixture").unwrap();
        let from = from.canonicalize().unwrap();
        let journal = MoveJournal { id: "fixture".into(), old_dir: from.join("bin").to_string_lossy().into_owned(), new_dir: dest.join("bin").to_string_lossy().into_owned(), from, dest };
        (root, journal)
    }

    #[test]
    fn nested_launch_folder_is_captured_before_rename() {
        let (root, journal) = fixture();
        let suffix = install_suffix(&journal.from, Path::new(&journal.old_dir)).unwrap();
        assert_eq!(suffix, PathBuf::from("bin"));
        assert!(install_suffix(&journal.from, &root).is_err());
        std::fs::rename(&journal.from, &journal.dest).unwrap();
        assert!(journal.dest.join(suffix).join("game.exe").exists());
        assert!(install_suffix(&journal.from, Path::new(&journal.old_dir)).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_failure_rolls_back_rename_and_preserves_a_copied_original() {
        for renamed in [true, false] {
            let (root, journal) = fixture();
            if renamed { std::fs::rename(&journal.from, &journal.dest).unwrap(); }
            else { copy_tree(&journal.from, &journal.dest, &mut |_| {}).unwrap(); }
            assert!(commit_move(&journal, renamed, || Err("injected save failure".into())).is_err());
            assert!(journal.from.join("bin/game.exe").exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn successful_root_and_nested_copies_keep_the_original_until_metadata_is_saved() {
        for nested in [false, true] {
            let (root, mut journal) = fixture();
            if !nested {
                journal.old_dir = journal.from.to_string_lossy().into_owned();
                journal.new_dir = journal.dest.to_string_lossy().into_owned();
            }
            let suffix = install_suffix(&journal.from, Path::new(&journal.old_dir)).unwrap();
            copy_tree(&journal.from, &journal.dest, &mut |_| {}).unwrap();
            commit_move(&journal, false, || {
                assert!(journal.from.join("bin/game.exe").exists());
                assert!(journal.dest.join(&suffix).is_dir());
                assert_eq!(std::fs::read(journal.dest.join("bin/game.exe")).unwrap(), b"fixture");
                Ok(())
            }).unwrap();
            recover_paths(&journal, &journal.new_dir).unwrap();
            assert!(journal.from.exists() && journal.dest.exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn failed_rollback_and_missing_committed_folder_keep_recovery_evidence() {
        let (root, journal) = fixture();
        let record = root.join("move.json");
        save_journal(&record, &journal).unwrap();
        std::fs::rename(&journal.from, &journal.dest).unwrap();
        // An unrelated file at the old path prevents rollback; keep the game
        // and its journal rather than replacing the new file.
        std::fs::write(&journal.from, "unrelated file").unwrap();
        let error = commit_move(&journal, true, || Err("injected save failure".into())).unwrap_err();
        assert!(error.contains("recovery record has been kept"));
        assert!(record.exists() && journal.dest.join("bin/game.exe").exists());
        assert_eq!(std::fs::read_to_string(&journal.from).unwrap(), "unrelated file");
        std::fs::remove_file(&journal.from).unwrap();
        std::fs::rename(&journal.dest, &journal.from).unwrap();
        assert!(recover_paths(&journal, &journal.new_dir).is_err());
        assert!(record.exists() && journal.from.join("bin/game.exe").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recovery_restores_uncommitted_rename_and_keeps_committed_destination() {
        let (root, journal) = fixture();
        save_journal(&root.join("move.json"), &journal).unwrap();
        std::fs::rename(&journal.from, &journal.dest).unwrap();
        recover_paths(&journal, &journal.old_dir).unwrap();
        assert!(journal.from.join("bin/game.exe").exists());
        std::fs::rename(&journal.from, &journal.dest).unwrap();
        recover_paths(&journal, &journal.new_dir).unwrap();
        assert!(journal.dest.join("bin/game.exe").exists());
        assert!(recover_paths(&journal, "unrelated edit").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_record_nothing_can_act_on_is_set_aside_and_both_folders_stay() {
        let (root, journal) = fixture();
        let record = root.join("storage-move.json");
        save_journal(&record, &journal).unwrap();
        copy_tree(&journal.from, &journal.dest, &mut |_| {}).unwrap();
        set_aside(&record, &journal, "its game is no longer in the library").unwrap();
        assert!(!record.exists());
        let kept = std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok()).any(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("storage-move-") && name.ends_with(".kept.json")
        });
        assert!(kept);
        assert!(journal.from.join("bin/game.exe").exists() && journal.dest.join("bin/game.exe").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A library entry called `title` at `dir`.
    fn titled(title: &str, dir: &Path) -> LibraryGame {
        LibraryGame { id: title.to_lowercase(), title: title.into(), install_dir: dir.to_string_lossy().into_owned(), ..Default::default() }
    }

    /// Kryoto's install record at `at`, listing `files` (relative to it).
    fn record_at(at: &Path, files: &[&str]) {
        let files: std::collections::BTreeMap<String, kryoto_repair::community::Stamp> =
            files.iter().map(|f| (f.to_string(), kryoto_repair::community::Stamp { size: 1, modified_ms: 0 })).collect();
        let record = crate::community_offer::ReleaseRecord { version: Some("1.0".into()), source: None, at: 1, files, unpacked: true };
        std::fs::write(at.join(crate::community_offer::RELEASE_RECORD), serde_json::to_vec(&record).unwrap()).unwrap();
    }

    /// The marker Kryoto writes for `game` in its folder.
    fn marker_at(at: &Path, game: &LibraryGame) {
        std::fs::write(at.join(crate::library::MARKER), serde_json::to_vec(game).unwrap()).unwrap();
    }

    #[test]
    fn a_game_folder_is_the_first_folder_under_the_library() {
        let root = std::env::temp_dir().join("kryoto-storage-test");
        let _ = std::fs::remove_dir_all(&root);
        let deep = root.join("Hades").join("bin");
        std::fs::create_dir_all(&deep).unwrap();
        let folders = vec![root.to_string_lossy().into_owned()];
        // D:\Kryoto Games\Hades\bin: Hades is named for it and holds nothing else.
        let found = game_folder(&folders, &titled("Hades", &deep)).unwrap();
        assert_eq!(found, root.canonicalize().unwrap().join("Hades"));
        // The library folder itself is never a game's folder.
        assert!(game_folder(&folders, &titled("Hades", &root)).is_none());
        assert!(game_folder(&folders, &titled("Hades", &std::env::temp_dir())).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    /// KB-0375, the hole left after the first fix: library `D:\`, the game in
    /// `D:\Games\Foo`, `D:\Games` also holding unrelated folders with no
    /// marker. The first folder (`Games`) is not the game's: a move takes only
    /// the game, and an update goes into the game's own folder.
    #[test]
    fn a_game_in_a_container_of_unmarked_folders_is_only_its_own_folder() {
        let root = std::env::temp_dir().join(format!("kryoto-storage-container-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let games_dir = root.join("Games");
        let (foo, photos, notes) = (games_dir.join("Foo"), games_dir.join("Photos"), games_dir.join("notes.txt"));
        std::fs::create_dir_all(foo.join("bin")).unwrap();
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::write(photos.join("a.jpg"), b"x").unwrap();
        std::fs::write(&notes, b"x").unwrap();
        std::fs::write(foo.join("Foo.exe"), b"x").unwrap();
        let folders = vec![root.to_string_lossy().into_owned()];
        let canon = |p: &Path| p.canonicalize().unwrap();
        let game = titled("Foo", &foo);
        // No marker, no record: the game's install folder itself.
        assert_eq!(game_folder(&folders, &game), Some(canon(&foo)));
        // With its record (a download): the same, never Games.
        record_at(&foo, &["Foo.exe"]);
        assert_eq!(game_folder(&folders, &game), Some(canon(&foo)));
        // Its root a level down (Foo\bin) with the record at Foo listing it: Foo.
        let deeper = titled("Foo", &foo.join("bin"));
        record_at(&foo, &["Foo.exe", "bin/Foo.exe"]);
        assert_eq!(game_folder(&folders, &deeper), Some(canon(&foo)));
        // Only the marker, at Foo: Foo.
        std::fs::remove_file(foo.join(crate::community_offer::RELEASE_RECORD)).unwrap();
        marker_at(&foo, &deeper);
        assert_eq!(game_folder(&folders, &deeper), Some(canon(&foo)));
        // A marker of another game does not count as this one's.
        marker_at(&foo, &titled("Other", &foo));
        assert_eq!(game_folder(&folders, &deeper), Some(canon(&foo.join("bin"))));
        // Games is named for nothing, and even named for the game it holds more.
        let games_named = titled("Games", &foo);
        assert_eq!(game_folder(&folders, &games_named), Some(canon(&foo)));

        // The update's destination is the game's own folder.
        let s = crate::settings::Settings { library_dir: root.to_string_lossy().into_owned(), ..Default::default() };
        let entry = LibraryGame { slug: Some("foo".into()), ..game.clone() };
        let item = crate::downloads::Download { slug: Some("foo".into()), ..Default::default() };
        let dest = crate::downloads::install_dest_for_tests(std::slice::from_ref(&entry), &s, &item, "Foo");
        assert_eq!(canon(&dest), canon(&foo));

        // A move takes only the game: the folder it would move is Foo, and
        // Games is never the game's own by these rules.
        assert!(is_own_top(&game, &canon(&foo), &canon(&foo)));
        assert!(!is_own_top(&game, &canon(&games_dir), &canon(&foo)));
        let _ = std::fs::remove_dir_all(root);
    }

    /// The first folder is a wrapper of this one game: it carries the game's
    /// record or marker at its top, or holds nothing but the way down (and
    /// files the record lists).
    #[test]
    fn a_wrapper_folder_of_one_game_is_its_folder() {
        let root = std::env::temp_dir().join(format!("kryoto-storage-wrapper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folders = vec![root.to_string_lossy().into_owned()];
        let canon = |p: &Path| p.canonicalize().unwrap();
        // A download: Library\Celeste\Celeste (the archive's own top folder).
        let outer = root.join("Celeste");
        let inner = outer.join("Celeste");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("Celeste.exe"), b"x").unwrap();
        let game = titled("Celeste", &inner);
        record_at(&inner, &["Celeste.exe"]);
        marker_at(&inner, &game);
        assert_eq!(game_folder(&folders, &game), Some(canon(&outer)));
        // Something of the player's beside it: no longer only the way down.
        std::fs::write(outer.join("mine.txt"), b"x").unwrap();
        assert_eq!(game_folder(&folders, &game), Some(canon(&inner)));
        // ...unless the game's record at the top lists it.
        record_at(&outer, &["mine.txt", "Celeste/Celeste.exe"]);
        assert_eq!(game_folder(&folders, &game), Some(canon(&outer)));
        // Its marker at the top, whatever the folder is called.
        let other = root.join("Stuff").join("Game");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(root.join("Stuff").join("readme.txt"), b"x").unwrap();
        let g = titled("Hollow", &other);
        assert_eq!(game_folder(&folders, &g), Some(canon(&other)));
        marker_at(&root.join("Stuff"), &g);
        assert_eq!(game_folder(&folders, &g), Some(canon(&root.join("Stuff"))));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A game right under the library folder is its own folder.
    #[test]
    fn a_game_directly_under_the_library_is_its_folder() {
        let root = std::env::temp_dir().join(format!("kryoto-storage-direct-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let foo = root.join("Foo");
        std::fs::create_dir_all(&foo).unwrap();
        std::fs::create_dir_all(root.join("Bar")).unwrap();
        let folders = vec![root.to_string_lossy().into_owned()];
        assert_eq!(game_folder(&folders, &titled("Foo", &foo)), Some(foo.canonicalize().unwrap()));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Out of `bin` only into a folder that is plainly the game's: a hand-added
    /// game at `X:\Stuff\bin` with the player's other files in `Stuff` is not
    /// moved, nor its update placed, as `Stuff`.
    #[test]
    fn a_game_outside_the_library_climbs_out_of_bin_only_into_its_own_folder() {
        let root = std::env::temp_dir().join(format!("kryoto-storage-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let stuff = root.join("Place").join("Stuff");
        let bin = stuff.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let game = titled("Stuff", &bin);
        let games = vec![game.clone()];
        let dirs = game_dirs(&games);
        let canon = |p: &Path| p.canonicalize().unwrap();
        assert_eq!(outside_root_in(&games, &dirs, &[], None, &game), Ok(canon(&stuff)));
        std::fs::write(stuff.join("holiday.mp4"), b"x").unwrap();
        assert_eq!(outside_root_in(&games, &dirs, &[], None, &game), Err(NOT_OWN.to_string()));
        marker_at(&stuff, &game);
        assert_eq!(outside_root_in(&games, &dirs, &[], None, &game), Ok(canon(&stuff)));
        let _ = std::fs::remove_dir_all(root);
    }

    /// With D:\Games and D:\Games\Kryoto both library folders, uninstalling a
    /// game in the second deleted all of D:\Games\Kryoto.
    #[test]
    fn a_game_folder_is_under_the_innermost_library_folder() {
        let root = std::env::temp_dir().join("kryoto-storage-nested-test");
        let _ = std::fs::remove_dir_all(&root);
        let inner = root.join("Kryoto");
        let hades = inner.join("Hades").join("bin");
        let other = root.join("Other");
        let steam_library = root.join("Steam").join("Library");
        let celeste = root.join("Steam").join("common").join("Celeste");
        for dir in [&hades, &other, &steam_library, &celeste] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let text = |p: &Path| p.to_string_lossy().into_owned();
        let canon = |p: &Path| p.canonicalize().unwrap();
        // Both orders: the default comes first, and it can be the outer or the inner one.
        for folders in [vec![text(&root), text(&inner), text(&steam_library)], vec![text(&steam_library), text(&inner), text(&root)]] {
            assert_eq!(game_folder(&folders, &titled("Hades", &hades)), Some(canon(&inner).join("Hades")));
            assert_eq!(game_folder(&folders, &titled("Other", &other)), Some(canon(&root).join("Other")));
            // Steam holds a library folder: it is not this game's to delete.
            assert_eq!(game_folder(&folders, &titled("Celeste", &celeste)), None);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    /// KB-0375: games moved into `Games\` under a drive-root library folder
    /// and found again a level down. Each one's folder is its own, not
    /// `Games`; a game whose root is a level down in its own folder (`bin`)
    /// still has the folder above.
    #[test]
    fn a_game_found_a_level_down_beside_other_games_is_its_own_folder() {
        let root = std::env::temp_dir().join(format!("kryoto-storage-beside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (foo, bar, hades) = (root.join("Games").join("Foo"), root.join("Games").join("Bar"), root.join("Hades").join("bin"));
        for d in [&foo, &bar, &hades] {
            std::fs::create_dir_all(d).unwrap();
        }
        let (foo_g, bar_g, hades_g) = (titled("Foo", &foo), titled("Bar", &bar), titled("Hades", &hades));
        marker_at(&foo, &foo_g);
        let folders = vec![root.to_string_lossy().into_owned()];
        let canon = |p: &Path| p.canonicalize().unwrap();
        // Alone in Games it is still Foo: Games is not named for it, and from
        // 0.4.0 a folder that only might be a wrapper is not taken for one.
        assert_eq!(game_folder(&folders, &foo_g), Some(canon(&foo)));
        marker_at(&bar, &bar_g);
        assert_eq!(game_folder(&folders, &foo_g), Some(canon(&foo)));
        assert_eq!(game_folder(&folders, &bar_g), Some(canon(&bar)));
        marker_at(&hades, &hades_g);
        assert_eq!(game_folder(&folders, &hades_g), Some(canon(&root.join("Hades"))));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_two_games_share_holds_other_games() {
        let root = std::env::temp_dir().join("kryoto-storage-shared-test");
        let _ = std::fs::remove_dir_all(&root);
        let (shared, solo) = (root.join("Shared"), root.join("Solo"));
        for dir in [shared.join("A"), shared.join("B").join("bin"), solo.clone()] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let game = |id: &str, dir: PathBuf| LibraryGame { id: id.into(), install_dir: dir.to_string_lossy().into_owned(), ..Default::default() };
        let games = vec![game("a", shared.join("A")), game("b", shared.join("B").join("bin")), game("solo", solo.clone())];
        let (shared, solo) = (shared.canonicalize().unwrap(), solo.canonicalize().unwrap());
        assert!(holds_other_games(&games, &["a"], &shared));
        assert!(!holds_other_games(&games, &["solo"], &solo));
        // Uninstalled in one go, the two do not keep each other's folder.
        assert!(!holds_other_games(&games, &["a", "b"], &shared));
        // Nor does a game its own folder.
        assert!(!holds_other_games(&games[..1], &["a"], &shared));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Settings > Storage listed a game in D:\Games\Kryoto under D:\Games, and
    /// would not let D:\Games go while D:\Games\Kryoto had games.
    #[test]
    fn a_game_counts_in_the_innermost_library_folder_only() {
        let root = std::env::temp_dir().join("kryoto-storage-innermost-test");
        let _ = std::fs::remove_dir_all(&root);
        let inner = root.join("Kryoto");
        let hades = inner.join("Hades");
        std::fs::create_dir_all(&hades).unwrap();
        let (root, inner, hades) = (root.canonicalize().unwrap(), inner.canonicalize().unwrap(), hades.canonicalize().unwrap());
        let games = vec![LibraryGame { id: "hades".into(), install_dir: hades.to_string_lossy().into_owned(), ..Default::default() }];
        for (roots, outer_at, inner_at) in [(vec![Some(root.clone()), Some(inner.clone())], 0, 1), (vec![Some(inner.clone()), Some(root.clone())], 1, 0)] {
            assert_eq!(innermost_root(&roots, &hades), Some(inner_at));
            assert_eq!(games_in(&roots, outer_at, &games), 0);
            assert_eq!(games_in(&roots, inner_at, &games), 1);
            // A game right in a library folder has no folder of its own there.
            let loose = [LibraryGame { id: "loose".into(), install_dir: inner.to_string_lossy().into_owned(), ..Default::default() }];
            assert_eq!(games_in(&roots, outer_at, &loose), 0);
            assert_eq!(games_in(&roots, inner_at, &loose), 0);
        }
        // A library folder that is not there holds nothing.
        assert_eq!(innermost_root(&[None, Some(root.clone())], &hades), Some(1));
        // A folder above every library folder is in none of them.
        assert_eq!(innermost_root(&[Some(inner.clone())], &root), None);
        let _ = std::fs::remove_dir_all(root);
    }

    /// `own_folder` and `outside_root_in`, on folders worked out beforehand,
    /// place each game as `game_folder` and `outside_root` do.
    #[test]
    fn outside_games_are_placed_from_folders_worked_out_beforehand() {
        let base = std::env::temp_dir().join("kryoto-storage-once-test");
        let _ = std::fs::remove_dir_all(&base);
        let (lib, hydra) = (base.join("Kryoto Games"), base.join("Hydra Games"));
        for dir in [lib.join("Hades").join("bin"), hydra.join("A").join("bin"), base.join("Solo").join("x64")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let game = |id: &str, dir: PathBuf| LibraryGame { id: id.into(), install_dir: dir.to_string_lossy().into_owned(), ..Default::default() };
        let games = vec![
            game("hades", lib.join("Hades").join("bin")),
            game("a", hydra.join("A").join("bin")),
            game("loose", hydra.clone()),
            game("solo", base.join("Solo").join("x64")),
            game("gone", base.join("Gone")),
        ];
        let dirs = game_dirs(&games);
        assert!(dirs[4].is_none());
        let roots = canonical_roots(&[base.join("Missing").to_string_lossy().into_owned(), lib.to_string_lossy().into_owned()]);
        let canon = |p: PathBuf| p.canonicalize().unwrap();
        assert_eq!(own_folder(&roots, dirs[0].as_ref().unwrap()), Some((1, canon(lib.join("Hades")))));
        assert_eq!(own_folder(&roots, dirs[1].as_ref().unwrap()), None);
        let place = |g: &LibraryGame| outside_root_in(&games, &dirs, &roots, None, g);
        assert_eq!(place(&games[1]), Ok(canon(hydra.join("A"))));
        assert_eq!(place(&games[3]), Ok(canon(base.join("Solo"))));
        // The folder game a is in too, and one that is gone.
        assert!(place(&games[2]).is_err());
        assert!(place(&games[4]).is_err());
        // The library folder itself.
        assert!(place(&game("lib", lib.clone())).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    /// KB-0375: a whole drive or the home folder is no library folder.
    #[test]
    fn a_drive_or_the_home_folder_is_too_wide() {
        let temp = dunce::canonicalize(std::env::temp_dir()).unwrap();
        let drive = temp.ancestors().last().unwrap().to_path_buf();
        assert_eq!(too_wide(&drive, None), Some("a whole drive"));
        assert_eq!(too_wide(&temp, Some(&temp)), Some("your home folder"));
        // A folder holding the home folder: on Linux the temp folder's parent
        // is `/`, a drive, so the home stands one level below the temp folder.
        let home = temp.join("kryoto-home");
        assert_eq!(too_wide(&temp, Some(&home)), Some("your home folder"));
        assert_eq!(too_wide(&temp.join("Kryoto Games"), Some(&temp)), None);
    }

    /// A typed folder is a whole path without "..": `New\..` is the folder it
    /// is in, which the check above cannot tell while New is not there.
    #[test]
    fn a_typed_folder_is_a_whole_path_without_dot_dot() {
        let temp = dunce::canonicalize(std::env::temp_dir()).unwrap();
        assert_eq!(whole_path(&temp.join("Kryoto Games")), Ok(()));
        assert!(whole_path(Path::new("Kryoto Games")).unwrap_err().contains("whole path"));
        assert!(whole_path(&temp.join("kryoto-not-there").join("..")).unwrap_err().contains("without \"..\""));
    }

    #[test]
    fn the_drive_has_a_size() {
        let (total, free) = disk_space(&std::env::temp_dir()).unwrap();
        assert!(total > 0 && free <= total);
    }
}
