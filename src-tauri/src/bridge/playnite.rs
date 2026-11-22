//! Playnite on Bridge (Windows). Its library is read by `kryoto-import`
//! (`playnite.rs`, LiteDB 4 read from a copy in memory), which never writes a
//! file of Playnite's. Playnite hands its games over like Hydra: each one is
//! matched on this PC against Kryoto's catalog, a game Kryoto has becomes a
//! Kryoto game, and the rest stay the player's own, never sent anywhere.
//!
//! What Kryoto can start is a game with a program of its own (a play action
//! of the File kind). One Playnite starts through an add-on (Steam, Epic,
//! GOG), a link, an emulator or a script is listed with the reason.

use super::{env_path, Incoming, Provider, ReadProblem};
use kryoto_import::playnite::{self as pn, NoPlay, PlayniteGame, ReadError};
use std::path::{Path, PathBuf};

pub(super) struct Playnite;

/// Debug builds only: a test library (`examples/make-playnite-library.rs`)
/// instead of the PC's own.
fn dev_dir() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    return env_path("KRYOTO_DEV_PLAYNITE_DIR").and_then(|d| pn::library_in(&d));
    #[cfg(not(debug_assertions))]
    None
}

/// Why a game cannot come over, as the player reads it.
fn why(no: &NoPlay) -> String {
    match no {
        NoPlay::Library(Some("Steam")) => "Playnite starts it through Steam. Bring it in from Steam instead.".into(),
        NoPlay::Library(Some(name)) => format!("Playnite starts it through {name}, which Kryoto cannot do yet."),
        NoPlay::Library(None) => "One of Playnite's add-ons starts it, which Kryoto cannot do yet.".into(),
        NoPlay::Link => "Its Play opens a link, not a program.".into(),
        NoPlay::Emulator => "It runs in an emulator, which Kryoto cannot set up yet.".into(),
        NoPlay::Script => "Playnite starts it with a script.".into(),
        NoPlay::Unknown => "Its Play uses a Playnite setting Kryoto cannot read.".into(),
        NoPlay::Nothing => "It has no Play action in Playnite.".into(),
    }
}

/// One of Playnite's games as Bridge takes it. Pure, for the tests.
pub(super) fn incoming(g: PlayniteGame) -> Incoming {
    let (executable, launch_options, cannot) = match (&g.play, &g.no_play) {
        // Not installed: left in Playnite, as "not on this PC".
        _ if !g.installed => (None, None, None),
        (Some(p), _) => (Some(p.executable.clone()), p.arguments.clone(), None),
        (None, Some(no)) => (None, None, Some(why(no))),
        (None, None) => (None, None, Some(why(&NoPlay::Nothing))),
    };
    Incoming {
        key: g.id,
        title: g.name,
        steam_appid: g.steam_appid,
        executable,
        folder: None,
        playtime_seconds: g.playtime_seconds,
        last_played: g.last_played,
        added_at: g.added_at,
        launch_options,
        // Its own pictures: copied into Kryoto when it comes over as the
        // player's own game; never sent through kryo.to.
        custom_cover: g.cover,
        custom_hero: g.background,
        cannot,
        ..Default::default()
    }
}

impl Provider for Playnite {
    fn id(&self) -> &'static str {
        "playnite"
    }

    fn name(&self) -> &'static str {
        "Playnite"
    }

    fn available(&self) -> bool {
        cfg!(windows) || dev_dir().is_some()
    }

    /// The folder the player picked (Playnite's portable folder or its
    /// `library`), else the installed Playnite's.
    fn locate(&self, picked: Option<&str>) -> Option<PathBuf> {
        if let Some(dir) = picked.map(str::trim).filter(|d| !d.is_empty()) {
            return pn::library_in(Path::new(dir));
        }
        dev_dir().or_else(|| pn::candidate_dirs(env_path("APPDATA").as_deref()).into_iter().find(|d| d.join("games.db").is_file()))
    }

    /// Playnite holds its library files while open.
    fn running(&self) -> bool {
        if dev_dir().is_some() {
            return false;
        }
        super::process_running(&["playnite.desktopapp.exe", "playnite.fullscreenapp.exe"], |_, _| false)
    }

    fn read(&self, dir: &Path) -> Result<Vec<Incoming>, ReadProblem> {
        let playnite = pn::playnite_dir(dir, env_path("APPDATA").as_deref(), env_path("LOCALAPPDATA").as_deref());
        match pn::read_library(dir, &playnite) {
            Ok(games) => Ok(games.into_iter().map(incoming).collect()),
            Err(ReadError::Locked) => Err(ReadProblem::Open),
            Err(ReadError::Missing) => Err(ReadProblem::Other("No Playnite library in that folder.".into())),
            Err(ReadError::Format(e) | ReadError::Io(e)) => Err(ReadProblem::Other(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kryoto_import::playnite::Play;

    fn game(play: Option<Play>, no: Option<NoPlay>, installed: bool) -> PlayniteGame {
        PlayniteGame { id: "g1".into(), name: "Lantern Rite".into(), installed, play, no_play: no, playtime_seconds: 5_400, cover: Some(r"C:\pn\files\g1\c.jpg".into()), ..Default::default() }
    }

    #[test]
    fn a_game_with_a_program_comes_with_its_arguments_and_pictures() {
        let h = incoming(game(Some(Play { executable: r"D:\Games\Lantern\Lantern.exe".into(), arguments: Some("-windowed".into()) }), None, true));
        assert_eq!((h.key.as_str(), h.executable.as_deref(), h.launch_options.as_deref(), h.cannot), ("g1", Some(r"D:\Games\Lantern\Lantern.exe"), Some("-windowed"), None));
        assert_eq!((h.playtime_seconds, h.custom_cover.as_deref(), h.cover), (5_400, Some(r"C:\pn\files\g1\c.jpg"), None));
    }

    #[test]
    fn the_others_say_why() {
        let steam = incoming(game(None, Some(NoPlay::Library(Some("Steam"))), true));
        assert!(steam.cannot.as_deref().unwrap().contains("from Steam instead"));
        assert!(incoming(game(None, Some(NoPlay::Link), true)).cannot.unwrap().contains("link"));
        assert!(incoming(game(None, None, true)).cannot.unwrap().contains("no Play action"));
        // Not installed: nothing to start, and no excuse needed.
        let gone = incoming(game(Some(Play { executable: "x.exe".into(), arguments: None }), None, false));
        assert_eq!((gone.executable, gone.cannot), (None, None));
    }
}
