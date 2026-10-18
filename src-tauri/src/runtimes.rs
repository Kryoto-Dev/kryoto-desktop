//! Game runtimes: the Microsoft redistributables most Windows games expect
//! (Visual C++, DirectX's extra files, XNA 4.0, .NET), found missing and
//! installed in one go.
//!
//! Each runtime is checked the way its installer records itself (a registry
//! value, or the DLL it puts in System32 / SysWOW64). Installing downloads the
//! official installers from Microsoft into the app's cache first, with
//! progress, then runs every one of them from a single elevated PowerShell
//! script, so Windows asks for permission once. That script refuses any file
//! not validly signed by Microsoft, runs each installer silently, and writes
//! one line per step to a status file this side reads for progress. Cancel
//! stops a download at once, and stops the script before its next installer.
//!
//! Windows only: on Linux, Proton brings its own.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Copy)]
enum Install {
    /// An .exe run with these arguments.
    Exe(&'static str),
    /// An .msi, through msiexec, quietly.
    Msi,
    /// A Windows feature, through DISM (nothing to download).
    Feature(&'static str),
}

#[derive(Clone, Copy)]
enum Check {
    /// HKLM DWORD (64-bit view; 32-bit keys name WOW6432Node) equal to 1.
    Reg(&'static str, &'static str),
    /// Every one of these files exists, relative to %SystemRoot%.
    Files(&'static [&'static str]),
    /// A folder under %ProgramFiles% holds an entry starting with this.
    DirPrefix(&'static str, &'static str),
}

struct Runtime {
    id: &'static str,
    name: &'static str,
    group: &'static str,
    /// Why a game wants it, in a few words.
    detail: &'static str,
    url: &'static str,
    file: &'static str,
    /// About how big the download is, for the prompt.
    mb: u32,
    install: Install,
    check: Check,
    /// In the set offered at startup (the rest are in Settings only).
    recommended: bool,
}

const RUNTIMES: &[Runtime] = &[
    Runtime {
        id: "vc2015-x64",
        name: "Visual C++ 2015-2022 (64-bit)",
        group: "Visual C++",
        detail: "Needed by most games made since 2015.",
        url: "https://aka.ms/vs/17/release/vc_redist.x64.exe",
        file: "vc_redist_2015_2022.x64.exe",
        mb: 25,
        install: Install::Exe("/install /quiet /norestart"),
        check: Check::Reg(r"SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64", "Installed"),
        recommended: true,
    },
    Runtime {
        id: "vc2015-x86",
        name: "Visual C++ 2015-2022 (32-bit)",
        group: "Visual C++",
        detail: "The same, for 32-bit games.",
        url: "https://aka.ms/vs/17/release/vc_redist.x86.exe",
        file: "vc_redist_2015_2022.x86.exe",
        mb: 14,
        install: Install::Exe("/install /quiet /norestart"),
        check: Check::Reg(r"SOFTWARE\WOW6432Node\Microsoft\VisualStudio\14.0\VC\Runtimes\x86", "Installed"),
        recommended: true,
    },
    Runtime {
        id: "vc2013-x64",
        name: "Visual C++ 2013 (64-bit)",
        group: "Visual C++",
        detail: "Games from around 2014.",
        url: "https://aka.ms/highdpimfc2013x64enu",
        file: "vcredist_2013.x64.exe",
        mb: 7,
        install: Install::Exe("/install /quiet /norestart"),
        check: Check::Files(&[r"System32\msvcr120.dll"]),
        recommended: true,
    },
    Runtime {
        id: "vc2013-x86",
        name: "Visual C++ 2013 (32-bit)",
        group: "Visual C++",
        detail: "Games from around 2014, 32-bit.",
        url: "https://aka.ms/highdpimfc2013x86enu",
        file: "vcredist_2013.x86.exe",
        mb: 7,
        install: Install::Exe("/install /quiet /norestart"),
        check: Check::Files(&[r"SysWOW64\msvcr120.dll"]),
        recommended: true,
    },
    Runtime {
        id: "vc2012-x64",
        name: "Visual C++ 2012 (64-bit)",
        group: "Visual C++",
        detail: "Games from around 2013.",
        url: "https://download.microsoft.com/download/1/6/B/16B06F60-3B20-4FF2-B699-5E9B7962F9AE/VSU_4/vcredist_x64.exe",
        file: "vcredist_2012.x64.exe",
        mb: 7,
        install: Install::Exe("/install /quiet /norestart"),
        check: Check::Files(&[r"System32\msvcr110.dll"]),
        recommended: true,
    },
    Runtime {
        id: "vc2012-x86",
        name: "Visual C++ 2012 (32-bit)",
        group: "Visual C++",
        detail: "Games from around 2013, 32-bit.",
        url: "https://download.microsoft.com/download/1/6/B/16B06F60-3B20-4FF2-B699-5E9B7962F9AE/VSU_4/vcredist_x86.exe",
        file: "vcredist_2012.x86.exe",
        mb: 7,
        install: Install::Exe("/install /quiet /norestart"),
        check: Check::Files(&[r"SysWOW64\msvcr110.dll"]),
        recommended: true,
    },
    Runtime {
        id: "vc2010-x64",
        name: "Visual C++ 2010 (64-bit)",
        group: "Visual C++",
        detail: "Older games, from 2010 to 2012.",
        url: "https://download.microsoft.com/download/1/6/5/165255E7-1014-4D0A-B094-B6A430A6BFFC/vcredist_x64.exe",
        file: "vcredist_2010.x64.exe",
        mb: 10,
        install: Install::Exe("/q /norestart"),
        check: Check::Files(&[r"System32\msvcr100.dll"]),
        recommended: true,
    },
    Runtime {
        id: "vc2010-x86",
        name: "Visual C++ 2010 (32-bit)",
        group: "Visual C++",
        detail: "Older games, 32-bit.",
        url: "https://download.microsoft.com/download/1/6/5/165255E7-1014-4D0A-B094-B6A430A6BFFC/vcredist_x86.exe",
        file: "vcredist_2010.x86.exe",
        mb: 9,
        install: Install::Exe("/q /norestart"),
        check: Check::Files(&[r"SysWOW64\msvcr100.dll"]),
        recommended: true,
    },
    Runtime {
        id: "directx",
        name: "DirectX End-User Runtime (June 2010)",
        group: "DirectX",
        detail: "The extra DirectX 9, 10 and 11 files (d3dx9, XInput 1.3, XAudio 2.7) older games load. Windows does not include them.",
        url: "https://download.microsoft.com/download/1/7/1/1718CCC4-6315-4D8E-9543-8E28A4E18C4C/dxwebsetup.exe",
        file: "dxwebsetup.exe",
        mb: 1,
        install: Install::Exe("/Q"),
        check: Check::Files(&[r"System32\d3dx9_43.dll", r"SysWOW64\d3dx9_43.dll", r"SysWOW64\xinput1_3.dll", r"SysWOW64\XAudio2_7.dll"]),
        recommended: true,
    },
    Runtime {
        id: "xna40",
        name: "XNA Framework 4.0",
        group: "XNA",
        detail: "Games built with XNA, like Terraria and many indies from 2010 to 2016.",
        url: "https://download.microsoft.com/download/A/C/2/AC2C903B-E6E8-42C2-9FD7-BEBAC362A930/xnafx40_redist.msi",
        file: "xnafx40_redist.msi",
        mb: 7,
        install: Install::Msi,
        check: Check::Reg(r"SOFTWARE\WOW6432Node\Microsoft\XNA\Framework\v4.0", "Installed"),
        recommended: true,
    },
    Runtime {
        id: "dotnet8-desktop",
        name: ".NET 8 Desktop Runtime (64-bit)",
        group: ".NET",
        detail: "Newer tools and games built on .NET, and many mod managers.",
        url: "https://aka.ms/dotnet/8.0/windowsdesktop-runtime-win-x64.exe",
        file: "windowsdesktop-runtime-8.x64.exe",
        mb: 56,
        install: Install::Exe("/install /quiet /norestart"),
        check: Check::DirPrefix(r"dotnet\shared\Microsoft.WindowsDesktop.App", "8."),
        recommended: false,
    },
    Runtime {
        id: "netfx35",
        name: ".NET Framework 3.5",
        group: ".NET",
        detail: "Games and launchers from the 2000s. A Windows feature: Windows Update supplies it, which can take a few minutes.",
        url: "",
        file: "",
        mb: 0,
        install: Install::Feature("NetFx3"),
        check: Check::Reg(r"SOFTWARE\Microsoft\NET Framework Setup\NDP\v3.5", "Install"),
        recommended: false,
    },
];

/// An installer's exit codes that mean it is there now.
const OK_CODES: &[i64] = &[0, 1638, 5100, 3010, 1641];
const REBOOT_CODES: &[i64] = &[3010, 1641];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeState {
    id: &'static str,
    name: &'static str,
    group: &'static str,
    detail: &'static str,
    mb: u32,
    recommended: bool,
    installed: bool,
    /// Its installer is in the cache already (no download needed).
    cached: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    id: String,
    name: String,
    ok: bool,
    detail: String,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// idle, downloading, permission, installing, done, failed, canceled
    phase: &'static str,
    /// The runtime being downloaded or installed now.
    current: Option<String>,
    step: usize,
    steps: usize,
    /// The whole job, 0 to 1.
    fraction: f64,
    message: Option<String>,
    results: Vec<Outcome>,
    /// Something asked for a restart to finish.
    reboot: bool,
}

static PROGRESS: Mutex<Option<Progress>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

fn publish(app: &AppHandle, p: &Progress) {
    if let Ok(mut g) = PROGRESS.lock() {
        *g = Some(p.clone());
    }
    let _ = app.emit("runtimes-progress", p);
}

fn cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let d = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("runtimes");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

#[cfg(windows)]
fn reg_dword(path: &str, name: &str) -> Option<u32> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_SUBKEY_WOW6464KEY};
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: both strings are NUL-terminated and `data` is a u32 of `size` bytes.
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6464KEY,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        )
    };
    (r == 0).then_some(data)
}

#[cfg(not(windows))]
fn reg_dword(_: &str, _: &str) -> Option<u32> {
    None
}

fn installed(r: &Runtime) -> bool {
    let env_dir = |k: &str, d: &str| std::env::var_os(k).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(d));
    match r.check {
        Check::Reg(path, name) => reg_dword(path, name) == Some(1),
        Check::Files(files) => {
            let root = env_dir("SystemRoot", r"C:\Windows");
            files.iter().all(|f| root.join(f).is_file())
        }
        Check::DirPrefix(dir, prefix) => {
            let root = env_dir("ProgramFiles", r"C:\Program Files");
            std::fs::read_dir(root.join(dir))
                .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with(prefix)))
                .unwrap_or(false)
        }
    }
}

/// Every runtime, with whether it is installed. Empty off Windows.
#[tauri::command(async)]
pub fn runtimes_list(app: AppHandle) -> Vec<RuntimeState> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let cache = cache_dir(&app).ok();
    RUNTIMES
        .iter()
        .map(|r| RuntimeState {
            id: r.id,
            name: r.name,
            group: r.group,
            detail: r.detail,
            mb: r.mb,
            recommended: r.recommended,
            installed: installed(r),
            cached: r.file.is_empty() || cache.as_ref().is_some_and(|c| c.join(r.file).is_file()),
        })
        .collect()
}

/// The job running now, or the last one's result.
#[tauri::command]
pub fn runtimes_progress() -> Option<Progress> {
    PROGRESS.lock().ok().and_then(|g| g.clone())
}

#[tauri::command]
pub fn runtimes_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Throw away the downloaded installers (they are kept so a repair needs no
/// download). Not while a job runs.
#[tauri::command(async)]
pub fn runtimes_clear_cache(app: AppHandle) -> Result<u64, String> {
    if RUNNING.load(Ordering::SeqCst) {
        return Err("Wait for the install to finish first.".into());
    }
    let dir = cache_dir(&app)?;
    let mut freed = 0;
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        if let Ok(m) = e.metadata() {
            if m.is_file() && std::fs::remove_file(e.path()).is_ok() {
                freed += m.len();
            }
        }
    }
    Ok(freed)
}

/// Install these runtimes: the ones missing, or every one named when
/// `repair` (installing over a broken copy is how these are repaired).
/// Returns at once; progress comes as `runtimes-progress` events.
#[tauri::command]
pub fn runtimes_install(app: AppHandle, ids: Vec<String>, repair: bool) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Game runtimes are only for Windows.".into());
    }
    let chosen: Vec<&'static Runtime> = RUNTIMES.iter().filter(|r| ids.iter().any(|i| i == r.id)).filter(|r| repair || !installed(r)).collect();
    if chosen.is_empty() {
        return Err("Everything you picked is installed already.".into());
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Game runtimes are being installed already.".into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        let result = run(&app, &chosen).await;
        if let Err(e) = result {
            crate::logging::error("runtimes", &e);
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

async fn run(app: &AppHandle, chosen: &[&'static Runtime]) -> Result<(), String> {
    let steps = chosen.len();
    let mut p = Progress { phase: "downloading", steps, ..Default::default() };
    publish(app, &p);
    let fail = |app: &AppHandle, p: &mut Progress, phase: &'static str, msg: String| {
        p.phase = phase;
        p.current = None;
        p.message = Some(msg.clone());
        publish(app, p);
        msg
    };
    let dir = match cache_dir(app) {
        Ok(d) => d,
        Err(e) => return Err(fail(app, &mut p, "failed", format!("Could not make a folder for the installers: {e}"))),
    };
    crate::logging::info("runtimes", &format!("installing {}", chosen.iter().map(|r| r.id).collect::<Vec<_>>().join(", ")));

    // 1. Download what is not in the cache, with progress (the first 40%).
    let to_get: Vec<&Runtime> = chosen.iter().copied().filter(|r| !r.url.is_empty() && !dir.join(r.file).is_file()).collect();
    let total_mb: f64 = to_get.iter().map(|r| r.mb.max(1) as f64).sum::<f64>().max(1.0);
    let mut done_mb = 0.0;
    let client = reqwest::Client::builder().user_agent("Kryoto Desktop").build().unwrap_or_default();
    for (i, r) in to_get.iter().enumerate() {
        p.current = Some(r.name.into());
        p.step = i + 1;
        p.message = Some(format!("Downloading {} of {}", i + 1, to_get.len()));
        publish(app, &p);
        let weight = r.mb.max(1) as f64;
        let base = done_mb;
        let res = download(&client, r.url, &dir.join(r.file), |f| {
            p.fraction = 0.4 * (base + weight * f) / total_mb;
            publish(app, &p);
        })
        .await;
        match res {
            Ok(()) => done_mb += weight,
            Err(e) if e == "canceled" => return Err(fail(app, &mut p, "canceled", "Canceled. Nothing was installed.".into())),
            Err(e) => return Err(fail(app, &mut p, "failed", format!("Could not download {}: {e}", r.name))),
        }
    }
    if CANCEL.load(Ordering::SeqCst) {
        return Err(fail(app, &mut p, "canceled", "Canceled. Nothing was installed.".into()));
    }

    // 2. One elevated script runs every installer (the other 60%).
    p.phase = "permission";
    p.current = None;
    p.step = 0;
    p.fraction = 0.4;
    p.message = Some("Windows is asking for permission".into());
    publish(app, &p);
    let status = dir.join("status.txt");
    let cancel_flag = dir.join("cancel.flag");
    let _ = std::fs::remove_file(&status);
    let _ = std::fs::remove_file(&cancel_flag);
    // A script left by an older version, which ran it from this folder.
    let _ = std::fs::remove_file(dir.join("install.ps1"));
    let script = script_for(chosen, &dir, &status, &cancel_flag);

    let mut child = elevate(&script).map_err(|e| fail(app, &mut p, "failed", format!("Could not start the installers: {e}")))?;
    let mut seen = 0usize;
    let mut results: Vec<Outcome> = Vec::new();
    let mut canceled = false;
    let mut cancel_sent = false;
    let exit = loop {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if CANCEL.load(Ordering::SeqCst) && !cancel_sent {
            let _ = std::fs::write(&cancel_flag, b"1");
            cancel_sent = true;
            p.message = Some("Stopping after this one".into());
            publish(app, &p);
        }
        let lines: Vec<String> = std::fs::read_to_string(&status).map(|s| s.lines().map(str::to_string).collect()).unwrap_or_default();
        for line in lines.iter().skip(seen) {
            let mut parts = line.trim_start_matches('\u{feff}').split_whitespace();
            let (verb, id) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let r = chosen.iter().find(|r| r.id == id);
            match (verb, r) {
                ("start", Some(r)) => {
                    p.phase = "installing";
                    p.current = Some(r.name.into());
                    p.step = results.len() + 1;
                    p.message = Some(format!("Installing {} of {}", p.step, steps));
                }
                ("done", Some(r)) => {
                    let code: i64 = parts.next().and_then(|c| c.parse().ok()).unwrap_or(-1);
                    let ok = OK_CODES.contains(&code);
                    if REBOOT_CODES.contains(&code) {
                        p.reboot = true;
                    }
                    results.push(Outcome {
                        id: r.id.into(),
                        name: r.name.into(),
                        ok,
                        detail: if ok { "Installed".into() } else { format!("The installer stopped with code {code}") },
                    });
                }
                ("bad", Some(r)) => results.push(Outcome { id: r.id.into(), name: r.name.into(), ok: false, detail: "Not signed by Microsoft, so it was not run. Clear the downloaded installers and try again.".into() }),
                ("canceled", _) => canceled = true,
                _ => {}
            }
            p.results = results.clone();
            p.fraction = 0.4 + 0.6 * (results.len() as f64 / steps as f64);
            publish(app, &p);
        }
        seen = lines.len();
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {}
            Err(e) => break { crate::logging::error("runtimes", &e.to_string()); None },
        }
    };
    let stderr = child.stderr.take().map(|mut s| {
        let mut out = String::new();
        let _ = std::io::Read::read_to_string(&mut s, &mut out);
        out
    });
    let _ = std::fs::remove_file(&cancel_flag);

    p.current = None;
    p.results = results.clone();
    if results.is_empty() && !canceled {
        // Never started: most often the permission prompt was declined.
        let why = stderr.unwrap_or_default();
        crate::logging::warn("runtimes", &format!("elevated script ended with {exit:?}: {}", why.trim()));
        let declined = exit == Some(1223) || why.contains("canceled by the user") || why.contains("cancelled by the user");
        return Err(fail(
            app,
            &mut p,
            "failed",
            if declined { "Windows did not get permission, so nothing was installed.".into() } else { "The installers did not start. Try again, or see Settings > Logs.".into() },
        ));
    }
    let failed = results.iter().filter(|o| !o.ok).count();
    p.fraction = 1.0;
    p.phase = if canceled { "canceled" } else if failed > 0 { "failed" } else { "done" };
    p.message = Some(match (canceled, failed) {
        (true, _) => format!("Stopped. {} installed before you canceled.", results.iter().filter(|o| o.ok).count()),
        (false, 0) if p.reboot => "Installed. Restart your PC to finish.".into(),
        (false, 0) => "All installed.".into(),
        (false, n) => format!("{n} did not install."),
    });
    crate::logging::info("runtimes", p.message.as_deref().unwrap_or(""));
    publish(app, &p);
    // Also as a notice: it may have finished behind the app (Continue on the
    // start screen), where nobody is watching the bar.
    if !canceled {
        let title = if failed > 0 { "Some game runtimes did not install" } else { "Game runtimes installed" };
        let body = if failed > 0 { "See Settings > Game runtimes for which, and try again there." } else { p.message.as_deref().unwrap_or("All installed.") };
        let notice = crate::downloads::Notice::new(title, body, None);
        let _ = app.emit("notify", if failed > 0 { notice.sound("error") } else { notice });
    }
    Ok(())
}

async fn download(client: &reqwest::Client, url: &str, to: &Path, mut on: impl FnMut(f64)) -> Result<(), String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let res = client.get(url).send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?;
    let total = res.content_length();
    let part = to.with_extension("part");
    let mut file = tokio::fs::File::create(&part).await.map_err(|e| e.to_string())?;
    let mut got = 0u64;
    let mut last = std::time::Instant::now();
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if CANCEL.load(Ordering::SeqCst) {
            drop(file);
            let _ = tokio::fs::remove_file(&part).await;
            return Err("canceled".into());
        }
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        got += chunk.len() as u64;
        if last.elapsed().as_millis() > 120 {
            last = std::time::Instant::now();
            if let Some(t) = total.filter(|t| *t > 0) {
                on((got as f64 / t as f64).min(1.0));
            }
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if total.is_some_and(|t| t != got) {
        let _ = tokio::fs::remove_file(&part).await;
        return Err("the download was cut short".into());
    }
    tokio::fs::rename(&part, to).await.map_err(|e| e.to_string())?;
    on(1.0);
    Ok(())
}

/// A single-quoted PowerShell string.
fn ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The elevated script: for each runtime, check the signature, run it,
/// write `start`, then `done <code>` (or `bad`); stop early on the flag.
///
/// Nothing it runs as administrator is read from a folder this user can write
/// to after the check. The installers were downloaded into the user's cache,
/// where any program running as the user could swap one between the signature
/// check and the run, and Windows would start the swapped file elevated. So
/// each is first copied into a new folder under Program Files (writable by
/// administrators only), and the copy is checked and run. The script itself
/// never touches the disk: it is handed to PowerShell on the command line
/// (`elevate`). Only the progress lines and the cancel flag stay in the cache,
/// and they carry no code.
fn script_for(chosen: &[&Runtime], dir: &Path, status: &Path, cancel: &Path) -> String {
    let mut s = String::new();
    s.push_str("$ErrorActionPreference = 'Continue'\r\n");
    s.push_str(&format!("$status = {}\r\n$cancel = {}\r\n", ps(&status.to_string_lossy()), ps(&cancel.to_string_lossy())));
    s.push_str("function Say($line) { Add-Content -LiteralPath $status -Value $line -Encoding ASCII }\r\n");
    s.push_str("function Signed($file) { $sig = Get-AuthenticodeSignature -LiteralPath $file; return ($sig.Status -eq 'Valid' -and $sig.SignerCertificate.Subject -match 'O=Microsoft Corporation') }\r\n");
    s.push_str("function Run($exe, $argv) { try { $p = Start-Process -FilePath $exe -ArgumentList $argv -Wait -PassThru -WindowStyle Hidden; return $p.ExitCode } catch { return -1 } }\r\n");
    // A fresh name each time, made by this elevated process: nobody can have
    // put anything there first.
    s.push_str("$safe = Join-Path $env:ProgramFiles ('Kryoto Runtimes ' + [guid]::NewGuid().ToString('N'))\r\n");
    s.push_str("New-Item -ItemType Directory -Path $safe -ErrorAction Stop | Out-Null\r\n");
    s.push_str("function CopyIn($file) { try { $to = Join-Path $safe (Split-Path -Leaf $file); Copy-Item -LiteralPath $file -Destination $to -ErrorAction Stop; return $to } catch { return $null } }\r\n");
    s.push_str("try {\r\n");
    for r in chosen {
        s.push_str(&format!("if (Test-Path -LiteralPath $cancel) {{ Say 'canceled'; exit 0 }}\r\nSay 'start {}'\r\n", r.id));
        match r.install {
            Install::Feature(feature) => {
                s.push_str(&format!("$code = Run 'dism.exe' '/Online /Enable-Feature /FeatureName:{feature} /All /NoRestart /Quiet'\r\nSay \"done {} $code\"\r\n", r.id));
            }
            Install::Exe(args) => {
                let file = ps(&dir.join(r.file).to_string_lossy());
                s.push_str(&format!(
                    "$copy = CopyIn {file}\r\nif ($copy -and (Signed $copy)) {{ $code = Run $copy {}; Say \"done {} $code\" }} else {{ Say 'bad {}' }}\r\n",
                    ps(args),
                    r.id,
                    r.id
                ));
            }
            Install::Msi => {
                let file = ps(&dir.join(r.file).to_string_lossy());
                s.push_str(&format!(
                    "$copy = CopyIn {file}\r\nif ($copy -and (Signed $copy)) {{ $code = Run 'msiexec.exe' ('/i \"' + $copy + '\" /quiet /norestart'); Say \"done {} $code\" }} else {{ Say 'bad {}' }}\r\n",
                    r.id, r.id
                ));
            }
        }
    }
    s.push_str("Say 'end'\r\n");
    s.push_str("} finally { Remove-Item -LiteralPath $safe -Recurse -Force -ErrorAction SilentlyContinue }\r\n");
    s
}

/// PowerShell's `Start-Process -Verb RunAs` on the script, waiting for it.
/// The script goes on the command line (`-EncodedCommand`, UTF-16LE base64),
/// never into a file another program could rewrite before it runs elevated.
#[cfg(windows)]
fn elevate(script: &str) -> std::io::Result<std::process::Child> {
    use base64::Engine;
    use std::os::windows::process::CommandExt;
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let inner = format!("-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -EncodedCommand {encoded}");
    let outer = format!(
        "try {{ $p = Start-Process -Verb RunAs -Wait -PassThru -WindowStyle Hidden -FilePath 'powershell.exe' -ArgumentList {}; exit $p.ExitCode }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1223 }}",
        ps(&inner)
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &outer])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
}

#[cfg(not(windows))]
fn elevate(_: &str) -> std::io::Result<std::process::Child> {
    Err(std::io::Error::other("only on Windows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_downloads_have_files() {
        let mut ids: Vec<_> = RUNTIMES.iter().map(|r| r.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), RUNTIMES.len());
        for r in RUNTIMES {
            match r.install {
                Install::Feature(_) => assert!(r.url.is_empty() && r.file.is_empty(), "{}", r.id),
                _ => assert!(r.url.starts_with("https://") && !r.file.is_empty(), "{}", r.id),
            }
        }
    }

    #[test]
    fn the_script_checks_signatures_and_honours_cancel() {
        let chosen: Vec<&Runtime> = RUNTIMES.iter().filter(|r| ["vc2015-x64", "xna40", "netfx35"].contains(&r.id)).collect();
        let s = script_for(&chosen, Path::new(r"C:\Cache\it's"), Path::new(r"C:\Cache\status.txt"), Path::new(r"C:\Cache\cancel.flag"));
        // A quote in the path is doubled, never ends the string.
        let file = Path::new(r"C:\Cache\it's").join("vc_redist_2015_2022.x64.exe").to_string_lossy().replace('\'', "''");
        // Copied somewhere only administrators can write, then checked and run from there.
        assert!(s.contains(&format!("$copy = CopyIn '{file}'")), "{s}");
        assert!(s.contains("if ($copy -and (Signed $copy))"), "{s}");
        assert!(s.contains("$env:ProgramFiles") && s.contains("Remove-Item -LiteralPath $safe"), "{s}");
        assert!(!s.contains(&format!("Run '{file}'")), "the cached file itself is never run: {s}");
        assert!(s.contains("msiexec.exe"));
        assert!(s.contains("/FeatureName:NetFx3"));
        assert_eq!(s.matches("Test-Path -LiteralPath $cancel").count(), 3);
        assert!(s.contains("O=Microsoft Corporation"));
        assert!(s.contains("Say 'end'\r\n} finally {"), "the copies are removed however it ends: {s}");
    }

    /// The whole script, for PowerShell's parser:
    /// `cargo test --lib print_the_script -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_the_script() {
        let all: Vec<&Runtime> = RUNTIMES.iter().collect();
        println!("{}", script_for(&all, Path::new(r"C:\Cache\it's"), Path::new(r"C:\Cache\status.txt"), Path::new(r"C:\Cache\cancel.flag")));
    }

    #[cfg(windows)]
    #[test]
    fn detection_runs_on_this_pc() {
        // Every Windows 10/11 PC has System32; this only proves the checks run.
        for r in RUNTIMES {
            println!("{:<18} {}", r.id, if installed(r) { "installed" } else { "missing" });
        }
        assert!(reg_dword(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentMajorVersionNumber").is_some());
    }
}
