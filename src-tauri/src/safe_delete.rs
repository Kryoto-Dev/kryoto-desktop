//! Deleting a game's files from a player's disk: the one way Kryoto does it.
//!
//! KB-0375 (2026-11-22): uninstalling one game deleted a player's whole games
//! folder, 500 GB, for good. The folder to delete was worked out from where the
//! game sat under a library folder (the first folder below it), and only other
//! games still on the list, with folders still there, protected anything.
//!
//! Every delete of a game's files now goes through [`plan`], which says exactly
//! what goes and why it may, and [`run`], which deletes only that: uninstall
//! from the Library and from Settings > Storage, from the phone and from
//! kryo.to's remote (all `library::library_remove`).
//!
//! What may go:
//! - Only games Kryoto installed. One kept by another launcher, brought over
//!   from one (Steam, Hydra, Playnite, Lutris) or added by the player leaves
//!   the Library and its files stay, unless the folder carries Kryoto's own
//!   install record.
//! - With that install record (`.kryoto-release.json`: every file the
//!   installer unpacked, written since 0.3.0) or kryo.to's list of the
//!   release's files: those files and Kryoto's own, then folders left empty.
//!   Anything else stays.
//! - Without either (installs before 0.3.0): the folder that holds this game's
//!   own `.kryoto-game.json`, inside a library folder and named for the game,
//!   and only when something says Kryoto installed it (the archive's hash, a
//!   row in Downloads): a game added by hand before 0.4.0 carries a marker too.
//! - Never: a drive, the home folder or its usual folders, a system folder, a
//!   library folder or a folder holding one, a folder holding another game
//!   (its marker, a library entry even one whose folder is gone, another
//!   launcher's game), a link out of the folder, or a folder far bigger than
//!   the game. Saves the emulator keeps inside the folder stay.
//! - Into the Recycle Bin or the Trash when it can take them ([`bin_for`]); for
//!   good only when the player was told so before confirming.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::community_offer::{read_json, ReleaseRecord, RELEASE_RECORD, UPDATE_RECORD};
use crate::library::{read_marker, LibraryGame, MARKER};

/// What is known on this PC, for the checks.
pub struct Known<'a> {
    /// The whole library.
    pub games: &'a [LibraryGame],
    /// The library folders, as Settings > Storage lists them.
    pub folders: &'a [String],
    pub home: Option<PathBuf>,
    /// kryo.to's list of the installed release's files, when one is kept
    /// (release_files.rs): relative paths and sizes.
    pub release_list: Option<Vec<(String, u64)>>,
    /// The games Downloads lists as installed by Kryoto (game id, folder).
    pub downloaded: Vec<(String, String)>,
}

#[derive(Debug)]
pub enum Verdict {
    /// Kryoto did not install it: it leaves the Library, its files stay. Why.
    ListOnly(String),
    /// Not provably this game's alone: nothing is deleted. Why.
    Refused(String),
    Delete(Plan),
}

/// Exactly what an uninstall deletes.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// The game's folder, as the disk spells it.
    pub folder: PathBuf,
    /// The whole folder goes. Otherwise `files` go, then folders left empty.
    pub whole: bool,
    /// Every file that goes (all of them when `whole`).
    pub files: Vec<PathBuf>,
    pub bytes: u64,
    /// What stays in the folder: saves, and files Kryoto did not install.
    pub kept_files: u64,
    pub kept_bytes: u64,
    /// The library folder it is in: empty folders above `folder` are removed
    /// up to, not including, this one. None: only `folder` itself.
    pub stop_at: Option<PathBuf>,
}

/// The folder is far bigger than what Kryoto installed in it: more than this
/// many times over, and by more than [`SLACK`].
const BIGGER: u64 = 3;
const SLACK: u64 = 5 << 30;

/// Settle what an uninstall of `game` deletes. `leaving` are the games
/// uninstalled in the same go (Settings > Storage), which may share a folder.
pub fn plan(known: &Known, game: &LibraryGame, leaving: &[&str]) -> Verdict {
    if let Some(who) = game.launcher.as_deref() {
        return Verdict::ListOnly(format!("{} keeps this game, so it stays installed there. Only its place in your Kryoto Library goes.", launcher_name(who)));
    }
    if game.install_dir.trim().is_empty() {
        return Verdict::ListOnly("It has no folder on this PC, so there is nothing to delete.".into());
    }
    let folder = match dunce::canonicalize(&game.install_dir) {
        Ok(f) if f.is_dir() => f,
        _ => return Verdict::ListOnly("Its folder is not there any more, so there is nothing to delete.".into()),
    };
    // Kryoto's install record: the installer wrote it there, with every file.
    let record = read_json::<ReleaseRecord>(&folder.join(RELEASE_RECORD)).filter(|r| r.at > 0 && !r.files.is_empty());
    if record.is_none() && (game.added_by_hand || game.origin.is_some()) {
        return Verdict::ListOnly(if game.added_by_hand {
            "You added this game yourself, so its files stay where they are. Only its place in your Kryoto Library goes.".into()
        } else {
            format!("It came from {}, so its files stay where they are. Only its place in your Kryoto Library goes.", origin_name(game.origin.as_deref()))
        });
    }
    // No record of its files and nothing saying Kryoto installed it: a game
    // added by hand before 0.4.0 flagged those (it may carry a marker, written
    // on Play), so it is taken for one.
    if record.is_none() && !installed_by_kryoto(known, game) {
        return Verdict::ListOnly(
            "Kryoto has no record of installing this game (it was probably added by hand), so its files stay where they are. Only its place in your Kryoto Library goes.".into(),
        );
    }
    let roots: Vec<PathBuf> = known.folders.iter().filter_map(|f| dunce::canonicalize(f).ok()).collect();
    let library = roots.iter().filter(|r| folder.starts_with(r)).max_by_key(|r| r.components().count()).cloned();
    // In a library folder by its name, somewhere else on the disk: a link.
    if library.is_none() && known.folders.iter().any(|f| path_is_in(&game.install_dir, f)) {
        return Verdict::Refused(format!(
            "{} leads somewhere else ({}) through a link, so Kryoto deletes nothing in it.",
            game.install_dir,
            show(&folder)
        ));
    }
    // Before anything it carries: a drive or a library folder is refused whatever is in it.
    if let Some(why) = protected(&folder, &roots, known.home.as_deref()) {
        return Verdict::Refused(format!("{} {why}, so Kryoto deletes nothing in it.", show(&folder)));
    }
    let own_marker = read_marker(&folder).is_some_and(|m| same_game(&m, game));
    let list: Option<Vec<(String, u64)>> = match &record {
        Some(r) => Some(r.files.iter().map(|(p, s)| (p.clone(), s.size)).collect()),
        // kryo.to's list says what the release is, not where: only for the
        // folder carrying this game's marker, and only when that folder
        // really holds the release (Properties > Browse can point anywhere).
        None if own_marker => known.release_list.clone().filter(|l| holds_release(&folder, l)),
        None => None,
    };
    if list.is_none() {
        // Installed before Kryoto recorded the files: the whole folder, only
        // when it is plainly this game's own.
        let plain = own_marker && library.as_ref().is_some_and(|lib| named_for(game, &folder, lib));
        if !plain {
            return Verdict::Refused(format!(
                "Kryoto has no record of which files in {} are this game's, so it deletes nothing. Remove it from the Library only, then delete the folder yourself if it holds only this game.",
                show(&folder)
            ));
        }
    }
    let found = match guard(known, game, leaving, &folder) {
        Ok(found) => found,
        Err(why) => return Verdict::Refused(why),
    };
    let saves = saves_in(&folder);
    let keep_save = |p: &Path| saves.iter().any(|s| p.starts_with(s));
    let mut out = Plan { folder: folder.clone(), stop_at: library, ..Default::default() };
    match list {
        Some(list) => {
            let listed: u64 = list.iter().map(|(_, size)| size).sum();
            let total: u64 = found.iter().map(|(_, size)| size).sum();
            if far_bigger(total, listed) {
                return Verdict::Refused(format!(
                    "{} holds {}, far more than the {} Kryoto installed there. It may hold other games, so Kryoto deletes nothing in it.",
                    show(&folder),
                    crate::storage::human_size(total),
                    crate::storage::human_size(listed)
                ));
            }
            let mut ours: HashSet<String> = list.into_iter().map(|(p, _)| key(&p)).collect();
            ours.extend(kryotos_own(game).into_iter().map(|p| key(&p)));
            for (path, size) in found {
                let rel = path.strip_prefix(&folder).map(|r| key(&r.to_string_lossy())).unwrap_or_default();
                let kryoto = rel.starts_with(".kryoto-repair/");
                if (ours.contains(&rel) || kryoto) && !keep_save(&path) {
                    out.bytes += size;
                    out.files.push(path);
                } else {
                    out.kept_files += 1;
                    out.kept_bytes += size;
                }
            }
        }
        None => {
            for (path, size) in found {
                if keep_save(&path) {
                    out.kept_files += 1;
                    out.kept_bytes += size;
                } else {
                    out.bytes += size;
                    out.files.push(path);
                }
            }
        }
    }
    out.whole = out.kept_files == 0;
    Verdict::Delete(out)
}

/// `folder` (canonical) holds this game alone, as far as anything on the PC
/// says: not a protected place, no other library entry, no other game's
/// marker or release, no other launcher's games, no link out. Its files and
/// sizes, never following a link. Also what a move checks before it deletes
/// the folder it copied (storage.rs).
pub fn guard(known: &Known, game: &LibraryGame, leaving: &[&str], folder: &Path) -> Result<Vec<(PathBuf, u64)>, String> {
    let roots: Vec<PathBuf> = known.folders.iter().filter_map(|f| dunce::canonicalize(f).ok()).collect();
    if let Some(why) = protected(folder, &roots, known.home.as_deref()) {
        return Err(format!("{} {why}, so Kryoto deletes nothing in it.", show(folder)));
    }
    if let Some(other) = entry_inside(known, game, leaving, folder) {
        return Err(format!("{} also holds {}, another game in your library, so Kryoto deletes nothing in it.", show(folder), other.title));
    }
    walk(folder, game, known.games, leaving)
}

/// Something says Kryoto installed `game`, beyond the fields "Add a game"
/// also fills in (title, slug, source, version, art): the archive it was
/// installed from (`build_sha`, the player's own file `unverified`), both
/// only ever set by an install, or a row in Downloads for it.
fn installed_by_kryoto(known: &Known, game: &LibraryGame) -> bool {
    game.build_sha.is_some() || game.unverified || known.downloaded.iter().any(|(id, _)| *id == game.id)
}

/// A folder of `total` bytes is far bigger than the `listed` installed in it.
fn far_bigger(total: u64, listed: u64) -> bool {
    total > listed.saturating_mul(BIGGER) && total - listed > SLACK
}

/// Nine in ten of the release's files are in `folder` at their listed size.
fn holds_release(folder: &Path, list: &[(String, u64)]) -> bool {
    let present = list
        .iter()
        .filter(|(rel, size)| kryoto_repair::community::safe_path(rel) && std::fs::metadata(folder.join(rel)).is_ok_and(|m| m.is_file() && m.len() == *size))
        .count();
    !list.is_empty() && present * 10 >= list.len() * 9
}

/// The player's home folder, as the disk spells it.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).and_then(|h| dunce::canonicalize(h).ok())
}

/// The same game: a marker written for this entry.
pub(crate) fn same_game(marker: &LibraryGame, game: &LibraryGame) -> bool {
    (!game.id.is_empty() && marker.id == game.id) || (game.slug.is_some() && marker.slug == game.slug)
}

/// Files Kryoto itself put in the game's folder, relative to it.
fn kryotos_own(game: &LibraryGame) -> Vec<String> {
    let mut out = vec![MARKER.to_string(), RELEASE_RECORD.to_string(), UPDATE_RECORD.to_string()];
    for a in &game.addons {
        out.extend(a.files.iter().cloned());
    }
    if let Some(o) = &game.online {
        out.extend(o.added.iter().cloned());
    }
    out
}

/// Paths compared as Windows does: case and slashes aside.
pub(crate) fn key(path: &str) -> String {
    let p = path.replace('\\', "/");
    if cfg!(windows) { p.to_lowercase() } else { p }
}

/// By the words alone: for folders that may not be there to ask.
fn path_is_in(path: &str, folder: &str) -> bool {
    let norm = |p: &str| key(p.trim().trim_end_matches(['\\', '/']));
    let (path, folder) = (norm(path), norm(folder));
    !folder.is_empty() && (path == folder || path.starts_with(&format!("{folder}/")))
}

fn show(path: &Path) -> String {
    dunce::simplified(path).display().to_string()
}

fn launcher_name(who: &str) -> String {
    let mut c = who.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_else(|| "Another launcher".into())
}

/// "Hydra" for `hydra:steam:123`.
fn origin_name(origin: Option<&str>) -> String {
    origin.and_then(|o| o.split(':').next()).filter(|o| !o.is_empty()).map(launcher_name).unwrap_or_else(|| "another launcher".into())
}

/// Why `dir` is never deleted, nor deleted from above: a drive, the home
/// folder, one of its usual folders or a system folder, or a library folder or
/// one holding one. All canonical.
pub fn protected(dir: &Path, roots: &[PathBuf], home: Option<&Path>) -> Option<String> {
    if dir.parent().is_none() {
        return Some("is a whole drive".into());
    }
    if let Some(home) = home {
        if dir == home {
            return Some("is your home folder".into());
        }
        if home.starts_with(dir) {
            return Some("holds your home folder".into());
        }
        const USUAL: [&str; 18] = [
            "Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos", "Games", "Saved Games", "OneDrive", "AppData", "AppData/Local",
            "AppData/LocalLow", "AppData/Roaming", ".local", ".local/share", ".config", ".steam", "Games/Heroic",
        ];
        if let Some(u) = USUAL.iter().find(|u| same_dir(dir, &home.join(u))) {
            return Some(format!("is your {} folder", u.rsplit('/').next().unwrap_or(u)));
        }
    }
    for sys in system_dirs() {
        if dir == sys || sys.starts_with(dir) {
            return Some("is a system folder".into());
        }
    }
    for root in roots {
        if dir == root {
            return Some("is one of your library folders".into());
        }
        if root.starts_with(dir) {
            return Some(format!("holds your library folder {}", show(root)));
        }
    }
    None
}

/// Equal, case aside on Windows, when one side does not exist to canonicalize.
fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || key(&a.to_string_lossy()) == key(&b.to_string_lossy())
}

fn system_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = ["SystemRoot", "windir", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData", "PUBLIC"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    if cfg!(unix) {
        out.extend(["/usr", "/etc", "/opt", "/var", "/bin", "/lib", "/lib64", "/sbin", "/boot", "/home", "/mnt", "/media", "/run/media", "/srv", "/tmp"].map(PathBuf::from));
    }
    out.into_iter().filter_map(|p| dunce::canonicalize(&p).ok().or(Some(p))).collect()
}

/// Another library entry whose folder is in `folder`, by its words or on the
/// disk: one whose folder is gone still counts (KB-0375: those were exactly
/// the games that went).
fn entry_inside<'a>(known: &'a Known, game: &LibraryGame, leaving: &[&str], folder: &Path) -> Option<&'a LibraryGame> {
    let words = folder.to_string_lossy();
    known.games.iter().filter(|g| g.id != game.id && !leaving.contains(&g.id.as_str()) && !g.install_dir.trim().is_empty()).find(|g| {
        path_is_in(&g.install_dir, &words)
            || path_is_in(&g.install_dir, &game.install_dir)
            || dunce::canonicalize(&g.install_dir).is_ok_and(|d| d.starts_with(folder))
    })
}

/// Every file in `folder` with its size, never following a link. Refused
/// when it holds another game: its `.kryoto-game.json`, a release Kryoto
/// installed, another launcher's library or game, or a link to a folder outside.
fn walk(folder: &Path, game: &LibraryGame, games: &[LibraryGame], leaving: &[&str]) -> Result<Vec<(PathBuf, u64)>, String> {
    let shown = show(folder);
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(folder).follow_links(false) {
        let e = e.map_err(|e| format!("Kryoto could not read everything in {shown} ({e}), so it deletes nothing in it."))?;
        if e.depth() == 0 {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        let path = e.path().to_path_buf();
        if e.file_type().is_symlink() {
            // Removing a link removes the link, never what it points to; a
            // folder link out of the game's folder says the folder is not
            // what it seems.
            if let Ok(target) = dunce::canonicalize(&path) {
                if target.is_dir() && !target.starts_with(folder) {
                    return Err(format!("{shown} has a link to {} inside, so Kryoto deletes nothing in it.", show(&target)));
                }
            }
            out.push((path, 0));
            continue;
        }
        if e.file_type().is_dir() {
            if name == "steamapps" || (name == ".egstore" && e.depth() > 1) {
                return Err(format!("{shown} holds another launcher's games ({}), so Kryoto deletes nothing in it.", show(&path)));
            }
            continue;
        }
        let other_launcher = name.starts_with("appmanifest_") && name.ends_with(".acf") || (e.depth() > 1 && name.starts_with("goggame-") && name.ends_with(".info"));
        if other_launcher {
            return Err(format!("{shown} holds a game from another launcher ({}), so Kryoto deletes nothing in it.", show(&path)));
        }
        if e.depth() > 1 && name == MARKER {
            let theirs = path.parent().and_then(read_marker);
            let ours = theirs.as_ref().is_some_and(|m| same_game(m, game) || games.iter().any(|g| leaving.contains(&g.id.as_str()) && same_game(m, g)));
            if !ours {
                let title = theirs.map(|m| m.title).filter(|t| !t.is_empty()).unwrap_or_else(|| "another game".into());
                return Err(format!("{shown} also holds {title}, so Kryoto deletes nothing in it."));
            }
        }
        if e.depth() > 1 && name == RELEASE_RECORD {
            return Err(format!("{shown} also holds another release ({}), so Kryoto deletes nothing in it.", show(path.parent().unwrap_or(&path))));
        }
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        out.push((path, size));
    }
    Ok(out)
}

/// Folders inside the game where its Steam emulator keeps the saves
/// (kryoto-saves' own rules), which an uninstall leaves.
fn saves_in(folder: &Path) -> Vec<PathBuf> {
    kryoto_saves::emulator::rules(folder, None)
        .into_iter()
        .filter(|r| r.root == kryoto_saves::Root::GameDir && !r.path.is_empty())
        .map(|r| folder.join(r.path))
        .collect()
}

/// The folder is called after the game: its own folder Kryoto made, or one of
/// those above it inside the library folder.
pub(crate) fn named_for(game: &LibraryGame, folder: &Path, library: &Path) -> bool {
    let plain = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let mut names: Vec<String> = vec![plain(&game.title), plain(&crate::downloads::plain_name(&game.title, game.slug.as_deref()))];
    names.extend(game.slug.as_deref().map(plain));
    names.retain(|n| n.chars().count() >= 2);
    folder
        .ancestors()
        .take_while(|a| *a != library && a.starts_with(library))
        .filter_map(|a| a.file_name())
        .any(|n| names.contains(&plain(&n.to_string_lossy())))
}

/// Where deleted files go.
#[derive(Debug, Clone, PartialEq)]
pub enum Bin {
    /// The Recycle Bin or the Trash takes them.
    Yes,
    /// They are deleted for good, and why.
    No(String),
}

/// More separate files than this are not sent to the Recycle Bin one by one.
const BIN_FILES: usize = 20_000;

/// Whether the Recycle Bin (or the Trash) can take what `plan` deletes.
pub fn bin_for(plan: &Plan) -> Bin {
    if !plan.whole && plan.files.len() > BIN_FILES {
        return Bin::No(format!("It is {} separate files, too many for the {}.", plan.files.len(), BIN_NAME));
    }
    bin_on(&plan.folder, plan.bytes)
}

#[cfg(windows)]
const BIN_NAME: &str = "Recycle Bin";
#[cfg(not(windows))]
const BIN_NAME: &str = "Trash";

#[cfg(windows)]
fn bin_on(folder: &Path, bytes: u64) -> Bin {
    match windows_bin::capacity(folder) {
        Err(why) => Bin::No(why),
        Ok(cap) if bytes > cap => Bin::No(format!(
            "At {} it is too big for the Recycle Bin on this drive, which holds up to {}.",
            crate::storage::human_size(bytes),
            crate::storage::human_size(cap)
        )),
        Ok(_) => Bin::Yes,
    }
}

#[cfg(target_os = "linux")]
fn bin_on(folder: &Path, _bytes: u64) -> Bin {
    match gio_runs().and_then(|()| drive_trash(folder)) {
        Ok(()) => Bin::Yes,
        Err(why) => Bin::No(why),
    }
}

#[cfg(target_os = "linux")]
const NO_DRIVE_TRASH: &str = "This drive has no Trash Kryoto can use.";

/// Whether the drive `folder` is on has a Trash, so the player is told before
/// the first press. Files on the home's own drive go to the home Trash. Any
/// other drive needs `.Trash/<uid>` or `.Trash-<uid>` in its top folder, or a
/// top folder one can be made in (the freedesktop Trash rules). Only a clear
/// no is a no: what cannot be told here is left to gio, which answers itself.
#[cfg(target_os = "linux")]
fn drive_trash(folder: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Ok(()) };
    let (Ok(there), Ok(here)) = (std::fs::metadata(folder), std::fs::metadata(&home)) else { return Ok(()) };
    if there.dev() == here.dev() {
        return Ok(());
    }
    let Ok(folder) = folder.canonicalize() else { return Ok(()) };
    let top = folder
        .ancestors()
        .take_while(|a| std::fs::metadata(a).is_ok_and(|m| m.dev() == there.dev()))
        .last()
        .map(Path::to_path_buf);
    let Some(top) = top else { return Ok(()) };
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    if trash_fits(&top, uid) { Ok(()) } else { Err(NO_DRIVE_TRASH.to_string()) }
}

#[cfg(target_os = "linux")]
fn trash_fits(top: &Path, uid: u32) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let writable = |p: &Path| {
        std::ffi::CString::new(p.as_os_str().as_bytes()).is_ok_and(|c| {
            // SAFETY: `c` is a valid NUL-terminated string for the whole call.
            unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
        })
    };
    let shared = top.join(".Trash");
    if std::fs::symlink_metadata(&shared).is_ok_and(|m| m.is_dir()) && (writable(&shared) || writable(&shared.join(uid.to_string()))) {
        return true;
    }
    let own = top.join(format!(".Trash-{uid}"));
    match std::fs::symlink_metadata(&own) {
        Ok(m) => m.is_dir() && writable(&own),
        Err(_) => writable(top),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn bin_on(_folder: &Path, _bytes: u64) -> Bin {
    Bin::No("This system has no Trash Kryoto can use.".into())
}

/// The system's `gio`, found on the PATH the desktop gave us: never a copy
/// inside the AppImage.
#[cfg(target_os = "linux")]
fn gio() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let appdir = std::env::var_os("APPDIR").filter(|d| !d.is_empty()).map(PathBuf::from);
    std::env::split_paths(&path)
        .filter(|d| appdir.as_ref().is_none_or(|a| !d.starts_with(a)))
        .map(|d| d.join("gio"))
        .find(|p| p.is_file())
}

/// `gio` as the desktop would run it: without the AppImage's library paths,
/// which made the system's gio load our older GLib and fail ("undefined
/// symbol: g_unix_mount_entry_get_options") after the player had confirmed.
#[cfg(target_os = "linux")]
fn gio_command() -> Option<std::process::Command> {
    let mut cmd = std::process::Command::new(gio()?);
    crate::linux_open::clean(&mut cmd);
    cmd.stdin(std::process::Stdio::null());
    Some(cmd)
}

/// Whether the Trash can really be used here: `gio` is there and runs, asked
/// once per session, so a system where it cannot says so before the player
/// confirms rather than failing after.
#[cfg(target_os = "linux")]
fn gio_runs() -> Result<(), String> {
    static ANSWER: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    ANSWER
        .get_or_init(|| {
            let mut cmd = gio_command().ok_or_else(|| "This system has no Trash Kryoto can use (gio is not installed).".to_string())?;
            match cmd.args(["help", "trash"]).output() {
                Ok(out) if out.status.success() => Ok(()),
                Ok(out) => {
                    let said = String::from_utf8_lossy(&out.stderr);
                    let said = said.lines().next().unwrap_or("").trim();
                    Err(format!("This system's Trash cannot be used from Kryoto (gio did not run: {said})."))
                }
                Err(e) => Err(format!("This system's Trash cannot be used from Kryoto (gio did not run: {e}).")),
            }
        })
        .clone()
}

/// What an uninstall did.
#[derive(Debug, Default, PartialEq)]
pub struct Done {
    pub to_bin: bool,
    /// Files that could not be deleted (in use, read-only): they stay.
    pub left: u64,
}

/// Delete what `plan` says, and nothing else: into the Recycle Bin or Trash,
/// or for good with `permanent` (only after the player was told). `report`
/// gets bytes removed of the total.
pub fn run(plan: &Plan, permanent: bool, report: &mut dyn FnMut(u64, u64)) -> Result<Done, String> {
    report(0, plan.bytes);
    let mut done = Done { to_bin: !permanent, left: 0 };
    if !permanent {
        let what: Vec<PathBuf> = if plan.whole { vec![plan.folder.clone()] } else { plan.files.clone() };
        trash(&what).map_err(|e| format!("Could not move it to the {BIN_NAME} ({e}). Nothing more was deleted."))?;
    } else {
        let mut removed = 0u64;
        let mut last = std::time::Instant::now();
        for f in &plan.files {
            // Only what is still a file (or a link) inside the folder.
            let ok = f.starts_with(&plan.folder) && std::fs::symlink_metadata(f).is_ok_and(|m| !m.is_dir());
            let size = std::fs::symlink_metadata(f).map(|m| m.len()).unwrap_or(0);
            if ok && remove_file(f) {
                removed += size;
            } else if ok {
                done.left += 1;
            }
            if last.elapsed().as_millis() > 200 {
                last = std::time::Instant::now();
                report(removed, plan.bytes);
            }
        }
    }
    prune(plan);
    report(plan.bytes, plan.bytes);
    Ok(done)
}

/// One file, a read-only one too (Windows refuses those until told otherwise).
fn remove_file(f: &Path) -> bool {
    if std::fs::remove_file(f).is_ok() {
        return true;
    }
    let Ok(meta) = std::fs::symlink_metadata(f) else { return true };
    let mut perms = meta.permissions();
    if !meta.file_type().is_symlink() && perms.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        let _ = std::fs::set_permissions(f, perms);
        return std::fs::remove_file(f).is_ok();
    }
    false
}

/// Folders inside the game's folder left empty, the folder itself once empty,
/// then empty folders above it up to its library folder. Only ever empty
/// ones: `remove_dir` refuses any other.
fn prune(plan: &Plan) {
    if plan.folder.is_dir() {
        let mut dirs: Vec<PathBuf> = walkdir::WalkDir::new(&plan.folder)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_dir())
            .map(|e| e.into_path())
            .collect();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
        for d in dirs {
            let _ = std::fs::remove_dir(d);
        }
    }
    let Some(stop) = &plan.stop_at else { return };
    let mut up = plan.folder.parent();
    while let Some(dir) = up {
        if dir == stop || !dir.starts_with(stop) || std::fs::remove_dir(dir).is_err() {
            break;
        }
        up = dir.parent();
    }
}

#[cfg(windows)]
fn trash(paths: &[PathBuf]) -> Result<(), String> {
    windows_bin::recycle(paths)
}

#[cfg(target_os = "linux")]
fn trash(paths: &[PathBuf]) -> Result<(), String> {
    gio_runs()?;
    for chunk in paths.chunks(200) {
        let mut cmd = gio_command().ok_or("no gio")?;
        let out = cmd.arg("trash").arg("--").args(chunk).output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            // gio's own words hold whole paths: they go to the log, not the dialog.
            crate::logging::warn("uninstall", &format!("gio trash: {}", String::from_utf8_lossy(&out.stderr).trim()));
            return Err("this drive's Trash refused it".to_string());
        }
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn trash(_paths: &[PathBuf]) -> Result<(), String> {
    Err("no Trash".into())
}

#[cfg(windows)]
mod windows_bin {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Path, PathBuf};

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// The drive's root, `D:\`.
    fn root(path: &Path) -> Option<PathBuf> {
        match path.components().next()? {
            Component::Prefix(p) => Some(PathBuf::from(format!("{}\\", p.as_os_str().to_string_lossy()))),
            _ => None,
        }
    }

    /// How much the Recycle Bin on `path`'s drive holds, from Windows' own
    /// settings for it (Recycle Bin > Properties), or why it takes nothing.
    pub fn capacity(path: &Path) -> Result<u64, String> {
        use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeNameForVolumeMountPointW};
        const DRIVE_FIXED: u32 = 3;
        let path = dunce::simplified(path);
        let root = root(path).ok_or("It is on a network folder, which has no Recycle Bin.")?;
        let root_w = wide(root.as_os_str());
        // SAFETY: a NUL-terminated root path.
        if unsafe { GetDriveTypeW(root_w.as_ptr()) } != DRIVE_FIXED {
            return Err("It is on a removable or network drive, which has no Recycle Bin.".into());
        }
        let mut name = [0u16; 64];
        // SAFETY: the root path and a buffer of the length given.
        let ok = unsafe { GetVolumeNameForVolumeMountPointW(root_w.as_ptr(), name.as_mut_ptr(), name.len() as u32) };
        let total = crate::storage::disk_space(path).map(|(t, _)| t).unwrap_or(0);
        // Windows' own default: about 5% of the drive.
        let fallback = total / 20;
        if ok == 0 {
            return Ok(fallback);
        }
        let name = String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())]);
        let Some(guid) = name.find('{').and_then(|a| name[a..].find('}').map(|b| name[a..a + b + 1].to_string())) else { return Ok(fallback) };
        let key = format!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\BitBucket\\Volume\\{guid}");
        if dword(&key, "NukeOnDelete") == Some(1) {
            return Err("The Recycle Bin is turned off for this drive.".into());
        }
        Ok(dword(&key, "MaxCapacity").map(|mb| u64::from(mb) << 20).unwrap_or(fallback))
    }

    fn dword(key: &str, value: &str) -> Option<u32> {
        use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
        let (key, value) = (wide(std::ffi::OsStr::new(key)), wide(std::ffi::OsStr::new(value)));
        let mut data = 0u32;
        let mut size = 4u32;
        // SAFETY: NUL-terminated names and a 4-byte out buffer of the size given.
        let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut data as *mut u32).cast(), &mut size) };
        (r == 0).then_some(data)
    }

    /// Into the Recycle Bin, as Explorer's Delete does, on a thread of its own
    /// set up for the shell (single-threaded COM). Should Windows find it too
    /// big after all, it asks the player itself before deleting for good
    /// (`FOF_WANTNUKEWARNING`).
    pub fn recycle(paths: &[PathBuf]) -> Result<(), String> {
        let mut from: Vec<u16> = Vec::new();
        for p in paths {
            from.extend(dunce::simplified(p).as_os_str().encode_wide());
            from.push(0);
        }
        from.push(0);
        std::thread::spawn(move || shell_delete(&from)).join().map_err(|_| "the Recycle Bin call failed".to_string())?
    }

    fn shell_delete(from: &[u16]) -> Result<(), String> {
        use windows_sys::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
        use windows_sys::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW};
        // SAFETY: COM for this new thread only, undone below.
        let com = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as _) } >= 0;
        // SAFETY: zeroed is a valid empty SHFILEOPSTRUCTW.
        let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
        op.wFunc = FO_DELETE;
        op.pFrom = from.as_ptr();
        op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOF_WANTNUKEWARNING) as _;
        // SAFETY: `from` is double-NUL-terminated and outlives the call.
        let r = unsafe { SHFileOperationW(&mut op) };
        if com {
            // SAFETY: paired with the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
        if r != 0 {
            return Err(format!("Windows answered {r:#x}"));
        }
        if op.fAnyOperationsAborted != 0 {
            return Err("it was stopped".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kryoto_repair::community::Stamp;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryoto-safe-delete-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        dunce::canonicalize(&d).unwrap()
    }

    fn put(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn game(id: &str, dir: &Path) -> LibraryGame {
        LibraryGame { id: id.into(), title: id.into(), slug: Some(id.into()), install_dir: dir.to_string_lossy().into_owned(), ..Default::default() }
    }

    /// The marker Kryoto writes in the folder of a game it installed.
    fn mark(g: &LibraryGame) {
        put(&Path::new(&g.install_dir).join(MARKER), &serde_json::to_vec(g).unwrap());
    }

    /// Kryoto's install record: every file in the folder now.
    fn record(dir: &Path) {
        let files = walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .map(|e| (e.path().strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"), Stamp { size: e.metadata().unwrap().len(), modified_ms: 0 }))
            .collect();
        let r = ReleaseRecord { version: Some("1".into()), source: None, at: 1, files };
        put(&dir.join(RELEASE_RECORD), &serde_json::to_vec(&r).unwrap());
    }

    fn known<'a>(games: &'a [LibraryGame], folders: &'a [String]) -> Known<'a> {
        Known { games, folders, home: None, release_list: None, downloaded: games.iter().map(|g| (g.id.clone(), g.install_dir.clone())).collect() }
    }

    fn text(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    fn refused(v: &Verdict) -> &str {
        match v {
            Verdict::Refused(why) => why,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn planned(v: Verdict) -> Plan {
        match v {
            Verdict::Delete(p) => p,
            other => panic!("expected a plan, got {other:?}"),
        }
    }

    fn uninstall(v: Verdict) -> Plan {
        let p = planned(v);
        run(&p, true, &mut |_, _| {}).unwrap();
        p
    }

    /// KB-0375 as it happened: the library folder is the drive (here `base`),
    /// the games were moved into `Games\`, and "Find games Kryoto installed"
    /// recorded Foo at `Games\Foo`, a level deeper than its own folder. The
    /// other games' entries still pointed at where they were before. The old
    /// rule took the first folder under the library folder, `Games`, and
    /// found nothing on the list in it. Now only Foo goes.
    #[test]
    fn the_reported_loss_takes_only_the_game() {
        let base = scratch("kb0375");
        let games_dir = base.join("Games");
        let (foo, bar, baz) = (games_dir.join("Foo"), games_dir.join("Bar"), games_dir.join("Baz"));
        put(&foo.join("Foo.exe"), b"foo");
        put(&bar.join("Bar.exe"), b"bar");
        put(&baz.join("Baz.exe"), b"baz, never in Kryoto");
        let g = game("foo", &foo);
        mark(&g);
        record(&foo);
        mark(&game("bar", &bar));
        let gone_bar = game("bar", &base.join("Old").join("Bar"));
        let games = vec![g.clone(), gone_bar];
        let folders = vec![text(&base)];
        // The old rule: the first folder under the library folder, kept only
        // by entries whose folders are still there.
        let roots = vec![Some(base.clone())];
        let (_, old) = crate::storage::own_folder_for_tests(&roots, &foo).unwrap();
        assert_eq!(old, games_dir);
        assert!(!crate::storage::holds_other_games(&games, &["foo"], &old), "the old guard let it go");
        // The new one.
        let p = uninstall(plan(&known(&games, &folders), &g, &["foo"]));
        assert_eq!(p.folder, foo);
        assert!(!foo.exists());
        assert!(bar.join("Bar.exe").exists() && baz.join("Baz.exe").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// The game's folder recorded on the container itself (Properties >
    /// Browse to an exe in `Games\`): another game's marker inside refuses.
    #[test]
    fn a_folder_with_another_games_marker_inside_is_refused() {
        let base = scratch("marker");
        let shared = base.join("Games");
        put(&shared.join("Foo.exe"), b"foo");
        let bar = game("bar", &shared.join("Bar"));
        put(&shared.join("Bar").join("Bar.exe"), b"bar");
        mark(&bar);
        let mut g = game("games", &shared);
        g.title = "Games".into();
        mark(&g);
        let folders = vec![text(&base)];
        let games = vec![g.clone()];
        let why = refused(&plan(&known(&games, &folders), &g, &["games"])).to_string();
        assert!(why.contains("also holds bar"), "{why}");
        assert!(shared.join("Bar").join("Bar.exe").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// An entry whose folder is gone still holds its place by its words.
    #[test]
    fn a_missing_entry_inside_is_refused() {
        let base = scratch("missing");
        let shared = base.join("Games");
        put(&shared.join("Foo.exe"), b"foo");
        let mut g = game("games", &shared);
        g.title = "Games".into();
        mark(&g);
        let games = vec![g.clone(), game("other", &shared.join("Other"))];
        let folders = vec![text(&base)];
        assert!(refused(&plan(&known(&games, &folders), &g, &["games"])).contains("another game in your library"));
        let _ = std::fs::remove_dir_all(base);
    }

    /// A game added from a folder of the player's own, even inside a library
    /// folder, only leaves the Library. So does one from another launcher.
    #[test]
    fn games_kryoto_did_not_install_only_leave_the_library() {
        let base = scratch("added");
        let shared = base.join("Games").join("Stuff");
        put(&shared.join("deep").join("Mine.exe"), b"mine");
        put(&shared.join("Other.exe"), b"other");
        let mut g = game("mine", &shared.join("deep"));
        g.added_by_hand = true;
        mark(&g);
        let folders = vec![text(&base)];
        let games = vec![g.clone()];
        assert!(matches!(plan(&known(&games, &folders), &g, &[]), Verdict::ListOnly(_)));
        let mut h = game("hydra", &shared);
        h.origin = Some("hydra:steam:1".into());
        assert!(matches!(plan(&known(&games, &folders), &h, &[]), Verdict::ListOnly(why) if why.contains("Hydra")));
        let mut s = game("steam", &shared);
        s.launcher = Some("steam".into());
        assert!(matches!(plan(&known(&games, &folders), &s, &[]), Verdict::ListOnly(why) if why.starts_with("Steam keeps")));
        assert!(shared.join("Other.exe").exists() && shared.join("deep").join("Mine.exe").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// A drive, the home folder, a library folder and a folder holding one are never deleted.
    #[test]
    fn places_are_never_deleted() {
        let base = scratch("places");
        let lib = base.join("Library");
        std::fs::create_dir_all(&lib).unwrap();
        let roots = vec![dunce::canonicalize(&lib).unwrap()];
        let drive = base.ancestors().last().unwrap().to_path_buf();
        assert_eq!(protected(&drive, &roots, None).as_deref(), Some("is a whole drive"));
        assert!(protected(&roots[0], &roots, None).unwrap().contains("library folder"));
        assert!(protected(&base, &roots, None).unwrap().starts_with("holds your library folder"));
        assert_eq!(protected(&base, &[], Some(&base)).as_deref(), Some("is your home folder"));
        assert_eq!(protected(&base.join("Downloads"), &[], Some(&base)).as_deref(), Some("is your Downloads folder"));
        assert!(protected(&lib.join("Game"), &roots, Some(&base)).is_none());
        // A game recorded on the library folder itself.
        let mut g = game("lib", &lib);
        mark(&g);
        record(&lib);
        g.title = "Library".into();
        let folders = vec![text(&lib)];
        let games = vec![g.clone()];
        assert!(refused(&plan(&known(&games, &folders), &g, &[])).contains("library folder"));
        let _ = std::fs::remove_dir_all(base);
    }

    /// A drive-root library folder: the game Kryoto made there goes, the rest stays.
    #[test]
    fn a_game_in_a_drive_root_library_takes_only_its_folder() {
        // `base` stands in for D:\, as a library folder.
        let base = scratch("driveroot");
        let hades = base.join("Hades");
        put(&hades.join("Hades.exe"), b"hades");
        put(&base.join("Photos").join("me.jpg"), b"photo");
        let mut g = game("hades", &hades);
        g.title = "Hades".into();
        mark(&g);
        // Installed before 0.3.0: no record, so the whole folder named for it.
        let folders = vec![text(&base)];
        let games = vec![g.clone()];
        let p = uninstall(plan(&known(&games, &folders), &g, &[]));
        assert!(p.whole && !hades.exists());
        assert!(base.join("Photos").join("me.jpg").exists() && base.exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// Without a record, a folder not named for the game, or without the
    /// game's own marker, is refused.
    #[test]
    fn without_a_record_only_the_games_own_named_folder() {
        let base = scratch("legacy");
        let other = base.join("Other Game");
        put(&other.join("x.exe"), b"x");
        let mut g = game("hades", &other);
        g.title = "Hades".into();
        let folders = vec![text(&base)];
        let games = vec![g.clone()];
        assert!(refused(&plan(&known(&games, &folders), &g, &[])).contains("no record"));
        mark(&g);
        assert!(refused(&plan(&known(&games, &folders), &g, &[])).contains("no record"));
        assert!(other.join("x.exe").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// A game added by hand before `added_by_hand` existed: in its own folder
    /// named for it, with the marker Play wrote, it looks like an install from
    /// before 0.3.0. With nothing saying Kryoto installed it (no archive hash,
    /// no row in Downloads) it only leaves the Library; with one, it goes.
    #[test]
    fn a_game_added_by_hand_before_the_flag_only_leaves_the_library() {
        let base = scratch("handold");
        let hades = base.join("Hades");
        put(&hades.join("Hades.exe"), b"hades");
        let mut g = game("hades", &hades);
        g.title = "Hades".into();
        // What "Add a game" linked to kryo.to fills in, like an install does.
        g.source = Some("gog".into());
        g.version = Some("1.0".into());
        mark(&g);
        let folders = vec![text(&base)];
        let games = vec![g.clone()];
        let nothing = Known { games: &games, folders: &folders, home: None, release_list: None, downloaded: Vec::new() };
        assert!(matches!(plan(&nothing, &g, &[]), Verdict::ListOnly(why) if why.contains("added by hand")));
        // A row in Downloads for another game says nothing about this one.
        let other_row = Known { downloaded: vec![("celeste".into(), String::new())], ..nothing };
        assert!(matches!(plan(&other_row, &g, &[]), Verdict::ListOnly(_)));
        assert!(hades.join("Hades.exe").exists());
        // The archive's hash only an install sets.
        let mut hashed = g.clone();
        hashed.build_sha = Some("ab".repeat(32));
        assert!(matches!(plan(&other_row, &hashed, &[]), Verdict::Delete(_)));
        // Its row in Downloads.
        let row = Known { downloaded: vec![("hades".into(), text(&hades))], ..other_row };
        let p = uninstall(plan(&row, &g, &[]));
        assert!(p.whole && !hades.exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// With the install record: the files it lists and Kryoto's own go,
    /// anything else (a mod, saves the emulator keeps) stays, and so does the folder.
    #[test]
    fn with_a_record_only_its_files_go() {
        let base = scratch("record");
        let dir = base.join("Hades");
        put(&dir.join("Hades.exe"), b"hades");
        put(&dir.join("bin").join("data.pak"), b"data");
        put(&dir.join("steam_settings").join("configs.user.ini"), b"[user::saves]\nlocal_save_path=./saves\n");
        let g = game("hades", &dir);
        mark(&g);
        record(&dir);
        put(&dir.join("mods").join("mine.pak"), b"a mod");
        put(&dir.join("saves").join("slot1.sav"), b"save");
        let folders = vec![text(&base)];
        let games = vec![g.clone()];
        let p = uninstall(plan(&known(&games, &folders), &g, &[]));
        assert!(!p.whole && p.kept_files == 2, "{p:?}");
        assert!(!dir.join("Hades.exe").exists() && !dir.join("bin").exists() && !dir.join(MARKER).exists());
        assert!(dir.join("mods").join("mine.pak").exists() && dir.join("saves").join("slot1.sav").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// A folder far bigger than what was installed in it is refused: three
    /// times over and by more than 5 GB (a game with its mods and caches is not).
    #[test]
    fn a_folder_far_bigger_than_the_game_is_refused() {
        const GB: u64 = 1 << 30;
        assert!(far_bigger(500 * GB, 40 * GB));
        assert!(far_bigger(6 * GB, 1));
        assert!(!far_bigger(90 * GB, 50 * GB));
        assert!(!far_bigger(4 * GB, 1));
        assert!(!far_bigger(40 * GB, 40 * GB));
    }

    /// Two games uninstalled in one go do not keep each other's folder.
    #[test]
    fn games_leaving_together_do_not_hold_each_other() {
        let base = scratch("together");
        let a = base.join("Pack");
        put(&a.join("A.exe"), b"a");
        let b = a.join("Extra");
        put(&b.join("B.exe"), b"b");
        let mut ga = game("pack", &a);
        ga.title = "Pack".into();
        let gb = game("extra", &b);
        mark(&ga);
        mark(&gb);
        let folders = vec![text(&base)];
        let games = vec![ga.clone(), gb.clone()];
        assert!(matches!(plan(&known(&games, &folders), &ga, &["pack"]), Verdict::Refused(_)));
        assert!(matches!(plan(&known(&games, &folders), &ga, &["pack", "extra"]), Verdict::Delete(_)));
        let _ = std::fs::remove_dir_all(base);
    }

    /// Another launcher's library inside is refused.
    #[test]
    fn another_launchers_games_inside_are_refused() {
        let base = scratch("launcher");
        let dir = base.join("Hades");
        put(&dir.join("Hades.exe"), b"h");
        put(&dir.join("Steam").join("steamapps").join("appmanifest_1.acf"), b"x");
        let mut g = game("hades", &dir);
        g.title = "Hades".into();
        mark(&g);
        let folders = vec![text(&base)];
        let games = vec![g.clone()];
        assert!(refused(&plan(&known(&games, &folders), &g, &[])).contains("another launcher"));
        let _ = std::fs::remove_dir_all(base);
    }

    /// A link (a junction on Windows) to a folder outside: refused, and what
    /// it points to is never touched.
    #[test]
    fn links_out_of_the_folder_are_refused() {
        let base = scratch("links");
        let dir = base.join("Hades");
        put(&dir.join("Hades.exe"), b"h");
        let outside = base.join("Precious");
        put(&outside.join("keep.txt"), b"keep");
        let mut g = game("hades", &dir);
        g.title = "Hades".into();
        mark(&g);
        if !make_dir_link(&outside, &dir.join("link")) {
            return;
        }
        let folders = vec![text(&base)];
        let games = vec![g.clone()];
        assert!(refused(&plan(&known(&games, &folders), &g, &[])).contains("has a link"));
        // The game's folder itself through a link out of the library.
        let lib = base.join("Lib");
        std::fs::create_dir_all(&lib).unwrap();
        if make_dir_link(&outside, &lib.join("Hades")) {
            let mut through = game("hades", &lib.join("Hades"));
            through.title = "Hades".into();
            let folders = vec![text(&lib)];
            assert!(refused(&plan(&known(&games, &folders), &through, &[])).contains("through a link"));
        }
        assert!(outside.join("keep.txt").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    fn make_dir_link(target: &Path, link: &Path) -> bool {
        std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(link).arg(target).output().is_ok_and(|o| o.status.success())
    }

    #[cfg(unix)]
    fn make_dir_link(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    /// kryo.to's file list counts only for the folder with this game's
    /// marker that really holds the release: a folder Properties > Browse
    /// pointed at, with a same-named file of another game's, is refused.
    #[test]
    fn the_release_list_needs_the_games_marker_and_the_release() {
        let base = scratch("list");
        let dir = base.join("Somewhere");
        put(&dir.join("game.exe"), b"g");
        put(&dir.join("data.pak"), b"data");
        put(&dir.join("notes.txt"), b"mine");
        let g = game("hades", &dir);
        let folders = vec![text(&base)];
        let games = vec![g.clone()];
        let listed = vec![("game.exe".into(), 1), ("data.pak".into(), 4)];
        let k = Known { games: &games, folders: &folders, home: None, release_list: Some(listed), downloaded: vec![("hades".into(), String::new())] };
        assert!(matches!(plan(&k, &g, &[]), Verdict::Refused(_)));
        mark(&g);
        let other = Known { games: &games, folders: &folders, home: None, release_list: Some(vec![("game.exe".into(), 1), ("bin/missing.dll".into(), 9)]), downloaded: k.downloaded.clone() };
        assert!(matches!(plan(&other, &g, &[]), Verdict::Refused(_)));
        let p = uninstall(plan(&k, &g, &[]));
        assert_eq!(p.files.len(), 3, "{p:?}");
        assert!(!dir.join("game.exe").exists() && !dir.join("data.pak").exists() && dir.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// A drive without a usable Trash is told before the player confirms.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_drive_without_a_trash_is_known_beforehand() {
        let top = scratch("drivetrash");
        std::fs::create_dir_all(&top).unwrap();
        assert!(trash_fits(&top, 4242), "a top folder a Trash can be made in");
        put(&top.join(".Trash-4242"), b"not a folder");
        assert!(!trash_fits(&top, 4242), "something else has the Trash's name");
        std::fs::remove_file(top.join(".Trash-4242")).unwrap();
        std::fs::create_dir(top.join(".Trash-4242")).unwrap();
        assert!(trash_fits(&top, 4242), "its own Trash");
        let _ = std::fs::remove_dir_all(top);
    }

    /// Too many separate files, or too many bytes, are deleted for good only
    /// when the player was told.
    #[test]
    fn the_bin_says_when_it_cannot_take_it() {
        let mut p = Plan { folder: std::env::temp_dir(), whole: false, files: vec![PathBuf::new(); BIN_FILES + 1], ..Default::default() };
        assert!(matches!(bin_for(&p), Bin::No(why) if why.contains("too many")));
        p.files.clear();
        p.whole = true;
        p.bytes = u64::MAX / 2;
        // Windows knows how much its Recycle Bin holds; gio's Trash says so itself.
        if cfg!(windows) {
            assert!(matches!(bin_for(&p), Bin::No(why) if why.contains("too big")));
        }
    }

    /// Deleting for good says how far it has got, ends at the whole size and
    /// takes a read-only file too (KB-0305).
    #[test]
    fn deleting_counts_and_takes_read_only_files() {
        let base = scratch("count");
        let dir = base.join("Game");
        put(&dir.join("game.exe"), &[1u8; 1000]);
        put(&dir.join("data").join("a.pak"), &[2u8; 3000]);
        put(&dir.join("data").join("deep").join("b.pak"), &[3u8; 6000]);
        let locked = dir.join("data").join("readonly.ini");
        put(&locked, b"x");
        let mut perms = std::fs::metadata(&locked).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&locked, perms).unwrap();
        let mut g = game("game", &dir);
        g.title = "Game".into();
        mark(&g);
        let folders = vec![text(&base)];
        let games = vec![g.clone()];
        let p = planned(plan(&known(&games, &folders), &g, &[]));
        let mut reports = Vec::new();
        let done = run(&p, true, &mut |removed, total| reports.push((removed, total))).unwrap();
        assert_eq!(done, Done { to_bin: false, left: 0 });
        assert!(!dir.exists(), "the folder is gone");
        assert_eq!(reports.last(), Some(&(p.bytes, p.bytes)), "ends at the whole size");
        assert!(p.bytes >= 10_001 && reports.windows(2).all(|w| w[0].0 <= w[1].0), "only ever goes forward");
        let _ = std::fs::remove_dir_all(base);
    }

    /// By hand only (`cargo test --lib recycle_bin -- --ignored`): a scratch
    /// folder goes into this PC's Recycle Bin, where it can be put back.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn a_folder_goes_to_the_recycle_bin() {
        let base = scratch("recycle-bin");
        let dir = base.join("Kryoto recycle test");
        put(&dir.join("game.exe"), b"recycle me");
        let p = Plan { folder: dir.clone(), whole: true, files: vec![dir.join("game.exe")], bytes: 10, stop_at: Some(base.clone()), ..Default::default() };
        assert_eq!(bin_for(&p), Bin::Yes);
        assert_eq!(run(&p, false, &mut |_, _| {}).unwrap(), Done { to_bin: true, left: 0 });
        assert!(!dir.exists() && base.exists());
        let _ = std::fs::remove_dir_all(base);
    }

    /// Emptied folders go up to, never including, the library folder.
    #[test]
    fn empty_folders_above_go_up_to_the_library_folder() {
        let base = scratch("prune");
        let lib = base.join("Lib");
        let dir = lib.join("Wrapper").join("Hades");
        put(&dir.join("Hades.exe"), b"h");
        let mut g = game("hades", &dir);
        g.title = "Hades".into();
        mark(&g);
        let folders = vec![text(&lib)];
        let games = vec![g.clone()];
        uninstall(plan(&known(&games, &folders), &g, &[]));
        assert!(!lib.join("Wrapper").exists() && lib.is_dir());
        let _ = std::fs::remove_dir_all(base);
    }
}
