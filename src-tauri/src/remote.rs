//! Kryoto Desktop, run from the website (kryo.to `lib/desktop-remote.ts`).
//!
//! A game queued on kryo.to ("Download in Kryoto Desktop") starts here, and
//! every download's progress shows on kryo.to/remote, from any device. Since
//! protocol 3 the library goes along too (the games, which are running), and
//! kryo.to/remote and Kryoto Mobile away from home can start and stop games,
//! clear finished downloads and take games off the PC: the same things the
//! phone does on this network (phone.rs), through kryo.to. The Store view's page script does the talking, signed in like any
//! browser (BROWSER_STATE_SCRIPT in lib.rs): it takes what kryo.to queued and
//! hands it to `remote_apply`, and sends `remote_snapshot` back while
//! something moves. Only a kryo.to page in the Store may call either.
//!
//! Remote is opt-in per account (kryo.to `lib/remote-consent.ts`). Until the
//! signed-in account has turned it on, nothing here runs: no beat, no
//! snapshot, no command. The answer comes with the account (`/api/auth/me`,
//! `set_consent`) and is forgotten on sign-out; kryo.to refusing a beat turns
//! it off at once (`remote_consent_off`). A phone paired on this network
//! (phone.rs) is not Remote and does not depend on it.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, Webview};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::io::Write;

use crate::downloads::{self, Downloads};
use crate::{library, phone};

/// What this build understands from kryo.to. 3: library, play/stop, clear, remove.
pub const PROTOCOL: u8 = 3;

/// How often the Store page is asked to take what kryo.to queued and report back.
const BEAT: std::time::Duration = std::time::Duration::from_secs(3);

/// Remote is on for the account signed in to the Store's kryo.to page. Off
/// until kryo.to says otherwise, so a start, a sign-in or a wake from sleep
/// never turns it on by itself.
static ALLOWED: AtomicBool = AtomicBool::new(false);
/// The account has answered (on or off), so Settings > Phone asks no more.
static DECIDED: AtomicBool = AtomicBool::new(false);
static SIGNED_IN: AtomicBool = AtomicBool::new(false);

pub const OFF: &str = "Remote is off for this account.";

pub fn allowed() -> bool {
    ALLOWED.load(Ordering::SeqCst)
}

/// What Settings > Phone shows about Remote.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentStatus {
    signed_in: bool,
    enabled: bool,
    decided: bool,
}

fn status() -> ConsentStatus {
    ConsentStatus { signed_in: SIGNED_IN.load(Ordering::SeqCst), enabled: allowed(), decided: DECIDED.load(Ordering::SeqCst) }
}

/// The account's answer (enabled, decided), from the Store page's account
/// report (lib.rs). None is signed out: off, whatever the last account said.
pub fn set_consent<R: tauri::Runtime>(app: &AppHandle<R>, account: Option<(bool, bool)>) {
    store_consent(account);
    let _ = tauri::Emitter::emit_to(app, "main", "remote-consent", status());
}

fn store_consent(account: Option<(bool, bool)>) {
    let (enabled, decided) = account.unwrap_or((false, false));
    SIGNED_IN.store(account.is_some(), Ordering::SeqCst);
    DECIDED.store(decided, Ordering::SeqCst);
    let was = ALLOWED.swap(enabled, Ordering::SeqCst);
    if was != enabled {
        crate::logging::info("remote", if enabled { "Remote is on for this account" } else { "Remote is off" });
        if !enabled {
            *PHONE_SEEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
}

/// Settings > Phone: is Remote on for the signed-in account.
#[tauri::command]
pub fn remote_status() -> ConsentStatus {
    status()
}

/// kryo.to refused a beat because Remote is off (turned off on another
/// device): stop now rather than at the next account check. The page may only
/// turn it off this way; on comes with the account.
#[tauri::command(async)]
pub fn remote_consent_off(app: AppHandle, webview: Webview) -> Result<(), String> {
    from_store(&app, &webview)?;
    if SIGNED_IN.load(Ordering::SeqCst) {
        set_consent(&app, Some((false, true)));
    }
    Ok(())
}

/// Settings > Phone: turn Remote on or off for the signed-in account, on
/// kryo.to with the Store's own session. Off stops this PC at once, before
/// kryo.to answers; on waits for kryo.to. The answer comes back through
/// `remote_consent_done`, then the account is read again.
#[tauri::command]
pub fn remote_set_consent(app: AppHandle, enabled: bool) -> Result<(), String> {
    let view = crate::store(&app)?;
    if !view.url().is_ok_and(|u| crate::is_kryoto(&u, &app)) {
        return Err("Open kryo.to in the Store first.".into());
    }
    if !SIGNED_IN.load(Ordering::SeqCst) {
        return Err("Sign in to kryo.to first.".into());
    }
    if !enabled {
        set_consent(&app, Some((false, true)));
    }
    view.eval(format!(
        "(() => {{ const done = (r) => {{ try {{ window.__TAURI_INTERNALS__.invoke('remote_consent_done', {{ result: r }}); }} catch (_) {{}} }}; \
         fetch('/api/account/remote', {{ method: 'POST', credentials: 'include', headers: {{ 'content-type': 'application/json' }}, body: JSON.stringify({{ enabled: {enabled} }}) }}) \
           .then((res) => res.json().catch(() => ({{}})).then((data) => done({{ ok: res.ok, error: res.ok ? null : String((data && data.error) || 'kryo.to did not save that.') }}))) \
           .catch(() => done({{ ok: false, error: 'Could not reach kryo.to.' }})) \
           .finally(() => window.__kryoDesktopRefresh && window.__kryoDesktopRefresh()); }})()"
    ))
    .map_err(|e| e.to_string())
}

/// kryo.to's answer to `remote_set_consent`, passed on to Settings.
#[tauri::command]
pub fn remote_consent_done(app: AppHandle, webview: Webview, result: serde_json::Value) -> Result<(), String> {
    from_store(&app, &webview)?;
    let ok = result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let error = result.get("error").and_then(|v| v.as_str()).map(|e| e.chars().take(200).collect::<String>());
    tauri::Emitter::emit_to(&app, "main", "remote-consent-done", serde_json::json!({ "ok": ok, "error": error })).map_err(|e| e.to_string())
}

/// The Remote loop's beat, kept here rather than in a timer on the Store page.
/// The Store is hidden whenever the Library, Settings or the tray is showing,
/// and the browser slows a hidden page's timers down to about one a minute:
/// a phone's Play then took 15 seconds to arrive and its Stop 30 (presence.rs
/// is native for the same reason). Running a script in the page is not slowed,
/// so the page's own `__kryoRemoteTick` does the talking, on this beat.
pub fn start<R: tauri::Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(BEAT).await;
            // Off: nothing is asked of the page, so nothing goes to kryo.to.
            if !allowed() {
                continue;
            }
            let Ok(view) = crate::store(&app) else { continue };
            if view.url().is_ok_and(|u| crate::is_kryoto(&u, &app)) {
                let _ = view.eval("window.__kryoRemoteTick&&window.__kryoRemoteTick()");
            }
        }
    });
}
/// Games sent along, most recently played first.
const MAX_GAMES: usize = 150;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteItem {
    id: String,
    slug: Option<String>,
    title: String,
    status: String,
    received: u64,
    total: Option<u64>,
    speed: u64,
    extracted: u64,
    extract_total: Option<u64>,
    error: Option<String>,
    queue_order: u64,
    added_at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    protocol: u8,
    install_id: String,
    name: String,
    version: String,
    os: String,
    downloads: Vec<RemoteItem>,
    games: Vec<phone::PhoneGame>,
    running: Vec<String>,
}

/// A Remote call from the Store, with Remote on.
fn remote_from_store(app: &AppHandle, webview: &Webview) -> Result<(), String> {
    from_store(app, webview)?;
    if !allowed() {
        return Err(OFF.into());
    }
    Ok(())
}

pub(crate) fn from_store(app: &AppHandle, webview: &Webview) -> Result<(), String> {
    if webview.label() != crate::STORE {
        return Err("Only the Store may ask.".into());
    }
    let page = webview.url().map_err(|e| e.to_string())?;
    if !crate::settings::is_catalog_origin(&page, &crate::settings::load(app)) {
        return Err("Only kryo.to may ask.".into());
    }
    Ok(())
}

/// When Kryoto Mobile last reached this PC through kryo.to, in Unix ms, as the
/// site said in its answer to the last report. Shown in Settings > Phone.
pub static PHONE_SEEN: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);

/// The Store page passing on kryo.to's answer about a phone (see the report in
/// lib.rs). Only from the Store's own kryo.to page.
#[tauri::command(async)]
pub fn remote_phone_seen(app: AppHandle, webview: Webview, seen_at: Option<f64>) -> Result<(), String> {
    remote_from_store(&app, &webview)?;
    let seen = seen_at.filter(|t| t.is_finite() && *t > 0.0).map(|t| t as u64);
    *PHONE_SEEN.lock().unwrap_or_else(|e| e.into_inner()) = seen;
    Ok(())
}

/// What the downloads are doing, for kryo.to's Downloads page. Finished ones
/// are left out after a day, so the list is about now.
#[tauri::command(async)]
pub fn remote_snapshot(app: AppHandle, webview: Webview, state: State<'_, Downloads>) -> Result<Snapshot, String> {
    remote_from_store(&app, &webview)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let downloads = downloads::downloads_list(state)
        .into_iter()
        .filter(|d| d.finished_at.is_none_or(|f| now.saturating_sub(f) < 86_400))
        .take(60)
        .map(|d| RemoteItem {
            title: if d.meta.title.is_empty() { downloads::title_from_file(&d.file_name) } else { d.meta.title.clone() },
            status: serde_json::to_value(d.status).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
            id: d.id,
            slug: d.slug,
            received: d.received,
            total: d.total,
            speed: d.speed,
            extracted: d.extracted,
            extract_total: d.extract_total,
            error: d.error,
            queue_order: d.queue_order,
            added_at: d.added_at,
        })
        .collect();
    Ok(Snapshot {
        protocol: PROTOCOL,
        games: phone::games(&app, Some(MAX_GAMES)),
        running: library::game_running(app.state::<library::Running>()),
        install_id: crate::logging::install_id(&app),
        name: sysinfo::System::host_name().unwrap_or_else(|| "Kryoto Desktop".into()),
        version: app.package_info().version.to_string(),
        os: format!("{} {}", std::env::consts::OS, sysinfo::System::os_version().unwrap_or_default()).trim().to_string(),
        downloads,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCommand {
    id: String,
    kind: String,
    url: Option<String>,
    slug: Option<String>,
    title: Option<String>,
    download_id: Option<String>,
    #[serde(default)]
    game_id: Option<String>,
    /// A release only on mirrors: their pages, for a download sent without `url`.
    #[serde(default)]
    mirrors: Vec<String>,
    /// That release's version, so it installs as that build.
    #[serde(default)]
    version: Option<String>,
}

fn game_id_of(c: &RemoteCommand) -> Result<String, String> {
    c.game_id.clone().filter(|i| !i.is_empty() && i.len() <= 80).ok_or_else(|| "Invalid game ID.".to_string())
}

/// Do what kryo.to queued: start a download (from kryo.to's own filehost only),
/// or pause, resume or cancel one. Confirm each durable acceptance separately.
#[tauri::command(async)]
pub fn remote_apply(app: AppHandle, webview: Webview, state: State<'_, Downloads>, commands: Vec<RemoteCommand>) -> Result<Vec<RemoteResult>, String> {
    remote_from_store(&app, &webview)?;
    let _guard = APPLY.lock().map_err(|_| "remote command lock poisoned")?;
    let settings = crate::settings::load(&app);
    let file = app.path().app_data_dir().map_err(|e| e.to_string())?.join("remote-receipts.json");
    let mut receipts = read_receipts(&file)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs();
    receipts.retain(|_, at| now.saturating_sub(*at) < 30 * 86400);
    let mut results = Vec::new();
    for c in commands.into_iter().take(20) {
        let id = c.id.clone();
        if id.is_empty() || id.len() > 19 || !id.bytes().all(|b| b.is_ascii_digit()) {
            results.push(RemoteResult { id, error: Some("Invalid remote command ID.".into()), retryable: false });
            continue;
        }
        // Asked again right before each one: Remote turned off while these
        // were on their way means none of the rest is carried out.
        if !allowed() {
            results.push(RemoteResult { id, error: Some(OFF.into()), retryable: false });
            continue;
        }
        let accepted = accept_once(&file, &mut receipts, &id, now, || -> Result<(), String> { match c.kind.as_str() {
            "download" => {
                let slug = c.slug.filter(|s| !s.is_empty() && s.len() <= 160 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
                let title = c.title.map(|t| t.chars().filter(|ch| !ch.is_control()).take(200).collect::<String>()).filter(|t| !t.is_empty());
                crate::logging::info("remote", &format!("queued from kryo.to: {}", title.as_deref().unwrap_or("a game")));
                if c.url.is_none() && !c.mirrors.is_empty() {
                    // Only on mirrors: the first one this app can download
                    // from, the way its own Store would.
                    let page = c.mirrors.iter().find(|m| crate::resolvers::classify(m).is_some()).ok_or("None of this game's mirrors can be downloaded from here. Open its page in the Store.")?;
                    let version = c.version.filter(|v| !v.is_empty() && v.len() <= 120);
                    downloads::enqueue_remote_mirror(&app, &id, page.clone(), slug, title, version)?;
                    return Ok(());
                }
                let url = c.url.as_deref().and_then(|u| url::Url::parse(u).ok()).ok_or("Invalid download URL.")?;
                if !downloads::is_ours(&url, &settings) {
                    return Err("This download does not come from Kryoto.".into());
                }
                downloads::enqueue_remote(&app, &id, url.to_string(), slug, title)?;
            }
            "pause" | "resume" | "cancel" => {
                let download_id = c.download_id.filter(|i| !i.is_empty() && i.len() <= 80).ok_or("Invalid download ID.")?;
                let current = downloads::snapshot(&app, &download_id).ok_or("That download is no longer in the list.")?;
                if current.status == downloads::Status::Installed {
                    return Err("That download has already finished transferring.".into());
                }
                match c.kind.as_str() {
                    "pause" => {
                        downloads::download_pause(app.clone(), state.clone(), download_id.clone());
                        downloads::remote_control(&app, &download_id, downloads::Status::Paused)?;
                    }
                    "resume" => {
                        downloads::download_resume(app.clone(), download_id)?;
                        downloads::persist_checked(&app)?;
                    }
                    _ => {
                        downloads::download_cancel(app.clone(), state.clone(), download_id.clone());
                        downloads::remote_control(&app, &download_id, downloads::Status::Canceled)?;
                    }
                }
            }
            // Protocol 3: what the phone does on this network, through kryo.to.
            "clear" => {
                let download_id = c.download_id.clone().filter(|i| !i.is_empty() && i.len() <= 80).ok_or("Invalid download ID.")?;
                phone::control_download(&app, &download_id, "remove")?;
            }
            "play" => {
                let game = game_id_of(&c)?;
                if !library::load(&app)?.iter().any(|g| g.id == game) {
                    return Err("That game is no longer in the library.".into());
                }
                crate::logging::info("remote", &format!("starting {game} from kryo.to"));
                let app = app.clone();
                // Launching can take a while (a first start writes the
                // achievement list); the command is accepted once it is known
                // to be a library game, and a failure goes to the log.
                tauri::async_runtime::spawn(async move {
                    let entry = library::default_entry(&app, &game);
                    if let Err(e) = library::game_launch(app.clone(), app.state::<library::Running>(), game, entry, None).await {
                        crate::logging::error("remote", &format!("could not start it: {e}"));
                    }
                });
            }
            "stop" => {
                let game = game_id_of(&c)?;
                library::game_stop(app.state::<library::Running>(), game)?;
            }
            "uninstall" | "unlist" => {
                let game = game_id_of(&c)?;
                phone::check_removable(&app, &game)?;
                let delete_files = c.kind == "uninstall";
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = phone::remove_game(&app, &game, delete_files).await {
                        crate::logging::error("remote", &format!("could not remove it: {e}"));
                    }
                });
            }
            _ => return Err("Unknown remote action.".into()),
        } Ok(()) });
        let error = accepted.err();
        if let Some(e) = &error {
            receipts.remove(&id);
            crate::logging::error("remote", e);
        }
        let retryable = error.as_deref().is_none_or(retryable_error);
        results.push(RemoteResult { id, error, retryable });
    }
    Ok(results)
}

static APPLY: Mutex<()> = Mutex::new(());

fn retryable_error(error: &str) -> bool {
    !matches!(error, "Invalid remote command ID." | "Invalid download URL." | "Invalid download ID." |
        "This download does not come from Kryoto." | "That download is no longer in the list." |
        "That download has already finished transferring." | "Unknown remote action." |
        "Invalid game ID." | "That game is no longer in the library." | "Stop the game first." | "Cancel it first." | OFF)
}

fn read_receipts(file: &std::path::Path) -> Result<BTreeMap<String, u64>, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("Remote receipts could not be read: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(format!("Remote receipts could not be read: {e}")),
    }
}

fn accept_once(file: &std::path::Path, receipts: &mut BTreeMap<String, u64>, id: &str, now: u64, apply: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    if receipts.contains_key(id) { return Ok(()); }
    apply()?;
    receipts.insert(id.into(), now);
    let saved = (|| -> Result<(), String> {
        let tmp = file.with_extension("json.tmp");
        let mut output = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        output.write_all(&serde_json::to_vec(receipts).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        std::fs::rename(tmp, file).map_err(|e| e.to_string())
    })();
    if let Err(e) = saved {
        receipts.remove(id);
        return Err(format!("Could not save remote receipt: {e}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_targets_are_permanent_but_persistence_and_settling_are_retryable() {
        assert!(!retryable_error("That download is no longer in the list."));
        assert!(!retryable_error("This download does not come from Kryoto."));
        assert!(retryable_error("Could not save remote receipt: access denied"));
        assert!(retryable_error("Wait for the download to stop, then try again."));
        assert!(!retryable_error(OFF));
    }

    #[test]
    fn remote_is_off_until_the_account_says_on_and_off_again_on_sign_out() {
        assert!(!allowed());
        store_consent(Some((false, false)));
        assert!(!allowed());
        store_consent(Some((true, true)));
        assert!(allowed());
        assert!(status().decided);
        // Signed out (or a new account whose answer is not known yet): off.
        store_consent(None);
        assert!(!allowed());
        assert!(!status().signed_in && !status().decided);
    }

    #[test]
    fn durable_receipt_recognizes_a_retry_after_restart() {
        let dir = std::env::temp_dir().join(format!("kryoto-receipts-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("receipts.json");
        let mut receipts = read_receipts(&file).unwrap();
        let mut applied = 0;
        accept_once(&file, &mut receipts, "123", 1, || { applied += 1; Ok(()) }).unwrap();
        let mut restarted = read_receipts(&file).unwrap();
        accept_once(&file, &mut restarted, "123", 2, || { applied += 1; Ok(()) }).unwrap();
        assert_eq!(applied, 1);
        assert_eq!(restarted.get("123"), Some(&1));
        assert!(accept_once(&file, &mut restarted, "124", 3, || Err("native rejection".into())).is_err());
        assert!(!read_receipts(&file).unwrap().contains_key("124"));
        let unwritable = dir.join("missing-parent/receipts.json");
        assert!(accept_once(&unwritable, &mut restarted, "125", 4, || Ok(())).is_err());
        assert!(!restarted.contains_key("125"));
        std::fs::write(&file, "damaged receipt").unwrap();
        assert!(read_receipts(&file).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[derive(Serialize)]
pub struct RemoteResult {
    id: String,
    error: Option<String>,
    retryable: bool,
}
