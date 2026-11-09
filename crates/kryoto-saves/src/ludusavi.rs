//! A Ludusavi backup on disk, as save files Kryoto can pack: what another
//! launcher left (Hydra keeps one per game under `hydralauncher/Backups`,
//! made by Ludusavi before it uploads to its own cloud).
//!
//! Ludusavi's "simple" layout: a folder per game with `mapping.yaml`, which
//! names each original path (`C:/Users/x/AppData/Roaming/Game/save.dat`) and
//! the drive folders they are stored under (`drive-C` for `C:`, `drive-0` for
//! `/` on Linux). A full backup is stored in the folder its `name` says (`.`
//! for the game folder itself); differential `children` hold only what
//! changed after it, and a file set to nothing was deleted. The newest full
//! backup and its children together are the saves as last backed up.
//!
//! Only read: the files are returned where they are, as [`Found`] with
//! their real place (root and path) worked out from the original path.

use crate::scan::Found;
use crate::SaveRoots;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Mapping {
    drives: BTreeMap<String, String>,
    backups: Vec<Backup>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Backup {
    name: String,
    when: Option<String>,
    files: BTreeMap<String, Option<serde_yaml::Value>>,
    children: Vec<Backup>,
}

/// The folders under `root` that hold a Ludusavi game backup (`mapping.yaml`):
/// `root` itself, or one level down (Ludusavi names that level after the game).
pub fn game_folders(root: &Path) -> Vec<PathBuf> {
    if root.join("mapping.yaml").is_file() {
        return vec![root.to_path_buf()];
    }
    let mut out: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.join("mapping.yaml").is_file()).collect())
        .unwrap_or_default();
    out.sort();
    out
}

/// Where Ludusavi stored `original`: under the drive folder whose drive the
/// path starts with (`C:` or, on Linux, the empty root).
fn stored(base: &Path, drives: &BTreeMap<String, String>, original: &str) -> Option<PathBuf> {
    let path = original.replace('\\', "/");
    let (folder, rest) = drives
        .iter()
        .filter_map(|(folder, drive)| {
            let rest = if drive.is_empty() { path.strip_prefix('/')? } else { path.strip_prefix(drive.as_str())?.trim_start_matches('/') };
            Some((folder, rest, drive.len()))
        })
        .max_by_key(|(_, _, len)| *len)
        .map(|(f, r, _)| (f.clone(), r.to_string()))?;
    if rest.is_empty() || rest.split('/').any(|p| p == "..") {
        return None;
    }
    Some(base.join(folder).join(rest))
}

/// The save files of the backup in `game_folder`, as it last stood, placed
/// under `roots`. Files whose original place is under none of the roots
/// (another user's folder, another drive than the game's) are left out.
/// Also returns when that backup was made, as Ludusavi wrote it.
pub fn files(game_folder: &Path, roots: &SaveRoots) -> (Vec<Found>, Option<String>) {
    let Some(mapping) = std::fs::read_to_string(game_folder.join("mapping.yaml")).ok().and_then(|t| serde_yaml::from_str::<Mapping>(&t).ok()) else {
        return (Vec::new(), None);
    };
    let Some(full) = mapping.backups.last() else { return (Vec::new(), None) };
    // original path -> where its newest copy is (None: deleted since).
    let mut state: BTreeMap<String, Option<PathBuf>> = BTreeMap::new();
    let mut when = full.when.clone();
    for (orig, _) in &full.files {
        let dir = if full.name == "." || full.name.is_empty() { game_folder.to_path_buf() } else { game_folder.join(&full.name) };
        state.insert(orig.clone(), stored(&dir, &mapping.drives, orig));
    }
    for child in &full.children {
        let dir = game_folder.join(&child.name);
        for (orig, info) in &child.files {
            state.insert(orig.clone(), info.as_ref().and_then(|_| stored(&dir, &mapping.drives, orig)));
        }
        when = child.when.clone().or(when);
    }
    let mut out = Vec::new();
    for (orig, at) in state {
        let Some(abs) = at.filter(|p| p.is_file()) else { continue };
        let Some((root, path)) = roots.placeholder_of(Path::new(&orig)) else { continue };
        let meta = std::fs::metadata(&abs).ok();
        let mtime = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        out.push(Found { root, path, size: meta.map(|m| m.len()).unwrap_or(0), mtime, abs, rule: "ludusavi-backup".into() });
    }
    (out, when)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Root;

    #[test]
    fn a_hydra_backup_becomes_files_in_their_real_place() {
        let base = std::env::temp_dir().join(format!("kryoto-ludusavi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        let game = base.join("Backups").join("steam-1145360").join("Hades");
        // What Ludusavi wrote: the full backup in the game folder, a change
        // and a deletion in a child backup.
        let drive = if cfg!(windows) { "drive-C" } else { "drive-0" };
        let home_str = home.to_string_lossy().replace('\\', "/");
        let (drive_name, home_rel) = if cfg!(windows) { (home_str[..2].to_string(), home_str[3..].to_string()) } else { (String::new(), home_str[1..].to_string()) };
        let save = |dir: &Path, rel: &str, body: &str| {
            let p = dir.join(drive).join(&home_rel).join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        save(&game, "Saved Games/Hades/Profile1.sav", "old");
        save(&game, "Saved Games/Hades/Profile2.sav", "two");
        save(&game.join("backup-20261001T000000Z"), "Saved Games/Hades/Profile1.sav", "new");
        let orig = |rel: &str| format!("{home_str}/{rel}");
        let yaml = format!(
            "name: Hades\ndrives:\n  {drive}: \"{drive_name}\"\nbackups:\n  - name: \".\"\n    when: \"2026-09-01T00:00:00Z\"\n    files:\n      \"{a}\":\n        hash: x\n        size: 3\n      \"{b}\":\n        hash: y\n        size: 3\n      \"{c}\":\n        hash: z\n        size: 1\n    children:\n      - name: \"backup-20261001T000000Z\"\n        when: \"2026-10-01T00:00:00Z\"\n        files:\n          \"{a}\":\n            hash: q\n            size: 3\n          \"{b}\": ~\n",
            a = orig("Saved Games/Hades/Profile1.sav"),
            b = orig("Saved Games/Hades/Profile2.sav"),
            c = "Z:/Elsewhere/x.sav",
        );
        std::fs::write(game.join("mapping.yaml"), yaml).unwrap();
        let roots = SaveRoots { home: Some(home.clone()), ..Default::default() };
        assert_eq!(game_folders(&base.join("Backups").join("steam-1145360")), vec![game.clone()]);
        let (found, when) = files(&game, &roots);
        assert_eq!(when.as_deref(), Some("2026-10-01T00:00:00Z"));
        // The child's newer Profile1, Profile2 deleted, the other drive left out.
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].root, found[0].path.as_str()), (Root::Home, "Saved Games/Hades/Profile1.sav"));
        assert_eq!(std::fs::read_to_string(&found[0].abs).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn paths_never_climb_out() {
        let drives: BTreeMap<String, String> = [("drive-C".to_string(), "C:".to_string())].into();
        assert_eq!(stored(Path::new("B"), &drives, "C:/a/../../x"), None);
        assert_eq!(stored(Path::new("B"), &drives, "D:/a"), None);
        assert_eq!(stored(Path::new("B"), &drives, "C:/Users/x/f.sav"), Some(Path::new("B").join("drive-C").join("Users/x/f.sav")));
    }
}
