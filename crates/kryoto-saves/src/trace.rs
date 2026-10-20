//! Telling a game's save folder apart from everything else that changed on
//! the PC while it ran.
//!
//! Kryoto Desktop watches the user's folders and the game's folder while a
//! game runs, and afterwards looks at what was written ([`Observation`]): the
//! files, the folders that are new, the files the game's own processes held
//! open, and the registry keys that were written. Most of that is not the
//! game - browsers, chat apps, shader caches, Windows. This turns it into
//! save locations ([`Candidate`]): caches and system folders are dropped, each
//! write is collapsed to the folder that belongs to the game (the one named
//! after it, or the deepest folder its files share), and each folder is scored
//! on how sure we are. Corroborated ones sync on their own; the rest are
//! offered to the player as possible save locations.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::rules::{relative, squash, wildcard, GameHints, Root, Rule, SaveRoots, Source};

/// A candidate at or above this score syncs without asking.
pub const CORROBORATED: i32 = 50;
/// Below this, a candidate is not even offered.
pub const SUGGEST: i32 = 15;

/// Folder names that are never a game's saves, anywhere under a root.
const NEVER: [&str; 58] = [
    "temp", "tmp", "inetcache", "inetcookies", "crashdumps", "crashpad", "crashreporter", "crash reports", "d3dscache", "dxcache", "glcache",
    "nvidia", "nvidia corporation", "amd", "ati", "intel", "microsoft", "packages", "google", "mozilla", "bravesoftware", "opera software",
    "vivaldi", "discord", "discordptb", "discordcanary", "spotify", "slack", "teams", "zoom", "steam", "epic games", "epicgameslauncher",
    "gog.com", "onedrive", "dropbox", "programs", "npm-cache", "npm", "pip", "node_modules", ".cache", "cef", "ebwebview", "webview2",
    "squirreltemp", "connecteddevicesplatform", "comms", "history", "recent", "unrealengine", "nuget", "jetbrains", "code", "obs-studio",
    "kryoto", "kryoto desktop", "to.kryo.desktop*",
];

/// The first folder under a root that is a container for many games' saves,
/// not one game's.
const CONTAINERS: [&str; 9] = ["my games", "games", "saved games", "savegames", "saves", "save", "savedata", "save data", "game saves"];

/// In the game's own folder: what came with the game, not what it saved.
const SHIPPED_DIRS: [&str; 16] = [
    "steam_settings", "engine", "binaries", "content", "plugins", "mono", "monobleedingedge", "_commonredist", "redist", "redistributables",
    "__pycache__", "shadercache", "logs", "crashreportclient", "directx", "support",
];
const SHIPPED_EXT: [&str; 22] = [
    "exe", "dll", "pak", "ucas", "utoc", "so", "pdb", "bin.bak", "assets", "resource", "ress", "bundle", "pck", "rpa", "rgss3a", "wad",
    "bik", "bk2", "usm", "awb", "acb", "ogg",
];

const SAVE_WORDS: [&str; 14] = [
    "save", "saves", "savegame", "savegames", "savedata", "saved", "slot", "profile", "profiles", "progress", "userdata", "persistent", "remote",
    "playerprefs",
];
const SAVE_EXT: [&str; 9] = ["sav", "save", "sv", "sl2", "ess", "dat", "rvdata2", "rpgsave", "celeste"];

/// What was seen while one game ran.
#[derive(Debug, Clone, Default)]
pub struct Observation {
    /// Files created or written while it ran.
    pub files: Vec<PathBuf>,
    /// Folders that did not exist before it started.
    pub new_dirs: HashSet<PathBuf>,
    /// Files its own processes held open.
    pub open_files: HashSet<PathBuf>,
    /// Registry keys under `Software` written while it ran (`Software/A/B`).
    pub registry_keys: Vec<String>,
}

/// A possible save location, with how sure we are and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub rule: Rule,
    pub score: i32,
    pub files: usize,
    pub reasons: Vec<&'static str>,
}

impl Candidate {
    pub fn corroborated(&self) -> bool {
        self.score >= CORROBORATED
    }
}

fn lower(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn is_never(seg: &str) -> bool {
    let s = seg.to_lowercase();
    NEVER.iter().any(|n| wildcard(n, &s))
}

fn save_like(name: &str) -> bool {
    let n = name.to_lowercase();
    let stem = n.rsplit_once('.').map(|(s, _)| s).unwrap_or(&n);
    let ext = n.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    SAVE_EXT.contains(&ext) || SAVE_WORDS.iter().any(|w| stem.starts_with(w) || squash(stem) == *w) || stem.contains("save")
}

#[derive(Default)]
struct Group {
    root: Option<Root>,
    /// The folder, or None while it is still the files' shared folder.
    fixed: Option<Vec<String>>,
    dirs: Vec<Vec<String>>,
    files: usize,
    open: bool,
    new_dir: bool,
    save_like: bool,
    named: bool,
    developer: bool,
}

fn common(dirs: &[Vec<String>]) -> Vec<String> {
    let Some(first) = dirs.first() else { return Vec::new() };
    let mut n = first.len();
    for d in dirs {
        n = n.min(d.len());
        for i in 0..n {
            if !d[i].eq_ignore_ascii_case(&first[i]) {
                n = i;
                break;
            }
        }
    }
    first[..n].to_vec()
}

/// The save locations an [`Observation`] points to, best first.
///
/// `ignore` are folders that are never this game's (Kryoto's own data, other
/// games' folders), and `noisy` says whether a location changed while other
/// games ran too - then it is not this game's either.
pub fn candidates(obs: &Observation, roots: &SaveRoots, hints: &GameHints, ignore: &[PathBuf], noisy: &dyn Fn(&Rule) -> bool) -> Vec<Candidate> {
    let ignore: Vec<String> = ignore.iter().map(|p| format!("{}/", lower(p).trim_end_matches('/'))).collect();
    let open: HashSet<String> = obs.open_files.iter().map(|p| lower(p)).collect();
    let new_dirs: HashSet<String> = obs.new_dirs.iter().map(|p| lower(p)).collect();
    let mut groups: HashMap<(Root, String), Group> = HashMap::new();

    for file in &obs.files {
        let low = lower(file);
        if ignore.iter().any(|i| low.starts_with(i.as_str())) {
            continue;
        }
        let Some((root, rel)) = roots.placeholder_of(file) else { continue };
        if root == Root::HKCU {
            continue;
        }
        let segs: Vec<String> = rel.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
        let Some((name, dir)) = segs.split_last() else { continue };
        if crate::scan::blocked(name) || crate::scan::excluded(&rel, &[]) || segs.iter().any(|s| is_never(s)) {
            continue;
        }
        if root == Root::GameDir {
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
            if SHIPPED_EXT.contains(&ext.as_str()) || dir.iter().any(|d| SHIPPED_DIRS.contains(&d.to_lowercase().as_str()) || d.starts_with(".kryoto")) {
                continue;
            }
            if name.starts_with(".kryoto") || name.eq_ignore_ascii_case("steam_appid.txt") {
                continue;
            }
        }
        // Dot-folders in the profile are tools' (.vscode, .cache, .gradle), not games'.
        if root == Root::Home && segs[0].starts_with('.') {
            continue;
        }
        if dir.is_empty() && root != Root::GameDir {
            // A file straight in Documents or AppData is no game's folder.
            continue;
        }
        // The folder named after the game, the shallowest one, holds it all.
        let named_at = dir.iter().position(|d| hints.matches(d));
        let developer = dir.iter().any(|d| hints.is_developer(d));
        let (key, fixed) = match named_at {
            Some(i) => (dir[..=i].join("/").to_lowercase(), Some(dir[..=i].to_vec())),
            None if dir.is_empty() => (name.to_lowercase(), Some(vec![name.clone()])),
            None => {
                // Grouped under its first folder (two for a container like
                // "My Games", or a company folder), narrowed later to what
                // its files share.
                let mut depth = 1;
                if CONTAINERS.contains(&dir[0].to_lowercase().as_str()) || hints.is_developer(&dir[0]) {
                    depth = 2;
                }
                let depth = depth.min(dir.len());
                (dir[..depth].join("/").to_lowercase(), None)
            }
        };
        let g = groups.entry((root, key)).or_default();
        g.root = Some(root);
        if g.fixed.is_none() {
            g.fixed = fixed;
        }
        g.dirs.push(dir.to_vec());
        g.files += 1;
        g.named |= named_at.is_some();
        g.developer |= developer;
        g.open |= open.contains(&low);
        g.save_like |= save_like(name) || dir.iter().any(|d| save_like(d));
        // A folder made during the session, at or above the file.
        let mut p = file.parent();
        while let Some(d) = p {
            if new_dirs.contains(&lower(d)) {
                g.new_dir = true;
                break;
            }
            if roots.get(root).is_some_and(|r| lower(r) == lower(d)) {
                break;
            }
            p = d.parent();
        }
    }

    let mut out: Vec<Candidate> = Vec::new();
    for ((root, _), g) in groups {
        let path = match &g.fixed {
            Some(f) => f.join("/"),
            None => {
                let mut c = common(&g.dirs);
                let min = if g.dirs.first().is_some_and(|d| CONTAINERS.contains(&d[0].to_lowercase().as_str())) { 2 } else { 1 };
                if c.len() < min {
                    c = g.dirs[0][..min.min(g.dirs[0].len())].to_vec();
                }
                c.join("/")
            }
        };
        // "My Games" on its own is every game's folder, not this one's.
        if root != Root::GameDir && !path.contains('/') && CONTAINERS.contains(&path.to_lowercase().as_str()) {
            continue;
        }
        let rule = Rule::files(root, path, Source::Detected);
        if rule.problem().is_some() || noisy(&rule) {
            continue;
        }
        let mut score = 0;
        let mut reasons = Vec::new();
        if g.named {
            score += 40;
            reasons.push("named after the game");
        }
        if g.open {
            score += 30;
            reasons.push("the game had its files open");
        }
        if g.new_dir {
            score += 20;
            reasons.push("made while the game ran");
        }
        if g.save_like {
            score += 15;
            reasons.push("save files");
        }
        if g.developer {
            score += 10;
            reasons.push("in the developer's folder");
        }
        // A folder called saves (or the like) that the game made in its own folder.
        if root == Root::GameDir && g.new_dir && rule.path.rsplit('/').next().is_some_and(|last| SAVE_WORDS.contains(&squash(last).as_str())) {
            score += 15;
            reasons.push("a save folder in the game's folder");
        }
        if g.files > 500 {
            score -= 15;
        }
        if root == Root::GameDir && !g.save_like && !g.open {
            score -= 10;
        }
        if score < SUGGEST {
            continue;
        }
        out.push(Candidate { rule, score, files: g.files, reasons });
    }

    for key in &obs.registry_keys {
        let segs: Vec<&str> = key.split('/').filter(|s| !s.is_empty()).collect();
        if segs.len() < 2 || !segs[0].eq_ignore_ascii_case("Software") || is_never(segs[1]) || REG_NEVER.iter().any(|n| segs[1].eq_ignore_ascii_case(n)) {
            continue;
        }
        let named_at = segs.iter().skip(1).position(|s| hints.matches(s)).map(|i| i + 1);
        let developer = hints.is_developer(segs[1]);
        let depth = match named_at {
            Some(i) => i + 1,
            None if developer && segs.len() >= 3 => 3,
            None => continue,
        };
        let rule = Rule::registry(segs[..depth.min(segs.len())].join("/"), Source::Detected);
        if rule.problem().is_some() || noisy(&rule) {
            continue;
        }
        let score = if named_at.is_some() { 40 } else { 0 } + if developer { 10 } else { 0 } + 15;
        let reasons = vec![if named_at.is_some() { "named after the game" } else { "in the developer's key" }];
        out.push(Candidate { rule, score, files: 0, reasons });
    }

    // One location inside another is the same save: keep the wider one.
    out.sort_by_key(|c| c.rule.path.len());
    let mut merged: Vec<Candidate> = Vec::new();
    for c in out {
        if let Some(outer) = merged.iter_mut().find(|m| m.rule.contains(&c.rule)) {
            outer.files += c.files;
            outer.score = outer.score.max(c.score);
            for r in c.reasons {
                if !outer.reasons.contains(&r) {
                    outer.reasons.push(r);
                }
            }
            continue;
        }
        merged.push(c);
    }
    merged.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.rule.path.cmp(&b.rule.path)));
    merged
}

/// Registry vendors whose keys are never a game's save.
const REG_NEVER: [&str; 22] = [
    "Classes", "Policies", "Wow6432Node", "Valve", "Kryoto", "RegisteredApplications", "Clients", "Khronos", "OpenSSH", "Python", "JavaSoft",
    "Oracle", "VMware, Inc.", "Wine", "Realtek", "Unity Technologies", "Epic Games", "AppDataLow", "WinRAR", "7-Zip", "Sysinternals", "Mozilla",
];

/// How a game's saves are put on a phone or another PC's prefix: placeholder
/// paths only, never this PC's folders or the user's name (what may be shared).
pub fn shareable(rule: &Rule) -> bool {
    rule.problem().is_none() && !rule.path.to_lowercase().contains("users/")
}

/// The folders whose writes a tracer should ignore by default: the parts of
/// the profile that are busy with other programs all the time.
pub fn walk_skip(name: &str) -> bool {
    is_never(name) || crate::scan::DEFAULT_EXCLUDES.iter().any(|p| wildcard(p, name))
}

/// Paths under the observed folders that the tracer walks after a session:
/// relative path for a [`relative`] check.
pub fn rel_of(roots: &SaveRoots, root: Root, path: &Path) -> Option<String> {
    relative(roots.get(root)?, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> SaveRoots {
        SaveRoots {
            game_dir: Some("D:/Games/Hollow Knight".into()),
            app_data: Some("C:/U/AppData/Roaming".into()),
            local_app_data: Some("C:/U/AppData/Local".into()),
            local_low: Some("C:/U/AppData/LocalLow".into()),
            documents: Some("C:/U/Documents".into()),
            home: Some("C:/U".into()),
            ..Default::default()
        }
    }

    fn hints() -> GameHints {
        GameHints { title: "Hollow Knight".into(), exe_stem: "hollow_knight".into(), developer: Some("Team Cherry".into()), appid: Some("367520".into()), ..Default::default() }
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn noise_is_dropped_and_the_game_folder_is_found() {
        let obs = Observation {
            files: vec![
                p("C:/U/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat"),
                p("C:/U/AppData/LocalLow/Team Cherry/Hollow Knight/shared.dat"),
                p("C:/U/AppData/LocalLow/Team Cherry/Hollow Knight/Player.log"),
                p("C:/U/AppData/Local/Google/Chrome/User Data/Default/History"),
                p("C:/U/AppData/Local/Temp/abc.tmp"),
                p("C:/U/AppData/Local/NVIDIA/DXCache/x.bin"),
                p("C:/U/AppData/Roaming/discord/Local Storage/leveldb/1.log"),
                p("C:/U/AppData/Local/to.kryo.desktop/EBWebView/x"),
                p("C:/U/Documents/notes.txt"),
            ],
            open_files: [p("C:/U/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat")].into_iter().collect(),
            ..Default::default()
        };
        let c = candidates(&obs, &roots(), &hints(), &[], &|_| false);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!((c[0].rule.root, c[0].rule.path.as_str()), (Root::LocalLow, "Team Cherry/Hollow Knight"));
        assert!(c[0].corroborated());
        assert_eq!(c[0].rule.source, Source::Detected);
    }

    #[test]
    fn a_folder_not_named_after_the_game_narrows_to_its_files() {
        let obs = Observation {
            files: vec![p("C:/U/Documents/My Games/SomeStudio/X9/saves/slot1.sav"), p("C:/U/Documents/My Games/SomeStudio/X9/saves/slot2.sav")],
            new_dirs: [p("C:/U/Documents/My Games/SomeStudio")].into_iter().collect(),
            ..Default::default()
        };
        let c = candidates(&obs, &roots(), &hints(), &[], &|_| false);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].rule.path, "My Games/SomeStudio/X9/saves");
        assert_eq!(c[0].score, 35, "new folder and save files, but not named: a suggestion");
        assert!(!c[0].corroborated());
    }

    #[test]
    fn the_game_folder_keeps_saves_but_not_what_shipped() {
        let obs = Observation {
            files: vec![
                p("D:/Games/Hollow Knight/saves/profile.dat"),
                p("D:/Games/Hollow Knight/hollow_knight.exe"),
                p("D:/Games/Hollow Knight/steam_settings/configs.user.ini"),
                p("D:/Games/Hollow Knight/Engine/Saved/x.bin"),
                p("D:/Games/Hollow Knight/.kryoto-game.json"),
            ],
            new_dirs: [p("D:/Games/Hollow Knight/saves")].into_iter().collect(),
            ..Default::default()
        };
        let c = candidates(&obs, &roots(), &hints(), &[], &|_| false);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!((c[0].rule.root, c[0].rule.path.as_str()), (Root::GameDir, "saves"));
        assert!(c[0].corroborated(), "a new saves folder in the game's own folder is its saves");
    }

    #[test]
    fn folders_other_games_also_change_are_not_this_games() {
        let obs = Observation { files: vec![p("C:/U/AppData/LocalLow/Team Cherry/Hollow Knight/user1.dat")], ..Default::default() };
        let c = candidates(&obs, &roots(), &hints(), &[], &|r| r.path == "Team Cherry/Hollow Knight");
        assert!(c.is_empty());
        let ignored = candidates(&obs, &roots(), &hints(), &[p("C:/U/AppData/LocalLow/Team Cherry")], &|_| false);
        assert!(ignored.is_empty());
    }

    #[test]
    fn dot_folders_in_the_profile_are_not_games() {
        let obs = Observation { files: vec![p("C:/U/.vscode/extensions/x/state.json")], new_dirs: [p("C:/U/.vscode/extensions/x")].into_iter().collect(), ..Default::default() };
        assert!(candidates(&obs, &roots(), &hints(), &[], &|_| false).is_empty());
    }

    #[test]
    fn a_root_is_never_a_candidate() {
        let obs = Observation { files: vec![p("C:/U/Documents/My Games/a.sav")], ..Default::default() };
        let c = candidates(&obs, &roots(), &hints(), &[], &|_| false);
        assert!(c.iter().all(|c| !c.rule.path.is_empty() && c.rule.path != "My Games"), "{c:?}");
    }

    #[test]
    fn registry_keys_named_after_the_game_count_and_others_do_not() {
        let obs = Observation {
            registry_keys: vec![
                "Software/Team Cherry/Hollow Knight".into(),
                "Software/Microsoft/Windows/CurrentVersion/Explorer".into(),
                "Software/SomeApp/Settings".into(),
            ],
            ..Default::default()
        };
        let c = candidates(&obs, &roots(), &hints(), &[], &|_| false);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].rule, Rule::registry("Software/Team Cherry/Hollow Knight", Source::Detected));
        assert!(c[0].corroborated());
    }

    #[test]
    fn save_words_are_recognised() {
        assert!(save_like("slot1.sav"));
        assert!(save_like("SaveGames"));
        assert!(save_like("profile_0.json"));
        assert!(!save_like("settings.ini"));
    }
}
