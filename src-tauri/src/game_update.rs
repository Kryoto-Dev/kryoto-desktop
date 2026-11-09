//! Game updates that can be undone.
//!
//! An update is a newer Kryoto release unpacked over the installed game
//! (`downloads::install`): the files it ships replace the old ones, and
//! everything else in the folder (saves, settings the game wrote next to
//! itself) stays. Releases are single archives, so there is no smaller "only
//! the changed files" download to be had; what this module adds is the safety
//! net around the merge:
//!
//!   * Before anything is replaced, each file the update will overwrite is
//!     copied to the backup folder the player chose, and the files it will
//!     add are listed. Both go in a journal next to the copies.
//!   * If the merge fails, the journal puts the old files back and removes
//!     the added ones, and the game is left as it was.
//!   * If Kryoto is closed or crashes in the middle, the journal is still
//!     marked `applying`, and the next start rolls it back the same way.
//!   * After a successful update, "Undo update" restores the previous build
//!     from the same journal.
//!
//! The journals are listed in `update-backups.json` in the app's data folder,
//! newest per game, since the backup folder is the player's and can be
//! anywhere.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Copies made, merge running (or interrupted).
    Applying,
    /// The new build is in place; the copies can undo it.
    Applied,
    /// Put back, after a failure, an interruption or "Undo update".
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub slug: String,
    pub title: String,
    /// The game's folder.
    pub dest: String,
    /// Where the copies are: `<backup root>/<game>-<time>/`.
    pub dir: String,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    /// Relative paths the update overwrote; their old copies are under `dir/files`.
    pub replaced: Vec<String>,
    /// Relative paths the update created; removed on rollback.
    pub added: Vec<String>,
    pub bytes: u64,
    pub created_at: i64,
    pub state: State,
    /// The backup this one takes over from, until this update is in for good.
    #[serde(default)]
    pub previous: Option<String>,
}

impl Journal {
    fn path(&self) -> PathBuf {
        Path::new(&self.dir).join("journal.json")
    }

    pub fn save(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(self.path(), text).map_err(|e| format!("Writing the update journal failed: {e}"))
    }

    pub fn load(dir: &Path) -> Option<Journal> {
        serde_json::from_slice(&std::fs::read(dir.join("journal.json")).ok()?).ok()
    }
}

/// Every file under `root`, as `/`-separated paths relative to it.
fn files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(_) => {
                    if let Ok(rel) = path.strip_prefix(root) {
                        out.push(rel.to_string_lossy().replace('\\', "/"));
                    }
                }
                Err(_) => {}
            }
        }
    }
    out.sort();
    out
}

/// What merging `staging` into `dest` will overwrite and what it will add.
pub fn plan(staging: &Path, dest: &Path) -> (Vec<String>, Vec<String>) {
    files(staging).into_iter().partition(|rel| dest.join(rel).is_file())
}

fn safe(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(60)
        .collect()
}

pub struct Prepare<'a> {
    pub slug: &'a str,
    pub title: &'a str,
    pub staging: &'a Path,
    pub dest: &'a Path,
    pub backup_root: &'a Path,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
}

/// Copy what the update will replace and write the journal, before the merge.
/// Fails without touching the game when the copies cannot all be made.
pub fn prepare(p: Prepare) -> Result<Journal, String> {
    let (replaced, added) = plan(p.staging, p.dest);
    let now = chrono_like_now();
    let dir = p.backup_root.join(format!("{}-{}", safe(p.title), now));
    let files_dir = dir.join("files");
    std::fs::create_dir_all(&files_dir)
        .map_err(|e| format!("The backup folder {} cannot be written: {e}", p.backup_root.display()))?;
    let bytes: u64 = replaced
        .iter()
        .filter_map(|r| std::fs::metadata(p.dest.join(r)).ok())
        .map(|m| m.len())
        .sum();
    crate::storage::check_room(&dir, bytes, p.title)?;
    for rel in &replaced {
        let to = files_dir.join(rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Backing up {rel} failed: {e}"))?;
        }
        std::fs::copy(p.dest.join(rel), &to).map_err(|e| {
            let _ = std::fs::remove_dir_all(&dir);
            format!("Backing up {rel} failed, so the update was not applied: {e}")
        })?;
    }
    let journal = Journal {
        slug: p.slug.to_string(),
        title: p.title.to_string(),
        dest: p.dest.to_string_lossy().into_owned(),
        dir: dir.to_string_lossy().into_owned(),
        from_version: p.from_version,
        to_version: p.to_version,
        replaced,
        added,
        bytes,
        created_at: now,
        state: State::Applying,
        previous: None,
    };
    journal.save()?;
    Ok(journal)
}

/// Put the game back the way the journal found it. Safe to run twice.
pub fn restore(journal: &mut Journal) -> Result<(), String> {
    let dest = PathBuf::from(&journal.dest);
    let files_dir = Path::new(&journal.dir).join("files");
    let mut failed = Vec::new();
    for rel in &journal.added {
        let path = dest.join(rel);
        if path.is_file() && std::fs::remove_file(&path).is_err() {
            failed.push(rel.clone());
        }
    }
    for rel in &journal.replaced {
        let from = files_dir.join(rel);
        let to = dest.join(rel);
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::copy(&from, &to).is_err() {
            failed.push(rel.clone());
        }
    }
    if !failed.is_empty() {
        return Err(format!(
            "{} file(s) could not be put back ({}{}). The copies are in {}.",
            failed.len(),
            failed[0],
            if failed.len() > 1 { " and more" } else { "" },
            files_dir.display()
        ));
    }
    journal.state = State::RolledBack;
    journal.save()
}

fn chrono_like_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---- The index: newest journal per game ------------------------------------

fn index_path(data_dir: &Path) -> PathBuf {
    data_dir.join("update-backups.json")
}

fn read_index(data_dir: &Path) -> std::collections::BTreeMap<String, String> {
    std::fs::read(index_path(data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Make this the game's newest. Says which backup it takes over from.
pub fn remember(data_dir: &Path, journal: &Journal) -> Option<String> {
    let mut index = read_index(data_dir);
    let previous = index.insert(journal.slug.clone(), journal.dir.clone());
    write_index(data_dir, &index);
    previous.filter(|p| *p != journal.dir)
}

fn write_index(data_dir: &Path, index: &std::collections::BTreeMap<String, String>) {
    if let Ok(text) = serde_json::to_string_pretty(index) {
        let _ = std::fs::create_dir_all(data_dir);
        let _ = std::fs::write(index_path(data_dir), text);
    }
}

/// A backup folder, when it really is this game's.
fn remove_backup(dir: &str, slug: &str) {
    if Journal::load(Path::new(dir)).is_some_and(|j| j.slug == slug) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The update is in for good, so the backup before it can never be used: Undo
/// goes back one. Every update of a big game left a copy that nothing listed
/// and nothing deleted.
pub fn drop_previous(journal: &Journal) {
    let (Some(dir), slug) = (journal.previous.clone(), journal.slug.clone()) else { return };
    std::thread::spawn(move || remove_backup(&dir, &slug));
}

/// The game was uninstalled: its backup has nothing left to go back to.
pub fn forget(data_dir: &Path, slug: &str) {
    let mut index = read_index(data_dir);
    if let Some(dir) = index.remove(slug) {
        write_index(data_dir, &index);
        remove_backup(&dir, slug);
    }
}

/// The newest journal for a game, if its copies are still where they were.
pub fn latest(data_dir: &Path, slug: &str) -> Option<Journal> {
    let dir = read_index(data_dir).get(slug)?.clone();
    Journal::load(Path::new(&dir))
}

/// Updates that never finished (closed or crashed mid-merge): roll each back.
/// Returns what was put back, for a notice.
pub fn recover_interrupted(data_dir: &Path) -> Vec<(Journal, Result<(), String>)> {
    let mut out = Vec::new();
    for dir in read_index(data_dir).into_values() {
        if let Some(mut j) = Journal::load(Path::new(&dir)) {
            if j.state == State::Applying {
                let res = restore(&mut j);
                out.push((j, res));
            }
        }
    }
    out
}

// ---- The app's side ---------------------------------------------------------

use tauri::{AppHandle, Emitter, Manager, Runtime};

pub fn data_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Run once at start: an update that was cut off mid-merge is rolled back,
/// and the player is told, rather than finding a half-old, half-new game.
pub fn recover_on_start<R: Runtime>(app: &AppHandle<R>) {
    let data = data_dir(app);
    let app = app.clone();
    std::thread::spawn(move || {
        for (j, res) in recover_interrupted(&data) {
            let (title, body) = match &res {
                Ok(()) => (
                    format!("{} was put back", j.title),
                    "Its update was interrupted, so the previous build was restored from your backup.".to_string(),
                ),
                Err(e) => (format!("{} needs a repair", j.title), format!("Its update was interrupted. {e}")),
            };
            crate::logging::error("game-update", &format!("{}: interrupted update, {}", j.title, res.as_ref().err().map(String::as_str).unwrap_or("rolled back")));
            let _ = app.emit("notify", crate::downloads::Notice::new(&title, &body, None));
        }
    });
}

/// The undo on offer for a game: its last update's journal, while the copies exist.
#[derive(Serialize)]
pub struct BackupInfo {
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub created_at: i64,
    pub files: usize,
    pub bytes: u64,
    pub dir: String,
    pub state: State,
}

#[tauri::command]
pub fn game_update_backup(app: AppHandle, slug: String) -> Option<BackupInfo> {
    let j = latest(&data_dir(&app), &slug)?;
    Path::new(&j.dir).join("files").is_dir().then(|| BackupInfo {
        from_version: j.from_version.clone(),
        to_version: j.to_version.clone(),
        created_at: j.created_at,
        files: j.replaced.len() + j.added.len(),
        bytes: j.bytes,
        dir: j.dir.clone(),
        state: j.state.clone(),
    })
}

/// Undo the last update: the old files back, the added ones gone, and the
/// library back on the old version (so the update is offered again).
#[tauri::command(async)]
pub fn game_update_undo(app: AppHandle, slug: String) -> Result<(), String> {
    let mut j = latest(&data_dir(&app), &slug).ok_or("There is no backup of this game's last update.")?;
    if j.state != State::Applied {
        return Err("This update was already undone.".into());
    }
    let games = crate::library::load(&app)?;
    let game = games.iter().find(|g| g.slug.as_deref() == Some(slug.as_str())).ok_or("That game is no longer in the library.")?;
    if crate::library::is_running(&app, &game.id) {
        return Err("Close the game first.".into());
    }
    // Moved since the update: the old files go where it is now. They went to
    // the folder it left, and the game stayed on the new build under the old
    // version number.
    let folders = crate::settings::all_folders(&crate::settings::load(&app));
    let dest = crate::storage::game_folder(&folders, &game.install_dir).unwrap_or_else(|| PathBuf::from(&game.install_dir));
    if !dest.is_dir() {
        return Err("The game's folder cannot be found.".into());
    }
    j.dest = dunce::simplified(&dest).to_string_lossy().into_owned();
    restore(&mut j)?;
    let from = j.from_version.clone();
    crate::library::update(&app, |games| {
        if let Some(g) = games.iter_mut().find(|g| g.slug.as_deref() == Some(slug.as_str())) {
            g.version = from;
        }
        Ok(())
    })?;
    let _ = app.emit("library-changed", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryoto-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(path: PathBuf, body: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn setup(d: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let game = d.join("Game");
        write(game.join("Game.exe"), b"v1");
        write(game.join("Data/level.bin"), b"L1");
        write(game.join("save.dat"), b"mine");
        let staging = d.join("staging");
        write(staging.join("Game.exe"), b"v2");
        write(staging.join("Data/level.bin"), b"L2");
        write(staging.join("Data/new.pak"), b"N");
        (game, staging, d.join("backups"))
    }

    fn prep<'a>(game: &'a Path, staging: &'a Path, root: &'a Path) -> Prepare<'a> {
        Prepare {
            slug: "game",
            title: "The Game: Deluxe",
            staging,
            dest: game,
            backup_root: root,
            from_version: Some("1.0".into()),
            to_version: Some("1.1".into()),
        }
    }

    #[test]
    fn backs_up_only_what_the_update_replaces() {
        let d = scratch("plan");
        let (game, staging, root) = setup(&d);
        let j = prepare(prep(&game, &staging, &root)).unwrap();
        assert_eq!(j.replaced, vec!["Data/level.bin", "Game.exe"]);
        assert_eq!(j.added, vec!["Data/new.pak"]);
        assert_eq!(j.state, State::Applying);
        let files = Path::new(&j.dir).join("files");
        assert_eq!(std::fs::read(files.join("Game.exe")).unwrap(), b"v1");
        assert!(!files.join("save.dat").exists(), "saves are not part of the update, so not copied");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_rollback_puts_the_old_build_back_and_keeps_saves() {
        let d = scratch("undo");
        let (game, staging, root) = setup(&d);
        let mut j = prepare(prep(&game, &staging, &root)).unwrap();
        crate::downloads::move_tree(&staging, &game).unwrap();
        std::fs::write(game.join("save.dat"), b"played after the update").unwrap();
        assert_eq!(std::fs::read(game.join("Game.exe")).unwrap(), b"v2");
        restore(&mut j).unwrap();
        assert_eq!(std::fs::read(game.join("Game.exe")).unwrap(), b"v1");
        assert_eq!(std::fs::read(game.join("Data/level.bin")).unwrap(), b"L1");
        assert!(!game.join("Data/new.pak").exists());
        assert_eq!(std::fs::read(game.join("save.dat")).unwrap(), b"played after the update");
        assert_eq!(Journal::load(Path::new(&j.dir)).unwrap().state, State::RolledBack);
        // Twice is harmless.
        restore(&mut j).unwrap();
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_interrupted_update_is_rolled_back_on_the_next_start() {
        let d = scratch("crash");
        let (game, staging, root) = setup(&d);
        let data = d.join("appdata");
        let j = prepare(prep(&game, &staging, &root)).unwrap();
        remember(&data, &j);
        // Half a merge: one file replaced, then the app died.
        std::fs::write(game.join("Game.exe"), b"v2").unwrap();
        let done = recover_interrupted(&data);
        assert_eq!(done.len(), 1);
        assert!(done[0].1.is_ok());
        assert_eq!(std::fs::read(game.join("Game.exe")).unwrap(), b"v1");
        assert!(recover_interrupted(&data).is_empty(), "nothing left to recover");
        assert_eq!(latest(&data, "game").unwrap().state, State::RolledBack);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn only_the_newest_backup_is_kept_and_it_goes_with_the_game() {
        let d = scratch("prune");
        let (game, staging, root) = setup(&d);
        let data = d.join("appdata");
        // Another title for a folder of its own: two in one second would share.
        let first = prepare(Prepare { title: "The Game", ..prep(&game, &staging, &root) }).unwrap();
        assert_eq!(remember(&data, &first), None);
        let mut second = prepare(prep(&game, &staging, &root)).unwrap();
        second.previous = remember(&data, &second);
        assert_eq!(second.previous.as_deref(), Some(first.dir.as_str()));
        remove_backup(second.previous.as_deref().unwrap(), &second.slug);
        assert!(!Path::new(&first.dir).exists());
        assert!(Path::new(&second.dir).exists());
        // Somebody else's folder, named by a damaged index, is left alone.
        remove_backup(&second.dir, "another-game");
        assert!(Path::new(&second.dir).exists());
        forget(&data, "game");
        assert!(!Path::new(&second.dir).exists());
        assert!(latest(&data, "game").is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_unwritable_backup_folder_stops_the_update_before_anything_changes() {
        let d = scratch("nofolder");
        let (game, staging, _) = setup(&d);
        // A file where the folder should be.
        let blocked = d.join("blocked");
        std::fs::write(&blocked, b"x").unwrap();
        assert!(prepare(prep(&game, &staging, &blocked)).is_err());
        assert_eq!(std::fs::read(game.join("Game.exe")).unwrap(), b"v1");
        let _ = std::fs::remove_dir_all(d);
    }
}
