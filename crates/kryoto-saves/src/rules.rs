//! Save locations as rules: a placeholder root and a path under it.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// The folders a save location is relative to. Each PC resolves them for
/// itself ([`SaveRoots`]), so a rule written on one PC works on another, and
/// in a Wine or Proton prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Root {
    /// The game's own install folder.
    GameDir,
    /// `%APPDATA%` (AppData\Roaming).
    AppData,
    /// `%LOCALAPPDATA%`.
    LocalAppData,
    /// AppData\LocalLow (Unity's home).
    LocalLow,
    /// The Documents folder, wherever Windows has it (OneDrive included).
    Documents,
    /// `Saved Games`.
    SavedGames,
    /// `%PUBLIC%\Documents` (RUNE, CODEX, OnlineFix).
    PublicDocuments,
    /// `%PROGRAMDATA%`.
    ProgramData,
    /// `%LOCALAPPDATA%\VirtualStore`, where Windows redirects old games that
    /// write into Program Files.
    VirtualStore,
    /// The user's profile folder.
    Home,
    /// `HKEY_CURRENT_USER` (registry rules only).
    HKCU,
}

impl Root {
    pub const ALL: [Root; 11] = [
        Root::GameDir,
        Root::AppData,
        Root::LocalAppData,
        Root::LocalLow,
        Root::Documents,
        Root::SavedGames,
        Root::PublicDocuments,
        Root::ProgramData,
        Root::VirtualStore,
        Root::Home,
        Root::HKCU,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Root::GameDir => "GameDir",
            Root::AppData => "AppData",
            Root::LocalAppData => "LocalAppData",
            Root::LocalLow => "LocalLow",
            Root::Documents => "Documents",
            Root::SavedGames => "SavedGames",
            Root::PublicDocuments => "PublicDocuments",
            Root::ProgramData => "ProgramData",
            Root::VirtualStore => "VirtualStore",
            Root::Home => "Home",
            Root::HKCU => "HKCU",
        }
    }

    pub fn parse(s: &str) -> Option<Root> {
        Root::ALL.into_iter().find(|r| r.name() == s)
    }

    /// How a player reads it.
    pub fn label(self) -> &'static str {
        match self {
            Root::GameDir => "Game folder",
            Root::AppData => "AppData\\Roaming",
            Root::LocalAppData => "AppData\\Local",
            Root::LocalLow => "AppData\\LocalLow",
            Root::Documents => "Documents",
            Root::SavedGames => "Saved Games",
            Root::PublicDocuments => "Public Documents",
            Root::ProgramData => "ProgramData",
            Root::VirtualStore => "VirtualStore",
            Root::Home => "Your user folder",
            Root::HKCU => "Registry",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Files,
    Registry,
}

/// Where a rule came from. Every source but `Detected` is trusted to sync on
/// its own; a detected one does once it is corroborated or the player keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Added by the player.
    Player,
    /// The Steam emulator's own save folder.
    Emulator,
    /// The game engine's convention (Unity, Unreal, Ren'Py...).
    Engine,
    /// The Ludusavi database (PCGamingWiki).
    Known,
    /// Seen by enough players.
    Community,
    /// Set by Kryoto staff.
    Staff,
    /// What Kryoto Desktop saw this game write.
    Detected,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Player => "Added by you",
            Source::Emulator => "Emulator",
            Source::Engine => "Engine",
            Source::Known => "Known",
            Source::Community => "Community",
            Source::Staff => "Kryoto",
            Source::Detected => "Detected",
        }
    }
}

/// One save location: a folder (or file) under a root, or a registry key.
///
/// `path` uses `/`, is relative, and may hold `*` inside a segment (one
/// folder level) - the Steam user id folder some games make, for example.
/// `excludes` are names or relative patterns inside it that are not saves.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    #[serde(default)]
    pub kind: Kind,
    pub root: Root,
    pub path: String,
    #[serde(default)]
    pub excludes: Vec<String>,
    pub source: Source,
}

impl Rule {
    pub fn files(root: Root, path: impl Into<String>, source: Source) -> Rule {
        Rule { kind: Kind::Files, root, path: normalize(&path.into()), excludes: Vec::new(), source }
    }

    pub fn registry(path: impl Into<String>, source: Source) -> Rule {
        Rule { kind: Kind::Registry, root: Root::HKCU, path: normalize(&path.into()), excludes: Vec::new(), source }
    }

    pub fn excluding(mut self, names: &[&str]) -> Rule {
        self.excludes.extend(names.iter().map(|s| s.to_string()));
        self
    }

    /// The location without where it came from: two sources naming the same
    /// folder are one location.
    pub fn key(&self) -> String {
        format!("{:?}:{}:{}", self.kind, self.root.name(), self.path.to_ascii_lowercase())
    }

    /// Whether this rule's location contains `other`'s (the same folder or a
    /// folder above it). Wildcards are compared as written.
    pub fn contains(&self, other: &Rule) -> bool {
        if self.kind != other.kind || self.root != other.root {
            return false;
        }
        let a = self.path.to_ascii_lowercase();
        let b = other.path.to_ascii_lowercase();
        b == a || b.starts_with(&format!("{a}/"))
    }

    /// Why this rule cannot be used, or None. A rule must never be a whole
    /// root (that would be "back up Documents"), never climb out of it, and a
    /// registry rule must stay under `HKCU\Software`.
    pub fn problem(&self) -> Option<&'static str> {
        if let Some(p) = path_problem(&self.path) {
            return Some(p);
        }
        match self.kind {
            Kind::Registry => {
                if self.root != Root::HKCU {
                    return Some("A registry location must be under HKEY_CURRENT_USER.");
                }
                let mut parts = self.path.split('/');
                if !parts.next().is_some_and(|p| p.eq_ignore_ascii_case("Software")) || parts.next().is_none() {
                    return Some("A registry location must be a key under HKEY_CURRENT_USER\\Software.");
                }
                if self.path.contains('*') {
                    return Some("A registry location cannot use wildcards.");
                }
            }
            Kind::Files => {
                if self.root == Root::HKCU {
                    return Some("Files cannot be under the registry.");
                }
            }
        }
        None
    }

    /// Where it is on this PC, wildcards still in (see [`expand`]).
    pub fn base(&self, roots: &SaveRoots) -> Option<PathBuf> {
        let root = roots.get(self.root)?;
        Some(join(root, &self.path))
    }
}

/// `\` to `/`, no leading, trailing or doubled separators, no `.` segments.
pub fn normalize(path: &str) -> String {
    path.replace('\\', "/").split('/').filter(|s| !s.is_empty() && *s != ".").collect::<Vec<_>>().join("/")
}

/// Why a relative save path is not acceptable, or None.
pub fn path_problem(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("A save location cannot be a whole folder like Documents or AppData.");
    }
    if path.starts_with('/') || path.starts_with('\\') || path.contains(':') {
        return Some("A save location must be relative to its folder.");
    }
    if path.len() > 400 {
        return Some("That path is too long.");
    }
    // `<steamId>` is the one token a path may hold (see [`expand`]).
    for seg in path.replace("<steamId>", "id").split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Some("A save location cannot leave its folder.");
        }
        if seg.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?')) {
            return Some("That path has characters a folder cannot have.");
        }
        if seg.chars().all(|c| c == '*') && seg.len() > 1 {
            return Some("Use * for one folder level.");
        }
    }
    None
}

/// `root` joined with a `/` path, segment by segment, refusing anything that
/// would leave `root` (see [`path_problem`]; this is the belt to its braces).
pub fn join(root: &Path, rel: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains(':') || seg.contains('\\') {
            continue;
        }
        out.push(seg);
    }
    out
}

/// `path` inside `root`, as a `/` path, or None when it is not inside.
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = strip_prefix_ci(path, root)?;
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// `Path::strip_prefix`, but case-insensitive on Windows, where `C:\Users`
/// and `c:\users` are one folder.
fn strip_prefix_ci<'a>(path: &'a Path, root: &Path) -> Option<&'a Path> {
    if let Ok(rel) = path.strip_prefix(root) {
        return Some(rel);
    }
    if !cfg!(windows) {
        return None;
    }
    let mut pc = path.components();
    for rc in root.components() {
        let p = pc.next()?;
        if !p.as_os_str().to_string_lossy().eq_ignore_ascii_case(&rc.as_os_str().to_string_lossy()) {
            return None;
        }
    }
    Some(pc.as_path())
}

/// Whether `name` matches `pattern`, where `*` is any run of characters and
/// `?` any one character. Case-insensitive: saves live on Windows.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Every existing path a rule's (possibly wildcarded) location names.
pub fn expand(rule: &Rule, roots: &SaveRoots) -> Vec<PathBuf> {
    let Some(root) = roots.get(rule.root) else { return Vec::new() };
    let mut current = vec![root.to_path_buf()];
    let path = rule.path.replace("<steamId>", roots.steam_id.as_deref().unwrap_or("*"));
    for seg in path.split('/') {
        let mut next = Vec::new();
        for dir in &current {
            if seg.contains('*') || seg.contains('?') {
                let Ok(read) = std::fs::read_dir(dir) else { continue };
                for e in read.flatten().take(500) {
                    if wildcard(seg, &e.file_name().to_string_lossy()) {
                        next.push(e.path());
                    }
                }
            } else {
                let p = dir.join(seg);
                if p.exists() {
                    next.push(p);
                }
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    current.sort();
    current
}

/// The folders behind each [`Root`] on this PC (or in a Wine prefix).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaveRoots {
    pub game_dir: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub local_low: Option<PathBuf>,
    pub documents: Option<PathBuf>,
    pub saved_games: Option<PathBuf>,
    pub public_documents: Option<PathBuf>,
    pub program_data: Option<PathBuf>,
    pub virtual_store: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// The Steam id the emulator uses here, for `<steamId>` in a path.
    pub steam_id: Option<String>,
}

impl SaveRoots {
    pub fn get(&self, root: Root) -> Option<&Path> {
        match root {
            Root::GameDir => self.game_dir.as_deref(),
            Root::AppData => self.app_data.as_deref(),
            Root::LocalAppData => self.local_app_data.as_deref(),
            Root::LocalLow => self.local_low.as_deref(),
            Root::Documents => self.documents.as_deref(),
            Root::SavedGames => self.saved_games.as_deref(),
            Root::PublicDocuments => self.public_documents.as_deref(),
            Root::ProgramData => self.program_data.as_deref(),
            Root::VirtualStore => self.virtual_store.as_deref(),
            Root::Home => self.home.as_deref(),
            Root::HKCU => None,
        }
    }

    /// The folders inside a Wine or Proton prefix (`drive_c/users/<user>`).
    pub fn for_prefix(prefix: &Path, game_dir: Option<PathBuf>) -> SaveRoots {
        let users = prefix.join("drive_c").join("users");
        let user = std::fs::read_dir(&users).ok().and_then(|d| {
            d.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.join("AppData").join("Roaming").is_dir() && !p.ends_with("Public"))
        });
        let under = |rel: &str| user.as_ref().map(|u| join(u, rel));
        SaveRoots {
            game_dir,
            app_data: under("AppData/Roaming"),
            local_app_data: under("AppData/Local"),
            local_low: under("AppData/LocalLow"),
            documents: under("Documents"),
            saved_games: under("Saved Games"),
            public_documents: Some(users.join("Public").join("Documents")),
            program_data: Some(prefix.join("drive_c").join("ProgramData")),
            virtual_store: under("AppData/Local/VirtualStore"),
            home: user.clone(),
            steam_id: None,
        }
    }

    /// The root and relative path for an absolute path: the most specific
    /// root that holds it, so `LocalAppData\VirtualStore\x` is VirtualStore
    /// and a game installed under Documents is GameDir.
    pub fn placeholder_of(&self, path: &Path) -> Option<(Root, String)> {
        let mut best: Option<(Root, String, usize)> = None;
        for root in Root::ALL {
            let Some(base) = self.get(root) else { continue };
            let Some(rel) = relative(base, path) else { continue };
            let depth = base.components().count();
            // The game's folder wins any tie: it is the most specific there is.
            let better = match &best {
                None => true,
                Some((r, _, d)) => depth > *d || (depth == *d && root == Root::GameDir && *r != Root::GameDir),
            };
            if better {
                best = Some((root, rel, depth));
            }
        }
        best.map(|(r, p, _)| (r, p))
    }
}

/// What the game is called, for matching folder names to it.
#[derive(Debug, Clone, Default)]
pub struct GameHints {
    pub title: String,
    /// The exe's name without `.exe`.
    pub exe_stem: String,
    pub slug: Option<String>,
    pub appid: Option<String>,
    pub developer: Option<String>,
}

/// Lowercase letters and digits only: "Hollow Knight: Silksong" is
/// "hollowknightsilksong".
pub fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

impl GameHints {
    fn names(&self) -> Vec<String> {
        let mut out = vec![squash(&self.title), squash(&self.exe_stem)];
        if let Some(s) = &self.slug {
            out.push(squash(s));
        }
        // "-Win64-Shipping" and the like are not the game's name.
        for n in out.clone() {
            for tail in ["win64shipping", "win32shipping", "shipping", "x64", "64", "launcher", "game"] {
                if let Some(stem) = n.strip_suffix(tail).filter(|s| s.len() >= 4) {
                    out.push(stem.to_string());
                }
            }
        }
        out.retain(|n| n.len() >= 3);
        out.sort();
        out.dedup();
        out
    }

    /// Whether a folder or key name is this game's: its title, exe, slug or
    /// AppID, allowing for punctuation and spacing.
    pub fn matches(&self, name: &str) -> bool {
        let seg = squash(name);
        if seg.is_empty() {
            return false;
        }
        if self.appid.as_deref().is_some_and(|a| a == name.trim()) {
            return true;
        }
        if seg.len() < 3 {
            return false;
        }
        self.names().iter().any(|n| *n == seg || (seg.len() >= 5 && n.contains(&seg)) || (n.len() >= 5 && seg.contains(n.as_str())))
    }

    /// Whether a name is the game's developer (a company folder above it).
    pub fn is_developer(&self, name: &str) -> bool {
        let seg = squash(name);
        self.developer.as_deref().map(squash).is_some_and(|d| d.len() >= 3 && (d == seg || (seg.len() >= 5 && d.contains(&seg))))
    }
}

/// Merge rule lists: the first list wins a location named twice (put the
/// most trusted source first), a location inside another one is dropped, and
/// unusable rules are left out.
pub fn merge(lists: &[&[Rule]]) -> Vec<Rule> {
    let mut out: Vec<Rule> = Vec::new();
    for list in lists {
        for rule in list.iter() {
            if rule.problem().is_some() {
                continue;
            }
            if out.iter().any(|r| r.contains(rule)) {
                continue;
            }
            // A wider location replaces the narrower ones already kept.
            out.retain(|r| !rule.contains(r));
            out.push(rule.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_never_is_a_whole_root_or_leaves_it() {
        assert!(Rule::files(Root::Documents, "", Source::Player).problem().is_some());
        assert!(Rule::files(Root::Documents, "My Games/../..", Source::Player).problem().is_some());
        assert!(Rule { path: "/etc".into(), ..Rule::files(Root::Home, "x", Source::Player) }.problem().is_some());
        assert!(Rule { path: "C:/Windows".into(), ..Rule::files(Root::Home, "x", Source::Player) }.problem().is_some());
        assert!(Rule::files(Root::Documents, "My Games/Skyrim", Source::Known).problem().is_none());
        assert!(Rule::registry("Software", Source::Known).problem().is_some());
        assert!(Rule::registry("Software/Studio/Game", Source::Known).problem().is_none());
        assert!(Rule::registry("System/CurrentControlSet", Source::Known).problem().is_some());
    }

    #[test]
    fn normalize_cleans_separators() {
        assert_eq!(normalize("\\My Games\\\\Skyrim\\./Saves/"), "My Games/Skyrim/Saves");
    }

    #[test]
    fn wildcards_match_like_windows() {
        assert!(wildcard("*.sav", "SLOT1.SAV"));
        assert!(wildcard("7656119*", "76561198000000000"));
        assert!(wildcard("save?", "save1"));
        assert!(!wildcard("*.sav", "slot1.sav.bak"));
        assert!(wildcard("*", "anything"));
    }

    #[test]
    fn placeholders_pick_the_most_specific_root() {
        let roots = SaveRoots {
            home: Some("C:/Users/mira".into()),
            documents: Some("C:/Users/mira/OneDrive/Documents".into()),
            local_app_data: Some("C:/Users/mira/AppData/Local".into()),
            virtual_store: Some("C:/Users/mira/AppData/Local/VirtualStore".into()),
            game_dir: Some("C:/Users/mira/OneDrive/Documents/Games/Celeste".into()),
            ..Default::default()
        };
        assert_eq!(
            roots.placeholder_of(Path::new("C:/Users/mira/OneDrive/Documents/My Games/Skyrim/Saves/a.ess")),
            Some((Root::Documents, "My Games/Skyrim/Saves/a.ess".into()))
        );
        assert_eq!(
            roots.placeholder_of(Path::new("C:/Users/mira/AppData/Local/VirtualStore/Program Files/Game/save.dat")),
            Some((Root::VirtualStore, "Program Files/Game/save.dat".into()))
        );
        assert_eq!(
            roots.placeholder_of(Path::new("C:/Users/mira/OneDrive/Documents/Games/Celeste/Saves/0.celeste")),
            Some((Root::GameDir, "Saves/0.celeste".into()))
        );
        assert_eq!(roots.placeholder_of(Path::new("D:/Elsewhere/file")), None);
    }

    #[cfg(windows)]
    #[test]
    fn placeholders_ignore_case_on_windows() {
        let roots = SaveRoots { documents: Some("C:\\Users\\Mira\\Documents".into()), ..Default::default() };
        assert_eq!(roots.placeholder_of(Path::new("c:\\users\\mira\\documents\\X\\s.sav")), Some((Root::Documents, "X/s.sav".into())));
    }

    #[test]
    fn a_wine_prefix_has_every_root() {
        let r = SaveRoots::for_prefix(Path::new("/p/pfx"), None);
        assert_eq!(r.public_documents, Some(PathBuf::from("/p/pfx/drive_c/users/Public/Documents")));
        assert_eq!(r.program_data, Some(PathBuf::from("/p/pfx/drive_c/ProgramData")));
    }

    #[test]
    fn merging_keeps_the_trusted_source_and_the_widest_folder() {
        let a = Rule::files(Root::Documents, "My Games/X", Source::Known);
        let b = Rule::files(Root::Documents, "My Games/X/Saves", Source::Detected);
        let c = Rule::files(Root::Documents, "my games/x", Source::Detected);
        let d = Rule::files(Root::AppData, "X", Source::Detected);
        let bad = Rule::files(Root::Documents, "", Source::Detected);
        let merged = merge(&[std::slice::from_ref(&b), &[a.clone(), c, d.clone(), bad]]);
        assert_eq!(merged, vec![a, d]);
        assert_eq!(merge(&[std::slice::from_ref(&b)]), vec![b]);
    }

    #[test]
    fn expanding_follows_wildcards_and_the_steam_id() {
        let dir = std::env::temp_dir().join(format!("kryo-saves-expand-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Game/76561198000000001/saves")).unwrap();
        std::fs::create_dir_all(dir.join("Game/76561198000000002/saves")).unwrap();
        std::fs::create_dir_all(dir.join("Game/other")).unwrap();
        let mut roots = SaveRoots { documents: Some(dir.clone()), ..Default::default() };
        let rule = Rule::files(Root::Documents, "Game/7656119*/saves", Source::Known);
        assert_eq!(expand(&rule, &roots).len(), 2);
        roots.steam_id = Some("76561198000000002".into());
        let pinned = Rule::files(Root::Documents, "Game/<steamId>/saves", Source::Known);
        assert_eq!(expand(&pinned, &roots), vec![dir.join("Game/76561198000000002/saves")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
