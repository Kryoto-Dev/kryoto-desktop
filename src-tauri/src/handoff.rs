//! Following a game past its launcher.
//!
//! Plenty of games start, hand over to another process and exit with code 0
//! within a second or two: a launcher that starts the real exe, a game that
//! restarts itself, a stub that re-executes under a different name. The client
//! watched only the process it started, so it said "It closed again straight
//! away" while the game was running fine (the most reported error of 0.2.3,
//! over 300 times) and counted no playtime for it.
//!
//! So when the started process ends quickly, this looks for a process running
//! from the game's folder - by its executable, or under Wine and Proton by its
//! command line - and follows that one instead.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

/// A first process that lived less than this may have handed over.
pub const HANDOFF_WITHIN_SECS: u64 = 15;
/// How long to look for the process it handed over to.
const LOOK_FOR: Duration = Duration::from_secs(20);

#[derive(Clone, Copy)]
struct Target {
    pid: u32,
    group: u32,
}

#[derive(Default)]
pub struct Session {
    target: Mutex<Option<Target>>,
    stopped: AtomicBool,
    tracking: Mutex<Option<(PathBuf, HashSet<u32>)>>,
}

impl Session {
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
    pub fn pid(&self) -> Option<u32> {
        self.target.lock().ok()?.map(|t| t.pid)
    }
    pub fn track(&self, dir: &Path, before: &HashSet<u32>) {
        if let Ok(mut tracking) = self.tracking.lock() {
            *tracking = Some((dir.to_path_buf(), before.clone()));
        }
    }

    /// Stop and spawn serialize, so cancellation cannot miss a new process.
    pub fn spawn(&self, cmd: &mut std::process::Command) -> std::io::Result<std::process::Child> {
        let mut target = self.target.lock().map_err(|_| std::io::Error::other("state lock poisoned"))?;
        if self.stopped() {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Launch cancelled."));
        }
        let child = cmd.spawn()?;
        *target = Some(Target { pid: child.id(), group: child.id() });
        Ok(child)
    }

    pub fn follow(&self, pid: u32) -> Result<(), String> {
        let mut target = self.target.lock().map_err(|_| "state lock poisoned")?;
        if self.stopped() {
            // Stop may have arrived between the scan and this lock.
            terminate(Target { pid, group: target.map(|t| t.group).unwrap_or(pid) })?;
        }
        if let Some(t) = target.as_mut() {
            t.pid = pid;
        }
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        let target = self.target.lock().map_err(|_| "state lock poisoned")?;
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(mut t) = *target {
            if !alive(t.pid) {
                if let Some((dir, before)) = self.tracking.lock().map_err(|_| "state lock poisoned")?.as_ref() {
                    if let Some(next) = find_in_folder(dir, t.pid, before) {
                        t.pid = next;
                    }
                }
            }
            if let Err(e) = terminate(t) {
                self.stopped.store(false, Ordering::SeqCst);
                return Err(e);
            }
        }
        Ok(())
    }
}

fn terminate(target: Target) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("taskkill")
            .args(["/PID", &target.pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() && alive(target.pid) {
            return Err(format!("Could not stop the game: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    #[cfg(unix)]
    {
        // The followed PID need not be a process-group leader. Keep the group
        // created by Play, and also cover children that moved to another one.
        let sys = snapshot();
        let mut ids = vec![target.pid];
        loop {
            let next: Vec<_> = sys
                .processes()
                .iter()
                .filter_map(|(pid, p)| {
                    (p.parent().is_some_and(|parent| ids.contains(&parent.as_u32())) && !ids.contains(&pid.as_u32())).then_some(pid.as_u32())
                })
                .collect();
            if next.is_empty() {
                break;
            }
            ids.extend(next);
        }
        // Stop is an explicit request to end the whole session, even when a
        // hung game or Wine ignores TERM. Never use a global wineserver kill.
        if unsafe { libc::kill(-(target.group as i32), libc::SIGKILL) } != 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::ESRCH) {
                return Err(format!("Could not stop the game: {e}"));
            }
        }
        for pid in ids.into_iter().rev() {
            if unsafe { libc::kill(pid as i32, libc::SIGKILL) } != 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(libc::ESRCH) {
                    return Err(format!("Could not stop the game: {e}"));
                }
            }
        }
    }
    #[cfg(windows)]
    let _ = target.group;
    Ok(())
}

pub fn alive(pid: u32) -> bool {
    let pid = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).is_some_and(|p| p.status() != sysinfo::ProcessStatus::Zombie)
}

/// Exclude pre-existing processes in the install folder (e.g. another game
/// started manually), rather than following or stopping something unrelated.
pub fn existing() -> HashSet<u32> {
    snapshot().processes().keys().map(|p| p.as_u32()).collect()
}

fn normalise(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    if cfg!(windows) { s.to_lowercase() } else { s }
}

fn snapshot() -> System {
    System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always).with_cmd(sysinfo::UpdateKind::Always)),
    )
}

/// A process (other than `not`) running out of `dir`, if there is one.
pub fn find_in_folder(dir: &Path, not: u32, before: &HashSet<u32>) -> Option<u32> {
    let root = normalise(dir);
    if root.len() < 4 {
        // A drive root or an empty path would match everything.
        return None;
    }
    let root = format!("{}/", root.trim_end_matches('/'));
    let sys = snapshot();
    sys.processes().iter().find_map(|(pid, p)| {
        let pid = pid.as_u32();
        if pid == not || pid == std::process::id() || before.contains(&pid) {
            return None;
        }
        // Linux lists individual threads beside processes. Following a new
        // worker's TID would mistake launcher activity for the game itself.
        if p.thread_kind().is_some() || p.status() == sysinfo::ProcessStatus::Zombie {
            return None;
        }
        let by_exe = p.exe().map(|e| normalise(e).starts_with(&root)).unwrap_or(false);
        let by_cmd = || {
            p.cmd().iter().any(|a| {
                let path = PathBuf::from(a);
                normalise(&path).starts_with(&root) && path.is_file()
            })
        };
        (by_exe || by_cmd()).then_some(pid)
    })
}

/// Look for the process a quick exit handed over to, for a while.
pub fn look_for(dir: &Path, not: u32, before: &HashSet<u32>, session: &Session) -> Option<u32> {
    let until = Instant::now() + LOOK_FOR;
    while Instant::now() < until && !session.stopped() {
        if let Some(pid) = find_in_folder(dir, not, before) {
            return Some(pid);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    None
}

/// Block until `pid` is gone. Polls: a process we did not start cannot be waited on.
pub fn wait_gone(pid: u32) {
    while alive(pid) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_prevents_spawn_and_ends_handoff_search() {
        let session = Session::default();
        session.stop().unwrap();
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        assert_eq!(session.spawn(&mut cmd).unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        let started = Instant::now();
        assert!(look_for(Path::new("/games/fixture"), 1, &HashSet::new(), &session).is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    fn helper(seconds: &str) -> std::process::Command {
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "handoff::tests::child_process", "--ignored"]).env("KRYOTO_HANDOFF_TEST_SECONDS", seconds);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd
    }

    #[test]
    #[ignore = "child process for the process lifecycle test"]
    fn child_process() {
        if let Ok(seconds) = std::env::var("KRYOTO_HANDOFF_TEST_SECONDS") {
            std::thread::sleep(Duration::from_secs(seconds.parse().unwrap()));
        }
    }

    #[test]
    fn a_real_handoff_stops_the_new_process_and_preserves_existing_ones() {
        let mut unrelated = helper("30").spawn().unwrap();
        let before = existing();
        // Keep a new launcher thread alive during the scan. On Linux it has
        // the same exe and command line as this harness, but is not a game.
        let (wake, sleeping) = std::sync::mpsc::channel();
        let (ready, started) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            sleeping.recv().unwrap();
        });
        started.recv().unwrap();
        let root = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        let session = Session::default();
        session.track(&root, &before);
        let mut launcher = session.spawn(&mut helper("0")).unwrap();
        let pid = launcher.id();
        assert!(launcher.wait().unwrap().success());
        let mut game = helper("30").spawn().unwrap();
        let next = find_in_folder(&root, pid, &before);
        wake.send(()).unwrap();
        worker.join().unwrap();
        if next != Some(game.id()) {
            let _ = game.kill();
            let _ = game.wait();
            let _ = unrelated.kill();
            let _ = unrelated.wait();
            panic!("The handed-over process was not found: {next:?}");
        }
        session.follow(game.id()).unwrap();
        session.stop().unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while alive(game.id()) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
        }
        let stopped = !alive(game.id());
        let kept = alive(unrelated.id());
        let _ = game.kill();
        let _ = game.wait();
        let _ = unrelated.kill();
        let _ = unrelated.wait();
        assert!(stopped, "Stop did not end the real game process");
        assert!(kept, "Stop touched a process that predates this session");
    }
}
