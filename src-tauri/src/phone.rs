//! The phone companion (Kryoto Mobile): this PC as something a phone can see
//! the library of and start games on.
//!
//! Two ways in, both carrying the same `PhoneSnapshot`:
//! - On this network: a small HTTP server (`serve`), paired once with a
//!   6-digit code, then a bearer token the phone keeps. Only runs while
//!   Settings > Phone > "On this network" is on.
//!
//! The phone also manages this PC from here: `control_download` (pause, resume,
//! cancel, clear) and `remove_game` (uninstall, or only unlist) are shared by
//! both ways in. It never sees install paths, launch options or custom art:
//! only what `PhoneGame` and `PhoneDownload` hold.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::State as AxumState;
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State};

use crate::achievements::{game_achievements, GameAchievement};
use crate::downloads::{self, Downloads};
use crate::{library, logging, settings};

const FIRST_PORT: u16 = 47821;
const LAST_PORT: u16 = 47830;
const CODE_LIFETIME: Duration = Duration::from_secs(10 * 60);
const MAX_FAILURES: u8 = 5;
/// `lastSeen` is written at most this often per phone.
const TOUCH_EVERY: u64 = 60;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneGame {
    pub id: String,
    pub title: String,
    pub slug: Option<String>,
    pub cover: Option<String>,
    pub header: Option<String>,
    pub playtime_seconds: u64,
    pub last_played: Option<u64>,
    pub added_at: u64,
    pub nsfw: bool,
}

/// One of the PC's downloads (the fields Downloads shows, plus
/// art and what it is waiting on).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneDownload {
    pub id: String,
    pub slug: Option<String>,
    pub title: String,
    pub status: String,
    pub received: u64,
    pub total: Option<u64>,
    pub speed: u64,
    pub extracted: u64,
    pub extract_total: Option<u64>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub queue_order: u64,
    pub added_at: u64,
    /// The library game this download became, once installed: what Play starts.
    pub game_id: Option<String>,
    pub cover: Option<String>,
    pub nsfw: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneSnapshot {
    pub protocol: u8,
    pub install_id: String,
    pub name: String,
    pub version: String,
    pub os: String,
    pub games: Vec<PhoneGame>,
    pub running: Vec<String>,
    pub downloads: Vec<PhoneDownload>,
    /// Who is signed in to kryo.to on this PC, for a phone that is not.
    pub profile: Option<PhoneProfile>,
    /// The look this PC is using now, which the phone follows.
    pub look: Option<PhoneLook>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneProfile {
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub supporter: bool,
    pub kryos: Option<i64>,
    /// How many games are on each of the account's kryo.to lists, by status.
    pub counts: BTreeMap<String, u32>,
}

/// The PC's resolved look: what Kryoto Desktop stamps on its own window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneLook {
    pub theme: String,
    pub palette: String,
    pub radius: String,
    pub font: String,
    #[serde(default)]
    pub season: Option<String>,
    #[serde(default)]
    pub reduced_motion: bool,
}

static PROFILE: Mutex<Option<PhoneProfile>> = Mutex::new(None);
static LOOK: Mutex<Option<PhoneLook>> = Mutex::new(None);

/// A short name made of letters, digits and dashes, else nothing.
fn token(v: &str) -> Option<String> {
    (!v.is_empty() && v.len() <= 24 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then(|| v.to_string())
}

/// Kryoto Desktop's own page tells us who is signed in (lib.rs `report_catalog_state`).
/// The same account reporting again keeps the list counts it had.
pub fn set_account(account: Option<PhoneProfile>) {
    let Ok(mut slot) = PROFILE.lock() else { return };
    *slot = account.map(|mut next| {
        next.avatar_url = https_only(next.avatar_url);
        next.counts = slot.as_ref().filter(|p| p.username == next.username).map(|p| p.counts.clone()).unwrap_or_default();
        next
    });
}

/// The account's saved games (`{ entries: [{ status }] }`), counted by list.
pub fn set_saved(saved: &serde_json::Value) {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for entry in saved["entries"].as_array().into_iter().flatten() {
        if let Some(status) = entry["status"].as_str().and_then(token) {
            *counts.entry(status).or_default() += 1;
        }
    }
    if let Ok(mut slot) = PROFILE.lock() {
        if let Some(p) = slot.as_mut() {
            p.counts = counts;
        }
    }
}

/// The window reports its resolved look whenever it changes.
#[tauri::command(async)]
pub fn phone_report_look(look: PhoneLook) {
    let clean = PhoneLook {
        theme: token(&look.theme).unwrap_or_else(|| "dark".into()),
        palette: token(&look.palette).unwrap_or_else(|| "monochrome".into()),
        radius: token(&look.radius).unwrap_or_else(|| "pill".into()),
        font: token(&look.font).unwrap_or_else(|| "teletext".into()),
        season: look.season.as_deref().and_then(token),
        reduced_motion: look.reduced_motion,
    };
    if let Ok(mut slot) = LOOK.lock() {
        *slot = Some(clean);
    }
}

/// Art the phone can fetch itself. A local file or a custom picture is not.
fn https_only(url: Option<String>) -> Option<String> {
    url.filter(|u| u.starts_with("https://"))
}

fn to_phone(g: library::LibraryGame) -> PhoneGame {
    PhoneGame {
        id: g.id,
        title: g.title,
        slug: g.slug,
        cover: https_only(g.cover),
        header: https_only(g.header),
        playtime_seconds: g.playtime_seconds,
        last_played: g.last_played,
        added_at: g.added_at,
        nsfw: g.nsfw,
    }
}

/// The library as the phone sees it. With `cap`, the most recently played
/// (then most recently added) `cap` games.
pub fn games(app: &AppHandle, cap: Option<usize>) -> Vec<PhoneGame> {
    let mut list: Vec<PhoneGame> = match library::library_list(app.clone()) {
        Ok(list) => list.into_iter().map(to_phone).collect(),
        Err(e) => {
            logging::error("phone", &format!("library: {e}"));
            Vec::new()
        }
    };
    if let Some(n) = cap {
        // `None` sorts below any `Some`, so reversing puts never-played last.
        list.sort_by(|a, b| b.last_played.cmp(&a.last_played).then(b.added_at.cmp(&a.added_at)));
        list.truncate(n);
    }
    list
}

pub fn snapshot(app: &AppHandle, cap: Option<usize>) -> PhoneSnapshot {
    PhoneSnapshot {
        protocol: 1,
        install_id: logging::install_id(app),
        name: sysinfo::System::host_name().unwrap_or_else(|| "Kryoto Desktop".into()),
        version: app.package_info().version.to_string(),
        os: format!("{} {}", std::env::consts::OS, sysinfo::System::os_version().unwrap_or_default()).trim().to_string(),
        games: games(app, cap),
        running: library::game_running(app.state::<library::Running>()),
        downloads: phone_downloads(app),
        profile: PROFILE.lock().ok().and_then(|p| p.clone()),
        look: LOOK.lock().ok().and_then(|l| l.clone()),
    }
}

/// The downloads as the phone sees them: the same list Downloads shows on the
/// PC, finished ones included until they are cleared there or on the phone.
fn phone_downloads(app: &AppHandle) -> Vec<PhoneDownload> {
    downloads::downloads_list(app.state::<Downloads>())
        .into_iter()
        .take(60)
        .map(|d| PhoneDownload {
            title: if d.meta.title.is_empty() { downloads::title_from_file(&d.file_name) } else { d.meta.title.clone() },
            status: serde_json::to_value(d.status).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
            cover: https_only(d.meta.header.clone().or(d.meta.cover.clone())),
            id: d.id,
            slug: d.slug,
            received: d.received,
            total: d.total,
            speed: d.speed,
            extracted: d.extracted,
            extract_total: d.extract_total,
            error: d.error,
            notice: d.notice,
            queue_order: d.queue_order,
            added_at: d.added_at,
            game_id: d.game_id,
            nsfw: d.meta.nsfw,
        })
        .collect()
}

// ---------------------------------------------------------------- pairing

#[derive(Default)]
struct Inner {
    port: Option<u16>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    code: Option<(String, Instant)>,
    failures: u8,
}

#[derive(Default)]
pub struct PhoneState(Mutex<Inner>);

#[derive(Debug, PartialEq, Eq)]
enum CodeError {
    Wrong,
    TooMany,
}

fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|_| "The system random source failed.".to_string())?;
    Ok(b)
}

/// Six decimal digits, every value equally likely.
fn random_code() -> Result<String, String> {
    const LIMIT: u32 = 4_294_000_000; // the largest multiple of 1_000_000 below 2^32
    loop {
        let n = u32::from_le_bytes(random_bytes::<4>()?);
        if n < LIMIT {
            return Ok(format!("{:06}", n % 1_000_000));
        }
    }
}

/// Equal-length strings compared without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

impl Inner {
    fn issue_code(&mut self, now: Instant) -> Result<String, String> {
        let code = random_code()?;
        self.code = Some((code.clone(), now));
        self.failures = 0;
        Ok(code)
    }

    /// The live code, if any; an expired one is dropped.
    fn live_code(&mut self, now: Instant) -> Option<(&str, u64)> {
        if self.code.as_ref().is_some_and(|(_, at)| now.saturating_duration_since(*at) >= CODE_LIFETIME) {
            self.code = None;
        }
        self.code.as_ref().map(|(c, at)| (c.as_str(), CODE_LIFETIME.saturating_sub(now.saturating_duration_since(*at)).as_secs()))
    }

    /// A right code works once. The fifth wrong try in a row burns the code.
    fn take_code(&mut self, given: &str, now: Instant) -> Result<(), CodeError> {
        let right = self.live_code(now).is_some_and(|(c, _)| same(c, given.trim()));
        if right {
            self.code = None;
            self.failures = 0;
            return Ok(());
        }
        if self.code.is_none() {
            return Err(CodeError::Wrong);
        }
        self.failures += 1;
        if self.failures >= MAX_FAILURES {
            self.code = None;
            self.failures = 0;
            return Err(CodeError::TooMany);
        }
        Err(CodeError::Wrong)
    }
}

fn lock(state: &PhoneState) -> std::sync::MutexGuard<'_, Inner> {
    state.0.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------- devices

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Device {
    id: String,
    name: String,
    token_hash: String,
    paired_at: u64,
    last_seen: u64,
}

/// One writer at a time; the file is small and read whole.
static DEVICES: Mutex<()> = Mutex::new(());

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn devices_file(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("phone-devices.json"))
}

fn read_devices(file: &std::path::Path) -> Result<Vec<Device>, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("Paired phones could not be read: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("Paired phones could not be read: {e}")),
    }
}

fn write_devices(file: &std::path::Path, devices: &[Device]) -> Result<(), String> {
    use std::io::Write;
    let tmp = file.with_extension("json.tmp");
    let mut output = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    output.write_all(&serde_json::to_vec(devices).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    std::fs::rename(tmp, file).map_err(|e| e.to_string())
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn token_matches(stored_hash: &str, token: &str) -> bool {
    same(stored_hash, &token_hash(token))
}

fn clean_name(raw: &str) -> String {
    let name: String = raw.chars().filter(|c| !c.is_control()).collect();
    let name: String = name.trim().chars().take(60).collect();
    if name.is_empty() { "Phone".into() } else { name }
}

// ---------------------------------------------------------------- server

type Reply = (StatusCode, Json<serde_json::Value>);

fn fail(status: StatusCode, message: &str) -> Reply {
    (status, Json(serde_json::json!({ "error": message })))
}

fn parse<T: for<'a> Deserialize<'a>>(body: &str) -> Result<T, Reply> {
    serde_json::from_str(body).map_err(|_| fail(StatusCode::BAD_REQUEST, "Bad request."))
}

/// Who is asking: the paired phone whose token this is. Refreshes `lastSeen`.
fn authorize(app: &AppHandle, headers: &HeaderMap) -> Result<(), Reply> {
    let denied = || fail(StatusCode::UNAUTHORIZED, "Pair this phone again.");
    let token = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).ok_or_else(denied)?;
    let _guard = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
    let file = devices_file(app).map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    let mut devices = read_devices(&file).map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, &e))?;
    let now = now_secs();
    let device = devices.iter_mut().find(|d| token_matches(&d.token_hash, token)).ok_or_else(denied)?;
    if now.saturating_sub(device.last_seen) >= TOUCH_EVERY {
        device.last_seen = now;
        if let Err(e) = write_devices(&file, &devices) {
            logging::error("phone", &e);
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairBody {
    code: String,
    #[serde(default)]
    device_name: String,
}

#[derive(Deserialize)]
struct GameBody {
    id: String,
}

async fn pair(AxumState(app): AxumState<AppHandle>, body: String) -> Reply {
    let body: PairBody = match parse(&body) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let state = app.state::<PhoneState>();
    match lock(&state).take_code(&body.code, Instant::now()) {
        Ok(()) => {}
        Err(CodeError::Wrong) => return fail(StatusCode::UNAUTHORIZED, "Wrong code."),
        Err(CodeError::TooMany) => return fail(StatusCode::TOO_MANY_REQUESTS, "Too many tries. Make a new code on the PC."),
    }
    let made = (|| -> Result<(String, String), String> {
        Ok((hex::encode(random_bytes::<32>()?), hex::encode(random_bytes::<8>()?)))
    })();
    let Ok((token, id)) = made else { return fail(StatusCode::INTERNAL_SERVER_ERROR, "The system random source failed.") };
    let now = now_secs();
    let device = Device { id, name: clean_name(&body.device_name), token_hash: token_hash(&token), paired_at: now, last_seen: now };
    let saved = (|| -> Result<(), String> {
        let _guard = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
        let file = devices_file(&app)?;
        let mut devices = read_devices(&file)?;
        devices.push(device.clone());
        write_devices(&file, &devices)
    })();
    if let Err(e) = saved {
        logging::error("phone", &e);
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not save the pairing.");
    }
    logging::info("phone", &format!("paired: {}", device.name));
    let snap = snapshot_light(&app);
    (StatusCode::OK, Json(serde_json::json!({ "token": token, "installId": snap.0, "name": snap.1 })))
}

fn snapshot_light(app: &AppHandle) -> (String, String) {
    (logging::install_id(app), sysinfo::System::host_name().unwrap_or_else(|| "Kryoto Desktop".into()))
}

async fn get_snapshot(AxumState(app): AxumState<AppHandle>, headers: HeaderMap) -> Reply {
    if let Err(r) = authorize(&app, &headers) {
        return r;
    }
    let for_task = app.clone();
    match tauri::async_runtime::spawn_blocking(move || snapshot(&for_task, None)).await {
        Ok(s) => (StatusCode::OK, Json(serde_json::to_value(s).unwrap_or_default())),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn game_id(body: &str) -> Result<String, Reply> {
    let GameBody { id } = parse(body)?;
    if id.is_empty() || id.len() > 80 {
        return Err(fail(StatusCode::BAD_REQUEST, "Which game?"));
    }
    Ok(id)
}

async fn play(AxumState(app): AxumState<AppHandle>, headers: HeaderMap, body: String) -> Reply {
    if let Err(r) = authorize(&app, &headers) {
        return r;
    }
    let id = match game_id(&body) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let entry = library::default_entry(&app, &id);
    match library::game_launch(app.clone(), app.state::<library::Running>(), id, entry, None).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(e) => fail(StatusCode::CONFLICT, &e),
    }
}

async fn stop(AxumState(app): AxumState<AppHandle>, headers: HeaderMap, body: String) -> Reply {
    if let Err(r) = authorize(&app, &headers) {
        return r;
    }
    let id = match game_id(&body) {
        Ok(id) => id,
        Err(r) => return r,
    };
    match library::game_stop(app.state::<library::Running>(), id) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(e) => fail(StatusCode::CONFLICT, &e),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControlBody {
    id: String,
    action: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveBody {
    id: String,
    #[serde(default)]
    delete_files: bool,
}

/// Pause, resume, cancel or clear one of this PC's downloads. Shared by the
/// phone on this network and by `remote.rs` for a phone through kryo.to.
pub fn control_download(app: &AppHandle, id: &str, action: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 80 {
        return Err("Invalid download ID.".into());
    }
    let current = downloads::snapshot(app, id).ok_or("That download is no longer in the list.")?;
    let state = app.state::<Downloads>();
    match action {
        "pause" | "resume" | "cancel" if current.status == downloads::Status::Installed => {
            return Err("That download has already finished transferring.".into());
        }
        "pause" => {
            downloads::download_pause(app.clone(), state, id.to_string());
            downloads::remote_control(app, id, downloads::Status::Paused)?;
        }
        "resume" => {
            downloads::download_resume(app.clone(), id.to_string())?;
            downloads::persist_checked(app)?;
        }
        "cancel" => {
            downloads::download_cancel(app.clone(), state, id.to_string());
            downloads::remote_control(app, id, downloads::Status::Canceled)?;
        }
        // Only a row that is finished, failed or cancelled can be cleared; the list is not a way to lose a running one.
        "remove" if matches!(current.status, downloads::Status::Installed | downloads::Status::Failed | downloads::Status::Canceled) => {
            downloads::download_remove(app.clone(), state, id.to_string());
        }
        "remove" => return Err("Cancel it first.".into()),
        _ => return Err("Unknown remote action.".into()),
    }
    Ok(())
}

/// A game that can be taken out of the library right now: it is there, and not running.
pub fn check_removable(app: &AppHandle, id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 80 {
        return Err("Invalid game ID.".into());
    }
    if !library::load(app)?.iter().any(|g| g.id == id) {
        return Err("That game is no longer in the library.".into());
    }
    if library::game_running(app.state::<library::Running>()).iter().any(|r| r == id) {
        return Err("Stop the game first.".into());
    }
    Ok(())
}

/// Take a game out of the library; with `delete_files`, uninstall it too. Says
/// whether files were deleted (a game that was added from a folder Kryoto did
/// not install into is only unlisted).
pub async fn remove_game(app: &AppHandle, id: &str, delete_files: bool) -> Result<bool, String> {
    check_removable(app, id)?;
    library::library_remove(app.clone(), id.to_string(), delete_files).await
}

async fn downloads_control(AxumState(app): AxumState<AppHandle>, headers: HeaderMap, body: String) -> Reply {
    if let Err(r) = authorize(&app, &headers) {
        return r;
    }
    let ControlBody { id, action } = match parse(&body) {
        Ok(b) => b,
        Err(r) => return r,
    };
    match control_download(&app, &id, &action) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(e) => fail(StatusCode::CONFLICT, &e),
    }
}

async fn library_remove(AxumState(app): AxumState<AppHandle>, headers: HeaderMap, body: String) -> Reply {
    if let Err(r) = authorize(&app, &headers) {
        return r;
    }
    let RemoveBody { id, delete_files } = match parse(&body) {
        Ok(b) => b,
        Err(r) => return r,
    };
    match remove_game(&app, &id, delete_files).await {
        Ok(deleted) => (StatusCode::OK, Json(serde_json::json!({ "ok": true, "deleted": deleted }))),
        Err(e) => fail(StatusCode::CONFLICT, &e),
    }
}

/// One achievement as the phone shows it. Only what the PC knows; the locked
/// picture is Steam's grey one when it lists it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PhoneAchievement {
    id: String,
    name: String,
    description: String,
    icon: Option<String>,
    unlocked: bool,
    unlocked_at: Option<u64>,
    hidden: bool,
    percent: Option<f32>,
}

fn phone_achievement(a: GameAchievement) -> PhoneAchievement {
    let GameAchievement { achievement: g, unlocked, unlocked_at } = a;
    let icon = if unlocked { g.icon } else { g.icon_gray.or(g.icon) };
    PhoneAchievement {
        name: if g.display_name.is_empty() { g.name.clone() } else { g.display_name },
        id: g.name,
        description: g.description,
        icon: https_only(icon),
        unlocked,
        unlocked_at,
        hidden: g.hidden,
        percent: g.percent,
    }
}

async fn achievements(AxumState(app): AxumState<AppHandle>, headers: HeaderMap, body: String) -> Reply {
    if let Err(r) = authorize(&app, &headers) {
        return r;
    }
    let id = match game_id(&body) {
        Ok(id) => id,
        Err(r) => return r,
    };
    let list = match tokio::time::timeout(Duration::from_secs(20), game_achievements(app.clone(), id)).await {
        Ok(Ok(list)) => list,
        Ok(Err(e)) => return fail(StatusCode::CONFLICT, &e),
        Err(_) => return fail(StatusCode::CONFLICT, "The PC took too long to load achievements."),
    };
    let (unlocked, total) = (list.unlocked, list.total);
    let achievements: Vec<PhoneAchievement> = list.list.into_iter().map(phone_achievement).collect();
    (StatusCode::OK, Json(serde_json::json!({ "achievements": achievements, "unlocked": unlocked, "total": total })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlugBody {
    slug: String,
    /// kryo.to's Turnstile token, solved on the phone. Without one the PC can
    /// only send the player to the game's page.
    #[serde(default)]
    turnstile_token: Option<String>,
}

/// A kryo.to game slug: lowercase letters, digits and dashes.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.len() <= 160 && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

async fn download(AxumState(app): AxumState<AppHandle>, headers: HeaderMap, body: String) -> Reply {
    if let Err(r) = authorize(&app, &headers) {
        return r;
    }
    let SlugBody { slug, turnstile_token } = match parse(&body) {
        Ok(b) => b,
        Err(r) => return r,
    };
    if !valid_slug(&slug) {
        return fail(StatusCode::BAD_REQUEST, "Which game?");
    }
    match start_download(&app, &slug, turnstile_token.as_deref().filter(|t| !t.is_empty() && t.len() <= 4096)).await {
        Ok(title) => (StatusCode::OK, Json(serde_json::json!({ "ok": true, "title": title }))),
        Err(e) => fail(StatusCode::CONFLICT, &e),
    }
}

/// What the phone's Download does: exactly what Download does in Kryoto
/// Desktop's own Store. This PC asks kryo.to for the game's current release
/// with the account signed in here (so the same checks apply as for the
/// desktop's own press), gets the link kryo.to mints for this PC, and queues
/// it in Downloads. Returns the game's title.
pub async fn start_download(app: &AppHandle, slug: &str, turnstile_token: Option<&str>) -> Result<String, String> {
    let endpoint = settings::catalog_endpoint(&settings::load(app));
    let base = url::Url::parse(&endpoint).map_err(|e| e.to_string())?;
    let cookie = app
        .get_webview(crate::STORE)
        .and_then(|view| view.cookies_for_url(base.clone()).ok())
        .map(|jar| jar.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; "))
        .filter(|c| !c.is_empty())
        .ok_or("Sign in to kryo.to in Kryoto Desktop first.")?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;

    let listing: serde_json::Value = client
        .get(format!("{endpoint}/api/games/{slug}/downloads"))
        .send()
        .await
        .map_err(|e| format!("Could not reach kryo.to: {e}"))?
        .error_for_status()
        .map_err(|e| if e.status() == Some(reqwest::StatusCode::NOT_FOUND) { "kryo.to has no such game.".to_string() } else { format!("kryo.to answered {}.", e.status().map_or(0, |s| s.as_u16())) })?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    // The release players get now, if it is on Kryoto's own filehost.
    let release = listing["releases"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["primary"].as_bool() == Some(true) && r["archived"].as_bool() != Some(true))
        .filter(|r| r["links"].as_array().is_some_and(|l| l.iter().any(|l| l["url"].as_str().is_some_and(|u| u.starts_with("/api/download/v/")))))
        .ok_or("This game is only on mirrors. Download it in Kryoto Desktop itself.")?;
    let release_id = release["id"].as_str().filter(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit())).ok_or("kryo.to sent a release without an ID.")?;
    let title = listing["title"].as_str().unwrap_or(slug).to_string();

    let response = client
        .post(format!("{endpoint}/api/download/v/{release_id}"))
        .header(reqwest::header::COOKIE, cookie)
        .json(&match turnstile_token {
            Some(token) => serde_json::json!({ "turnstileToken": token }),
            None => serde_json::json!({}),
        })
        .send()
        .await
        .map_err(|e| format!("Could not reach kryo.to: {e}"))?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    if !status.is_success() {
        return Err(match (status.as_u16(), body["error"].as_str()) {
            (403, _) => {
                // Only a browser that renders kryo.to's challenge can pass it, so
                // this PC cannot ask for the link itself. Put the game's page
                // in front of the player on the PC: its Download button solves
                // the check, and the Store's download is queued by the app.
                let opened = crate::navigate_catalog(app.clone(), format!("/game/{slug}")).is_ok();
                if opened {
                    crate::system::show_main(app);
                }
                if opened {
                    "kryo.to wants a check only the PC's own window can pass. The game's page is open in Kryoto Desktop: press Download there.".into()
                } else {
                    "kryo.to wants a check only the PC's own window can pass. Press Download on the game's page in Kryoto Desktop.".into()
                }
            }
            (429, _) => "Too many downloads started. Wait a moment.".into(),
            (404, _) => "That build is no longer on Kryoto's filehost.".into(),
            (_, Some(text)) => text.to_string(),
            (code, None) => format!("kryo.to answered {code}."),
        });
    }
    let link = body["url"].as_str().and_then(|u| url::Url::parse(u).ok()).ok_or("kryo.to did not send a download link.")?;
    if !crate::downloads::is_ours(&link, &settings::load(app)) {
        return Err("That download does not come from Kryoto.".into());
    }
    crate::downloads::enqueue_url(app, link.to_string(), Some(slug.to_string()), Some(title.clone()))?;
    logging::info("phone", &format!("download queued from a phone: {title}"));
    Ok(title)
}

fn router(app: AppHandle) -> Router {
    Router::new()
        .route("/v1/info", get(|AxumState(app): AxumState<AppHandle>| async move {
            Json(serde_json::json!({ "protocol": 1, "installId": logging::install_id(&app) }))
        }))
        .route("/v1/pair", post(pair))
        .route("/v1/snapshot", get(get_snapshot))
        .route("/v1/play", post(play))
        .route("/v1/stop", post(stop))
        .route("/v1/download", post(download))
        .route("/v1/downloads", post(downloads_control))
        .route("/v1/library/remove", post(library_remove))
        .route("/v1/achievements", post(achievements))
        .with_state(app)
}

/// Listen on the first free port from 47821. Already running: the same port.
fn start(app: &AppHandle) -> Result<u16, String> {
    let state = app.state::<PhoneState>();
    let mut inner = lock(&state);
    if let Some(port) = inner.port {
        return Ok(port);
    }
    let mut bound = None;
    for port in FIRST_PORT..=LAST_PORT {
        match std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)) {
            Ok(l) => {
                bound = Some((l, port));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(format!("Could not listen for phones: {e}")),
        }
    }
    let (listener, port) = bound.ok_or("Ports 47821 to 47830 are all in use.")?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => return logging::error("phone", &e.to_string()),
        };
        let served = axum::serve(listener, router(app)).with_graceful_shutdown(async move {
            let _ = rx.await;
        });
        if let Err(e) = served.await {
            logging::error("phone", &e.to_string());
        }
    });
    inner.port = Some(port);
    inner.shutdown = Some(tx);
    logging::info("phone", &format!("listening on port {port}"));
    Ok(port)
}

fn stop_server(app: &AppHandle) {
    let state = app.state::<PhoneState>();
    let mut inner = lock(&state);
    if let Some(tx) = inner.shutdown.take() {
        let _ = tx.send(());
    }
    inner.port = None;
    inner.code = None;
    inner.failures = 0;
}

/// At launch: listen again if the player left it on.
pub fn start_if_enabled(app: &AppHandle) {
    if settings::load(app).phone_lan {
        if let Err(e) = start(app) {
            logging::error("phone", &e);
        }
    }
}

// ---------------------------------------------------------------- commands

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedPhone {
    id: String,
    name: String,
    paired_at: u64,
    last_seen: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneStatus {
    lan: bool,
    addresses: Vec<String>,
    code: Option<String>,
    code_expires_in: Option<u64>,
    /// What the QR code on the PC holds: this PC's addresses and the live code.
    pair_uri: Option<String>,
    devices: Vec<PairedPhone>,
}

/// A private-network address (10/8, 172.16/12, 192.168/16): where a phone on
/// the same Wi-Fi can reach this PC. Loopback, link-local and public ones are not.
fn is_lan(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

/// The address this PC would use to reach the internet: the real Wi-Fi or
/// Ethernet one, as opposed to a VirtualBox, VMware or WSL adapter that is
/// also private but that a phone cannot reach. Connecting a UDP socket only
/// picks a route; nothing is sent.
fn preferred_address() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if is_lan(v4) => Some(v4),
        _ => None,
    }
}

/// Every private address with `preferred` first, then the rest in order.
fn rank(mut found: Vec<Ipv4Addr>, preferred: Option<Ipv4Addr>, port: u16) -> Vec<String> {
    found.sort();
    found.dedup();
    if let Some(p) = preferred {
        if let Some(at) = found.iter().position(|a| *a == p) {
            let best = found.remove(at);
            found.insert(0, best);
        }
    }
    found.into_iter().map(|a| format!("{a}:{port}")).collect()
}

fn addresses(port: u16) -> Vec<String> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    let found = networks
        .values()
        .flat_map(|n| n.ip_networks().iter().map(|ip| ip.addr))
        .filter_map(|ip| match ip {
            IpAddr::V4(v4) if is_lan(v4) => Some(v4),
            _ => None,
        })
        .collect();
    rank(found, preferred_address(), port)
}

/// The QR carries a one-use LAN code, private addresses and this PC's identity
/// for authenticated account fallback when the phone cannot reach those addresses.
fn pair_uri(addresses: &[String], code: &str, install_id: &str) -> Option<String> {
    if code.is_empty() {
        return None;
    }
    let hosts: String = addresses.iter().map(|a| format!("&h={a}")).collect();
    Some(format!("kryoto://pair?c={code}{hosts}&i={install_id}"))
}

#[tauri::command(async)]
pub fn phone_status(app: AppHandle, state: State<'_, PhoneState>) -> Result<PhoneStatus, String> {
    let s = settings::load(&app);
    let (port, code) = {
        let mut inner = lock(&state);
        let code = inner.live_code(Instant::now()).map(|(c, left)| (c.to_string(), left));
        (inner.port, code)
    };
    let devices = {
        let _guard = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
        read_devices(&devices_file(&app)?)?
    };
    let addresses = port.map(addresses).unwrap_or_default();
    Ok(PhoneStatus {
        lan: s.phone_lan,
        pair_uri: code.as_ref().and_then(|(c, _)| pair_uri(&addresses, c, &logging::install_id(&app))),
        addresses,
        code_expires_in: code.as_ref().map(|(_, left)| *left),
        code: code.map(|(c, _)| c),
        devices: devices.into_iter().map(|d| PairedPhone { id: d.id, name: d.name, paired_at: d.paired_at, last_seen: d.last_seen }).collect(),
    })
}

fn persist(app: &AppHandle, change: impl FnOnce(&mut settings::Settings)) -> Result<(), String> {
    let mut s = settings::load(app);
    change(&mut s);
    settings::write(app, &s)
}

#[tauri::command(async)]
pub fn phone_set_lan(app: AppHandle, state: State<'_, PhoneState>, on: bool) -> Result<(), String> {
    if on {
        start(&app)?;
        if let Err(e) = persist(&app, |s| s.phone_lan = true) {
            stop_server(&app);
            return Err(e);
        }
        lock(&state).issue_code(Instant::now())?;
    } else {
        persist(&app, |s| s.phone_lan = false)?;
        stop_server(&app);
    }
    Ok(())
}

#[tauri::command(async)]
pub fn phone_new_code(state: State<'_, PhoneState>) -> Result<String, String> {
    let mut inner = lock(&state);
    if inner.port.is_none() {
        return Err("Turn on \"On this network\" first.".into());
    }
    inner.issue_code(Instant::now())
}

#[tauri::command(async)]
pub fn phone_revoke(app: AppHandle, id: String) -> Result<(), String> {
    let _guard = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
    let file = devices_file(&app)?;
    let mut devices = read_devices(&file)?;
    devices.retain(|d| d.id != id);
    write_devices(&file, &devices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issued(now: Instant) -> (Inner, String) {
        let mut inner = Inner::default();
        let code = inner.issue_code(now).unwrap();
        (inner, code)
    }

    #[test]
    fn code_is_six_digits() {
        for _ in 0..200 {
            let code = random_code().unwrap();
            assert_eq!(code.len(), 6);
            assert!(code.bytes().all(|b| b.is_ascii_digit()), "{code}");
        }
    }

    #[test]
    fn right_code_works_once() {
        let now = Instant::now();
        let (mut inner, code) = issued(now);
        assert_eq!(inner.take_code(&code, now), Ok(()));
        assert_eq!(inner.take_code(&code, now), Err(CodeError::Wrong));
    }

    #[test]
    fn five_wrong_tries_burn_the_code() {
        let now = Instant::now();
        let (mut inner, code) = issued(now);
        let wrong = if code == "000000" { "000001" } else { "000000" };
        for _ in 0..4 {
            assert_eq!(inner.take_code(wrong, now), Err(CodeError::Wrong));
        }
        assert_eq!(inner.take_code(wrong, now), Err(CodeError::TooMany));
        // The right code no longer works either.
        assert_eq!(inner.take_code(&code, now), Err(CodeError::Wrong));
    }

    #[test]
    fn a_right_code_resets_the_count() {
        let now = Instant::now();
        let (mut inner, code) = issued(now);
        let wrong = if code == "000000" { "000001" } else { "000000" };
        for _ in 0..4 {
            let _ = inner.take_code(wrong, now);
        }
        assert_eq!(inner.take_code(&code, now), Ok(()));
        assert_eq!(inner.failures, 0);
    }

    #[test]
    fn expired_code_is_rejected() {
        let now = Instant::now();
        let (mut inner, code) = issued(now);
        let later = now + CODE_LIFETIME;
        assert_eq!(inner.take_code(&code, later), Err(CodeError::Wrong));
        assert!(inner.live_code(later).is_none());
    }

    #[test]
    fn token_hash_accepts_the_token_and_rejects_others() {
        let stored = token_hash("a".repeat(64).as_str());
        assert!(token_matches(&stored, &"a".repeat(64)));
        assert!(!token_matches(&stored, &"b".repeat(64)));
        assert!(!token_matches(&stored, ""));
    }

    #[test]
    fn only_private_ranges_are_lan_addresses() {
        for ok in ["10.0.0.5", "172.16.0.1", "192.168.1.5"] {
            assert!(is_lan(ok.parse().unwrap()), "{ok}");
        }
        for no in ["127.0.0.1", "169.254.1.1", "8.8.8.8", "172.32.0.1"] {
            assert!(!is_lan(no.parse().unwrap()), "{no}");
        }
    }

    #[test]
    fn only_plain_slugs_reach_the_download_path() {
        assert!(valid_slug("stellar-blade"));
        assert!(valid_slug("2077"));
        for bad in ["", "Stellar-Blade", "a/b", "a b", "../x", "x?y=1", &"a".repeat(161)] {
            assert!(!valid_slug(bad), "{bad}");
        }
    }

    #[test]
    fn the_phone_gets_the_signed_in_account_with_list_counts_and_https_avatar_only() {
        let who = |avatar: &str, supporter: bool, kryos: Option<i64>| PhoneProfile { username: "sink".into(), display_name: Some("SIN.K//".into()), avatar_url: Some(avatar.into()), supporter, kryos, counts: BTreeMap::new() };
        set_account(Some(who("http://insecure/a.png", true, Some(12))));
        set_saved(&serde_json::json!({ "entries": [{ "status": "playing" }, { "status": "playing" }, { "status": "plan" }, { "status": "bad status!" }] }));
        let p = PROFILE.lock().unwrap().clone().unwrap();
        assert_eq!((p.username.as_str(), p.supporter, p.kryos), ("sink", true, Some(12)));
        assert_eq!(p.avatar_url, None);
        assert_eq!(p.counts.get("playing"), Some(&2));
        assert_eq!(p.counts.get("plan"), Some(&1));
        assert_eq!(p.counts.len(), 2);
        // The same account reporting again keeps its counts; signing out clears everything.
        set_account(Some(who("https://x/a.png", false, None)));
        assert_eq!(PROFILE.lock().unwrap().as_ref().unwrap().counts.get("playing"), Some(&2));
        set_account(None);
        assert!(PROFILE.lock().unwrap().is_none());
    }

    #[test]
    fn the_look_a_phone_follows_is_cleaned() {
        phone_report_look(PhoneLook { theme: "dark".into(), palette: "amber".into(), radius: "x\"><script>".into(), font: "".into(), season: Some("bad season".into()), reduced_motion: true });
        let look = LOOK.lock().unwrap().clone().unwrap();
        assert_eq!((look.palette.as_str(), look.radius.as_str(), look.font.as_str()), ("amber", "pill", "teletext"));
        assert_eq!(look.season, None);
        assert!(look.reduced_motion);
    }

    #[test]
    fn the_route_to_the_internet_comes_first_and_the_rest_follow_in_order() {
        let a = |s: &str| s.parse::<Ipv4Addr>().unwrap();
        let found = vec![a("192.168.56.1"), a("192.168.1.166"), a("10.0.0.2"), a("192.168.1.166")];
        assert_eq!(rank(found.clone(), Some(a("192.168.1.166")), 47821), ["192.168.1.166:47821", "10.0.0.2:47821", "192.168.56.1:47821"]);
        // No known route, or one that is not among the addresses: plain order.
        assert_eq!(rank(found.clone(), None, 47821), ["10.0.0.2:47821", "192.168.1.166:47821", "192.168.56.1:47821"]);
        assert_eq!(rank(found, Some(a("172.16.0.9")), 47821)[0], "10.0.0.2:47821");
    }

    #[test]
    fn the_qr_text_carries_the_code_and_every_address() {
        let both = vec!["192.168.1.5:47821".to_string(), "10.0.0.2:47822".to_string()];
        assert_eq!(pair_uri(&both, "042517", "test-install").as_deref(), Some("kryoto://pair?c=042517&h=192.168.1.5:47821&h=10.0.0.2:47822&i=test-install"));
        // Account fallback still works without an address; a live code is required.
        assert_eq!(pair_uri(&[], "042517", "test-install").as_deref(), Some("kryoto://pair?c=042517&i=test-install"));
        assert_eq!(pair_uri(&both, "", "test-install"), None);
    }

    #[test]
    fn phone_names_are_tidied() {
        assert_eq!(clean_name("  Pixel\u{7}  9 "), "Pixel  9");
        assert_eq!(clean_name("\n\t "), "Phone");
        assert_eq!(clean_name(&"x".repeat(100)).len(), 60);
    }

    #[test]
    fn art_that_is_not_https_stays_on_the_pc() {
        assert_eq!(https_only(Some("https://kryo.to/a.jpg".into())), Some("https://kryo.to/a.jpg".into()));
        assert_eq!(https_only(Some("http://kryo.to/a.jpg".into())), None);
        assert_eq!(https_only(Some("kryoto-art://x".into())), None);
        assert_eq!(https_only(None), None);
    }

    #[test]
    fn achievement_maps_for_the_phone() {
        let steam = |icon: Option<&str>, gray: Option<&str>, display: &str| GameAchievement {
            achievement: kryoto_repair::achievements::Achievement {
                name: "ACH_ONE".into(),
                display_name: display.into(),
                description: "Do it.".into(),
                hidden: true,
                icon: icon.map(String::from),
                icon_gray: gray.map(String::from),
                percent: Some(12.5),
            },
            unlocked: false,
            unlocked_at: None,
        };
        let locked = phone_achievement(steam(Some("https://cdn/a.jpg"), Some("https://cdn/g.jpg"), ""));
        assert_eq!((locked.id.as_str(), locked.name.as_str()), ("ACH_ONE", "ACH_ONE"));
        assert_eq!(locked.icon.as_deref(), Some("https://cdn/g.jpg"));
        let mut done = steam(Some("https://cdn/a.jpg"), Some("https://cdn/g.jpg"), "One");
        done.unlocked = true;
        done.unlocked_at = Some(5);
        let done = phone_achievement(done);
        assert_eq!((done.name.as_str(), done.icon.as_deref(), done.unlocked_at), ("One", Some("https://cdn/a.jpg"), Some(5)));
        assert_eq!(phone_achievement(steam(Some("http://cdn/a.jpg"), None, "One")).icon, None);
    }
}
