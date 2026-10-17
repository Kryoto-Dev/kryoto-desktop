//! Achievements saved to the kryo.to account (kryo.to
//! `lib/cloud-achievements.ts`), a supporter perk.
//!
//! Like the remote (remote.rs), the Store view's page script does the
//! talking, signed in like any browser: every half minute it asks
//! `cloud_achievements_pending` for unlocks not yet saved and games not
//! fetched lately, sends the first to kryo.to and fetches the second, and
//! hands both answers to `cloud_achievements_report`. That marks what kryo.to
//! now holds, and puts saved achievements this PC lacks back into the game's
//! gbe_fork save, so the game itself (and the Library) has them again.
//!
//! A ledger in the app's data folder remembers what was saved and whether
//! the account has the perk; without it, nothing is sent for a few hours.

use kryoto_repair::achievements::{self as ach, Roots};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, Webview};

use crate::achievements::{appid_of, roots_for, tracking_of, Tracking};
use crate::library::{LibraryGame, Running};

/// How long a "no perk" answer stands before asking again.
const PERK_RECHECK_SECS: u64 = 6 * 3600;
/// How often one game's saved achievements are fetched again.
const REFETCH_SECS: u64 = 6 * 3600;
/// Per round, so one exchange stays small.
const MAX_GAMES: usize = 25;

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ledger {
    /// The account's answer: has the perk, has not, or not asked yet.
    perk: Option<bool>,
    perk_checked: u64,
    /// Whose ledger this is: another account signing in starts afresh.
    account: Option<String>,
    /// By AppID, the achievements kryo.to has.
    saved: BTreeMap<String, BTreeSet<String>>,
    /// By AppID, when its saved achievements were last fetched.
    fetched: BTreeMap<String, u64>,
    last_sync: Option<u64>,
}

static LOCK: Mutex<()> = Mutex::new(());

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn ledger_path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_data_dir().ok()?.join("achievements-cloud.json"))
}

fn load(app: &AppHandle) -> Ledger {
    ledger_path(app).and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn store(app: &AppHandle, l: &Ledger) {
    let Some(p) = ledger_path(app) else { return };
    if let Ok(json) = serde_json::to_vec(l) {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CloudUnlock {
    name: String,
    unlocked_at: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CloudGame {
    appid: String,
    slug: Option<String>,
    title: String,
    unlocks: Vec<CloudUnlock>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    /// Unlocks kryo.to does not have yet (empty without the perk).
    games: Vec<CloudGame>,
    /// Games whose saved achievements to fetch.
    fetch: Vec<String>,
}

/// The games that can have achievements here: an AppID, and an emulator that
/// keeps them (not Kryoto Online).
fn tracked(app: &AppHandle) -> Vec<(LibraryGame, String)> {
    crate::library::load(app)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|g| {
            let appid = appid_of(Path::new(&g.install_dir))?;
            (tracking_of(&g).0 != Tracking::Unsupported).then_some((g, appid))
        })
        .collect()
}

/// Where this game saves: the PC's own folders on Windows, its prefix on Linux.
fn roots(app: &AppHandle, game: &LibraryGame) -> Roots {
    if cfg!(windows) {
        return Roots::from_env();
    }
    crate::library::plan_for(app, game, None).map(|(plan, _)| roots_for(&plan.env)).unwrap_or_else(|_| Roots::from_env())
}

/// What to send and what to fetch this round. Only the Store's kryo.to page.
#[tauri::command]
pub fn cloud_achievements_pending(app: AppHandle, webview: Webview, account: Option<String>) -> Result<Pending, String> {
    crate::remote::from_store(&app, &webview)?;
    let _g = LOCK.lock().map_err(|_| "busy")?;
    let mut ledger = load(&app);
    if account.is_some() && ledger.account != account {
        ledger = Ledger { account, ..Default::default() };
        store(&app, &ledger);
    }
    let t = now();
    let refused = ledger.perk == Some(false) && t.saturating_sub(ledger.perk_checked) < PERK_RECHECK_SECS;
    let mut games = Vec::new();
    let mut fetch = Vec::new();
    for (game, appid) in tracked(&app) {
        if fetch.len() < MAX_GAMES && t.saturating_sub(*ledger.fetched.get(&appid).unwrap_or(&0)) > REFETCH_SECS {
            fetch.push(appid.clone());
        }
        if refused || games.len() >= MAX_GAMES {
            continue;
        }
        let saved = ledger.saved.get(&appid);
        let new: Vec<CloudUnlock> = ach::read_unlocked(&roots(&app, &game), &appid)
            .into_iter()
            .filter(|u| saved.is_none_or(|s| !s.contains(&u.name)))
            .map(|u| CloudUnlock { name: u.name, unlocked_at: u.time })
            .collect();
        if !new.is_empty() {
            games.push(CloudGame { appid, slug: game.slug.clone(), title: game.title.clone(), unlocks: new });
        }
    }
    Ok(Pending { games, fetch })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// kryo.to's answer on the perk, when it gave one.
    perk: Option<bool>,
    /// What was sent, and which of those games kryo.to took.
    #[serde(default)]
    sent: Vec<CloudGame>,
    #[serde(default)]
    saved: Vec<String>,
    /// Saved achievements, by AppID, for the games fetched.
    #[serde(default)]
    fetched: HashMap<String, Vec<CloudUnlock>>,
}

/// kryo.to's answers: mark what it holds, and put back what this PC lacks.
#[tauri::command]
pub fn cloud_achievements_report(app: AppHandle, webview: Webview, running: State<'_, Running>, report: Report) -> Result<usize, String> {
    crate::remote::from_store(&app, &webview)?;
    let _g = LOCK.lock().map_err(|_| "busy")?;
    let mut ledger = load(&app);
    let t = now();
    if let Some(p) = report.perk {
        ledger.perk = Some(p);
        ledger.perk_checked = t;
    }
    for g in report.sent.iter().filter(|g| report.saved.contains(&g.appid)) {
        ledger.saved.entry(g.appid.clone()).or_default().extend(g.unlocks.iter().map(|u| u.name.clone()));
        ledger.last_sync = Some(t);
    }
    let running: Vec<String> = running.0.lock().map(|r| r.keys().cloned().collect()).unwrap_or_default();
    let mut restored = 0;
    let games = tracked(&app);
    for (appid, unlocks) in &report.fetched {
        ledger.fetched.insert(appid.clone(), t);
        ledger.saved.entry(appid.clone()).or_default().extend(unlocks.iter().map(|u| u.name.clone()));
        let Some((game, _)) = games.iter().find(|(_, a)| a == appid) else { continue };
        // Never under a running game's feet: it would write over the file.
        if running.contains(&game.id) || unlocks.is_empty() {
            continue;
        }
        let roots = roots(&app, game);
        let have: BTreeSet<String> = ach::read_unlocked(&roots, appid).into_iter().map(|u| u.name).collect();
        let missing: Vec<&CloudUnlock> = unlocks.iter().filter(|u| !have.contains(&u.name)).collect();
        if missing.is_empty() {
            continue;
        }
        let Some(dir) = roots.app_data.as_ref().map(|a| a.join("GSE Saves").join(appid)) else { continue };
        match restore(&dir, &missing) {
            Ok(n) if n > 0 => {
                restored += n;
                crate::logging::info("achievements", &format!("{}: {n} restored from the account", game.title));
                let _ = app.emit(
                    "notify",
                    crate::downloads::Notice::new(
                        &format!("{n} achievement{} restored", if n == 1 { "" } else { "s" }),
                        &format!("{} has the achievements saved to your kryo.to account.", game.title),
                        Some(game.id.clone()),
                    ),
                );
            }
            Ok(_) => {}
            Err(e) => crate::logging::warn("achievements", &format!("{}: could not restore: {e}", game.title)),
        }
    }
    if !report.fetched.is_empty() || !report.saved.is_empty() {
        ledger.last_sync = Some(t);
    }
    store(&app, &ledger);
    let _ = app.emit("cloud-achievements", status_of(&ledger));
    Ok(restored)
}

/// Add unlocks to gbe_fork's save (`GSE Saves/<appid>/achievements.json`),
/// keeping everything already in it. Returns how many were added.
fn restore(dir: &Path, missing: &[&CloudUnlock]) -> Result<usize, String> {
    let file = dir.join(ach::GBE_FILE);
    let mut doc: serde_json::Map<String, serde_json::Value> = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice(String::from_utf8_lossy(&b).trim_start_matches('\u{feff}').as_bytes()).ok())
        .unwrap_or_default();
    let mut added = 0;
    for u in missing {
        let earned = doc.get(&u.name).and_then(|v| v.get("earned")).is_some_and(|e| e.as_bool() == Some(true) || e.as_u64() == Some(1));
        if earned {
            continue;
        }
        doc.insert(u.name.clone(), serde_json::json!({ "earned": true, "earned_time": u.unlocked_at.unwrap_or(0) }));
        added += 1;
    }
    if added == 0 {
        return Ok(0);
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
    Ok(added)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Has the perk, has not, or not known yet (never synced).
    perk: Option<bool>,
    /// Games with achievements on the account.
    games: usize,
    last_sync: Option<u64>,
}

fn status_of(l: &Ledger) -> Status {
    Status { perk: l.perk, games: l.saved.values().filter(|s| !s.is_empty()).count(), last_sync: l.last_sync }
}

/// For the Library's achievements card.
#[tauri::command]
pub fn cloud_achievements_status(app: AppHandle) -> Status {
    status_of(&load(&app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kryoto_repair::achievements::Unlock;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryo-cloud-ach-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn restore_adds_what_is_missing_and_keeps_the_rest() {
        let d = dir("merge");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("achievements.json"), r#"{"A":{"earned":true,"earned_time":5},"B":{"earned":false,"earned_time":0},"STAT":{"x":1}}"#).unwrap();
        let b = CloudUnlock { name: "B".into(), unlocked_at: Some(1_700_000_000) };
        let a = CloudUnlock { name: "A".into(), unlocked_at: Some(9) };
        let c = CloudUnlock { name: "C".into(), unlocked_at: None };
        assert_eq!(restore(&d, &[&a, &b, &c]).unwrap(), 2);
        let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(d.join("achievements.json")).unwrap()).unwrap();
        assert_eq!(doc["A"]["earned_time"], 5, "an earned one keeps its own time");
        assert_eq!(doc["B"], serde_json::json!({ "earned": true, "earned_time": 1_700_000_000u64 }));
        assert_eq!(doc["C"]["earned"], true);
        assert_eq!(doc["STAT"]["x"], 1, "anything else in the file stays");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restore_makes_the_save_when_there_is_none() {
        let d = dir("fresh");
        let x = CloudUnlock { name: "X".into(), unlocked_at: Some(1_700_000_000) };
        assert_eq!(restore(&d, &[&x]).unwrap(), 1);
        assert_eq!(restore(&d, &[&x]).unwrap(), 0, "twice adds nothing");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restored_saves_read_back_as_unlocked() {
        let base = dir("readback");
        let save = base.join("GSE Saves").join("620");
        let x = CloudUnlock { name: "ACH.WAKE_UP".into(), unlocked_at: Some(1_700_000_000) };
        restore(&save, &[&x]).unwrap();
        let got = ach::read_unlocked(&Roots { app_data: Some(base.clone()), public_documents: None }, "620");
        assert_eq!(got, vec![Unlock { name: "ACH.WAKE_UP".into(), time: Some(1_700_000_000) }]);
        let _ = std::fs::remove_dir_all(&base);
    }
}
