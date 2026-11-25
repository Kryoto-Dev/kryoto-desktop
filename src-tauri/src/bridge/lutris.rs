//! Lutris on Bridge (Linux).
//!
//! In: Lutris hands its games over like Hydra: each installed Wine game (and
//! a native game started by a `.sh` script) comes with its program,
//! arguments, Wine prefix, Wine build and environment, written into
//! Kryoto's own launch settings (the launch options line carries
//! `WINEPREFIX=...` and the rest, as a player would type them; the Wine build
//! becomes the game's compatibility tool). It is matched on this PC against
//! Kryoto's catalog; one Kryoto does not list stays the player's own. Lutris's
//! library is opened read only, and can be read while Lutris runs.
//!
//! Out: Kryoto's games as Lutris games that start Kryoto Desktop with
//! `kryoto://library/<id>`, like Steam's shortcuts, so play time, saves and
//! Wine work as from the Library. `kryoto-import` writes them the way Lutris's
//! own "Add locally installed game" does, only while Lutris is closed, after
//! copying its library, all or nothing; a ledger makes it undoable.

use super::{env_path, Export, ExportGame, ExportList, ExportUser, Incoming, Outgoing, Provider, ReadProblem, Sent};
use crate::library::LibraryGame;
use kryoto_import::lutris::{self as lu, Added, LutrisGame, NewGame};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub(super) struct Lutris;

const LEDGER: &str = "bridge-lutris.json";
/// The one "account" a Lutris library has.
const USER: &str = "lutris";

/// Debug builds only: a test Lutris data folder (`examples/make-lutris-library.rs`).
/// Without one a debug build can read this PC's Lutris but never writes to it.
fn dev_dir() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    return env_path("KRYOTO_DEV_LUTRIS_DIR");
    #[cfg(not(debug_assertions))]
    None
}

fn config_dir(data: &Path) -> PathBuf {
    lu::config_for(data, env_path("XDG_CONFIG_HOME").as_deref())
}

/// The launch options line for a Lutris game: its environment and prefix
/// before `%command%`, its arguments after. Pure, for the tests.
pub(super) fn options_line(env: &[(String, String)], prefix: Option<&str>, args: Option<&str>) -> Option<String> {
    let quote = |v: &str| {
        if !v.is_empty() && !v.contains(|c: char| c.is_whitespace() || "\"'$`\\".contains(c)) {
            v.to_string()
        } else if !v.contains('\'') {
            format!("'{v}'")
        } else {
            format!("\"{}\"", v.replace('"', ""))
        }
    };
    let mut words: Vec<String> = env.iter().filter(|(k, _)| k != "WINEPREFIX" || prefix.is_none()).map(|(k, v)| format!("{k}={}", quote(v))).collect();
    if let Some(p) = prefix {
        words.push(format!("WINEPREFIX={}", quote(p)));
    }
    if words.is_empty() {
        return super::arguments_line(args);
    }
    let args = args.map(str::trim).filter(|a| !a.is_empty());
    Some(format!("{} %command%{}", words.join(" "), args.map(|a| format!(" {a}")).unwrap_or_default()))
}

/// One of Lutris's games as Bridge takes it. Pure, for the tests.
pub(super) fn incoming(g: LutrisGame, data: &Path) -> Incoming {
    let runner = g.runner.as_str();
    let script = g.exe.as_deref().is_some_and(crate::launch::is_shell_script);
    let cannot = if !g.installed {
        None
    } else if g.from_kryoto {
        Some("Sent from Kryoto: it is in your Library already.".to_string())
    } else if runner == "steam" {
        Some("Lutris starts it through Steam. Bring it in from Steam instead.".to_string())
    } else if runner == "linux" && !script && g.exe.is_some() {
        Some("A Linux program: Kryoto starts Windows games and .sh start scripts for now.".to_string())
    } else if runner != "wine" && runner != "linux" {
        Some(format!("Lutris runs it with {}, which Kryoto does not start.", if runner.is_empty() { "a runner it did not name" } else { runner }))
    } else if g.exe.is_none() {
        Some("Lutris has no program to start for it.".to_string())
    } else {
        None
    };
    let wine = runner == "wine";
    let launch_options = options_line(&g.env, g.prefix.as_deref().filter(|_| wine), g.args.as_deref());
    Incoming {
        key: g.id.to_string(),
        title: g.name,
        executable: g.exe.filter(|_| g.installed),
        playtime_seconds: g.playtime_seconds,
        last_played: g.last_played,
        added_at: g.installed_at,
        launch_options,
        proton_path: g.wine_version.as_deref().filter(|_| wine).and_then(|v| lu::wine_binary(data, v)).map(|p| p.to_string_lossy().into_owned()),
        custom_cover: g.cover,
        cannot,
        ..Default::default()
    }
}

impl Provider for Lutris {
    fn id(&self) -> &'static str {
        "lutris"
    }

    fn name(&self) -> &'static str {
        "Lutris"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux") || dev_dir().is_some()
    }

    /// Lutris's data folder: the one picked (or the Flatpak's or home folder
    /// holding it), else the usual one, else the Flatpak's.
    fn locate(&self, picked: Option<&str>) -> Option<PathBuf> {
        if let Some(dir) = picked.map(str::trim).filter(|d| !d.is_empty()) {
            return lu::data_in(Path::new(dir));
        }
        if let Some(dev) = dev_dir() {
            return lu::data_in(&dev);
        }
        let home = env_path("HOME");
        lu::candidate_data_dirs(home.as_deref(), env_path("XDG_DATA_HOME").as_deref()).into_iter().find(|d| d.join("pga.db").is_file())
    }

    fn running(&self) -> bool {
        if dev_dir().is_some() {
            return false;
        }
        super::process_running(&["lutris"], |_, exe| exe.contains("net.lutris.lutris"))
    }

    /// Lutris's SQLite library reads fine while it runs: only sending games
    /// to it waits for it to close.
    fn reads_while_open(&self) -> bool {
        true
    }

    fn read(&self, dir: &Path) -> Result<Vec<Incoming>, ReadProblem> {
        let home = env_path("HOME");
        lu::read_library(dir, &config_dir(dir), home.as_deref()).map(|games| games.into_iter().map(|g| incoming(g, dir)).collect()).map_err(ReadProblem::Other)
    }

    fn export(&self) -> Option<&dyn Export> {
        Some(self)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Ledger {
    sent: Vec<Added>,
}

fn ledger_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join(LEDGER))
}

fn load_ledger(app: &AppHandle) -> Ledger {
    ledger_file(app).ok().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn backups(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("bridge-backups"))
}

/// A Kryoto game Lutris could start through Kryoto: not one that came from
/// Lutris (it is there already).
fn eligible(g: &LibraryGame) -> bool {
    super::sendable(g) && !g.origin.as_deref().is_some_and(|o| o.starts_with("lutris:"))
}

fn dev_guard() -> Result<(), String> {
    #[cfg(debug_assertions)]
    if dev_dir().is_none() {
        return Err("A development build only writes to a test Lutris folder (KRYOTO_DEV_LUTRIS_DIR).".into());
    }
    Ok(())
}

impl Export for Lutris {
    fn list(&self, app: &AppHandle, dir: &Path, games: &[LibraryGame]) -> Result<ExportList, String> {
        let ledger = load_ledger(app);
        let list = games
            .iter()
            .filter(|g| eligible(g))
            .map(|g| ExportGame { id: g.id.clone(), title: g.title.clone(), cover: g.custom_cover.clone().or_else(|| g.cover.clone()), nsfw: g.nsfw, sent: ledger.sent.iter().any(|s| s.key == g.id) })
            .collect();
        let note = dir
            .to_string_lossy()
            .contains("net.lutris.Lutris")
            .then(|| "This Lutris is the Flatpak: it may not be able to start programs outside its sandbox, so these games may not open Kryoto.".to_string());
        Ok(ExportList { users: vec![ExportUser { id: USER.into(), name: "Lutris".into() }], games: list, note, undoable: ledger.sent.len() })
    }

    fn send(&self, app: &AppHandle, dir: &Path, _user: &str, games: Vec<Outgoing>) -> Result<Sent, String> {
        dev_guard()?;
        let exe = super::this_app()?.to_string_lossy().into_owned();
        let going: Vec<NewGame> = games
            .into_iter()
            .filter(|o| eligible(&o.game))
            .map(|o| NewGame {
                args: format!("kryoto://library/{}", o.game.id),
                key: o.game.id,
                name: o.game.title,
                exe: exe.clone(),
                directory: o.game.install_dir,
                playtime_seconds: o.game.playtime_seconds,
                last_played: o.game.last_played,
                cover: o.cover,
            })
            .collect();
        let mut ledger = load_ledger(app);
        let written = lu::send(dir, &config_dir(dir), &going, &ledger.sent, &backups(app)?, super::now())?;
        ledger.sent.retain(|a| !written.games.iter().any(|w| w.key == a.key));
        ledger.sent.extend(written.games.iter().cloned());
        // A game nothing remembers could not be undone: if the ledger cannot
        // be written, the new games go back out.
        if let Err(e) = ledger_file(app).and_then(|f| crate::saves::write_json(&f, &ledger)) {
            let new: Vec<Added> = written.games.iter().filter(|w| !load_ledger(app).sent.iter().any(|a| a.key == w.key)).cloned().collect();
            let _ = lu::take_back(dir, &new, &backups(app)?, super::now());
            return Err(format!("Kryoto could not note what it sent, so it took the games back out: {e}"));
        }
        crate::logging::info("bridge", &format!("lutris: {} games added, {} updated", written.added, written.updated));
        Ok(Sent { added: written.added, updated: written.updated, pictures: written.pictures, backup: written.backup.map(|b| b.to_string_lossy().into_owned()) })
    }

    fn undo(&self, app: &AppHandle, dir: &Path) -> Result<usize, String> {
        dev_guard()?;
        let ledger = load_ledger(app);
        let removed = lu::take_back(dir, &ledger.sent, &backups(app)?, super::now())?;
        crate::saves::write_json(&ledger_file(app)?, &Ledger::default())?;
        crate::logging::info("bridge", &format!("lutris: {removed} games taken back"));
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lutris(runner: &str, exe: Option<&str>) -> LutrisGame {
        LutrisGame { id: 7, name: "The Witcher".into(), runner: runner.into(), installed: true, exe: exe.map(String::from), playtime_seconds: 45_000, ..Default::default() }
    }

    #[test]
    fn a_wine_game_brings_its_prefix_wine_and_environment() {
        let root = std::env::temp_dir().join(format!("kryoto-lutris-bridge-{}", std::process::id()));
        let wine = root.join("runners/wine/lutris-GE-8/bin");
        std::fs::create_dir_all(&wine).unwrap();
        std::fs::write(wine.join("wine"), b"").unwrap();
        let mut g = lutris("wine", Some("/games/witcher/drive_c/witcher.exe"));
        g.prefix = Some("/games/witcher pfx".into());
        g.args = Some("-skipintro".into());
        g.wine_version = Some("lutris-GE-8".into());
        g.env = vec![("DXVK_HUD".into(), "fps".into())];
        let h = incoming(g, &root);
        assert_eq!((h.key.as_str(), h.executable.as_deref(), h.cannot), ("7", Some("/games/witcher/drive_c/witcher.exe"), None));
        assert_eq!(h.launch_options.as_deref(), Some("DXVK_HUD=fps WINEPREFIX='/games/witcher pfx' %command% -skipintro"));
        assert_eq!(h.proton_path.as_deref().map(Path::new), Some(wine.join("wine").as_path()));
        // What Kryoto's launch reads back from that line.
        let line = crate::launch::parse_line(h.launch_options.as_deref().unwrap());
        assert!(line.env.contains(&("WINEPREFIX".to_string(), "/games/witcher pfx".to_string())));
        assert_eq!(line.extra_args, "-skipintro");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn what_kryoto_cannot_start_says_why() {
        let data = Path::new("/nowhere");
        assert!(incoming(lutris("steam", None), data).cannot.unwrap().contains("from Steam instead"));
        assert!(incoming(lutris("dosbox", Some("/g/x.exe")), data).cannot.unwrap().contains("dosbox"));
        assert!(incoming(lutris("linux", Some("/g/game.x86_64")), data).cannot.unwrap().contains("Linux program"));
        assert_eq!(incoming(lutris("linux", Some("/g/start.sh")), data).cannot, None);
        assert!(incoming(lutris("wine", None), data).cannot.unwrap().contains("no program"));
        let mut sent = lutris("linux", Some("/usr/bin/kryoto-desktop"));
        sent.from_kryoto = true;
        assert!(incoming(sent, data).cannot.unwrap().contains("Library already"));
        // Not installed: no reason, just not on this PC.
        let mut gone = lutris("wine", Some("/g/x.exe"));
        gone.installed = false;
        let h = incoming(gone, data);
        assert_eq!((h.executable, h.cannot), (None, None));
    }

    #[test]
    fn the_options_line_only_when_there_is_something_to_say() {
        assert_eq!(options_line(&[], None, None), None);
        assert_eq!(options_line(&[], None, Some(" -w ")).as_deref(), Some("%command% -w"));
        // Arguments that look like settings are still arguments.
        let line = options_line(&[], None, Some("ResX=1920 ResY=1080 -windowed")).unwrap();
        let read = crate::launch::parse_line(&line);
        assert_eq!((read.env.len(), read.extra_args.as_str()), (0, "ResX=1920 ResY=1080 -windowed"));
        assert_eq!(options_line(&[("A".into(), "it's".into())], None, None).as_deref(), Some("A=\"it's\" %command%"));
        // The prefix Lutris set wins over one in the game's own environment.
        assert_eq!(options_line(&[("WINEPREFIX".into(), "/old".into())], Some("/new"), None).as_deref(), Some("WINEPREFIX=/new %command%"));
    }

    #[test]
    fn games_from_lutris_do_not_go_back_to_it() {
        let g = LibraryGame { id: "witcher".into(), install_dir: "/g/w".into(), executable: "w.exe".into(), ..Default::default() };
        assert!(eligible(&g));
        assert!(!eligible(&LibraryGame { origin: Some("lutris:7".into()), ..g.clone() }));
        assert!(!eligible(&LibraryGame { launcher: Some("steam".into()), ..g }));
    }
}
