//! Playnite's library (github.com/JosefNemec/Playnite), read-only. Windows.
//!
//! Playnite keeps its library in `%APPDATA%\Playnite\library` (a portable
//! copy: `library` beside `Playnite.DesktopApp.exe`), one LiteDB 4 file per
//! kind of record (`source/Playnite/Database/GameDatabase.cs`): the games are
//! `games.db`, collection `Game`, and pictures sit under `files\<game id>\`,
//! named in the record relative to `files`. Playnite opens those files
//! exclusively, so they cannot be read while it runs.
//!
//! A record (`source/PlayniteSDK/Models/Game.cs`) has `_id` (a Guid), `Name`,
//! `InstallDirectory`, `IsInstalled`, `Playtime` (seconds), `LastActivity`,
//! `Added`, `CoverImage`, `BackgroundImage`, `PluginId` (the library add-on
//! that owns it: Steam, Epic...) and `GameActions` (`GameAction.cs`): `Type`
//! (File, URL, Emulator, Script), `Path`, `Arguments`, `WorkingDir`,
//! `IsPlayAction`. Paths may use Playnite's variables (`{InstallDir}`,
//! `{PlayniteDir}`...), and a relative path is inside the game's folder.
//! A game an add-on starts (most Steam, Epic or GOG games) has no play action
//! of its own: Playnite asks the add-on.

use crate::bson::Bson;
use crate::litedb::{self, LiteError};
use std::path::{Path, PathBuf};

const STEAM: &str = "cb91dfc9-b977-43bf-8e70-55f46e410fab";
const EPIC: &str = "00000002-dbd1-46c6-b5d0-b1ba559d10e4";
const GOG: &str = "aebe8b7c-6dc3-4a66-af31-e7375c6b5e9e";

/// One game from Playnite's library, as Kryoto uses it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayniteGame {
    /// Playnite's own id for it (a Guid).
    pub id: String,
    pub name: String,
    pub install_dir: Option<String>,
    pub installed: bool,
    /// How Play starts it, or why Kryoto cannot.
    pub play: Option<Play>,
    pub no_play: Option<NoPlay>,
    pub playtime_seconds: u64,
    /// Unix seconds.
    pub last_played: Option<u64>,
    pub added_at: Option<u64>,
    /// A file on this PC or a web address.
    pub cover: Option<String>,
    pub background: Option<String>,
    /// A Steam game's AppID, from Playnite's Steam add-on.
    pub steam_appid: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Play {
    /// Absolute path of the program.
    pub executable: String,
    pub arguments: Option<String>,
}

/// Why a game has no program Kryoto could start.
#[derive(Debug, Clone, PartialEq)]
pub enum NoPlay {
    /// A Playnite add-on starts it: Steam, Epic, GOG, or another (None).
    Library(Option<&'static str>),
    /// Its play action opens a link.
    Link,
    /// It runs in an emulator Playnite set up.
    Emulator,
    /// Its play action is a script.
    Script,
    /// Its play action names something only Playnite can fill in.
    Unknown,
    /// No play action at all.
    Nothing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReadError {
    /// Playnite is open and holds the file.
    Locked,
    /// No `games.db` there.
    Missing,
    /// A file this reader does not understand (a newer Playnite, or damage).
    Format(String),
    Io(String),
}

/// Where Playnite keeps its library on this PC, most likely first.
pub fn candidate_dirs(appdata: Option<&Path>) -> Vec<PathBuf> {
    appdata.map(|a| a.join("Playnite").join("library")).into_iter().collect()
}

/// The library folder in what the player picked: the library itself, or a
/// portable Playnite's folder with `library` in it.
pub fn library_in(picked: &Path) -> Option<PathBuf> {
    if picked.join("games.db").is_file() {
        return Some(picked.to_path_buf());
    }
    let inner = picked.join("library");
    inner.join("games.db").is_file().then_some(inner)
}

/// The folder `{PlayniteDir}` stands for: the installed Playnite's program
/// folder for the library in Roaming, else (portable) the library's parent.
pub fn playnite_dir(library: &Path, appdata: Option<&Path>, local_appdata: Option<&Path>) -> PathBuf {
    let roaming = appdata.map(|a| a.join("Playnite").join("library"));
    let same = |a: &Path, b: &Path| a.to_string_lossy().to_lowercase().trim_end_matches(['\\', '/']) == b.to_string_lossy().to_lowercase().trim_end_matches(['\\', '/']);
    match (roaming, local_appdata) {
        (Some(r), Some(local)) if same(&r, library) => local.join("Playnite"),
        _ => library.parent().map(Path::to_path_buf).unwrap_or_else(|| library.to_path_buf()),
    }
}

/// Every game in the library at `dir`. Only reads.
pub fn read_library(dir: &Path, playnite: &Path) -> Result<Vec<PlayniteGame>, ReadError> {
    let file = dir.join("games.db");
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ReadError::Missing),
        // Windows' sharing and lock violations: Playnite has it open.
        Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => return Err(ReadError::Locked),
        Err(e) => return Err(ReadError::Io(e.to_string())),
    };
    let db = litedb::File::open(&bytes).map_err(|e| match e {
        LiteError::NotLiteDb => ReadError::Format("This Playnite library is in a format Kryoto cannot read yet.".into()),
        LiteError::Encrypted => ReadError::Format("This Playnite library is protected with a password.".into()),
        LiteError::Damaged(what) => ReadError::Format(format!("Playnite's library looks damaged ({what}). Open Playnite once, close it, and try again.")),
    })?;
    let name = if db.collections().iter().any(|c| c.eq_ignore_ascii_case("Game")) { "Game".to_string() } else { db.collections().first().map(|c| c.to_string()).unwrap_or_default() };
    let docs = db.documents(&name).map_err(|e| ReadError::Format(format!("{e:?}")))?;
    Ok(docs.iter().filter_map(|d| game(d, dir, playnite)).collect())
}

/// One record. Pure, for the tests.
pub fn game(d: &Bson, library: &Path, playnite: &Path) -> Option<PlayniteGame> {
    let id = d.get("_id").and_then(Bson::as_guid)?;
    let name = d.str("Name")?.trim().to_string();
    let plugin = d.get("PluginId").and_then(Bson::as_guid).filter(|g| g.chars().any(|c| c != '0' && c != '-'));
    let mut vars = Vars { install: None, playnite: playnite.to_string_lossy().into_owned(), name: name.clone() };
    let install_dir = d.str("InstallDirectory").and_then(|p| vars.expand(p)).map(|p| absolute(&p, playnite));
    vars.install = install_dir.clone();
    let (play, no_play) = match play_action(d, &vars) {
        Ok(p) => (Some(p), None),
        Err(NoPlay::Nothing) if plugin.is_some() => (None, Some(NoPlay::Library(plugin.as_deref().and_then(plugin_name)))),
        Err(why) => (None, Some(why)),
    };
    let picture = |key: &str| -> Option<String> {
        let p = d.str(key)?.trim();
        if p.starts_with("https://") || p.starts_with("http://") {
            return Some(p.to_string());
        }
        let path = if Path::new(p).is_absolute() { PathBuf::from(p) } else { library.join("files").join(p) };
        path.is_file().then(|| path.to_string_lossy().into_owned())
    };
    let steam_appid = (plugin.as_deref() == Some(STEAM)).then(|| d.str("GameId").map(str::to_string)).flatten().filter(|a| !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()));
    Some(PlayniteGame {
        id,
        name,
        install_dir,
        installed: d.bool("IsInstalled").unwrap_or(false),
        play,
        no_play,
        playtime_seconds: d.get("Playtime").and_then(Bson::as_u64).unwrap_or(0),
        last_played: d.get("LastActivity").and_then(Bson::as_unix_seconds),
        added_at: d.get("Added").and_then(Bson::as_unix_seconds),
        cover: picture("CoverImage"),
        background: picture("BackgroundImage"),
        steam_appid,
    })
}

fn plugin_name(id: &str) -> Option<&'static str> {
    match id {
        STEAM => Some("Steam"),
        EPIC => Some("Epic"),
        GOG => Some("GOG"),
        _ => None,
    }
}

struct Vars {
    install: Option<String>,
    playnite: String,
    name: String,
}

impl Vars {
    /// Playnite's variables filled in; None when one is left that only
    /// Playnite knows (`{EmulatorDir}`, `{ImagePath}`...).
    fn expand(&self, text: &str) -> Option<String> {
        let mut s = text.trim().trim_matches('"').to_string();
        let install = self.install.as_deref().unwrap_or("");
        let dir_name = Path::new(install).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        for (var, value) in [("{InstallDir}", install), ("{InstallDirName}", dir_name.as_str()), ("{PlayniteDir}", self.playnite.as_str()), ("{Name}", self.name.as_str())] {
            if s.contains(var) {
                if value.is_empty() {
                    return None;
                }
                s = s.replace(var, value);
            }
        }
        let open = s.find('{');
        let left = open.and_then(|o| s[o..].find('}').map(|c| &s[o + 1..o + c]));
        if left.is_some_and(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric())) {
            return None;
        }
        Some(s)
    }
}

fn absolute(p: &str, base: &Path) -> String {
    if Path::new(p).is_absolute() || p.starts_with("\\\\") {
        p.to_string()
    } else {
        base.join(p).to_string_lossy().into_owned()
    }
}

fn play_action(d: &Bson, vars: &Vars) -> Result<Play, NoPlay> {
    let action = d.array("GameActions").iter().find(|a| a.bool("IsPlayAction") == Some(true)).ok_or(NoPlay::Nothing)?;
    let kind = match action.get("Type") {
        Some(Bson::Int32(n)) => *n,
        Some(Bson::Int64(n)) => *n as i32,
        Some(Bson::Str(s)) => match s.as_str() {
            "File" => 0,
            "URL" => 1,
            "Emulator" => 2,
            "Script" => 3,
            _ => -1,
        },
        // Playnite leaves out a field at its default: File.
        None => 0,
        _ => -1,
    };
    match kind {
        0 => {}
        1 => return Err(NoPlay::Link),
        2 => return Err(NoPlay::Emulator),
        3 => return Err(NoPlay::Script),
        _ => return Err(NoPlay::Unknown),
    }
    let path = action.str("Path").ok_or(NoPlay::Nothing)?;
    let path = vars.expand(path).ok_or(NoPlay::Unknown)?;
    if path.contains("://") {
        return Err(NoPlay::Link);
    }
    let executable = if Path::new(&path).is_absolute() || path.starts_with("\\\\") {
        path
    } else {
        let base = action.str("WorkingDir").and_then(|w| vars.expand(w)).filter(|w| Path::new(w).is_absolute()).or_else(|| vars.install.clone()).ok_or(NoPlay::Unknown)?;
        Path::new(&base).join(&path).to_string_lossy().into_owned()
    };
    let arguments = match action.str("Arguments") {
        Some(a) => Some(vars.expand(a).ok_or(NoPlay::Unknown)?),
        None => None,
    };
    Ok(Play { executable, arguments: arguments.filter(|a| !a.trim().is_empty()) })
}

/// Synthetic Playnite libraries, for tests and `examples/make-playnite-library.rs`.
#[doc(hidden)]
pub mod testlib {
    use crate::bson::Bson;

    /// A .NET Guid's bytes for `n`, so tests can name games by number.
    pub fn guid(n: u8) -> Vec<u8> {
        let mut b = vec![0x11u8; 16];
        b[15] = n;
        b
    }

    pub fn action(kind: &str, path: &str, args: Option<&str>, play: bool) -> Bson {
        let mut f = vec![("Type".to_string(), Bson::Str(kind.into())), ("Path".to_string(), Bson::Str(path.into())), ("IsPlayAction".to_string(), Bson::Bool(play))];
        if let Some(a) = args {
            f.push(("Arguments".to_string(), Bson::Str(a.into())));
        }
        Bson::Doc(f)
    }
}

/// Writes a `games.db` with these records. For tests and the example only: no
/// installer or game is ever put in one, only names of files left empty.
#[doc(hidden)]
pub fn write_library(dir: &Path, records: &[Vec<(&str, Bson)>]) -> std::io::Result<()> {
    let docs: Vec<Vec<u8>> = records.iter().map(|r| crate::bson::encode(r)).collect();
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("games.db"), litedb::testfile::build("Game", &docs, 1_500, 3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kryoto-playnite-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Playnite is Windows only, and so are its paths.
    #[test]
    #[cfg(windows)]
    fn reads_a_library_with_every_kind_of_game() {
        use super::testlib::*;
        let root = temp("kinds");
        let lib = root.join("library");
        let games = root.join("Games");
        let hk = games.join("Hollow Knight");
        std::fs::create_dir_all(&hk).unwrap();
        std::fs::write(hk.join("hollow_knight.exe"), b"").unwrap();
        std::fs::create_dir_all(lib.join("files").join("g1")).unwrap();
        std::fs::write(lib.join("files").join("g1").join("cover.jpg"), b"\xff\xd8\xff").unwrap();
        let long_notes = "notes ".repeat(2_000);
        write_library(
            &lib,
            &[
                vec![
                    ("_id", Bson::Binary(4, guid(1))),
                    ("Name", Bson::Str("Hollow Knight".into())),
                    ("InstallDirectory", Bson::Str(hk.to_string_lossy().into_owned())),
                    ("IsInstalled", Bson::Bool(true)),
                    ("Playtime", Bson::Int64(36_000)),
                    ("LastActivity", Bson::DateTime(1_760_000_000_000)),
                    ("CoverImage", Bson::Str("g1\\cover.jpg".into())),
                    ("Notes", Bson::Str(long_notes)),
                    ("GameActions", Bson::Array(vec![action("File", "{InstallDir}\\hollow_knight.exe", Some("-windowed"), true)])),
                ],
                // A Steam game: Playnite's Steam add-on starts it.
                vec![
                    ("_id", Bson::Binary(4, guid(2))),
                    ("Name", Bson::Str("Hades".into())),
                    ("PluginId", Bson::Binary(4, uuid_bytes(STEAM))),
                    ("GameId", Bson::Str("1145360".into())),
                    ("IsInstalled", Bson::Bool(true)),
                ],
                // A relative path, a unicode name, playtime as Int32.
                vec![
                    ("_id", Bson::Binary(4, guid(3))),
                    ("Name", Bson::Str("Ōkami 大神".into())),
                    ("InstallDirectory", Bson::Str("{PlayniteDir}\\..\\Games\\Okami".into())),
                    ("IsInstalled", Bson::Bool(true)),
                    ("Playtime", Bson::Int32(60)),
                    ("GameActions", Bson::Array(vec![action("File", "bin\\okami.exe", None, true), action("URL", "https://example.com", None, false)])),
                ],
                vec![
                    ("_id", Bson::Binary(4, guid(4))),
                    ("Name", Bson::Str("Web Game".into())),
                    ("GameActions", Bson::Array(vec![action("URL", "https://example.com/play", None, true)])),
                ],
                vec![("_id", Bson::Binary(4, guid(5))), ("Name", Bson::Str("No Actions".into()))],
                vec![
                    ("_id", Bson::Binary(4, guid(6))),
                    ("Name", Bson::Str("Emulated".into())),
                    ("GameActions", Bson::Array(vec![action("Emulator", "", None, true)])),
                ],
                vec![
                    ("_id", Bson::Binary(4, guid(7))),
                    ("Name", Bson::Str("Odd Variable".into())),
                    ("InstallDirectory", Bson::Str("D:\\Games\\Odd".into())),
                    ("GameActions", Bson::Array(vec![action("File", "{EmulatorDir}\\x.exe", None, true)])),
                ],
                // No name: not a game Kryoto can show.
                vec![("_id", Bson::Binary(4, guid(8)))],
            ],
        )
        .unwrap();
        let playnite = playnite_dir(&lib, None, None);
        assert_eq!(playnite, root);
        let got = read_library(&lib, &playnite).unwrap();
        assert_eq!(got.len(), 7);
        let hk_game = &got[0];
        assert_eq!(hk_game.id, "11111111-1111-1111-1111-111111111101");
        assert_eq!((hk_game.name.as_str(), hk_game.installed, hk_game.playtime_seconds, hk_game.last_played), ("Hollow Knight", true, 36_000, Some(1_760_000_000)));
        let play = hk_game.play.as_ref().unwrap();
        assert_eq!(play.executable, format!("{}\\hollow_knight.exe", hk.to_string_lossy()));
        assert_eq!(play.arguments.as_deref(), Some("-windowed"));
        assert!(hk_game.cover.as_deref().unwrap().ends_with("cover.jpg"));
        assert_eq!((got[1].no_play.clone(), got[1].steam_appid.as_deref()), (Some(NoPlay::Library(Some("Steam"))), Some("1145360")));
        let okami = &got[2];
        assert_eq!(okami.name, "Ōkami 大神");
        assert!(okami.install_dir.as_deref().unwrap().ends_with("Games\\Okami"));
        assert!(okami.play.as_ref().unwrap().executable.ends_with("Okami\\bin\\okami.exe"));
        assert_eq!(okami.playtime_seconds, 60);
        assert_eq!(got[3].no_play, Some(NoPlay::Link));
        assert_eq!(got[4].no_play, Some(NoPlay::Nothing));
        assert_eq!(got[5].no_play, Some(NoPlay::Emulator));
        assert_eq!(got[6].no_play, Some(NoPlay::Unknown));
        // Reading changed nothing.
        let before = std::fs::read(lib.join("games.db")).unwrap();
        let _ = read_library(&lib, &playnite);
        assert_eq!(std::fs::read(lib.join("games.db")).unwrap(), before);
        assert_eq!(std::fs::read_dir(&lib).unwrap().count(), 2, "no journal, lock or copy left beside it");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finds_the_library_where_the_player_points() {
        let root = temp("find");
        write_library(&root.join("library"), &[]).unwrap();
        assert_eq!(library_in(&root), Some(root.join("library")));
        assert_eq!(library_in(&root.join("library")), Some(root.join("library")));
        assert_eq!(library_in(&root.join("nowhere")), None);
        assert_eq!(read_library(&root, &root).err(), Some(ReadError::Missing));
        std::fs::write(root.join("games.db"), b"not a database").unwrap();
        assert!(matches!(read_library(&root, &root), Err(ReadError::Format(_))));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(windows)]
    fn the_installed_playnite_dir_is_in_local_appdata() {
        let roaming = Path::new("C:\\Users\\a\\AppData\\Roaming");
        let local = Path::new("C:\\Users\\a\\AppData\\Local");
        assert_eq!(playnite_dir(&roaming.join("Playnite").join("library"), Some(roaming), Some(local)), local.join("Playnite"));
        assert_eq!(playnite_dir(Path::new("E:\\Portable\\Playnite\\library"), Some(roaming), Some(local)), PathBuf::from("E:\\Portable\\Playnite"));
    }

    fn uuid_bytes(text: &str) -> Vec<u8> {
        let hex: Vec<u8> = text.split('-').collect::<String>().as_bytes().chunks(2).map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap()).collect();
        let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut out = vec![0u8; 16];
        for (i, &at) in order.iter().enumerate() {
            out[at] = hex[i];
        }
        out
    }

    #[test]
    fn guid_bytes_round_trip() {
        assert_eq!(crate::bson::guid_text(&uuid_bytes(STEAM)), STEAM);
    }
}
