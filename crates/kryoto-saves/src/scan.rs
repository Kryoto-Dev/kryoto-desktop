//! From rules to files: what a backup of a game's saves holds.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::rules::{expand, relative, wildcard, Kind, Root, Rule, SaveRoots};

/// Never part of a save, wherever a rule points: logs, caches of every name
/// (`mapcache`, `ShaderCache`...), crash dumps, screenshots, temporary files,
/// and Steam Workshop downloads (fetched again, and often gigabytes). Matched
/// against each name below a save folder, case-insensitively.
pub const DEFAULT_EXCLUDES: [&str; 17] = [
    "*.tmp", "*.temp", "*.dmp", "*.log", "*.lock", "*.crdownload", "logs", "log", "crashes", "crashdumps", "crash reports", "*cache",
    "*caches", "temp", "screenshots", "workshop", "Unity",
];

/// In the game's own folder: folders that came with the game, not saves.
pub const SHIPPED_DIRS: &[&str] = &[
    "steam_settings", "engine", "binaries", "content", "plugins", "mono", "monobleedingedge", "_commonredist", "redist", "redistributables",
    "__pycache__", "shadercache", "logs", "crashreportclient", "directx", "support", "paks",
];
/// In the game's own folder: the kinds of file a game ships, never saves.
pub const SHIPPED_EXT: &[&str] = &[
    "exe", "dll", "pak", "ucas", "utoc", "so", "pdb", "sig", "assets", "resource", "ress", "bundle", "pck", "rpa", "rgss3a", "wad", "bik",
    "bk2", "usm", "awb", "acb", "ogg", "wem", "bnk", "uasset", "umap", "ubulk",
];

/// Whether a path in the game's own folder (relative to that folder, or to a
/// save location in it) is what the game shipped with: its programs, packed
/// data, engine and plugin folders. A location in the game's folder never
/// backs these up, however wide it is - a game named like its own folder
/// (Unreal's `DiggingGame`) once took 1.7 GB of `Binaries` along with a 6 MB save.
pub fn shipped(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let Some((name, dirs)) = parts.split_last() else { return false };
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    SHIPPED_EXT.contains(&ext.as_str())
        || name.starts_with(".kryoto")
        || name.eq_ignore_ascii_case("steam_appid.txt")
        || dirs.iter().any(|d| SHIPPED_DIRS.contains(&d.to_ascii_lowercase().as_str()) || d.starts_with(".kryoto"))
}

/// Why kryo.to would not take this path, or None. Windows never makes a name
/// that ends in a space or a dot, but a Wine prefix can; one such file is
/// left out rather than failing the whole backup.
pub fn unsendable(rel: &str) -> bool {
    rel.len() > 400 || rel.split('/').any(|s| s.is_empty() || s.len() > 255 || s.ends_with(' ') || s.ends_with('.'))
}

/// Files a backup never carries, even inside a save folder: programs and
/// scripts, and video. Saves are never these, and kryo.to refuses them.
pub const BLOCKED_EXTENSIONS: [&str; 18] =
    ["exe", "dll", "msi", "bat", "cmd", "ps1", "scr", "com", "vbs", "sys", "mp4", "mkv", "avi", "mov", "webm", "wmv", "flv", "m4v"];

/// At most this many files in one backup (kryo.to's limit too).
pub const MAX_FILES: usize = 5000;

pub fn blocked(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    BLOCKED_EXTENSIONS.contains(&ext.as_str())
}

/// Whether a path below a save folder is left out by `excludes` (names, or
/// relative patterns with `/`).
pub fn excluded(rel: &str, excludes: &[String]) -> bool {
    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let by_name = |pat: &str| parts.iter().any(|p| wildcard(pat, p));
    DEFAULT_EXCLUDES.iter().any(|p| by_name(p))
        || excludes.iter().any(|p| {
            let p = p.trim_matches('/');
            if p.contains('/') {
                // A relative pattern: the path itself or anything under it.
                let depth = p.split('/').count();
                parts.len() >= depth && wildcard(p, &parts[..depth].join("/"))
            } else {
                by_name(p)
            }
        })
}

/// One file of a game's saves on this PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub root: Root,
    /// Relative to the root, with `/`.
    pub path: String,
    pub abs: PathBuf,
    pub size: u64,
    /// Unix seconds.
    pub mtime: u64,
    /// Which rule (its [`Rule::key`]) found it.
    pub rule: String,
}

#[derive(Debug, Clone, Default)]
pub struct Collected {
    pub files: Vec<Found>,
    pub bytes: u64,
    /// Files left out as programs, video or past the limits.
    pub skipped: usize,
    /// More files than a backup may hold.
    pub truncated: bool,
}

impl Collected {
    /// Files and bytes each rule found, by [`Rule::key`].
    pub fn per_rule(&self) -> HashMap<String, (usize, u64)> {
        let mut out: HashMap<String, (usize, u64)> = HashMap::new();
        for f in &self.files {
            let e = out.entry(f.rule.clone()).or_default();
            e.0 += 1;
            e.1 += f.size;
        }
        out
    }
}

fn mtime_of(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Every file the rules name on this PC, at most `max_files`.
pub fn collect(rules: &[Rule], roots: &SaveRoots, max_files: usize) -> Collected {
    let mut out = Collected::default();
    let mut seen: HashSet<(Root, String)> = HashSet::new();
    for rule in rules.iter().filter(|r| r.kind == Kind::Files && r.problem().is_none()) {
        let Some(root_dir) = roots.get(rule.root) else { continue };
        for base in expand(rule, roots) {
            let in_game = rule.root == Root::GameDir;
            let walker = walkdir::WalkDir::new(&base).follow_links(false).into_iter().filter_entry(|e| {
                // Excludes apply below the save folder, not to its own name.
                let rel = relative(&base, e.path()).unwrap_or_default();
                rel.is_empty() || !(excluded(&rel, &rule.excludes) || (in_game && e.file_type().is_dir() && shipped(&format!("{rel}/x"))))
            });
            for entry in walker.filter_map(|e| e.ok()) {
                if !entry.file_type().is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy();
                let Ok(meta) = entry.metadata() else { continue };
                let Some(rel) = relative(root_dir, entry.path()) else { continue };
                let in_rule = relative(&base, entry.path()).unwrap_or_default();
                if blocked(&name) || unsendable(&rel) || crate::rules::path_problem(&rel).is_some() || (in_game && shipped(&in_rule)) {
                    out.skipped += 1;
                    continue;
                }
                if !seen.insert((rule.root, rel.to_lowercase())) {
                    continue;
                }
                if out.files.len() >= max_files {
                    out.truncated = true;
                    out.skipped += 1;
                    continue;
                }
                out.bytes += meta.len();
                out.files.push(Found { root: rule.root, path: rel, abs: entry.path().to_path_buf(), size: meta.len(), mtime: mtime_of(&meta), rule: rule.key() });
            }
        }
    }
    out.files.sort_by_key(|f| (f.root, f.path.to_lowercase()));
    out
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// File hashes remembered by size and modified time, so a game's unchanged
/// saves are not read again on every check.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct HashCache(pub HashMap<String, (u64, u64, String)>);

impl HashCache {
    pub fn hash(&mut self, f: &Found) -> std::io::Result<String> {
        let key = f.abs.to_string_lossy().to_string();
        if let Some((size, mtime, sha)) = self.0.get(&key) {
            if *size == f.size && *mtime == f.mtime && f.mtime != 0 {
                return Ok(sha.clone());
            }
        }
        let sha = sha256_file(&f.abs)?;
        self.0.insert(key, (f.size, f.mtime, sha.clone()));
        Ok(sha)
    }

    /// Forget files that are no longer there.
    pub fn retain(&mut self, files: &[Found]) {
        let keep: HashSet<String> = files.iter().map(|f| f.abs.to_string_lossy().to_string()).collect();
        self.0.retain(|k, _| keep.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Source;

    #[test]
    fn caches_of_every_name_and_workshop_downloads_are_not_saves() {
        assert!(excluded("GB27/mapcache/x.gbmc.tex", &[]));
        assert!(excluded("Game/ShaderCache/a.bin", &[]));
        assert!(excluded("Game/workshop/123/map.bin", &[]));
        assert!(!excluded("GB27/Saves/ [Pillars].gbsave", &[]));
        assert!(!excluded("Game/cached_state.sav", &[]), "a file whose name only starts with cache is kept");
    }

    #[test]
    fn a_wide_location_in_the_game_folder_never_takes_what_shipped() {
        let dir = std::env::temp_dir().join(format!("kryo-saves-scan-shipped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let game = dir.join("A Game About Digging A Hole");
        for d in ["DiggingGame/Binaries/Win64", "DiggingGame/Content/Paks", "DiggingGame/Saved/SaveGames", "DiggingGame/Saved/Config/CrashReportClient"] {
            std::fs::create_dir_all(game.join(d)).unwrap();
        }
        std::fs::write(game.join("DiggingGame/Binaries/Win64/DiggingGame-Win64-Shipping.pdb"), vec![0u8; 4096]).unwrap();
        std::fs::write(game.join("DiggingGame/Content/Paks/pakchunk0.pak"), vec![0u8; 4096]).unwrap();
        std::fs::write(game.join("DiggingGame/Saved/SaveGames/MainSave.sav"), b"dig").unwrap();
        std::fs::write(game.join("DiggingGame/Saved/Config/CrashReportClient/x.ini"), b"x").unwrap();
        let roots = SaveRoots { game_dir: Some(game.clone()), ..Default::default() };
        let c = collect(&[Rule::files(Root::GameDir, "DiggingGame", Source::Detected)], &roots, MAX_FILES);
        let paths: Vec<&str> = c.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["DiggingGame/Saved/SaveGames/MainSave.sav"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_leading_space_is_a_real_name_but_a_trailing_one_is_not() {
        assert!(!unsendable("F2Games/GoreBox/Saves/ [Pillars].gbsave"));
        assert!(unsendable("Game/saves /a.sav"));
        assert!(unsendable("Game/a.sav."));
    }

    #[test]
    fn excludes_match_names_and_relative_patterns() {
        assert!(excluded("Logs/today.txt", &[]));
        assert!(excluded("slot1.sav.tmp", &[]));
        assert!(!excluded("Saves/slot1.sav", &[]));
        assert!(excluded("remote/achievements.json", &["achievements.json".into()]));
        assert!(excluded("Saved/Config/Windows/Game.ini", &["Saved/Config".into()]));
        assert!(!excluded("Saved/SaveGames/0.sav", &["Saved/Config".into()]));
    }

    #[test]
    fn collects_saves_but_not_programs_caches_or_excluded_files() {
        let dir = std::env::temp_dir().join(format!("kryo-saves-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let game = dir.join("Game");
        std::fs::create_dir_all(game.join("Saves/cache")).unwrap();
        std::fs::create_dir_all(game.join("Saves/remote")).unwrap();
        std::fs::write(game.join("Saves/slot1.sav"), b"one").unwrap();
        std::fs::write(game.join("Saves/remote/data.bin"), b"two").unwrap();
        std::fs::write(game.join("Saves/remote/achievements.json"), b"{}").unwrap();
        std::fs::write(game.join("Saves/cache/x.bin"), b"cache").unwrap();
        std::fs::write(game.join("Saves/evil.exe"), b"MZ").unwrap();
        std::fs::write(game.join("Saves/debug.log"), b"log").unwrap();
        let roots = SaveRoots { documents: Some(dir.clone()), ..Default::default() };
        let rules = vec![Rule::files(Root::Documents, "Game/Saves", Source::Known).excluding(&["achievements.json"])];
        let c = collect(&rules, &roots, MAX_FILES);
        let paths: Vec<&str> = c.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["Game/Saves/remote/data.bin", "Game/Saves/slot1.sav"]);
        assert_eq!(c.bytes, 6);
        assert_eq!(c.skipped, 1, "the exe is counted as left out");
        let capped = collect(&rules, &roots, 1);
        assert!(capped.truncated && capped.files.len() == 1);
        let mut cache = HashCache::default();
        let first = cache.hash(&c.files[1]).unwrap();
        assert_eq!(first, sha256_file(&game.join("Saves/slot1.sav")).unwrap());
        assert_eq!(cache.hash(&c.files[1]).unwrap(), first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_rule_and_a_folder_rule_do_not_double_count() {
        let dir = std::env::temp_dir().join(format!("kryo-saves-scan2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("G")).unwrap();
        std::fs::write(dir.join("G/Save1.rvdata2"), b"1").unwrap();
        let roots = SaveRoots { game_dir: Some(dir.join("G")), ..Default::default() };
        let rules = vec![Rule::files(Root::GameDir, "Save*.rvdata2", Source::Engine), Rule::files(Root::GameDir, "Save1.rvdata2", Source::Player)];
        assert_eq!(collect(&rules, &roots, MAX_FILES).files.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
