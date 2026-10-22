//! A log per game launch, the way Heroic keeps them: what was started, on
//! what machine, with which compatibility tool and settings, then everything
//! the game (and Wine or Proton) printed, then how it ended.
//!
//! When a game does not start, this is the first thing staff ask for, and
//! on a compatibility host the Wine/Proton/wrapper output is the only clue
//! there is. One file per
//! launch under the app's logs folder (`games/<game id>/<when>.log`), the
//! last KEEP kept, readable in the game's Properties > Logs and copyable.
//!
//! Nothing here leaves the machine on its own.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

use crate::launch::LaunchPlan;
use crate::library::LibraryGame;

/// Launches kept per game.
const KEEP: usize = 10;
/// Past this the game's output is no longer written (a chatty game must not fill the disk).
const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// What Properties reads of one log, from the end.
const READ_TAIL: u64 = 2 * 1024 * 1024;

pub fn games_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let base = Some(crate::logging::logs_folder())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| app.path().app_log_dir().ok())?;
    Some(base.join("games"))
}

/// A game id is our own slug, but it names a folder: nothing that could climb out.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 160 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

struct Inner {
    file: File,
    written: u64,
    capped: bool,
}

/// One launch's log, shared by the threads draining the game's output.
#[derive(Clone)]
pub struct GameLog(Arc<Mutex<Inner>>);

fn clock() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    crate::logging::stamp(secs)
}

impl GameLog {
    fn write(&self, text: &str) {
        let Ok(mut g) = self.0.lock() else { return };
        if g.capped {
            return;
        }
        if g.written + text.len() as u64 > MAX_BYTES {
            g.capped = true;
            let _ = g.file.write_all(b"\n[Kryoto] The log reached 20 MB; the rest of the game's output is not kept.\n");
            return;
        }
        if g.file.write_all(text.as_bytes()).is_ok() {
            g.written += text.len() as u64;
        }
    }

    /// A line from Kryoto itself.
    pub fn note(&self, message: &str) {
        self.write(&format!("({}) [Kryoto]: {message}\n", clock()));
    }

    /// Drain the game's stdout and stderr into the log, each on its own thread.
    pub fn capture(&self, child: &mut Child) {
        if let Some(out) = child.stdout.take() {
            self.drain(out);
        }
        if let Some(err) = child.stderr.take() {
            self.drain(err);
        }
    }

    fn drain(&self, stream: impl Read + Send + 'static) {
        let log = self.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = Vec::new();
            // Bytes, not str: Wine writes whatever encoding the game uses.
            while reader.read_until(b'\n', &mut line).map(|n| n > 0).unwrap_or(false) {
                let mut text = String::from_utf8_lossy(&line).into_owned();
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                log.write(&text);
                line.clear();
            }
        });
    }

    /// How it ended.
    pub fn finish(&self, code: Option<i32>, seconds: u64, followed: Option<u32>) {
        if let Some(pid) = followed {
            self.note(&format!("The game handed over to process {pid}; its output is not in this log."));
        }
        let ended = match code {
            Some(c) => format!("exited with code {c}"),
            None if followed.is_some() => "ended".to_string(),
            None => "ended without an exit code (stopped, or a crash)".to_string(),
        };
        self.note(&format!("Game {ended} after {}.", duration(seconds)));
        self.write("============= End of log =============\n");
    }
}

fn duration(s: u64) -> String {
    if s >= 3600 {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// Open this launch's log and write its header. None when there is nowhere to write.
pub fn begin<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, plan: &LaunchPlan, tool: Option<&Path>) -> Option<GameLog> {
    if !safe_id(&game.id) {
        return None;
    }
    let dir = games_dir(app)?.join(&game.id);
    std::fs::create_dir_all(&dir).ok()?;
    prune(&dir);
    let name = format!("{}.log", file_stamp());
    let file = OpenOptions::new().create(true).append(true).open(dir.join(&name)).ok()?;
    let log = GameLog(Arc::new(Mutex::new(Inner { file, written: 0, capped: false })));
    log.write(&header(app, game, plan, tool));
    Some(log)
}

fn file_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // 2026-10-09 14:02:00 -> 2026-10-09_14-02-00
    crate::logging::stamp(secs).replace(' ', "_").replace(':', "-")
}

/// Keep the newest KEEP - 1, so with the new one there are KEEP.
fn prune(dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut logs: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "log")).collect();
    logs.sort();
    while logs.len() >= KEEP {
        let old = logs.remove(0);
        let _ = std::fs::remove_file(old);
    }
}

fn header<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, plan: &LaunchPlan, tool: Option<&Path>) -> String {
    let t = clock();
    let native = plan.program == plan.exe;
    let mut h = String::new();
    h.push_str(&format!("({t}) [Kryoto]: Launching \"{}\"\n", game.title));
    h.push_str(&format!("({t}) [Kryoto]: Native? {native}\n"));
    h.push_str(&format!("({t}) [Kryoto]: Installed in: {}\n", game.install_dir));
    if let Some(src) = &game.source {
        h.push_str(&format!("({t}) [Kryoto]: Source: {src}\n"));
    }
    h.push('\n');
    h.push_str(&format!("({t}) [Kryoto]: System info:\n{}\n", system_info(app)));
    h.push_str(&format!("({t}) [Kryoto]: Game settings:\n"));
    h.push_str(&format!("  executable: {}\n", plan.exe.display()));
    h.push_str(&format!("  working folder: {}\n", plan.cwd.display()));
    h.push_str(&format!("  launch options: {}\n", if game.launch_options.trim().is_empty() { "(none)" } else { game.launch_options.trim() }));
    h.push_str(&format!("  release overrides: {}\n", game.apply_overrides));
    if !native {
        h.push_str(&format!(
            "  compatibility tool: {}\n",
            tool.map(|p| format!("{} ({})", crate::compat::kind_of(p), p.display())).unwrap_or_else(|| "(none)".into())
        ));
        if let Some((_, prefix)) = plan.env.iter().find(|(k, _)| k == "WINEPREFIX" || k == "STEAM_COMPAT_DATA_PATH") {
            h.push_str(&format!("  prefix: {prefix}\n"));
        }
    }
    if !plan.env.is_empty() {
        h.push_str("  environment:\n");
        for (k, v) in &plan.env {
            h.push_str(&format!("    {k}={v}\n"));
        }
    }
    h.push_str(&format!("\nCommand: {}\n\nGame log:\n", plan.display()));
    h
}

fn system_info<R: Runtime>(app: &AppHandle<R>) -> String {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
    let gb = |b: u64| format!("{:.2} GB", b as f64 / 1_000_000_000.0);
    let cpu = sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_else(|| "unknown".into());
    let mut s = String::new();
    s.push_str(&format!("CPU: {}x {cpu}\n", sys.cpus().len()));
    s.push_str(&format!("Memory: {} (used: {})\n", gb(sys.total_memory()), gb(sys.used_memory())));
    s.push_str("GPUs:\n");
    let gpus = gpus();
    if gpus.is_empty() {
        s.push_str("  (could not tell)\n");
    }
    for (i, g) in gpus.iter().enumerate() {
        s.push_str(&format!("  GPU {i}: {g}\n"));
    }
    s.push_str(&format!(
        "OS: {} {} (kernel {})\n",
        System::name().unwrap_or_default(),
        System::os_version().unwrap_or_default(),
        System::kernel_version().unwrap_or_default()
    ));
    #[cfg(target_os = "linux")]
    {
        let flatpak = std::env::var_os("FLATPAK_ID").is_some();
        let appimage = std::env::var_os("APPIMAGE").is_some();
        let deck = std::fs::read_to_string("/sys/devices/virtual/dmi/id/board_vendor").is_ok_and(|v| v.trim() == "Valve");
        s.push_str(&format!("{}\n", if deck { "This is a Steam Deck" } else { "Not a Steam Deck" }));
        s.push_str(&format!("{}\n", if flatpak { "Running inside a Flatpak" } else { "Not running in a Flatpak" }));
        s.push_str(&format!("{}\n", if appimage { "Running from an AppImage" } else { "Not running from an AppImage" }));
        s.push_str(&format!(
            "Session: {}\n",
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| if std::env::var_os("WAYLAND_DISPLAY").is_some() { "wayland".into() } else { "x11".into() })
        ));
        if let Some(how) = crate::display_env::describe() {
            s.push_str(&format!("Kryoto's renderer: {how}\n"));
        }
    }
    s.push_str(&format!("Kryoto Desktop: {}\n", app.package_info().version));
    s
}

/// The graphics cards, by name where the system says it, else by vendor and ids.
fn gpus() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        let mut out = Vec::new();
        let Ok(read) = std::fs::read_dir("/sys/class/drm") else { return out };
        let mut cards: Vec<PathBuf> = read
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("card") && !n.contains('-')))
            .collect();
        cards.sort();
        for card in cards {
            let dev = card.join("device");
            let id = |f: &str| std::fs::read_to_string(dev.join(f)).map(|s| s.trim().trim_start_matches("0x").to_string()).unwrap_or_default();
            let (vendor, device) = (id("vendor"), id("device"));
            if vendor.is_empty() {
                continue;
            }
            let driver = std::fs::read_link(dev.join("driver"))
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "unknown".into());
            let name = match vendor.as_str() {
                "10de" => "NVIDIA",
                "1002" => "AMD",
                "8086" => "Intel",
                "1af4" => "virtio (a virtual machine)",
                "15ad" => "VMware (a virtual machine)",
                _ => "Unknown vendor",
            };
            out.push(format!("{name} (V={vendor} D={device}), driver {driver}"));
        }
        out
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // The display adapters' class key: each subkey has the adapter's name.
        let out = std::process::Command::new("reg")
            .args(["query", r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}", "/s", "/v", "DriverDesc"])
            .creation_flags(0x0800_0000)
            .output();
        let Ok(out) = out else { return Vec::new() };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut names: Vec<String> = text
            .lines()
            .filter_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string()))
            .filter(|v| !v.is_empty())
            .collect();
        names.dedup();
        names
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Vec::new()
    }
}

// ---- Reading them back ---------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameLogInfo {
    pub name: String,
    pub size: u64,
    /// Unix seconds.
    pub modified: u64,
}

/// A game's logs, newest first.
#[tauri::command(async)]
pub fn game_logs(app: AppHandle, id: String) -> Result<Vec<GameLogInfo>, String> {
    if !safe_id(&id) {
        return Err("Not a game.".into());
    }
    let Some(dir) = games_dir(&app).map(|d| d.join(&id)) else { return Ok(Vec::new()) };
    let Ok(read) = std::fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut out: Vec<GameLogInfo> = read
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "log"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            Some(GameLogInfo { name: e.file_name().to_string_lossy().into_owned(), size: meta.len(), modified })
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(out)
}

/// One log's text (the last READ_TAIL of it).
#[tauri::command(async)]
pub fn game_log_read(app: AppHandle, id: String, name: String) -> Result<String, String> {
    if !safe_id(&id) || !name.ends_with(".log") || name.contains(['/', '\\']) || name.contains("..") {
        return Err("Not a game log.".into());
    }
    let path = games_dir(&app).ok_or("No logs folder.")?.join(&id).join(&name);
    let mut f = File::open(&path).map_err(|_| "That log is gone.".to_string())?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut prefix = String::new();
    if len > READ_TAIL {
        use std::io::Seek;
        f.seek(std::io::SeekFrom::Start(len - READ_TAIL)).map_err(|e| e.to_string())?;
        prefix = format!("[Kryoto] Showing the last {} MB of this log. Open the folder for all of it.\n", READ_TAIL / 1024 / 1024);
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(prefix + &String::from_utf8_lossy(&buf))
}

/// The folder a game's logs are in, for "Open folder".
#[tauri::command]
pub fn game_logs_folder(app: AppHandle, id: String) -> Result<String, String> {
    if !safe_id(&id) {
        return Err("Not a game.".into());
    }
    let dir = games_dir(&app).ok_or("No logs folder.")?.join(&id);
    let _ = std::fs::create_dir_all(&dir);
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_names_cannot_climb_out() {
        assert!(safe_id("captain-hardcore"));
        assert!(!safe_id("../etc"));
        assert!(!safe_id(""));
    }

    #[test]
    fn durations_read_like_a_person_wrote_them() {
        assert_eq!(duration(5), "5s");
        assert_eq!(duration(125), "2m 05s");
        assert_eq!(duration(3700), "1h 01m");
    }
}
