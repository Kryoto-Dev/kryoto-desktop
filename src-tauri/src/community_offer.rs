//! Community updates: offering kryo.to a newer build of a game this PC has
//! (kryo.to lib/community-updates.ts, flag `community_updates`).
//!
//! Nothing here takes the player's word for a file. The steps, in order:
//!
//!   1. When Kryoto installs a release, [`record_release`] notes every file's
//!      size and time in `.kryoto-release.json` beside the game.
//!   2. An updater that brings the game past Kryoto's release writes
//!      `.kryoto-update.json` ([`UpdateRecord`]): the version it made and every
//!      file it wrote, with what each is. That record is the only way a game
//!      becomes offerable; there is no button that marks a folder "newer".
//!   3. The player plays it. While the game runs, [`during_play`] takes the
//!      screenshots itself (Windows; elsewhere the player adds one they took),
//!      and [`after_play`] keeps the longest session and how it ended.
//!   4. [`community_offer_send`]: the manifest of every file (a file that
//!      changed without the updater stops it here), Desktop's own checks
//!      (kryoto_repair::community), the screenshots up, the offer, and only
//!      if kryo.to accepts it the zip of exactly the manifest's files, uploaded
//!      in parts. kryo.to then reads the zip's own index back and compares.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use kryoto_repair::community::{self as cu, Role, Stamp};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::library::LibraryGame;

pub const RELEASE_RECORD: &str = ".kryoto-release.json";
pub const UPDATE_RECORD: &str = ".kryoto-update.json";
/// The play kryo.to asks for on the newer build (MIN_RUN_SECONDS there).
pub const MIN_PLAY_SECS: u64 = 10 * 60;
/// When, into a session, Desktop takes its screenshots.
const SHOT_AT_SECS: [u64; 2] = [3 * 60, 9 * 60];
const MAX_SHOTS: usize = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReleaseRecord {
    pub version: Option<String>,
    pub source: Option<String>,
    pub at: u64,
    pub files: BTreeMap<String, Stamp>,
}

/// What an updater did to a game folder. Written by the updater, read here.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateRecord {
    /// The version the game is now.
    pub to_version: String,
    /// Steam's build id it was brought to, when the updater knows it.
    pub build_id: Option<String>,
    /// Every file the updater wrote, `/`-separated, with what it is.
    pub written: BTreeMap<String, Role>,
    pub at: u64,
    /// The longest session on this build, and how it ended.
    pub playtest: Option<Playtest>,
    /// Screenshots of it running, files in the app's data folder.
    pub shots: Vec<String>,
    /// Once offered: kryo.to's id for it, and what it said.
    pub offer_id: Option<String>,
    pub offer_state: Option<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Playtest {
    pub seconds: u64,
    pub exit_code: Option<i32>,
    pub at: u64,
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub(crate) fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn read_update(dir: &Path) -> Option<UpdateRecord> {
    read_json(&dir.join(UPDATE_RECORD))
}

/// For an updater: say what it did (community_update.rs is the one Kryoto
/// ships). Replaces any earlier record: a new build needs its own playtest.
pub fn write_update(dir: &Path, record: &UpdateRecord) -> Result<(), String> {
    write_json(&dir.join(UPDATE_RECORD), record)
}

fn edit_update(dir: &Path, f: impl FnOnce(&mut UpdateRecord)) {
    if let Some(mut r) = read_update(dir) {
        f(&mut r);
        let _ = write_json(&dir.join(UPDATE_RECORD), &r);
    }
}

/// 1. A Kryoto release was installed here: note its files, off the UI thread
///    (a big game on a hard disk takes a while to walk). An earlier update record
///    belongs to the build this one replaced, so it goes.
///    `lost` are files the release has that never arrived (antivirus.rs
///    `unpack_losses`): kept in the record, so they read as missing.
pub fn record_release(game: &LibraryGame, lost: &[(String, u64)]) {
    let dir = PathBuf::from(&game.install_dir);
    let record = ReleaseRecord { version: game.version.clone(), source: game.source.clone(), at: now(), files: BTreeMap::new() };
    let lost = lost.to_vec();
    std::thread::spawn(move || {
        let _ = std::fs::remove_file(dir.join(UPDATE_RECORD));
        let mut files = cu::stamp_tree(&dir);
        for (rel, size) in lost {
            files.entry(rel).or_insert(Stamp { size, modified_ms: 0 });
        }
        if let Err(e) = write_json(&dir.join(RELEASE_RECORD), &ReleaseRecord { files, ..record }) {
            crate::logging::warn("community", &format!("{}: could not note the release's files: {e}", dir.display()));
        }
    });
}

fn shots_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    Some(app.path().app_data_dir().ok()?.join("community-shots"))
}

/// Something worth playtesting: a game an updater brought past Kryoto's
/// release, on a source kryo.to takes, not offered yet.
fn pending(game: &LibraryGame) -> Option<UpdateRecord> {
    let r = read_update(Path::new(&game.install_dir))?;
    (r.offer_id.is_none() && cu::eligible_source(game.source.as_deref().unwrap_or_default()).is_some()).then_some(r)
}

/// 3. While the game runs: take the screenshots, from the game's own window.
pub fn during_play<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, pid: u32) {
    if pending(game).is_none() || !cfg!(windows) {
        return;
    }
    let Some(dir) = shots_dir(app) else { return };
    let install = PathBuf::from(&game.install_dir);
    let slug = game.slug.clone().unwrap_or_else(|| game.id.clone());
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        for at in SHOT_AT_SECS {
            let wait = Duration::from_secs(at).saturating_sub(start.elapsed());
            std::thread::sleep(wait);
            // Gone (closed, or handed over to another process): no picture of it.
            let Some(png) = kryoto_repair::window_capture::capture_png(pid) else { continue };
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("{}-{}-{at}.png", safe(&slug), now()));
            if std::fs::write(&path, png).is_ok() {
                edit_update(&install, |r| {
                    r.shots.push(path.to_string_lossy().into_owned());
                    let extra = r.shots.len().saturating_sub(MAX_SHOTS);
                    for old in r.shots.drain(..extra) {
                        let _ = std::fs::remove_file(old);
                    }
                });
            }
        }
    });
}

/// 3. The game closed: keep the longest session on this build.
pub fn after_play(install_dir: &Path, seconds: u64, code: Option<i32>) {
    edit_update(install_dir, |r| {
        if r.offer_id.is_none() && r.playtest.as_ref().is_none_or(|p| seconds > p.seconds) {
            r.playtest = Some(Playtest { seconds, exit_code: code, at: now() });
        }
    });
}

pub(crate) fn safe(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).take(60).collect()
}

// ---- kryo.to, as the account signed in to the Store ----------------------------

pub(crate) struct Kryo {
    http: reqwest::Client,
    base: String,
    cookie: String,
    /// This install's id, which kryo.to's Desktop check asks for (lib/download-gate.ts).
    install: String,
}

impl Kryo {
    pub(crate) fn new<R: Runtime>(app: &AppHandle<R>) -> Result<Kryo, String> {
        let base = crate::settings::catalog_endpoint(&crate::settings::load(app));
        let url = url::Url::parse(&base).map_err(|e| e.to_string())?;
        let cookie = app
            .get_webview(crate::STORE)
            .and_then(|view| view.cookies_for_url(url).ok())
            .map(|jar| jar.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; "))
            .filter(|c| !c.is_empty())
            .ok_or("Sign in to kryo.to in Kryoto Desktop first.")?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Kryo { http, base, cookie, install: crate::logging::install_id(app) })
    }

    /// `Ok(None)` for a 404: the feature is off for this account.
    pub(crate) async fn call(&self, method: reqwest::Method, path: &str, body: Option<serde_json::Value>) -> Result<Option<serde_json::Value>, String> {
        let mut req =
            self.http.request(method, format!("{}{path}", self.base)).header("cookie", &self.cookie).header("x-kryo-install", &self.install);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req.send().await.map_err(|e| format!("Could not reach kryo.to: {e}"))?;
        let status = res.status().as_u16();
        let body: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
        match status {
            200..=299 => Ok(Some(body)),
            403 if body.get("error").and_then(|e| e.as_str()) == Some("pow_required") => {
                Err("kryo.to wants this PC to prove itself first: start any download from the Store once, then try again.".into())
            }
            401 => Err("Sign in to kryo.to in Kryoto Desktop first.".into()),
            404 if body.get("error").and_then(|e| e.as_str()) == Some("not_found") => Ok(None),
            _ => Err(body.get("error").and_then(|e| e.as_str()).unwrap_or("kryo.to could not do that.").to_string()),
        }
    }

    /// A screenshot onto kryo.to's filehost, through its media route.
    async fn upload_shot(&self, path: &Path) -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("Reading the screenshot failed: {e}"))?;
        let content_type = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            "image/png"
        } else if bytes.starts_with(&[0xff, 0xd8]) {
            "image/jpeg"
        } else {
            return Err("A screenshot has to be a PNG or a JPEG.".into());
        };
        let body = json!({ "type": "update_proof", "contentType": content_type, "size": bytes.len() });
        let got = self.call(reqwest::Method::POST, "/api/account/media", Some(body)).await?.ok_or("Community updates are not on for your account.")?;
        let upload = got["uploadUrl"].as_str().ok_or("kryo.to did not give an upload address.")?;
        let public = got["publicUrl"].as_str().ok_or("kryo.to did not give a screenshot address.")?.to_string();
        let res = storage()
            .put(upload)
            .header("content-type", content_type)
            .body(bytes)
            .send()
            .await
            .map_err(|e| format!("Uploading the screenshot failed: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("Uploading the screenshot failed ({}).", res.status()));
        }
        Ok(public)
    }

    async fn step(&self, offer: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
        self.call(reqwest::Method::POST, &format!("/api/community/updates/{offer}"), Some(body))
            .await?
            .ok_or_else(|| "Community updates are not on for your account.".to_string())
    }
}

/// Storage addresses: no cookies.
pub(crate) fn storage() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(30 * 60))
        .build()
        .unwrap_or_default()
}

// ---- What the game page shows ----------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferState {
    /// Show the card at all: the feature is on, an updater made this build,
    /// and the release is on a source kryo.to takes.
    pub shown: bool,
    pub to_version: String,
    pub kryoto_version: Option<String>,
    pub played_seconds: u64,
    pub need_seconds: u64,
    pub exit_code: Option<i32>,
    pub shots: Vec<String>,
    /// Desktop takes the screenshots itself here (Windows).
    pub auto_shots: bool,
    pub offer_id: Option<String>,
    pub offer_state: Option<String>,
    pub reasons: Vec<String>,
}

pub(crate) fn find<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<LibraryGame, String> {
    crate::library::load(app)?.into_iter().find(|g| g.id == id).ok_or_else(|| "That game is no longer in the library.".into())
}

/// An offer that is over without a release: the player may make a new one.
fn is_dead(state: Option<&str>) -> bool {
    matches!(state, Some("rejected_auto" | "rejected" | "withdrawn"))
}

fn state_of(game: &LibraryGame, record: &UpdateRecord) -> OfferState {
    let play = record.playtest.clone().unwrap_or_default();
    let dead = is_dead(record.offer_state.as_deref());
    OfferState {
        shown: true,
        to_version: record.to_version.clone(),
        kryoto_version: game.version.clone(),
        played_seconds: play.seconds,
        need_seconds: MIN_PLAY_SECS,
        exit_code: play.exit_code,
        shots: record.shots.iter().filter(|p| Path::new(p).is_file()).cloned().collect(),
        auto_shots: cfg!(windows),
        offer_id: record.offer_id.clone().filter(|_| !dead),
        offer_state: record.offer_state.clone().filter(|_| !dead),
        reasons: record.reasons.clone(),
    }
}

/// What kryo.to says about this offer now: staff decide long after it was sent.
fn sync_offer(install: &Path, record: &UpdateRecord, mine: &serde_json::Value) {
    let Some(id) = record.offer_id.as_deref() else { return };
    let Some(offer) = mine["offers"].as_array().and_then(|a| a.iter().find(|o| o["id"].as_str() == Some(id))) else { return };
    let Some(state) = offer["state"].as_str() else { return };
    if record.offer_state.as_deref() == Some(state) {
        return;
    }
    let mut reasons: Vec<String> =
        offer["scan"]["reasons"].as_array().map(|a| a.iter().filter_map(|r| r.as_str().map(String::from)).collect()).unwrap_or_default();
    if let Some(note) = offer["reviewNote"].as_str().filter(|_| state == "rejected") {
        reasons = vec![note.to_string()];
    }
    edit_update(install, |r| {
        r.offer_state = Some(state.to_string());
        r.reasons = reasons;
    });
}

#[tauri::command(async)]
pub async fn community_offer_state(app: AppHandle, id: String) -> Result<OfferState, String> {
    let game = find(&app, &id)?;
    let Some(record) = read_update(Path::new(&game.install_dir)) else { return Ok(OfferState::default()) };
    if game.slug.is_none() || cu::eligible_source(game.source.as_deref().unwrap_or_default()).is_none() {
        return Ok(OfferState::default());
    }
    // Behind the flag: kryo.to answers 404 when it is off for this account.
    let mine = match Kryo::new(&app) {
        Ok(k) => k.call(reqwest::Method::GET, "/api/community/updates", None).await.unwrap_or(None),
        Err(_) => None,
    };
    let Some(mine) = mine else { return Ok(OfferState::default()) };
    let install = PathBuf::from(&game.install_dir);
    sync_offer(&install, &record, &mine);
    Ok(state_of(&game, &read_update(&install).unwrap_or(record)))
}

/// A screenshot the player took themselves (where Desktop cannot take one).
#[tauri::command(async)]
pub fn community_offer_add_shot(app: AppHandle, id: String, path: String) -> Result<OfferState, String> {
    let game = find(&app, &id)?;
    let src = PathBuf::from(&path);
    let bytes = std::fs::read(&src).map_err(|e| format!("Reading {path} failed: {e}"))?;
    if !(bytes.starts_with(&[0x89, b'P', b'N', b'G']) || bytes.starts_with(&[0xff, 0xd8])) || bytes.len() > 8 * 1024 * 1024 {
        return Err("Pick a PNG or JPEG screenshot under 8 MB.".into());
    }
    let dir = shots_dir(&app).ok_or("No data folder.")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ext = if bytes.starts_with(&[0xff, 0xd8]) { "jpg" } else { "png" };
    let dest = dir.join(format!("{}-{}-own.{ext}", safe(game.slug.as_deref().unwrap_or(&game.id)), now()));
    std::fs::write(&dest, bytes).map_err(|e| e.to_string())?;
    let install = PathBuf::from(&game.install_dir);
    edit_update(&install, |r| {
        r.shots.push(dest.to_string_lossy().into_owned());
        let extra = r.shots.len().saturating_sub(MAX_SHOTS);
        r.shots.drain(..extra);
    });
    let record = read_update(&install).ok_or("This game has no update to offer.")?;
    Ok(state_of(&game, &record))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    id: &'a str,
    stage: &'a str,
    done: u64,
    total: u64,
}

pub(crate) fn progress<R: Runtime>(app: &AppHandle<R>, id: &str, stage: &str, done: u64, total: u64) {
    let _ = app.emit("community-offer", Progress { id, stage, done, total });
}

struct NoCancel;
impl kryoto_repair::Cancellation for NoCancel {
    fn requested(&self) -> bool {
        false
    }
}

/// Games whose offer is being sent right now, and those the player cancelled.
static SENDING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
static CANCELLED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn cancelled(id: &str) -> bool {
    CANCELLED.lock().map(|set| set.contains(id)).unwrap_or(false)
}

/// Take an offer back: a send under way stops at its next part and finishes
/// this itself; otherwise kryo.to is told now, and its file goes at once.
#[tauri::command(async)]
pub async fn community_offer_cancel(app: AppHandle, id: String) -> Result<OfferState, String> {
    let game = find(&app, &id)?;
    let install = PathBuf::from(&game.install_dir);
    let record = read_update(&install).ok_or("This game has no update to offer.")?;
    if SENDING.lock().map(|set| set.contains(&id)).unwrap_or(false) {
        if let Ok(mut set) = CANCELLED.lock() {
            set.insert(id);
        }
        return Ok(state_of(&game, &record));
    }
    withdraw(&app, &install, &record).await?;
    Ok(state_of(&game, &read_update(&install).unwrap_or_default()))
}

async fn withdraw<R: Runtime>(app: &AppHandle<R>, install: &Path, record: &UpdateRecord) -> Result<(), String> {
    if let Some(offer) = &record.offer_id {
        if !is_dead(record.offer_state.as_deref()) {
            Kryo::new(app)?.call(reqwest::Method::DELETE, &format!("/api/community/updates/{offer}"), None).await?;
        }
    }
    edit_update(install, |r| {
        r.offer_id = None;
        r.offer_state = None;
        r.reasons = Vec::new();
    });
    Ok(())
}

/// 4. Offer it. Every refusal, Desktop's or kryo.to's, comes back as the
/// record's `reasons`, and nothing is uploaded unless kryo.to took the offer.
#[tauri::command(async)]
pub async fn community_offer_send(app: AppHandle, id: String, confirmed_works: bool, confirmed_version: bool) -> Result<OfferState, String> {
    if let Ok(mut set) = SENDING.lock() {
        set.insert(id.clone());
    }
    let result = send_offer(&app, &id, confirmed_works, confirmed_version).await;
    if let Ok(mut set) = SENDING.lock() {
        set.remove(&id);
    }
    // The player cancelled meanwhile: the upload is stopped, so take the offer back.
    let stopped = CANCELLED.lock().map(|mut set| set.remove(&id)).unwrap_or(false);
    if stopped {
        let game = find(&app, &id)?;
        let install = PathBuf::from(&game.install_dir);
        let record = read_update(&install).ok_or("This game has no update to offer.")?;
        withdraw(&app, &install, &record).await?;
        return Ok(state_of(&game, &read_update(&install).unwrap_or_default()));
    }
    result
}

async fn send_offer(app: &AppHandle, id: &str, confirmed_works: bool, confirmed_version: bool) -> Result<OfferState, String> {
    let game = find(app, id)?;
    let install = PathBuf::from(&game.install_dir);
    let record = read_update(&install).ok_or("This game has no update to offer.")?;
    // An offer whose upload never finished picks up there; one that is over
    // starts again; anything else is with kryo.to and only shown.
    let resume = record.offer_id.clone().filter(|_| matches!(record.offer_state.as_deref(), Some("offered" | "uploading")));
    if record.offer_id.is_some() && resume.is_none() && !is_dead(record.offer_state.as_deref()) {
        return Ok(state_of(&game, &record));
    }
    let slug = game.slug.clone().ok_or("Only a kryo.to game can be offered.")?;
    let source = game.source.clone().unwrap_or_default();
    let release: ReleaseRecord = read_json(&install.join(RELEASE_RECORD)).ok_or("Kryoto has no record of the release installed here, so it cannot tell what changed. Reinstall it from Kryoto first.")?;
    let refuse = |reasons: Vec<String>| {
        edit_update(&install, |r| r.reasons = reasons.clone());
        Ok::<OfferState, String>(state_of(&game, &read_update(&install).unwrap_or_default()))
    };

    progress(app, id, "Checking the files", 0, 0);
    let (manifest, before) = {
        let install = install.clone();
        let written = record.written.clone();
        let before = release.files.clone();
        tauri::async_runtime::spawn_blocking(move || (cu::build_manifest(&install, &before, &written, &NoCancel), before))
            .await
            .map_err(|e| e.to_string())?
    };
    let manifest = match manifest {
        Ok(m) => m,
        Err(foreign) => {
            let list = foreign.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            return refuse(vec![format!("Files changed that the updater did not write ({list}{}).", if foreign.len() > 3 { "…" } else { "" })]);
        }
    };
    let mut reasons = cu::check_offer(&source, &manifest, None);
    let play = record.playtest.clone().unwrap_or_default();
    if play.seconds < MIN_PLAY_SECS {
        reasons.push(format!("Play the newer build for at least {} minutes first.", MIN_PLAY_SECS / 60));
    }
    let shots: Vec<String> = record.shots.iter().filter(|p| Path::new(p).is_file()).cloned().collect();
    if shots.is_empty() {
        reasons.push("Add a screenshot of the game running on the newer build.".into());
    }
    if !reasons.is_empty() {
        return refuse(reasons);
    }

    let kryo = Kryo::new(app)?;
    let offer_id = match resume {
        Some(offer) => offer,
        None => {
            progress(app, id, "Sending the screenshots", 0, shots.len() as u64);
            let mut urls = Vec::new();
            for (i, shot) in shots.iter().enumerate() {
                urls.push(kryo.upload_shot(Path::new(shot)).await?);
                progress(app, id, "Sending the screenshots", i as u64 + 1, shots.len() as u64);
            }
            let diff = cu::diff(&before, &manifest);
            let body = json!({
                "slug": slug,
                "offeredVersion": record.to_version,
                "offeredBuildId": record.build_id,
                "installId": game.id,
                "evidence": {
                    "seconds": play.seconds,
                    "exitCode": play.exit_code,
                    "confirmedWorks": confirmed_works,
                    "confirmedVersion": confirmed_version,
                    "screenshots": urls,
                },
                "diff": diff,
                "manifest": manifest,
            });
            let got = kryo.call(reqwest::Method::POST, "/api/community/updates", Some(body)).await?.ok_or("Community updates are not on for your account.")?;
            let offer = &got["offer"];
            let offer_id = offer["id"].as_str().ok_or("kryo.to did not take the offer.")?.to_string();
            let state = offer["state"].as_str().unwrap_or_default().to_string();
            let server_reasons: Vec<String> =
                offer["scan"]["reasons"].as_array().map(|a| a.iter().filter_map(|r| r.as_str().map(String::from)).collect()).unwrap_or_default();
            edit_update(&install, |r| {
                r.offer_id = Some(offer_id.clone());
                r.offer_state = Some(state.clone());
                r.reasons = server_reasons.clone();
            });
            if state != "offered" {
                return Ok(state_of(&game, &read_update(&install).unwrap_or_default()));
            }
            offer_id
        }
    };

    // Accepted for upload: the zip of exactly the manifest's files, beside the game.
    let zip = install.parent().unwrap_or(&install).join(format!(".kryoto-offer-{}.zip", safe(&slug)));
    progress(app, id, "Packing", 0, manifest.len() as u64);
    {
        let (install, zip, manifest) = (install.clone(), zip.clone(), manifest.clone());
        tauri::async_runtime::spawn_blocking(move || cu::pack_zip(&install, &manifest, &zip, &NoCancel)).await.map_err(|e| e.to_string())??;
    }
    let result = upload(app, &kryo, id, &offer_id, &zip).await;
    let _ = std::fs::remove_file(&zip);
    let done = result?;
    edit_update(&install, |r| {
        r.offer_state = Some(done.0.clone());
        r.reasons = done.1.clone();
    });
    Ok(state_of(&game, &read_update(&install).unwrap_or_default()))
}

/// Parts big enough to stay under the bucket's 10,000, never under 64 MB.
fn part_size(total: u64) -> u64 {
    const MB: u64 = 1024 * 1024;
    (total.div_ceil(9000).div_ceil(MB) * MB).max(64 * MB)
}

/// The archive up, in parts, through the offer's own upload steps. Returns
/// kryo.to's verdict: the state and its reasons.
async fn upload<R: Runtime>(app: &AppHandle<R>, kryo: &Kryo, id: &str, offer: &str, zip: &Path) -> Result<(String, Vec<String>), String> {
    let total = std::fs::metadata(zip).map_err(|e| e.to_string())?.len();
    let sha = {
        let zip = zip.to_path_buf();
        tauri::async_runtime::spawn_blocking(move || cu::fingerprint(&zip).map(|f| f.0)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?
    };
    let init = kryo.step(offer, json!({ "step": "init", "size": total })).await?;
    let key = init["key"].as_str().ok_or("kryo.to did not start the upload.")?.to_string();
    let upload_id = init["uploadId"].as_str().ok_or("kryo.to did not start the upload.")?.to_string();
    let size = part_size(total);
    let count = total.div_ceil(size).max(1);
    let http = storage();
    let mut parts = Vec::new();
    let mut file = std::fs::File::open(zip).map_err(|e| e.to_string())?;
    let mut sent = 0u64;
    let outcome: Result<(), String> = async {
        for window in (1..=count).collect::<Vec<_>>().chunks(64) {
            let signed = kryo.step(offer, json!({ "step": "presign", "key": key, "uploadId": upload_id, "partNumbers": window })).await?;
            let urls = signed["urls"].as_array().cloned().unwrap_or_default();
            for n in window {
                if cancelled(id) {
                    return Err("The upload was cancelled.".to_string());
                }
                let url = urls
                    .iter()
                    .find(|u| u["partNumber"].as_u64() == Some(*n))
                    .and_then(|u| u["url"].as_str())
                    .ok_or("kryo.to did not sign a part.")?
                    .to_string();
                let start = (n - 1) * size;
                let len = size.min(total - start);
                let mut buf = vec![0u8; len as usize];
                file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
                file.read_exact(&mut buf).map_err(|e| e.to_string())?;
                let mut tries = 0;
                let etag = loop {
                    tries += 1;
                    match http.put(&url).body(buf.clone()).send().await {
                        Ok(res) if res.status().is_success() => {
                            break res.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                        }
                        Ok(res) if tries >= 4 => return Err(format!("A part of the upload was refused ({}).", res.status())),
                        Err(e) if tries >= 4 => return Err(format!("The upload stopped: {e}")),
                        _ => tokio::time::sleep(Duration::from_secs(2 * tries)).await,
                    }
                };
                parts.push(json!({ "partNumber": n, "etag": etag }));
                sent += len;
                progress(app, id, "Uploading", sent, total);
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = outcome {
        let _ = kryo.step(offer, json!({ "step": "abort", "key": key, "uploadId": upload_id })).await;
        return Err(e);
    }
    progress(app, id, "Checking on kryo.to", total, total);
    kryo.step(offer, json!({ "step": "complete", "key": key, "uploadId": upload_id, "parts": parts, "sha256": sha })).await?;
    // The verdict is on the offer now: the archive's own index against the list.
    let mine = kryo.call(reqwest::Method::GET, "/api/community/updates", None).await?.unwrap_or_default();
    let offer = mine["offers"].as_array().and_then(|a| a.iter().find(|o| o["id"].as_str() == Some(offer))).cloned().unwrap_or_default();
    let reasons = offer["scan"]["reasons"].as_array().map(|a| a.iter().filter_map(|r| r.as_str().map(String::from)).collect()).unwrap_or_default();
    Ok((offer["state"].as_str().unwrap_or("review").to_string(), reasons))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_stay_under_the_bucket_limit() {
        const MB: u64 = 1024 * 1024;
        assert_eq!(part_size(10 * MB), 64 * MB);
        let huge = 2_000 * 1024 * MB;
        assert!(huge.div_ceil(part_size(huge)) <= 9000);
        assert_eq!(part_size(huge) % MB, 0);
    }

    #[test]
    fn only_an_updater_record_makes_a_game_offerable_and_the_longest_session_counts() {
        let dir = std::env::temp_dir().join(format!("kry-offer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut game = LibraryGame { install_dir: dir.to_string_lossy().into(), source: Some("Steam + gbe_fork".into()), ..Default::default() };
        assert!(pending(&game).is_none());
        write_update(&dir, &UpdateRecord { to_version: "1.1".into(), ..Default::default() }).unwrap();
        assert!(pending(&game).is_some());
        after_play(&dir, 700, Some(0));
        after_play(&dir, 30, Some(1));
        let r = read_update(&dir).unwrap();
        assert_eq!(r.playtest.as_ref().map(|p| (p.seconds, p.exit_code)), Some((700, Some(0))));
        game.source = Some("Steam + RUNE".into());
        assert!(pending(&game).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
