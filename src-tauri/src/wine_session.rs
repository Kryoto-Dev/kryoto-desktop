//! Shut down detached Wine services when a per-game session ends.
//!
//! A process-group kill alone misses wineserver and services that daemonize.
//! Always address the selected engine with an explicit, Kryoto-owned prefix;
//! never kill by process name or fall back to the user's default ~/.wine.

use crate::launch::{self, LaunchPlan};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

pub struct WineSession {
    prefix: PathBuf,
    server: PathBuf,
    game_root: PathBuf,
    env: Vec<(String, String)>,
    // Keep the lock through game exit AND server shutdown. A second launch
    // (even in another desktop process) must not race an old cleanup.
    _lease: File,
}

impl WineSession {
    pub fn prepare(plan: &LaunchPlan, tool: Option<&Path>, owned: &Path, game_root: &Path) -> Result<Option<Self>, String> {
        let Some(tool) = tool.filter(|tool| crate::compat::kind_of(tool) == "wine") else {
            // Proton/umu and wrapper apps own their runtime lifecycle. Their
            // prefixes may also be shared by several games outside Kryoto.
            return Ok(None);
        };
        let Some(prefix) = owned_prefix(plan, owned) else { return Ok(None) };
        let server = plan.env.iter().find(|(k, _)| k == "WINESERVER")
            .map(|(_, v)| PathBuf::from(v))
            .or_else(|| tool.parent().map(|bin| bin.join("wineserver")))
            .filter(|path| path.is_absolute() && path.is_file())
            .ok_or("Wine's matching wineserver is missing. Reinstall the selected compatibility tool before launching.")?;
        let session = Self {
            _lease: lease(&prefix)?,
            prefix,
            server,
            game_root: game_root.canonicalize().unwrap_or_else(|_| game_root.to_path_buf()),
            env: plan.env.clone(),
        };
        // Recover services left by a previous desktop crash or older client,
        // but do not interrupt a real Windows app still using this prefix.
        session.cleanup()?;
        Ok(Some(session))
    }

    /// Called while this launch is still reserved in Running, after the game
    /// and any handed-over executable have ended (including Stop and crashes).
    pub fn cleanup(&self) -> Result<(), String> {
        if self.in_use()? {
            return Err("This game's Wine prefix still has a Windows app running. Close it before launching again; its processes were left alone.".into());
        }
        self.shutdown(SHUTDOWN_TIMEOUT)
    }

    fn command(&self, arg: &str) -> Command {
        let mut cmd = Command::new(&self.server);
        launch::clean_appimage_env(&mut cmd);
        // Match the game environment, including the Template native libraries,
        // without going through arch or a shell (which can strip DYLD_*).
        if cfg!(target_os = "macos") && !self.env.iter().any(|(key, _)| key == "DYLD_LIBRARY_PATH") {
            cmd.env_remove("DYLD_LIBRARY_PATH");
        }
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)))
            .env("WINEPREFIX", &self.prefix)
            .arg(arg)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd
    }

    fn shutdown(&self, timeout: Duration) -> Result<(), String> {
        // -k sends the prefix-specific shutdown request; -w is essential:
        // spawning another Wine before the old server exits can reuse it.
        let deadline = Instant::now() + timeout;
        let killed = run_bounded(&mut self.command("-k"), deadline)?;
        // Wine returns 1 for -k when no server owns this prefix. Do not turn a
        // fresh/already-clean prefix into an error. There is no -w to issue in
        // that case: Wine can start a new server just to service the wait.
        if killed.code() == Some(1) {
            return Ok(());
        }
        if !killed.success() {
            return Err(format!("Wine cleanup failed ({killed})."));
        }
        wait_bounded(&mut self.command("-w"), deadline)
    }

    fn in_use(&self) -> Result<bool, String> {
        let mut sys = System::new_with_specifics(RefreshKind::nothing().with_processes(
            ProcessRefreshKind::nothing().with_exe(UpdateKind::Always).with_cmd(UpdateKind::Always),
        ));
        let candidates: Vec<_> = sys.processes().iter()
            .filter(|(_, p)| p.status() != sysinfo::ProcessStatus::Zombie && p.thread_kind().is_none())
            .filter(|(_, p)| wine_process(p.exe(), &p.name().to_string_lossy()))
            .map(|(pid, _)| *pid).collect();
        if candidates.is_empty() { return Ok(false); }
        // Only inspect Wine clients' environments, and only use WINEPREFIX.
        // Never log or retain their environment (which can contain secrets).
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&candidates), true,
            ProcessRefreshKind::nothing().with_environ(UpdateKind::Always));
        for pid in candidates {
            let Some(p) = sys.process(pid) else { continue };
            // Wine processes do not reliably retain WINEPREFIX in their
            // environment on macOS. The executable/cmdline path is stable,
            // so protect a live process from this game's install before using
            // the prefix-scoped server shutdown below.
            if process_in_game(p.exe(), p.cmd(), &self.game_root) {
                return Ok(true);
            }
            let prefix = p.environ().iter().find_map(|item| {
                item.to_str()?.strip_prefix("WINEPREFIX=").map(PathBuf::from)
            });
            if prefix.as_deref().and_then(|p| p.canonicalize().ok()).as_ref() != Some(&self.prefix) {
                continue;
            }
            let name = p.name().to_string_lossy();
            if name == "wineserver" || p.exe().and_then(Path::file_name).is_some_and(|n| n == "wineserver") {
                continue;
            }
            if !background_client(p.cmd()) { return Ok(true); }
        }
        Ok(false)
    }
}

fn owned_prefix(plan: &LaunchPlan, owned: &Path) -> Option<PathBuf> {
    if plan.program == plan.exe || !owned.is_absolute() { return None; }
    let value = plan.env.iter().find(|(k, _)| k == "WINEPREFIX")?.1.as_str();
    let prefix = Path::new(value);
    if !prefix.is_absolute() || std::fs::symlink_metadata(owned).ok()?.file_type().is_symlink() {
        return None;
    }
    let root = owned.canonicalize().ok()?;
    let actual = prefix.canonicalize().ok()?;
    // An explicit shared/custom prefix, including a symlink out of pfx, is not
    // ours to sweep. Preserve an existing Proton-style pfx within this game.
    (actual == root || actual == root.join("pfx")).then_some(actual)
}

fn lease(prefix: &Path) -> Result<File, String> {
    let file = OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(prefix.join(".kryoto-wine.lock"))
        .map_err(|e| format!("Could not reserve the game's Wine prefix: {e}"))?;
    // SAFETY: flock only uses a live file descriptor. File's drop releases the
    // lock, and CLOEXEC prevents Wine children from inheriting this ownership.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("This game's Wine prefix is in use or still being cleaned up. Try again after it closes.".into());
    }
    // Do not unlink this file: another opener must lock the same inode.
    Ok(file)
}

fn wait_bounded(cmd: &mut Command, deadline: Instant) -> Result<(), String> {
    let status = run_bounded(cmd, deadline)?;
    if status.success() { Ok(()) } else { Err(format!("Wine cleanup failed ({status}).")) }
}

fn run_bounded(cmd: &mut Command, deadline: Instant) -> Result<std::process::ExitStatus, String> {
    let mut child = cmd.spawn().map_err(|e| format!("Could not start Wine cleanup: {e}"))?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            result => {
                // Reap the helper on errors/timeouts too. Never leave a second
                // stuck wineserver -w around while the player retries.
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(e) => format!("Could not wait for Wine cleanup: {e}"),
                    _ => "Wine cleanup timed out. Try again after Wine closes.".into(),
                });
            }
        }
    }
}

fn wine_process(exe: Option<&Path>, name: &str) -> bool {
    let is_wine = |name: &str| {
        let name = name.to_ascii_lowercase();
        matches!(name.as_str(), "wine" | "wine64" | "wine-preloader" | "wine64-preloader" | "wineserver") || name.ends_with(".exe")
    };
    is_wine(name) || exe.and_then(Path::file_name).and_then(|n| n.to_str()).is_some_and(is_wine)
}

fn process_in_game(exe: Option<&Path>, args: &[std::ffi::OsString], root: &Path) -> bool {
    let root = normalise_path(root);
    let matches = |candidate: &Path| {
        let candidate = normalise_path(candidate);
        candidate == root || candidate.starts_with(&format!("{root}/"))
    };
    exe.is_some_and(matches)
        || args.iter().any(|arg| {
            let text = arg.to_string_lossy();
            let value = text.trim_matches(['"', '\'']);
            matches(Path::new(value))
        })
}

fn normalise_path(path: &Path) -> String {
    let path = path
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    if cfg!(windows) { path.to_ascii_lowercase() } else { path }
}

fn background_client(args: &[std::ffi::OsString]) -> bool {
    // Only Wine's known system services. An unknown executable is an app,
    // never evidence that a prefix is idle. Require a system path as well as
    // a name so a game named explorer.exe is not mistaken for a service.
    args.iter().find_map(|arg| {
        let path = arg.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        let exe = path.rsplit('/').next()?;
        if !exe.ends_with(".exe") { return None; }
        Some((path.starts_with("c:/windows/system32/") || path.starts_with("c:/windows/syswow64/"))
            && matches!(exe, "services.exe" | "explorer.exe" | "rpcss.exe" | "winedevice.exe" | "plugplay.exe" | "svchost.exe" | "conhost.exe"))
    }).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{fs::{symlink, PermissionsExt}, process::CommandExt};

    struct Fixture {
        root: PathBuf,
        prefix: PathBuf,
        plan: LaunchPlan,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("kryoto-wine-session-{}-{}", std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
            let prefix = root.join("prefix with spaces");
            std::fs::create_dir_all(&prefix).unwrap();
            std::fs::create_dir_all(root.join("bin")).unwrap();
            let plan = LaunchPlan {
                program: root.join("bin/wine"), exe: root.join("game.exe"), cwd: root.clone(),
                lead_args: Vec::new(), game_args: String::new(),
                env: vec![("WINEPREFIX".into(), prefix.to_string_lossy().into_owned()),
                    ("DYLD_FALLBACK_LIBRARY_PATH".into(), "/test/template/Frameworks".into())],
            };
            let fixture = Self { root, prefix, plan };
            fixture.script("printf '%s\\n' \"$1\" >> \"$WINEPREFIX/calls\"\nif [ \"$1\" = -k ] && [ -f \"$WINEPREFIX/daemon.pid\" ]; then read -r pid < \"$WINEPREFIX/daemon.pid\"; kill \"$pid\" 2>/dev/null || true; fi\nif [ \"$1\" = -w ]; then sleep 0.1; fi\n");
            fixture
        }
        fn script(&self, body: &str) {
            let server = self.root.join("bin/wineserver");
            std::fs::write(&server, format!("#!/bin/sh\n{body}")).unwrap();
            std::fs::set_permissions(server, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        fn prepare(&self) -> Result<WineSession, String> {
            WineSession::prepare(&self.plan, Some(&self.plan.program), &self.prefix, &self.root).map(|s| s.unwrap())
        }
        fn calls(&self) -> String {
            std::fs::read_to_string(self.prefix.join("calls")).unwrap_or_default()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
    }

    struct Child(std::process::Child);
    impl Child {
        fn daemon(prefix: &Path) -> Self {
            let child = Command::new("/bin/sleep").arg("30").process_group(0).spawn().unwrap();
            std::fs::write(prefix.join("daemon.pid"), child.id().to_string()).unwrap();
            Self(child)
        }
        fn wait_ended(&mut self) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while self.0.try_wait().unwrap().is_none() {
                assert!(Instant::now() < deadline, "prefix daemon survived cleanup");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    impl Drop for Child {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }

    #[test]
    fn normal_exit_and_stop_clean_detached_services_before_relaunch() {
        let a = Fixture::new();
        let b = Fixture::new();
        let mut other = Child::daemon(&b.prefix);
        // Also recover leftovers from an older client before the first launch.
        let mut stale = Child::daemon(&a.prefix);
        let wine = a.prepare().unwrap();
        stale.wait_ended();
        assert_eq!(a.calls(), "-k\n-w\n");
        let env: std::collections::HashMap<_, _> = wine.command("-k").get_envs()
            .map(|(k, v)| (k.to_os_string(), v.map(|v| v.to_os_string()))).collect();
        assert_eq!(env[std::ffi::OsStr::new("WINEPREFIX")], Some(wine.prefix.clone().into_os_string()));
        assert_eq!(env[std::ffi::OsStr::new("DYLD_FALLBACK_LIBRARY_PATH")], Some("/test/template/Frameworks".into()));
        drop(wine);

        for stopped in [false, true] {
            let wine = a.prepare().unwrap();
            let session = crate::handoff::Session::default();
            let mut cmd = Command::new(if stopped { "/bin/sleep" } else { "/usr/bin/true" });
            if stopped { cmd.arg("30"); }
            cmd.process_group(0);
            let mut game = session.spawn(&mut cmd).unwrap();
            let mut daemon = Child::daemon(&a.prefix); // not in the game's group
            if stopped { session.stop().unwrap(); }
            game.wait().unwrap();
            assert!(daemon.0.try_wait().unwrap().is_none(), "stand-in service must outlive the game");
            assert!(a.prepare().is_err(), "relaunch raced the exiting session");
            wine.cleanup().unwrap();
            daemon.wait_ended();
            assert!(other.0.try_wait().unwrap().is_none(), "another game's prefix was stopped");
            drop(wine);
        }
        let again = a.prepare().unwrap();
        again.cleanup().unwrap();
        assert!(other.0.try_wait().unwrap().is_none());
    }

    #[test]
    fn cleanup_never_targets_missing_relative_shared_or_symlinked_prefixes() {
        let mut f = Fixture::new();
        assert_eq!(owned_prefix(&f.plan, &f.prefix), f.prefix.canonicalize().ok());
        std::fs::create_dir(f.prefix.join("pfx")).unwrap();
        f.plan.env[0].1 = f.prefix.join("pfx").to_string_lossy().into_owned();
        assert_eq!(owned_prefix(&f.plan, &f.prefix), f.prefix.join("pfx").canonicalize().ok());
        for value in ["".to_string(), "relative".to_string(), f.root.to_string_lossy().into_owned()] {
            f.plan.env[0].1 = value;
            assert!(owned_prefix(&f.plan, &f.prefix).is_none());
        }
        f.plan.env.clear();
        assert!(owned_prefix(&f.plan, &f.prefix).is_none());
        std::fs::remove_dir(f.prefix.join("pfx")).unwrap();
        symlink(&f.root, f.prefix.join("pfx")).unwrap();
        f.plan.env = vec![("WINEPREFIX".into(), f.prefix.join("pfx").to_string_lossy().into_owned())];
        assert!(owned_prefix(&f.plan, &f.prefix).is_none());
        assert!(WineSession::prepare(&f.plan, Some(Path::new("/tool/proton")), &f.prefix, &f.root).unwrap().is_none());
        assert!(WineSession::prepare(&f.plan, Some(Path::new("/tool/umu-run")), &f.prefix, &f.root).unwrap().is_none());
        f.plan.program = f.plan.exe.clone();
        assert!(owned_prefix(&f.plan, &f.prefix).is_none());
        assert!(f.calls().is_empty());
    }

    #[test]
    fn cleanup_failure_and_timeout_are_bounded_and_release_the_reservation() {
        let f = Fixture::new();
        f.script("exit 7\n");
        assert!(f.prepare().err().unwrap().contains("cleanup failed"));
        f.script("if [ \"$1\" = -k ]; then exit 1; fi\nexit 0\n");
        let wine = f.prepare().unwrap();
        f.script("printf '%s\\n' \"$$\" > \"$WINEPREFIX/helper.pid\"\nexec sleep 30\n");
        let started = Instant::now();
        assert!(wine.shutdown(Duration::from_millis(150)).unwrap_err().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(3));
        let pid: u32 = std::fs::read_to_string(f.prefix.join("helper.pid")).unwrap().trim().parse().unwrap();
        assert!(!crate::handoff::alive(pid), "timed-out cleanup helper was not reaped");
        drop(wine);
        f.script("exit 0\n");
        assert!(f.prepare().is_ok());
    }

    #[test]
    fn only_known_wine_system_services_are_treated_as_idle() {
        let args = |s: &str| vec![std::ffi::OsString::from(s)];
        assert!(background_client(&args(r"C:\windows\system32\services.exe")));
        assert!(background_client(&args(r"C:\windows\system32\winedevice.exe")));
        assert!(!background_client(&args(r"C:\games\explorer.exe")));
        assert!(!background_client(&args(r"C:\windows\system32\notepad.exe")));
        assert!(!background_client(&args("wine")));
        assert!(!background_client(&args("game.exe")));
        assert!(wine_process(Some(Path::new("/engine/wine64-preloader")), "game.exe"));
        assert!(!wine_process(Some(Path::new("/usr/bin/sleep")), "sleep"));
    }

    #[test]
    fn a_live_client_blocks_cleanup_but_a_different_prefix_does_not() {
        let f = Fixture::new();
        let other = Fixture::new();
        // A native stand-in named wine lets the real process scan exercise
        // prefix matching without downloading or depending on Wine in CI.
        // Copy our ad-hoc-signed helper, not an Apple platform binary (macOS
        // kills relocated platform binaries such as /bin/sleep).
        std::fs::copy(std::env::current_exe().unwrap(), &f.plan.program).unwrap();
        std::fs::copy(std::env::current_exe().unwrap(), &f.plan.exe).unwrap();
        let mut game = Child(Command::new(&f.plan.exe)
            .args(["--exact", "handoff::tests::child_process", "--ignored"])
            .env("KRYOTO_HANDOFF_TEST_SECONDS", "30").env("WINEPREFIX", &f.prefix)
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
        let probe = WineSession {
            _lease: lease(&f.prefix).unwrap(), prefix: f.prefix.canonicalize().unwrap(),
            server: f.root.join("bin/wineserver"), game_root: f.root.clone(), env: f.plan.env.clone(),
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        while !probe.in_use().unwrap() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(probe.cleanup().unwrap_err().contains("Windows app running"));
        assert!(f.calls().is_empty(), "a busy prefix received a kill request");
        assert!(game.0.try_wait().unwrap().is_none());
        other.prepare().unwrap().cleanup().unwrap();
        let status = game.0.try_wait().unwrap();
        assert!(status.is_none(), "client unexpectedly ended: {status:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "real Sikarugir smoke test; set KRYOTO_SIKARUGIR_WINE to the managed bin/wine"]
    fn real_engine_exit_cleanup_relaunch_and_other_prefix_survives() {
        struct CleanOnDrop(WineSession);
        impl std::ops::Deref for CleanOnDrop {
            type Target = WineSession;
            fn deref(&self) -> &Self::Target { &self.0 }
        }
        impl Drop for CleanOnDrop {
            fn drop(&mut self) { let _ = self.0.shutdown(SHUTDOWN_TIMEOUT); }
        }
        let tool = PathBuf::from(std::env::var_os("KRYOTO_SIKARUGIR_WINE").expect("set KRYOTO_SIKARUGIR_WINE"));
        assert!(crate::compat::is_sikarugir_engine(&tool));
        let configure = |f: &mut Fixture| {
            f.plan.program = tool.clone();
            f.plan.env = crate::compat::sikarugir_engine_environment(&tool, None).unwrap();
            f.plan.env.push(("WINEPREFIX".into(), f.prefix.to_string_lossy().into_owned()));
            f.plan.env.push(("WINEDEBUG".into(), "-all".into()));
            f.plan.lead_args = vec!["cmd.exe".into(), "/c".into(), "exit 0".into()];
        };
        let mut a = Fixture::new();
        let mut b = Fixture::new();
        configure(&mut a);
        configure(&mut b);
        let wine_b = CleanOnDrop(b.prepare().unwrap());
        let boot = |f: &Fixture| {
            let mut plan = f.plan.clone();
            plan.lead_args = vec!["wineboot.exe".into(), "-u".into()];
            wait_bounded(&mut launch::command(&plan), Instant::now() + Duration::from_secs(120)).unwrap();
        };
        boot(&b);
        // Force a persistent server: normal Windows clients ending alone will
        // not release it. This reproduces detached, otherwise-idle Wine.
        wine_b.cleanup().unwrap();
        wait_bounded(&mut wine_b.command("-p"), Instant::now() + Duration::from_secs(10)).unwrap();
        for round in 0..2 {
            let wine = CleanOnDrop(a.prepare().unwrap());
            if round == 0 { boot(&a); wine.cleanup().unwrap(); }
            wait_bounded(&mut wine.command("-p"), Instant::now() + Duration::from_secs(10)).unwrap();
            let session = crate::handoff::Session::default();
            let mut game = Child(session.spawn(&mut launch::command(&a.plan)).unwrap());
            game.wait_ended();
            assert!(game.0.try_wait().unwrap().unwrap().success());
            let mut waiter = Child(wine.command("-w").spawn().unwrap());
            std::thread::sleep(Duration::from_millis(150));
            assert!(waiter.0.try_wait().unwrap().is_none(), "expected an orphan persistent wineserver");
            wine.cleanup().unwrap();
            waiter.wait_ended();
            let mut other = b.plan.clone();
            other.lead_args = vec!["cmd.exe".into(), "/c".into(), "exit".into(), "0".into()];
            assert!(launch::command(&other).status().unwrap().success(), "other prefix was shut down");
            println!("round {}: game exited, prefix shut down, other prefix retained", round + 1);
        }
        wine_b.cleanup().unwrap();
    }
}
