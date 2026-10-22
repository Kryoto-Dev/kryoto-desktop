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

/// Folder names that are never a game's saves, anywhere under a root: other
/// programs, and the places developers' tools write (a game running beside a
/// compile once "found" a git checkout and a compiler cache).
const NEVER: &[&str] = &[
    "temp", "tmp", "inetcache", "inetcookies", "crashdumps", "crashpad", "crashreporter", "crash reports", "d3dscache", "dxcache", "glcache",
    "nvidia", "nvidia corporation", "amd", "ati", "intel", "microsoft", "packages", "google", "mozilla", "bravesoftware", "opera software",
    "vivaldi", "discord", "discordptb", "discordcanary", "spotify", "slack", "teams", "zoom", "steam", "epic games", "epicgameslauncher",
    "gog.com", "onedrive", "dropbox", "programs", "npm-cache", "npm", "pip", "node_modules", ".cache", "cef", "ebwebview", "webview2",
    "squirreltemp", "connecteddevicesplatform", "comms", "history", "recent", "unrealengine", "nuget", "jetbrains", "code", "obs-studio",
    "kryoto", "kryoto desktop", "to.kryo.desktop*", "github", "gitlab", ".git", ".vs", "ccache", "sccache", "cargo", ".cargo", "rustup",
    "gradle", ".gradle", "android", "pnpm", "yarn", "go", "python*", "claude", "codex", "cursor", "cline", "copilot", "openai", "chatgpt",
    "antigravity", "windsurf", "zed", "notepad++", "sublime text*", ".mono",
    // Programs that run beside every game: recorders, overlays, peripherals, chat.
    "medal", "overwolf", "outplayed", "obs", "streamlabs*", "xsplit", "bandicam", "fraps", "shadowplay", "geforce experience", "rivatuner*",
    "msi afterburner", "razer", "corsair", "icue", "logitech*", "lghub", "steelseries*", "wallpaper engine", "whatsapp", "telegram desktop",
    "signal", "guilded", "teamspeak*", "mumble", "parsec", "anydesk", "teamviewer", "nahimic", "xbox game bar", "gamebar",
];

/// The first folder under a root that is a container for many games' saves,
/// not one game's.
const CONTAINERS: [&str; 9] = ["my games", "games", "saved games", "savegames", "saves", "save", "savedata", "save data", "game saves"];

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
    /// Precise mode: the files the game's own processes created, wrote,
    /// renamed or deleted, as Windows itself reported them. When this is
    /// there it is the whole truth about files, and `files` is not used.
    pub traced: Option<Vec<PathBuf>>,
    /// Folder names (lowercase) the game's own files mention
    /// ([`crate::mentions`]), from [`names_to_check`].
    pub mentioned: HashSet<String>,
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
    mentioned: bool,
    traced: bool,
}

/// Folder names too common to tie a folder to one game when a game's files
/// mention them: words every engine's and runtime's code contains (measured
/// on real games: `client`, `cache`, `signal`, `origin` are in nearly all of
/// them). Kryoto Desktop also checks a name against the player's other games
/// (saves/sync.rs `specific`); this list covers a PC with few.
const GENERIC: &[&str] = &[
    "data", "config", "configs", "settings", "user", "users", "local", "locallow", "roaming", "appdata", "default", "common", "shared",
    "files", "file", "game", "games", "documents", "my documents", "public", "programdata", "windows", "system", "backup", "backups",
    "content", "assets", "resources", "player", "players", "steam", "remote", "storage", "cloud", "global", "options", "preferences",
    "unity", "unreal", "godot", "client", "server", "cache", "temp", "code", "signal", "speech", "origin", "android", "google",
    "microsoft", "adobe", "autodesk", "packages", "package cache", "programs", "installed", "installing", "install", "update", "updates",
    "userprofile", "persistentstorage", "isolatedstorage", "performancedata", "crashreportclient", "crashdumps", "shadercache", "sentry",
    "electron", "proton", "fontconfig", "nuget", "vivox", "epic", "epic games", "unreal engine", "unrealengine", "openvr", "steamvr",
    "oculus", "nvidia", "nvidia corporation", "discord", "mozilla", "beyond", "until then", "new folder", "logs", "plugins", "mods",
    "downloads", "pictures", "videos", "music", "desktop", "profile", "session", "sessions", "history", "network", "audio", "video",
    "input", "output", "media", "images", "fonts", "shaders", "editor", "engine", "launcher", "service", "services", "tools", "library",
];

/// A file the tracer saw that may be a game's save, and its place:
/// everything other programs and the system own is dropped here.
fn plausible(file: &Path, roots: &SaveRoots, ignore: &[String]) -> Option<(Root, String)> {
    let low = lower(file);
    if ignore.iter().any(|i| low.starts_with(i.as_str())) {
        return None;
    }
    let (root, rel) = roots.placeholder_of(file)?;
    if root == Root::HKCU {
        return None;
    }
    let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let (name, _) = segs.split_last()?;
    if crate::scan::blocked(name) || crate::scan::excluded(&rel, &[]) || segs.iter().any(|s| is_never(s)) {
        return None;
    }
    if root == Root::GameDir && crate::scan::shipped(&rel) {
        return None;
    }
    // Dot-folders in the profile are tools' (.vscode, .cache, .gradle), not games'.
    if root == Root::Home && segs[0].starts_with('.') {
        return None;
    }
    if segs.len() < 2 && root != Root::GameDir {
        // A file straight in Documents or AppData is no game's folder.
        return None;
    }
    Some((root, rel))
}

/// The folder names to look for in the game's own files
/// ([`crate::mentions::find`]): every distinct folder name outside the game's
/// folder that a written file sits in, but not the ones that say nothing
/// (containers, save words, "Data"), nor those already named after the game.
pub fn names_to_check(obs: &Observation, roots: &SaveRoots, hints: &GameHints, ignore: &[PathBuf]) -> Vec<String> {
    let ignore: Vec<String> = ignore.iter().map(|p| format!("{}/", lower(p).trim_end_matches('/'))).collect();
    let mut out: Vec<String> = Vec::new();
    for file in &obs.files {
        let Some((root, rel)) = plausible(file, roots, &ignore) else { continue };
        if root == Root::GameDir {
            continue;
        }
        let segs: Vec<&str> = rel.split('/').collect();
        for d in &segs[..segs.len() - 1] {
            let low = d.to_lowercase();
            let says_nothing = low.chars().count() < 4
                || low.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '_' || c == '.')
                || GENERIC.contains(&low.as_str())
                || CONTAINERS.contains(&low.as_str())
                || SAVE_WORDS.iter().any(|w| squash(&low) == *w)
                || hints.matches(d)
                || hints.is_developer(d);
            if !says_nothing && !out.contains(&low) {
                out.push(low);
            }
        }
        if out.len() >= 400 {
            break;
        }
    }
    out
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
    // Precise mode saw which files the game itself wrote: only those.
    let traced = obs.traced.is_some();
    let files = obs.traced.as_ref().unwrap_or(&obs.files);

    for file in files {
        let low = lower(file);
        let Some((root, rel)) = plausible(file, roots, &ignore) else { continue };
        let segs: Vec<String> = rel.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
        let Some((name, dir)) = segs.split_last() else { continue };
        // The folder named after the game, the shallowest one, holds it all.
        // Not inside the game's own folder, where every folder is the game's
        // and one named like its exe is the whole project (Unreal's
        // `DiggingGame`, with its Binaries and Content): there the save is
        // only the folder its written files share.
        let in_game = root == Root::GameDir;
        let named_at = if in_game { None } else { dir.iter().position(|d| hints.matches(d)) };
        let developer = !in_game && dir.iter().any(|d| hints.is_developer(d));
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
        g.traced |= traced;
        g.mentioned |= !in_game && dir.iter().any(|d| obs.mentioned.contains(&d.to_lowercase()));
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
        // Only what ties a folder to THIS game makes it a save location:
        // Windows saw the game's processes write it (precise mode), they had
        // files open in it, or (outside the game's folder) it is named after
        // the game or its developer or named in the game's own files, or
        // (inside it) save files were written there. That something changed
        // while the game ran is never enough on its own: a compile, a git
        // pull or a browser writes then too.
        let evidence = g.traced || g.open || if root == Root::GameDir { g.save_like } else { g.named || g.developer || g.mentioned };
        if !evidence {
            continue;
        }
        let mut score = 0;
        let mut reasons = Vec::new();
        if g.traced {
            score += 60;
            reasons.push("the game wrote here");
        }
        // Short of syncing on its own: with save files and a folder made
        // this session, or seen again next session, it is.
        if g.mentioned {
            score += 30;
            reasons.push("named in the game's files");
        }
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
        if root == Root::GameDir && !g.save_like && !g.open && !g.traced {
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
    fn a_folder_not_named_after_the_game_needs_the_game_to_have_had_it_open() {
        let mut obs = Observation {
            files: vec![p("C:/U/Documents/My Games/SomeStudio/X9/saves/slot1.sav"), p("C:/U/Documents/My Games/SomeStudio/X9/saves/slot2.sav")],
            new_dirs: [p("C:/U/Documents/My Games/SomeStudio")].into_iter().collect(),
            ..Default::default()
        };
        let c = candidates(&obs, &roots(), &hints(), &[], &|_| false);
        assert!(c.is_empty(), "written while it ran, but nothing ties it to this game: {c:?}");
        obs.open_files.insert(p("C:/U/Documents/My Games/SomeStudio/X9/saves/slot1.sav"));
        let c = candidates(&obs, &roots(), &hints(), &[], &|_| false);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].rule.path, "My Games/SomeStudio/X9/saves", "narrowed to what its files share");
    }

    #[test]
    fn precise_mode_takes_only_what_the_game_wrote() {
        // A game called "Untitled" saving under a codename, its settings in
        // its own folder, while a compile ran beside it.
        let roots = roots();
        let hints = GameHints { title: "Untitled".into(), exe_stem: "Launcher".into(), ..Default::default() };
        let obs = Observation {
            files: vec![p("C:/U/AppData/Local/ccache/1/a.o"), p("C:/U/Documents/GitHub/x/main.o"), p("C:/U/AppData/Roaming/ProjectZebra/slot0")],
            traced: Some(vec![
                p("C:/U/AppData/Roaming/ProjectZebra/profiles/slot0"),
                p("C:/U/AppData/Roaming/ProjectZebra/profiles/slot0.bak"),
                p("C:/U/AppData/Local/Temp/zebra.tmp"),
                p("C:/U/AppData/LocalLow/NVIDIA/PerDriverVersion/DXCache/x.bin"),
                p("D:/Games/Hollow Knight/settings.cfg"),
                p("D:/Games/Hollow Knight/Launcher.exe"),
            ]),
            ..Default::default()
        };
        let c = candidates(&obs, &roots, &hints, &[], &|_| false);
        let got: Vec<(Root, &str)> = c.iter().map(|c| (c.rule.root, c.rule.path.as_str())).collect();
        assert_eq!(got.len(), 2, "{c:?}");
        assert!(got.contains(&(Root::AppData, "ProjectZebra/profiles")));
        assert!(got.contains(&(Root::GameDir, "settings.cfg")), "the game wrote it in its own folder: its data, whatever its name");
        assert!(c.iter().all(|c| c.corroborated()));
    }

    #[test]
    fn a_folder_named_in_the_games_own_files_is_its_save() {
        let obs = Observation {
            files: vec![p("C:/U/Documents/My Games/SomeStudio/X9/saves/slot1.sav"), p("C:/U/AppData/Local/ccache/1/abc.o"), p("C:/U/AppData/Local/SomeTool/state/profile.json")],
            ..Default::default()
        };
        let names = names_to_check(&obs, &roots(), &hints(), &[]);
        assert!(names.contains(&"somestudio".to_string()) && !names.contains(&"x9".to_string()), "{names:?}");
        assert!(names.contains(&"sometool".to_string()));
        assert!(!names.iter().any(|n| n == "my games" || n == "saves" || n == "ccache"), "{names:?}");
        // The game's exe names SomeStudio; nothing names SomeTool.
        let mut obs = Observation { mentioned: ["somestudio".to_string()].into_iter().collect(), ..obs };
        let c = candidates(&obs, &roots(), &hints(), &[], &|_| false);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].rule.path, "My Games/SomeStudio/X9/saves");
        assert!(!c[0].corroborated(), "a mention and save files: offered, not yet synced");
        obs.new_dirs.insert(p("C:/U/Documents/My Games/SomeStudio"));
        assert!(candidates(&obs, &roots(), &hints(), &[], &|_| false)[0].corroborated(), "and made this session: the game's");
        // A word in every game's code is no evidence.
        let obs = Observation { files: vec![p("C:/U/AppData/Local/Signal/db/x.dat")], ..Default::default() };
        assert!(names_to_check(&obs, &roots(), &hints(), &[]).is_empty());
    }

    #[test]
    fn what_other_programs_wrote_while_it_ran_is_never_a_save() {
        // A compile and a git pull beside the game: new folders and many
        // files, written in the same minutes, and not one of them the game's.
        let obs = Observation {
            files: vec![
                p("C:/U/Documents/GitHub/ctw-native/build/main.o"),
                p("C:/U/Documents/GitHub/ctw-native/.git/index"),
                p("C:/U/Documents/notes/ctw-native/save.txt"),
                p("C:/U/AppData/Local/ccache/1/2/abc.o"),
                p("C:/U/AppData/Local/SomeTool/state/profile.json"),
                p("C:/U/AppData/Roaming/Code/User/globalStorage/state.vscdb"),
            ],
            new_dirs: [p("C:/U/Documents/GitHub/ctw-native/build"), p("C:/U/AppData/Local/ccache/1"), p("C:/U/AppData/Local/SomeTool")].into_iter().collect(),
            ..Default::default()
        };
        let c = candidates(&obs, &roots(), &hints(), &[], &|_| false);
        assert!(c.is_empty(), "{c:?}");
    }

    #[test]
    fn a_project_folder_named_after_the_exe_is_not_taken_whole() {
        // Unreal: the game's folder holds `DiggingGame/` with its Binaries and
        // Content beside `Saved/`. Its name matches the exe, but the save is
        // only the folder the written files share.
        let roots = SaveRoots { game_dir: Some("D:/Games/Digging".into()), ..roots() };
        let hints = GameHints { title: "A Game About Digging A Hole".into(), exe_stem: "DiggingGame".into(), ..Default::default() };
        let obs = Observation {
            files: vec![
                p("D:/Games/Digging/DiggingGame/Saved/SaveGames/MainSave.sav"),
                p("D:/Games/Digging/DiggingGame/Saved/Logs/DiggingGame.log"),
                p("D:/Games/Digging/DiggingGame/Binaries/Win64/DiggingGame-Win64-Shipping.exe"),
                p("D:/Games/Digging/DiggingGame/Content/Paks/DiggingGame-Windows.pak"),
            ],
            ..Default::default()
        };
        let c = candidates(&obs, &roots, &hints, &[], &|_| false);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!((c[0].rule.root, c[0].rule.path.as_str()), (Root::GameDir, "DiggingGame/Saved/SaveGames"));
        // Settings written in the game's folder, with no save among them, are not saves.
        let obs = Observation { files: vec![p("D:/Games/Digging/DiggingGame/Config/Input.ini"), p("D:/Games/Digging/settings.cfg")], ..Default::default() };
        assert!(candidates(&obs, &roots, &hints, &[], &|_| false).is_empty());
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
