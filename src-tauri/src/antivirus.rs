//! When an antivirus acts on a game: what it did, said plainly, and the way
//! back (the folder exclusion in defender.rs, and the files again).
//!
//! Nothing here guesses. Each finding says how it is known:
//!
//! - Windows refused to start a file as a threat (`ERROR_VIRUS_INFECTED`,
//!   225, at the start; `STATUS_VIRUS_INFECTED`, 0xC0000906, as the exit):
//!   Windows says an antivirus did it (library.rs, useLibrary.ts).
//! - Windows Security's own log (Microsoft-Windows-Windows Defender/
//!   Operational, events 1116 and 1117), which a standard user can read:
//!   it names each file it found in the game's folder, the threat and what it
//!   did (quarantined, removed, blocked). Only that is called Windows Security.
//! - Files Kryoto installed that are gone now, from the release's own record
//!   (`.kryoto-release.json`, community_offer.rs) and, for what went missing
//!   while the game was unpacking, the archive's list (`unpack_losses`).
//!   Without a log entry those are only "missing": another antivirus may have
//!   taken them, or the player did, and the page does not say which.
//!
//! Files a repair replaced or removed (`.kryoto-repair`) are not missing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Runtime};

use crate::community_offer::{read_json, ReleaseRecord, RELEASE_RECORD};

/// The kinds of file an antivirus takes from a game: programs, libraries and scripts.
const WATCHED: [&str; 11] = ["exe", "dll", "asi", "ocx", "sys", "com", "scr", "bat", "cmd", "vbs", "ps1"];

fn watched(rel: &str) -> bool {
    rel.rsplit('.').next().is_some_and(|ext| WATCHED.iter().any(|w| ext.eq_ignore_ascii_case(w)))
}

/// One thing Windows Security's log says it did to a file in the game's folder.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    /// The file, relative to the game's folder when it is in it.
    pub file: String,
    /// What Windows Security called it ("Trojan:Win32/Wacatac.B!ml").
    pub threat: String,
    /// What it did: "Quarantine", "Remove", "Block", or empty while it is deciding.
    pub action: String,
    /// When, in seconds since 1970.
    pub at: u64,
}

/// What `game_antivirus` found for one game.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Files of the installed release that are gone, relative to its folder (the first few).
    pub missing: Vec<String>,
    pub missing_count: usize,
    /// Windows Security's log for this folder, newest first.
    pub hits: Vec<Hit>,
    /// The game is from kryo.to, so its files can be fetched again.
    pub can_restore: bool,
}

/// The release record's programs that are no longer in `dir`, leaving out
/// what a repair changed. Pure apart from the file checks, for tests.
pub fn missing_files(dir: &Path) -> Vec<String> {
    let Some(record) = read_json::<ReleaseRecord>(&dir.join(RELEASE_RECORD)) else { return Vec::new() };
    let repaired: BTreeSet<String> = kryoto_repair::journal::load(dir)
        .ok()
        .flatten()
        .filter(|j| j.state != "undone")
        .map(|j| j.entries.into_iter().map(|e| e.path.replace('\\', "/").to_ascii_lowercase()).collect())
        .unwrap_or_default();
    record
        .files
        .keys()
        .filter(|rel| watched(rel) && !repaired.contains(&rel.to_ascii_lowercase()))
        .filter(|rel| !dir.join(rel).is_file())
        .cloned()
        .collect()
}

/// `rel` was part of the release Kryoto installed in `dir`.
pub fn was_installed(dir: &Path, rel: &str) -> bool {
    let rel = rel.replace('\\', "/");
    read_json::<ReleaseRecord>(&dir.join(RELEASE_RECORD)).is_some_and(|r| r.files.keys().any(|k| k.eq_ignore_ascii_case(&rel)))
}

/// The programs in a .7z that did not arrive in `dest` after unpacking, relative
/// to it, with their sizes. An archive that wraps everything in one folder was
/// unpacked without it (downloads.rs `single_child_dir`), so that folder is cut
/// off. Other archives, and one already gone (unpacked in place), say nothing.
pub fn unpack_losses(archive: &Path, dest: &Path) -> Vec<(String, u64)> {
    if !archive.extension().is_some_and(|e| e.eq_ignore_ascii_case("7z")) {
        return Vec::new();
    }
    let Ok(a) = sevenz_rust::Archive::open(archive) else { return Vec::new() };
    let files: Vec<(String, u64)> = a.files.iter().filter(|f| !f.is_directory()).map(|f| (f.name().replace('\\', "/"), f.size())).collect();
    let top = |n: &str| n.split('/').next().unwrap_or_default().to_string();
    let first = files.first().map(|(n, _)| top(n)).unwrap_or_default();
    let wrapped = !first.is_empty() && files.iter().all(|(n, _)| n.contains('/') && top(n) == first);
    files
        .into_iter()
        .map(|(n, size)| (if wrapped { n[first.len() + 1..].to_string() } else { n }, size))
        .filter(|(rel, _)| watched(rel) && kryoto_repair::community::safe_path(rel) && !dest.join(rel).is_file())
        .collect()
}

/// Windows Security's log for files under `dir` since `since` (seconds since
/// 1970). Read-only, with no administrator: the Operational log is readable by
/// the signed-in user. Empty elsewhere, or when the log cannot be read.
pub fn defender_hits(dir: &Path, since: u64) -> Vec<Hit> {
    #[cfg(windows)]
    {
        let start = since.saturating_sub(3600);
        let script = format!(
            "$ErrorActionPreference = 'SilentlyContinue'\n\
             $since = [DateTimeOffset]::FromUnixTimeSeconds({start}).LocalDateTime\n\
             $out = @(Get-WinEvent -FilterHashtable @{{ LogName = 'Microsoft-Windows-Windows Defender/Operational'; Id = 1116, 1117; StartTime = $since }} -MaxEvents 400 | ForEach-Object {{\n\
             \x20 $d = @{{}}; ([xml]$_.ToXml()).Event.EventData.Data | ForEach-Object {{ $d[$_.Name] = [string]$_.'#text' }}\n\
             \x20 [pscustomobject]@{{ id = $_.Id; at = [DateTimeOffset]::new($_.TimeCreated).ToUnixTimeSeconds(); path = $d['Path']; threat = $d['Threat Name']; action = $d['Action Name'] }}\n\
             }})\n\
             ConvertTo-Json -InputObject $out -Compress -Depth 3\n"
        );
        use std::os::windows::process::CommandExt;
        let Ok(out) = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
            .creation_flags(0x0800_0000)
            .output()
        else {
            return Vec::new();
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return Vec::new() };
        hits_in(&v, dir)
    }
    #[cfg(not(windows))]
    {
        let _ = (dir, since);
        Vec::new()
    }
}

/// The log's entries that name a file under `dir`, one per file (its latest),
/// newest first. Pure, for tests.
pub fn hits_in(events: &serde_json::Value, dir: &Path) -> Vec<Hit> {
    let root = dir.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
    let mut out: Vec<Hit> = Vec::new();
    let list: Vec<&serde_json::Value> = match events {
        serde_json::Value::Array(a) => a.iter().collect(),
        serde_json::Value::Object(_) => vec![events],
        _ => Vec::new(),
    };
    for e in list {
        let at = e["at"].as_u64().unwrap_or(0);
        // 1116 is "found", 1117 "acted": the action only counts on the second.
        let acted = e["id"].as_u64() == Some(1117);
        for path in e["path"].as_str().unwrap_or_default().split(';') {
            // `file:_C:\Games\x.dll`, `containerfile:_C:\Games\x.zip`, sometimes with `->` into an archive.
            let path = path.split("->").next().unwrap_or_default();
            let path = path.split_once(":_").map(|(_, p)| p).unwrap_or(path).trim();
            let lower = path.replace('/', "\\").to_ascii_lowercase();
            if lower.is_empty() || !(lower.starts_with(&format!("{root}\\")) || lower == root) {
                continue;
            }
            let file = path[root.len()..].trim_start_matches(['\\', '/']).replace('\\', "/");
            let action = if acted { e["action"].as_str().unwrap_or_default().to_string() } else { String::new() };
            let hit = Hit { file: file.clone(), threat: e["threat"].as_str().unwrap_or_default().to_string(), action, at };
            match out.iter_mut().find(|h| h.file.eq_ignore_ascii_case(&file)) {
                Some(h) if at >= h.at => {
                    let keep = std::mem::take(&mut h.action);
                    *h = Hit { action: if hit.action.is_empty() { keep } else { hit.action }, ..hit };
                }
                Some(_) => {}
                None => out.push(hit),
            }
        }
    }
    out.sort_by_key(|h| std::cmp::Reverse(h.at));
    out
}

/// What an antivirus did to this game, for its page. `deep` also reads
/// Windows Security's log when nothing is missing (after Windows refused to
/// start a file as a threat); otherwise only missing files make it look,
/// since reading the log starts PowerShell.
#[tauri::command(async)]
pub fn game_antivirus(app: AppHandle, id: String, deep: bool) -> Result<Report, String> {
    let game = crate::library::load(&app)?.into_iter().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
    if game.launcher.is_some() || game.install_dir.is_empty() {
        return Ok(Report::default());
    }
    let dir = PathBuf::from(&game.install_dir);
    let missing = missing_files(&dir);
    let since = read_json::<ReleaseRecord>(&dir.join(RELEASE_RECORD)).map(|r| r.at).unwrap_or(game.added_at);
    let hits = if cfg!(windows) && (deep || !missing.is_empty()) { defender_hits(&dir, since) } else { Vec::new() };
    Ok(Report { missing_count: missing.len(), missing: missing.into_iter().take(8).collect(), hits, can_restore: game.slug.is_some() })
}

/// Games whose next download of their own build is to put files back, not a
/// second press of Download (downloads.rs `already_installed`).
static RESTORING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// "Get the files back": the next download of this game's build installs over
/// its folder, which puts back what is missing and leaves saves and settings.
#[tauri::command]
pub fn game_restore_files<R: Runtime>(app: AppHandle<R>, id: String) -> Result<String, String> {
    let game = crate::library::load(&app)?.into_iter().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
    let slug = game.slug.ok_or("This game is not from kryo.to, so Kryoto cannot fetch its files again. Reinstall it from where you got it.")?;
    if let Ok(mut list) = RESTORING.lock() {
        if !list.contains(&slug) {
            list.push(slug.clone());
        }
    }
    crate::logging::info("antivirus", &format!("{}: getting its files back", game.title));
    Ok(slug)
}

/// A download of this game is putting its files back.
pub fn restoring(slug: &str) -> bool {
    RESTORING.lock().is_ok_and(|l| l.iter().any(|s| s == slug))
}

/// Its files are back (the install finished).
pub fn restored(slug: &str) {
    if let Ok(mut l) = RESTORING.lock() {
        l.retain(|s| s != slug);
    }
}

/// After an install: programs that never arrived in the folder. Said as a
/// notice with what Windows Security's log says about them, if anything.
pub fn after_install<R: Runtime>(app: &AppHandle<R>, game_id: &str, title: &str, dir: &Path, lost: &[(String, u64)], started: u64) {
    if lost.is_empty() {
        return;
    }
    crate::logging::warn("antivirus", &format!("{title}: {} file(s) did not arrive after unpacking: {}", lost.len(), lost.iter().take(5).map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")));
    let (app, id, title, dir, first) = (app.clone(), game_id.to_string(), title.to_string(), dir.to_path_buf(), lost[0].0.clone());
    let count = lost.len();
    std::thread::spawn(move || {
        let hits = defender_hits(&dir, started);
        let body = lost_notice(count, &first, &hits);
        use tauri::Emitter;
        let _ = app.emit("notify", crate::downloads::Notice::new(&format!("{title} is missing files"), &body, Some(id)));
    });
}

/// The notice's words: Windows Security only when its log says so.
fn lost_notice(count: usize, first: &str, hits: &[Hit]) -> String {
    let name = first.rsplit('/').next().unwrap_or(first);
    let what = if count == 1 { name.to_string() } else { format!("{name} and {} more", count - 1) };
    if !hits.is_empty() {
        format!("Windows Security removed {what} as the game unpacked. Open the game to exclude its folder and get the files back.")
    } else {
        format!("{what} did not arrive in the game's folder. Open the game to get the files back.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_read_for_this_folder_only() {
        let events = serde_json::json!([
            { "id": 1116, "at": 100, "path": "file:_D:\\Kryoto Games\\Hades\\steam_api64.dll", "threat": "Trojan:Win32/Wacatac.B!ml", "action": "" },
            { "id": 1117, "at": 105, "path": "file:_D:\\Kryoto Games\\Hades\\steam_api64.dll", "threat": "Trojan:Win32/Wacatac.B!ml", "action": "Quarantine" },
            { "id": 1117, "at": 90, "path": "file:_D:\\Kryoto Games\\Hades II\\x.exe", "threat": "HackTool:Win32/Crack", "action": "Remove" },
            { "id": 1117, "at": 80, "path": "containerfile:_D:\\Kryoto Games\\Hades\\bin\\a.zip;file:_D:\\Kryoto Games\\Hades\\bin\\a.zip->(Zip)", "threat": "PUA", "action": "Remove" }
        ]);
        let hits = hits_in(&events, Path::new("D:\\Kryoto Games\\Hades"));
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0], Hit { file: "steam_api64.dll".into(), threat: "Trojan:Win32/Wacatac.B!ml".into(), action: "Quarantine".into(), at: 105 });
        assert_eq!(hits[1].file, "bin/a.zip");
        // One event comes back as an object, not a list.
        let one = serde_json::json!({ "id": 1117, "at": 1, "path": "file:_D:\\G\\a.dll", "threat": "t", "action": "Block" });
        assert_eq!(hits_in(&one, Path::new("d:/G/")).len(), 1);
        assert!(hits_in(&serde_json::json!(null), Path::new("D:\\G")).is_empty());
    }

    #[test]
    fn missing_programs_are_found_and_repairs_are_not_missing() {
        let dir = std::env::temp_dir().join(format!("kryoto-av-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        for f in ["bin/Game.exe", "bin/steam_api64.dll", "data.pak"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let record = ReleaseRecord { files: kryoto_repair::community::stamp_tree(&dir), ..Default::default() };
        std::fs::write(dir.join(RELEASE_RECORD), serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(missing_files(&dir).is_empty());
        std::fs::remove_file(dir.join("bin/steam_api64.dll")).unwrap();
        std::fs::remove_file(dir.join("data.pak")).unwrap();
        assert_eq!(missing_files(&dir), ["bin/steam_api64.dll"], "only programs count");
        assert!(was_installed(&dir, "bin\\steam_api64.dll") && !was_installed(&dir, "bin/other.dll"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_notice_names_windows_security_only_on_its_word() {
        assert!(lost_notice(1, "bin/steam_api64.dll", &[]).starts_with("steam_api64.dll did not arrive"));
        let hit = Hit { file: "bin/steam_api64.dll".into(), ..Default::default() };
        assert!(lost_notice(3, "bin/steam_api64.dll", &[hit]).starts_with("Windows Security removed steam_api64.dll and 2 more"));
    }
}
