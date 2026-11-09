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
//!
//! With an achievements server of the player's own (Settings > Patchbay),
//! the Store page sends kryo.to nothing: `patchbay/achievements.rs` does the
//! same exchange with that server from here, through `pending_for` and
//! `apply`, on a ledger of its own.
//!
//! Kept on this PC instead (Settings > Cloud), for everyone: the same
//! exchange with a file in the player's own folder (`local_round`), so
//! achievements survive reinstalling a game or the whole library.

use kryoto_repair::achievements::{self as ach, Roots};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, Webview};

use crate::achievements::{appid_of, tracking_of, Tracking};
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

/// Where achievements are kept.
pub(crate) enum Target {
    /// The kryo.to account, through the Store page (a supporter perk).
    Account,
    /// `achievements.json` in a folder on this PC.
    Local(PathBuf),
    /// The player's own server (Settings > Patchbay).
    Own(crate::patchbay::Own),
}

/// Settings' choice (`achievements_mode`); before there was one, an own server
/// set in Patchbay was used whenever there was one.
pub(crate) fn target<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Target {
    let s = crate::settings::load(app);
    match s.achievements_mode.as_str() {
        "local" if !s.saves_local_dir.trim().is_empty() => Target::Local(PathBuf::from(s.saves_local_dir.trim())),
        "account" => Target::Account,
        _ => crate::patchbay::achievements(app).map(Target::Own).unwrap_or(Target::Account),
    }
}

/// The ledger's name for the place it is kept with.
fn ledger_key(t: &Target) -> Option<String> {
    match t {
        Target::Account => None,
        Target::Local(dir) => Some(format!("local-{}", crate::patchbay::ledger_key(&dir.to_string_lossy()))),
        Target::Own(own) => Some(crate::patchbay::ledger_key(&own.base)),
    }
}

fn ledger_path(app: &AppHandle) -> Option<PathBuf> {
    let name = match ledger_key(&target(app)) {
        Some(key) => format!("achievements-cloud-{key}.json"),
        None => "achievements-cloud.json".into(),
    };
    Some(app.path().app_data_dir().ok()?.join(name))
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
    pub(crate) name: String,
    pub(crate) unlocked_at: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CloudGame {
    pub(crate) appid: String,
    slug: Option<String>,
    title: String,
    unlocks: Vec<CloudUnlock>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    /// Unlocks kryo.to does not have yet (empty without the perk).
    pub(crate) games: Vec<CloudGame>,
    /// Games whose saved achievements to fetch.
    pub(crate) fetch: Vec<String>,
}

/// The games that can have achievements here: an AppID, and an emulator that
/// keeps them (not Kryoto Online).
fn tracked(app: &AppHandle) -> Vec<(LibraryGame, String)> {
    crate::library::load(app)
        .unwrap_or_default()
        .into_iter()
        // Another launcher keeps its own games' achievements.
        .filter(|g| g.launcher.is_none())
        .filter_map(|g| {
            let appid = appid_of(Path::new(&g.install_dir))?;
            (tracking_of(&g).0 != Tracking::Unsupported).then_some((g, appid))
        })
        .collect()
}

/// Where this game saves: the PC's own folders on Windows, its prefix on Linux.
fn roots(app: &AppHandle, game: &LibraryGame) -> Roots {
    crate::achievements::roots_of(app, game)
}

/// What to send and what to fetch this round. Only the Store's kryo.to page.
///
/// Off the UI thread, like everything the Store page calls on a timer. This
/// opens files in every tracked game's folder every 30 s; on the window's
/// thread, a download keeping a hard disk busy turned that into freezes of
/// several seconds, twice a minute.
#[tauri::command(async)]
pub fn cloud_achievements_pending(app: AppHandle, webview: Webview, account: Option<String>) -> Result<Pending, String> {
    crate::remote::from_store(&app, &webview)?;
    // Kept on this PC or the player's own server instead: kryo.to gets nothing.
    if !matches!(target(&app), Target::Account) {
        return Ok(Pending { games: Vec::new(), fetch: Vec::new() });
    }
    pending_for(&app, account)
}

/// What to send and fetch this round, for whichever server the ledger is for.
pub(crate) fn pending_for(app: &AppHandle, account: Option<String>) -> Result<Pending, String> {
    let app = app.clone();
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

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// kryo.to's answer on the perk, when it gave one.
    pub(crate) perk: Option<bool>,
    /// What was sent, and which of those games kryo.to took.
    #[serde(default)]
    pub(crate) sent: Vec<CloudGame>,
    #[serde(default)]
    pub(crate) saved: Vec<String>,
    /// Saved achievements, by AppID, for the games fetched.
    #[serde(default)]
    pub(crate) fetched: HashMap<String, Vec<CloudUnlock>>,
}

/// kryo.to's answers: mark what it holds, and put back what this PC lacks.
#[tauri::command(async)]
pub fn cloud_achievements_report(app: AppHandle, webview: Webview, running: State<'_, Running>, report: Report) -> Result<usize, String> {
    crate::remote::from_store(&app, &webview)?;
    if !matches!(target(&app), Target::Account) {
        return Ok(0);
    }
    apply(&app, &running, report)
}

/// A server's answers: mark what it holds, and put back what this PC lacks.
pub(crate) fn apply(app: &AppHandle, running: &Running, report: Report) -> Result<usize, String> {
    let app = app.clone();
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
        ledger.saved.entry(appid.clone()).or_default().extend(unlocks.iter().map(|u| u.name.clone()));
        let Some((game, _)) = games.iter().find(|(_, a)| a == appid) else {
            ledger.fetched.insert(appid.clone(), t);
            continue;
        };
        // Never under a running game's feet: it would write over the file. Not
        // marked as fetched either, so the next round restores them once it stops.
        if running.contains(&game.id) {
            continue;
        }
        ledger.fetched.insert(appid.clone(), t);
        if unlocks.is_empty() {
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
                crate::logging::info("achievements", &format!("{}: {n} restored", game.title));
                let _ = app.emit(
                    "notify",
                    crate::downloads::Notice::new(
                        &format!("{n} achievement{} restored", if n == 1 { "" } else { "s" }),
                        &format!("{} has its achievements back from {}.", game.title, where_kept(&app)),
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
    let _ = app.emit("cloud-achievements", with_place(&app, status_of(&ledger)));
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
    /// The player's own server (Settings > Patchbay), by host; none for kryo.to.
    endpoint: Option<String>,
    /// Where they are kept: `account`, `local` or `patchbay`.
    mode: &'static str,
    /// `local`: the folder.
    local_dir: Option<String>,
}

fn status_of(l: &Ledger) -> Status {
    Status { perk: l.perk, games: l.saved.values().filter(|s| !s.is_empty()).count(), last_sync: l.last_sync, endpoint: None, mode: "account", local_dir: None }
}

fn with_place(app: &AppHandle, s: Status) -> Status {
    match target(app) {
        Target::Account => s,
        Target::Local(dir) => Status { mode: "local", local_dir: Some(dir.to_string_lossy().into_owned()), perk: Some(true), ..s },
        Target::Own(own) => Status { mode: "patchbay", endpoint: Some(own.host), ..s },
    }
}

/// Where they are kept, for a sentence.
fn where_kept(app: &AppHandle) -> String {
    match target(app) {
        Target::Account => "your kryo.to account".into(),
        Target::Local(dir) => dir.to_string_lossy().into_owned(),
        Target::Own(own) => own.host,
    }
}

pub(crate) fn current_status(app: &AppHandle) -> Status {
    with_place(app, status_of(&load(app)))
}

/* -- kept on this PC ---------------------------------------------------- */

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LocalGame {
    title: String,
    unlocks: Vec<CloudUnlock>,
}

const LOCAL_FILE: &str = "achievements.json";

/// Unlocks into the folder's store, as a server's POST does: each name once,
/// its first time kept. Returns the AppIDs taken. Pure, for the tests.
fn local_take(store: &mut BTreeMap<String, LocalGame>, games: &[CloudGame]) -> Vec<String> {
    let mut saved = Vec::new();
    for g in games {
        if g.appid.is_empty() || g.appid.len() > 10 || !g.appid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let entry = store.entry(g.appid.clone()).or_default();
        if entry.title.is_empty() {
            entry.title = g.title.clone();
        }
        for u in &g.unlocks {
            if !entry.unlocks.iter().any(|x| x.name == u.name) {
                entry.unlocks.push(u.clone());
            }
        }
        saved.push(g.appid.clone());
    }
    saved
}

/// One round with the folder: what this PC unlocked goes in, what the folder
/// has comes back to games that lack it (a reinstall, another copy). No
/// network at all.
pub(crate) fn local_round(app: &AppHandle, dir: &Path) -> Result<(), String> {
    let account = ledger_key(&Target::Local(dir.to_path_buf()));
    let pending = pending_for(app, account)?;
    if pending.games.is_empty() && pending.fetch.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not use {}: {e}", dir.display()))?;
    let file = dir.join(LOCAL_FILE);
    let mut store: BTreeMap<String, LocalGame> = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let saved = local_take(&mut store, &pending.games);
    if !saved.is_empty() {
        crate::saves::write_json(&file, &store)?;
    }
    let fetched = pending.fetch.iter().map(|id| (id.clone(), store.get(id).map(|g| g.unlocks.clone()).unwrap_or_default())).collect();
    let report = Report { perk: Some(true), sent: pending.games, saved, fetched };
    let running = app.state::<Running>();
    apply(app, &running, report).map(|_| ())
}

/// For the Library's achievements card.
#[tauri::command(async)]
pub fn cloud_achievements_status(app: AppHandle) -> Status {
    current_status(&app)
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
    fn kept_on_this_pc_each_unlock_once_with_its_first_time() {
        let unlock = |name: &str, at: u64| CloudUnlock { name: name.into(), unlocked_at: Some(at) };
        let game = |appid: &str, unlocks: Vec<CloudUnlock>| CloudGame { appid: appid.into(), slug: None, title: "Hades".into(), unlocks };
        let mut store = BTreeMap::new();
        assert_eq!(local_take(&mut store, &[game("1145360", vec![unlock("A", 10), unlock("B", 20)])]), vec!["1145360".to_string()]);
        // Again from another copy: B is not added twice, nor its time moved.
        local_take(&mut store, &[game("1145360", vec![unlock("B", 99), unlock("C", 30)])]);
        let names: Vec<_> = store["1145360"].unlocks.iter().map(|u| (u.name.as_str(), u.unlocked_at)).collect();
        assert_eq!(names, vec![("A", Some(10)), ("B", Some(20)), ("C", Some(30))]);
        // Only Steam AppIDs: nothing that could be a path.
        assert!(local_take(&mut store, &[game("../x", vec![unlock("A", 1)])]).is_empty());
        assert_eq!(store.len(), 1);
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
        let got = ach::read_unlocked(&Roots { app_data: Some(base.clone()), ..Default::default() }, "620");
        assert_eq!(got, vec![Unlock { name: "ACH.WAKE_UP".into(), time: Some(1_700_000_000) }]);
        let _ = std::fs::remove_dir_all(&base);
    }
}
