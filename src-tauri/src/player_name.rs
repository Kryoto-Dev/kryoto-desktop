//! The name you have inside games: your K// username, one you type, or
//! whatever the build came with.
//!
//! Offline Steam emulators answer the game's "what is the player called"
//! question from a file beside them, and every build ships with a placeholder
//! there ("kryoto", RUNE's "RUNE"). Before a game starts, the name picked in
//! Settings (or in the game's Properties) is written into each of those files
//! in its folder, in the format that emulator reads:
//!
//! - gbe_fork / Goldberg (what Kryoto Forge builds with): `steam_settings/
//!   configs.user.ini`, `[user::general] account_name=`; older Goldberg builds
//!   also read `steam_settings/force_account_name.txt`.
//! - RUNE and CODEX-style emulators: `steam_emu.ini` (RUNE Steakclient:
//!   `steak_emu.ini`), `UserName=` under `[Settings]`.
//! - SmartSteamEmu: `SmartSteamEmu.ini`, `PersonaName=`; ALI213:
//!   `SteamConfig.ini`, `PlayerName=` - only when the key is already there.
//!
//! Kryoto Online builds use the player's real Steam account, so there is no
//! file to write and Steam's own name is what others see.
//!
//! The game's language lives in the same files (`language=` for gbe_fork,
//! `Language=` for the others, `force_language.txt` for older Goldberg), so it
//! is written in the same pass: a game asks the emulator which language to use
//! and picks that one if it has it.
//!
//! Only files that already belong to an emulator are touched, and only when
//! the name in them differs, so a game's folder is left alone when nothing
//! needs changing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

/// Steam's own limit for a persona name.
pub const MAX_LEN: usize = 32;

/// How a game's name is chosen. `Account`: the K// username. `Custom`: the
/// typed one. `Build`: leave the emulator's files alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Account,
    Custom,
    Build,
}

/// A name a game can be given: no control characters, no newlines, at most
/// `MAX_LEN` characters. None when nothing is left.
pub fn clean(name: &str) -> Option<String> {
    let s: String = name.chars().filter(|c| !c.is_control()).take(MAX_LEN).collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// The name to write, from the global setting and the game's own choice
/// (`None` follows the global one).
pub fn resolve(
    global: Mode,
    global_custom: &str,
    game: Option<Mode>,
    game_custom: &str,
    account: Option<&str>,
) -> Option<String> {
    match game.unwrap_or(global) {
        Mode::Build => None,
        Mode::Account => account.and_then(clean),
        Mode::Custom => {
            let typed = if game == Some(Mode::Custom) { game_custom } else { global_custom };
            clean(typed)
        }
    }
}

/// A language as Steam names it (`german`, `schinese`), which is what the
/// emulators hand the game. None for anything else, the game's own included.
pub fn clean_language(language: &str) -> Option<String> {
    let s = language.trim().to_ascii_lowercase();
    ((2..=16).contains(&s.len()) && s.chars().all(|c| c.is_ascii_lowercase())).then_some(s)
}

/// The language to write, from Settings and the game's own choice. `None`
/// follows Settings; an empty choice, there or here, leaves the game's files.
pub fn resolve_language(global: &str, game: Option<&str>) -> Option<String> {
    clean_language(game.unwrap_or(global))
}

/* ── the last signed-in account, remembered for launches without the Store ── */

fn account_file<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("player-name.json"))
}

#[derive(Default, Serialize, Deserialize)]
struct Remembered {
    username: Option<String>,
}

/// kryo.to said who is signed in. Kept on disk so a game started from the
/// tray, offline, or before the Store has loaded still gets the right name.
pub fn remember_account<R: Runtime>(app: &AppHandle<R>, username: Option<&str>) {
    let Some(file) = account_file(app) else { return };
    let next = Remembered { username: username.and_then(clean) };
    if remembered_account(app) == next.username {
        return;
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(&next) {
        let _ = std::fs::write(file, text);
    }
}

pub fn remembered_account<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let file = account_file(app)?;
    let text = std::fs::read_to_string(file).ok()?;
    serde_json::from_str::<Remembered>(&text).ok()?.username
}

/// The K// username Settings offers ("Use my K// username (name)").
#[tauri::command]
pub fn player_account_name(app: AppHandle) -> Option<String> {
    remembered_account(&app)
}

/* ── writing the files ───────────────────────────────────────────── */

/// Set `key` in `section` (None: the top of the file) to `value`, keeping
/// everything else - comments, order, line endings - as it was. Keys and
/// section names compare case-insensitively, as the emulators read them.
/// With `add`, a missing key (and section) is added; without, the text comes
/// back unchanged.
pub fn set_ini(text: &str, section: Option<&str>, key: &str, value: &str, add: bool) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let in_wanted = |name: &str| section.is_some_and(|s| name.eq_ignore_ascii_case(s));
    let mut current: Option<String> = None;
    let mut section_end: Option<usize> = if section.is_none() { Some(0) } else { None };
    let mut found = false;
    for (i, line) in lines.iter_mut().enumerate() {
        let trimmed = line.trim().trim_start_matches('\u{feff}').to_string();
        let trimmed = trimmed.as_str();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current = Some(trimmed[1..trimmed.len() - 1].trim().to_string());
            continue;
        }
        let here = match (&current, section) {
            (None, None) => true,
            (Some(name), Some(_)) => in_wanted(name),
            _ => false,
        };
        if !here {
            continue;
        }
        // A new key goes after the section's last line with something on it.
        if !trimmed.is_empty() {
            section_end = Some(i + 1);
        }
        if trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        let Some((k, _)) = trimmed.split_once('=') else { continue };
        if k.trim().eq_ignore_ascii_case(key) {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            // Keep the file's own spacing around `=`.
            let spaced = line.contains(" = ");
            *line = if spaced { format!("{indent}{} = {value}", k.trim()) } else { format!("{indent}{}={value}", k.trim()) };
            found = true;
            break;
        }
    }
    if !found {
        if !add {
            return text.to_string();
        }
        match (section, section_end) {
            (Some(_), Some(at)) => lines.insert(at, format!("{key}={value}")),
            (None, _) => lines.insert(0, format!("{key}={value}")),
            (Some(name), None) => {
                if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.push(format!("[{name}]"));
                lines.push(format!("{key}={value}"));
            }
        }
    }
    let mut out = lines.join(newline);
    if text.ends_with('\n') || text.is_empty() {
        out.push_str(newline);
    }
    out
}

/// Every emulator file in a game's folder that carries a player name.
pub fn name_files(root: &Path) -> Vec<(PathBuf, NameFile)> {
    let mut out = Vec::new();
    walk(root, 6, &mut 0, &mut out);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameFile {
    /// gbe_fork / Goldberg: `steam_settings/configs.user.ini` (made if missing).
    GbeUser,
    /// Goldberg: `steam_settings/force_account_name.txt`.
    GoldbergForce,
    /// RUNE / CODEX: `steam_emu.ini` or `steak_emu.ini`.
    SteamEmuIni,
    /// SmartSteamEmu.
    SmartSteamEmu,
    /// ALI213.
    Ali213,
    /// Goldberg: `steam_settings/force_language.txt`. Language only.
    GoldbergLanguage,
    /// Online-Fix's `OnlineFix.ini`. Language only.
    OnlineFix,
}

fn walk(dir: &Path, depth: usize, seen: &mut usize, out: &mut Vec<(PathBuf, NameFile)>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        *seen += 1;
        // A game folder with tens of thousands of files is not searched to
        // the bottom on every launch.
        if *seen > 50_000 {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if path.is_dir() {
            if name == "steam_settings" {
                out.push((path.join("configs.user.ini"), NameFile::GbeUser));
                let force = path.join("force_account_name.txt");
                if force.is_file() {
                    out.push((force, NameFile::GoldbergForce));
                }
                let force = path.join("force_language.txt");
                if force.is_file() {
                    out.push((force, NameFile::GoldbergLanguage));
                }
            } else if depth > 0 {
                walk(&path, depth - 1, seen, out);
            }
            continue;
        }
        let kind = match name.as_str() {
            "steam_emu.ini" | "steak_emu.ini" => Some(NameFile::SteamEmuIni),
            "smartsteamemu.ini" => Some(NameFile::SmartSteamEmu),
            "steamconfig.ini" => Some(NameFile::Ali213),
            "onlinefix.ini" => Some(NameFile::OnlineFix),
            _ => None,
        };
        if let Some(kind) = kind {
            out.push((path, kind));
        }
    }
}

/// The file's text with `name` in it, or None when nothing should be written.
pub fn renamed(kind: NameFile, text: Option<&str>, name: &str) -> Option<String> {
    let next = match (kind, text) {
        (NameFile::GbeUser, None) => format!("[user::general]\naccount_name={name}\n"),
        (NameFile::GbeUser, Some(t)) => set_ini(t, Some("user::general"), "account_name", name, true),
        (NameFile::GoldbergForce, _) => name.to_string(),
        (NameFile::SteamEmuIni, Some(t)) => set_ini(t, Some("Settings"), "UserName", name, true),
        (NameFile::SmartSteamEmu, Some(t)) => set_ini(t, Some("SmartSteamEmu"), "PersonaName", name, false),
        (NameFile::Ali213, Some(t)) => set_ini(t, Some("Settings"), "PlayerName", name, false),
        (NameFile::GoldbergLanguage | NameFile::OnlineFix, _) | (_, None) => return None,
    };
    (text != Some(next.as_str())).then_some(next)
}

/// The file's text with `language` in it, or None when nothing should be
/// written. The formats that may not have the key are left without it.
pub fn relanguaged(kind: NameFile, text: Option<&str>, language: &str) -> Option<String> {
    let next = match (kind, text) {
        (NameFile::GbeUser, None) => format!("[user::general]\nlanguage={language}\n"),
        (NameFile::GbeUser, Some(t)) => set_ini(t, Some("user::general"), "language", language, true),
        (NameFile::GoldbergLanguage, _) => language.to_string(),
        (NameFile::SteamEmuIni, Some(t)) => set_ini(t, Some("Settings"), "Language", language, true),
        (NameFile::SmartSteamEmu, Some(t)) => set_ini(t, Some("SmartSteamEmu"), "Language", language, false),
        (NameFile::Ali213, Some(t)) => set_ini(t, Some("Settings"), "Language", language, false),
        (NameFile::OnlineFix, Some(t)) => set_ini(t, Some("Main"), "Language", language, false),
        (NameFile::GoldbergForce, _) | (_, None) => return None,
    };
    (text != Some(next.as_str())).then_some(next)
}

/// Write `name` into every emulator file under `root`. Returns how many files
/// changed.
#[cfg(test)]
pub fn apply(root: &Path, name: &str) -> usize {
    write_all(&name_files(root), |kind, text| renamed(kind, text, name))
}

/// Run `edit` over each file and save what it changes. Returns how many files
/// changed. A file that cannot be read as text or written is skipped.
fn write_all(files: &[(PathBuf, NameFile)], edit: impl Fn(NameFile, Option<&str>) -> Option<String>) -> usize {
    let mut changed = 0;
    for (path, kind) in files.iter().cloned() {
        let text = match std::fs::read(&path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(t) => Some(t),
                // Not UTF-8: some other encoding we would corrupt. Leave it.
                Err(_) => continue,
            },
            Err(_) if kind == NameFile::GbeUser => None,
            Err(_) => continue,
        };
        let Some(next) = edit(kind, text.as_deref()) else { continue };
        match std::fs::write(&path, next) {
            Ok(()) => changed += 1,
            Err(e) => crate::logging::warn("player-name", &format!("could not write {}: {e}", path.display())),
        }
    }
    changed
}

/// Before a game starts: give it the name and language Settings and its
/// Properties ask for. One walk of the folder for both, none when neither is set.
pub fn apply_for_launch<R: Runtime>(app: &AppHandle<R>, game: &crate::library::LibraryGame) {
    let settings = crate::settings::load(app);
    let account = remembered_account(app);
    let name = resolve(
        settings.player_name_mode,
        &settings.player_name,
        game.player_name_mode,
        &game.player_name,
        account.as_deref(),
    );
    let language = resolve_language(&settings.game_language, game.game_language.as_deref());
    if name.is_none() && language.is_none() {
        return;
    }
    let files = name_files(Path::new(&game.install_dir));
    if let Some(name) = name {
        let changed = write_all(&files, |kind, text| renamed(kind, text, &name));
        if changed > 0 {
            crate::logging::info("player-name", &format!("{}: in-game name set in {changed} file(s)", game.title));
        }
    }
    if let Some(language) = language {
        let changed = write_all(&files, |kind, text| relanguaged(kind, text, &language));
        if changed > 0 {
            crate::logging::info("player-name", &format!("{}: language set to {language} in {changed} file(s)", game.title));
        }
    }
}

/// The languages a build says it has (gbe_fork's `steam_settings/
/// supported_languages.txt`, which also makes it ignore any other), or nothing
/// when it does not say.
pub fn supported_languages(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for (path, kind) in name_files(root) {
        if kind != NameFile::GbeUser {
            continue;
        }
        let Some(text) = path.parent().and_then(|dir| std::fs::read_to_string(dir.join("supported_languages.txt")).ok()) else { continue };
        for language in text.lines().filter_map(clean_language) {
            if !out.contains(&language) {
                out.push(language);
            }
        }
    }
    out
}

/// What Properties offers for this game.
#[tauri::command]
pub async fn game_languages(install_dir: String) -> Vec<String> {
    tauri::async_runtime::spawn_blocking(move || supported_languages(Path::new(&install_dir))).await.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_is_a_steam_name_or_nothing() {
        assert_eq!(clean_language(" German "), Some("german".into()));
        assert_eq!(clean_language(""), None);
        assert_eq!(clean_language("../x"), None);
        assert_eq!(resolve_language("french", None), Some("french".into()));
        assert_eq!(resolve_language("french", Some("german")), Some("german".into()));
        // The game's own, picked for this game, beats Settings.
        assert_eq!(resolve_language("french", Some("")), None);
        assert_eq!(resolve_language("", None), None);
    }

    #[test]
    fn writes_the_language_each_emulator_reads() {
        let gbe = "[user::general]\r\naccount_name=kryoto\r\nlanguage=english\r\n";
        assert_eq!(relanguaged(NameFile::GbeUser, Some(gbe), "french").unwrap(), "[user::general]\r\naccount_name=kryoto\r\nlanguage=french\r\n");
        assert_eq!(relanguaged(NameFile::GbeUser, Some(gbe), "english"), None);
        assert_eq!(relanguaged(NameFile::GbeUser, None, "french").unwrap(), "[user::general]\nlanguage=french\n");
        assert_eq!(relanguaged(NameFile::SteamEmuIni, Some("[Settings]\nUserName=RUNE\n"), "polish").unwrap(), "[Settings]\nUserName=RUNE\nLanguage=polish\n");
        assert_eq!(relanguaged(NameFile::GoldbergLanguage, Some("english"), "german").unwrap(), "german");
        assert_eq!(relanguaged(NameFile::OnlineFix, Some("[Main]\nLanguage=english\n"), "german").unwrap(), "[Main]\nLanguage=german\n");
        // Formats that may not have the key are never given one.
        assert_eq!(relanguaged(NameFile::OnlineFix, Some("[Main]\nFakeAppId=480\n"), "german"), None);
        assert_eq!(relanguaged(NameFile::Ali213, Some("[Settings]\nPlayerName=x\n"), "german"), None);
        assert_eq!(relanguaged(NameFile::GoldbergForce, Some("Mira"), "german"), None);
        assert_eq!(renamed(NameFile::GoldbergLanguage, Some("english"), "Mira"), None);
        assert_eq!(renamed(NameFile::OnlineFix, Some("[Main]\n"), "Mira"), None);
    }

    #[test]
    fn reads_what_a_build_says_it_has() {
        let dir = std::env::temp_dir().join(format!("kryoto-langs-{}", std::process::id()));
        let settings = dir.join("bin/steam_settings");
        std::fs::create_dir_all(&settings).unwrap();
        assert!(supported_languages(&dir).is_empty());
        std::fs::write(settings.join("supported_languages.txt"), "english\r\nfrench\r\n\r\nenglish\r\n").unwrap();
        assert_eq!(supported_languages(&dir), vec!["english".to_string(), "french".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleans_names() {
        assert_eq!(clean("  Mira \n"), Some("Mira".into()));
        assert_eq!(clean("   "), None);
        assert_eq!(clean(&"x".repeat(40)).unwrap().len(), MAX_LEN);
    }

    #[test]
    fn picks_the_name() {
        assert_eq!(resolve(Mode::Account, "", None, "", Some("mira")), Some("mira".into()));
        assert_eq!(resolve(Mode::Account, "", None, "", None), None);
        assert_eq!(resolve(Mode::Custom, "Neo", None, "", Some("mira")), Some("Neo".into()));
        assert_eq!(resolve(Mode::Custom, "", None, "", Some("mira")), None);
        assert_eq!(resolve(Mode::Account, "", Some(Mode::Custom), "Trinity", Some("mira")), Some("Trinity".into()));
        assert_eq!(resolve(Mode::Custom, "Neo", Some(Mode::Build), "", Some("mira")), None);
        assert_eq!(resolve(Mode::Build, "", Some(Mode::Account), "", Some("mira")), Some("mira".into()));
    }

    #[test]
    fn edits_gbe_fork_config_in_place() {
        let ini = "[user::general]\r\naccount_name=kryoto\r\nlanguage=english\r\nip_country=US\r\n";
        let out = renamed(NameFile::GbeUser, Some(ini), "Mira").unwrap();
        assert_eq!(out, "[user::general]\r\naccount_name=Mira\r\nlanguage=english\r\nip_country=US\r\n");
        assert_eq!(renamed(NameFile::GbeUser, Some(&out), "Mira"), None, "unchanged files are not rewritten");
        assert_eq!(renamed(NameFile::GbeUser, None, "Mira").unwrap(), "[user::general]\naccount_name=Mira\n");
        let other = "[user::saves]\nlocal_save_path=./saves\n";
        assert_eq!(
            renamed(NameFile::GbeUser, Some(other), "Mira").unwrap(),
            "[user::saves]\nlocal_save_path=./saves\n\n[user::general]\naccount_name=Mira\n"
        );
    }

    #[test]
    fn edits_rune_config_without_touching_other_sections() {
        let ini = "[Settings]\nAppId=620\nUserName=RUNE\n\n[Interfaces]\nSteamUser021\n";
        let out = renamed(NameFile::SteamEmuIni, Some(ini), "Mira").unwrap();
        assert_eq!(out, "[Settings]\nAppId=620\nUserName=Mira\n\n[Interfaces]\nSteamUser021\n");
        let missing = "[Settings]\nAppId=620\n\n[DLC]\nDLCUnlockall=1\n";
        assert_eq!(
            renamed(NameFile::SteamEmuIni, Some(missing), "Mira").unwrap(),
            "[Settings]\nAppId=620\nUserName=Mira\n\n[DLC]\nDLCUnlockall=1\n"
        );
    }

    #[test]
    fn only_replaces_existing_keys_for_other_emulators() {
        assert_eq!(renamed(NameFile::Ali213, Some("[Settings]\nAppID=1\n"), "Mira"), None);
        assert_eq!(
            renamed(NameFile::SmartSteamEmu, Some("[SmartSteamEmu]\nPersonaName = Player\n"), "Mira").unwrap(),
            "[SmartSteamEmu]\nPersonaName = Mira\n"
        );
    }

    #[test]
    fn finds_and_writes_every_emulator_file() {
        let dir = std::env::temp_dir().join(format!("kryoto-name-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin/steam_settings")).unwrap();
        std::fs::create_dir_all(dir.join("engine/rune")).unwrap();
        std::fs::write(dir.join("bin/steam_settings/configs.user.ini"), "[user::general]\naccount_name=kryoto\n").unwrap();
        std::fs::write(dir.join("engine/rune/steam_emu.ini"), "[Settings]\nUserName=RUNE\n").unwrap();
        std::fs::write(dir.join("readme.ini"), "UserName=keep\n").unwrap();
        assert_eq!(apply(&dir, "Mira"), 2);
        assert!(std::fs::read_to_string(dir.join("bin/steam_settings/configs.user.ini")).unwrap().contains("account_name=Mira"));
        assert!(std::fs::read_to_string(dir.join("engine/rune/steam_emu.ini")).unwrap().contains("UserName=Mira"));
        assert_eq!(std::fs::read_to_string(dir.join("readme.ini")).unwrap(), "UserName=keep\n");
        assert_eq!(apply(&dir, "Mira"), 0, "a second launch writes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
