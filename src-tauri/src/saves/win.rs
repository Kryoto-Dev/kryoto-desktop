//! Windows: the user's real folders, and the files a game holds open.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{GetFileType, GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, FILE_TYPE_DISK};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE, PROCESS_QUERY_INFORMATION};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_Documents, FOLDERID_LocalAppData, FOLDERID_LocalAppDataLow, FOLDERID_Profile, FOLDERID_ProgramData, FOLDERID_Public,
    FOLDERID_RoamingAppData, FOLDERID_SavedGames, SHGetKnownFolderPath,
};

fn known(id: &GUID) -> Option<PathBuf> {
    let mut out: windows_sys::core::PWSTR = std::ptr::null_mut();
    // SAFETY: SHGetKnownFolderPath writes a CoTaskMem string we free below.
    let hr = unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut out) };
    if hr != 0 || out.is_null() {
        if !out.is_null() {
            unsafe { CoTaskMemFree(out as *const _) };
        }
        return None;
    }
    let len = (0..).take_while(|&i| unsafe { *out.add(i) } != 0).count();
    let path = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(out, len) });
    unsafe { CoTaskMemFree(out as *const _) };
    Some(PathBuf::from(path)).filter(|p| p.is_dir())
}

/// This PC's folders, where Windows really has them: a Documents folder
/// moved into OneDrive or to another drive is found where it is.
pub fn roots(game_dir: Option<PathBuf>) -> kryoto_saves::SaveRoots {
    let local = known(&FOLDERID_LocalAppData);
    kryoto_saves::SaveRoots {
        game_dir,
        app_data: known(&FOLDERID_RoamingAppData),
        local_low: known(&FOLDERID_LocalAppDataLow),
        documents: known(&FOLDERID_Documents),
        saved_games: known(&FOLDERID_SavedGames),
        public_documents: known(&FOLDERID_Public).map(|p| p.join("Documents")).filter(|p| p.is_dir()),
        program_data: known(&FOLDERID_ProgramData),
        virtual_store: local.as_ref().map(|l| l.join("VirtualStore")),
        local_app_data: local,
        home: known(&FOLDERID_Profile),
        steam_id: None,
    }
}

/* ── open files ───────────────────────────────────────────────────── */

const PROCESS_HANDLE_INFORMATION: u32 = 51;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;

#[repr(C)]
struct HandleEntry {
    handle: HANDLE,
    handle_count: usize,
    pointer_count: usize,
    granted_access: u32,
    object_type_index: u32,
    handle_attributes: u32,
    reserved: u32,
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(process: HANDLE, class: u32, info: *mut core::ffi::c_void, len: u32, ret: *mut u32) -> i32;
}

/// The handles one process holds (Windows 8 and later; works on a process of
/// the same user without administrator rights).
fn handles_of(process: HANDLE) -> Vec<HANDLE> {
    let mut size: u32 = 64 * 1024;
    for _ in 0..6 {
        let mut buf = vec![0u8; size as usize];
        let mut ret = 0u32;
        // SAFETY: the buffer is `size` bytes; the kernel writes at most that.
        let status = unsafe { NtQueryInformationProcess(process, PROCESS_HANDLE_INFORMATION, buf.as_mut_ptr().cast(), size, &mut ret) };
        if status == STATUS_INFO_LENGTH_MISMATCH {
            size = ret.max(size.saturating_mul(2)).min(64 << 20);
            continue;
        }
        if status < 0 {
            return Vec::new();
        }
        let count = usize::from_ne_bytes(buf[..std::mem::size_of::<usize>()].try_into().unwrap_or_default());
        let first = 2 * std::mem::size_of::<usize>();
        let each = std::mem::size_of::<HandleEntry>();
        let count = count.min((buf.len().saturating_sub(first)) / each);
        return (0..count)
            .map(|i| {
                // SAFETY: inside the buffer by the bound above; read unaligned.
                let e: HandleEntry = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(first + i * each).cast()) };
                e.handle
            })
            .collect();
    }
    Vec::new()
}

/// Asks for a handle's path on a helper thread, because the call can hang on
/// some handle types; one that does not answer in time is given up on.
struct Namer {
    tx: mpsc::Sender<(usize, mpsc::Sender<Option<String>>)>,
}

impl Namer {
    fn new() -> Namer {
        let (tx, rx) = mpsc::channel::<(usize, mpsc::Sender<Option<String>>)>();
        std::thread::spawn(move || {
            for (h, reply) in rx {
                let mut buf = vec![0u16; 1024];
                // SAFETY: a handle duplicated into this process; buffer sized as passed.
                let n = unsafe { GetFinalPathNameByHandleW(h as HANDLE, buf.as_mut_ptr(), buf.len() as u32, FILE_NAME_NORMALIZED) } as usize;
                let name = (n > 0 && n < buf.len()).then(|| String::from_utf16_lossy(&buf[..n]));
                let _ = reply.send(name);
            }
        });
        Namer { tx }
    }

    fn name(&mut self, h: HANDLE) -> Option<String> {
        let (reply, answer) = mpsc::channel();
        if self.tx.send((h as usize, reply)).is_err() {
            *self = Namer::new();
            return None;
        }
        match answer.recv_timeout(Duration::from_millis(200)) {
            Ok(n) => n,
            Err(_) => {
                // Stuck: leave that thread to it and start another.
                *self = Namer::new();
                None
            }
        }
    }
}

/// The disk files these processes have open right now.
pub fn open_files(pids: &[u32]) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    let mut namer = Namer::new();
    let me = unsafe { GetCurrentProcess() };
    for &pid in pids {
        // SAFETY: plain Win32 calls; every handle opened here is closed here.
        let process = unsafe { OpenProcess(PROCESS_DUP_HANDLE | PROCESS_QUERY_INFORMATION, 0, pid) };
        if process.is_null() {
            continue;
        }
        for h in handles_of(process).into_iter().take(20_000) {
            let mut dup: HANDLE = std::ptr::null_mut();
            let ok = unsafe { DuplicateHandle(process, h, me, &mut dup, 0, 0, DUPLICATE_SAME_ACCESS) };
            if ok == 0 || dup.is_null() {
                continue;
            }
            if unsafe { GetFileType(dup) } == FILE_TYPE_DISK {
                if let Some(name) = namer.name(dup) {
                    if let Some(p) = name.strip_prefix(r"\\?\").filter(|p| !p.starts_with("UNC\\")) {
                        let path = PathBuf::from(p);
                        if path.is_file() {
                            out.insert(path);
                        }
                    }
                }
            }
            unsafe { CloseHandle(dup) };
        }
        unsafe { CloseHandle(process) };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_folders_are_found() {
        let r = roots(None);
        assert!(r.app_data.is_some() && r.documents.is_some() && r.local_low.is_some() && r.home.is_some());
    }

    /// A child process that holds a file open is seen holding it.
    #[test]
    fn open_files_of_another_process_are_seen() {
        let dir = std::env::temp_dir().join(format!("kryo-open-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("held.sav");
        std::fs::write(&file, b"x").unwrap();
        // PowerShell holding the file open for a few seconds.
        let script = format!("$f=[IO.File]::Open('{}','Open','ReadWrite','ReadWrite'); Start-Sleep 4; $f.Close()", file.display());
        let mut child = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).spawn().unwrap();
        let mut seen = false;
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(200));
            if open_files(&[child.id()]).iter().any(|p| p.file_name() == file.file_name()) {
                seen = true;
                break;
            }
        }
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(seen, "the open save was not seen");
    }
}
