//! Microsoft Defender exclusions for the game folders (Windows only).
//!
//! Real-time scanning reads every file a game unpacks as it is written, which
//! slows downloads and unpacking, and it quarantines files cracked games need
//! (a false alarm on the files that let a game run without its store). With
//! the player's yes, Kryoto asks Windows to skip the library folders, or the
//! drives they are on, through one elevated PowerShell (`Add-MpPreference
//! -ExclusionPath`), after a UAC prompt.
//!
//! The paths come from Settings, never from the page: the page only says which
//! kind (`folders` or `drives`). Reading the state is read-only and needs no
//! administrator, except the list of exclusions itself, which Windows only
//! shows to one; the elevated run reports it back afterwards.
//!
//! When it cannot (UAC declined, a policy manages Defender, another antivirus
//! is in charge), Settings shows the manual steps with the paths to copy.

use serde::Serialize;
use tauri::{AppHandle, Runtime};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefenderStatus {
    /// Windows with Defender's PowerShell module: false elsewhere.
    pub supported: bool,
    /// Real-time protection on, when Windows says.
    pub realtime: Option<bool>,
    /// Defender is not the active antivirus (passive, or off for another one).
    pub passive: bool,
    /// Antivirus products Windows Security Center knows, other than Defender.
    pub others: Vec<String>,
    /// The exclusions, when Windows shows them; `None` without administrator.
    pub excluded: Option<Vec<String>>,
    /// What Kryoto itself added. Windows shows the real list only to an
    /// administrator, so this is what says it is done when `excluded` is `None`.
    pub remembered: Vec<String>,
    /// The library folders, and the drives they are on (not the Windows one).
    pub folders: Vec<String>,
    pub drives: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcludeResult {
    pub ok: bool,
    /// `declined` (UAC), `blocked` (Windows refused it) or `failed`.
    pub reason: Option<String>,
    pub message: Option<String>,
    /// The exclusions afterwards, as the elevated run read them.
    pub excluded: Option<Vec<String>>,
}

/// `defender-excluded.json`: the paths this PC's exclusions were added for.
fn record_file<R: Runtime>(app: &AppHandle<R>) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path().app_data_dir().ok().map(|d| d.join("defender-excluded.json"))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn remember<R: Runtime>(app: &AppHandle<R>, paths: &[String]) {
    let Some(file) = record_file(app) else { return };
    let mut list = remembered(app);
    for p in paths {
        if !list.iter().any(|known| crate::settings::same_path(known, p)) {
            list.push(p.clone());
        }
    }
    if let Ok(body) = serde_json::to_vec_pretty(&list) {
        let _ = std::fs::write(file, body);
    }
}

/// What Kryoto has added. A first look with no record yet reads the log:
/// 0.3.0 kept none, and whoever pressed the button then said yes to Windows
/// already, so they are not asked again.
#[cfg_attr(not(windows), allow(dead_code))]
fn remembered<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    use tauri::Manager;
    let Some(file) = record_file(app) else { return Vec::new() };
    if let Ok(body) = std::fs::read(&file) {
        return serde_json::from_slice(&body).unwrap_or_default();
    }
    let logged = app.path().app_data_dir().ok().and_then(|d| std::fs::read_dir(d.join("logs")).ok()).is_some_and(|logs| {
        logs.flatten().any(|f| std::fs::read(f.path()).is_ok_and(|text| said_added(&String::from_utf8_lossy(&text))))
    });
    let list = if logged { folders(app) } else { Vec::new() };
    if let Ok(body) = serde_json::to_vec_pretty(&list) {
        let _ = std::fs::write(file, body);
    }
    list
}

/// The line `defender_exclude` logs when Windows took them.
fn said_added(log: &str) -> bool {
    log.lines().any(|l| l.contains("exclusions for") && l.ends_with(": added"))
}

fn folders<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    let _ = crate::storage::forget_gone_folders(app);
    crate::settings::all_folders(&crate::settings::load(app)).into_iter().filter(|f| !f.trim().is_empty()).collect()
}

/// `D:\Games\Kryoto` -> `D:\`.
fn drive_of(path: &str) -> Option<String> {
    let b = path.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':').then(|| format!("{}:\\", (b[0] as char).to_ascii_uppercase()))
}

fn drives(folders: &[String]) -> Vec<String> {
    let mut out: Vec<String> = folders.iter().filter_map(|f| drive_of(f)).collect();
    out.dedup();
    out.sort();
    out.dedup();
    out
}

/// The drives a whole-drive exclusion may cover: never the Windows drive.
fn whole_drives(folders: &[String]) -> Vec<String> {
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_ascii_uppercase();
    drives(folders).into_iter().filter(|d| !d.starts_with(&system)).collect()
}

/// Which paths a choice means. `game` (an id from the library, never a path
/// from the page): the library folder that game is in, or its own folder when
/// it is in none, for the antivirus card on its page (antivirus.rs).
fn targets<R: Runtime>(app: &AppHandle<R>, kind: &str, game: Option<&str>) -> Result<Vec<String>, String> {
    let folders = folders(app);
    match kind {
        "game" => {
            let id = game.ok_or("Which game?")?;
            let game = crate::library::load(app)?.into_iter().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
            let dir = std::path::Path::new(&game.install_dir);
            let holder = folders.iter().find(|f| {
                let (f, d) = (dunce::canonicalize(f).ok(), dunce::canonicalize(dir).ok());
                matches!((f, d), (Some(f), Some(d)) if d.starts_with(&f))
            });
            Ok(vec![holder.cloned().unwrap_or_else(|| game.install_dir.clone())])
        }
        "folders" => Ok(folders),
        "drives" => {
            let picked = whole_drives(&folders);
            if picked.is_empty() {
                return Err("Your games are on the Windows drive, which is never excluded whole. Exclude the library folders instead.".into());
            }
            Ok(picked)
        }
        _ => Err("Pick the folders or the drives.".into()),
    }
}

/// A PowerShell single-quoted string.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// What the elevated PowerShell runs: add the paths, then write the result
/// (and the exclusions now in place) to `out` for Kryoto to read. `dry`
/// (debug builds' tests) checks the cmdlet is there and adds nothing:
/// Add-MpPreference has no -WhatIf.
fn script(paths: &[String], out: &str, dry: bool) -> String {
    let list = paths.iter().map(|p| quote(p)).collect::<Vec<_>>().join(", ");
    let add = if dry { format!("$null = Get-Command Add-MpPreference; $null = @({list})") } else { format!("Add-MpPreference -ExclusionPath @({list})") };
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         $out = {out}\n\
         try {{\n\
         \x20   {add}\n\
         \x20   $now = @((Get-MpPreference).ExclusionPath)\n\
         \x20   @{{ ok = $true; excluded = $now }} | ConvertTo-Json -Compress | Set-Content -LiteralPath $out -Encoding UTF8\n\
         }} catch {{\n\
         \x20   @{{ ok = $false; message = $_.Exception.Message }} | ConvertTo-Json -Compress | Set-Content -LiteralPath $out -Encoding UTF8\n\
         }}\n",
        out = quote(out),
    )
}

/// The script, for the player to read before saying yes.
#[tauri::command]
pub fn defender_script(app: AppHandle, kind: String, game: Option<String>) -> Result<String, String> {
    let paths = targets(&app, &kind, game.as_deref())?;
    Ok(script(&paths, "%TEMP%\\kryoto-defender.json", false))
}

#[cfg(windows)]
fn powershell(args: &[&str]) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"])
        .args(args)
        .creation_flags(0x0800_0000)
        .output()
}

/// The state, read-only: no prompt, nothing changed.
#[tauri::command(async)]
pub fn defender_status(app: AppHandle) -> DefenderStatus {
    let folders = folders(&app);
    let drives = whole_drives(&folders);
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut status = DefenderStatus { folders, drives, ..Default::default() };
    #[cfg(windows)]
    {
        status.remembered = remembered(&app);
        const READ: &str = "$ErrorActionPreference = 'SilentlyContinue'; \
            $p = Get-MpPreference; $s = Get-MpComputerStatus; \
            $av = @(Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntivirusProduct | ForEach-Object displayName); \
            [pscustomobject]@{ module = [bool](Get-Command Get-MpPreference); excluded = @($p.ExclusionPath); realtime = $s.RealTimeProtectionEnabled; mode = [string]$s.AMRunningMode; av = $av } | ConvertTo-Json -Compress";
        if let Ok(out) = powershell(&["-Command", READ]) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                read_status(&v, &mut status);
            }
        }
    }
    status
}

#[cfg_attr(not(windows), allow(dead_code))]
fn read_status(v: &serde_json::Value, status: &mut DefenderStatus) {
    status.supported = v["module"].as_bool().unwrap_or(false);
    status.realtime = v["realtime"].as_bool();
    // "Normal" is Defender doing the scanning; "Passive Mode" or "SxS Passive
    // Mode" means another antivirus is.
    status.passive = v["mode"].as_str().is_some_and(|m| !m.is_empty() && !m.eq_ignore_ascii_case("normal"));
    status.others = v["av"]
        .as_array()
        .map(|a| a.iter().filter_map(|n| n.as_str()).filter(|n| !n.to_ascii_lowercase().contains("defender")).map(str::to_string).collect())
        .unwrap_or_default();
    let excluded: Vec<String> = v["excluded"].as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default();
    // Without administrator Windows answers with one line saying so.
    status.excluded = if excluded.iter().any(|e| e.starts_with("N/A") || e.to_ascii_lowercase().contains("administrator")) { None } else { Some(excluded) };
}

/// Add the exclusions, through one elevated PowerShell after a UAC prompt.
#[tauri::command(async)]
pub fn defender_exclude(app: AppHandle, kind: String, game: Option<String>) -> Result<ExcludeResult, String> {
    let paths = targets(&app, &kind, game.as_deref())?;
    #[cfg(not(windows))]
    {
        let _ = (app, paths);
        Err("Only on Windows.".into())
    }
    #[cfg(windows)]
    {
        use tauri::Manager;
        let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("defender");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let out = dir.join("result.json");
        let file = dir.join("exclude.ps1");
        let _ = std::fs::remove_file(&out);
        // Debug builds can run the whole path harmlessly: unelevated and
        // without the Add-MpPreference call, so a test never changes the
        // owner's Defender.
        let dry = cfg!(debug_assertions) && std::env::var_os("KRYOTO_DEV_DEFENDER_DRY").is_some();
        // With BOM: Windows PowerShell 5.1 reads a file without one as ANSI.
        let body = script(&paths, &out.to_string_lossy(), dry);
        std::fs::write(&file, [&[0xEF, 0xBB, 0xBF][..], body.as_bytes()].concat()).map_err(|e| e.to_string())?;
        let file_arg = file.to_string_lossy().to_string();
        let launched = if dry {
            powershell(&["-File", &file_arg])
        } else {
            // Start-Process -Verb RunAs is the UAC prompt; saying no throws.
            let launch = format!(
                "try {{ $p = Start-Process -FilePath powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', {}); exit $p.ExitCode }} catch {{ Write-Output ('declined: ' + $_.Exception.Message); exit 1223 }}",
                quote(&format!("\"{file_arg}\""))
            );
            powershell(&["-Command", &launch])
        };
        let _ = std::fs::remove_file(&file);
        let launched = launched.map_err(|e| format!("PowerShell did not start: {e}"))?;
        let result = std::fs::read(&out).ok();
        let _ = std::fs::remove_file(&out);
        let outcome = outcome(launched.status.code(), &String::from_utf8_lossy(&launched.stdout), result.as_deref());
        if outcome.ok {
            remember(&app, &paths);
        }
        crate::logging::info("defender", &format!("exclusions for {kind} ({} paths): {}", paths.len(), if outcome.ok { "added".to_string() } else { outcome.reason.clone().unwrap_or_default() }));
        Ok(outcome)
    }
}

/// What a run came to, from its exit code, its output and its result file.
#[cfg_attr(not(windows), allow(dead_code))]
fn outcome(code: Option<i32>, stdout: &str, result: Option<&[u8]>) -> ExcludeResult {
    if code == Some(1223) || stdout.trim_start().starts_with("declined") {
        return ExcludeResult { reason: Some("declined".into()), message: Some("The administrator prompt was declined.".into()), ..Default::default() };
    }
    let Some(v) = result.and_then(|b| serde_json::from_slice::<serde_json::Value>(strip_bom(b)).ok()) else {
        return ExcludeResult { reason: Some("failed".into()), message: Some("Windows did not say how it went.".into()), ..Default::default() };
    };
    if v["ok"].as_bool() == Some(true) {
        let excluded: Vec<String> = v["excluded"].as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default();
        // Read without administrator (the dry run): Windows hides the list.
        let hidden = excluded.iter().any(|e| e.starts_with("N/A"));
        return ExcludeResult { ok: true, excluded: (!hidden).then_some(excluded), ..Default::default() };
    }
    // Tamper Protection, Group Policy or Intune managing Defender, or Defender
    // switched off for another antivirus: Windows refuses the change.
    let message = v["message"].as_str().unwrap_or("Windows refused the change.").trim().to_string();
    ExcludeResult { reason: Some("blocked".into()), message: Some(message), ..Default::default() }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn strip_bom(b: &[u8]) -> &[u8] {
    b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_says_exclusions_went_in() {
        assert!(said_added("2026-11-05 21:44:40Z INFO  defender: exclusions for folders (2 paths): added\n"));
        assert!(said_added("x\r\n2026-11-05 21:44:40Z INFO  defender: exclusions for drives (1 paths): added\r\n"));
        assert!(!said_added("2026-11-05 21:44:40Z INFO  defender: exclusions for folders (2 paths): failed (declined)\n"));
    }

    #[test]
    fn the_script_quotes_every_path_and_reports_back() {
        let s = script(&["D:\\Kryoto Games".into(), "E:\\It's games".into()], "C:\\t\\r.json", false);
        assert!(s.contains("Add-MpPreference -ExclusionPath @('D:\\Kryoto Games', 'E:\\It''s games')\n"));
        assert!(s.contains("$out = 'C:\\t\\r.json'"));
        let dry = script(&["D:\\G".into()], "x", true);
        assert!(!dry.contains("Add-MpPreference -ExclusionPath") && dry.contains("Get-Command Add-MpPreference"));
    }

    #[test]
    fn drives_come_from_the_folders() {
        assert_eq!(drives(&["d:\\Games".into(), "D:\\More".into(), "E:\\K".into(), "\\\\nas\\share".into()]), ["D:\\", "E:\\"]);
    }

    #[test]
    fn outcomes_are_told_apart() {
        assert_eq!(outcome(Some(1223), "declined: The operation was canceled by the user.", None).reason.as_deref(), Some("declined"));
        let ok = outcome(Some(0), "", Some(b"\xEF\xBB\xBF{\"ok\":true,\"excluded\":[\"D:\\\\Games\"]}"));
        assert!(ok.ok);
        assert_eq!(ok.excluded.unwrap(), ["D:\\Games"]);
        let blocked = outcome(Some(0), "", Some(b"{\"ok\":false,\"message\":\"Operation failed with the following error: 0x800106ba\"}"));
        assert_eq!(blocked.reason.as_deref(), Some("blocked"));
        assert_eq!(outcome(Some(1), "", None).reason.as_deref(), Some("failed"));
    }

    #[test]
    fn exclusions_hidden_without_administrator_read_as_unknown() {
        let mut s = DefenderStatus::default();
        read_status(&serde_json::json!({ "module": true, "excluded": ["N/A: Must be an administrator to view exclusions"], "realtime": true, "mode": "Normal", "av": ["Windows Defender"] }), &mut s);
        assert!(s.supported && s.excluded.is_none() && !s.passive && s.others.is_empty());
        read_status(&serde_json::json!({ "module": true, "excluded": ["D:\\Games"], "realtime": false, "mode": "Passive Mode", "av": ["Windows Defender", "Avast Antivirus"] }), &mut s);
        assert_eq!(s.excluded.as_deref(), Some(&["D:\\Games".to_string()][..]));
        assert!(s.passive);
        assert_eq!(s.others, ["Avast Antivirus"]);
    }
}
