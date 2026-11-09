//! Steam, from its folders.
//!
//! Reading: which games are installed (`steamapps/libraryfolders.vdf`, then an
//! `appmanifest_<id>.acf` per game in every library) and how long each was
//! played (`userdata/<account>/config/localconfig.vdf`, in minutes), never
//! changing a file.
//!
//! Writing: non-Steam shortcuts in `userdata/<account>/config/shortcuts.vdf`.
//! Only through [`Shortcuts`], which refuses a file it cannot reproduce byte
//! for byte, so nothing of the player's is lost to a misunderstanding.

use crate::vdf::{self, Bin, Text};
use std::path::{Path, PathBuf};

/// Steam's id space: an account's id64 is this plus its folder's number.
const ID64_BASE: u64 = 76_561_197_960_265_728;

#[derive(Debug, Clone, PartialEq)]
pub struct SteamGame {
    pub appid: String,
    pub name: String,
    pub install_dir: PathBuf,
    pub playtime_seconds: u64,
    pub last_played: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamUser {
    /// The folder under `userdata`.
    pub id: String,
    pub name: String,
    pub recent: bool,
}

/// Steam itself: the runtimes and Proton are installed like games, and are not.
fn is_tool(appid: &str, name: &str) -> bool {
    const TOOLS: &[&str] = &["228980", "1070560", "1391110", "1628350", "2348590", "1826330", "250820"];
    let n = name.to_ascii_lowercase();
    TOOLS.contains(&appid)
        || n.starts_with("proton ")
        || n == "proton experimental"
        || n.starts_with("steam linux runtime")
        || n.starts_with("steamworks common")
        || n.contains("dedicated server")
}

/// Every library: Steam's own folder, and each one `libraryfolders.vdf` names
/// (both layouts: `"1" { "path" ".." }`, and the older `"1" ".."`).
pub fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    let text = std::fs::read(root.join("steamapps").join("libraryfolders.vdf")).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    if let Some(t) = text.as_deref().and_then(vdf::parse_text) {
        for (key, value) in t.get("libraryfolders").map(Text::entries).unwrap_or_default() {
            if !key.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let path = value.str().or_else(|| value.get("path").and_then(Text::str));
            if let Some(p) = path.map(PathBuf::from).filter(|p| p.as_os_str().len() > 1) {
                if !out.iter().any(|o| same_folder(o, &p)) {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn same_folder(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| {
        let s = p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_string();
        if cfg!(windows) { s.to_ascii_lowercase() } else { s }
    };
    n(a) == n(b)
}

/// One `appmanifest_*.acf`: a game fully installed in `library`, or None for
/// a download in progress, a tool, or something unreadable.
fn manifest(library: &Path, text: &str) -> Option<(String, String, PathBuf, Option<u64>)> {
    let t = vdf::parse_text(text)?;
    let app = t.get("AppState")?;
    let s = |k: &str| app.get(k).and_then(Text::str);
    let appid = s("appid")?.to_string();
    let name = s("name").map(String::from).unwrap_or_else(|| format!("Steam game {appid}"));
    // 4 is "fully installed"; an update waiting keeps it set.
    let flags: u32 = s("StateFlags").and_then(|f| f.parse().ok()).unwrap_or(0);
    if flags & 4 == 0 || is_tool(&appid, &name) {
        return None;
    }
    let dir = library.join("steamapps").join("common").join(s("installdir")?);
    let last = s("LastPlayed").and_then(|v| v.parse().ok()).filter(|v| *v > 0);
    Some((appid, name, dir, last))
}

/// The Steam accounts that used this PC, the last one first.
pub fn users(root: &Path) -> Vec<SteamUser> {
    let names = std::fs::read(root.join("config").join("loginusers.vdf")).ok().and_then(|b| vdf::parse_text(&String::from_utf8_lossy(&b)));
    let mut out: Vec<SteamUser> = std::fs::read_dir(root.join("userdata"))
        .map(|r| {
            r.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| {
                    let id = e.file_name().to_string_lossy().into_owned();
                    let n: u64 = id.parse().ok().filter(|n| *n > 0)?;
                    let entry = names.as_ref().and_then(|t| t.path(&["users", &(n + ID64_BASE).to_string()]));
                    let name = entry.and_then(|u| u.get("PersonaName")).and_then(Text::str).map(String::from).unwrap_or_else(|| id.clone());
                    let recent = entry.and_then(|u| u.get("MostRecent")).and_then(Text::str) == Some("1");
                    Some(SteamUser { id, name, recent })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.recent.cmp(&a.recent).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Play time in seconds and last played, by AppID, from one account's
/// `localconfig.vdf` (Steam keeps minutes there).
pub fn playtimes(root: &Path, user: &str) -> Vec<(String, u64, Option<u64>)> {
    let file = root.join("userdata").join(user).join("config").join("localconfig.vdf");
    let Some(t) = std::fs::read(file).ok().and_then(|b| vdf::parse_text(&String::from_utf8_lossy(&b))) else { return Vec::new() };
    let apps = t.path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"]);
    apps.map(Text::entries)
        .unwrap_or_default()
        .iter()
        .map(|(id, v)| {
            let n = |k: &str| v.get(k).and_then(Text::str).and_then(|s| s.parse::<u64>().ok());
            (id.clone(), n("Playtime").unwrap_or(0) * 60, n("LastPlayed").filter(|t| *t > 0))
        })
        .collect()
}

/// Every game installed, with the hours of `user` when there is one.
pub fn read_games(root: &Path, user: Option<&str>) -> Vec<SteamGame> {
    let played = user.map(|u| playtimes(root, u)).unwrap_or_default();
    let mut out: Vec<SteamGame> = Vec::new();
    for lib in libraries(root) {
        let Ok(read) = std::fs::read_dir(lib.join("steamapps")) else { continue };
        for e in read.flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            let Some(text) = std::fs::read(e.path()).ok().map(|b| String::from_utf8_lossy(&b).into_owned()) else { continue };
            let Some((appid, title, install_dir, last)) = manifest(&lib, &text) else { continue };
            if out.iter().any(|g| g.appid == appid) {
                continue;
            }
            let hours = played.iter().find(|(id, _, _)| *id == appid);
            out.push(SteamGame {
                playtime_seconds: hours.map(|h| h.1).unwrap_or(0),
                last_played: hours.and_then(|h| h.2).or(last),
                appid,
                name: title,
                install_dir,
            });
        }
    }
    out.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/* ── shortcuts ─────────────────────────────────────────────────────── */

/// CRC-32 (the zip one), which Steam builds a shortcut's id from.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in bytes {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// The id Steam gives a shortcut, which also names its pictures in `grid`.
/// `exe` as it is in the file, quotes included.
pub fn shortcut_appid(exe: &str, name: &str) -> u32 {
    crc32(format!("{exe}{name}").as_bytes()) | 0x8000_0000
}

/// `steam://rungameid/` wants the long form of that id.
pub fn shortcut_gameid(appid: u32) -> u64 {
    ((appid as u64) << 32) | 0x0200_0000
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewShortcut {
    pub name: String,
    /// The program, unquoted.
    pub exe: String,
    pub start_dir: String,
    pub launch_options: String,
    /// A picture file for the icon, or empty.
    pub icon: String,
    pub tag: String,
}

impl NewShortcut {
    fn quoted(s: &str) -> String {
        format!("\"{s}\"")
    }

    pub fn appid(&self) -> u32 {
        shortcut_appid(&Self::quoted(&self.exe), &self.name)
    }

    fn entry(&self) -> Bin {
        let s = |k: &str, v: &str| (k.as_bytes().to_vec(), Bin::Str(v.as_bytes().to_vec()));
        let i = |k: &str, v: u32| (k.as_bytes().to_vec(), Bin::Int(v));
        Bin::Map(vec![
            i("appid", self.appid()),
            s("AppName", &self.name),
            s("Exe", &Self::quoted(&self.exe)),
            s("StartDir", &Self::quoted(&self.start_dir)),
            s("icon", &self.icon),
            s("ShortcutPath", ""),
            s("LaunchOptions", &self.launch_options),
            i("IsHidden", 0),
            i("AllowDesktopConfig", 1),
            i("AllowOverlay", 1),
            i("OpenVR", 0),
            i("Devkit", 0),
            s("DevkitGameID", ""),
            i("DevkitOverrideAppID", 0),
            i("LastPlayTime", 0),
            s("FlatpakAppID", ""),
            (b"tags".to_vec(), Bin::Map(vec![(b"0".to_vec(), Bin::Str(self.tag.as_bytes().to_vec()))])),
        ])
    }
}

/// `shortcuts.vdf`, held as it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcuts {
    top: Vec<(Vec<u8>, Bin)>,
}

fn field<'a>(entry: &'a Bin, key: &str) -> Option<&'a Bin> {
    match entry {
        Bin::Map(m) => m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key.as_bytes())).map(|(_, v)| v),
        _ => None,
    }
}

fn entry_appid(entry: &Bin) -> Option<u32> {
    match field(entry, "appid") {
        Some(Bin::Int(n)) => Some(*n),
        _ => None,
    }
}

impl Shortcuts {
    /// An account with no shortcuts yet.
    pub fn empty() -> Self {
        Shortcuts { top: vec![(b"shortcuts".to_vec(), Bin::Map(Vec::new()))] }
    }

    /// The file's bytes, or None when writing them back would not give the
    /// same file, or there is no `shortcuts` table: never touched then.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let top = vdf::parse_binary(bytes)?;
        if vdf::write_binary(&top) != bytes {
            return None;
        }
        let s = Shortcuts { top };
        s.list()?;
        Some(s)
    }

    fn list(&self) -> Option<&Vec<(Vec<u8>, Bin)>> {
        self.top.iter().find(|(k, _)| k.eq_ignore_ascii_case(b"shortcuts")).and_then(|(_, v)| if let Bin::Map(m) = v { Some(m) } else { None })
    }

    fn list_mut(&mut self) -> Option<&mut Vec<(Vec<u8>, Bin)>> {
        self.top.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(b"shortcuts")).and_then(|(_, v)| if let Bin::Map(m) = v { Some(m) } else { None })
    }

    pub fn len(&self) -> usize {
        self.list().map(Vec::len).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn has(&self, appid: u32) -> bool {
        self.list().is_some_and(|l| l.iter().any(|(_, e)| entry_appid(e) == Some(appid)))
    }

    /// Entries are numbered from 0 in order; a gap makes Steam drop the rest.
    fn renumber(&mut self) {
        if let Some(list) = self.list_mut() {
            for (i, (k, _)) in list.iter_mut().enumerate() {
                *k = i.to_string().into_bytes();
            }
        }
    }

    /// Add one, or replace the entry with its id (a game sent again).
    pub fn upsert(&mut self, s: &NewShortcut) {
        let id = s.appid();
        let entry = s.entry();
        if let Some(list) = self.list_mut() {
            match list.iter_mut().find(|(_, e)| entry_appid(e) == Some(id)) {
                Some((_, old)) => *old = entry,
                None => list.push((Vec::new(), entry)),
            }
        }
        self.renumber();
    }

    /// Take these ids out, leaving every other shortcut as it was. How many went.
    pub fn remove(&mut self, ids: &[u32]) -> usize {
        let before = self.len();
        if let Some(list) = self.list_mut() {
            list.retain(|(_, e)| !entry_appid(e).is_some_and(|a| ids.contains(&a)));
        }
        self.renumber();
        before - self.len()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        vdf::write_binary(&self.top)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryoto-steam-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn acf(dir: &Path, id: &str, name: &str, install: &str, flags: u32) {
        std::fs::create_dir_all(dir.join("steamapps").join("common").join(install)).unwrap();
        std::fs::write(
            dir.join("steamapps").join(format!("appmanifest_{id}.acf")),
            format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"{flags}\"\n\t\"installdir\"\t\t\"{install}\"\n\t\"LastPlayed\"\t\t\"1700000000\"\n}}\n"),
        )
        .unwrap();
    }

    fn fake_steam(name: &str) -> (PathBuf, PathBuf) {
        let root = temp(name);
        let second = root.join("D Library");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(second.join("steamapps")).unwrap();
        let path = second.to_string_lossy().replace('\\', "\\\\");
        std::fs::write(
            root.join("steamapps").join("libraryfolders.vdf"),
            format!("\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{path}\"\n\t}}\n}}\n", root.to_string_lossy().replace('\\', "\\\\")),
        )
        .unwrap();
        acf(&root, "620", "Portal 2", "Portal 2", 4);
        acf(&second, "1145360", "Hades", "Hades", 6);
        acf(&root, "228980", "Steamworks Common Redistributables", "Steamworks Shared", 4);
        acf(&second, "1493710", "Proton Experimental", "Proton - Experimental", 4);
        acf(&root, "570", "Half Downloaded", "dota 2 beta", 1026 & !4);
        let cfg = root.join("userdata").join("12345").join("config");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(
            cfg.join("localconfig.vdf"),
            "\"UserLocalConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"1145360\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1790000000\"\n\t\t\t\t\t\t\"Playtime\"\t\t\"90\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("userdata").join("777")).unwrap();
        std::fs::create_dir_all(root.join("userdata").join("0")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(
            root.join("config").join("loginusers.vdf"),
            format!("\"users\"\n{{\n\t\"{}\"\n\t{{\n\t\t\"PersonaName\"\t\t\"Mira\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}}\n}}\n", 12345 + ID64_BASE),
        )
        .unwrap();
        (root, second)
    }

    #[test]
    fn reads_every_library_and_only_games() {
        let (root, second) = fake_steam("games");
        assert_eq!(libraries(&root).len(), 2, "its own folder is listed once");
        let games = read_games(&root, Some("12345"));
        let names: Vec<_> = games.iter().map(|g| g.name.as_str()).collect();
        // Played last first; the tools and the half-downloaded game are not games.
        assert_eq!(names, ["Hades", "Portal 2"]);
        assert_eq!((games[0].playtime_seconds, games[0].last_played), (5400, Some(1_790_000_000)));
        assert_eq!(games[0].install_dir, second.join("steamapps").join("common").join("Hades"));
        // No account known: no hours, the manifest's last played.
        assert_eq!(read_games(&root, None)[0].playtime_seconds, 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_last_account_comes_first_and_folder_zero_is_nobody() {
        let (root, _) = fake_steam("users");
        let u = users(&root);
        assert_eq!(u.iter().map(|x| (x.id.as_str(), x.name.as_str(), x.recent)).collect::<Vec<_>>(), vec![("12345", "Mira", true), ("777", "777", false)]);
        let _ = std::fs::remove_dir_all(root);
    }

    fn shortcut(name: &str) -> NewShortcut {
        NewShortcut { name: name.into(), exe: r"C:\Kryoto\kryoto-desktop.exe".into(), start_dir: r"C:\Kryoto".into(), launch_options: format!("kryoto://play/{}", name.to_lowercase()), icon: String::new(), tag: "Kryoto".into() }
    }

    #[test]
    fn ids_are_the_ones_steam_makes() {
        // The known result of the published algorithm for this pair.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let id = shortcut_appid("\"C:\\Games\\a.exe\"", "A");
        assert!(id & 0x8000_0000 != 0);
        assert_eq!(shortcut_gameid(id) >> 32, id as u64);
        assert_eq!(shortcut_gameid(id) & 0xFFFF_FFFF, 0x0200_0000);
    }

    #[test]
    fn a_players_shortcuts_survive_and_ours_come_and_go() {
        // What the player has: an entry with fields Kryoto never heard of.
        let mine = Bin::Map(vec![
            (b"appid".to_vec(), Bin::Int(0x9000_0001)),
            (b"AppName".to_vec(), Bin::Str(b"Emulator".to_vec())),
            (b"sortas".to_vec(), Bin::Str(b"aaa".to_vec())),
            (b"Mystery".to_vec(), Bin::Raw(7, vec![9; 8])),
        ]);
        let file = vdf::write_binary(&[(b"shortcuts".to_vec(), Bin::Map(vec![(b"0".to_vec(), mine)]))]);
        let mut s = Shortcuts::parse(&file).unwrap();
        assert_eq!(s.to_bytes(), file);

        s.upsert(&shortcut("Hades"));
        s.upsert(&shortcut("Celeste"));
        assert_eq!(s.len(), 3);
        // Sent again: replaced, not doubled.
        s.upsert(&shortcut("Hades"));
        assert_eq!(s.len(), 3);
        let back = Shortcuts::parse(&s.to_bytes()).unwrap();
        assert!(back.has(shortcut("Hades").appid()) && back.has(0x9000_0001));

        // Undo: exactly the player's file again.
        let mut undone = back;
        assert_eq!(undone.remove(&[shortcut("Hades").appid(), shortcut("Celeste").appid()]), 2);
        assert_eq!(undone.to_bytes(), file);
    }

    #[test]
    fn taking_one_out_of_the_middle_leaves_no_gap() {
        let mut s = Shortcuts::empty();
        for n in ["A", "B", "C"] {
            s.upsert(&shortcut(n));
        }
        s.remove(&[shortcut("B").appid()]);
        let keys: Vec<_> = s.list().unwrap().iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, vec![b"0".to_vec(), b"1".to_vec()]);
    }

    #[test]
    fn a_file_it_cannot_reproduce_is_refused() {
        assert!(Shortcuts::parse(b"not a shortcuts file").is_none());
        // Parses, but has no shortcuts table.
        assert!(Shortcuts::parse(&vdf::write_binary(&[(b"other".to_vec(), Bin::Map(Vec::new()))])).is_none());
        // Cut short.
        let mut s = Shortcuts::empty();
        s.upsert(&shortcut("A"));
        let bytes = s.to_bytes();
        assert!(Shortcuts::parse(&bytes[..bytes.len() - 3]).is_none());
    }
}
