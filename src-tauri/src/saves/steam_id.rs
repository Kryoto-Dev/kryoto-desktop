//! The Steam id gbe_fork plays a game as, and the game's saves when it changes.
//!
//! KB-0376 (2026-11-22): a player's Silksong saves made as a guest seemed gone
//! after signing in. Silksong files its saves in a folder named after the
//! Steam id (`LocalLow/Team Cherry/Hollow Knight Silksong/<id>/`). As a guest,
//! gbe_fork used its own id (made once per PC, kept in its global settings);
//! the first Play signed in wrote the account's id into the game's
//! `configs.user.ini` (`sync.rs` `pin_steam_id`), so the game started in an
//! empty folder. Nothing was deleted: the saves sat under the old id.
//!
//! Now the id is only pinned where cloud saves use it (a supporter), and
//! never over saves made under another id without copying them across first
//! ([`carry`]); a game with saves Kryoto cannot locate keeps the id it has.
//! For players it already happened to, Play offers to bring the earlier saves
//! over ([`stranded`], [`bring_over`]), backing up both folders first.

use std::path::{Path, PathBuf};

/// A 64-bit Steam id as gbe_fork writes them: `7656...`, 17 digits.
pub fn is_steam_id(s: &str) -> bool {
    s.len() == 17 && s.starts_with("7656") && s.bytes().all(|b| b.is_ascii_digit())
}

/// The id gbe_fork plays a game as on this PC when the game's own
/// `steam_settings/configs.user.ini` names none: the one in its global
/// settings under `app_data` (AppData\Roaming, or the Wine prefix's), which
/// it makes on its first run. Goldberg's older `user_steam_id.txt` too.
pub fn emulator_default(app_data: &Path) -> Option<String> {
    for base in ["GSE Saves", "Goldberg SteamEmu Saves"] {
        let settings = app_data.join(base).join("settings");
        if let Ok(text) = std::fs::read(settings.join("configs.user.ini")) {
            let text = String::from_utf8_lossy(&text);
            if let Some(id) = kryoto_saves::emulator::ini_get(&text, "user::general", "account_steamid").filter(|i| is_steam_id(i)) {
                return Some(id);
            }
        }
        if let Some(id) = std::fs::read_to_string(settings.join("user_steam_id.txt")).ok().map(|t| t.trim().to_string()).filter(|i| is_steam_id(i)) {
            return Some(id);
        }
    }
    None
}

/// The id the game's own config pins, if any (the first `steam_settings` that names one).
pub fn pinned(game_dir: &Path) -> Option<String> {
    kryoto_saves::emulator::steam_settings_dirs(game_dir).into_iter().find_map(|dir| {
        let text = std::fs::read(dir.join("configs.user.ini")).ok()?;
        kryoto_saves::emulator::ini_get(&String::from_utf8_lossy(&text), "user::general", "account_steamid")
    })
}

/// Folders named after a Steam id in or around the game's save folders
/// (`save_dirs`: where its save rules point, ids left as wildcards): the
/// folder itself, up to two levels above it and two below. Sorted, unique.
pub fn id_folders(save_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let named = |p: &Path| p.file_name().is_some_and(|n| is_steam_id(&n.to_string_lossy()));
    let mut out = Vec::new();
    for dir in save_dirs {
        out.extend(dir.ancestors().take(3).filter(|a| named(a) && a.is_dir()).map(Path::to_path_buf));
        out.extend(
            walkdir::WalkDir::new(dir)
                .min_depth(1)
                .max_depth(2)
                .follow_links(false)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_dir() && named(e.path()))
                .map(|e| e.into_path()),
        );
    }
    out.sort();
    out.dedup();
    out
}

/// Files, bytes and the newest change (unix seconds) under `dir`.
pub fn measure(dir: &Path) -> (u64, u64, u64) {
    let mut out = (0, 0, 0);
    for e in walkdir::WalkDir::new(dir).follow_links(false).into_iter().filter_map(Result::ok).filter(|e| e.file_type().is_file()) {
        let Ok(m) = e.metadata() else { continue };
        out.0 += 1;
        out.1 += m.len();
        let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        out.2 = out.2.max(t);
    }
    out
}

/// The same folder under another id: `<parent>/<id>`.
fn twin(folder: &Path, id: &str) -> Option<PathBuf> {
    Some(folder.parent()?.join(id))
}

/// Before the game is pinned to `new`: copy every save folder named `old`
/// to one named `new` beside it, where that has nothing yet. A copy, never a
/// move, and never over anything. How many folders were copied.
pub fn carry(save_dirs: &[PathBuf], old: &str, new: &str) -> Result<usize, String> {
    let mut copied = 0;
    for from in id_folders(save_dirs).into_iter().filter(|f| f.file_name().is_some_and(|n| n.to_string_lossy() == old)) {
        let Some(to) = twin(&from, new) else { continue };
        if to.is_dir() && measure(&to).0 > 0 {
            continue;
        }
        copy_tree(&from, &to, false).map_err(|e| format!("Could not copy the saves in {} to {}: {e}", from.display(), to.display()))?;
        copied += 1;
    }
    Ok(copied)
}

/// For a game pinned to `pinned` whose saves stayed under another id (the
/// KB-0376 case): `(earlier, pinned folder)`, when the pinned id's folder is
/// empty, or newer and smaller than the earlier one, which has saves.
pub fn stranded(save_dirs: &[PathBuf], pinned: &str) -> Option<(PathBuf, PathBuf)> {
    let folders = id_folders(save_dirs);
    for to in folders.iter().filter(|f| f.file_name().is_some_and(|n| n.to_string_lossy() == pinned)) {
        let (files, bytes, newest) = measure(to);
        let earlier = folders
            .iter()
            .filter(|o| *o != to && o.parent() == to.parent())
            .map(|o| (o, measure(o)))
            .filter(|(_, (f, _, _))| *f > 0)
            .max_by_key(|(_, (_, _, t))| *t);
        if let Some((from, (_, from_bytes, from_newest))) = earlier {
            if files == 0 || (newest >= from_newest && bytes < from_bytes) {
                return Some((from.clone(), to.clone()));
            }
        }
    }
    // The pinned id has no folder at all yet.
    let mut parents: Vec<&Path> = folders.iter().filter_map(|f| f.parent()).collect();
    parents.dedup();
    for parent in parents {
        let to = parent.join(pinned);
        if to.exists() {
            continue;
        }
        let from = folders.iter().filter(|o| o.parent() == Some(parent)).map(|o| (o, measure(o))).filter(|(_, m)| m.0 > 0).max_by_key(|(_, m)| m.2);
        if let Some((from, _)) = from {
            return Some((from.clone(), to));
        }
    }
    None
}

/// "Bring them over": both folders backed up into `backup` first, then the
/// earlier saves copied into the pinned id's folder, over what is there.
pub fn bring_over(from: &Path, to: &Path, backup: &Path) -> Result<(), String> {
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    copy_tree(from, &backup.join(format!("earlier-{}", name(from))), false).map_err(|e| format!("Could not back up {}: {e}", from.display()))?;
    if to.is_dir() {
        copy_tree(to, &backup.join(format!("current-{}", name(to))), false).map_err(|e| format!("Could not back up {}: {e}", to.display()))?;
    }
    copy_tree(from, to, true).map_err(|e| format!("Could not copy the saves to {}: {e}", to.display()))
}

/// Copy a folder's files, never following a link. `over`: replace files
/// already there; otherwise leave them.
fn copy_tree(from: &Path, to: &Path, over: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let (kind, target) = (e.file_type()?, to.join(e.file_name()));
        if kind.is_dir() {
            copy_tree(&e.path(), &target, over)?;
        } else if kind.is_file() && (over || !target.exists()) {
            std::fs::copy(e.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "76561197960287930";
    const NEW: &str = "76561198000000002";

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryoto-steam-id-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn the_emulators_own_id_is_read_from_its_settings() {
        let app = scratch("default");
        assert_eq!(emulator_default(&app), None);
        put(&app.join("Goldberg SteamEmu Saves/settings/user_steam_id.txt"), format!("{OLD}\r\n").as_bytes());
        assert_eq!(emulator_default(&app).as_deref(), Some(OLD));
        put(&app.join("GSE Saves/settings/configs.user.ini"), format!("[user::general]\naccount_name=Guest\naccount_steamid={NEW}\n").as_bytes());
        assert_eq!(emulator_default(&app).as_deref(), Some(NEW));
        put(&app.join("GSE Saves/settings/configs.user.ini"), b"[user::general]\naccount_steamid=nonsense\n");
        assert_eq!(emulator_default(&app).as_deref(), Some(OLD));
        let _ = std::fs::remove_dir_all(app);
    }

    /// Silksong's layout: the saves folder holds one folder per Steam id.
    fn silksong(base: &Path) -> PathBuf {
        let saves = base.join("LocalLow/Team Cherry/Hollow Knight Silksong");
        put(&saves.join(OLD).join("user1.dat"), &[1u8; 4000]);
        put(&saves.join(OLD).join("user1.dat.bak1"), &[1u8; 4000]);
        saves
    }

    #[test]
    fn carrying_copies_the_earlier_ids_saves_and_keeps_them() {
        let base = scratch("carry");
        let saves = silksong(&base);
        assert_eq!(carry(std::slice::from_ref(&saves), OLD, NEW).unwrap(), 1);
        assert!(saves.join(NEW).join("user1.dat").is_file() && saves.join(OLD).join("user1.dat").is_file());
        // Never over saves the new id already has.
        put(&saves.join(NEW).join("user1.dat"), b"newer");
        assert_eq!(carry(std::slice::from_ref(&saves), OLD, NEW).unwrap(), 0);
        assert_eq!(std::fs::read(saves.join(NEW).join("user1.dat")).unwrap(), b"newer");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn stranded_saves_are_found_and_brought_over_with_both_backed_up() {
        let base = scratch("stranded");
        let saves = silksong(&base);
        // Pinned, and the game has not made the new folder yet.
        assert_eq!(stranded(std::slice::from_ref(&saves), NEW), Some((saves.join(OLD), saves.join(NEW))));
        // It started a new game there: newer, and smaller.
        put(&saves.join(NEW).join("user1.dat"), b"fresh");
        let (from, to) = stranded(std::slice::from_ref(&saves), NEW).unwrap();
        assert_eq!((from.clone(), to.clone()), (saves.join(OLD), saves.join(NEW)));
        let backup = base.join("backup");
        bring_over(&from, &to, &backup).unwrap();
        assert_eq!(std::fs::read(saves.join(NEW).join("user1.dat")).unwrap().len(), 4000);
        assert_eq!(std::fs::read(backup.join(format!("current-{NEW}")).join("user1.dat")).unwrap(), b"fresh");
        assert!(backup.join(format!("earlier-{OLD}")).join("user1.dat").is_file());
        assert!(saves.join(OLD).join("user1.dat").is_file(), "the earlier saves stay");
        // Now the pinned folder has them: nothing to offer.
        assert_eq!(stranded(std::slice::from_ref(&saves), NEW), None);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_pinned_folder_that_is_older_or_bigger_is_left_alone() {
        let base = scratch("fine");
        let saves = silksong(&base);
        put(&saves.join(NEW).join("user1.dat"), &[2u8; 9000]);
        assert_eq!(stranded(std::slice::from_ref(&saves), NEW), None);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn id_folders_are_found_above_and_below_a_save_folder() {
        let base = scratch("ids");
        let deep = base.join("Game").join(OLD).join("saves");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(base.join("Other").join("profiles").join(NEW)).unwrap();
        assert_eq!(id_folders(std::slice::from_ref(&deep)), vec![base.join("Game").join(OLD)]);
        assert_eq!(id_folders(&[base.join("Other")]), vec![base.join("Other").join("profiles").join(NEW)]);
        assert!(!is_steam_id("12345") && is_steam_id(OLD));
        let _ = std::fs::remove_dir_all(base);
    }
}
