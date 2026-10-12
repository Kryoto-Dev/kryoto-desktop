//! Steam lobbies for game invites (FRIENDS-AND-CHAT.md 16.1, tier 2).
//!
//! Kryoto Online games run through the real Steam client as Spacewar, so
//! Steam's own lobbies work; what an invite needs is which lobby the host is
//! in. Kryoto Online's core writes that, per game process, to
//!
//!   %LOCALAPPDATA%\Kryoto\online\<pid>.json
//!   { "pid": 1234, "lobby": "1097...", "host": "7656...", "ogAppId": "1966720", "updatedAt": 1759500000 }
//!
//! (inside the Wine prefix on Linux). This module reads it for a running
//! game, and joins a lobby from an invite: `+connect_lobby <id>` on the
//! command line of a game that is starting, or `steam://joinlobby/480/<id>/<host>`
//! for one that is already running (Steam hands the game a join request).
//!
//! Lobby and Steam ids are plain decimal numbers and nothing else gets
//! through: they end up on a command line and in a URL.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, State};

use crate::library::Running;

/// The app id every Kryoto Online game runs as.
const SPACEWAR: &str = "480";
/// Kryoto Online rewrites its file every 30 seconds while the game is in a
/// lobby (include/lobby_watch.h there); one older than this is left over.
const FRESH_SECS: u64 = 120;

pub fn is_steam_id(s: &str) -> bool {
    (1..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// The argument that makes a starting game join a lobby.
pub fn connect_arg(lobby: &str) -> Option<String> {
    is_steam_id(lobby).then(|| format!("+connect_lobby {lobby}"))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LobbyFile {
    pid: u32,
    #[serde(default)]
    lobby: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    updated_at: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Lobby {
    pub lobby: String,
    pub host_steam_id: String,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A folder Kryoto Online may have written lobby files to, and whether the
/// pids in it are this machine's. Under Wine they are Wine's own numbers,
/// which mean nothing to the host, so there only freshness counts.
struct LobbyDir {
    path: PathBuf,
    host_pids: bool,
}

/// Folders Kryoto Online may have written lobby files to.
fn lobby_dirs<R: Runtime>(app: &AppHandle<R>, game_id: &str) -> Vec<LobbyDir> {
    let mut dirs = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(LobbyDir { path: PathBuf::from(local).join("Kryoto").join("online"), host_pids: true });
    }
    // Wine/Proton: the game's own prefix, any user inside it.
    if let Ok(data) = app.path().app_data_dir() {
        let prefix = data.join("prefixes").join(game_id);
        for drive in [prefix.join("pfx").join("drive_c"), prefix.join("drive_c")] {
            if let Ok(users) = std::fs::read_dir(drive.join("users")) {
                for u in users.flatten() {
                    let path = u.path().join("AppData").join("Local").join("Kryoto").join("online");
                    dirs.push(LobbyDir { path, host_pids: false });
                }
            }
        }
    }
    dirs
}

/// The freshest lobby among files whose process is alive, preferring `pid`.
fn pick(dirs: &[LobbyDir], pid: Option<u32>, alive: impl Fn(u32) -> bool) -> Option<Lobby> {
    let mut best: Option<(bool, u64, Lobby)> = None;
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir.path) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(f) = read(&path) else { continue };
            if !is_steam_id(&f.lobby) || (!f.host.is_empty() && !is_steam_id(&f.host)) {
                continue;
            }
            if now().saturating_sub(f.updated_at) > FRESH_SECS || (dir.host_pids && !alive(f.pid)) {
                continue;
            }
            let exact = dir.host_pids && Some(f.pid) == pid;
            let lobby = Lobby { lobby: f.lobby, host_steam_id: f.host };
            let better = match &best {
                None => true,
                Some((e, at, _)) => (exact, f.updated_at) > (*e, *at),
            };
            if better {
                best = Some((exact, f.updated_at, lobby));
            }
        }
    }
    best.map(|(_, _, l)| l)
}

fn read(path: &Path) -> Option<LobbyFile> {
    let text = std::fs::read_to_string(path).ok()?;
    if text.len() > 4096 {
        return None;
    }
    serde_json::from_str(&text).ok()
}

fn process_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true, ProcessRefreshKind::nothing());
    sys.process(Pid::from_u32(pid)).is_some()
}

/// The Steam lobby a running game of ours is in, if Kryoto Online reported one.
#[tauri::command(async)]
pub fn online_lobby(app: AppHandle, running: State<'_, Running>, id: String) -> Option<Lobby> {
    let pid = running.0.lock().ok()?.get(&id)?.pid()?;
    pick(&lobby_dirs(&app, &id), Some(pid), process_alive)
}

/// Join a lobby with a game that is already running: Steam passes the game a
/// join request for it.
#[tauri::command]
pub fn steam_join_lobby(lobby: String, host: String) -> Result<(), String> {
    if !is_steam_id(&lobby) || !is_steam_id(&host) {
        return Err("That invite does not name a lobby.".into());
    }
    let url = format!("steam://joinlobby/{SPACEWAR}/{lobby}/{host}");
    #[cfg(windows)]
    let result = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", &url]).creation_flags(0x0800_0000).spawn()
    };
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(&url).spawn();
    result.map(|_| ()).map_err(|e| format!("Could not open Steam: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_numbers_reach_the_command_line() {
        assert_eq!(connect_arg("109775241075364289").as_deref(), Some("+connect_lobby 109775241075364289"));
        for bad in ["", "12 34", "1;calc", "-1", "123456789012345678901", "0x10"] {
            assert_eq!(connect_arg(bad), None, "{bad}");
        }
        assert!(steam_join_lobby("1".into(), "a".into()).is_err());
    }

    #[test]
    fn picks_the_live_fresh_lobby_and_prefers_the_game_process() {
        let dir = std::env::temp_dir().join(format!("kryoto-lobbies-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let t = now();
        let write = |name: &str, body: String| std::fs::write(dir.join(name), body).unwrap();
        write("10.json", format!(r#"{{"pid":10,"lobby":"111","host":"7","updatedAt":{t}}}"#));
        write("20.json", format!(r#"{{"pid":20,"lobby":"222","host":"7","updatedAt":{}}}"#, t - 5));
        write("30.json", format!(r#"{{"pid":30,"lobby":"333","host":"7","updatedAt":{}}}"#, t - FRESH_SECS - 10));
        write("40.json", format!(r#"{{"pid":40,"lobby":"4; rm","host":"7","updatedAt":{t}}}"#));
        write("50.json", format!(r#"{{"pid":50,"lobby":"555","host":"7","updatedAt":{t}}}"#));
        let alive = |pid: u32| pid != 50;
        let dirs = vec![LobbyDir { path: dir.clone(), host_pids: true }];
        assert_eq!(pick(&dirs, Some(20), alive).map(|l| l.lobby).as_deref(), Some("222"), "the game's own process first");
        assert_eq!(pick(&dirs, Some(99), alive).map(|l| l.lobby).as_deref(), Some("111"), "else the newest live one");
        assert_eq!(pick(&dirs, None, |p| p == 30 || p == 40), None, "stale and unsafe entries are skipped");
        // Wine's pids are not the host's: a fresh file counts even though no
        // host process has its number.
        let wine = vec![LobbyDir { path: dir.clone(), host_pids: false }];
        let got = pick(&wine, Some(20), |_| false).map(|l| l.lobby);
        assert!(matches!(got.as_deref(), Some("111" | "555")), "a newest fresh one, whatever its pid: {got:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
