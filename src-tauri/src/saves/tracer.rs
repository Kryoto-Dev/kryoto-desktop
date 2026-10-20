//! Watching where a game writes while it runs.
//!
//! No administrator rights, driver or injection: while the game runs, the
//! user's folders, Public Documents, ProgramData and the game's own folder are
//! watched for writes (ReadDirectoryChangesW, through `notify`), the game's
//! processes are asked every few seconds which files they hold open, and the
//! registry keys under `HKCU\Software` are compared before and after. When it
//! closes (and a little after, as many games write their save on the way
//! out) those folders are walked once for anything written since it started,
//! which catches whatever the watchers missed. `kryoto_saves::trace` then
//! picks the game's own folders out of it all.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use kryoto_saves::trace::Observation;
use kryoto_saves::SaveRoots;
use tauri::{AppHandle, Runtime};

use crate::library::LibraryGame;

/// At most this many paths are remembered from the watchers.
const MAX_EVENTS: usize = 100_000;
/// The walk after the game closes stops after this long or this many entries.
const WALK_BUDGET: Duration = Duration::from_secs(8);
const WALK_ENTRIES: usize = 400_000;

pub struct Tracer {
    started: SystemTime,
    stop: Arc<AtomicBool>,
    events: Arc<Mutex<HashSet<PathBuf>>>,
    open: Arc<Mutex<HashSet<PathBuf>>>,
    #[cfg(windows)]
    registry_before: HashMap<String, u64>,
    _watchers: Vec<notify::RecommendedWatcher>,
    pub roots: SaveRoots,
    ignore: Vec<PathBuf>,
}

fn watch_dirs(roots: &SaveRoots) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for d in [&roots.home, &roots.documents, &roots.saved_games, &roots.public_documents, &roots.program_data, &roots.game_dir].into_iter().flatten() {
        if d.is_dir() && !dirs.iter().any(|w| d.starts_with(w)) {
            dirs.retain(|w| !w.starts_with(d));
            dirs.push(d.clone());
        }
    }
    dirs
}

/// Start watching for one game session. None when detection is off.
pub fn start<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, session: Arc<crate::handoff::Session>) -> Option<Tracer> {
    if !super::enabled(app) || !crate::settings::load(app).cloud_saves_detect || super::account(app).perk == Some(false) {
        return None;
    }
    let dir = PathBuf::from(&game.install_dir);
    #[cfg(not(windows))]
    let _ = (&session, &dir);
    let pids = move || -> Vec<u32> {
        #[cfg(windows)]
        return game_pids(&session, &dir);
        #[cfg(not(windows))]
        Vec::new()
    };
    Some(watch(super::roots(app, game), super::ignore_dirs(app, game), pids))
}

/// Watch these roots until [Tracer::finish]; pids names the game's
/// processes each time the open files are read.
pub fn watch(roots: SaveRoots, ignore: Vec<PathBuf>, pids: impl Fn() -> Vec<u32> + Send + 'static) -> Tracer {
    #[cfg(windows)]
    let registry_before = kryoto_saves::registry::key_times(3);
    let started = SystemTime::now();
    let stop = Arc::new(AtomicBool::new(false));
    let events: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
    let open: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));

    let mut watchers = Vec::new();
    for dir in watch_dirs(&roots) {
        let sink = events.clone();
        let handler = move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if !matches!(event.kind, notify::EventKind::Create(_) | notify::EventKind::Modify(_)) {
                return;
            }
            if let Ok(mut set) = sink.lock() {
                for p in event.paths {
                    if set.len() < MAX_EVENTS {
                        set.insert(p);
                    }
                }
            }
        };
        match notify::recommended_watcher(handler) {
            Ok(mut w) => {
                use notify::Watcher;
                if w.watch(&dir, notify::RecursiveMode::Recursive).is_ok() {
                    watchers.push(w);
                }
            }
            Err(e) => crate::logging::warn("saves", &format!("cannot watch {}: {e}", dir.display())),
        }
    }

    // The files the game's processes hold open, every few seconds.
    #[cfg(windows)]
    {
        let (stop, open) = (stop.clone(), open.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let found = super::win::open_files(&pids());
                if let Ok(mut set) = open.lock() {
                    set.extend(found);
                }
                for _ in 0..15 {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        });
    }
    #[cfg(not(windows))]
    let _ = pids;

    Tracer {
        started,
        stop,
        events,
        open,
        #[cfg(windows)]
        registry_before,
        _watchers: watchers,
        ignore,
        roots,
    }
}

/// The game's processes: the one Play started (or followed) and any running
/// out of its folder.
#[cfg(windows)]
fn game_pids(session: &crate::handoff::Session, dir: &Path) -> Vec<u32> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
    let mut out: Vec<u32> = session.pid().into_iter().collect();
    let root = dir.to_string_lossy().replace('\\', "/").to_lowercase();
    if root.len() < 4 {
        return out;
    }
    let root = format!("{}/", root.trim_end_matches('/'));
    let mut sys = System::new_with_specifics(RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always)));
    sys.refresh_processes(ProcessesToUpdate::All, true);
    for (pid, p) in sys.processes() {
        if p.exe().is_some_and(|e| e.to_string_lossy().replace('\\', "/").to_lowercase().starts_with(&root)) && !out.contains(&pid.as_u32()) {
            out.push(pid.as_u32());
        }
    }
    out
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Tracer {
    /// Keep watching for `grace` (a game writes its save on the way out),
    /// then stop and say what was seen.
    pub fn finish(self, grace: Duration) -> Observation {
        std::thread::sleep(grace);
        self.stop.store(true, Ordering::SeqCst);
        drop(self._watchers);
        // A second of slack for coarse timestamps.
        let since = secs(self.started).saturating_sub(2);
        let mut obs = Observation::default();
        let mut files: HashSet<PathBuf> = HashSet::new();
        let events: Vec<PathBuf> = self.events.lock().map(|s| s.iter().cloned().collect()).unwrap_or_default();
        for p in events {
            if let Ok(m) = std::fs::metadata(&p) {
                if m.is_file() && m.modified().map(secs).unwrap_or(0) >= since {
                    files.insert(p);
                } else if m.is_dir() && m.created().map(secs).unwrap_or(0) >= since {
                    obs.new_dirs.insert(p);
                }
            }
        }
        walk(&self.roots, &self.ignore, since, &mut files, &mut obs.new_dirs);
        obs.files = files.into_iter().collect();
        obs.files.sort();
        obs.open_files = self.open.lock().map(|s| s.clone()).unwrap_or_default();
        #[cfg(windows)]
        {
            let after = kryoto_saves::registry::key_times(3);
            obs.registry_keys = after
                .into_iter()
                .filter(|(k, t)| *t >= since && self.registry_before.get(k).is_none_or(|b| t > b))
                .map(|(k, _)| k)
                .collect();
            obs.registry_keys.sort();
        }
        obs
    }
}

/// One pass over the save roots for files written (and folders made) since
/// `since`, skipping the busy folders no game saves in.
fn walk(roots: &SaveRoots, ignore: &[PathBuf], since: u64, files: &mut HashSet<PathBuf>, new_dirs: &mut HashSet<PathBuf>) {
    let started = Instant::now();
    let mut seen = 0usize;
    let ignored = |p: &Path| ignore.iter().any(|i| p.starts_with(i));
    let dirs = [
        (&roots.game_dir, 8),
        (&roots.documents, 7),
        (&roots.saved_games, 6),
        (&roots.app_data, 6),
        (&roots.local_low, 6),
        (&roots.local_app_data, 6),
        (&roots.public_documents, 6),
        (&roots.program_data, 4),
    ];
    for (dir, depth) in dirs {
        let Some(dir) = dir else { continue };
        let walker = walkdir::WalkDir::new(dir).max_depth(depth).follow_links(false).into_iter().filter_entry(|e| {
            !(e.file_type().is_dir() && e.depth() > 0 && (kryoto_saves::trace::walk_skip(&e.file_name().to_string_lossy()) || ignored(e.path())))
        });
        for entry in walker.filter_map(|e| e.ok()) {
            seen += 1;
            if seen > WALK_ENTRIES || started.elapsed() > WALK_BUDGET {
                return;
            }
            let Ok(m) = entry.metadata() else { continue };
            if m.is_file() {
                if m.modified().map(secs).unwrap_or(0) >= since {
                    files.insert(entry.into_path());
                }
            } else if m.is_dir() && entry.depth() > 0 && m.created().map(secs).unwrap_or(0) >= since {
                new_dirs.insert(entry.into_path());
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use kryoto_saves::rules::GameHints;
    use kryoto_saves::{Kind, Root};

    /// A real "game" (PowerShell) writes saves in four kinds of places, plus
    /// noise in a folder that is not its own, while the tracer watches the
    /// real user folders. Opt-in, it touches the real profile and registry
    /// (cleaned up after): `cargo test tracer_finds_a_real_games_saves -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn tracer_finds_a_real_games_saves() {
        let game_dir = std::env::current_exe().unwrap().parent().unwrap().join("Kryo Save Test");
        let _ = std::fs::remove_dir_all(&game_dir);
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("KryoSaveTest.exe"), b"MZ").unwrap();
        let roots = super::super::win::roots(Some(game_dir.clone()));
        let docs = roots.documents.clone().unwrap();
        let app_data = roots.app_data.clone().unwrap();
        let local = roots.local_app_data.clone().unwrap();
        // Noise: a tool that already exists and keeps writing its state.
        let noise = local.join("KryoNoiseTool");
        std::fs::create_dir_all(&noise).unwrap();
        let script = format!(
            r#"$d='{docs}\My Games\KryoSaveTest'; New-Item -ItemType Directory -Force $d | Out-Null
$f=[IO.File]::Open("$d\slot1.sav",'Create','ReadWrite','ReadWrite'); $b=[byte[]](1..64); $f.Write($b,0,64); $f.Flush()
New-Item -ItemType Directory -Force '{game}\saves' | Out-Null; Set-Content '{game}\saves\profile.dat' 'level=3'
New-Item -ItemType Directory -Force '{app}\GSE Saves\999999\remote' | Out-Null; Set-Content '{app}\GSE Saves\999999\remote\data.bin' 'x'
New-Item -Path 'HKCU:\Software\KryoTest\KryoSaveTest' -Force | Out-Null; Set-ItemProperty 'HKCU:\Software\KryoTest\KryoSaveTest' -Name gold -Value 99
Set-Content '{noise}\state.json' '{{}}'
Start-Sleep 5; $f.Close()"#,
            docs = docs.display(),
            game = game_dir.display(),
            app = app_data.display(),
            noise = noise.display()
        );
        let script_file = game_dir.join("run.ps1");
        std::fs::write(&script_file, script).unwrap();
        let pid = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let p2 = pid.clone();
        let tracer = watch(roots.clone(), Vec::new(), move || vec![*p2.lock().unwrap()]);
        std::thread::sleep(Duration::from_secs(1));
        let mut child = std::process::Command::new("powershell").args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script_file).spawn().unwrap();
        *pid.lock().unwrap() = child.id();
        child.wait().unwrap();
        let obs = tracer.finish(Duration::from_secs(2));
        println!("{} files, {} new dirs, {} open, registry {:?}", obs.files.len(), obs.new_dirs.len(), obs.open_files.len(), obs.registry_keys);
        let hints = GameHints { title: "Kryo Save Test".into(), exe_stem: "KryoSaveTest".into(), appid: Some("999999".into()), ..Default::default() };
        let found = kryoto_saves::trace::candidates(&obs, &roots, &hints, &[], &|_| false);
        for c in &found {
            println!("{:>4} {:?} {:?} {} {:?}", c.score, c.rule.kind, c.rule.root, c.rule.path, c.reasons);
        }
        // Clean up before asserting.
        let _ = std::fs::remove_dir_all(docs.join("My Games").join("KryoSaveTest"));
        let _ = std::fs::remove_dir_all(app_data.join("GSE Saves").join("999999"));
        let _ = std::fs::remove_dir_all(&noise);
        let _ = std::process::Command::new("powershell").args(["-NoProfile", "-Command", "Remove-Item -Recurse -Force HKCU:\\Software\\KryoTest"]).status();
        let _ = std::fs::remove_dir_all(&game_dir);
        let has = |root: Root, path: &str| found.iter().any(|c| c.rule.root == root && c.rule.path == path && c.corroborated());
        assert!(has(Root::Documents, "My Games/KryoSaveTest"), "Documents save not found");
        assert!(has(Root::AppData, "GSE Saves/999999"), "emulator save not found");
        assert!(has(Root::HKCU, "Software/KryoTest/KryoSaveTest"), "registry save not found");
        assert!(found.iter().any(|c| c.rule.root == Root::GameDir && c.rule.path == "saves" && c.rule.kind == Kind::Files), "game-folder save not found");
        assert!(!found.iter().any(|c| c.rule.path.contains("KryoNoiseTool")), "noise was taken for a save");
    }
}