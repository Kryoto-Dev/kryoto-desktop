//! Steam on Bridge.
//!
//! In: every game installed in the local Steam (`kryoto-import` reads its
//! folders, nothing is changed). They stay Steam's: each shows in the Library
//! as a Steam game and Play opens `steam://rungameid/<appid>`.
//!
//! Out: Kryoto's games as non-Steam shortcuts in
//! `userdata/<account>/config/shortcuts.vdf`, with their pictures in
//! `config/grid`. A shortcut starts Kryoto Desktop itself with
//! `kryoto://library/<id>`, so play time, saves, Wine or Proton and the rest
//! work as from the Library; to Steam it is a native program, which is also
//! what a Steam Deck needs (no compatibility tool to set up on Steam's side).
//! On Linux, Steam starts what it runs with its overlay preloaded and its
//! runtime's libraries first, which are not for a WebKit app, so the shortcut
//! drops both.
//!
//! Steam reads that file when it starts and writes it when it closes, so it
//! must be closed. The file is copied first, a file that cannot be reproduced
//! byte for byte is refused, every other shortcut is left exactly as it was,
//! and a ledger of what was added makes it undoable.

use super::{env_path, Export, ExportGame, ExportList, ExportUser, Incoming, Outgoing, Provider, ReadProblem, Sent};
use crate::library::LibraryGame;
use kryoto_import::steam::{self as st, NewShortcut, Shortcuts};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub(super) struct Steam;

const LEDGER: &str = "bridge-steam.json";
const TAG: &str = "Kryoto";

/// Debug builds only: a fake Steam folder. Without one a debug build can
/// read this PC's Steam but never writes to it.
fn dev_dir() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    return env_path("KRYOTO_DEV_STEAM_DIR");
    #[cfg(not(debug_assertions))]
    None
}

fn is_steam(dir: &Path) -> bool {
    dir.join("steamapps").is_dir() || dir.join("userdata").is_dir()
}

/// One shortcut Kryoto added: enough to take it back out.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct SentGame {
    user: String,
    appid: u32,
    game: String,
    /// The pictures written for it.
    files: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Ledger {
    sent: Vec<SentGame>,
}

fn ledger_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join(LEDGER))
}

fn load_ledger(app: &AppHandle) -> Ledger {
    ledger_file(app).ok().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn config_dir(root: &Path, user: &str) -> Result<PathBuf, String> {
    if user.is_empty() || !user.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Not a Steam account.".into());
    }
    Ok(root.join("userdata").join(user).join("config"))
}

/// The account's shortcuts, or a refusal: nothing is ever written over a file
/// that could not be read back exactly.
fn read_shortcuts(file: &Path) -> Result<Shortcuts, String> {
    match std::fs::read(file) {
        Ok(bytes) => Shortcuts::parse(&bytes).ok_or_else(|| "Steam's list of non-Steam games could not be read, so nothing was changed.".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Shortcuts::empty()),
        Err(e) => Err(format!("Could not read Steam's list of non-Steam games: {e}")),
    }
}

/// Written through a temporary file, so a crash leaves the old list or the
/// new one, never half of one.
fn write_shortcuts(file: &Path, s: &Shortcuts) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("vdf.kryoto-tmp");
    std::fs::write(&tmp, s.to_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, file).map_err(|e| e.to_string())
}

/// A copy of the file as it was, in Kryoto's folder.
fn back_up(app: &AppHandle, user: &str, file: &Path) -> Result<Option<String>, String> {
    if !file.is_file() {
        return Ok(None);
    }
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("bridge-backups");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let copy = dir.join(format!("shortcuts-{user}-{}.vdf", super::now()));
    std::fs::copy(file, &copy).map_err(|e| format!("Could not copy Steam's list first, so nothing was changed: {e}"))?;
    Ok(Some(copy.to_string_lossy().into_owned()))
}

/// The program a shortcut starts: this app, as Steam can reach it.
fn this_app() -> Result<PathBuf, String> {
    // The AppImage, not the temporary mount it runs from.
    if let Some(p) = env_path("APPIMAGE") {
        return Ok(p);
    }
    std::env::current_exe().map_err(|e| e.to_string())
}

/// What the shortcut hands Kryoto. Library ids are lower case letters, digits
/// and dashes, which is what a link can carry.
fn link_to(g: &LibraryGame) -> String {
    format!("kryoto://library/{}", g.id)
}

/// The shortcut's launch options, as Steam reads them.
fn options_for(g: &LibraryGame) -> String {
    if cfg!(windows) {
        link_to(g)
    } else {
        format!("env -u LD_PRELOAD -u LD_LIBRARY_PATH %command% {}", link_to(g))
    }
}

fn shortcut_for(g: &LibraryGame, exe: &Path, icon: String) -> NewShortcut {
    NewShortcut {
        name: g.title.clone(),
        exe: exe.to_string_lossy().into_owned(),
        start_dir: exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        launch_options: options_for(g),
        icon,
        tag: TAG.into(),
    }
}

/// `grid` wants the right name and extension for each kind of picture.
fn ext(bytes: &[u8]) -> &'static str {
    match crate::managed_art::mime(bytes) {
        "image/png" => "png",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

/// A picture into `grid`. One already there that Kryoto did not write (the
/// player's own art for that shortcut) is left alone.
fn put(grid: &Path, name: &str, bytes: &[u8], ours: &[String], out: &mut Vec<String>) -> Result<Option<String>, String> {
    let file = grid.join(format!("{name}.{}", ext(bytes)));
    let shown = file.to_string_lossy().into_owned();
    if file.exists() && !ours.contains(&shown) {
        return Ok(None);
    }
    std::fs::create_dir_all(grid).map_err(|e| e.to_string())?;
    std::fs::write(&file, bytes).map_err(|e| format!("Could not write {}: {e}", file.display()))?;
    out.push(shown.clone());
    Ok(Some(shown))
}

fn eligible(g: &LibraryGame) -> bool {
    let linkable = crate::links::parse(&link_to(g)).is_some_and(|l| l.slug == g.id);
    g.launcher.is_none() && linkable && !g.install_dir.is_empty() && (!g.executable.is_empty() || !g.entries.is_empty())
}

impl Provider for Steam {
    fn id(&self) -> &'static str {
        "steam"
    }

    fn name(&self) -> &'static str {
        "Steam"
    }

    fn locate(&self, picked: Option<&str>) -> Option<PathBuf> {
        if let Some(dir) = picked.map(str::trim).filter(|d| !d.is_empty()) {
            let p = PathBuf::from(dir);
            return is_steam(&p).then_some(p);
        }
        if let Some(dev) = dev_dir() {
            return is_steam(&dev).then_some(dev);
        }
        crate::vr::steam_roots().into_iter().find(|r| is_steam(r))
    }

    fn running(&self) -> bool {
        // A test against a fake folder has nothing to do with the real Steam.
        if dev_dir().is_some() {
            return false;
        }
        super::process_running(&["steam.exe", "steam", "steamwebhelper", "steamwebhelper.exe"], |_, _| false)
    }

    fn read(&self, dir: &Path) -> Result<Vec<Incoming>, ReadProblem> {
        let user = st::users(dir).into_iter().next().map(|u| u.id);
        Ok(st::read_games(dir, user.as_deref())
            .into_iter()
            .map(|g| {
                // Steam's usual picture, for the review; the real art comes
                // from its store API when the game is brought over.
                let cover = format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/library_600x900.jpg", g.appid);
                Incoming {
                    key: g.appid.clone(),
                    title: g.name,
                    steam_appid: Some(g.appid),
                    folder: Some(g.install_dir.to_string_lossy().into_owned()),
                    playtime_seconds: g.playtime_seconds,
                    last_played: g.last_played,
                    cover: Some(cover),
                    ..Default::default()
                }
            })
            .collect())
    }

    fn keeps_games(&self) -> bool {
        true
    }

    fn launch_url(&self, game: &Incoming) -> Option<String> {
        game.steam_appid.as_ref().map(|id| format!("steam://rungameid/{id}"))
    }

    fn export(&self) -> Option<&dyn Export> {
        Some(self)
    }
}

impl Export for Steam {
    fn list(&self, app: &AppHandle, dir: &Path, games: &[LibraryGame]) -> Result<ExportList, String> {
        let ledger = load_ledger(app);
        let users: Vec<ExportUser> = st::users(dir).into_iter().map(|u| ExportUser { id: u.id, name: u.name }).collect();
        let list = games
            .iter()
            .filter(|g| eligible(g))
            .map(|g| ExportGame {
                id: g.id.clone(),
                title: g.title.clone(),
                cover: g.custom_cover.clone().or_else(|| g.cover.clone()),
                nsfw: g.nsfw,
                sent: ledger.sent.iter().any(|s| s.game == g.id),
            })
            .collect();
        let at = dir.to_string_lossy();
        let sandbox = if at.contains("com.valvesoftware.Steam") {
            Some("Flatpak")
        } else if at.contains("/snap/steam/") {
            Some("Snap")
        } else {
            None
        };
        let note = sandbox.map(|kind| format!("This Steam is the {kind}: it cannot start programs outside its sandbox, so the shortcuts may not open Kryoto."));
        let note = note.or_else(|| users.is_empty().then(|| "Sign in to Steam once on this PC first: it has no account folder yet.".to_string()));
        Ok(ExportList { users, games: list, note, undoable: ledger.sent.len() })
    }

    fn send(&self, app: &AppHandle, dir: &Path, user: &str, games: Vec<Outgoing>) -> Result<Sent, String> {
        #[cfg(debug_assertions)]
        if dev_dir().is_none() {
            return Err("A development build only writes to a test Steam folder (KRYOTO_DEV_STEAM_DIR).".into());
        }
        let config = config_dir(dir, user)?;
        let file = config.join("shortcuts.vdf");
        let mut shortcuts = read_shortcuts(&file)?;
        let backup = back_up(app, user, &file)?;
        let mut ledger = load_ledger(app);
        let exe = this_app()?;
        let grid = config.join("grid");
        let mut out = Sent { backup, ..Default::default() };
        let mut seen = std::collections::HashSet::new();
        for o in &games {
            let mut shortcut = shortcut_for(&o.game, &exe, String::new());
            // A shortcut's id comes from its program and name, so two games
            // of one name would be one shortcut.
            let id = shortcut.appid();
            if !seen.insert(id) || ledger.sent.iter().any(|s| s.user == user && s.appid == id && s.game != o.game.id) {
                shortcut.name = format!("{} ({})", o.game.title, o.game.id);
            }
            let id = shortcut.appid();
            seen.insert(id);
            let earlier: Vec<String> = ledger.sent.iter().filter(|s| s.user == user && s.appid == id).flat_map(|s| s.files.clone()).collect();
            let mut files = Vec::new();
            // A picture that cannot be written costs that picture, not the game.
            let mut picture = |name: String, bytes: &Option<Vec<u8>>| -> Option<String> {
                match put(&grid, &name, bytes.as_ref()?, &earlier, &mut files) {
                    Ok(written) => written,
                    Err(e) => {
                        crate::logging::warn("bridge", &e);
                        None
                    }
                }
            };
            picture(format!("{id}p"), &o.cover);
            picture(id.to_string(), &o.header);
            picture(format!("{id}_hero"), &o.hero);
            picture(format!("{id}_logo"), &o.logo);
            // Steam takes any picture for the icon: the logo reads best small.
            shortcut.icon = picture(format!("{id}_icon"), if o.logo.is_some() { &o.logo } else { &o.cover }).unwrap_or_default();
            out.pictures += files.len();
            let again = shortcuts.has(id);
            shortcuts.upsert(&shortcut);
            if again {
                out.updated += 1;
            } else {
                out.added += 1;
            }
            let keep: Vec<String> = earlier.into_iter().chain(files).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
            ledger.sent.retain(|s| !(s.user == user && s.appid == id));
            ledger.sent.push(SentGame { user: user.to_string(), appid: id, game: o.game.id.clone(), files: keep });
        }
        // The ledger first: a shortcut nothing remembers could not be undone.
        crate::saves::write_json(&ledger_file(app)?, &ledger)?;
        write_shortcuts(&file, &shortcuts)?;
        crate::logging::info("bridge", &format!("steam: {} shortcuts added, {} updated", out.added, out.updated));
        Ok(out)
    }

    fn undo(&self, app: &AppHandle, dir: &Path) -> Result<usize, String> {
        #[cfg(debug_assertions)]
        if dev_dir().is_none() {
            return Err("A development build only writes to a test Steam folder (KRYOTO_DEV_STEAM_DIR).".into());
        }
        let ledger = load_ledger(app);
        let mut users: Vec<String> = ledger.sent.iter().map(|s| s.user.clone()).collect();
        users.sort();
        users.dedup();
        let mut removed = 0;
        for user in users {
            let file = config_dir(dir, &user)?.join("shortcuts.vdf");
            if !file.is_file() {
                continue;
            }
            let mut shortcuts = read_shortcuts(&file)?;
            back_up(app, &user, &file)?;
            let ids: Vec<u32> = ledger.sent.iter().filter(|s| s.user == user).map(|s| s.appid).collect();
            removed += shortcuts.remove(&ids);
            write_shortcuts(&file, &shortcuts)?;
        }
        for s in &ledger.sent {
            for f in &s.files {
                let _ = std::fs::remove_file(f);
            }
        }
        crate::saves::write_json(&ledger_file(app)?, &Ledger::default())?;
        crate::logging::info("bridge", &format!("steam: {removed} shortcuts taken back"));
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_kryotos_own_entries_go_out() {
        let g = LibraryGame { id: "hades".into(), install_dir: "D:/Games/Hades".into(), executable: "Hades.exe".into(), ..Default::default() };
        assert!(eligible(&g));
        // A Steam game is in Steam already.
        assert!(!eligible(&LibraryGame { launcher: Some("steam".into()), ..g.clone() }));
        assert!(!eligible(&LibraryGame { executable: String::new(), ..g.clone() }));
        // A link could not name it.
        assert!(!eligible(&LibraryGame { id: "Bad Id".into(), ..g }));
    }

    #[test]
    fn a_shortcut_starts_kryoto_with_the_game() {
        let g = LibraryGame { id: "hades".into(), title: "Hades".into(), ..Default::default() };
        let s = shortcut_for(&g, Path::new("/usr/bin/kryoto-desktop"), String::new());
        assert_eq!((s.start_dir.as_str(), s.tag.as_str()), ("/usr/bin", TAG));
        assert!(s.launch_options.ends_with("kryoto://library/hades"));
        if !cfg!(windows) {
            assert!(s.launch_options.contains("%command%") && s.launch_options.contains("-u LD_PRELOAD"));
        }
        assert!(crate::links::parse(&link_to(&g)).is_some());
    }

    #[test]
    fn pictures_keep_their_kind_and_never_replace_the_players_own() {
        let dir = std::env::temp_dir().join(format!("kryoto-grid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let png = b"\x89PNG\r\n\x1a\nart";
        let mut files = Vec::new();
        let first = put(&dir, "123p", png, &[], &mut files).unwrap().unwrap();
        assert!(first.ends_with("123p.png"));
        // The player's file with that name: left alone, not recorded.
        std::fs::write(dir.join("456p.jpg"), b"mine").unwrap();
        let mut none = Vec::new();
        assert_eq!(put(&dir, "456p", b"\xff\xd8\xffnew", &[], &mut none).unwrap(), None);
        assert!(none.is_empty());
        assert_eq!(std::fs::read(dir.join("456p.jpg")).unwrap(), b"mine");
        // One Kryoto wrote before is updated.
        assert!(put(&dir, "123p", b"\x89PNG\r\n\x1a\nnew", &files.clone(), &mut files).unwrap().is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_broken_list_is_refused_and_a_missing_one_is_new() {
        let dir = std::env::temp_dir().join(format!("kryoto-sc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shortcuts.vdf");
        assert!(read_shortcuts(&file).unwrap().is_empty());
        std::fs::write(&file, b"garbage").unwrap();
        assert!(read_shortcuts(&file).unwrap_err().contains("nothing was changed"));
        assert!(config_dir(&dir, "../x").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
