//! Where the Steam emulator in a game's folder keeps the game's Steam Cloud
//! files: exact, read from the emulator's own config, no guessing.
//!
//! A game that saves through Steam's remote storage writes wherever the
//! emulator says: gbe_fork's `GSE Saves\<appid>` (or the folder its
//! `local_save_path` names), Goldberg's `Goldberg SteamEmu Saves\<appid>` (or
//! its `local_save.txt` folder), RUNE and CODEX under `Public\Documents\Steam`,
//! EMPRESS and OnlineFix their own. The emulator's achievement files there are
//! left out: cloud achievements merges those, and restoring a save must never
//! take back an achievement unlocked since.

use std::path::{Path, PathBuf};

use crate::rules::{normalize, relative, Root, Rule, Source};

/// Files in an emulator's save folder that are not the game's saves.
pub const NOT_SAVES: [&str; 3] = ["achievements.json", "achievements.ini", "achievement_images"];

/// Every `steam_settings` folder of gbe_fork or Goldberg in the game.
pub fn steam_settings_dirs(game_dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(game_dir)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| !e.file_name().to_string_lossy().starts_with(".kryoto"))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir() && e.file_name().to_string_lossy().eq_ignore_ascii_case("steam_settings"))
        .map(|e| e.into_path())
        .take(16)
        .collect()
}

/// `key` in `[section]` of an ini, case-insensitively, as the emulators read it.
pub fn ini_get(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    for line in text.lines() {
        let t = line.trim().trim_start_matches('\u{feff}');
        if t.starts_with('[') && t.ends_with(']') {
            current = t[1..t.len() - 1].trim().to_string();
            continue;
        }
        if t.starts_with(';') || t.starts_with('#') || !current.eq_ignore_ascii_case(section) {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            if k.trim().eq_ignore_ascii_case(key) {
                let v = v.trim().trim_matches('"').to_string();
                return (!v.is_empty()).then_some(v);
            }
        }
    }
    None
}

fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// A folder the emulator names relative to its DLL, as a rule in the game's
/// folder (or nothing, when it points outside it).
fn game_dir_rule(game_dir: &Path, dll_dir: &Path, named: &str) -> Option<Rule> {
    let named = named.trim().replace('\\', "/");
    // `..` is followed for real, so a path that climbs out of the game is
    // seen to be outside it rather than quietly kept in.
    let target = if Path::new(&named).is_absolute() {
        PathBuf::from(&named)
    } else {
        let mut t = dll_dir.to_path_buf();
        for seg in named.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    t.pop();
                }
                s => t.push(s),
            }
        }
        t
    };
    let rel = relative(game_dir, &target)?;
    let rule = Rule::files(Root::GameDir, rel, Source::Emulator).excluding(&NOT_SAVES);
    rule.problem().is_none().then_some(rule)
}

/// The save locations every Steam emulator Kryoto ships (or a player may have
/// put in) would use for `appid`, plus the ones a game's emulator config
/// moves into its own folder.
pub fn rules(game_dir: &Path, appid: Option<&str>) -> Vec<Rule> {
    let mut out = Vec::new();
    let mut gbe_folder = "GSE Saves".to_string();
    for settings in steam_settings_dirs(game_dir) {
        let dll_dir = settings.parent().unwrap_or(game_dir).to_path_buf();
        if let Some(text) = read_text(&settings.join("configs.user.ini")) {
            if let Some(p) = ini_get(&text, "user::saves", "local_save_path") {
                out.extend(game_dir_rule(game_dir, &dll_dir, &p));
            }
            if let Some(name) = ini_get(&text, "user::saves", "saves_folder_name") {
                if crate::rules::path_problem(&normalize(&name)).is_none() {
                    gbe_folder = normalize(&name);
                }
            }
        }
        // Goldberg: the folder's name in local_save.txt, beside the DLL or in
        // steam_settings.
        for file in [settings.join("local_save.txt"), dll_dir.join("local_save.txt")] {
            if let Some(name) = read_text(&file).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
                out.extend(game_dir_rule(game_dir, &dll_dir, &name));
            }
        }
    }
    if let Some(appid) = appid.filter(|a| !a.is_empty() && a.chars().all(|c| c.is_ascii_digit())) {
        let ex = |r: Rule| r.excluding(&NOT_SAVES);
        out.push(ex(Rule::files(Root::AppData, format!("{gbe_folder}/{appid}"), Source::Emulator)));
        if gbe_folder != "GSE Saves" {
            out.push(ex(Rule::files(Root::AppData, format!("GSE Saves/{appid}"), Source::Emulator)));
        }
        out.push(ex(Rule::files(Root::AppData, format!("Goldberg SteamEmu Saves/{appid}"), Source::Emulator)));
        out.push(ex(Rule::files(Root::AppData, format!("EMPRESS/remote/{appid}"), Source::Emulator)));
        out.push(ex(Rule::files(Root::AppData, format!("EMPRESS/{appid}"), Source::Emulator)));
        out.push(ex(Rule::files(Root::AppData, format!("Steam/CODEX/{appid}"), Source::Emulator)));
        out.push(ex(Rule::files(Root::PublicDocuments, format!("Steam/RUNE/{appid}"), Source::Emulator)));
        out.push(ex(Rule::files(Root::PublicDocuments, format!("Steam/CODEX/{appid}"), Source::Emulator)));
        out.push(ex(Rule::files(Root::PublicDocuments, format!("OnlineFix/{appid}"), Source::Emulator)));
    }
    out
}

/// The real game's AppID for a Kryoto Online build (which reports itself as
/// Spacewar, 480, to Steam): `ogAppId` in `kryoto-online.ini`.
pub fn online_appid(game_dir: &Path) -> Option<String> {
    for e in walkdir::WalkDir::new(game_dir).max_depth(6).into_iter().filter_map(|e| e.ok()) {
        if !e.file_name().to_string_lossy().eq_ignore_ascii_case("kryoto-online.ini") {
            continue;
        }
        let text = read_text(e.path())?;
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim();
                if k.trim().eq_ignore_ascii_case("ogAppId") && !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) && v != "480" {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryo-saves-emu-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn every_emulator_folder_for_the_appid_without_achievements() {
        let game = tmp("plain");
        let found = rules(&game, Some("620"));
        let paths: Vec<String> = found.iter().map(|r| format!("{}:{}", r.root.name(), r.path)).collect();
        assert!(paths.contains(&"AppData:GSE Saves/620".to_string()));
        assert!(paths.contains(&"AppData:Goldberg SteamEmu Saves/620".to_string()));
        assert!(paths.contains(&"PublicDocuments:Steam/RUNE/620".to_string()));
        assert!(paths.contains(&"PublicDocuments:OnlineFix/620".to_string()));
        assert!(found.iter().all(|r| r.excludes.iter().any(|e| e == "achievements.json") && r.source == Source::Emulator));
        assert!(rules(&game, None).is_empty());
        let _ = std::fs::remove_dir_all(&game);
    }

    #[test]
    fn gbe_local_save_path_and_folder_name_are_followed() {
        let game = tmp("gbe");
        std::fs::create_dir_all(game.join("bin/steam_settings")).unwrap();
        std::fs::write(
            game.join("bin/steam_settings/configs.user.ini"),
            "[user::general]\naccount_name=Mira\n\n[user::saves]\nlocal_save_path=./my saves\nsaves_folder_name=Kryo Saves\n",
        )
        .unwrap();
        let rules = rules(&game, Some("620"));
        assert!(rules.iter().any(|r| r.root == Root::GameDir && r.path == "bin/my saves"), "{rules:?}");
        assert!(rules.iter().any(|r| r.root == Root::AppData && r.path == "Kryo Saves/620"));
        let _ = std::fs::remove_dir_all(&game);
    }

    #[test]
    fn goldberg_local_save_txt_and_nothing_outside_the_game() {
        let game = tmp("goldberg");
        std::fs::create_dir_all(game.join("steam_settings")).unwrap();
        std::fs::write(game.join("local_save.txt"), "save_data\r\n").unwrap();
        std::fs::create_dir_all(game.join("x/steam_settings")).unwrap();
        std::fs::write(game.join("x/steam_settings/configs.user.ini"), "[user::saves]\nlocal_save_path=../../../escape\n").unwrap();
        let rules = rules(&game, None);
        assert_eq!(rules.len(), 1, "{rules:?}");
        assert_eq!((rules[0].root, rules[0].path.as_str()), (Root::GameDir, "save_data"));
        let _ = std::fs::remove_dir_all(&game);
    }

    #[test]
    fn ini_reading_is_case_insensitive_and_skips_comments() {
        let t = "\u{feff}[User::Saves]\n; local_save_path=no\nLOCAL_SAVE_PATH = \"./s\"\n";
        assert_eq!(ini_get(t, "user::saves", "local_save_path").as_deref(), Some("./s"));
        assert_eq!(ini_get(t, "user::general", "local_save_path"), None);
    }

    #[test]
    fn kryoto_online_gives_the_real_appid() {
        let game = tmp("online");
        std::fs::write(game.join("kryoto-online.ini"), "[KryotoOnline]\nogAppId=1145350\nappId=480\n").unwrap();
        assert_eq!(online_appid(&game).as_deref(), Some("1145350"));
        let _ = std::fs::remove_dir_all(&game);
    }
}
