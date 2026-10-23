//! Save locations a game engine decides, read from the game's own files.
//!
//! - Unity: `<Game>_Data/app.info` holds the company and product names, which
//!   are the game's folder under AppData\LocalLow and its PlayerPrefs key in
//!   the registry.
//! - Unreal: `<Project>/Saved/SaveGames`, under AppData\Local or, for a
//!   portable build, in the game's folder.
//! - Ren'Py: `AppData\RenPy\<config.save_directory>` and `game/saves`.
//! - RPG Maker: `save/` or `www/save/`, and VX Ace's `Save*.rvdata2`.
//! - GameMaker and Godot: their AppData folder named after the game, when it is
//!   there.

use std::path::Path;

use crate::rules::{relative, GameHints, Root, Rule, SaveRoots, Source};

fn lines(path: &Path) -> Vec<String> {
    std::fs::read(path)
        .map(|b| String::from_utf8_lossy(&b).lines().map(|l| l.trim().trim_start_matches('\u{feff}').to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

fn entries(dir: &Path) -> Vec<std::fs::DirEntry> {
    std::fs::read_dir(dir).map(|r| r.flatten().take(2000).collect()).unwrap_or_default()
}

/// A folder name a path can carry: no separators or `..`.
fn safe_name(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', ':']) && s.len() <= 120).then(|| s.to_string())
}

/// Unity's company and product, from every `*_Data/app.info` near the top.
pub fn unity_identity(game_dir: &Path) -> Option<(String, String)> {
    let mut dirs = vec![game_dir.to_path_buf()];
    dirs.extend(entries(game_dir).into_iter().filter(|e| e.path().is_dir()).map(|e| e.path()));
    for dir in dirs {
        for e in entries(&dir) {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.to_ascii_lowercase().ends_with("_data") || !e.path().is_dir() {
                continue;
            }
            let info = lines(&e.path().join("app.info"));
            if let (Some(company), Some(product)) = (info.first().and_then(|c| safe_name(c)), info.get(1).and_then(|p| safe_name(p))) {
                return Some((company, product));
            }
        }
    }
    None
}

/// Unreal project folders: a folder holding `Binaries/Win64` (or Win32) and
/// `Content`, at the top of the game or one level down.
fn unreal_projects(game_dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in entries(game_dir) {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name.eq_ignore_ascii_case("Engine") {
            continue;
        }
        let is_project = (p.join("Binaries").join("Win64").is_dir() || p.join("Binaries").join("Win32").is_dir()) && p.join("Content").is_dir();
        if is_project {
            if let (Some(n), Some(rel)) = (safe_name(&name), relative(game_dir, &p)) {
                out.push((n, rel));
            }
        }
    }
    out
}

/// `config.save_directory = "Name-1234"` from a Ren'Py game's scripts.
fn renpy_save_directory(game_dir: &Path) -> Option<String> {
    let game = game_dir.join("game");
    if !game.is_dir() || !(game_dir.join("renpy").is_dir() || game_dir.join("lib").is_dir()) {
        return None;
    }
    for file in ["options.rpy", "gui.rpy", "script.rpy"] {
        let text = std::fs::read_to_string(game.join(file)).unwrap_or_default();
        for line in text.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("define config.save_directory").or_else(|| l.strip_prefix("config.save_directory")) {
                let value = rest.split_once('=').map(|(_, v)| v.trim()).unwrap_or("");
                let value = value.trim_matches(|c| c == '"' || c == '\'');
                if let Some(v) = safe_name(value).filter(|v| v != "None") {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// The engine's save locations for this game.
pub fn rules(game_dir: &Path, roots: &SaveRoots, hints: &GameHints) -> Vec<Rule> {
    let mut out = Vec::new();
    if let Some((company, product)) = unity_identity(game_dir) {
        out.push(Rule::files(Root::LocalLow, format!("{company}/{product}"), Source::Engine).excluding(&["Unity", "Player.log", "Player-prev.log"]));
        out.push(Rule::registry(format!("Software/{company}/{product}"), Source::Engine));
    }
    for (project, rel) in unreal_projects(game_dir) {
        out.push(Rule::files(Root::LocalAppData, format!("{project}/Saved/SaveGames"), Source::Engine));
        out.push(Rule::files(Root::GameDir, format!("{rel}/Saved/SaveGames"), Source::Engine));
    }
    if let Some(dir) = renpy_save_directory(game_dir) {
        out.push(Rule::files(Root::AppData, format!("RenPy/{dir}"), Source::Engine));
        out.push(Rule::files(Root::GameDir, "game/saves", Source::Engine));
    }
    // RPG Maker MV/MZ (www/ or the top) and VX Ace.
    if game_dir.join("www").join("data").join("System.json").is_file() || game_dir.join("www").join("save").is_dir() {
        out.push(Rule::files(Root::GameDir, "www/save", Source::Engine));
    }
    if game_dir.join("data").join("System.json").is_file() && game_dir.join("js").is_dir() {
        out.push(Rule::files(Root::GameDir, "save", Source::Engine));
    }
    if entries(game_dir).iter().any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".rgss3a")) {
        out.push(Rule::files(Root::GameDir, "Save*.rvdata2", Source::Engine));
    }
    let has = |ext: &str| entries(game_dir).iter().any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(ext));
    // GameMaker: %LOCALAPPDATA%\<game>. Godot: %APPDATA%\Godot\app_userdata\<project>.
    // Neither name is in the game's files in a readable way, so only a folder
    // that is already there and is named after the game counts.
    let named_dirs = |root: Root, under: &str| -> Vec<Rule> {
        let Some(base) = roots.get(root) else { return Vec::new() };
        entries(&crate::rules::join(base, under))
            .into_iter()
            .filter(|e| e.path().is_dir() && hints.matches(&e.file_name().to_string_lossy()))
            .filter_map(|e| safe_name(&e.file_name().to_string_lossy()))
            .map(|n| Rule::files(root, if under.is_empty() { n } else { format!("{under}/{n}") }, Source::Engine))
            .take(2)
            .collect()
    };
    if game_dir.join("data.win").is_file() || game_dir.join("options.ini").is_file() && has(".win") {
        out.extend(named_dirs(Root::LocalAppData, ""));
    }
    if has(".pck") {
        out.extend(named_dirs(Root::AppData, "Godot/app_userdata"));
    }
    out.retain(|r| r.problem().is_none());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryo-saves-engine-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn unity_gives_locallow_and_playerprefs() {
        let g = tmp("unity");
        std::fs::create_dir_all(g.join("Hollow Knight_Data")).unwrap();
        std::fs::write(g.join("Hollow Knight_Data/app.info"), "Team Cherry\r\nHollow Knight\r\n").unwrap();
        let r = rules(&g, &SaveRoots::default(), &GameHints::default());
        assert!(r.contains(&Rule::files(Root::LocalLow, "Team Cherry/Hollow Knight", Source::Engine).excluding(&["Unity", "Player.log", "Player-prev.log"])));
        assert!(r.contains(&Rule::registry("Software/Team Cherry/Hollow Knight", Source::Engine)));
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn unity_identity_with_slashes_is_refused() {
        let g = tmp("unity-bad");
        std::fs::create_dir_all(g.join("X_Data")).unwrap();
        std::fs::write(g.join("X_Data/app.info"), "..\\..\nWindows\n").unwrap();
        assert_eq!(unity_identity(&g), None);
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn unreal_projects_give_saved_savegames() {
        let g = tmp("unreal");
        std::fs::create_dir_all(g.join("Stray/Binaries/Win64")).unwrap();
        std::fs::create_dir_all(g.join("Stray/Content/Paks")).unwrap();
        std::fs::create_dir_all(g.join("Engine/Binaries/Win64")).unwrap();
        let r = rules(&g, &SaveRoots::default(), &GameHints::default());
        assert!(r.contains(&Rule::files(Root::LocalAppData, "Stray/Saved/SaveGames", Source::Engine)));
        assert!(r.contains(&Rule::files(Root::GameDir, "Stray/Saved/SaveGames", Source::Engine)));
        assert_eq!(r.len(), 2);
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn renpy_reads_the_save_directory() {
        let g = tmp("renpy");
        std::fs::create_dir_all(g.join("game")).unwrap();
        std::fs::create_dir_all(g.join("renpy")).unwrap();
        std::fs::write(g.join("game/options.rpy"), "## x\ndefine config.save_directory = \"DDLC-1454445547\"\n").unwrap();
        let r = rules(&g, &SaveRoots::default(), &GameHints::default());
        assert!(r.contains(&Rule::files(Root::AppData, "RenPy/DDLC-1454445547", Source::Engine)));
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn godot_folder_only_when_it_is_named_after_the_game() {
        let g = tmp("godot");
        let app = tmp("godot-appdata");
        std::fs::write(g.join("Brotato.pck"), b"x").unwrap();
        std::fs::create_dir_all(app.join("Godot/app_userdata/Brotato")).unwrap();
        std::fs::create_dir_all(app.join("Godot/app_userdata/Other Game")).unwrap();
        let roots = SaveRoots { app_data: Some(app.clone()), ..Default::default() };
        let hints = GameHints { title: "Brotato".into(), exe_stem: "Brotato".into(), ..Default::default() };
        let r = rules(&g, &roots, &hints);
        assert_eq!(r, vec![Rule::files(Root::AppData, "Godot/app_userdata/Brotato", Source::Engine)]);
        let _ = std::fs::remove_dir_all(&g);
        let _ = std::fs::remove_dir_all(&app);
    }
}
