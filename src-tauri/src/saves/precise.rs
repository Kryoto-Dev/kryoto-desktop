//! Precise save finding: Windows itself says which files a game wrote.
//!
//! The smart tracer (tracer.rs) sees what changed on the PC while a game ran
//! and has to work out which of it was the game. Windows knows: its kernel
//! reports every file a process creates, writes, renames or deletes through
//! Event Tracing for Windows (the `Microsoft-Windows-Kernel-File` provider),
//! with the process that did it. Reading those events needs administrator
//! rights, so for one session per game Kryoto Desktop starts a copy of
//! itself as administrator (one Windows prompt), with [`HELPER_ARG`]: it
//! starts the trace, follows the game's processes (and the ones they start),
//! and when told to stop writes the files they wrote to `trace.json`. No
//! driver, nothing injected into the game, and nothing kept running after.
//!
//! A player who says no to the prompt (or has no administrator) gets the
//! smart tracer, which needs none.

use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::{Duration, Instant};

/// The argument that makes the exe the elevated tracing helper, not the app.
pub const HELPER_ARG: &str = "--kryoto-save-trace";

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TraceFile {
    /// Every file the game's processes wrote (and what they created,
    /// renamed or deleted), as Windows paths.
    pub written: Vec<String>,
    /// Events Windows dropped because the helper fell behind.
    pub lost: u32,
    /// The game's processes it followed.
    pub processes: u32,
}

/// A running helper, until [`Precise::finish`].
pub struct Precise {
    dir: PathBuf,
    #[cfg(windows)]
    process: usize,
    done: bool,
}

// Off Windows `begin` only ever fails.
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Begin {
    Running(Precise),
    /// The player said no to the administrator prompt.
    Declined,
    Failed(String),
}

#[cfg(not(windows))]
pub fn begin(_dir: &Path, _game_dir: &Path) -> Begin {
    Begin::Failed("Precise save finding needs Windows.".into())
}

#[cfg(not(windows))]
impl Precise {
    pub fn finish(mut self) -> Option<TraceFile> {
        self.done = true;
        let _ = &self.dir;
        None
    }
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Start the helper as administrator (Windows asks the player), and wait
/// until it is tracing.
#[cfg(windows)]
pub fn begin(dir: &Path, game_dir: &Path) -> Begin {
    match std::env::current_exe() {
        Ok(exe) => begin_with(&exe, dir, game_dir),
        Err(e) => Begin::Failed(e.to_string()),
    }
}

/// [`begin`], with the helper being `exe` (Kryoto Desktop's own).
#[cfg(windows)]
fn begin_with(exe: &Path, dir: &Path, game_dir: &Path) -> Begin {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_CANCELLED};
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};

    let _ = std::fs::remove_dir_all(dir);
    if let Err(e) = std::fs::create_dir_all(dir) {
        return Begin::Failed(e.to_string());
    }
    let params = format!("{HELPER_ARG} \"{}\" \"{}\" {}", dir.display(), game_dir.display(), std::process::id());
    let (verb, file, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(&params));
    // SAFETY: a zeroed SHELLEXECUTEINFOW is its documented empty state; the
    // strings outlive the call.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = 0;
    // Blocks while Windows asks the player.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let e = unsafe { GetLastError() };
        return if e == ERROR_CANCELLED { Begin::Declined } else { Begin::Failed(format!("could not start the helper (error {e})")) };
    }
    if info.hProcess.is_null() {
        return Begin::Failed("the helper did not start".into());
    }
    let precise = Precise { dir: dir.to_path_buf(), process: info.hProcess as usize, done: false };
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        if dir.join("ready").is_file() {
            return Begin::Running(precise);
        }
        // Gone already: it could not trace.
        if unsafe { WaitForSingleObject(info.hProcess, 0) } == 0 {
            let why = std::fs::read_to_string(dir.join("error.txt")).unwrap_or_else(|_| "the helper stopped".into());
            return Begin::Failed(why);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Begin::Failed("the helper did not start tracing in time".into())
}

#[cfg(windows)]
impl Precise {
    /// Tell the helper to stop and read what it saw.
    pub fn finish(mut self) -> Option<TraceFile> {
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        self.done = true;
        let _ = std::fs::write(self.dir.join("stop"), b"");
        // SAFETY: our own process handle, closed in Drop.
        unsafe { WaitForSingleObject(self.process as _, 20_000) };
        if let Ok(e) = std::fs::read_to_string(self.dir.join("error.txt")) {
            crate::logging::warn("saves", &format!("precise finding: {e}"));
        }
        let trace: TraceFile = serde_json::from_slice(&std::fs::read(self.dir.join("trace.json")).ok()?).ok()?;
        Some(trace)
    }
}

impl Drop for Precise {
    fn drop(&mut self) {
        if !self.done {
            let _ = std::fs::write(self.dir.join("stop"), b"");
        }
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.process as _);
        }
    }
}

/* ── the helper (runs as administrator) ─────────────────────────────── */

/// When the exe was started as the helper: run it and say the exit code.
pub fn helper_from_args() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(HELPER_ARG) {
        return None;
    }
    #[cfg(windows)]
    {
        let (Some(dir), Some(game), Some(parent)) = (args.get(2), args.get(3), args.get(4).and_then(|p| p.parse::<u32>().ok())) else { return Some(2) };
        let dir = PathBuf::from(dir);
        // Only a real folder, not a link: an administrator writing where a
        // link points is how an elevated helper gets turned against the PC.
        let real = std::fs::canonicalize(&dir).ok().map(|p| p.to_string_lossy().trim_start_matches(r"\\?\").to_lowercase());
        if real.as_deref() != Some(dir.to_string_lossy().to_lowercase().trim_end_matches('\\')) || std::fs::symlink_metadata(&dir).map(|m| !m.is_dir()).unwrap_or(true) {
            return Some(3);
        }
        Some(match etw::run(&dir, Path::new(game), parent) {
            Ok(()) => 0,
            Err(e) => {
                let _ = std::fs::write(dir.join("error.txt"), e);
                1
            }
        })
    }
    #[cfg(not(windows))]
    Some(2)
}

#[cfg(windows)]
mod etw {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use windows_sys::core::GUID;
    use windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW;
    use windows_sys::Win32::System::Diagnostics::Etw::{
        CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW, TdhGetProperty, TdhGetPropertySize, CONTROLTRACE_HANDLE,
        EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_RECORD, EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES,
        EVENT_TRACE_REAL_TIME_MODE, PROCESSTRACE_HANDLE, PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME, PROPERTY_DATA_DESCRIPTOR,
        TRACE_LEVEL_VERBOSE, WNODE_FLAG_TRACED_GUID,
    };

    use super::{wide, TraceFile};

    const SESSION: &str = "Kryoto Save Trace";
    const SESSION_GUID: GUID = GUID::from_u128(0x6b1d_2f4e_9c3a_4e8b_a1f0_5d7c_3e9b_2a64);
    /// Microsoft-Windows-Kernel-File and Microsoft-Windows-Kernel-Process.
    const KERNEL_FILE: GUID = GUID::from_u128(0xEDD0_8927_9CC4_4E65_B970_C256_0FB5_C289);
    const KERNEL_PROCESS: GUID = GUID::from_u128(0x22FB_2CD6_0E7B_422B_A0C7_2FAD_1FD0_E716);
    /// Kernel-File keywords: Create, Write, DeletePath, RenamePath, CreateNewFile.
    const FILE_KEYWORDS: u64 = 0x80 | 0x200 | 0x400 | 0x800 | 0x1000;
    const PROCESS_KEYWORDS: u64 = 0x10;
    /// Kernel-File event ids.
    const CREATE: u16 = 12;
    const WRITE: u16 = 16;
    const DELETE_PATH: u16 = 26;
    const RENAME_PATH: u16 = 27;
    const CREATE_NEW_FILE: u16 = 30;
    /// Kernel-Process: ProcessStart.
    const PROCESS_START: u16 = 1;
    /// The CreateDisposition that only opens what is there (FILE_OPEN); any
    /// other creates, replaces or may create.
    const FILE_OPEN: u32 = 1;
    /// A session is never kept longer than this, whatever happens.
    const LONGEST: Duration = Duration::from_secs(12 * 3600);
    const MAX_WRITTEN: usize = 20_000;

    fn same(a: &GUID, b: &GUID) -> bool {
        a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
    }

    struct Names {
        file_name: Vec<u16>,
        file_path: Vec<u16>,
        file_object: Vec<u16>,
        create_options: Vec<u16>,
        process_id: Vec<u16>,
        parent_id: Vec<u16>,
        image_name: Vec<u16>,
    }

    struct State {
        game_root: String,
        devices: Vec<(String, String)>,
        names: Names,
        watched: Mutex<HashSet<u32>>,
        /// A file object the game opened, to its name, for its writes.
        objects: Mutex<HashMap<u64, String>>,
        written: Mutex<HashSet<String>>,
    }

    /// A property of an event, by name, through TDH (the event's own schema,
    /// so no layout is assumed).
    fn prop(rec: &EVENT_RECORD, name: &[u16]) -> Option<Vec<u8>> {
        let desc = PROPERTY_DATA_DESCRIPTOR { PropertyName: name.as_ptr() as u64, ArrayIndex: u32::MAX, Reserved: 0 };
        let mut size = 0u32;
        // SAFETY: the record is the one ETW handed the callback; the buffer is `size` bytes.
        if unsafe { TdhGetPropertySize(rec, 0, std::ptr::null(), 1, &desc, &mut size) } != 0 || size == 0 || size > 64 * 1024 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if unsafe { TdhGetProperty(rec, 0, std::ptr::null(), 1, &desc, size, buf.as_mut_ptr()) } != 0 {
            return None;
        }
        Some(buf)
    }

    fn prop_u64(rec: &EVENT_RECORD, name: &[u16]) -> Option<u64> {
        let b = prop(rec, name)?;
        match b.len() {
            4 => Some(u32::from_le_bytes(b[..4].try_into().ok()?) as u64),
            8 => Some(u64::from_le_bytes(b[..8].try_into().ok()?)),
            _ => None,
        }
    }

    fn prop_str(rec: &EVENT_RECORD, name: &[u16]) -> Option<String> {
        let b = prop(rec, name)?;
        let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).take_while(|&u| u != 0).collect();
        (!units.is_empty()).then(|| String::from_utf16_lossy(&units))
    }

    impl State {
        /// `\Device\HarddiskVolume3\Users\x` as `C:\Users\x`.
        fn dos(&self, nt: &str) -> Option<String> {
            self.devices.iter().find_map(|(dev, drive)| {
                let head = nt.get(..dev.len())?;
                (head.eq_ignore_ascii_case(dev) && nt[dev.len()..].starts_with('\\')).then(|| format!("{drive}{}", &nt[dev.len()..]))
            })
        }

        fn wrote(&self, nt: &str) {
            if let Some(path) = self.dos(nt.trim_end_matches('\\')) {
                if let Ok(mut w) = self.written.lock() {
                    if w.len() < MAX_WRITTEN {
                        w.insert(path);
                    }
                }
            }
        }

        fn in_game(&self, path: &str) -> bool {
            path.to_lowercase().replace('/', "\\").starts_with(&self.game_root)
        }

        fn event(&self, rec: &EVENT_RECORD) {
            let header = &rec.EventHeader;
            let id = header.EventDescriptor.Id;
            if same(&header.ProviderId, &KERNEL_PROCESS) {
                if id != PROCESS_START {
                    return;
                }
                let (Some(pid), parent) = (prop_u64(rec, &self.names.process_id), prop_u64(rec, &self.names.parent_id)) else { return };
                let image = prop_str(rec, &self.names.image_name).and_then(|i| self.dos(&i)).unwrap_or_default();
                if let Ok(mut w) = self.watched.lock() {
                    // A process the game started, or one run from its folder.
                    if parent.is_some_and(|p| w.contains(&(p as u32))) || self.in_game(&image) {
                        w.insert(pid as u32);
                    }
                }
                return;
            }
            if !same(&header.ProviderId, &KERNEL_FILE) {
                return;
            }
            if !self.watched.lock().is_ok_and(|w| w.contains(&header.ProcessId)) {
                return;
            }
            match id {
                CREATE | CREATE_NEW_FILE => {
                    let Some(name) = prop_str(rec, &self.names.file_name) else { return };
                    let disposition = prop_u64(rec, &self.names.create_options).map(|o| (o >> 24) as u32).unwrap_or(FILE_OPEN);
                    if id == CREATE_NEW_FILE || disposition != FILE_OPEN {
                        self.wrote(&name);
                    } else if let Some(object) = prop_u64(rec, &self.names.file_object) {
                        if let Ok(mut o) = self.objects.lock() {
                            if o.len() > 500_000 {
                                o.clear();
                            }
                            o.insert(object, name);
                        }
                    }
                }
                WRITE => {
                    let Some(object) = prop_u64(rec, &self.names.file_object) else { return };
                    let name = self.objects.lock().ok().and_then(|o| o.get(&object).cloned());
                    if let Some(name) = name {
                        self.wrote(&name);
                    }
                }
                DELETE_PATH | RENAME_PATH => {
                    if let Some(name) = prop_str(rec, &self.names.file_path) {
                        self.wrote(&name);
                    }
                }
                _ => {}
            }
        }
    }

    unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
        // SAFETY: ETW passes a valid record; UserContext is the State that
        // `run` keeps alive until ProcessTrace has returned.
        let Some(rec) = (unsafe { rec.as_ref() }) else { return };
        if let Some(state) = unsafe { (rec.UserContext as *const State).as_ref() } {
            state.event(rec);
        }
    }

    /// `C:` -> `\Device\HarddiskVolume3`, for every drive letter.
    fn devices() -> Vec<(String, String)> {
        let mut out = Vec::new();
        for letter in b'A'..=b'Z' {
            let drive = format!("{}:", letter as char);
            let mut buf = vec![0u16; 1024];
            // SAFETY: buffer of the length passed.
            let n = unsafe { QueryDosDeviceW(wide(&drive).as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
            if n == 0 {
                continue;
            }
            let target: Vec<u16> = buf[..n].iter().copied().take_while(|&u| u != 0).collect();
            if !target.is_empty() {
                out.push((String::from_utf16_lossy(&target), drive));
            }
        }
        // Longest first: \Device\HarddiskVolume10 before \Device\HarddiskVolume1.
        out.sort_by_key(|(d, _)| std::cmp::Reverse(d.len()));
        out
    }

    /// The processes running from the game's folder now.
    fn running_in(root: &str) -> Vec<u32> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
        let mut sys = System::new_with_specifics(RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always)));
        sys.refresh_processes(ProcessesToUpdate::All, true);
        sys.processes()
            .iter()
            .filter(|(_, p)| p.exe().is_some_and(|e| e.to_string_lossy().to_lowercase().replace('/', "\\").starts_with(root)))
            .map(|(pid, _)| pid.as_u32())
            .collect()
    }

    /// The session's properties, with room for its name after them.
    fn properties() -> Vec<u64> {
        let name = wide(SESSION);
        let base = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
        let size = base + name.len() * 2 + 2048;
        let mut buf = vec![0u64; size.div_ceil(8)];
        // SAFETY: the buffer is 8-aligned and at least `base` bytes; the
        // struct is plain data.
        let p = unsafe { &mut *(buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) };
        p.Wnode.BufferSize = size as u32;
        p.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        p.Wnode.ClientContext = 1;
        p.Wnode.Guid = SESSION_GUID;
        p.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        p.BufferSize = 256;
        p.MinimumBuffers = 32;
        p.MaximumBuffers = 128;
        p.FlushTimer = 1;
        p.LoggerNameOffset = base as u32;
        buf
    }

    fn stop_session() -> u32 {
        let mut props = properties();
        let name = wide(SESSION);
        // SAFETY: a properties buffer as ControlTrace wants it.
        let r = unsafe { ControlTraceW(CONTROLTRACE_HANDLE { Value: 0 }, name.as_ptr(), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, EVENT_TRACE_CONTROL_STOP) };
        // SAFETY: as above, filled in by the call.
        let lost = unsafe { (*(props.as_ptr() as *const EVENT_TRACE_PROPERTIES)).EventsLost };
        if r == 0 {
            lost
        } else {
            0
        }
    }

    pub fn run(dir: &Path, game_dir: &Path, parent: u32) -> Result<(), String> {
        let game_root = format!("{}\\", game_dir.to_string_lossy().to_lowercase().replace('/', "\\").trim_end_matches('\\'));
        if game_root.len() < 4 {
            return Err("no game folder".into());
        }
        // A session left behind by a helper that was killed.
        stop_session();
        let mut props = properties();
        let name = wide(SESSION);
        let mut session = CONTROLTRACE_HANDLE { Value: 0 };
        // SAFETY: properties buffer built for this call.
        let r = unsafe { StartTraceW(&mut session, name.as_ptr(), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) };
        if r != 0 {
            return Err(format!("could not start the trace (error {r})"));
        }
        for (provider, keywords) in [(KERNEL_PROCESS, PROCESS_KEYWORDS), (KERNEL_FILE, FILE_KEYWORDS)] {
            // SAFETY: a session handle from StartTraceW and a provider GUID.
            let r = unsafe { EnableTraceEx2(session, &provider, EVENT_CONTROL_CODE_ENABLE_PROVIDER, TRACE_LEVEL_VERBOSE as u8, keywords, 0, 0, std::ptr::null()) };
            if r != 0 {
                stop_session();
                return Err(format!("could not follow file writes (error {r})"));
            }
        }

        let state = Box::new(State {
            game_root: game_root.clone(),
            devices: devices(),
            names: Names {
                file_name: wide("FileName"),
                file_path: wide("FilePath"),
                file_object: wide("FileObject"),
                create_options: wide("CreateOptions"),
                process_id: wide("ProcessID"),
                parent_id: wide("ParentProcessID"),
                image_name: wide("ImageName"),
            },
            watched: Mutex::new(running_in(&game_root).into_iter().collect()),
            objects: Mutex::new(HashMap::new()),
            written: Mutex::new(HashSet::new()),
        });
        // Leaked on purpose: the callback may still run if ProcessTrace does
        // not return; the helper exits right after anyway.
        let state: &'static State = Box::leak(state);

        let mut logger = wide(SESSION);
        // SAFETY: zeroed EVENT_TRACE_LOGFILEW is its empty state; the name and
        // the state outlive the trace.
        let mut logfile: EVENT_TRACE_LOGFILEW = unsafe { std::mem::zeroed() };
        logfile.LoggerName = logger.as_mut_ptr();
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(on_event);
        logfile.Context = state as *const State as *mut _;
        let trace: PROCESSTRACE_HANDLE = unsafe { OpenTraceW(&mut logfile) };
        if trace.Value == u64::MAX || trace.Value == u32::MAX as u64 {
            stop_session();
            return Err("could not read the trace".into());
        }
        let handle = trace.Value;
        let reader = std::thread::spawn(move || {
            let h = PROCESSTRACE_HANDLE { Value: handle };
            // SAFETY: a handle from OpenTraceW; returns when the session stops.
            unsafe { ProcessTrace(&h, 1, std::ptr::null(), std::ptr::null()) };
        });
        let _ = std::fs::write(dir.join("ready"), b"");

        // Until Kryoto Desktop says stop (or is gone): keep adding processes
        // started from the game's folder another way.
        let started = Instant::now();
        let mut followed: HashSet<u32> = HashSet::new();
        loop {
            if dir.join("stop").exists() || !crate::handoff::alive(parent) || started.elapsed() > LONGEST {
                break;
            }
            let now = running_in(&game_root);
            if let Ok(mut w) = state.watched.lock() {
                w.extend(now.iter().copied());
                followed.extend(w.iter().copied());
            }
            std::thread::sleep(Duration::from_millis(500));
        }

        let lost = stop_session();
        let waited = Instant::now();
        while !reader.is_finished() && waited.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        // SAFETY: the handle from OpenTraceW.
        unsafe { CloseTrace(trace) };
        let mut written: Vec<String> = state.written.lock().map(|w| w.iter().cloned().collect()).unwrap_or_default();
        written.sort();
        let out = TraceFile { written, lost, processes: followed.len() as u32 };
        let tmp = dir.join("trace.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&out).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join("trace.json")).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// A "game" (a copy of cmd.exe in its own folder) writes its save while
    /// another program writes beside it at the same time: precise finding
    /// sees the game's file and not the other. Opt-in, it asks for
    /// administrator (one Windows prompt) and needs the app built
    /// (`cargo build`): `cargo test precise_sees -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn precise_sees_exactly_what_the_game_wrote() {
        let exe = std::env::var_os("KRYOTO_DESKTOP_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("kryoto-desktop.exe"));
        assert!(exe.is_file(), "build the app first: {}", exe.display());
        let base = std::env::temp_dir().join(format!("kryo-precise-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let game = base.join("Kryo Precise Test");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::copy(r"C:\Windows\System32\cmd.exe", game.join("Game.exe")).unwrap();
        let saves = base.join("Saves").join("Zebra");
        let noise = base.join("Noise");
        std::fs::create_dir_all(&noise).unwrap();

        let precise = match begin_with(&exe, &base.join("trace"), &game) {
            Begin::Running(p) => p,
            Begin::Declined => panic!("administrator was declined"),
            Begin::Failed(e) => panic!("did not start: {e}"),
        };
        // cmd.exe takes its line as written: Rust's quoting (\") would mangle it.
        use std::os::windows::process::CommandExt;
        let other = std::process::Command::new("cmd")
            .raw_arg(format!("/c \"for /l %i in (1,1,6) do (echo %i>> \"{}\\build.o\" & ping -n 2 127.0.0.1 >nul)\"", noise.display()))
            .spawn()
            .unwrap();
        let script = format!("/c \"mkdir \"{s}\" & echo level=3> \"{s}\\slot.sav\" & ping -n 3 127.0.0.1 >nul & echo gold=99>> \"{s}\\slot.sav\"\"", s = saves.display());
        let mut game_run = std::process::Command::new(game.join("Game.exe")).raw_arg(script).spawn().unwrap();
        game_run.wait().unwrap();
        let mut other = other;
        other.wait().unwrap();
        let trace = precise.finish().expect("no trace written");
        println!("{} written by {} process(es), {} lost", trace.written.len(), trace.processes, trace.lost);
        for w in &trace.written {
            println!("  {w}");
        }
        let has = |p: &Path| trace.written.iter().any(|w| w.eq_ignore_ascii_case(&p.to_string_lossy()));
        assert!(has(&saves.join("slot.sav")), "the game's save was not seen");
        assert!(!has(&noise.join("build.o")), "another program's file was taken for the game's");
        assert!(trace.processes >= 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The whole path the app takes: the smart tracer watching the real
    /// profile with the precise helper beside it, then `candidates`. The game
    /// saves in Documents under a codename nothing ties to it, while another
    /// program writes in Documents too: only the game's folder comes out.
    /// Opt-in, asks for administrator: `cargo test precise_finds -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn precise_finds_a_codename_save_folder_and_nothing_else() {
        use std::os::windows::process::CommandExt;
        let exe = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("kryoto-desktop.exe");
        let base = std::env::temp_dir().join(format!("kryo-precise-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let game = base.join("Kryo Precise Test");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::copy(r"C:\Windows\System32\cmd.exe", game.join("Game.exe")).unwrap();
        let roots = super::super::win::roots(Some(game.clone()));
        let docs = roots.documents.clone().unwrap();
        let (saves, noise) = (docs.join("KryoZebraCodename"), docs.join("KryoNoiseBuild"));
        let tracer_dir = base.join("trace");
        let mut tracer = super::super::tracer::watch(roots.clone(), vec![tracer_dir.clone()], Vec::new);
        tracer.precise = match begin_with(&exe, &tracer_dir, &game) {
            Begin::Running(p) => Some(p),
            Begin::Declined => panic!("administrator was declined"),
            Begin::Failed(e) => panic!("did not start: {e}"),
        };
        let mut other = std::process::Command::new("cmd")
            .raw_arg(format!("/c \"mkdir \"{n}\" & for /l %i in (1,1,5) do (echo %i>> \"{n}\\build.o\" & ping -n 2 127.0.0.1 >nul)\"", n = noise.display()))
            .spawn()
            .unwrap();
        let mut run = std::process::Command::new(game.join("Game.exe"))
            .raw_arg(format!("/c \"mkdir \"{s}\\profiles\" & echo level=3> \"{s}\\profiles\\slot0\" & ping -n 3 127.0.0.1 >nul\"", s = saves.display()))
            .spawn()
            .unwrap();
        run.wait().unwrap();
        other.wait().unwrap();
        let obs = tracer.finish(Duration::from_secs(2));
        let hints = kryoto_saves::rules::GameHints { title: "Kryo Precise Test".into(), exe_stem: "Game".into(), ..Default::default() };
        let found = kryoto_saves::trace::candidates(&obs, &roots, &hints, &[], &|_| false);
        for c in &found {
            println!("{:>4} {:?} {} {:?}", c.score, c.rule.root, c.rule.path, c.reasons);
        }
        let _ = std::fs::remove_dir_all(&saves);
        let _ = std::fs::remove_dir_all(&noise);
        let _ = std::fs::remove_dir_all(&base);
        let files: Vec<_> = found.iter().filter(|c| c.rule.kind == kryoto_saves::Kind::Files).collect();
        assert_eq!(files.len(), 1, "{found:?}");
        assert_eq!((files[0].rule.root, files[0].rule.path.as_str()), (kryoto_saves::Root::Documents, "KryoZebraCodename/profiles"));
        assert!(files[0].corroborated());
    }
}
