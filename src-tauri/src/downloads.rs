//! Downloads: from a kryo.to Download button to a game in the Library.
//!
//! The store page asks kryo.to for a signed link and navigates to it, which
//! the Store web view reports as a download. That download is cancelled there
//! and taken over here, so it gets what a browser download cannot: a queue,
//! pause and resume, speed, and - when it lands - extraction into the library
//! folder and a Library entry that already knows how the game starts.
//!
//! Links are signed for an hour and bound to this network, and dl.kryo.to
//! honours `Range` on the same link past its hour (a resume grace), so pausing
//! and resuming is a ranged GET against the URL the page handed over.
//!
//! One download at a time, like Steam: a second one queues.

use crate::launch::LaunchEntry;
use crate::library::{self, LibraryGame};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Queued,
    /// Asking a mirror for its file (see `resolvers`).
    Resolving,
    Downloading,
    Paused,
    /// Checking the archive against the SHA-256 kryo.to lists for it.
    Verifying,
    Extracting,
    Installed,
    Failed,
    Canceled,
}

/// What kryo.to says about the release, fetched when the download starts so
/// the install can finish offline and the list can show art straight away.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CatalogMeta {
    pub title: String,
    pub cover: Option<String>,
    pub hero: Option<String>,
    /// The transparent title logo, when Steam has one.
    pub logo: Option<String>,
    /// The wide store header (460x215), for rows and cards.
    pub header: Option<String>,
    pub executable: String,
    pub default_args: String,
    pub entries: Vec<LaunchEntry>,
    pub source: Option<String>,
    pub version: Option<String>,
    pub short: Option<String>,
    pub developer: Option<String>,
    pub size_bytes: Option<u64>,
    pub nsfw: bool,
    /// The Steam branch the listing builds from: `public`, or a beta listed as
    /// a game of its own (GoreBox27 is GoreBox's `gb27`). None from a site
    /// that does not say.
    pub steam_branch: Option<String>,
    /// kryo.to's VR flag (`vr_supported`, `vr_only`), from Steam's
    /// categories. None for a flat game, or a site that does not say.
    pub vr: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Download {
    pub id: String,
    pub slug: Option<String>,
    pub url: String,
    pub file_name: String,
    pub archive_path: String,
    pub total: Option<u64>,
    pub received: u64,
    /// Bytes a second, averaged over the last couple of seconds.
    pub speed: u64,
    /// Extraction progress, in uncompressed bytes.
    pub extracted: u64,
    pub extract_total: Option<u64>,
    pub status: Status,
    pub error: Option<String>,
    pub install_dir: Option<String>,
    pub game_id: Option<String>,
    pub added_at: u64,
    pub finished_at: Option<u64>,
    pub meta: CatalogMeta,
    /// The byte ranges being fetched in parallel, and how far each has got.
    pub segments: Vec<Segment>,
    /// The archive's expected SHA-256, when kryo.to lists one.
    pub sha256: Option<String>,
    /// The archive matched it.
    pub verified: bool,
    /// The archive did not match the hash. It is kept, so the player can press
    /// Install anyway (`download_install_anyway`);
    /// Retry throws it away and downloads it again.
    pub hash_mismatch: bool,
    /// Set when the file is one of the game's add-ons (a language pack, the
    /// Online add-on): its name, e.g. "Japanese language pack".
    pub addon: Option<String>,
    /// The add-on is a repair fix (kind `repair` on kryo.to, Online-Fix's
    /// generic fix): applied under the repair journal so Undo puts back what it
    /// replaced (`addons::install_fix`).
    pub repair: bool,
    /// Installed, but the unpacker reported damaged files (a CRC mismatch
    /// inside the archive). Games often run anyway; this says which files.
    pub warning: Option<String>,
    /// Set when this comes from a mirror rather than kryo.to's own copy: the
    /// mirror's page, resolved again whenever the file's address runs out.
    pub mirror: Option<Mirror>,
    /// The release picked under a game's Versions, when it is not the
    /// current one: its version string, recorded on the installed game.
    pub release: Option<String>,
    /// Added from a game's Add-ons & DLC card (addons.rs `addons_get`): the
    /// add-on's release id on kryo.to. Its link is minted for that release at
    /// its turn, and it is an add-on of `game_id` whatever its file is called.
    pub addon_release: Option<String>,
    /// Place in the queue: the lowest waiting one goes next. Changed by
    /// moving it up or down, or "Download now" (`download_move`).
    pub queue_order: u64,
    /// The file's ETag when the download started. A resume checks every
    /// range against it, so a file replaced on the server is started over
    /// instead of having its bytes stitched onto the old one's.
    pub etag: Option<String>,
    /// What it is waiting on right now, for the Downloads page ("Connection
    /// lost. Trying again in 8 s."). Cleared once bytes move again.
    pub notice: Option<String>,
    /// The library folder the player picked for this one (suggestion #59);
    /// none is the default folder.
    pub library_dir: Option<String>,
    /// Waiting for the player to pick a folder: queued, with no task until
    /// they answer (`download_pick_folder`), so the queue goes past it.
    pub ask_folder: bool,
    /// Queued as part of a collection (collections.rs): it has no link yet.
    /// Links run out and a long queue outlives them, so kryo.to is asked for
    /// this one when its turn comes, and again on each Retry or Resume.
    pub mint_at_turn: bool,
    /// The hidden folder this row is unpacking into, until the game is in
    /// place. A Cancel after a failed unpack, or after a restart, is what
    /// takes it away: it can hold a whole game.
    pub staging: Option<String>,
    /// One stream (a host with no ranges): how much of the archive is known
    /// to be on the disk itself, not only in the system's write cache. A
    /// resume after a crash or power cut starts here rather than at the
    /// file's length, which can count bytes that never reached the disk (the
    /// ranged path has `checkpoint` for the same). None: an older row, which
    /// goes by the length as before.
    pub synced: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Mirror {
    /// The mirror's own link, as kryo.to lists it.
    pub page: String,
    /// The host's name, for the list ("Pixeldrain").
    pub host: String,
}

/// One byte range of a download, on a connection of its own.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Segment {
    pub start: u64,
    /// Inclusive.
    pub end: u64,
    pub done: u64,
}

impl Segment {
    fn len(&self) -> u64 {
        self.end - self.start + 1
    }
}

/// Split `total` bytes into ranges for `n` connections to work through: four
/// per connection, of at least 16 MB. More ranges than connections is what
/// keeps every connection busy to the end: one that finishes early takes the
/// next range instead of sitting idle while the slowest one crawls home.
pub fn split(total: u64, n: u32) -> Vec<Segment> {
    if total == 0 {
        return Vec::new();
    }
    let min = 16 * 1024 * 1024;
    let n = (u64::from(n.max(1)) * RANGES_PER_CONNECTION).min((total / min).max(1));
    let size = total.div_ceil(n);
    (0..n)
        .map(|i| Segment { start: i * size, end: ((i + 1) * size).min(total) - 1, done: 0 })
        .filter(|s| s.start < total)
        .collect()
}

const RANGES_PER_CONNECTION: u64 = 4;

/// Bytes a connection gathers before writing them out. Writing each network
/// chunk (a few KB) on its own went through the blocking pool once per chunk,
/// which capped a download near 30 MB/s however fast the line was.
const WRITE_BUFFER: usize = 4 * 1024 * 1024;

/// A shared speed cap: every connection draws from the same budget.
///
/// A budget that cannot be saved up. This used to measure everything sent
/// against the time since the download started, so every slow stretch (a
/// busy disk, a dropped connection) banked allowance that was then spent at
/// full line speed: a 25 MB/s cap ran at 100 MB/s, flooded a hard disk faster
/// than it could write, crawled at a few KB/s while the disk caught up -
/// banking more - and went round again.
pub struct Limiter {
    bytes_per_sec: u64,
    /// When the bytes taken so far will have been paid for.
    paid_until: Mutex<Instant>,
}

/// How far ahead of the cap a burst may run: a second's worth.
const LIMIT_BURST: std::time::Duration = std::time::Duration::from_secs(1);

impl Limiter {
    pub fn new(mb_per_sec: u32) -> Self {
        Self { bytes_per_sec: u64::from(mb_per_sec) * 1024 * 1024, paid_until: Mutex::new(Instant::now()) }
    }

    async fn take(&self, n: u64) {
        if self.bytes_per_sec == 0 {
            return;
        }
        let wait = {
            let mut paid = self.paid_until.lock().unwrap_or_else(|p| p.into_inner());
            limit_wait(&mut paid, Instant::now(), n, self.bytes_per_sec)
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// How long to wait after taking `n` bytes at `rate`. Time nobody used is not
/// carried forward: the bill starts again from now.
fn limit_wait(paid_until: &mut Instant, now: Instant, n: u64, rate: u64) -> std::time::Duration {
    let cost = std::time::Duration::from_secs_f64(n as f64 / rate as f64);
    *paid_until = (*paid_until).max(now) + cost;
    paid_until.saturating_duration_since(now).saturating_sub(LIMIT_BURST)
}

/// The longest a transfer waits for data before it looks at Pause and Cancel.
const STOP_POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// How long a connection may go without a byte before it is dropped and
/// opened again. Without it a connection that went quiet without closing (a
/// router dropping state, a server restarting behind a proxy) waited forever:
/// the download sat at 0 B/s until the player paused and resumed it by hand.
const STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A download whose connections slow to a crawl without stopping: many hosts
/// throttle a connection the longer it runs, and only a Pause and Resume (new
/// connections, and for a mirror a fresh link) brought the speed back. This
/// does that by itself when the speed stays under an eighth of its best for
/// 40 s, on a download that was fast (a megabyte a second or more) and has
/// more than 64 MB to go. A slow line all along is never touched, and it gives
/// up after `SLOW_REFRESHES` tries in one run, so a host that is simply slow
/// now is not asked again and again.
struct SlowWatch {
    started: Instant,
    best: u64,
    slow_since: Option<Instant>,
}

const SLOW_WARMUP: std::time::Duration = std::time::Duration::from_secs(20);
const SLOW_FOR: std::time::Duration = std::time::Duration::from_secs(40);
const SLOW_FRACTION: u64 = 8;
const SLOW_MIN_BEST: u64 = 1024 * 1024;
const SLOW_MIN_LEFT: u64 = 64 * 1024 * 1024;
const SLOW_REFRESHES: u32 = 3;

impl SlowWatch {
    fn new(now: Instant) -> Self {
        SlowWatch { started: now, best: 0, slow_since: None }
    }

    /// One reading of the averaged speed and what is left: true when it is
    /// time for new connections.
    fn note(&mut self, now: Instant, speed: u64, left: u64) -> bool {
        if now.duration_since(self.started) < SLOW_WARMUP {
            return false;
        }
        self.best = self.best.max(speed);
        if self.best < SLOW_MIN_BEST || left < SLOW_MIN_LEFT || speed.saturating_mul(SLOW_FRACTION) >= self.best {
            self.slow_since = None;
            return false;
        }
        let since = *self.slow_since.get_or_insert(now);
        now.duration_since(since) >= SLOW_FOR
    }
}

/// How long a server has to start answering a request.
const ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Tries in a row, without a byte arriving, before a download gives up and
/// waits for the player. Spaced out (`backoff`), they ride out a few minutes
/// of lost connection; any progress starts the count again.
const MAX_FAILURES: u32 = 10;

/// Tries for one range on its own connection before the whole transfer
/// steps back and starts again from where every range got to.
const RANGE_RETRIES: u32 = 3;

/// Seconds to wait before try `n` (from 1): 2, 4, 8, 16, 30, then a minute.
fn backoff(n: u32) -> u64 {
    match n {
        0 | 1 => 2,
        2 => 4,
        3 => 8,
        4 => 16,
        5 => 30,
        _ => 60,
    }
}

/// The HTTP client every download of ours starts from.
fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .tcp_nodelay(true)
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(15))
        .pool_max_idle_per_host(64)
}

/// Wait for `work`, or for the download to be paused or cancelled, whichever is
/// first. None when it was stopped: a server that takes its time to answer
/// never holds up Pause.
async fn unless_stopped<T>(flag: &AtomicU8, work: impl std::future::Future<Output = T>) -> Option<T> {
    tokio::pin!(work);
    loop {
        match tokio::time::timeout(STOP_POLL, &mut work).await {
            Ok(done) => return Some(done),
            Err(_) if flag.load(Ordering::SeqCst) != RUN => return None,
            Err(_) => {}
        }
    }
}

/// How long the speed shown is averaged over. Long enough that the number and
/// the time left stop jumping with every burst from a connection, short enough
/// to follow a real change.
const SPEED_WINDOW: std::time::Duration = std::time::Duration::from_secs(8);

/// Bytes a second over the last `SPEED_WINDOW`, from running totals.
struct SpeedMeter {
    samples: std::collections::VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    fn new(at: Instant, total: u64) -> Self {
        Self { samples: std::collections::VecDeque::from([(at, total)]) }
    }

    /// Note where the count is now and say the speed over the window.
    fn add(&mut self, at: Instant, total: u64) -> u64 {
        self.samples.push_back((at, total));
        // One sample stays at or past the window's edge, so the span covers it.
        while self.samples.len() > 2 && self.samples.get(1).is_some_and(|(t, _)| at.duration_since(*t) >= SPEED_WINDOW) {
            self.samples.pop_front();
        }
        let (start, from) = self.samples[0];
        let secs = at.duration_since(start).as_secs_f64();
        if secs < 0.1 {
            return 0;
        }
        (total.saturating_sub(from) as f64 / secs) as u64
    }
}

/// Send, giving up when the server takes longer than `ANSWER_TIMEOUT` to answer.
async fn send(request: reqwest::RequestBuilder) -> Result<reqwest::Response, Transfer> {
    match tokio::time::timeout(ANSWER_TIMEOUT, request.send()).await {
        Ok(Ok(res)) => Ok(res),
        Ok(Err(e)) => Err(Transfer::Retry(e.to_string())),
        Err(_) => Err(Transfer::Retry("the server took too long to answer".into())),
    }
}

/// A `Content-Range` header: `bytes 100-199/1000`, or `bytes */1000` on a 416.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub total: Option<u64>,
}

pub fn parse_content_range(value: &str) -> Option<ContentRange> {
    let rest = value.trim().strip_prefix("bytes")?.trim_start();
    let (span, total) = rest.split_once('/')?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    let (start, end) = match span.trim() {
        "*" => (None, None),
        s => {
            let (a, b) = s.split_once('-')?;
            (Some(a.trim().parse().ok()?), Some(b.trim().parse().ok()?))
        }
    };
    Some(ContentRange { start, end, total })
}

fn content_range_of(res: &reqwest::Response) -> Option<ContentRange> {
    res.headers().get(reqwest::header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(parse_content_range)
}

/// The response's ETag, without a weak marker, for comparing two answers.
fn etag_of(res: &reqwest::Response) -> Option<String> {
    let v = res.headers().get(reqwest::header::ETAG)?.to_str().ok()?.trim();
    let v = v.strip_prefix("W/").unwrap_or(v).trim_matches('"');
    (!v.is_empty()).then(|| v.to_string())
}

/// Whether an answer is still the file the download started from. Only a
/// difference counts: a server that sends no ETag one time says nothing.
fn same_file(expected_etag: Option<&str>, expected_total: Option<u64>, etag: Option<&str>, total: Option<u64>) -> bool {
    let etag_ok = match (expected_etag, etag) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    let total_ok = match (expected_total, total) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    etag_ok && total_ok
}

/// Sleep for `secs`, waking early when the download is paused or cancelled.
/// False when it was.
async fn wait_unless_stopped(flag: &AtomicU8, secs: u64) -> bool {
    let until = Instant::now() + std::time::Duration::from_secs(secs);
    while Instant::now() < until {
        if flag.load(Ordering::SeqCst) != RUN {
            return false;
        }
        tokio::time::sleep(STOP_POLL).await;
    }
    flag.load(Ordering::SeqCst) == RUN
}

pub(crate) const RUN: u8 = 0;
const PAUSE: u8 = 1;
const CANCEL: u8 = 2;
/// Set by a connection that hit something the others cannot carry on past
/// (the file changed, the server refused): stop them all now.
const HALT: u8 = 3;

pub struct Downloads {
    list: Mutex<Vec<Download>>,
    controls: Mutex<HashMap<String, Arc<AtomicU8>>>,
    /// One transfer at a time: the one holding the turn. Not a FIFO
    /// semaphore, so the queue's order can change while games wait.
    active: Mutex<Option<String>>,
    /// Wakes the waiting downloads when the turn frees or the order changes.
    turn: tokio::sync::Notify,
    /// What the taskbar button last showed (`taskbar_state`).
    taskbar: Mutex<Option<(u8, u64)>>,
    last_emit: Mutex<Option<Instant>>,
    /// What each mirror download resolved to. Not saved: cookies and signed
    /// addresses go stale, so a download resumed after a restart asks again.
    resolved: Mutex<HashMap<String, crate::resolvers::Resolved>>,
}

impl Downloads {
    pub fn new() -> Self {
        Self {
            list: Mutex::new(Vec::new()),
            controls: Mutex::new(HashMap::new()),
            active: Mutex::new(None),
            turn: tokio::sync::Notify::new(),
            taskbar: Mutex::new(None),
            last_emit: Mutex::new(None),
            resolved: Mutex::new(HashMap::new()),
        }
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn state_file<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("downloads.json"))
}

static PERSIST: Mutex<()> = Mutex::new(());

pub(crate) fn persist_checked<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    use std::io::Write;
    let _guard = PERSIST.lock().map_err(|_| "download persistence lock poisoned")?;
    let file = state_file(app).ok_or("Could not find the download state folder.")?;
    let list = app.state::<Downloads>().list.lock().map_err(|_| "download list lock poisoned")?.clone();
    let tmp = file.with_extension("json.tmp");
    let mut output = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    output.write_all(&serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    std::fs::rename(tmp, file).map_err(|e| e.to_string())
}

fn persist<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = persist_checked(app) { crate::logging::error("download-save", &e); }
}

fn emit<R: Runtime>(app: &AppHandle<R>, force: bool) {
    let state = app.state::<Downloads>();
    if !force {
        if let Ok(mut last) = state.last_emit.lock() {
            if last.is_some_and(|t| t.elapsed().as_millis() < 250) {
                return;
            }
            *last = Some(Instant::now());
        }
    }
    let list = state.list.lock().map(|l| l.clone()).unwrap_or_default();
    show_on_taskbar(app, &list);
    let _ = app.emit("downloads", list);
}

/// What the taskbar button shows for the downloads: (status, percent).
/// 0 none, 1 normal, 2 waiting, 3 paused - as Steam does on its icon.
pub fn taskbar_state(list: &[Download]) -> (u8, u64) {
    let working = list.iter().find(|d| {
        matches!(d.status, Status::Resolving | Status::Downloading | Status::Verifying | Status::Extracting)
    });
    if let Some(d) = working {
        return (1, (overall_progress(d) * 100.0).round().clamp(0.0, 100.0) as u64);
    }
    if list.iter().any(|d| d.status == Status::Queued) {
        return (2, 0);
    }
    if let Some(d) = list.iter().find(|d| d.status == Status::Paused) {
        return (3, (overall_progress(d) * 100.0).round().clamp(0.0, 100.0) as u64);
    }
    (0, 0)
}

/// The same split the Downloads page draws: fetching to 85%, checking to 90%,
/// unpacking the rest.
fn overall_progress(d: &Download) -> f64 {
    let part = d.extract_total.filter(|t| *t > 0).map(|t| (d.extracted as f64 / t as f64).min(1.0)).unwrap_or(0.0);
    match d.status {
        Status::Verifying => 0.85 + 0.05 * part,
        Status::Extracting => {
            if d.extract_total.is_some() {
                0.9 + 0.1 * part
            } else {
                0.95
            }
        }
        _ => d.total.filter(|t| *t > 0).map(|t| (d.received as f64 / t as f64).min(1.0) * 0.85).unwrap_or(0.0),
    }
}

/// Progress on the app's taskbar button, so a download can be watched with
/// the window minimised or behind a game. Only sent when it changes.
fn show_on_taskbar<R: Runtime>(app: &AppHandle<R>, list: &[Download]) {
    use tauri::window::{ProgressBarState, ProgressBarStatus};
    let next = taskbar_state(list);
    let state = app.state::<Downloads>();
    if let Ok(mut last) = state.taskbar.lock() {
        if *last == Some(next) {
            return;
        }
        *last = Some(next);
    }
    let Some(window) = app.get_window("main") else { return };
    let status = match next.0 {
        1 => ProgressBarStatus::Normal,
        2 => ProgressBarStatus::Indeterminate,
        3 => ProgressBarStatus::Paused,
        _ => ProgressBarStatus::None,
    };
    let progress = (next.0 == 1 || next.0 == 3).then_some(next.1);
    let _ = window.set_progress_bar(ProgressBarState { status: Some(status), progress });
}

fn edit<R: Runtime>(app: &AppHandle<R>, id: &str, f: impl FnOnce(&mut Download)) -> Option<Download> {
    let state = app.state::<Downloads>();
    let mut list = state.list.lock().ok()?;
    let item = list.iter_mut().find(|d| d.id == id)?;
    f(item);
    Some(item.clone())
}

/// The download as it stands (for the add-on installer).
/// The game's row in Downloads goes with the game: its Play and Open folder had
/// nothing to open any more.
pub fn forget_game<R: Runtime>(app: &AppHandle<R>, game_id: &str) {
    if let Ok(mut list) = app.state::<Downloads>().list.lock() {
        list.retain(|d| !(d.status == Status::Installed && d.game_id.as_deref() == Some(game_id)));
    }
    persist(app);
    emit(app, true);
}

const KEPT_FOR_RETRY: &str = "then press Retry. The download is kept.";

/// Refused for a game that is running: its files are in use.
pub(crate) fn close_first(title: &str) -> String {
    format!("Close {title} first, {KEPT_FOR_RETRY}")
}

/// This game's new files are being unpacked, or merged over the old ones.
pub fn unpacking<R: Runtime>(app: &AppHandle<R>, slug: &str) -> bool {
    let state = app.state::<Downloads>();
    let Ok(list) = state.list.lock() else { return false };
    list.iter().any(|d| d.slug.as_deref() == Some(slug) && d.addon.is_none() && d.status == Status::Extracting)
}

pub fn snapshot<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<Download> {
    edit(app, id, |_| {})
}

pub fn set_status_pub<R: Runtime>(app: &AppHandle<R>, id: &str, status: Status, error: Option<String>) {
    set_status(app, id, status, error)
}

pub fn extract_progress<R: Runtime>(app: &AppHandle<R>, id: &str, done: u64, total: Option<u64>) {
    edit(app, id, |d| {
        d.extracted = done;
        d.extract_total = total;
    });
    emit(app, false);
}

/// Mark a download installed, pointing at the game it went into.
pub fn finish_as<R: Runtime>(app: &AppHandle<R>, id: &str, game: &LibraryGame) {
    edit(app, id, |d| {
        d.install_dir = Some(game.install_dir.clone());
        d.game_id = Some(game.id.clone());
    });
    set_status(app, id, Status::Installed, None);
}

fn set_status<R: Runtime>(app: &AppHandle<R>, id: &str, status: Status, error: Option<String>) {
    edit(app, id, |d| {
        d.status = status;
        d.error = error;
        d.speed = 0;
        if matches!(status, Status::Paused | Status::Failed | Status::Installed | Status::Canceled) {
            d.notice = None;
        }
        if matches!(status, Status::Installed | Status::Failed | Status::Canceled) {
            d.finished_at = Some(now());
        }
    });
    persist(app);
    emit(app, true);
}

/// Load what was left from last time. Anything that was moving is paused -
/// the player resumes it, rather than the client silently using the network.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let Some(file) = state_file(app) else { return };
    let mut list: Vec<Download> = std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for d in &mut list {
        // Still waiting for its folder: asked again (src/downloads/InstallFolderPrompt.tsx).
        if d.ask_folder && d.status == Status::Queued {
            continue;
        }
        if matches!(d.status, Status::Queued | Status::Resolving | Status::Downloading | Status::Verifying | Status::Extracting) {
            d.status = Status::Paused;
            d.speed = 0;
        }
        d.notice = None;
        // The ranges are only ever saved once their bytes are on the disk;
        // `received` is the live count and can be ahead of them after a crash.
        if !d.segments.is_empty() {
            d.received = d.segments.iter().map(|s| s.done.min(s.len())).sum();
        }
    }
    if let Ok(mut l) = app.state::<Downloads>().list.lock() {
        *l = list;
    }
}

/// Whether a download from the Store should be taken over: only kryo.to's own
/// files. A mirror elsewhere downloads the way a browser would.
pub fn is_ours(url: &url::Url, settings: &crate::settings::Settings) -> bool {
    let host = url.host_str().unwrap_or("");
    if url.scheme() == "https" && (host == "kryo.to" || host.ends_with(".kryo.to")) {
        return true;
    }
    if crate::settings::is_catalog_origin(url, settings) {
        return true;
    }
    // Debug builds only: a local file server standing in for dl.kryo.to, so the
    // whole download-to-play path can be exercised without a real release.
    // Compiled out of release builds entirely.
    #[cfg(debug_assertions)]
    if let Ok(dev) = std::env::var("KRYOTO_DEV_DOWNLOAD_HOST") {
        let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
        return !dev.is_empty() && format!("{host}{port}") == dev;
    }
    false
}

/// The placeholder a download is called until something better is known.
const UNNAMED: &str = "Download";

/// Whether `title` is only a stand-in: empty, the placeholder, or the slug.
fn is_placeholder(title: &str, slug: Option<&str>) -> bool {
    let t = title.trim();
    t.is_empty() || t.eq_ignore_ascii_case(UNNAMED) || slug.is_some_and(|s| s.eq_ignore_ascii_case(t))
}

/// The name dl.kryo.to will save a link under, read from the link itself.
///
/// A `/d/<token>` link carries its file name in the token: the part before the
/// dot is base64url JSON (`{"k":…,"exp":…,"n":"Hades II - Kryoto.7z"}`). It is
/// signed, not secret, so it can be read for a label the moment the download
/// is handed over, before a byte has arrived. Only ever used as a name.
pub fn file_name_in_link(url: &str) -> Option<String> {
    let url: url::Url = url.parse().ok()?;
    let mut parts = url.path_segments()?;
    if parts.next()? != "d" {
        return None;
    }
    let body = parts.next()?.split('.').next()?;
    let json: serde_json::Value = serde_json::from_slice(&base64url(body)?).ok()?;
    json["n"].as_str().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 255).map(String::from)
}

/// The address family a `/d/<token>` link is bound to, as a local address to
/// connect from: `0.0.0.0` for an IPv4 binding, `::` for IPv6.
///
/// dl.kryo.to binds a link to the network that asked for it (`/24` or `/64`),
/// and the page asked through the WebView. On a PC with both IPv4 and IPv6
/// the WebView and this downloader can each pick a different one, and the
/// link was then refused as "made for another network" (88 reports in 0.2.3).
/// Connecting over the same family as the binding makes the two agree.
pub fn link_family(url: &str) -> Option<std::net::IpAddr> {
    let url: url::Url = url.parse().ok()?;
    let mut parts = url.path_segments()?;
    if parts.next()? != "d" {
        return None;
    }
    let body = parts.next()?.split('.').next()?;
    let json: serde_json::Value = serde_json::from_slice(&base64url(body)?).ok()?;
    let ip = json["ip"].as_str()?;
    Some(if ip.contains(':') {
        std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
    } else {
        std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
    })
}

/// Decode unpadded base64url. `None` for anything that is not.
fn base64url(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32)
    };
    let text = text.trim_end_matches('=');
    if text.len() > 8192 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Queue a download the Store handed over. `slug` is the game page it came from.
pub fn enqueue<R: Runtime>(app: &AppHandle<R>, url: String, slug: Option<String>, title: Option<String>) {
    if let Err(e) = enqueue_item(app, url, slug, title, None, None, None) {
        let _ = app.emit("notify", Notice::new("Could not queue download", &e, None).sound("error"));
    }
}

pub fn enqueue_remote<R: Runtime>(app: &AppHandle<R>, command: &str, url: String, slug: Option<String>, title: Option<String>) -> Result<(), String> {
    enqueue_item(app, url, slug, title, None, None, Some(format!("remote-{command}")))
}

/// A release only on mirrors, sent from kryo.to (Remote, Kryoto Mobile): one of
/// its mirror pages, downloaded like a mirror picked in the Store.
pub fn enqueue_remote_mirror<R: Runtime>(
    app: &AppHandle<R>,
    command: &str,
    page: String,
    slug: Option<String>,
    title: Option<String>,
    release: Option<String>,
) -> Result<(), String> {
    let (_, host) = crate::resolvers::classify(&page).ok_or("Kryoto can't download from that host yet.")?;
    enqueue_item(app, page.clone(), slug, title, Some(Mirror { page, host: host.into() }), release, Some(format!("remote-{command}")))
}

/// Queue a download from a link kryo.to minted for this PC, as pressing
/// Download in the Store does: a game already downloading is not queued twice.
pub fn enqueue_url<R: Runtime>(app: &AppHandle<R>, url: String, slug: Option<String>, title: Option<String>) -> Result<(), String> {
    enqueue_item(app, url, slug, title, None, None, None)
}

pub fn remote_control<R: Runtime>(app: &AppHandle<R>, id: &str, status: Status) -> Result<(), String> {
    if edit(app, id, |d| d.status = status).is_none() { return Err("That download is no longer in the list.".into()); }
    persist_checked(app)?;
    emit(app, true);
    Ok(())
}

/// Queue a download from one of a game's mirrors, or any other link it lists:
/// resolved to its file when its turn comes (`resolvers`), then fetched and
/// installed like any other.
pub fn enqueue_mirror<R: Runtime>(
    app: &AppHandle<R>,
    page: String,
    slug: Option<String>,
    title: Option<String>,
    release: Option<String>,
) -> Result<(), String> {
    let (_, host) = crate::resolvers::classify_any(&page).ok_or("Kryoto can't download from that link. Open it in your browser instead.")?;
    enqueue_item(app, page.clone(), slug, title, Some(Mirror { page, host }), release, None)
}

fn enqueue_item<R: Runtime>(
    app: &AppHandle<R>,
    url: String,
    slug: Option<String>,
    title: Option<String>,
    mirror: Option<Mirror>,
    release: Option<String>,
    remote_id: Option<String>,
) -> Result<(), String> {
    let state = app.state::<Downloads>();
    if let Some(id) = &remote_id {
        if state.list.lock().map_err(|_| "download list lock poisoned")?.iter().any(|d| &d.id == id) {
            return persist_checked(app);
        }
    }
    // Download pressed again for a file whose download stopped part way (its
    // link ran out, or it failed): carry that one on with the fresh link
    // instead of starting a second from zero. Every range is checked against
    // the file it started from, so a different build under the same name
    // simply starts over.
    if mirror.is_none() && remote_id.is_none() {
        if let Some(found) = unfinished_copy(&state, &url, slug.as_deref()) {
            edit(app, &found, |d| {
                d.url = url.clone();
                d.error = None;
                take_request(d, release.clone());
            });
            persist(app);
            let _ = app.emit("notify", Notice::new("Picking up where it left off", "That download carries on from where it stopped.", None));
            let _ = app.emit("download-started", found.clone());
            let _ = requeue(app, &found);
            return Ok(());
        }
    }
    // Download pressed again for a game whose download failed with nothing
    // kept (a damaged archive is deleted): that row starts over with the fresh
    // link, rather than a second row of the same game failing beside it.
    if mirror.is_none() && remote_id.is_none() {
        if let Some(found) = failed_copy(&state, slug.as_deref()).filter(|_| !is_dlc_link(&url)) {
            edit(app, &found, |d| {
                d.url = url.clone();
                d.error = None;
                d.sha256 = None;
                d.verified = false;
                take_request(d, release.clone());
            });
            persist(app);
            let _ = app.emit("download-started", found.clone());
            let _ = requeue(app, &found);
            return Ok(());
        }
    }
    // The same game asked for twice while the first is still going, here or
    // from kryo.to's "Download in Kryoto Desktop".
    if let Ok(list) = state.list.lock() {
        if list.iter().any(|d| {
            d.slug.is_some()
                && d.slug == slug
                && same_request(d, &url, mirror.is_some())
                && matches!(d.status, Status::Queued | Status::Resolving | Status::Downloading | Status::Verifying | Status::Extracting)
        }) {
            let _ = app.emit("notify", Notice::new("Already downloading", "That game is already in Downloads.", None));
            return Ok(());
        }
    }
    let from_here = remote_id.is_none();
    let id = remote_id.unwrap_or_else(|| format!("dl-{}", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)));
    // The page's title first; then the name the link itself carries, so a
    // download that arrives with nothing else is still called after its game
    // from the first frame rather than "Download".
    let named = title
        .filter(|t| !is_placeholder(t, slug.as_deref()))
        .or_else(|| file_name_in_link(&url).and_then(|n| title_in_file_name(&n)));
    // At the back of the queue.
    let queue_order = state.list.lock().map(|l| l.iter().map(|d| d.queue_order).max().unwrap_or(0) + 1).unwrap_or(1);
    // Two library folders or more: the player is asked which one, unless it
    // came from another device (nobody may be at this PC) or the game is
    // already installed (an update goes where it is).
    let ask_folder = from_here && {
        let settings = crate::settings::load(app);
        let installed = slug.as_ref().is_some_and(|s| library::load(app).is_ok_and(|games| games.iter().any(|g| g.slug.as_ref() == Some(s))));
        asks_for_folder(&settings, installed)
    };
    let item = Download {
        ask_folder,
        id: id.clone(),
        queue_order,
        meta: CatalogMeta {
            title: named.or_else(|| slug.clone()).unwrap_or_else(|| UNNAMED.into()),
            ..Default::default()
        },
        slug,
        url,
        added_at: now(),
        mirror,
        release,
        ..Default::default()
    };
    state.list.lock().map_err(|_| "download list lock poisoned")?.insert(0, item);
    if let Err(e) = persist_checked(app) {
        if let Ok(mut list) = state.list.lock() { list.retain(|d| d.id != id); }
        return Err(format!("Could not save the download queue: {e}"));
    }
    emit(app, true);
    let _ = app.emit("download-started", id.clone());
    if !ask_folder {
        start(app.clone(), id);
    }
    Ok(())
}

/// Queue the games of a collection, in order, into `library_dir` (none is the
/// default folder). Each takes its place like any download and gets its link
/// at its turn. One already on its way is left as it is. How many were queued.
pub fn enqueue_collection<R: Runtime>(app: &AppHandle<R>, games: &[crate::collections::Game], library_dir: Option<String>) -> Result<usize, String> {
    let state = app.state::<Downloads>();
    let mut added: Vec<String> = Vec::new();
    {
        let mut list = state.list.lock().map_err(|_| "download list lock poisoned")?;
        let mut order = list.iter().map(|d| d.queue_order).max().unwrap_or(0);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        for (n, g) in games.iter().enumerate() {
            let going = |d: &Download| d.slug.as_deref() == Some(g.slug.as_str()) && matches!(d.status, Status::Queued | Status::Resolving | Status::Downloading | Status::Verifying | Status::Extracting | Status::Paused);
            if list.iter().any(going) {
                continue;
            }
            order += 1;
            let id = format!("dl-{stamp}-{n}");
            added.push(id.clone());
            list.insert(0, Download {
                id,
                queue_order: order,
                slug: Some(g.slug.clone()),
                added_at: now(),
                total: g.size_bytes,
                library_dir: library_dir.clone(),
                mint_at_turn: true,
                meta: CatalogMeta { title: g.title.clone(), cover: g.cover.clone(), nsfw: g.nsfw, ..Default::default() },
                ..Default::default()
            });
        }
    }
    if let Err(e) = persist_checked(app) {
        if let Ok(mut list) = state.list.lock() {
            list.retain(|d| !added.contains(&d.id));
        }
        return Err(format!("Could not save the download queue: {e}"));
    }
    emit(app, true);
    for id in &added {
        start(app.clone(), id.clone());
    }
    Ok(added.len())
}

/// What a row added from a game's Add-ons & DLC card says when kryo.to wants
/// the check only its page can pass (a new account, say). The card offers the
/// game's download window for it.
pub const ADDON_CHALLENGE: &str = "kryo.to wants a check only its download window can pass for this account. Open the download window to add it there.";

/// One of a game's add-ons, as its Add-ons & DLC card asks for it.
pub struct AddonRequest {
    /// The add-on's release id on kryo.to.
    pub release: String,
    /// Its label there ("DLC: Firebird", "Japanese language pack").
    pub label: String,
    /// kind `repair`: applied under the repair journal (`addons::install_fix`).
    pub repair: bool,
    /// Only on mirrors: the page Kryoto downloads it from.
    pub mirror: Option<Mirror>,
    pub total: Option<u64>,
}

/// Queue an add-on for an installed game, straight from its card: no page,
/// no game to pick. Its link is minted at its turn, like a collection's games,
/// and it applies to `game` when it lands. Asked again while it is still
/// going (or after it failed), the same row carries on. Its download id.
pub fn enqueue_addon<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame, req: AddonRequest) -> Result<String, String> {
    let state = app.state::<Downloads>();
    let slug = game.slug.clone().ok_or("This game is not from kryo.to.")?;
    let found = {
        let list = state.list.lock().map_err(|_| "download list lock poisoned")?;
        list.iter()
            .find(|d| d.slug.as_deref() == Some(slug.as_str()) && d.addon_release.as_deref() == Some(req.release.as_str()) && !matches!(d.status, Status::Installed | Status::Canceled))
            .map(|d| (d.id.clone(), d.status))
    };
    if let Some((id, status)) = found {
        if matches!(status, Status::Failed | Status::Paused) {
            requeue(app, &id)?;
        }
        return Ok(id);
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let id = format!("dl-{stamp}-addon-{}", req.release);
    let queue_order = state.list.lock().map(|l| l.iter().map(|d| d.queue_order).max().unwrap_or(0) + 1).unwrap_or(1);
    // Named as a DLC fetched from the Store is ("Forts DLC - Firebird"), so a
    // row in Downloads and the side rail says which add-on it is, not only
    // whose: seven DLC queued at once read as seven rows of the game itself.
    let title = match req.label.strip_prefix("DLC:").or_else(|| req.label.strip_prefix("dlc:")) {
        Some(name) => format!("{} DLC - {}", game.title, name.trim()),
        None => format!("{} - {}", game.title, req.label),
    };
    let item = Download {
        id: id.clone(),
        queue_order,
        slug: Some(slug),
        game_id: Some(game.id.clone()),
        added_at: now(),
        total: req.total,
        addon: Some(req.label),
        repair: req.repair,
        addon_release: Some(req.release),
        mint_at_turn: req.mirror.is_none(),
        url: req.mirror.as_ref().map(|m| m.page.clone()).unwrap_or_default(),
        mirror: req.mirror,
        // The game's own details: a version here means kryo.to is not asked
        // for the game's page (fill_meta), whose size is not this file's.
        meta: CatalogMeta {
            title,
            cover: game.cover.clone(),
            hero: game.hero.clone(),
            version: Some(game.version.clone().unwrap_or_default()),
            nsfw: game.nsfw,
            ..Default::default()
        },
        ..Default::default()
    };
    state.list.lock().map_err(|_| "download list lock poisoned")?.insert(0, item);
    if let Err(e) = persist_checked(app) {
        if let Ok(mut list) = state.list.lock() {
            list.retain(|d| d.id != id);
        }
        return Err(format!("Could not save the download queue: {e}"));
    }
    emit(app, true);
    start(app.clone(), id.clone());
    Ok(id)
}

/// Whether a new download asks where to install. Pure, for the tests.
pub fn asks_for_folder(settings: &crate::settings::Settings, installed: bool) -> bool {
    settings.ask_install_folder && !installed && crate::settings::all_folders(settings).len() > 1
}

/// The answer to "where should it go?": `folder` is one of the library
/// folders. `always_default` stops the question for good (Settings > Storage
/// can turn it back on). Then the download takes its place in the queue.
#[tauri::command]
pub fn download_pick_folder(app: AppHandle, id: String, folder: String, always_default: bool) -> Result<(), String> {
    let mut settings = crate::settings::load(&app);
    let folders = crate::settings::all_folders(&settings);
    let folder = folders.iter().find(|f| crate::settings::same_path(f, &folder)).cloned().ok_or("That is not one of your library folders.")?;
    if always_default && settings.ask_install_folder {
        settings.ask_install_folder = false;
        crate::settings::write(&app, &settings)?;
    }
    let chosen = (!crate::settings::same_path(&folder, &settings.library_dir)).then_some(folder);
    edit(&app, &id, |d| {
        d.library_dir = chosen;
        d.ask_folder = false;
    })
    .ok_or("That download is gone.")?;
    persist(&app);
    emit(&app, true);
    if edit(&app, &id, |_| {}).is_some_and(|d| d.status == Status::Queued) {
        start(app.clone(), id);
    }
    Ok(())
}

/// Settings for one download's run: its own library folder, when the player
/// picked one that is still there, in place of the default.
fn settings_for<R: Runtime>(app: &AppHandle<R>, id: &str) -> crate::settings::Settings {
    let mut settings = crate::settings::load(app);
    if let Some(dir) = edit(app, id, |_| {}).and_then(|d| d.library_dir).filter(|d| Path::new(d).is_dir()) {
        settings.library_dir = dir;
    }
    settings
}

/// The library game this not-yet-started download would install again: the
/// same kryo.to page at the same build, still on disk.
fn installed_build<R: Runtime>(app: &AppHandle<R>, d: &Download) -> Option<LibraryGame> {
    if d.received > 0 || !d.archive_path.is_empty() {
        return None;
    }
    let games = crate::library::load(app).ok()?;
    let game = installed_build_in(&games, d)?;
    Path::new(&game.install_dir).is_dir().then(|| game.clone())
}

/// The library entry that already holds the very build this download asks
/// for. The build asked for is the release picked (`release`), else what
/// kryo.to said the current build is (`meta.version`, fetched for this
/// request). A row reused after a failed try starts with neither (see
/// `take_request`), so an old answer never speaks for a new request: that is
/// how an update was once thrown away as "already installed".
fn installed_build_in<'a>(games: &'a [LibraryGame], d: &Download) -> Option<&'a LibraryGame> {
    let slug = d.slug.as_deref()?;
    let version = d.release.as_deref().or(d.meta.version.as_deref())?;
    // A file whose hash is known and is not the one installed is another
    // upload of that build (replaced on kryo.to): an update, not the same one.
    let same_file = |g: &LibraryGame| match (g.build_sha.as_deref(), d.sha256.as_deref()) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => true,
    };
    games.iter().find(|g| g.slug.as_deref() == Some(slug) && g.version.as_deref() == Some(version) && same_file(g))
}

/// A row reused for a new Download press takes the new request whole: the
/// release asked for now (none means the current build), and none of what
/// kryo.to said about the old request. `run` asks kryo.to again when
/// `meta.version` is unset, so the row is named and sized for the build it
/// is now fetching.
fn take_request(d: &mut Download, release: Option<String>) {
    d.release = release;
    let title = std::mem::take(&mut d.meta.title);
    d.meta = CatalogMeta { title, ..Default::default() };
    // A row with bytes on disk keeps its size: every range of a resume is
    // checked against it (`same_file`). One with nothing kept starts fresh.
    if d.received == 0 && d.archive_path.is_empty() {
        d.total = None;
    }
}

/// A paused or failed download of the same file as `url` (our own, from the
/// same game page), with bytes on disk worth keeping.
fn unfinished_copy(state: &Downloads, url: &str, slug: Option<&str>) -> Option<String> {
    let name = safe_name(&file_name_in_link(url)?);
    let running = state.controls.lock().ok()?.keys().cloned().collect::<Vec<_>>();
    let list = state.list.lock().ok()?;
    list.iter()
        .filter(|d| matches!(d.status, Status::Paused | Status::Failed) && !running.contains(&d.id))
        .filter(|d| d.mirror.is_none() && d.received > 0 && !d.archive_path.is_empty())
        .filter(|d| slug.is_none() || d.slug.as_deref() == slug)
        .find(|d| d.file_name.eq_ignore_ascii_case(&name))
        .map(|d| d.id.clone())
}

/// A failed download of this game with nothing on disk to carry on from.
fn failed_copy(state: &Downloads, slug: Option<&str>) -> Option<String> {
    let slug = slug?;
    let running = state.controls.lock().ok()?.keys().cloned().collect::<Vec<_>>();
    let list = state.list.lock().ok()?;
    list.iter()
        .filter(|d| d.status == Status::Failed && !running.contains(&d.id))
        .filter(|d| d.mirror.is_none() && d.received == 0 && d.archive_path.is_empty() && !is_dlc_link(&d.url))
        .find(|d| d.slug.as_deref() == Some(slug))
        .map(|d| d.id.clone())
}

fn start<R: Runtime>(app: AppHandle<R>, id: String) {
    let flag = Arc::new(AtomicU8::new(RUN));
    if let Ok(mut c) = app.state::<Downloads>().controls.lock() {
        c.insert(id.clone(), flag.clone());
    }
    tauri::async_runtime::spawn(async move {
        let result = run(&app, &id, &flag).await;
        if let Ok(mut c) = app.state::<Downloads>().controls.lock() {
            c.remove(&id);
        }
        match result {
            Ok(()) => {}
            Err(e) => {
                let e = explain_os_error(e);
                // A full disk is the player's to fix and the message says how:
                // in this PC's log, not in the reports to kryo.to, where it was
                // a third of everything sent. So is a game still running.
                if e.starts_with("Not enough space") || e.ends_with(KEPT_FOR_RETRY) {
                    crate::logging::warn("download", &format!("{id}: {e}"));
                } else {
                    crate::logging::error("download", &format!("{id}: {e}"));
                }
                set_status(&app, &id, Status::Failed, Some(e))
            }
        }
    });
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    title: String,
    body: String,
    game_id: Option<String>,
    /// Which of the app's sounds goes with it (`src/lib/sound-recipes.ts`);
    /// none means the plain notification one.
    #[serde(skip_serializing_if = "Option::is_none")]
    sound: Option<&'static str>,
}

impl Notice {
    pub fn new(title: &str, body: &str, game_id: Option<String>) -> Self {
        Self { title: title.into(), body: body.into(), game_id, sound: None }
    }

    pub fn sound(mut self, sound: &'static str) -> Self {
        self.sound = Some(sound);
        self
    }
}

/// The kryo.to game whose title is exactly `name` (letters and digits
/// compared, case aside), from the site's search. Only an exact match: a
/// download filed under the wrong game would install as it.
pub(crate) async fn find_slug(client: &reqwest::Client, endpoint: &str, name: &str) -> Option<String> {
    let squash = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let wanted = squash(name);
    if wanted.is_empty() {
        return None;
    }
    let q: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
    let res = client.get(format!("{endpoint}/api/games/search?q={q}&limit=8")).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    let json: serde_json::Value = res.json().await.ok()?;
    json["results"]
        .as_array()?
        .iter()
        .find(|g| g["title"].as_str().is_some_and(|t| squash(t) == wanted))
        .and_then(|g| g["slug"].as_str())
        .filter(|s| !s.is_empty() && s.len() <= 160 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .map(str::to_ascii_lowercase)
}

pub(crate) async fn fetch_meta(client: &reqwest::Client, endpoint: &str, slug: &str) -> Option<CatalogMeta> {
    let res = client.get(format!("{endpoint}/api/games/{slug}")).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    let json: serde_json::Value = res.json().await.ok()?;
    Some(meta_from_api(&json))
}

/// A `/api/games/<slug>` answer: the game, with its `art` (real, resolved
/// URLs from kryo.to) over anything guessed from the game row.
pub fn meta_from_api(json: &serde_json::Value) -> CatalogMeta {
    let mut meta = meta_from_json(&json["game"]);
    let art = |k: &str| json["art"][k].as_str().map(str::trim).filter(|v| !v.is_empty()).map(String::from);
    if json["art"].is_object() {
        meta.hero = art("hero").or(meta.hero);
        meta.logo = art("logo");
        meta.header = art("header").or(meta.header);
        meta.cover = art("capsule").or(meta.cover);
    }
    meta
}

pub fn meta_from_json(g: &serde_json::Value) -> CatalogMeta {
    let s = |k: &str| g[k].as_str().map(str::trim).filter(|v| !v.is_empty()).map(String::from);
    // A Steam branch listed as its own game carries a suffix ("2027330x");
    // Steam's art is under the number alone.
    let appid = s("steam_appid").map(|a| a.chars().take_while(char::is_ascii_digit).collect::<String>()).filter(|a| !a.is_empty());
    let entries: Vec<LaunchEntry> = serde_json::from_value(g["game_launch_options"]["available"].clone())
        .unwrap_or_default();
    CatalogMeta {
        title: s("title").unwrap_or_default(),
        cover: s("cover_vertical").or_else(|| s("cover")),
        // The legacy Steam path is only a guess (it 404s for every app with
        // hashed art); `meta_from_api` replaces it with the real one.
        hero: s("hero_image_override")
            .or_else(|| g["screenshots"][0].as_str().map(String::from))
            .or_else(|| appid.map(|a| format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{a}/library_hero.jpg"))),
        logo: None,
        header: s("cover_horizontal").or_else(|| s("cover")),
        executable: s("game_executable_path").unwrap_or_default(),
        default_args: s("game_executable_args").unwrap_or_default(),
        entries: entries.into_iter().filter(LaunchEntry::is_windows).collect(),
        source: s("source"),
        version: s("version"),
        short: s("short"),
        developer: s("developer"),
        nsfw: g["nsfw"].as_bool().unwrap_or(false),
        steam_branch: s("steam_branch"),
        size_bytes: g["download_size_bytes"].as_u64(),
        vr: crate::vr::kind_of(&g["vr"]),
    }
}

/// `attachment; filename*=UTF-8''Captain%20Hardcore%20-%20Kryoto.7z`, or the
/// plain `filename="..."` form.
pub fn filename_from_disposition(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    if let Some(at) = lower.find("filename*=") {
        let raw = value[at + 10..].split(';').next()?.trim();
        let encoded = raw.rsplit("''").next()?.trim_matches('"');
        let decoded: String = url::form_urlencoded::parse(format!("x={}", encoded.replace('+', "%2B")).as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())?;
        return Some(decoded);
    }
    if let Some(at) = lower.find("filename=") {
        let raw = value[at + 9..].split(';').next()?.trim().trim_matches('"');
        if !raw.is_empty() {
            return Some(raw.to_string());
        }
    }
    None
}

/// A game's name from its archive's: `Captain Hardcore - Kryoto.7z` is
/// "Captain Hardcore" - the extension and the `- Kryoto` signature every
/// release file carries come off.
/// A download that started with no game to name it after (no page, no
/// catalog answer yet) reads "Download" in the list and its notices. Once the
/// file's own name is known, that is a better name than none.
fn name_from_file(d: &mut Download) {
    if is_placeholder(&d.meta.title, d.slug.as_deref()) {
        if let Some(title) = title_in_file_name(&d.file_name) {
            d.meta.title = title;
        }
    }
}

pub fn title_from_file(name: &str) -> String {
    title_in_file_name(name).unwrap_or_else(|| "Game".into())
}

/// The game's name in a release file's name, or `None` when the name says
/// nothing: a server that sent no file name leaves the last piece of the
/// address - `download`, `file`, a signed token - and none of those is a game.
pub fn title_in_file_name(name: &str) -> Option<String> {
    let name = name.trim();
    // Only a short, known archive extension is an extension: a token's dot
    // splits it into two long runs, and "Portal 2.5" is not "Portal 2".
    let stem = match name.rsplit_once('.') {
        Some((stem, ext)) if (1..=4).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) => stem,
        _ => name,
    }
    .trim();
    let lower = stem.to_ascii_lowercase();
    let stem = if lower.ends_with("- kryoto") { stem[..stem.len() - "- kryoto".len()].trim() } else { stem };
    const GENERIC: [&str; 9] = ["download", "downloads", "file", "files", "game", "archive", "release", "d", "attachment"];
    if stem.is_empty() || GENERIC.iter().any(|g| stem.eq_ignore_ascii_case(g)) {
        return None;
    }
    // A token or a hash: one long word of letters, digits, `-`, `_` and dots.
    let tokenish = stem.len() >= 24 && stem.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if tokenish {
        return None;
    }
    Some(stem.to_string())
}

/// A name that is safe as a single path component on every OS.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { ' ' } else { c })
        .collect();
    // Words that are only dots are `.`/`..` in disguise.
    let cleaned = cleaned
        .split_whitespace()
        .filter(|w| !w.chars().all(|c| c == '.'))
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = cleaned.trim_matches('.').trim().to_string();
    if cleaned.is_empty() { "Game".into() } else { cleaned.chars().take(120).collect() }
}

/// A game folder's name in plain letters: `Baldur’s Gate`, `Ōkami` and
/// `Pokémon™` become `Baldur's Gate`, `Okami` and `Pokemon`. Some games (and
/// the runtimes and anti-tamper they ship) fail to start from a path with a
/// curly quote or an accent in it, so a new install never gets one. What has
/// no plain spelling (a Japanese title) is left out; when nothing is left,
/// `fallback` (the kryo.to slug) names it.
pub fn plain_name(name: &str, fallback: Option<&str>) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        let plain = match c {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{02BC}' | '\u{00B4}' | '\u{2032}' => "'",
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{00AB}' | '\u{00BB}' => "",
            '\u{2010}'..='\u{2015}' | '\u{2212}' => "-",
            '\u{2026}' => "...",
            '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{3000}' => " ",
            '\u{00D7}' => "x",
            '\u{00B2}' => "2",
            '\u{00B3}' => "3",
            '\u{00B9}' => "1",
            'À'..='Å' | 'Ā' | 'Ă' | 'Ą' => "A",
            'à'..='å' | 'ā' | 'ă' | 'ą' => "a",
            'Æ' => "AE",
            'æ' => "ae",
            'Ç' | 'Ć' | 'Č' => "C",
            'ç' | 'ć' | 'č' => "c",
            'Ď' | 'Đ' | 'Ð' => "D",
            'ď' | 'đ' | 'ð' => "d",
            'È'..='Ë' | 'Ē' | 'Ė' | 'Ę' | 'Ě' => "E",
            'è'..='ë' | 'ē' | 'ė' | 'ę' | 'ě' => "e",
            'Ğ' => "G",
            'ğ' => "g",
            'Ì'..='Ï' | 'Ī' | 'İ' => "I",
            'ì'..='ï' | 'ī' | 'ı' => "i",
            'Ł' => "L",
            'ł' => "l",
            'Ñ' | 'Ń' | 'Ň' => "N",
            'ñ' | 'ń' | 'ň' => "n",
            'Ò'..='Ö' | 'Ø' | 'Ō' | 'Ő' => "O",
            'ò'..='ö' | 'ø' | 'ō' | 'ő' => "o",
            'Œ' => "OE",
            'œ' => "oe",
            'Ř' => "R",
            'ř' => "r",
            'Ś' | 'Š' | 'Ş' => "S",
            'ś' | 'š' | 'ş' => "s",
            'ß' => "ss",
            'Ť' => "T",
            'ť' => "t",
            'Þ' => "Th",
            'þ' => "th",
            'Ù'..='Ü' | 'Ū' | 'Ů' | 'Ű' => "U",
            'ù'..='ü' | 'ū' | 'ů' | 'ű' => "u",
            'Ý' | 'Ÿ' => "Y",
            'ý' | 'ÿ' => "y",
            'Ź' | 'Ż' | 'Ž' => "Z",
            'ź' | 'ż' | 'ž' => "z",
            // ™, ®, ©, symbols and scripts with no plain spelling.
            _ => "",
        };
        out.push_str(plain);
    }
    let named = out.chars().any(|c| c.is_ascii_alphanumeric());
    match fallback.filter(|_| !named) {
        Some(slug) => safe_name(slug),
        None => safe_name(&out),
    }
}


/// The game's details from kryo.to, when the row has none yet.
async fn fill_meta<R: Runtime>(app: &AppHandle<R>, id: &str, client: &reqwest::Client, endpoint: &str) {
    let Some(item) = edit(app, id, |_| {}) else { return };
    if !item.meta.executable.is_empty() || item.meta.version.is_some() {
        return;
    }
    let Some(slug) = &item.slug else { return };
    if let Some(meta) = fetch_meta(client, endpoint, slug).await {
        edit(app, id, |d| {
            // The current release's size says nothing about another's.
            if d.release.is_none() {
                d.total = d.total.or(meta.size_bytes);
            }
            let named = std::mem::take(&mut d.meta.title);
            d.meta = meta;
            if d.meta.title.trim().is_empty() {
                d.meta.title = named;
            }
            if let Some(v) = d.release.clone() {
                d.meta.version = Some(v);
                d.meta.size_bytes = None;
            }
        });
        emit(app, true);
    }
}

/// Download pressed for the very build already in the library: point at the
/// game instead of fetching it a second time, and take the row away. A newer
/// build is an update and goes ahead.
async fn already_installed<R: Runtime>(app: &AppHandle<R>, id: &str, client: &reqwest::Client, endpoint: &str) -> bool {
    let Some(d) = edit(app, id, |_| {}) else { return false };
    let installed = installed_build(app, &d);
    // kryo.to is only asked when its answer changes what happens: a mirror page
    // names no file, so even a game not installed may be a second press.
    if installed.is_none() && d.mirror.is_none() {
        return false;
    }
    let json = downloads_json(client, endpoint, &d).await;
    // The same build number, but kryo.to's file is another one now (replaced,
    // or a new upload that has no number): that is an update, not a second press.
    let installed = installed.filter(|g| !json.as_ref().is_some_and(|j| replaced(j, g.build_sha.as_deref(), d.release.as_deref())));
    // "Get the files back" on the game's page: the same build again, over its folder.
    let installed = installed.filter(|g| !g.slug.as_deref().is_some_and(crate::antivirus::restoring));
    match row_kind(&d, json.as_ref()) {
        // An add-on shares its game's page and build, so a DLC for an up-to-date
        // game read as the game itself, and its row was thrown away unapplied.
        FileKind::Addon(label) => {
            if !label.is_empty() {
                edit(app, id, |d| d.addon = Some(label));
            }
            return false;
        }
        // A mirror kryo.to says nothing about: settled once it is fetched
        // (`already_had`), when its name is known.
        FileKind::Unknown if d.mirror.is_some() => return false,
        _ => {}
    }
    if let Some(game) = installed {
        let body = format!("{} is already in your Library, on this build.", game.title);
        drop_row(app, id);
        let _ = app.emit("notify", Notice::new("Already installed", &body, Some(game.id)));
        return true;
    }
    if d.mirror.is_some() && game_already_going(app, &d) {
        drop_row(app, id);
        let _ = app.emit("notify", Notice::new("Already downloading", "That game is already in Downloads.", None));
        return true;
    }
    false
}

fn drop_row<R: Runtime>(app: &AppHandle<R>, id: &str) {
    if let Ok(mut list) = app.state::<Downloads>().list.lock() {
        list.retain(|d| d.id != id);
    }
    persist(app);
    emit(app, true);
}

/// Another row is already fetching this row's game, not one of its add-ons.
fn game_already_going<R: Runtime>(app: &AppHandle<R>, d: &Download) -> bool {
    let state = app.state::<Downloads>();
    let Ok(list) = state.list.lock() else { return false };
    list.iter().any(|o| {
        o.id != d.id
            && o.slug == d.slug
            && o.addon.is_none()
            && !is_dlc_link(&o.url)
            && !is_dlc_file(&o.file_name)
            && matches!(o.status, Status::Queued | Status::Resolving | Status::Downloading | Status::Verifying | Status::Extracting)
    })
}

/// A mirror row that could not be placed before it was fetched, now that its
/// file is here: the game itself, at the build already installed, goes no further.
fn already_had<R: Runtime>(app: &AppHandle<R>, id: &str) -> bool {
    let Some(d) = edit(app, id, |_| {}) else { return false };
    // "Get the files back" (antivirus.rs): the same build, wanted over the folder.
    if d.mirror.is_none() || d.addon.is_some() || d.slug.as_deref().is_some_and(crate::antivirus::restoring) {
        return false;
    }
    let Ok(games) = crate::library::load(app) else { return false };
    let Some(game) = installed_build_in(&games, &d).filter(|g| Path::new(&g.install_dir).is_dir()) else { return false };
    let body = format!("{} is already in your Library, on this build.", game.title);
    let _ = app.emit("notify", Notice::new("Already installed", &body, Some(game.id.clone())));
    let _ = std::fs::remove_file(&d.archive_path);
    drop_row(app, id);
    true
}

/// Forge names a DLC's archive `<game> DLC - <name>`.
fn is_dlc_file(name: &str) -> bool {
    name.to_ascii_lowercase().contains(" dlc - ")
}

fn is_dlc_link(url: &str) -> bool {
    file_name_in_link(url).is_some_and(|name| is_dlc_file(&name))
}

/// The label kryo.to gives a DLC, from Forge's name for its file.
fn dlc_label(file: &str) -> Option<String> {
    let at = file.to_ascii_lowercase().find(" dlc - ")? + " dlc - ".len();
    let name = file[at..].rsplit_once('.').map_or(&file[at..], |(stem, _)| stem);
    let name = name.strip_suffix(" - Kryoto").or(name.strip_suffix(" - kryoto")).unwrap_or(name);
    Some(format!("DLC: {}", name.trim()))
}

/// Whether a press for `new` asks for what the row `going` already fetches. A
/// game's DLC sit on its page, so they are told apart by file: "Download all
/// DLC" sends them one after another, and only the first used to get in. A
/// mirror page names no file, so only the same page counts here; the rest is
/// settled when its turn to ask kryo.to comes (`already_installed`).
fn same_request(going: &Download, new: &str, new_is_mirror: bool) -> bool {
    if new_is_mirror {
        return going.mirror.as_ref().is_some_and(|m| m.page == new);
    }
    let going_addon = going.addon.is_some() || is_dlc_link(&going.url) || is_dlc_file(&going.file_name);
    match (going_addon, is_dlc_link(new)) {
        (false, false) => true,
        (true, true) => file_name_in_link(&going.url) == file_name_in_link(new),
        _ => false,
    }
}

/// What kryo.to's `/downloads` answer makes of a file or a mirror page.
#[derive(Debug, PartialEq)]
enum FileKind {
    /// One of the game's add-ons, with its label (empty when only its name says so).
    Addon(String),
    Game,
    Unknown,
}

fn file_kind(json: &serde_json::Value, file: Option<&str>, page: Option<&str>) -> FileKind {
    let list = |v: &serde_json::Value, key: &str| v[key].as_array().cloned().unwrap_or_default();
    let named = |l: &serde_json::Value| file.is_some_and(|f| l["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(f)));
    for addon in list(json, "addons") {
        let by_page = page.is_some_and(|p| list(&addon, "pages").iter().any(|u| u.as_str() == Some(p)));
        if by_page || list(&addon, "links").iter().chain(list(&addon, "hashes").iter()).any(named) {
            return FileKind::Addon(addon["label"].as_str().unwrap_or("Add-on").to_string());
        }
    }
    let lists_it = |l: &serde_json::Value| named(l) || page.is_some_and(|p| l["url"].as_str() == Some(p));
    let in_a_release = list(json, "releases").iter().any(|r| list(r, "links").iter().chain(list(r, "hashes").iter()).any(lists_it));
    if in_a_release || list(json, "links").iter().any(lists_it) {
        return FileKind::Game;
    }
    FileKind::Unknown
}

/// What this row fetches. A DLC's link says so; anything else is asked of kryo.to.
/// `/api/games/<slug>/downloads` for this row's game, when it names one.
async fn downloads_json(client: &reqwest::Client, endpoint: &str, d: &Download) -> Option<serde_json::Value> {
    let slug = d.slug.as_deref()?;
    let res = client.get(format!("{endpoint}/api/games/{slug}/downloads")).send().await.ok()?;
    res.json::<serde_json::Value>().await.ok()
}

/// The build installed from an archive whose SHA-256 was `installed` is not
/// the one kryo.to has now: the release asked for (`release`), else the
/// current one, lists hashes and none is that one. Nothing known, nothing
/// replaced: an install from before the hash was kept, or a release with no
/// hash of its own, compares by version as before.
pub fn replaced(json: &serde_json::Value, installed: Option<&str>, release: Option<&str>) -> bool {
    let Some(installed) = installed.filter(|s| s.len() == 64) else { return false };
    let releases = json["releases"].as_array().cloned().unwrap_or_default();
    let picked = match release {
        Some(v) => releases.iter().find(|r| r["version"].as_str() == Some(v)),
        None => releases.iter().find(|r| r["primary"].as_bool() == Some(true)),
    };
    let hashes = picked.map(|r| &r["hashes"]).unwrap_or(&json["hashes"]);
    let list: Vec<String> = hashes.as_array().into_iter().flatten().filter_map(|h| h["sha256"].as_str()).map(str::to_ascii_lowercase).collect();
    !list.is_empty() && !list.iter().any(|h| h.eq_ignore_ascii_case(installed))
}

fn row_kind(d: &Download, json: Option<&serde_json::Value>) -> FileKind {
    if d.addon_release.is_some() || is_dlc_link(&d.url) {
        return FileKind::Addon(String::new());
    }
    let file = file_name_in_link(&d.url);
    let page = d.mirror.as_ref().map(|m| m.page.as_str());
    match json {
        Some(json) if file.is_some() || page.is_some() => file_kind(json, file.as_deref(), page),
        _ => FileKind::Unknown,
    }
}

/// A collection's row has reached its turn: its link now, minted for this
/// PC, or the mirror page when the game is only on mirrors.
async fn mint_link<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<(), String> {
    let d = edit(app, id, |_| {}).ok_or("The download is gone.")?;
    if d.mirror.is_some() {
        return Ok(());
    }
    // An add-on from a game's Add-ons & DLC card: that release's own link.
    if let Some(release) = &d.addon_release {
        let minted = match crate::phone::session(app) {
            Ok(session) => crate::phone::mint_release(app, &session, release, None).await,
            Err(e) => Err(crate::phone::MintError::Other(e)),
        };
        match minted {
            Ok(url) => {
                edit(app, id, |d| d.url = url);
            }
            Err(crate::phone::MintError::Other(e)) => return Err(e),
            Err(crate::phone::MintError::Challenge) => return Err(ADDON_CHALLENGE.into()),
        }
        persist(app);
        return Ok(());
    }
    let slug = d.slug.ok_or("This download has no game to ask kryo.to about.")?;
    match crate::phone::mint(app, &slug, None, true).await {
        Ok(crate::phone::Minted::Link { url, .. }) => {
            edit(app, id, |d| d.url = url);
        }
        Ok(crate::phone::Minted::Mirror { page, .. }) => {
            let host = crate::resolvers::classify(&page).map(|(_, h)| h.to_string()).unwrap_or_default();
            edit(app, id, |d| {
                d.url = page.clone();
                d.mirror = Some(Mirror { page, host });
            });
        }
        Err(crate::phone::MintError::Other(e)) => return Err(e),
        Err(crate::phone::MintError::Challenge) => {
            return Err(format!("kryo.to wants a check only its page can pass. Press Download on {}'s page, or Retry later.", d.meta.title));
        }
    }
    persist(app);
    Ok(())
}

async fn run<R: Runtime>(app: &AppHandle<R>, id: &str, flag: &Arc<AtomicU8>) -> Result<(), String> {
    let settings = crate::settings::load(app);
    let client = client_builder().build().map_err(|e| e.to_string())?;

    // Named before waiting for a free slot, so a queued download reads as its
    // game rather than as its slug until the ones ahead of it finish.
    let mut item = edit(app, id, |_| {}).ok_or("The download is gone.")?;
    let endpoint = crate::settings::catalog_endpoint(&settings);
    // Nothing said which game this is (no page, no context the page could
    // pass on): look it up on kryo.to by the name the file downloads as, so
    // it still installs with its art, its exe and its place in the library.
    if item.slug.is_none() {
        let name = file_name_in_link(&item.url)
            .and_then(|n| title_in_file_name(&n))
            .or_else(|| (!is_placeholder(&item.meta.title, None)).then(|| item.meta.title.clone()));
        if let Some(name) = name {
            if let Some(slug) = find_slug(&client, &endpoint, &name).await {
                item = edit(app, id, |d| d.slug = Some(slug)).ok_or("The download is gone.")?;
                emit(app, true);
            }
        }
    }
    // A collection's rows wait with what the collection said about them; a
    // hundred of them must not all ask kryo.to at once. They ask at their turn.
    if !item.mint_at_turn {
        fill_meta(app, id, &client, &endpoint).await;
        if already_installed(app, id, &client, &endpoint).await {
            return Ok(());
        }
    }

    let Some(_turn) = wait_for_turn(app, id, flag).await else {
        return settle_stopped(app, id, flag);
    };
    // Settings as they are now, not as they were when it was queued.
    let settings = settings_for(app, id);

    if item.mint_at_turn {
        set_status(app, id, Status::Resolving, None);
        fill_meta(app, id, &client, &endpoint).await;
        if already_installed(app, id, &client, &endpoint).await {
            return Ok(());
        }
        mint_link(app, id).await?;
    }

    let is_mirror = edit(app, id, |_| {}).is_some_and(|d| d.mirror.is_some());
    let (mut client, mut connections) = (client.clone(), settings.connections);
    // Our own link: connect over the address family it was bound to, and keep
    // the resume pass dl.kryo.to hands out with the first range, so a range
    // retried from another network after a drop is not refused.
    if !is_mirror {
        let family = edit(app, id, |_| {}).and_then(|d| link_family(&d.url));
        let mut builder = client_builder().cookie_store(true);
        if let Some(local) = family {
            builder = builder.local_address(local);
        }
        if let Ok(ours) = builder.build() {
            client = ours;
        }
    }
    let already_complete = {
        let d = edit(app, id, |_| {}).ok_or("The download is gone.")?;
        let fetched = !d.archive_path.is_empty() && d.total.is_some_and(|t| t > 0 && d.received >= t) && d.segments.iter().all(|s| s.done >= s.len());
        // Fetched once, but the file has gone since (an antivirus took it, or
        // somebody tidied the folder): Retry fetches it again. It used to fail
        // the same way for ever, "Could not read the archive to check it".
        // An unpack in place has eaten its archive on purpose.
        let path = Path::new(&d.archive_path);
        let there = path.is_file() || crate::hatchzip::resumes(path);
        if fetched && !there {
            crate::logging::warn("download", &format!("{id}: the finished archive is gone; fetching it again"));
            discard(app, id);
        }
        fetched && there
    };
    if !already_complete {
        if is_mirror {
            match mirror_client(app, id, flag, false, settings.connections).await {
                Ok(v) => (client, connections) = v,
                Err(_) if flag.load(Ordering::SeqCst) != RUN => return settle_stopped(app, id, flag),
                Err(e) => return Err(e),
            }
        }
        set_status(app, id, Status::Downloading, None);
        let limiter = Arc::new(Limiter::new(settings.speed_limit_mb));
        // filez.stream: ciphertext by the host's grants, opened chunk by chunk here.
        let filez = if is_mirror { edit(app, id, |_| {}).and_then(|d| crate::resolvers::filez::file_for(&d.url)) } else { None };
        let done = match filez {
            Some(file) => fetch_filez(app, id, flag, &mut client, &settings, &limiter, file).await?,
            None => fetch_all(app, id, flag, &mut client, &mut connections, &settings, &limiter, is_mirror).await?,
        };
        if !done {
            return settle_stopped(app, id, flag);
        }
    }
    identify(app, id, &client).await;
    // A DLC kryo.to's answer did not place (it was out of reach, or leaves links
    // out for this caller) still says what it is in its name.
    edit(app, id, |d| {
        if d.addon.is_none() {
            d.addon = dlc_label(&d.file_name);
        }
    });
    if already_had(app, id) {
        return Ok(());
    }
    // A mismatch is never downloaded again by itself: a big archive twice is a
    // lot of bandwidth, and the player chooses between Install anyway and Retry.
    match verify(app, id, flag).await {
        Ok(()) => {}
        Err(_) if flag.load(Ordering::SeqCst) != RUN => return settle_stopped(app, id, flag),
        Err(e) => return Err(e),
    }
    edit(app, id, |d| d.notice = None);
    if edit(app, id, |_| {}).and_then(|d| d.addon).is_some() {
        return crate::addons::install(app, id, &settings).await;
    }
    match install(app, id, &settings, flag).await {
        Err(_) if flag.load(Ordering::SeqCst) != RUN => settle_stopped(app, id, flag),
        done => done,
    }
}

/// Fetch every byte of the archive, riding out dropped connections.
/// `Ok(true)` when it is all on disk, `Ok(false)` when paused or cancelled.
#[allow(clippy::too_many_arguments)]
/// Plaintext chunks per range of a filez.stream download: 16 MiB, so a
/// resume loses at most that, and the host's grants come in useful sizes.
const FILEZ_SEGMENT_CHUNKS: u64 = 16;
/// Connections to filez.stream at once (its Worker pipes every byte).
const FILEZ_CONNECTIONS: usize = 4;

/// The ranges of a filez download, on whole sealed chunks.
fn filez_segments(size: u64) -> Vec<Segment> {
    use crate::resolvers::filez::{chunk_count, CHUNK};
    let step = FILEZ_SEGMENT_CHUNKS * CHUNK;
    (0..chunk_count(size).div_ceil(FILEZ_SEGMENT_CHUNKS))
        .map(|i| Segment { start: i * step, end: ((i + 1) * step).min(size).max(1) - 1, done: 0 })
        .collect()
}

/// A filez.stream download (resolvers/filez.rs): the ciphertext of each 16 MiB
/// range fetched with the host's grant and its check's pass, every 1 MiB chunk
/// opened and checked here, and the plaintext written in its place in the
/// archive. A range counts once all of it is on the disk, so a resume picks up
/// at the first unfinished one. `Ok(false)`: paused or cancelled.
#[allow(clippy::too_many_arguments)]
async fn fetch_filez<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    flag: &Arc<AtomicU8>,
    client: &mut reqwest::Client,
    settings: &crate::settings::Settings,
    limiter: &Arc<Limiter>,
    mut file: crate::resolvers::filez::FilezFile,
) -> Result<bool, String> {
    let d = edit(app, id, |_| {}).ok_or("The download is gone.")?;
    let reusable = !d.archive_path.is_empty()
        && std::fs::metadata(&d.archive_path).is_ok_and(|m| m.len() == file.size)
        && d.total == Some(file.size)
        && d.segments.len() == filez_segments(file.size).len();
    if !reusable {
        let dir = archive_dir(settings);
        crate::storage::check_download_room(&dir, Path::new(&settings.library_dir), file.size, file.size, &d.meta.title)?;
        let name = safe_name(&file.name);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot write to {}: {e}", dir.display()))?;
        let path = dir.join(&name);
        crate::hatchzip::forget(&path);
        let f = std::fs::File::create(&path).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
        f.set_len(file.size).map_err(|e| format!("Cannot make room for {}: {e}", path.display()))?;
        drop(f);
        edit(app, id, |d| {
            d.file_name = name.clone();
            d.archive_path = path.to_string_lossy().into_owned();
            name_from_file(d);
            d.total = Some(file.size);
            d.received = 0;
            d.segments = filez_segments(file.size);
            d.etag = None;
            d.verified = false;
        });
        persist(app);
    }
    let mut asked_again = false;
    let mut failures = 0u32;
    loop {
        let before = edit(app, id, |_| {}).map(|d| d.received).unwrap_or(0);
        match filez_pass(app, id, flag, client, limiter, &file).await {
            Ok(done) => {
                edit(app, id, |d| d.notice = None);
                return Ok(done);
            }
            Err(Transfer::Fatal(e)) => return Err(e),
            // The host's pass ran out (a day), or was issued to this PC over
            // the other address family: pass its check again, once.
            Err(Transfer::Refused(_)) if !asked_again => {
                asked_again = true;
                match mirror_client(app, id, flag, true, FILEZ_CONNECTIONS as u32).await {
                    Ok((c, _)) => *client = c,
                    Err(_) if flag.load(Ordering::SeqCst) != RUN => return Ok(false),
                    Err(e) => return Err(e),
                }
                file = edit(app, id, |_| {}).and_then(|d| crate::resolvers::filez::file_for(&d.url)).unwrap_or(file);
                set_status(app, id, Status::Downloading, None);
            }
            Err(Transfer::Refused(code)) => {
                return Err(format!(
                    "filez.stream refused the download twice (it answered {code}). Its check is tied to the network that passed it: press Retry, or take another mirror."
                ))
            }
            Err(Transfer::Restart(e)) | Err(Transfer::Retry(e)) => {
                let after = edit(app, id, |_| {}).map(|d| d.received).unwrap_or(0);
                if after > before {
                    failures = 0;
                }
                failures += 1;
                if failures > MAX_FAILURES {
                    return Err(format!("The connection to filez.stream kept dropping ({e}). Resume to try again."));
                }
                let wait = backoff(failures);
                crate::logging::warn("download", &format!("{id}: filez: {e}; trying again in {wait} s"));
                edit(app, id, |d| d.notice = Some(format!("Connection lost. Trying again in {wait} s.")));
                emit(app, true);
                if !wait_unless_stopped(flag, wait).await {
                    return Ok(false);
                }
            }
            Err(Transfer::NoRanges) => return Err("filez.stream did not send the part of the file that was asked for.".into()),
            // Only the ranged path watches for a crawl.
            Err(Transfer::Slow) => {}
        }
    }
}

/// One pass over the unfinished ranges of a filez download, on a few
/// connections. See `fetch_filez`.
async fn filez_pass<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    flag: &Arc<AtomicU8>,
    client: &reqwest::Client,
    limiter: &Arc<Limiter>,
    file: &crate::resolvers::filez::FilezFile,
) -> Result<bool, Transfer> {
    let item = edit(app, id, |_| {}).ok_or(Transfer::Fatal("The download is gone.".into()))?;
    let path = PathBuf::from(&item.archive_path);
    let segments = Arc::new(item.segments.clone());
    let progress: Arc<Vec<AtomicU64>> = Arc::new(segments.iter().map(|s| AtomicU64::new(s.done.min(s.len()))).collect());
    // A range only counts as done once it is opened and on disk, 16 MiB at a
    // time; the speed and the bar follow what has arrived, as the ranged path does.
    let arrived: Arc<Vec<AtomicU64>> = Arc::new(segments.iter().map(|s| AtomicU64::new(s.done.min(s.len()))).collect());
    let queue: Arc<Vec<usize>> = Arc::new((0..segments.len()).filter(|&i| segments[i].done < segments[i].len()).collect());
    let next = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stop = Arc::new(AtomicU8::new(RUN));
    let mut tasks = Vec::new();
    for _ in 0..FILEZ_CONNECTIONS.min(queue.len().max(1)) {
        let (client, path, progress, arrived, limiter, stop, segments, queue, next, file, flag) =
            (client.clone(), path.clone(), progress.clone(), arrived.clone(), limiter.clone(), stop.clone(), segments.clone(), queue.clone(), next.clone(), file.clone(), flag.clone());
        tasks.push(tokio::spawn(async move {
            loop {
                let Some(&i) = queue.get(next.fetch_add(1, Ordering::SeqCst)) else { return Ok(()) };
                let mut tries = 0;
                loop {
                    if stop.load(Ordering::SeqCst) != RUN || flag.load(Ordering::SeqCst) != RUN {
                        return Ok(());
                    }
                    match filez_range(&client, &file, &path, &segments[i], &arrived[i], &limiter, &|| stop.load(Ordering::SeqCst) != RUN || flag.load(Ordering::SeqCst) != RUN).await {
                        Ok(()) => {
                            progress[i].store(segments[i].len(), Ordering::SeqCst);
                            arrived[i].store(segments[i].len(), Ordering::SeqCst);
                            break;
                        }
                        Err(Transfer::Retry(e)) => {
                            tries += 1;
                            if tries > RANGE_RETRIES || !wait_unless_stopped(&stop, 1u64 << tries).await {
                                return Err(Transfer::Retry(e));
                            }
                        }
                        Err(other) => {
                            stop.store(HALT, Ordering::SeqCst);
                            return Err(other);
                        }
                    }
                }
            }
        }));
    }
    let mut last_sum: u64 = progress.iter().map(|p| p.load(Ordering::SeqCst)).sum();
    let mut meter = SpeedMeter::new(Instant::now(), last_sum);
    let mut last_checkpoint = Instant::now();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        if flag.load(Ordering::SeqCst) != RUN {
            stop.store(flag.load(Ordering::SeqCst), Ordering::SeqCst);
        }
        if stop.load(Ordering::SeqCst) != RUN {
            let deadline = Instant::now() + std::time::Duration::from_millis(1200);
            while Instant::now() < deadline && !tasks.iter().all(|t| t.is_finished()) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            for t in &tasks {
                t.abort();
            }
            break;
        }
        let sum: u64 = arrived.iter().map(|p| p.load(Ordering::SeqCst)).sum();
        let speed = meter.add(Instant::now(), sum);
        let moved = sum > last_sum;
        last_sum = sum;
        edit(app, id, |d| {
            d.received = sum;
            d.speed = speed;
            if moved {
                d.notice = None;
            }
        });
        emit(app, false);
        if last_checkpoint.elapsed() >= CHECKPOINT_EVERY {
            checkpoint(app, id, &path, &progress).await;
            last_checkpoint = Instant::now();
        }
        if tasks.iter().all(|t| t.is_finished()) {
            break;
        }
    }
    let mut outcome: Result<(), Transfer> = Ok(());
    for t in tasks {
        match t.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                // The most telling failure wins: fatal, then refused, then a retry.
                let rank = |e: &Transfer| match e {
                    Transfer::Fatal(_) => 3,
                    Transfer::Refused(_) => 2,
                    _ => 1,
                };
                if outcome.as_ref().err().is_none_or(|o| rank(&e) > rank(o)) {
                    outcome = Err(e);
                }
            }
            Err(e) if e.is_cancelled() => {}
            Err(e) => outcome = Err(Transfer::Retry(e.to_string())),
        }
    }
    if flag.load(Ordering::SeqCst) == CANCEL {
        return Ok(false);
    }
    checkpoint(app, id, &path, &progress).await;
    let sum: u64 = progress.iter().map(|p| p.load(Ordering::SeqCst)).sum();
    edit(app, id, |d| d.received = sum);
    outcome?;
    if flag.load(Ordering::SeqCst) != RUN {
        return Ok(false);
    }
    if progress.iter().zip(segments.iter()).any(|(p, s)| p.load(Ordering::SeqCst) < s.len()) {
        return Err(Transfer::Retry("a range was left unfinished".into()));
    }
    Ok(true)
}

/// One range of a filez download: its grant, its ciphertext, each chunk
/// opened, the plaintext written at its offset and flushed.
async fn filez_range(
    client: &reqwest::Client,
    file: &crate::resolvers::filez::FilezFile,
    path: &Path,
    seg: &Segment,
    arrived: &AtomicU64,
    limiter: &Limiter,
    stopped: &(dyn Fn() -> bool + Sync),
) -> Result<(), Transfer> {
    use crate::resolvers::filez::{chunk_count, cipher_range, grant, open_chunk, CHUNK, ENC_CHUNK};
    arrived.store(0, Ordering::SeqCst);
    let (c0, c1) = (seg.start / CHUNK, seg.end / CHUNK);
    let (from, to) = cipher_range(file.cipher_size, c0, c1);
    grant(client, file, to - from + 1, stopped).await.map_err(|e| if e == STOPPED { Transfer::Retry(e) } else if e.contains("refused") { Transfer::Refused(403) } else { Transfer::Retry(e) })?;
    let res = send(client.get(format!("{}/api/files/{}/data", file.origin, file.id)).header(reqwest::header::RANGE, format!("bytes={from}-{to}"))).await?;
    match res.status().as_u16() {
        206 => {}
        200 if from == 0 => {}
        403 | 401 => return Err(Transfer::Refused(res.status().as_u16())),
        404 | 410 => return Err(Transfer::Fatal("The file is gone from filez.stream. Take another mirror.".into())),
        s if s == 429 || s >= 500 => return Err(Transfer::Retry(format!("filez.stream answered {s}"))),
        s => return Err(Transfer::Fatal(format!("filez.stream answered {s}."))),
    }
    let want = (to - from + 1) as usize;
    let mut sealed: Vec<u8> = Vec::with_capacity(want);
    let mut stream = res.bytes_stream();
    let mut last_data = Instant::now();
    while sealed.len() < want {
        if stopped() {
            return Err(Transfer::Retry(STOPPED.into()));
        }
        match tokio::time::timeout(STOP_POLL, stream.next()).await {
            Err(_) if last_data.elapsed() >= STALL_TIMEOUT => return Err(Transfer::Retry(format!("no data for {} s", STALL_TIMEOUT.as_secs()))),
            Err(_) => continue,
            Ok(None) => break,
            Ok(Some(Err(e))) => return Err(Transfer::Retry(e.to_string())),
            Ok(Some(Ok(bytes))) => {
                limiter.take(bytes.len() as u64).await;
                sealed.extend_from_slice(&bytes);
                // Ciphertext, a little longer than the range: the bar stops at its size.
                arrived.store((sealed.len() as u64).min(seg.len()), Ordering::SeqCst);
                last_data = Instant::now();
            }
        }
    }
    if sealed.len() < want {
        return Err(Transfer::Retry("the connection closed early".into()));
    }
    sealed.truncate(want);
    let total = chunk_count(file.size);
    let mut plain = Vec::with_capacity(seg.len() as usize);
    for (n, c) in (c0..=c1).enumerate() {
        let at = n * ENC_CHUNK as usize;
        let end = (at + ENC_CHUNK as usize).min(sealed.len());
        let opened = open_chunk(&file.content, c, c == total - 1, &sealed[at..end]).ok_or_else(|| {
            Transfer::Fatal("A part of the file from filez.stream did not decrypt: the file there is damaged, or the link's key is wrong. Take another mirror.".into())
        })?;
        plain.extend_from_slice(&opened);
    }
    if plain.len() as u64 != seg.len() {
        return Err(Transfer::Fatal(format!("filez.stream sent {} bytes where {} were expected.", plain.len(), seg.len())));
    }
    let mut out = tokio::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .await
        .map_err(|e| Transfer::Fatal(format!("Cannot write {}: {e}", path.display())))?;
    out.seek(std::io::SeekFrom::Start(seg.start)).await.map_err(|e| Transfer::Fatal(e.to_string()))?;
    out.write_all(&plain).await.map_err(|e| Transfer::Fatal(format!("Writing the download failed: {e}")))?;
    out.flush().await.map_err(|e| Transfer::Fatal(e.to_string()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn fetch_all<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    flag: &AtomicU8,
    client: &mut reqwest::Client,
    connections: &mut u32,
    settings: &crate::settings::Settings,
    limiter: &Arc<Limiter>,
    is_mirror: bool,
) -> Result<bool, String> {
    let mut failures = 0u32;
    let mut restarts = 0u32;
    let mut asked_again = false;
    let mut refreshed = 0u32;
    // Once a server has answered a range with the whole file, it gets one
    // stream for the rest of this run.
    let mut one_stream = false;
    loop {
        let before = edit(app, id, |_| {}).map(|d| d.received).unwrap_or(0);
        // Always the ranged path, whatever the connection count: a download
        // started on ranges must resume on them. Resuming one on a single
        // stream read the full-size file it had made as "already there" and
        // installed an archive with holes in it.
        let step = if one_stream {
            transfer(app, id, flag, client, settings, limiter).await
        } else {
            transfer_parallel(app, id, flag, client, settings, *connections, limiter, refreshed < SLOW_REFRESHES).await
        };
        match step {
            Ok(true) => {
                edit(app, id, |d| d.notice = None);
                return Ok(true);
            }
            Ok(false) => {
                edit(app, id, |d| d.notice = None);
                return Ok(false);
            }
            Err(Transfer::Fatal(e)) => return Err(e),
            // A mirror's address ran out (they are signed, or tied to a
            // session): ask the mirror again, once per run.
            Err(Transfer::Refused(_)) if is_mirror && !asked_again => {
                asked_again = true;
                match mirror_client(app, id, flag, true, settings.connections).await {
                    Ok(v) => (*client, *connections) = v,
                    Err(_) if flag.load(Ordering::SeqCst) != RUN => return Ok(false),
                    Err(e) => return Err(e),
                }
                set_status(app, id, Status::Downloading, None);
            }
            // Our own link refused as made for another network: once, the
            // other address family is tried (see `foreign_ip_retry`); the
            // second refusal opens the game's page so one press re-mints it.
            Err(Transfer::Refused(403)) if !is_mirror && !asked_again => {
                asked_again = true;
                match foreign_ip_retry(app, id, client).await {
                    Some(other) => {
                        *client = other;
                        set_status(app, id, Status::Downloading, None);
                    }
                    None => return Err(refused_ours(app, id, 403)),
                }
            }
            Err(Transfer::Refused(code)) if !is_mirror => return Err(refused_ours(app, id, code)),
            Err(Transfer::Refused(code)) => return Err(refused_text(code, is_mirror)),
            Err(Transfer::Restart(why)) => {
                restarts += 1;
                crate::logging::warn("download", &format!("{id}: starting over: {why}"));
                if restarts > 2 {
                    return Err(format!("The file kept changing while it downloaded ({why}). Press Retry to start it again."));
                }
                discard(app, id);
            }
            Err(Transfer::NoRanges) => {
                crate::logging::warn("download", &format!("{id}: the server stopped answering ranges; one stream from here"));
                one_stream = true;
                discard(app, id);
            }
            // What a Pause and Resume did by hand: new connections, and for a
            // mirror a fresh link, carrying on from where every range got to.
            Err(Transfer::Slow) => {
                refreshed += 1;
                crate::logging::warn("download", &format!("{id}: the speed fell to a crawl; new connections ({refreshed} of {SLOW_REFRESHES})"));
                edit(app, id, |d| d.notice = Some("The connection slowed down. Reconnecting.".into()));
                emit(app, true);
                if is_mirror {
                    match mirror_client(app, id, flag, true, settings.connections).await {
                        Ok(v) => (*client, *connections) = v,
                        Err(_) if flag.load(Ordering::SeqCst) != RUN => return Ok(false),
                        // The page cannot be asked again right now: the old link still works.
                        Err(e) => crate::logging::warn("download", &format!("{id}: no fresh link ({e}); same link, new connections")),
                    }
                    set_status(app, id, Status::Downloading, None);
                }
            }
            Err(Transfer::Retry(e)) => {
                let after = edit(app, id, |_| {}).map(|d| d.received).unwrap_or(0);
                if after > before {
                    failures = 0;
                }
                failures += 1;
                if failures > MAX_FAILURES {
                    return Err(format!("The connection kept dropping ({e}). Resume to try again."));
                }
                let wait = backoff(failures);
                crate::logging::warn("download", &format!("{id}: {e}; trying again in {wait} s"));
                edit(app, id, |d| {
                    d.speed = 0;
                    d.notice = Some(format!("Connection lost. Trying again in {wait} s."));
                });
                emit(app, true);
                if !wait_unless_stopped(flag, wait).await {
                    edit(app, id, |d| d.notice = None);
                    return Ok(false);
                }
                edit(app, id, |d| d.notice = Some("Reconnecting.".into()));
                emit(app, true);
            }
        }
    }
}

/// Throw away what was fetched so far, so the next try starts from zero.
fn discard<R: Runtime>(app: &AppHandle<R>, id: &str) {
    if let Some(d) = edit(app, id, |_| {}) {
        if !d.archive_path.is_empty() {
            let _ = std::fs::remove_file(&d.archive_path);
            crate::hatchzip::forget(Path::new(&d.archive_path));
        }
    }
    edit(app, id, |d| {
        d.received = 0;
        d.segments.clear();
        d.etag = None;
        d.verified = false;
        d.synced = None;
    });
    persist(app);
}

/// What kryo.to says about this exact file: its SHA-256, and whether it is
/// one of the game's add-ons. Matched by the name it downloads as.
async fn identify<R: Runtime>(app: &AppHandle<R>, id: &str, client: &reqwest::Client) {
    let Some(d) = edit(app, id, |_| {}) else { return };
    let Some(slug) = d.slug.clone() else { return };
    if d.file_name.is_empty() {
        return;
    }
    #[allow(unused_mut)]
    let mut base = crate::settings::catalog_endpoint(&crate::settings::load(app));
    // Debug builds only, beside KRYOTO_DEV_DOWNLOAD_HOST: the stand-in filehost
    // also answers this, so a local archive can be checked against its own hash.
    #[cfg(debug_assertions)]
    if let Ok(dev) = std::env::var("KRYOTO_DEV_DOWNLOAD_HOST") {
        if !dev.is_empty() {
            base = format!("http://{dev}");
        }
    }
    let Ok(res) = client.get(format!("{base}/api/games/{slug}/downloads")).send().await else { return };
    let Ok(json) = res.json::<serde_json::Value>().await else { return };
    let hint = FileHint {
        size: d.total,
        version: d.release.as_deref(),
        page: d.mirror.as_ref().map(|m| m.page.as_str()),
    };
    let found = match_file_with(&json, &d.file_name, &hint);
    edit(app, id, |d| {
        // kryo.to's answer now, whatever an earlier try was told: a hash that
        // was wrong for this file (another build's) must not outlive a Retry.
        d.sha256 = found.sha;
        // Added from its game's card, it is that add-on even when kryo.to's
        // answer does not name the file (links left out, renamed by a host).
        if found.addon.is_some() || d.addon_release.is_none() {
            d.addon = found.addon;
            d.repair = found.repair;
        }
        // A build other than the current one, whichever way it was picked:
        // it installs as that build.
        if let Some(v) = found.release {
            d.meta.version = Some(v.clone());
            d.release = Some(v);
        }
    });
}

/// The SHA-256 and (for an add-on) the name of `file` in a
/// `/api/games/<slug>/downloads` answer.
#[derive(Debug, Default, PartialEq)]
pub struct FileMatch {
    pub sha: Option<String>,
    /// The add-on's name, when the file is one.
    pub addon: Option<String>,
    /// The release's version, when the file is a release other than the current one.
    pub release: Option<String>,
    /// The add-on is a repair fix (`kind: "repair"`).
    pub repair: bool,
}

/// What the download itself knows about which build it is, beside its name.
///
/// THE NAME IS NOT ENOUGH. Every build of a game downloads as the same
/// "<Title> - Kryoto.7z", so matching by name alone gave an older build the
/// CURRENT build's hash, and every switch to an older build failed its check
/// (Happy Wheels 1.99 against 1.99.2, October 2026). Each listed hash carries
/// the file's size, which tells two builds apart without anything else; the
/// version picked under Versions and the mirror page it came from pin it down
/// where they are known.
#[derive(Debug, Default)]
pub struct FileHint<'a> {
    /// The archive's size, once the transfer has it.
    pub size: Option<u64>,
    /// The version picked under Versions, when it is not the current one.
    pub version: Option<&'a str>,
    /// The mirror page it came from.
    pub page: Option<&'a str>,
}

#[cfg(test)]
pub fn match_file(json: &serde_json::Value, file: &str) -> FileMatch {
    match_file_with(json, file, &FileHint::default())
}

pub fn match_file_with(json: &serde_json::Value, file: &str, hint: &FileHint) -> FileMatch {
    let same = |name: &str| safe_name(name).eq_ignore_ascii_case(file);
    let hashes = |v: &serde_json::Value| -> Vec<(String, Option<u64>, bool)> {
        v["hashes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| {
                let sha = h["sha256"].as_str().filter(|s| s.len() == 64)?.to_ascii_lowercase();
                let named = h["name"].as_str().is_some_and(same);
                Some((sha, h["bytes"].as_u64(), named))
            })
            .collect()
    };
    let fits = |bytes: Option<u64>| match (hint.size, bytes) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    let releases: Vec<&serde_json::Value> = json["releases"].as_array().into_iter().flatten().collect();
    let found = |release: &serde_json::Value, sha: String| {
        let other = !release["primary"].as_bool().unwrap_or(false);
        FileMatch {
            sha: Some(sha),
            release: other.then(|| release["version"].as_str().map(str::to_string)).flatten(),
            ..Default::default()
        }
    };

    // 1. The build is known: the version picked, or the mirror page it came
    //    from. Its own hash, by name, or its only file when a mirror renamed it.
    let picked = releases.iter().find(|r| {
        hint.version.is_some_and(|v| r["version"].as_str() == Some(v))
            || hint.page.is_some_and(|p| {
                r["links"].as_array().into_iter().flatten().any(|l| l["url"].as_str() == Some(p))
            })
    });
    if let Some(r) = picked {
        let hs = hashes(r);
        let by_name = hs.iter().find(|(_, b, named)| *named && fits(*b));
        let only = (hs.len() == 1 && fits(hs[0].1) && hint.size.is_some()).then(|| &hs[0]);
        if let Some((sha, _, _)) = by_name.or(only) {
            return found(r, sha.clone());
        }
    }

    // 2. Every release's file of this name, and the one whose size is the
    //    archive's. With no size to go on, the current build, as before.
    let named: Vec<(&serde_json::Value, String, Option<u64>)> = releases
        .iter()
        .flat_map(|r| hashes(r).into_iter().filter(|(_, _, n)| *n).map(move |(sha, b, _)| (*r, sha, b)))
        .collect();
    if hint.size.is_some() {
        if let Some((r, sha, _)) = named.iter().find(|(_, _, b)| b.is_some() && fits(*b)) {
            return found(r, sha.clone());
        }
    }
    // A size that matches no build at all is still checked against the
    // current one: a broken transfer must fail, not skip the check.
    if let Some((sha, _, _)) = hashes(json).into_iter().find(|(_, _, n)| *n) {
        return FileMatch { sha: Some(sha), ..Default::default() };
    }
    // Only an older build has a file of this name. It is that build's file
    // only when nothing says otherwise: never when the size is another one,
    // and never while the current build's hash is still being worked out -
    // a new build under the usual name was checked against the build before
    // it and always failed (Schedule I, October 2026). No hash then, and the
    // transfer's own length check stands.
    let current_pending = releases
        .iter()
        .any(|r| r["primary"].as_bool().unwrap_or(false) && hashes(r).is_empty());
    if let Some((r, sha, b)) = named.first() {
        if !current_pending && (hint.size.is_none() || b.is_none() || fits(*b)) {
            return found(r, sha.clone());
        }
        return FileMatch::default();
    }

    for addon in json["addons"].as_array().into_iter().flatten() {
        let named = addon["links"].as_array().into_iter().flatten().any(|l| l["name"].as_str().is_some_and(same));
        let hash = hashes(addon).into_iter().find(|(_, _, n)| *n).map(|(sha, _, _)| sha);
        // Fetched from one of its mirrors: the host may have renamed the file.
        let mirrored = hint.page.is_some_and(|p| addon["pages"].as_array().into_iter().flatten().any(|u| u.as_str() == Some(p)));
        let hash = hash.or_else(|| mirrored.then(|| hashes(addon).into_iter().next().map(|(sha, _, _)| sha)).flatten());
        if named || mirrored || hash.is_some() {
            let label = addon["label"].as_str().filter(|l| !l.trim().is_empty()).unwrap_or("Add-on").to_string();
            return FileMatch { sha: hash, addon: Some(label), release: None, repair: addon["kind"].as_str() == Some("repair") };
        }
    }
    FileMatch::default()
}

/// Check the archive against the SHA-256 kryo.to lists for it. A mismatch
/// keeps the archive and marks it (`hash_mismatch`): Install anyway uses it,
/// Retry deletes it and downloads it again.
async fn verify<R: Runtime>(app: &AppHandle<R>, id: &str, flag: &Arc<AtomicU8>) -> Result<(), String> {
    let d = edit(app, id, |_| {}).ok_or("The download is gone.")?;
    let Some(expected) = d.sha256.clone() else { return Ok(()) };
    // Partly unpacked in place, what is left of it can only fail the check.
    if d.verified || crate::hatchzip::resumes(Path::new(&d.archive_path)) {
        return Ok(());
    }
    if flag.load(Ordering::SeqCst) != RUN {
        return Err(STOPPED.into());
    }
    set_status(app, id, Status::Verifying, None);
    let stop = flag.clone();
    let path = PathBuf::from(&d.archive_path);
    let total = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let progress_app = app.clone();
    let pid = id.to_string();
    let actual = tauri::async_runtime::spawn_blocking(move || -> std::io::Result<String> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        let mut done = 0u64;
        loop {
            // Hashing a big archive takes a while; Pause and Cancel answer
            // between reads instead of after the last one.
            if stop.load(Ordering::SeqCst) != RUN {
                return Ok(String::new());
            }
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            done += n as u64;
            edit(&progress_app, &pid, |d| {
                d.extracted = done;
                d.extract_total = Some(total);
            });
            emit(&progress_app, false);
        }
        Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Could not read the archive to check it: {e}"))?;
    if flag.load(Ordering::SeqCst) != RUN {
        return Err(STOPPED.into());
    }
    if actual != expected {
        crate::logging::error("download", &format!("{id}: sha256 {actual} is not {expected}"));
        // The archive stays: the player decides between installing it anyway
        // and downloading it again (Retry, which throws this copy away).
        edit(app, id, |d| {
            d.hash_mismatch = true;
            d.extracted = 0;
            d.extract_total = None;
        });
        return Err("The archive doesn't match kryo.to's checksum. Install it anyway, or Retry to download it again.".into());
    }
    edit(app, id, |d| {
        d.verified = true;
        d.extracted = 0;
        d.extract_total = None;
    });
    Ok(())
}

/* ── the queue ─────────────────────────────────────────────── */

/// Holding the turn; giving it back wakes whoever is next.
struct Turn<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> Drop for Turn<R> {
    fn drop(&mut self) {
        let state = self.app.state::<Downloads>();
        if let Ok(mut a) = state.active.lock() {
            *a = None;
        }
        state.turn.notify_waiters();
    }
}

/// Queue order, then age: the order every screen and the scheduler agree on.
fn queue_key(d: &Download) -> (u64, u64) {
    (if d.queue_order == 0 { u64::MAX / 2 + d.added_at } else { d.queue_order }, d.added_at)
}

/// The waiting download that goes next: queued, with a live task, first in order.
fn next_in_line<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let state = app.state::<Downloads>();
    let alive: Vec<String> = state.controls.lock().ok()?.keys().cloned().collect();
    let list = state.list.lock().ok()?;
    list.iter()
        .filter(|d| d.status == Status::Queued && alive.contains(&d.id))
        .min_by_key(|d| queue_key(d))
        .map(|d| d.id.clone())
}

/// Wait until it is this download's turn. None when it was paused or
/// cancelled while it waited.
async fn wait_for_turn<R: Runtime>(app: &AppHandle<R>, id: &str, flag: &AtomicU8) -> Option<Turn<R>> {
    let state = app.state::<Downloads>();
    loop {
        if flag.load(Ordering::SeqCst) != RUN {
            return None;
        }
        let next = next_in_line(app);
        let mine = match state.active.lock() {
            Ok(mut active) if active.is_none() && next.as_deref() == Some(id) => {
                *active = Some(id.to_string());
                true
            }
            Ok(_) => false,
            Err(_) => return None,
        };
        if mine {
            return Some(Turn { app: app.clone() });
        }
        // Woken on every change; the timeout is only a safety net.
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), state.turn.notified()).await;
    }
}

/// Renumber the queue: `order` first, then whatever else is unfinished.
fn renumber<R: Runtime>(app: &AppHandle<R>, order: &[String]) {
    let state = app.state::<Downloads>();
    if let Ok(mut list) = state.list.lock() {
        let mut rest: Vec<(u64, u64, String)> = list
            .iter()
            .filter(|d| !matches!(d.status, Status::Installed | Status::Canceled) && !order.contains(&d.id))
            .map(|d| {
                let (a, b) = queue_key(d);
                (a, b, d.id.clone())
            })
            .collect();
        rest.sort();
        let all: Vec<String> = order.iter().cloned().chain(rest.into_iter().map(|r| r.2)).collect();
        for d in list.iter_mut() {
            if let Some(i) = all.iter().position(|x| x == &d.id) {
                d.queue_order = i as u64 + 1;
            }
        }
    };
}

/// The unfinished downloads other than the one running, in queue order.
fn waiting_order<R: Runtime>(app: &AppHandle<R>, active: Option<&str>) -> Vec<String> {
    let state = app.state::<Downloads>();
    let Ok(list) = state.list.lock() else { return Vec::new() };
    let mut v: Vec<&Download> = list
        .iter()
        .filter(|d| !matches!(d.status, Status::Installed | Status::Canceled) && Some(d.id.as_str()) != active)
        .collect();
    v.sort_by_key(|d| queue_key(d));
    v.into_iter().map(|d| d.id.clone()).collect()
}

/// Mark a stopped download queued again and start its task.
fn requeue<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<(), String> {
    if let Some(flag) = app.state::<Downloads>().controls.lock().map_err(|_| "Download controls are unavailable.")?.get(id) {
        return if flag.load(Ordering::SeqCst) == 0 { Ok(()) } else { Err("Wait for the download to stop, then try again.".into()) };
    }
    edit(app, id, |d| {
        // Retry on an archive that failed its check: that copy is thrown away
        // and the download starts over (Install anyway clears the flag first).
        if d.hash_mismatch {
            let _ = std::fs::remove_file(&d.archive_path);
            d.hash_mismatch = false;
            d.received = 0;
            d.segments.clear();
            d.archive_path.clear();
            d.verified = false;
        }
        d.error = None;
        d.status = Status::Queued;
        // Resumed without an answer: the default folder.
        d.ask_folder = false;
    })
    .ok_or("That download is gone.")?;
    emit(app, true);
    start(app.clone(), id.to_string());
    Ok(())
}

/// What a cancelled row leaves on the disk goes: the archive, and whatever an
/// unpack had got to. On its own thread: a many-gigabyte delete can take a while
/// (antivirus, a slow disk), and the row says Cancelled straight away regardless.
fn clear_leftovers(archive: Option<String>, staging: Option<String>) {
    if let Some(dir) = staging {
        throw_away(PathBuf::from(dir));
    }
    if let Some(path) = archive {
        std::thread::spawn(move || {
            let _ = std::fs::remove_file(&path);
            crate::hatchzip::forget(Path::new(&path));
        });
    }
}

/// Delete a folder without making anyone wait. It changes name first, so a
/// Retry pressed straight after unpacks into a folder of its own and not into
/// one being deleted under it.
fn throw_away(dir: PathBuf) {
    let gone = dir.with_extension(format!("gone-{}", now()));
    let target = if std::fs::rename(&dir, &gone).is_ok() { gone } else { dir };
    std::thread::spawn(move || {
        let _ = std::fs::remove_dir_all(target);
    });
}

fn settle_stopped<R: Runtime>(app: &AppHandle<R>, id: &str, flag: &AtomicU8) -> Result<(), String> {
    if flag.load(Ordering::SeqCst) == CANCEL {
        let row = edit(app, id, |d| d.received = 0);
        let (archive, staging) = row.map(|d| (Some(d.archive_path).filter(|p| !p.is_empty()), d.staging)).unwrap_or_default();
        edit(app, id, |d| d.staging = None);
        set_status(app, id, Status::Canceled, None);
        clear_leftovers(archive, staging);
    } else {
        set_status(app, id, Status::Paused, None);
    }
    Ok(())
}

/// Windows' error text, said in terms of what the player can do about it.
fn explain_os_error(e: String) -> String {
    if e.contains("os error 225") {
        return format!(
            "{e}\n\nWindows Security blocked a file in this game as a threat. Game cracks and emulators are often flagged. Allow it under Windows Security > Virus & threat protection > Protection history, or add the library folder as an exclusion, then press Retry."
        );
    }
    if e.contains("os error 5)") && e.starts_with("Installing into") {
        return format!(
            "{e}\n\nWindows would not let Kryoto write there. Usually it is antivirus holding a file it is scanning, or a folder that needs administrator rights. Wait a moment and press Retry, or pick another library folder in Settings > Storage."
        );
    }
    e
}

enum Transfer {
    Retry(String),
    Fatal(String),
    /// The server turned the address down (403, 410): it expired, or is
    /// bound to something this request is not.
    Refused(u16),
    /// What arrived is not the file this download started from: another
    /// length, another ETag, or a range that does not start where it was
    /// asked to. Start over rather than stitch two files into one archive.
    Restart(String),
    /// The server answered a range with the whole file: carry on in one stream.
    NoRanges,
    /// The connections slowed to a crawl (`SlowWatch`): what arrived is kept,
    /// and the download carries on over new ones.
    Slow,
}

/// The site's words when direct downloads are for supporters only (an owner
/// switch while the servers are full). Kryoto's own text, so a phone, the
/// Downloads page and a notice all say the same.
pub const SUPPORTER_REQUIRED: &str =
    "Direct downloads are for supporters right now, our servers are full. Donate from $5 for 30 days, or take a mirror.";

/// dl.kryo.to's reason for refusing our link (`{"error":"foreign_ip"}`,
/// `link_expired`, `gone`), read from a tiny range request: the transfer only
/// kept the status code, and a 403 is also what an expired or wrong link gets.
async fn refusal_reason(client: &reqwest::Client, url: &str) -> Option<String> {
    let res = tokio::time::timeout(ANSWER_TIMEOUT, client.get(url).header(reqwest::header::RANGE, "bytes=0-0").send()).await.ok()?.ok()?;
    if res.status().is_success() {
        return None;
    }
    let body: serde_json::Value = res.json().await.ok()?;
    body["error"].as_str().map(str::to_string)
}

/// The IP-family mismatch behind "made for another network": kryo.to's page
/// asked for the link over one family (the token says which), and this
/// downloader fetches over whichever the system picks. `link_family` already
/// binds to the token's; when that is still refused (a carrier-grade NAT
/// handing out a different IPv4 per connection, say), the other family is
/// tried once - a dual-stack line often agrees on IPv6 where IPv4 wandered.
/// `None` when the refusal is not that, or there is no other family to try.
async fn foreign_ip_retry<R: Runtime>(app: &AppHandle<R>, id: &str, client: &reqwest::Client) -> Option<reqwest::Client> {
    let d = edit(app, id, |_| {})?;
    let reason = refusal_reason(client, &d.url).await;
    if reason.as_deref() != Some("foreign_ip") {
        return None;
    }
    let bound = link_family(&d.url);
    let other: std::net::IpAddr = match bound {
        Some(std::net::IpAddr::V6(_)) => std::net::Ipv4Addr::UNSPECIFIED.into(),
        Some(std::net::IpAddr::V4(_)) => std::net::Ipv6Addr::UNSPECIFIED.into(),
        // An unbound link that is still refused: the family was not the reason.
        None => return None,
    };
    crate::logging::warn("download", &format!("{id}: refused as foreign_ip over {bound:?}; trying once over {other}"));
    client_builder().cookie_store(true).local_address(other).build().ok()
}

/// Our own link refused for good: the player's words, and - for a link bound
/// to another network - the game's page in the Store, where one press on
/// Download mints a fresh link that this row then carries on with
/// (`unfinished_copy`). The page opening is the fix; the text explains it.
fn refused_ours<R: Runtime>(app: &AppHandle<R>, id: &str, code: u16) -> String {
    let text = refused_text(code, false);
    if code == 403 {
        if let Some(slug) = edit(app, id, |_| {}).and_then(|d| d.slug) {
            let _ = app.emit_to("main", "store-download-page", slug);
        }
    }
    text
}

/// What a refused address means, for the person.
fn refused_text(code: u16, mirror: bool) -> String {
    match (mirror, code) {
        (true, _) => format!("The mirror refused the download twice (it answered {code}). Try another mirror, or our own copy."),
        (false, 410) => {
            "The download link expired. Press Download on the game's Store page again: it carries on from where it stopped.".into()
        }
        (false, _) => {
            "kryo.to refused the link - it only works on the network it was made for. The game's page is open in the Store: press Download there and this download carries on.".into()
        }
    }
}

/// Resolve a mirror download (again, with `fresh`) and build the client its
/// file wants: the mirror's cookies, user agent and referer on every request,
/// and no more connections than the host puts up with.
async fn mirror_client<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    flag: &AtomicU8,
    fresh: bool,
    connections: u32,
) -> Result<(reqwest::Client, u32), String> {
    let state = app.state::<Downloads>();
    let d = edit(app, id, |_| {}).ok_or("The download is gone.")?;
    let mirror = d.mirror.clone().ok_or("Not a mirror download.")?;
    let known = if fresh { None } else { state.resolved.lock().ok().and_then(|r| r.get(id).cloned()) };
    let resolved = match known {
        Some(r) => r,
        None => {
            set_status(app, id, Status::Resolving, None);
            let r = crate::resolvers::resolve(app, &mirror.page).await?;
            if flag.load(Ordering::SeqCst) != RUN {
                return Err("Stopped while the mirror was being asked.".into());
            }
            if let Ok(mut map) = state.resolved.lock() {
                map.insert(id.to_string(), r.clone());
            }
            r
        }
    };
    edit(app, id, |d| {
        d.url = resolved.url.clone();
        // The name the mirror gives the file, for when its address has none
        // (a page host's `/download?ticket=` says nothing about the file).
        if d.archive_path.is_empty() {
            if let Some(name) = &resolved.file_name {
                d.file_name = safe_name(name);
            }
        }
        if d.total.is_none() {
            d.total = resolved.size;
        }
        // A resolved file of another size is another file: start it over.
        if let (Some(size), Some(total)) = (resolved.size, d.total) {
            if size != total && !d.segments.is_empty() {
                d.segments.clear();
                d.received = 0;
                d.total = Some(size);
                d.etag = None;
            }
        }
    });
    persist(app);
    let mut headers = reqwest::header::HeaderMap::new();
    for (k, v) in &resolved.headers {
        if let (Ok(k), Ok(v)) = (reqwest::header::HeaderName::from_bytes(k.as_bytes()), reqwest::header::HeaderValue::from_str(v)) {
            headers.insert(k, v);
        }
    }
    let client = client_builder()
        .user_agent(crate::resolvers::UA)
        .default_headers(headers)
        .build()
        .map_err(|e| e.to_string())?;
    Ok((client, connections.min(resolved.connections.max(1))))
}

/// The size of a file whose answer did not say it: from a one-byte range past the
/// first (some hosts treat `bytes=0-0` as the whole file), or a HEAD.
async fn size_by_asking(client: &reqwest::Client, url: &str) -> Option<u64> {
    let wait = std::time::Duration::from_secs(10);
    let ranged = client.get(url).header(reqwest::header::RANGE, "bytes=1-1").timeout(wait).send().await.ok();
    if let Some(total) = ranged.as_ref().and_then(content_range_of).and_then(|r| r.total) {
        return Some(total);
    }
    drop(ranged);
    client.head(url).timeout(wait).send().await.ok().filter(|r| r.status().is_success()).and_then(|r| r.content_length()).filter(|n| *n > 0)
}

/// Move bytes on one stream. `Ok(true)` finished, `Ok(false)` paused or cancelled.
async fn transfer<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    flag: &AtomicU8,
    client: &reqwest::Client,
    settings: &crate::settings::Settings,
    limiter: &Limiter,
) -> Result<bool, Transfer> {
    let item = edit(app, id, |_| {}).ok_or(Transfer::Fatal("The download is gone.".into()))?;
    let mut on_disk = if item.archive_path.is_empty() {
        0
    } else {
        std::fs::metadata(&item.archive_path).map(|m| m.len()).unwrap_or(0)
    };
    // Longer than the file can be is not a partial download of it.
    if item.total.is_some_and(|t| on_disk > t) {
        on_disk = 0;
    }
    // Only what is known to be on the disk counts (`synced`).
    if let Some(synced) = item.synced {
        on_disk = on_disk.min(synced);
    }
    let mut request = client.get(&item.url);
    if on_disk > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={on_disk}-"));
    }
    let Some(response) = unless_stopped(flag, send(request)).await else { return Ok(false) };
    let response = response?;
    let status = response.status();
    if status.as_u16() == 416 && on_disk > 0 {
        // Nothing left past what is here: done, if what is here is the whole
        // file. Taking any 416 as "done" is how a file the size of the
        // download, with nothing in it, was once installed.
        let total = content_range_of(&response).and_then(|r| r.total).or(item.total);
        if total == Some(on_disk) {
            return Ok(true);
        }
        return Err(Transfer::Restart(format!("the server has {total:?} bytes and {on_disk} are here")));
    }
    check_status(status.as_u16())?;
    let resumed = status.as_u16() == 206;
    let range = content_range_of(&response);
    let etag = etag_of(&response);
    if resumed {
        // The bytes must pick up exactly where the file on disk ends, from
        // the same file. Anything else would be appended in the wrong place.
        if range.and_then(|r| r.start) != Some(on_disk) {
            return Err(Transfer::Restart(format!("asked to resume at {on_disk}, the server sent {range:?}")));
        }
        if !same_file(item.etag.as_deref(), item.total, etag.as_deref(), range.and_then(|r| r.total)) {
            return Err(Transfer::Restart("the file on the server changed".into()));
        }
    }
    let total = if resumed { range.and_then(|r| r.total) } else { response.content_length() };
    // A file streamed with no length has no percentage, and only a pause and a
    // resume used to bring one (a resume's range says the size).
    let total = match total {
        Some(t) => Some(t),
        None => size_by_asking(client, &item.url).await,
    };

    let mut path = PathBuf::from(&item.archive_path);
    if item.archive_path.is_empty() {
        let name = response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .and_then(filename_from_disposition)
            .or_else(|| (!item.file_name.is_empty()).then(|| item.file_name.clone()))
            .or_else(|| file_name_in_link(&item.url))
            .or_else(|| {
                response.url().path_segments().and_then(|mut s| s.next_back()).map(String::from)
            })
            .map(|n| safe_name(&n))
            .unwrap_or_else(|| format!("{}.7z", safe_name(&item.meta.title)));
        let dir = archive_dir(settings);
        std::fs::create_dir_all(&dir).map_err(|e| Transfer::Fatal(format!("Cannot write to {}: {e}", dir.display())))?;
        path = dir.join(&name);
        edit(app, id, |d| {
            d.file_name = name.clone();
            d.archive_path = path.to_string_lossy().into_owned();
            name_from_file(d);
        });
    }

    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!resumed)
        .open(&path)
        .await
        .map_err(|e| Transfer::Fatal(format!("Cannot write {}: {e}", path.display())))?;
    let mut file = tokio::io::BufWriter::with_capacity(WRITE_BUFFER, file);
    if resumed {
        // At the end of what was asked for, not at whatever the end is now.
        file.seek(std::io::SeekFrom::Start(on_disk)).await.map_err(|e| Transfer::Fatal(e.to_string()))?;
    }
    let mut received = if resumed { on_disk } else { 0 };
    // Room for the rest of the archive, and for about as much again unpacked.
    // The exact unpacked size is checked again before extracting.
    if let Some(t) = total {
        crate::storage::check_download_room(&path_dir(&path), Path::new(&settings.library_dir), t.saturating_sub(received), t, &item.meta.title)
            .map_err(Transfer::Fatal)?;
    }
    edit(app, id, |d| {
        d.received = received;
        d.total = total.or(d.total);
        d.segments.clear();
        if !resumed {
            d.etag = etag.clone();
            d.verified = false;
            d.synced = Some(0);
        }
    });

    let mut stream = response.bytes_stream();
    // The disk catching up in the background, on a second handle to the file,
    // so the stream never waits on it: how far it had got once it is done.
    let mut syncing: Option<tokio::task::JoinHandle<Option<u64>>> = None;
    let mut last_sync = Instant::now();
    let mut meter = SpeedMeter::new(Instant::now(), received);
    let mut last_report = Instant::now();
    let mut last_persist = Instant::now();
    let mut last_data = Instant::now();
    let mut failure = None;
    loop {
        // Waiting on the next chunk is bounded, so Pause and Cancel answer
        // within a moment even on a connection that has gone quiet.
        let next = tokio::time::timeout(STOP_POLL, stream.next()).await;
        if flag.load(Ordering::SeqCst) != RUN {
            break;
        }
        let chunk = match next {
            Err(_) if last_data.elapsed() >= STALL_TIMEOUT => {
                failure = Some(format!("no data for {} s", STALL_TIMEOUT.as_secs()));
                break;
            }
            Err(_) => continue,
            Ok(None) => break,
            Ok(Some(Ok(chunk))) => chunk,
            Ok(Some(Err(e))) => {
                failure = Some(e.to_string());
                break;
            }
        };
        last_data = Instant::now();
        file.write_all(&chunk)
            .await
            .map_err(|e| Transfer::Fatal(format!("Writing the download failed: {e}")))?;
        received += chunk.len() as u64;
        // The cap applies on one stream too; it used to be skipped here.
        limiter.take(chunk.len() as u64).await;
        if last_report.elapsed().as_secs_f64() >= 1.0 {
            last_report = Instant::now();
            let speed = meter.add(last_report, received);
            edit(app, id, |d| {
                d.received = received;
                d.speed = speed;
                d.notice = None;
            });
            emit(app, false);
        }
        if last_sync.elapsed() >= CHECKPOINT_EVERY && syncing.as_ref().is_none_or(|t| t.is_finished()) {
            if let Some(Ok(Some(n))) = match syncing.take() { Some(t) => Some(t.await), None => None } {
                edit(app, id, |d| d.synced = Some(n));
            }
            file.flush().await.map_err(|e| Transfer::Fatal(format!("Writing the download failed: {e}")))?;
            let mark = received;
            if let Ok(handle) = file.get_ref().try_clone().await {
                syncing = Some(tokio::spawn(async move { handle.sync_data().await.ok().map(|()| mark) }));
            }
            last_sync = Instant::now();
        }
        if last_persist.elapsed().as_secs() >= 5 {
            persist(app);
            last_persist = Instant::now();
        }
    }
    // Cancelled: it is about to be deleted, so it is not flushed first.
    if flag.load(Ordering::SeqCst) == CANCEL {
        return Ok(false);
    }
    // On disk before it is counted: a resume starts from the file's length.
    file.flush().await.map_err(|e| Transfer::Fatal(format!("Writing the download failed: {e}")))?;
    // Paused: the file's length already says how far it got, so the sync only
    // matters for a power cut. It may not hold Pause up for minutes on a hard disk.
    let synced = if flag.load(Ordering::SeqCst) == RUN {
        file.get_ref().sync_data().await.is_ok()
    } else {
        matches!(tokio::time::timeout(STOP_SAVE_WAIT, file.get_ref().sync_data()).await, Ok(Ok(())))
    };
    // Not all on the disk yet: the last mark that was stands.
    let earlier = match syncing.take() {
        Some(t) if t.is_finished() => t.await.ok().flatten(),
        _ => None,
    };
    edit(app, id, |d| {
        d.received = received;
        d.total = Some(d.total.unwrap_or(received).max(received));
        if synced {
            d.synced = Some(received);
        } else if let Some(n) = earlier {
            d.synced = Some(n.max(d.synced.unwrap_or(0)));
        }
    });
    persist(app);
    if flag.load(Ordering::SeqCst) != RUN {
        return Ok(false);
    }
    if let Some(e) = failure {
        return Err(Transfer::Retry(e));
    }
    if total.is_some_and(|t| received < t) {
        return Err(Transfer::Retry("the connection closed early".into()));
    }
    Ok(true)
}

/// What a status code means for a download.
fn check_status(code: u16) -> Result<(), Transfer> {
    match code {
        200 | 206 => Ok(()),
        401 | 403 | 410 => Err(Transfer::Refused(code)),
        408 | 429 => Err(Transfer::Retry(format!("the server answered {code}"))),
        s if s >= 500 => Err(Transfer::Retry(format!("the server answered {s}"))),
        s => Err(Transfer::Fatal(format!("The download failed: the server answered {s}."))),
    }
}

/// Fetch in parallel: the file is split into ranges, each on its own
/// connection, written straight to its place in a file made at full size. A
/// server that does not do ranges falls back to one stream.
#[allow(clippy::too_many_arguments)]
async fn transfer_parallel<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    flag: &AtomicU8,
    client: &reqwest::Client,
    settings: &crate::settings::Settings,
    connections: u32,
    limiter: &Arc<Limiter>,
    watch_slow: bool,
) -> Result<bool, Transfer> {
    let mut item = edit(app, id, |_| {}).ok_or(Transfer::Fatal("The download is gone.".into()))?;
    let on_disk = Path::new(&item.archive_path).is_file();
    // Started on one stream (the server had no ranges then): carry on there,
    // rather than throw away what is on disk.
    if item.segments.is_empty() && !item.archive_path.is_empty() && on_disk && item.received > 0 {
        return transfer(app, id, flag, client, settings, limiter).await;
    }
    let resumable = !item.segments.is_empty() && !item.archive_path.is_empty() && on_disk;
    if !resumable {
        let probe = send(client.get(&item.url).header(reqwest::header::RANGE, "bytes=0-0"));
        let Some(probe) = unless_stopped(flag, probe).await else { return Ok(false) };
        let probe = probe?;
        check_status(probe.status().as_u16())?;
        let total = (probe.status().as_u16() == 206).then(|| content_range_of(&probe).and_then(|r| r.total)).flatten();
        let Some(total) = total.filter(|t| *t > 0) else {
            drop(probe);
            return transfer(app, id, flag, client, settings, limiter).await;
        };
        let etag = etag_of(&probe);
        let name = probe
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .and_then(filename_from_disposition)
            .or_else(|| (!item.file_name.is_empty()).then(|| item.file_name.clone()))
            .or_else(|| file_name_in_link(&item.url))
            .or_else(|| probe.url().path_segments().and_then(|mut s| s.next_back()).map(String::from))
            .map(|n| safe_name(&n))
            .unwrap_or_else(|| format!("{}.7z", safe_name(&item.meta.title)));
        drop(probe);
        let dir = archive_dir(settings);
        crate::storage::check_download_room(&dir, Path::new(&settings.library_dir), total, total, &item.meta.title).map_err(Transfer::Fatal)?;
        std::fs::create_dir_all(&dir).map_err(|e| Transfer::Fatal(format!("Cannot write to {}: {e}", dir.display())))?;
        let path = dir.join(&name);
        crate::hatchzip::forget(&path);
        let file = std::fs::File::create(&path).map_err(|e| Transfer::Fatal(format!("Cannot write {}: {e}", path.display())))?;
        file.set_len(total).map_err(|e| Transfer::Fatal(format!("Cannot make room for {}: {e}", path.display())))?;
        drop(file);
        let segments = split(total, connections);
        edit(app, id, |d| {
            d.file_name = name.clone();
            d.archive_path = path.to_string_lossy().into_owned();
            name_from_file(d);
            d.total = Some(total);
            d.received = 0;
            d.segments = segments;
            d.etag = etag;
            d.verified = false;
        });
        persist(app);
        item = edit(app, id, |_| {}).ok_or(Transfer::Fatal("The download is gone.".into()))?;
    }

    let path = PathBuf::from(&item.archive_path);
    let segments = Arc::new(item.segments.clone());
    let progress: Arc<Vec<AtomicU64>> = Arc::new(segments.iter().map(|s| AtomicU64::new(s.done.min(s.len()))).collect());
    // What has come in, flushed or not, for the speed and the bar. `progress` only
    // moves every 8 MB a connection, all of them about together: on a line under
    // about 30 Mbit that is a 32 MB step every 8 s, and the speed read 0 in between.
    let arrived: Arc<Vec<AtomicU64>> = Arc::new(segments.iter().map(|s| AtomicU64::new(s.done.min(s.len()))).collect());
    let expect = Arc::new(Expected { etag: item.etag.clone(), total: item.total });
    let stop = Arc::new(AtomicU8::new(RUN));
    // The unfinished ranges, handed out one at a time to whichever connection
    // is free. A connection that drops tries its range again by itself a few
    // times; only one that keeps failing stops taking more, and the whole
    // thing is then retried from where each range got to.
    let queue: Arc<Vec<usize>> = Arc::new((0..segments.len()).filter(|&i| segments[i].done < segments[i].len()).collect());
    let next = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let workers = (connections.max(1) as usize).min(queue.len().max(1));
    let mut tasks = Vec::new();
    for _ in 0..workers {
        let (client, url, path, progress, arrived, limiter, stop, segments, queue, next, expect) = (
            client.clone(),
            item.url.clone(),
            path.clone(),
            progress.clone(),
            arrived.clone(),
            limiter.clone(),
            stop.clone(),
            segments.clone(),
            queue.clone(),
            next.clone(),
            expect.clone(),
        );
        tasks.push(tokio::spawn(async move {
            loop {
                let Some(&i) = queue.get(next.fetch_add(1, Ordering::SeqCst)) else { return Ok(()) };
                if stop.load(Ordering::SeqCst) != RUN {
                    return Ok(());
                }
                let mut tries = 0;
                loop {
                    let before = progress[i].load(Ordering::SeqCst);
                    match fetch_range(&client, &url, &path, i, segments[i].clone(), &progress, &arrived, &stop, &limiter, &expect).await {
                        Ok(()) => break,
                        Err(Transfer::Retry(e)) => {
                            if progress[i].load(Ordering::SeqCst) > before {
                                tries = 0;
                            }
                            tries += 1;
                            if tries > RANGE_RETRIES || !wait_unless_stopped(&stop, 1u64 << tries).await {
                                return Err(Transfer::Retry(e));
                            }
                        }
                        Err(other) => {
                            // Nothing the others fetch from here on can be used.
                            stop.store(HALT, Ordering::SeqCst);
                            return Err(other);
                        }
                    }
                }
            }
        }));
    }

    let mut last_sum: u64 = arrived.iter().map(|p| p.load(Ordering::SeqCst)).sum();
    let mut meter = SpeedMeter::new(Instant::now(), last_sum);
    let mut last_checkpoint = Instant::now();
    let mut saving: Option<tokio::task::JoinHandle<()>> = None;
    let mut slow = SlowWatch::new(Instant::now());
    let mut crawled = false;
    let whole: u64 = segments.iter().map(Segment::len).sum::<u64>() + segments.first().map_or(0, |s| s.start);
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        // Pause and cancel reach the connections through `stop`.
        if flag.load(Ordering::SeqCst) != RUN {
            stop.store(flag.load(Ordering::SeqCst), Ordering::SeqCst);
        }
        if stop.load(Ordering::SeqCst) != RUN {
            // Give the connections a moment to stop on their own, then stop
            // the rest outright: one still waiting for a server to answer
            // used to hold Pause and Cancel for as long as the server took.
            // Safe, because progress only counts bytes already flushed.
            let deadline = Instant::now() + std::time::Duration::from_millis(1200);
            while Instant::now() < deadline && !tasks.iter().all(|t| t.is_finished()) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            for t in &tasks {
                t.abort();
            }
            break;
        }
        let sum: u64 = arrived.iter().map(|p| p.load(Ordering::SeqCst)).sum();
        let speed = meter.add(Instant::now(), sum);
        // Throttled to a crawl: stop the connections (what they have is
        // counted as on a pause) and let `fetch_all` start new ones.
        if watch_slow && slow.note(Instant::now(), speed, item.total.unwrap_or(whole).saturating_sub(sum)) {
            crawled = true;
            stop.store(HALT, Ordering::SeqCst);
            continue;
        }
        if sum > last_sum {
            edit(app, id, |d| d.notice = None);
        }
        last_sum = sum;
        edit(app, id, |d| {
            d.received = sum;
            d.speed = speed;
        });
        emit(app, false);
        // Saved in the background, so a slow disk never holds up the count.
        // Not every 5 s, as it was: each save flushes the whole archive to
        // disk while every connection is still writing into it, and on a hard
        // disk that flush held the writes up for seconds at a time. A crash
        // now costs at most this much downloading again.
        if last_checkpoint.elapsed() >= CHECKPOINT_EVERY && saving.as_ref().is_none_or(|t| t.is_finished()) {
            let (app, id, path, progress) = (app.clone(), id.to_string(), path.clone(), progress.clone());
            saving = Some(tokio::spawn(async move { checkpoint(&app, &id, &path, &progress).await }));
            last_checkpoint = Instant::now();
        }
        if tasks.iter().all(|t| t.is_finished()) {
            break;
        }
    }
    let mut retry = None;
    let mut fatal = None;
    let mut refused = None;
    let mut restart = None;
    let mut no_ranges = false;
    for t in tasks {
        match t.await {
            Ok(Ok(())) => {}
            Ok(Err(Transfer::Retry(e))) => retry = Some(e),
            Ok(Err(Transfer::Fatal(e))) => fatal = Some(e),
            Ok(Err(Transfer::Refused(code))) => refused = Some(code),
            Ok(Err(Transfer::Restart(e))) => restart = Some(e),
            Ok(Err(Transfer::NoRanges)) => no_ranges = true,
            Ok(Err(Transfer::Slow)) => {}
            Err(e) if e.is_cancelled() => {}
            Err(e) => retry = Some(e.to_string()),
        }
    }
    // Cancelled: the file is about to be deleted, so neither wait for the save
    // in flight nor flush it. Flushing gigabytes of a big download to disk
    // first is what kept Cancel spinning for minutes.
    if flag.load(Ordering::SeqCst) == CANCEL {
        if let Some(t) = saving {
            t.abort();
        }
        return Ok(false);
    }
    let sum: u64 = progress.iter().map(|p| p.load(Ordering::SeqCst)).sum();
    edit(app, id, |d| d.received = sum);
    if flag.load(Ordering::SeqCst) == PAUSE {
        // A pause does not wait for the disk. Syncing a big archive can take
        // minutes on a hard disk, and the row sat on Pausing all that while. The
        // save carries on by itself; a resume before it lands fetches again what
        // the last one had not reached, which is at most `CHECKPOINT_EVERY` of it.
        let (app, id, path, progress) = (app.clone(), id.to_string(), path.clone(), progress.clone());
        let mut last_save = tokio::spawn(async move {
            if let Some(t) = saving {
                let _ = t.await;
            }
            checkpoint(&app, &id, &path, &progress).await;
        });
        let _ = tokio::time::timeout(STOP_SAVE_WAIT, &mut last_save).await;
        return Ok(false);
    }
    if let Some(t) = saving {
        let _ = t.await;
    }
    checkpoint(app, id, &path, &progress).await;
    if let Some(e) = fatal {
        return Err(Transfer::Fatal(e));
    }
    if let Some(e) = restart {
        return Err(Transfer::Restart(e));
    }
    if no_ranges {
        return Err(Transfer::NoRanges);
    }
    if crawled && progress.iter().zip(segments.iter()).any(|(p, s)| p.load(Ordering::SeqCst) < s.len()) {
        return Err(Transfer::Slow);
    }
    if let Some(code) = refused {
        return Err(Transfer::Refused(code));
    }
    if flag.load(Ordering::SeqCst) != RUN {
        return Ok(false);
    }
    if let Some(e) = retry {
        return Err(Transfer::Retry(e));
    }
    if progress.iter().zip(segments.iter()).any(|(p, s)| p.load(Ordering::SeqCst) < s.len()) {
        return Err(Transfer::Retry("a range was left unfinished".into()));
    }
    Ok(true)
}

/// How often a ranged download saves how far it has got (`checkpoint`).
const CHECKPOINT_EVERY: std::time::Duration = std::time::Duration::from_secs(15);
/// The longest Pause waits for the disk before saying Paused.
const STOP_SAVE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Where an archive is written: the folder picked in Settings > Storage, else
/// `_downloads` in the library folder the game goes to.
fn archive_dir(settings: &crate::settings::Settings) -> PathBuf {
    match settings.download_dir.trim() {
        "" => Path::new(&settings.library_dir).join("_downloads"),
        own => PathBuf::from(own),
    }
}

/// The folder a file is in.
fn path_dir(file: &Path) -> PathBuf {
    file.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// What every range must agree with to be part of this download.
struct Expected {
    etag: Option<String>,
    total: Option<u64>,
}

/// Save how far each range has got, counting only bytes that are on the disk.
///
/// The counts are read first, the file is synced, and only then are they
/// saved. Saving the live counts, as this used to, could record bytes that
/// were still in the system's write cache: a crash or power cut lost them,
/// the resume skipped them, and the archive came out with stretches of zeros
/// that only 7-Zip's checksum noticed.
async fn checkpoint<R: Runtime>(app: &AppHandle<R>, id: &str, path: &Path, progress: &[AtomicU64]) {
    let counts: Vec<u64> = progress.iter().map(|p| p.load(Ordering::SeqCst)).collect();
    let file = path.to_path_buf();
    let synced = tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(file).and_then(|f| f.sync_data()))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);
    if !synced {
        return;
    }
    edit(app, id, |d| {
        for (s, n) in d.segments.iter_mut().zip(&counts) {
            s.done = (*n).min(s.len());
        }
    });
    persist(app);
}

/// One range, on one connection, written at its own offset. Progress is only
/// counted for bytes that have been written.
#[allow(clippy::too_many_arguments)]
async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    index: usize,
    seg: Segment,
    progress: &[AtomicU64],
    arrived: &[AtomicU64],
    stop: &AtomicU8,
    limiter: &Limiter,
    expect: &Expected,
) -> Result<(), Transfer> {
    let done = progress[index].load(Ordering::SeqCst);
    // A retry starts from what is on the disk; what was only buffered comes again.
    arrived[index].store(done, Ordering::SeqCst);
    if done >= seg.len() {
        return Ok(());
    }
    let from = seg.start + done;
    let res = send(client.get(url).header(reqwest::header::RANGE, format!("bytes={from}-{}", seg.end))).await?;
    check_status(res.status().as_u16())?;
    if res.status().as_u16() != 206 {
        return Err(Transfer::NoRanges);
    }
    // Exactly the bytes asked for, of the same file. Writing whatever came
    // back at the offset that was asked for is how a range of one file, or
    // the wrong stretch of the right one, ended up inside an archive.
    let range = content_range_of(&res);
    if range.and_then(|r| r.start) != Some(from) || range.and_then(|r| r.end).is_some_and(|e| e > seg.end) {
        return Err(Transfer::Restart(format!("asked for {from}-{}, the server sent {range:?}", seg.end)));
    }
    if !same_file(expect.etag.as_deref(), expect.total, etag_of(&res).as_deref(), range.and_then(|r| r.total)) {
        return Err(Transfer::Restart("the file on the server changed".into()));
    }
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .await
        .map_err(|e| Transfer::Fatal(format!("Cannot write {}: {e}", path.display())))?;
    file.seek(std::io::SeekFrom::Start(from)).await.map_err(|e| Transfer::Fatal(e.to_string()))?;
    let mut file = tokio::io::BufWriter::with_capacity(WRITE_BUFFER, file);
    let mut written = done;
    let mut stream = res.bytes_stream();
    let mut failure = None;
    let mut last_data = Instant::now();
    loop {
        let next = tokio::time::timeout(STOP_POLL, stream.next()).await;
        if stop.load(Ordering::SeqCst) != RUN {
            break;
        }
        let chunk = match next {
            Err(_) if last_data.elapsed() >= STALL_TIMEOUT => {
                failure = Some(format!("no data for {} s", STALL_TIMEOUT.as_secs()));
                break;
            }
            Err(_) => continue,
            Ok(None) => break,
            Ok(Some(Ok(chunk))) => chunk,
            Ok(Some(Err(e))) => {
                failure = Some(e.to_string());
                break;
            }
        };
        let take = (chunk.len() as u64).min(seg.len() - written) as usize;
        if let Err(e) = file.write_all(&chunk[..take]).await {
            return Err(Transfer::Fatal(format!("Writing the download failed: {e}")));
        }
        written += take as u64;
        arrived[index].store(written, Ordering::SeqCst);
        limiter.take(take as u64).await;
        last_data = Instant::now();
        if written >= seg.len() {
            break;
        }
        // Counted once the bytes are flushed often enough to matter: every
        // 8 MB, and at the end.
        if written - progress[index].load(Ordering::SeqCst) >= 8 * 1024 * 1024 {
            file.flush().await.map_err(|e| Transfer::Fatal(e.to_string()))?;
            progress[index].store(written, Ordering::SeqCst);
        }
    }
    file.flush().await.map_err(|e| Transfer::Fatal(e.to_string()))?;
    progress[index].store(written, Ordering::SeqCst);
    if let Some(e) = failure {
        return Err(Transfer::Retry(e));
    }
    if written < seg.len() && stop.load(Ordering::SeqCst) == RUN {
        return Err(Transfer::Retry("the connection closed early".into()));
    }
    Ok(())
}

/// Unpack into the library folder, find the exe, and list the game.
async fn install<R: Runtime>(app: &AppHandle<R>, id: &str, settings: &crate::settings::Settings, flag: &Arc<AtomicU8>) -> Result<(), String> {
    if flag.load(Ordering::SeqCst) != RUN {
        return Err(STOPPED.into());
    }
    set_status(app, id, Status::Extracting, None);
    let item = edit(app, id, |_| {}).ok_or("The download is gone.")?;
    let archive = PathBuf::from(&item.archive_path);
    let title = if is_placeholder(&item.meta.title, item.slug.as_deref()) {
        title_from_file(&item.file_name)
    } else {
        item.meta.title.clone()
    };
    // An update goes where the game already is - whichever library folder
    // that is, under whatever its folder is called - so saves stay with it.
    let folders = crate::settings::all_folders(settings);
    let games = library::load(app).unwrap_or_default();
    let existing = item.slug.as_ref().and_then(|slug| games.iter().find(|g| g.slug.as_deref() == Some(slug.as_str())).cloned());
    // A running game's files are in use: the merge would stop halfway through,
    // over a game with no way back. The archive stays, so Retry is quick.
    if let Some(game) = existing.as_ref().filter(|g| library::is_running(app, &g.id)) {
        return Err(close_first(&game.title));
    }
    // An update of an installed game, with the backup folder the player chose:
    // what it replaces is copied there first, so it can be undone
    // (game_update.rs). No folder chosen, no copies, as before.
    let from_version = existing.as_ref().and_then(|g| g.version.clone());
    let update_backup = existing
        .as_ref()
        .filter(|_| !settings.update_backup_dir.trim().is_empty())
        .map(|_| (PathBuf::from(settings.update_backup_dir.trim()), crate::game_update::data_dir(app)));
    // One kept outside the library folders too: it used to land in the default
    // folder as a second copy, with the old one and its saves left behind.
    let dest = existing
        .as_ref()
        .filter(|g| g.launcher.is_none())
        .and_then(|g| crate::storage::game_folder(&folders, &g.install_dir).or_else(|| crate::storage::outside_root(&games, &folders, g).ok()))
        // Those are canonical, \\?\D:\..., which a .bat cannot start from and
        // Properties would show.
        .map(|dir| dunce::simplified(&dir).to_path_buf())
        .unwrap_or_else(|| Path::new(&settings.library_dir).join(plain_name(&title, item.slug.as_deref())));
    // An old repair backup must never restore files over a newer release.
    // Keep the same lock as the fixer through installation and library saving.
    let _repair_lock = if dest.join(".kryoto-repair").exists() {
        let guard = kryoto_repair::journal::lock(&dest)?;
        if kryoto_repair::journal::load(&dest)?.is_some_and(|r| r.state != "undone") {
            return Err("Undo the game's repair under Properties > Repair before installing another release.".into());
        }
        Some(guard)
    } else {
        None
    };
    let parent = dest.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(&settings.library_dir));
    let staging = parent.join(format!(".{}.installing", plain_name(&title, item.slug.as_deref())));
    // An unpack in place to carry on from, with what it unpacked gone: the
    // archive is partly eaten and HatchZip would say done with nothing there.
    if crate::hatchzip::resumes(&archive) && !staging.is_dir() {
        crate::logging::warn("download", &format!("{id}: an unpack in place lost its folder; fetching the game again"));
        discard(app, id);
        return Err("The files already unpacked are gone, so this has to be downloaded again. Press Retry.".into());
    }
    edit(app, id, |d| d.staging = Some(staging.to_string_lossy().into_owned()));
    persist(app);
    // A download that is the program itself goes in as it is, and is what Play starts.
    let program = is_program(&archive).then(|| item.file_name.clone());
    let unpacked = unpacked_size(&archive);
    let archive_len = std::fs::metadata(&archive).map(|m| m.len()).unwrap_or(0);
    // Unpacking in place (hatchzip.rs): asked for in Settings > Storage, or
    // forced when the drive has room for the game but not for the game and
    // its archive side by side. It eats the archive, so a player who keeps
    // archives only gets it when the alternative is no game at all.
    // An archive kept on another drive (Settings > Storage) takes no room here,
    // and eating it would free none: that drive never unpacks in place.
    let same_drive = crate::storage::same_drive(&path_dir(&archive), &parent);
    let short = same_drive && program.is_none() && unpacked.is_some_and(|size| crate::storage::check_room(&parent, size + archive_len, &title).is_err());
    let in_place = same_drive && program.is_none() && (settings.extract_in_place || short) && (settings.delete_archives || short);
    let hatchzip = if in_place || crate::hatchzip::resumes(&archive) { crate::hatchzip::ensure(app).await } else { None };
    if hatchzip.is_none() {
        if let Some(size) = unpacked {
            crate::storage::check_room(&parent, size, &title)?;
        }
    }

    // Real 7-Zip on hand before unpacking a .7z: the built-in unpacker cannot
    // read every filter (the ARM64 one is "UnsupportedCompressionMethod([10])")
    // and Windows' own tar.exe has no LZMA at all.
    #[cfg(windows)]
    if archive.extension().is_some_and(|e| e.eq_ignore_ascii_case("7z")) {
        ensure_7zr(app).await;
    }

    let progress_app = app.clone();
    let progress_id = id.to_string();
    let (dest_clone, staging_clone) = (dest.clone(), staging.clone());
    let stop = flag.clone();
    let lone = program.clone();
    let room_needed = unpacked.map(|size| size + if same_drive { archive_len } else { 0 }).unwrap_or(archive_len * 2);
    let (title_clone, parent_clone) = (title.clone(), parent.clone());
    let (journal_slug, to_version) = (item.slug.clone().unwrap_or_default(), item.meta.version.clone());
    let damaged = tauri::async_runtime::spawn_blocking(move || -> Result<(Vec<String>, Option<crate::game_update::Journal>), String> {
        let (title, parent) = (title_clone, parent_clone);
        // An unpack HatchZip left half done carries on from where it got to;
        // what it had already written is part of that.
        let resuming = hatchzip.is_some() && crate::hatchzip::resumes(&archive);
        if !resuming {
            let _ = std::fs::remove_dir_all(&staging_clone);
        }
        if let Some(name) = lone {
            std::fs::create_dir_all(&staging_clone).map_err(|e| e.to_string())?;
            let placed = staging_clone.join(safe_name(&name));
            std::fs::copy(&archive, &placed).map_err(|e| format!("Installing {name} failed: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&placed, std::fs::Permissions::from_mode(0o755));
            }
            move_tree(&staging_clone, &dest_clone).map_err(|e| format!("Installing into {} failed: {e}", dest_clone.display()))?;
            let _ = std::fs::remove_dir_all(&staging_clone);
            return Ok((Vec::new(), None));
        }
        let progress = |done: u64, total: Option<u64>| {
            edit(&progress_app, &progress_id, |d| {
                d.extracted = done;
                d.extract_total = total;
            });
            emit(&progress_app, false);
        };
        let unpacked = match hatchzip.as_deref() {
            Some(tool) => match extract_in_place(tool, &archive, &staging_clone, &parent, &title, room_needed, unpacked, resuming, &progress, &stop) {
                // No gain (a solid 7z): 7-Zip, with the room check it skipped.
                Ok(None) => crate::storage::check_room(&parent, unpacked.unwrap_or(archive_len), &title)
                    .and_then(|()| extract_until(&archive, &staging_clone, &progress, &stop)),
                Ok(Some(())) => Ok(Vec::new()),
                Err(e) => Err(e),
            },
            None => extract_until(&archive, &staging_clone, &progress, &stop),
        };
        // An unpack in place has eaten that much of its archive: what it wrote
        // is the only copy, and Resume carries on from it.
        let carries_on = hatchzip.is_some() && crate::hatchzip::resumes(&archive) && stop.load(Ordering::SeqCst) != CANCEL;
        if unpacked.is_err() && !carries_on {
            // Half a game is no use to anyone; it goes, without holding up the
            // Pause or Cancel that asked for it. A failed unpack starts over too.
            throw_away(staging_clone.clone());
        }
        if unpacked.as_ref().is_err_and(|e| e == STOPPED) {
            return Err(STOPPED.into());
        }
        let damaged = unpacked?;
        // Unpacked beside the game rather than into it, then merged in: an
        // archive that wraps everything in one folder does not end up nested
        // a level down, and an update overwrites the files it ships while
        // leaving saves and settings the game wrote next to itself.
        let top = single_child_dir(&staging_clone).unwrap_or_else(|| staging_clone.clone());
        let journal = match &update_backup {
            Some((root, data)) => {
                let mut j = crate::game_update::prepare(crate::game_update::Prepare {
                    slug: &journal_slug,
                    title: &title,
                    staging: &top,
                    dest: &dest_clone,
                    backup_root: root,
                    from_version: from_version.clone(),
                    to_version: to_version.clone(),
                })?;
                j.previous = crate::game_update::remember(data, &j);
                Some(j)
            }
            None => None,
        };
        if let Err(e) = move_tree(&top, &dest_clone) {
            return Err(match journal {
                Some(mut j) => match crate::game_update::restore(&mut j) {
                    Ok(()) => format!("The update could not be applied ({e}), so the previous build was put back."),
                    Err(r) => format!("The update could not be applied ({e}), and putting the previous build back failed: {r}"),
                },
                None => format!("Installing into {} failed: {e}", dest_clone.display()),
            });
        }
        let _ = std::fs::remove_dir_all(&staging_clone);
        Ok((damaged, journal))
    })
    .await
    .map_err(|e| e.to_string())??;
    let (damaged, journal) = damaged;
    let warning = damage_warning(&damaged, item.verified);
    if let Some(w) = &warning {
        crate::logging::error("install", &format!("{title}: {w}"));
    }

    let (root, executable) = match program.as_deref() {
        Some(name) => (dest.clone(), safe_name(name)),
        None => locate(&dest, &item.meta.executable),
    };
    // Programs that were in the archive and are not in the folder: taken
    // while it unpacked (antivirus.rs), said now and kept in the release record
    // so the game's page can bring them back.
    let lost: Vec<(String, u64)> = if Path::new(&item.archive_path).is_file() { crate::antivirus::unpack_losses(Path::new(&item.archive_path), &dest) } else { Vec::new() }
        .into_iter()
        .filter_map(|(rel, size)| Some((dest.join(&rel).strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/"), size)))
        .collect();
    let game = library::upsert_installed(
        app,
        LibraryGame {
            title: title.clone(),
            slug: item.slug.clone(),
            cover: item.meta.cover.clone(),
            hero: item.meta.hero.clone(),
            logo: item.meta.logo.clone(),
            header: item.meta.header.clone(),
            install_dir: root.to_string_lossy().into_owned(),
            executable,
            default_args: item.meta.default_args.clone(),
            entries: item.meta.entries.clone(),
            source: item.meta.source.clone(),
            version: item.meta.version.clone(),
            // What the archive was, so a replaced upload reads as an update.
            build_sha: item.sha256.clone().filter(|_| item.verified),
            // An older build keeps itself; the current one follows updates.
            pinned_version: item.release.clone(),
            short: item.meta.short.clone(),
            developer: item.meta.developer.clone(),
            nsfw: item.meta.nsfw,
            steam_branch: item.meta.steam_branch.clone(),
            vr: item.meta.vr.clone(),
            ..Default::default()
        },
    )?;
    // Only now, with the library saying so, is the update done; until here a
    // crash leaves the journal at "applying" and the next start rolls it back.
    if let Some(mut j) = journal {
        j.state = crate::game_update::State::Applied;
        let _ = j.save();
        crate::game_update::drop_previous(&j);
    }
    if settings.delete_archives {
        let _ = std::fs::remove_file(&item.archive_path);
    }
    // Its art on disk now, so the Library shows it with no connection.
    crate::art::prefetch(app, [&game.cover, &game.hero, &game.logo, &game.header].into_iter().flatten().cloned().collect());
    edit(app, id, |d| {
        d.install_dir = Some(game.install_dir.clone());
        d.game_id = Some(game.id.clone());
        d.warning = warning.clone();
        d.staging = None;
    });
    set_status(app, id, Status::Installed, None);
    // What Kryoto's release put here, so a later community update can tell
    // what its updater changed from what anything else did (community_offer.rs).
    crate::community_offer::record_release(&game, &lost);
    crate::antivirus::after_install(app, &game.id, &title, &root, &lost, item.added_at);
    if let Some(slug) = &game.slug {
        crate::antivirus::restored(slug);
    }
    let _ = app.emit("library-changed", ());
    if settings.notify_downloads {
        let _ = app.emit(
            "notify",
            match &warning {
                None => Notice::new(&format!("{title} is ready to play"), "Installed and added to your library.", Some(game.id)).sound("ready"),
                Some(_) => Notice::new(
                    &format!("{title} is installed, with a warning"),
                    "7-Zip reported a damaged file. It may still run: open Downloads for details.",
                    Some(game.id),
                )
                .sound("ready"),
            },
        );
    }
    Ok(())
}

/// Unpack in place with HatchZip (hatchzip.rs). `Ok(None)` when its check
/// says the archive gains nothing from it (a solid single-block 7z) and 7-Zip
/// should do the work; `Ok(Some(()))` when the game is in `staging` and the
/// archive is gone. `room_needed` is what 7-Zip would have needed, for the
/// message when not even in-place fits.
#[allow(clippy::too_many_arguments)]
fn extract_in_place(
    tool: &Path,
    archive: &Path,
    staging: &Path,
    parent: &Path,
    title: &str,
    room_needed: u64,
    unpacked: Option<u64>,
    resuming: bool,
    progress: &dyn Fn(u64, Option<u64>),
    stop: &AtomicU8,
) -> Result<Option<()>, String> {
    use crate::hatchzip::{self, Failure};
    let explain = |f: Failure| -> String {
        match f {
            Failure::Stopped => STOPPED.into(),
            Failure::NoSpace => crate::storage::check_room(parent, room_needed, title).err().unwrap_or_else(|| {
                format!("Not enough space on the drive for {title}, even unpacking it in place. Free some up, or add a library folder on another drive in Settings > Storage.")
            }),
            Failure::Damaged(text) => {
                // Gone, so that Retry does what this says: kept, it would only
                // be read and refused again.
                let _ = std::fs::remove_file(archive);
                hatchzip::forget(archive);
                format!("The archive is damaged and could not be unpacked ({}). Retry downloads it again.", first_line(&text))
            }
            Failure::Password => "The archive wants a password, which Kryoto does not have. Report the release on its kryo.to page.".into(),
            Failure::Other(text) => format!("HatchZip could not unpack it: {}. Press Retry to carry on from where it stopped.", first_line(&text)),
        }
    };
    if !resuming {
        let plan = hatchzip::check(tool, archive, hatchzip::KEEP_FREE).map_err(explain)?;
        if !plan.gains() {
            crate::logging::info("hatchzip", &format!("{title}: no gain unpacking in place (solid archive); 7-Zip instead"));
            return Ok(None);
        }
        crate::logging::info("hatchzip", &format!("{title}: unpacking in place, {} needed instead of {}", crate::storage::human_size(plan.required), crate::storage::human_size(plan.standard)));
    }
    std::fs::create_dir_all(staging).map_err(|e| format!("Cannot create {}: {e}", staging.display()))?;
    hatchzip::extract(tool, archive, staging, hatchzip::KEEP_FREE, unpacked, progress, stop).map_err(explain)?;
    Ok(Some(()))
}

fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no details");
    line.trim_start_matches("error:").trim().chars().take(300).collect()
}

/// What to tell the player when the unpacker reported damaged files.
fn damage_warning(damaged: &[String], verified: bool) -> Option<String> {
    let (first, rest) = damaged.split_first()?;
    let files = match rest.len() {
        0 => first.clone(),
        n => format!("{first} and {n} more"),
    };
    let origin = if verified {
        "The download itself is intact (it matches kryo.to's checksum), so the damage is inside the archive as it was packed."
    } else {
        "The download may have been cut or corrupted on the way."
    };
    Some(format!(
        "7-Zip reported a checksum error in {files}. The files were unpacked anyway and many games run fine like this. {origin} If the game crashes or looks wrong, try a mirror or report the release."
    ))
}

/// A download that is a program rather than an archive of one: an installer,
/// a one-file game, an AppImage. 7-Zip opens most .exe files as archives of
/// their sections, so these are never handed to it.
fn is_program(file: &Path) -> bool {
    file.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| e == "exe" || e == "appimage")
}

/// What a .7z says it unpacks to, from its headers. `None` for other formats.
fn unpacked_size(archive: &Path) -> Option<u64> {
    let ext = archive.extension()?.to_string_lossy().to_ascii_lowercase();
    if ext != "7z" {
        return None;
    }
    let a = sevenz_rust::Archive::open(archive).ok()?;
    Some(a.files.iter().map(|f| f.size()).sum())
}

/// The one folder inside `dir`, when that is all there is.
pub(crate) fn single_child_dir(dir: &Path) -> Option<PathBuf> {
    let items: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
    match items.as_slice() {
        [only] if only.path().is_dir() => Some(only.path()),
        _ => None,
    }
}

/// Move everything under `from` into `to`, replacing files that are there and
/// keeping the ones that are not.
pub fn move_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            if target.is_dir() {
                move_tree(&entry.path(), &target)?;
            } else {
                if target.exists() {
                    std::fs::remove_file(&target)?;
                }
                std::fs::rename(entry.path(), &target)?;
            }
        } else {
            if target.exists() {
                std::fs::remove_file(&target)?;
            }
            std::fs::rename(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Reject archive entries that would land outside the install folder.
fn safe_join(dest: &Path, name: &str) -> Option<PathBuf> {
    let rel = Path::new(name);
    if rel.components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return None;
    }
    Some(dest.join(rel))
}

/// Unpack `archive` into `dest`, reporting uncompressed bytes as it goes.
///
/// Real 7-Zip does it when there is one (`seven_zip_tools`), with progress.
/// Without it, a .7z is read in-process, and when that refuses (Forge packs
/// with 7-Zip, which can use the BCJ2 filter the Rust decoder does not
/// implement) the system's libarchive (`tar`) finishes the job without progress.
///
/// Returns the files the unpacker reported as damaged (a CRC mismatch inside
/// the archive). 7-Zip and libarchive still write every file when that
/// happens, and the game often runs anyway (a checksum wrong in the archive's
/// own header, a byte off in an asset), so that is a warning, not a failure.
/// Any other error (no space, no permission, not an archive) still fails.
pub fn extract(archive: &Path, dest: &Path, progress: &dyn Fn(u64, Option<u64>)) -> Result<Vec<String>, String> {
    extract_until(archive, dest, progress, &AtomicU8::new(RUN))
}

/// What `extract_until`, `verify` and `install` answer when they were stopped
/// (Pause or Cancel).
pub const STOPPED: &str = "stopped";

/// `extract`, giving up as soon as `stop` is no longer RUN: 7-Zip is killed
/// and the built-in unpacker stops between reads. A big game takes minutes to
/// unpack, and Cancel pressed in the middle used to wait for all of it.
pub fn extract_until(archive: &Path, dest: &Path, progress: &dyn Fn(u64, Option<u64>), stop: &AtomicU8) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("Cannot create {}: {e}", dest.display()))?;
    let is_7z = archive
        .extension()
        .map(|e| e.to_string_lossy().eq_ignore_ascii_case("7z"))
        .unwrap_or(false);
    // Real 7-Zip first. The built-in decoder is pure Rust, single-threaded and
    // a few times slower than 7-Zip's own, so a big game took noticeably longer
    // to unpack in Kryoto than by hand. 7-Zip also reads every filter (BCJ2,
    // ARM64) and reports damaged files by name.
    let mut errors = Vec::new();
    for tool in seven_zip_tools(is_7z) {
        match extract_with_7zip(&tool, archive, dest, progress, stop) {
            None => continue,
            Some(Err(_)) if stop.load(Ordering::SeqCst) != RUN => return Err(STOPPED.into()),
            Some(Ok(damaged)) => return Ok(damaged),
            Some(Err(e)) => errors.push(e),
        }
    }
    if is_7z {
        match extract_7z(archive, dest, progress, stop) {
            Ok(()) => return Ok(Vec::new()),
            Err(_) if stop.load(Ordering::SeqCst) != RUN => return Err(STOPPED.into()),
            Err(e) => errors.push(e),
        }
    }
    progress(0, None);
    extract_with_tar(archive, dest).map_err(|t| {
        errors.push(t);
        errors.join("; ")
    })
}

/// Where 7-Zip's console might be: an installed 7-Zip, then the standalone
/// `7zr.exe` this client keeps (which reads only .7z), or on Linux whatever
/// the distribution calls it. Missing ones are skipped when they fail to start.
fn seven_zip_tools(is_7z: bool) -> Vec<PathBuf> {
    #[cfg(windows)]
    let list = {
        let mut list: Vec<PathBuf> = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .map(|root| PathBuf::from(root).join(r"7-Zip\7z.exe"))
            .filter(|p| p.is_file())
            .collect();
        // ProgramFiles and ProgramW6432 are usually the same folder.
        list.dedup();
        if is_7z {
            if let Some(zr) = SEVEN_ZR.get().filter(|p| p.is_file()) {
                list.push(zr.clone());
            }
        }
        list
    };
    #[cfg(not(windows))]
    let list = {
        let _ = is_7z;
        ["7zz", "7z", "7za"].iter().map(PathBuf::from).collect()
    };
    list
}

/// The percentage at the start of one of 7-Zip's progress lines
/// (`" 42% 118 - Game\data.pak"`).
pub fn seven_zip_percent(line: &str) -> Option<u64> {
    let line = line.trim_start();
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 || !line[digits..].starts_with('%') {
        return None;
    }
    line[..digits].parse().ok().filter(|p| *p <= 100)
}

/// Unpack with 7-Zip's console, following its progress. `None` when the tool
/// is not there; otherwise what it reported damaged, or why it failed.
fn extract_with_7zip(tool: &Path, archive: &Path, dest: &Path, progress: &dyn Fn(u64, Option<u64>), stop: &AtomicU8) -> Option<Result<Vec<String>, String>> {
    use std::io::Read;
    // The bar counts unpacked bytes when the archive says how many there are
    // (a .7z's headers do), and the archive's own size otherwise.
    let scale = unpacked_size(archive).or_else(|| std::fs::metadata(archive).ok().map(|m| m.len())).filter(|s| *s > 0);
    let mut cmd = std::process::Command::new(tool);
    // -bso0: no listing. -bsp1: progress on stdout. -bse2: errors on stderr.
    cmd.arg("x").arg("-y").arg("-bso0").arg("-bsp1").arg("-bse2").arg(format!("-o{}", dest.display())).arg(archive);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return Some(Err(format!("{}: {e}", tool.display()))),
    };
    let mut stderr = child.stderr.take()?;
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    progress(0, scale);
    if let Some(mut out) = child.stdout.take() {
        // Progress lines overwrite each other with backspaces, not newlines.
        let mut buf = [0u8; 4096];
        let mut line = Vec::new();
        let mut last = Instant::now();
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 {
                break;
            }
            // 7-Zip writes its progress several times a second: stopped, it
            // is killed here rather than left to unpack the whole game.
            if stop.load(Ordering::SeqCst) != RUN {
                let _ = child.kill();
                let _ = child.wait();
                return Some(Err(STOPPED.into()));
            }
            for &b in &buf[..n] {
                if matches!(b, b'\x08' | b'\r' | b'\n') {
                    if let (Some(pct), Some(total)) = (seven_zip_percent(&String::from_utf8_lossy(&line)), scale) {
                        if last.elapsed().as_millis() > 200 {
                            progress(total * pct / 100, Some(total));
                            last = Instant::now();
                        }
                    }
                    line.clear();
                } else {
                    line.push(b);
                }
            }
        }
    }
    let status = child.wait();
    let errors = errors.join().unwrap_or_default();
    match status {
        Ok(s) if s.success() => {
            if let Some(total) = scale {
                progress(total, Some(total));
            }
            Some(Ok(Vec::new()))
        }
        Ok(_) => {
            let wrote_something = std::fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(false);
            if let Some(damaged) = damaged_files(&errors).filter(|_| wrote_something) {
                return Some(Ok(damaged));
            }
            Some(Err(format!("7-Zip: {}", errors.trim())))
        }
        Err(e) => Some(Err(format!("{}: {e}", tool.display()))),
    }
}

/// The files an unpacker's error output says are damaged, when damage is ALL
/// it reports. `None` when anything else went wrong.
///
/// libarchive (Windows' tar.exe, bsdtar): `Game.pck: 7-Zip bad CRC 0x... should
/// be 0x...` then `Error exit delayed from previous errors.` 7-Zip: `ERROR: CRC
/// Failed : Game.pck` or `ERROR: Data Error : Game.pck`, plus its summary lines.
fn damaged_files(stderr: &str) -> Option<Vec<String>> {
    let mut damaged = Vec::new();
    for line in stderr.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let lower = line.to_ascii_lowercase();
        if let Some(at) = lower.find(": 7-zip bad crc").or_else(|| lower.find(": bad crc")) {
            damaged.push(line[..at].trim().to_string());
        } else if let Some(rest) = ["error: crc failed", "error: data error", "crc failed", "data error"]
            .iter()
            .find_map(|p| lower.strip_prefix(p).map(|r| r.len()))
        {
            let name = line[line.len() - rest..].trim().trim_start_matches(':').trim();
            let name = name.strip_suffix("wrong password?").unwrap_or(name).trim().trim_end_matches(". ").to_string();
            if !name.is_empty() {
                damaged.push(name);
            }
        } else if lower.contains("error exit delayed from previous errors")
            || lower.starts_with("sub items errors")
            || lower.starts_with("archives with errors")
            || lower.starts_with("errors:")
        {
            // A summary of the lines above.
        } else {
            return None;
        }
    }
    (!damaged.is_empty()).then_some(damaged)
}

fn extract_7z(archive: &Path, dest: &Path, progress: &dyn Fn(u64, Option<u64>), stop: &AtomicU8) -> Result<(), String> {
    let total: u64 = sevenz_rust::Archive::open(archive)
        .map_err(|e| format!("Not a readable 7z ({e})"))?
        .files
        .iter()
        .map(|f| f.size())
        .sum();
    let mut done = 0u64;
    let mut last = Instant::now();
    progress(0, Some(total));
    sevenz_rust::decompress_file_with_extract_fn(archive, dest, |entry, reader, _| {
        let Some(path) = safe_join(dest, entry.name()) else {
            return Err(sevenz_rust::Error::other(format!("unsafe path in archive: {}", entry.name())));
        };
        if entry.is_directory() {
            std::fs::create_dir_all(&path).map_err(sevenz_rust::Error::io)?;
            return Ok(true);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(sevenz_rust::Error::io)?;
        }
        let file = std::fs::File::create(&path).map_err(sevenz_rust::Error::io)?;
        let mut writer = std::io::BufWriter::with_capacity(1 << 20, file);
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if stop.load(Ordering::SeqCst) != RUN {
                return Err(sevenz_rust::Error::other(STOPPED));
            }
            let n = reader.read(&mut buf).map_err(sevenz_rust::Error::io)?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut writer, &buf[..n]).map_err(sevenz_rust::Error::io)?;
            done += n as u64;
            if last.elapsed().as_millis() > 200 {
                progress(done, Some(total));
                last = Instant::now();
            }
        }
        std::io::Write::flush(&mut writer).map_err(sevenz_rust::Error::io)?;
        Ok(true)
    })
    .map_err(|e| format!("Unpacking failed ({e})"))?;
    progress(total, Some(total));
    Ok(())
}

/// The standalone 7-Zip (`7zr.exe`) this client keeps, once fetched.
#[cfg(windows)]
static SEVEN_ZR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Fetch 7-Zip's official standalone console once, into the app's data folder.
/// Best effort: without it the unpack falls back to tar.exe as before.
#[cfg(windows)]
pub(crate) async fn ensure_7zr<R: Runtime>(app: &AppHandle<R>) {
    if SEVEN_ZR.get().is_some() {
        return;
    }
    let Ok(dir) = app.path().app_data_dir().map(|d| d.join("tools")) else { return };
    let file = dir.join("7zr.exe");
    if file.metadata().map(|m| m.len() > 100_000).unwrap_or(false) {
        let _ = SEVEN_ZR.set(file);
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    let fetched = async {
        let res = reqwest::Client::builder()
            .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .ok()?
            .get("https://www.7-zip.org/a/7zr.exe")
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None;
        }
        let bytes = res.bytes().await.ok()?;
        (bytes.len() > 100_000 && bytes.starts_with(b"MZ")).then_some(bytes)
    }
    .await;
    match fetched {
        Some(bytes) => {
            let tmp = file.with_extension("part");
            if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &file).is_ok() {
                let _ = SEVEN_ZR.set(file);
            }
        }
        None => crate::logging::warn("install", "could not fetch 7zr.exe; unpacking with tar.exe"),
    }
}

fn extract_with_tar(archive: &Path, dest: &Path) -> Result<Vec<String>, String> {
    // libarchive, after 7-Zip (`extract`) had its turn. Windows' tar.exe reads
    // zip and rar, but not LZMA, which is most .7z. GNU tar cannot read 7z, so
    // on Linux bsdtar comes first.
    #[cfg(windows)]
    let candidates: Vec<PathBuf> = {
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        vec![PathBuf::from(system).join(r"System32\tar.exe")]
    };
    #[cfg(not(windows))]
    let candidates: Vec<PathBuf> = vec![PathBuf::from("bsdtar"), PathBuf::from("tar")];
    let mut last_error = String::from("no archive tool found (install 7-Zip or bsdtar)");
    for tool in candidates {
        let mut cmd = std::process::Command::new(&tool);
        cmd.arg("-xf").arg(archive).arg("-C").arg(dest);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        match cmd.output() {
            Ok(out) if out.status.success() => return Ok(Vec::new()),
            Ok(out) => {
                let errors = format!("{}\n{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
                let wrote_something = std::fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(false);
                if let Some(damaged) = damaged_files(&errors).filter(|_| wrote_something) {
                    return Ok(damaged);
                }
                last_error = errors.trim().to_string();
            }
            Err(e) => last_error = e.to_string(),
        }
    }
    Err(format!("Could not unpack the archive: {last_error}"))
}

/// Work out the game's root folder and its exe after unpacking.
///
/// An archive may or may not wrap everything in one top folder, and the release
/// names its exe relative to the game's root (`bin/win64/Game.exe`), so: find
/// that exe anywhere shallow, and the root is whatever sits above its relative
/// path. A release may name a script (`Run Me.bat`) just the same. Without
/// one, a start script at the game's top level, because a game that ships one
/// is telling you that is how it starts; else the biggest plausible exe near
/// the top.
pub fn locate(dest: &Path, wanted: &str) -> (PathBuf, String) {
    let wanted = wanted.replace('\\', "/").trim_matches('/').to_string();
    let exes = find_exes(dest, 4);
    if !wanted.is_empty() {
        let wanted_lower = wanted.to_ascii_lowercase();
        for exe in &exes {
            let rel = exe.strip_prefix(dest).unwrap_or(exe).to_string_lossy().replace('\\', "/");
            if rel.to_ascii_lowercase() == wanted_lower || rel.to_ascii_lowercase().ends_with(&format!("/{wanted_lower}")) {
                let depth = wanted.split('/').count();
                let mut root = exe.clone();
                for _ in 0..depth {
                    root.pop();
                }
                return (root, wanted);
            }
        }
    }
    // One folder and nothing else: that folder is the game.
    let mut root = dest.to_path_buf();
    if let Ok(read) = std::fs::read_dir(dest) {
        let items: Vec<_> = read.flatten().collect();
        if items.len() == 1 && items[0].path().is_dir() {
            root = items[0].path();
        }
    }
    if let Some(script) = start_script(&root) {
        return (root, script);
    }
    const JUNK: [&str; 8] = ["unins", "crash", "redist", "vcredist", "dxsetup", "setup", "report", "helper"];
    let best = exes
        .iter()
        .filter(|p| p.starts_with(&root))
        .filter(|p| is_exe(p))
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            !JUNK.iter().any(|j| name.contains(j))
        })
        .min_by_key(|p| {
            let depth = p.strip_prefix(&root).map(|r| r.components().count()).unwrap_or(9);
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            (depth, u64::MAX - size)
        });
    let exe = best
        .and_then(|p| p.strip_prefix(&root).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    (root, exe)
}

/// Files Play can start.
fn find_exes(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else { return out };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                out.extend(find_exes(&path, depth - 1));
            }
        } else if path.extension().is_some_and(|e| ["exe", "bat", "cmd", "sh"].iter().any(|x| e.eq_ignore_ascii_case(x))) {
            out.push(path);
        }
    }
    out
}

fn is_exe(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

/// The script a game ships to be started with, at the top of its folder: Forge's
/// `RunMe.bat`, a repack's `Run Me.bat`, a `start.bat`. By name, so an installer
/// or a redistributable's script is never taken for it; `.sh` only where it runs.
fn start_script(root: &Path) -> Option<String> {
    let rank = |name: &str| -> Option<u8> {
        let path = Path::new(name);
        let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
        let by_ext = match ext.as_str() {
            "bat" | "cmd" => 0,
            "sh" if !cfg!(windows) => 1,
            _ => return None,
        };
        let stem: String = path.file_stem()?.to_string_lossy().chars().filter(char::is_ascii_alphanumeric).flat_map(|c| c.to_lowercase()).collect();
        match stem.as_str() {
            "runme" => Some(by_ext),
            "start" | "startgame" | "play" | "playgame" | "launch" | "launchgame" | "rungame" | "run" => Some(2 + by_ext),
            _ => None,
        }
    };
    let read = std::fs::read_dir(root).ok()?;
    read.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| Some((rank(&name)?, name)))
        .min()
        .map(|(_, name)| name)
}

/* ── commands ─────────────────────────────────────────────── */

#[tauri::command]
pub fn downloads_list(state: State<'_, Downloads>) -> Vec<Download> {
    state.list.lock().map(|l| l.clone()).unwrap_or_default()
}

#[tauri::command]
pub fn download_pause(app: AppHandle, state: State<'_, Downloads>, id: String) {
    if let Some(flag) = state.controls.lock().ok().and_then(|c| c.get(&id).cloned()) {
        flag.store(PAUSE, Ordering::SeqCst);
        stop_resolving(&app, &id);
        // A waiting one stops at once instead of at its next check.
        state.turn.notify_waiters();
    } else if edit(&app, &id, |_| {}).is_some_and(|d| matches!(d.status, Status::Queued | Status::Resolving | Status::Downloading)) {
        // Nothing running behind the row, so nothing would ever say it stopped.
        set_status(&app, &id, Status::Paused, None);
    }
}

/// A download stopped while its mirror's page was open: close the page.
fn stop_resolving(app: &AppHandle, id: &str) {
    if edit(app, id, |_| {}).is_some_and(|d| d.status == Status::Resolving) {
        crate::resolvers::cancel_browser(app);
    }
}

#[tauri::command]
pub fn download_resume(app: AppHandle, id: String) -> Result<(), String> {
    requeue(&app, &id)
}

/// Install an archive that failed its checksum, at the player's word: the
/// button beside Retry when the check failed. Logged, because a damaged
/// or replaced archive is what the check is there to catch.
#[tauri::command]
pub fn download_install_anyway(app: AppHandle, id: String) -> Result<(), String> {
    let d = edit(&app, &id, |_| {}).ok_or("That download is gone.")?;
    if d.status != Status::Failed || !d.hash_mismatch || !Path::new(&d.archive_path).is_file() {
        return Err("There is no unchecked archive to install. Retry to download it again.".into());
    }
    crate::logging::warn("download", &format!("{id}: installing {} without a matching checksum, at the player's word", d.file_name));
    edit(&app, &id, |d| {
        d.hash_mismatch = false;
        d.verified = true;
    });
    persist(&app);
    requeue(&app, &id)
}

/// Move a download in the queue: "up", "down", "top", or "now" (to the front,
/// and the one running steps aside: paused, then queued again right behind
/// it, as Steam does).
#[tauri::command]
pub fn download_move(app: AppHandle, state: State<'_, Downloads>, id: String, to: String) -> Result<(), String> {
    let active = state.active.lock().ok().and_then(|a| a.clone());
    let mut order = waiting_order(&app, active.as_deref());
    let Some(at) = order.iter().position(|x| x == &id) else {
        return Err("That download is not waiting.".into());
    };
    match to.as_str() {
        "up" if at > 0 => order.swap(at, at - 1),
        "down" if at + 1 < order.len() => order.swap(at, at + 1),
        "top" | "now" => {
            let item = order.remove(at);
            order.insert(0, item);
        }
        "up" | "down" => {}
        _ => return Err("Unknown move.".into()),
    }
    if to == "now" {
        if let Some(current) = active.as_ref().filter(|c| *c != &id) {
            order.insert(1, current.clone());
        }
    }
    renumber(&app, &order);
    persist(&app);
    emit(&app, true);
    if to == "now" {
        // A paused or failed one starts too.
        requeue(&app, &id)?;
        if let Some(current) = active.filter(|c| c != &id) {
            if let Some(flag) = state.controls.lock().ok().and_then(|c| c.get(&current).cloned()) {
                flag.store(PAUSE, Ordering::SeqCst);
                stop_resolving(&app, &current);
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                // Once it has stopped, back in the queue behind the new first.
                for _ in 0..300 {
                    let running = app.state::<Downloads>().controls.lock().map(|c| c.contains_key(&current)).unwrap_or(false);
                    if !running {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                if edit(&app, &current, |_| {}).is_some_and(|d| d.status == Status::Paused) {
                    let _ = requeue(&app, &current);
                }
            });
        }
    }
    state.turn.notify_waiters();
    Ok(())
}

#[tauri::command]
pub fn download_cancel(app: AppHandle, state: State<'_, Downloads>, id: String) {
    if let Some(flag) = state.controls.lock().ok().and_then(|c| c.get(&id).cloned()) {
        flag.store(CANCEL, Ordering::SeqCst);
        stop_resolving(&app, &id);
        state.turn.notify_waiters();
        // The row says so now. The transfer, the check or the unpack stops
        // within a moment and tidies up behind it (`settle_stopped`), but the
        // press is answered at once instead of a spinner until it has.
        set_status(&app, &id, Status::Canceled, None);
        return;
    }
    // Not running (paused or failed): tidy up here, the files in the background.
    let (archive, staging) = edit(&app, &id, |_| {})
        .filter(|d| d.status != Status::Installed)
        .map(|d| (Some(d.archive_path).filter(|p| !p.is_empty()), d.staging))
        .unwrap_or_default();
    edit(&app, &id, |d| {
        d.received = 0;
        d.staging = None;
    });
    set_status(&app, &id, Status::Canceled, None);
    clear_leftovers(archive, staging);
}

/// Clear a finished, failed or cancelled row.
#[tauri::command]
pub fn download_remove(app: AppHandle, state: State<'_, Downloads>, id: String) {
    if state.controls.lock().map(|c| c.contains_key(&id)).unwrap_or(false) {
        return;
    }
    if let Ok(mut list) = state.list.lock() {
        list.retain(|d| d.id != id);
    }
    persist(&app);
    emit(&app, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_where_only_with_two_folders_and_a_new_game() {
        let mut s = crate::settings::Settings { library_dir: "C:/Games".into(), ..Default::default() };
        assert!(!asks_for_folder(&s, false), "one folder: nothing to pick");
        s.library_folders = vec!["D:/Games".into()];
        assert!(asks_for_folder(&s, false));
        assert!(!asks_for_folder(&s, true), "an update goes where the game is");
        s.library_folders = vec!["C:/Games/".into()];
        assert!(!asks_for_folder(&s, false), "the same folder twice is one");
        s.library_folders = vec!["D:/Games".into()];
        s.ask_install_folder = false;
        assert!(!asks_for_folder(&s, false), "always the default");
    }

    /// A failed row from last week still says the build kryo.to served then.
    /// Update pressed today reuses that row: it must ask for today's build,
    /// not be thrown away as "already installed" on the strength of the old
    /// answer (the Update button that did nothing, 2026-10-29).
    #[test]
    fn a_reused_failed_row_forgets_the_old_builds_facts() {
        let installed = LibraryGame { slug: Some("hades-ii".into()), version: Some("1.0".into()), install_dir: "C:/g".into(), ..Default::default() };
        let mut stale = Download {
            slug: Some("hades-ii".into()),
            status: Status::Failed,
            meta: CatalogMeta { title: "Hades II".into(), version: Some("1.0".into()), size_bytes: Some(5), ..Default::default() },
            total: Some(5),
            ..Default::default()
        };
        // Before: the stale answer matched the library and the row was dropped.
        assert!(installed_build_in(std::slice::from_ref(&installed), &stale).is_some());
        take_request(&mut stale, None);
        assert_eq!(stale.meta.title, "Hades II", "the name is kept for the list");
        assert_eq!((stale.meta.version.as_deref(), stale.total, stale.release.as_deref()), (None, None, None));
        assert!(installed_build_in(std::slice::from_ref(&installed), &stale).is_none(), "nothing is known until kryo.to answers again");
        // kryo.to answers 1.1: an update, which goes ahead.
        stale.meta.version = Some("1.1".into());
        assert!(installed_build_in(std::slice::from_ref(&installed), &stale).is_none());
        // Asked for the very build installed (Reinstall of 1.0 under Builds): that is what is compared.
        take_request(&mut stale, Some("1.0".into()));
        assert!(installed_build_in(std::slice::from_ref(&installed), &stale).is_some());
        // A release picked wins over a stale current-build answer.
        stale.meta.version = Some("1.0".into());
        take_request(&mut stale, Some("0.9".into()));
        stale.meta.version = Some("1.0".into());
        assert!(installed_build_in(std::slice::from_ref(&installed), &stale).is_none());
        // A paused row with bytes on disk keeps its size for the resume's checks.
        let mut paused = Download { received: 10, archive_path: "C:/x.7z".into(), total: Some(99), meta: CatalogMeta { version: Some("1.0".into()), ..Default::default() }, ..Default::default() };
        take_request(&mut paused, None);
        assert_eq!((paused.total, paused.meta.version), (Some(99), None));
    }

    #[test]
    fn the_speed_shown_ignores_bursts_and_follows_a_real_change() {
        let start = Instant::now();
        let at = |ms: u64| start + std::time::Duration::from_millis(ms);
        let mut meter = SpeedMeter::new(start, 0);
        // A connection delivering in bursts, 10 MB/s on average.
        let (mut total, mut last) = (0u64, 0u64);
        for tick in 1..=40u64 {
            total += if tick % 2 == 0 { 8_000_000 } else { 0 };
            last = meter.add(at(tick * 400), total);
            if tick >= 10 {
                assert!((8_000_000..=12_000_000).contains(&last), "tick {tick}: {last}");
            }
        }
        assert!(last > 0);
        // It stops: the number falls to nothing within the window.
        for tick in 41..=70 {
            last = meter.add(at(tick * 400), total);
        }
        assert_eq!(last, 0);
        // Asked twice in the same moment, nothing divides by zero.
        let mut fresh = SpeedMeter::new(start, 0);
        assert_eq!(fresh.add(start, 100), 0);
    }

    #[test]
    fn archives_go_to_the_download_folder_when_there_is_one() {
        let mut s = crate::settings::Settings { library_dir: "/games".into(), ..Default::default() };
        assert_eq!(archive_dir(&s), Path::new("/games").join("_downloads"));
        s.download_dir = " /fast/downloads ".into();
        assert_eq!(archive_dir(&s), PathBuf::from("/fast/downloads"));
    }

    #[tokio::test]
    async fn a_server_that_says_nothing_does_not_hold_up_pause() {
        let flag = AtomicU8::new(PAUSE);
        let started = Instant::now();
        assert_eq!(unless_stopped(&flag, std::future::pending::<u8>()).await, None);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        let running = AtomicU8::new(RUN);
        assert_eq!(unless_stopped(&running, async { 5 }).await, Some(5));
    }

    #[test]
    fn the_speed_cap_cannot_be_saved_up() {
        let mb = 1024 * 1024;
        let t0 = Instant::now();
        let mut paid = t0;
        // A second's worth goes straight through, then it waits.
        assert!(limit_wait(&mut paid, t0, 10 * mb, 10 * mb).is_zero());
        assert_eq!(limit_wait(&mut paid, t0, 10 * mb, 10 * mb), std::time::Duration::from_secs(1));
        // A minute of nothing (a busy disk, a dropped connection) banks no
        // more than the one-second burst: the old cap let all sixty seconds
        // through at full speed.
        let later = t0 + std::time::Duration::from_secs(62);
        assert!(limit_wait(&mut paid, later, 10 * mb, 10 * mb).is_zero());
        assert_eq!(limit_wait(&mut paid, later, 10 * mb, 10 * mb), std::time::Duration::from_secs(1));
    }

    #[test]
    fn taskbar_follows_the_download_that_is_running() {
        let mut d = Download { status: Status::Downloading, total: Some(1000), received: 500, ..Default::default() };
        let queued = Download { status: Status::Queued, ..Default::default() };
        assert_eq!(taskbar_state(&[queued.clone(), d.clone()]), (1, 43));
        d.status = Status::Paused;
        assert_eq!(taskbar_state(&[d.clone()]), (3, 43));
        assert_eq!(taskbar_state(&[queued]), (2, 0));
        d.status = Status::Installed;
        assert_eq!(taskbar_state(&[d]), (0, 0));
    }

    #[test]
    fn ranges_cover_the_file_exactly() {
        let total = 2_458_559_531u64;
        let segs = split(total, 8);
        // Four ranges per connection, so a connection that finishes early has
        // more to take.
        assert_eq!(segs.len(), 32);
        assert_eq!(segs[0].start, 0);
        assert_eq!(segs.last().unwrap().end, total - 1);
        for w in segs.windows(2) {
            assert_eq!(w[0].end + 1, w[1].start);
        }
        assert_eq!(segs.iter().map(|s| s.len()).sum::<u64>(), total);
        // Small files are not split into slivers.
        assert_eq!(split(5 * 1024 * 1024, 8).len(), 1);
        assert!(split(0, 8).is_empty());
    }

    #[test]
    fn files_are_matched_to_their_hash_and_add_ons_are_recognised() {
        let json = serde_json::json!({
            "hashes": [{ "name": "Until Then - Kryoto.7z", "sha256": "8087783ba95268c527b59dba1a7a745bfbef5df90574aa05d5dd02742c6d3ec5" }],
            "addons": [{ "label": "Online add-on", "links": [{ "name": "Until Then - Online - Kryoto.7z" }], "hashes": [] }]
        });
        let m = match_file(&json, "Until Then - Kryoto.7z");
        assert_eq!(m.sha.as_deref(), Some("8087783ba95268c527b59dba1a7a745bfbef5df90574aa05d5dd02742c6d3ec5"));
        assert!(m.addon.is_none() && m.release.is_none());
        let m = match_file(&json, "Until Then - Online - Kryoto.7z");
        assert!(m.sha.is_none());
        assert_eq!(m.addon.as_deref(), Some("Online add-on"));
        assert_eq!(match_file(&json, "something else.7z"), FileMatch::default());
    }

    #[test]
    fn an_older_releases_file_is_recognised_as_that_release() {
        let json = serde_json::json!({
            "hashes": [{ "name": "Game - Kryoto.7z", "sha256": "a".repeat(64) }],
            "releases": [
                { "version": "2.0", "primary": true, "hashes": [{ "name": "Game - Kryoto.7z", "sha256": "a".repeat(64) }] },
                { "version": "1.4", "primary": false, "hashes": [{ "name": "Game 1.4 - Kryoto.7z", "sha256": "b".repeat(64) }] }
            ]
        });
        let m = match_file(&json, "Game 1.4 - Kryoto.7z");
        assert_eq!(m.sha, Some("b".repeat(64)));
        assert_eq!(m.release.as_deref(), Some("1.4"));
        assert_eq!(match_file(&json, "Game - Kryoto.7z").release, None);
    }

    /// Every build downloads under the same name; the size says which it is.
    #[test]
    fn an_older_build_under_the_same_name_gets_its_own_hash() {
        let json = serde_json::json!({
            "hashes": [{ "name": "Happy Wheels - Kryoto.7z", "sha256": "a".repeat(64), "bytes": 216135135u64 }],
            "releases": [
                { "version": "1.99.2", "primary": true, "links": [],
                  "hashes": [{ "name": "Happy Wheels - Kryoto.7z", "sha256": "a".repeat(64), "bytes": 216135135u64 }] },
                { "version": "1.99", "primary": false,
                  "links": [{ "host": "VikingFile", "url": "https://vikingfile.com/f/x" }],
                  "hashes": [{ "name": "Happy Wheels - Kryoto.7z", "sha256": "b".repeat(64), "bytes": 388552088u64 }] }
            ]
        });
        let file = "Happy Wheels - Kryoto.7z";
        // Our copy, through the Store: only the size to go on.
        let m = match_file_with(&json, file, &FileHint { size: Some(388552088), ..Default::default() });
        assert_eq!(m.sha, Some("b".repeat(64)));
        assert_eq!(m.release.as_deref(), Some("1.99"));
        // The current build is still the current build.
        let m = match_file_with(&json, file, &FileHint { size: Some(216135135), ..Default::default() });
        assert_eq!(m.sha, Some("a".repeat(64)));
        assert_eq!(m.release, None);
        // Picked under Versions, before the size is known.
        let m = match_file_with(&json, file, &FileHint { version: Some("1.99"), ..Default::default() });
        assert_eq!(m.sha, Some("b".repeat(64)));
        // From the old build's mirror, under whatever name the mirror gave it.
        let m = match_file_with(
            &json,
            "renamed.7z",
            &FileHint { size: Some(388552088), page: Some("https://vikingfile.com/f/x"), ..Default::default() },
        );
        assert_eq!(m.sha, Some("b".repeat(64)));
        // A size no build has is still checked, against the current one.
        let m = match_file_with(&json, file, &FileHint { size: Some(5), ..Default::default() });
        assert_eq!(m.sha, Some("a".repeat(64)));
    }

    /// A new build whose hash is not worked out yet, under the same name as
    /// the build before it: never checked against the old build's hash.
    #[test]
    fn a_new_build_without_a_hash_is_not_checked_against_the_old_one() {
        let json = serde_json::json!({
            "hashes": [],
            "releases": [
                { "version": "2022.3.62.28864 (b24705572)", "primary": true, "links": [], "hashes": [] },
                { "version": "V0.4.6f13", "primary": false, "links": [],
                  "hashes": [{ "name": "Schedule I - Kryoto.7z", "sha256": "b".repeat(64), "bytes": 1848167115u64 }] }
            ]
        });
        let file = "Schedule I - Kryoto.7z";
        // The new build's size: no hash, and not relabelled as the old build.
        let m = match_file_with(&json, file, &FileHint { size: Some(2075315145), ..Default::default() });
        assert_eq!(m, FileMatch::default());
        // No size yet: still no guess while the current build's hash is pending.
        assert_eq!(match_file(&json, file), FileMatch::default());
        // The old build itself (its size, or picked under Versions) keeps its hash.
        let m = match_file_with(&json, file, &FileHint { size: Some(1848167115), ..Default::default() });
        assert_eq!(m.sha, Some("b".repeat(64)));
        assert_eq!(m.release.as_deref(), Some("V0.4.6f13"));
        let m = match_file_with(&json, file, &FileHint { version: Some("V0.4.6f13"), ..Default::default() });
        assert_eq!(m.sha, Some("b".repeat(64)));
    }

    #[test]
    fn filenames_come_out_of_content_disposition() {
        assert_eq!(
            filename_from_disposition("attachment; filename*=UTF-8''Captain%20Hardcore%20-%20Kryoto.7z").as_deref(),
            Some("Captain Hardcore - Kryoto.7z")
        );
        assert_eq!(
            filename_from_disposition(r#"attachment; filename="Game - Kryoto.7z""#).as_deref(),
            Some("Game - Kryoto.7z")
        );
        assert_eq!(filename_from_disposition("inline"), None);
    }

    #[test]
    fn an_update_merges_over_the_old_install_and_keeps_saves() {
        let d = scratch("merge");
        let old = d.join("Game");
        std::fs::create_dir_all(old.join("Data")).unwrap();
        std::fs::write(old.join("Game.exe"), b"old").unwrap();
        std::fs::write(old.join("save.dat"), b"mine").unwrap();
        let new = d.join("staging/Game");
        std::fs::create_dir_all(new.join("Data")).unwrap();
        std::fs::write(new.join("Game.exe"), b"new").unwrap();
        std::fs::write(new.join("Data/level.bin"), b"L").unwrap();
        let top = single_child_dir(&d.join("staging")).unwrap();
        move_tree(&top, &old).unwrap();
        assert_eq!(std::fs::read(old.join("Game.exe")).unwrap(), b"new");
        assert_eq!(std::fs::read(old.join("save.dat")).unwrap(), b"mine", "saves stay");
        assert!(old.join("Data/level.bin").is_file());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn titles_come_from_release_file_names() {
        assert_eq!(title_from_file("Captain Hardcore - Kryoto.7z"), "Captain Hardcore");
        assert_eq!(title_from_file("Portal 2.zip"), "Portal 2");
        assert_eq!(title_from_file(".7z"), "Game");
    }

    /// A download with no name from anywhere else was called "download" - the
    /// last piece of an address is not a game.
    #[test]
    fn generic_file_names_name_nothing() {
        for name in ["download", "Download.7z", "file", "d", "eyJrIjoiYWJjIiwiZXhwIjoxfQ.c2lnbmF0dXJlc2lnbmF0dXJl"] {
            assert_eq!(title_in_file_name(name), None, "{name}");
        }
        assert_eq!(title_in_file_name("Hades II - Kryoto.7z").as_deref(), Some("Hades II"));
        assert_eq!(title_in_file_name("Portal 2.5 Remix.7z").as_deref(), Some("Portal 2.5 Remix"));
        assert!(is_placeholder("download", None));
        assert!(is_placeholder("hades-ii", Some("hades-ii")));
        assert!(!is_placeholder("Hades II", Some("hades-ii")));
    }

    /// Every DLC of a game goes in while the others are still going, and none
    /// of them counts as the game. The game asked for twice is still refused.
    #[test]
    fn a_games_dlc_are_requests_of_their_own() {
        let link = |body: &str| format!("https://dl.kryo.to/d/{body}.c2ln");
        let firebird = link("eyJrIjoiYS9iIiwiZXhwIjoxLCJuIjoiRm9ydHMgRExDIC0gRmlyZWJpcmQgLSBLcnlvdG8uN3oifQ");
        let high_seas = link("eyJrIjoiYS9iIiwiZXhwIjoxLCJuIjoiRm9ydHMgRExDIC0gSGlnaCBTZWFzIC0gS3J5b3RvLjd6In0");
        let game = link("eyJrIjoiYS9iIiwiZXhwIjoxLCJuIjoiRm9ydHMgLSBLcnlvdG8uN3oifQ");
        let row = |url: &str| Download { url: url.into(), ..Default::default() };
        assert!(is_dlc_link(&firebird) && !is_dlc_link(&game));
        assert!(!same_request(&row(&firebird), &high_seas, false));
        assert!(!same_request(&row(&game), &firebird, false) && !same_request(&row(&firebird), &game, false));
        assert!(same_request(&row(&firebird), &firebird, false));
        assert!(same_request(&row(&game), &game, false));
    }

    /// A mirror page names no file: only the same page is the same press here.
    #[test]
    fn a_mirror_is_only_the_same_request_on_the_same_page() {
        let going = Download {
            url: "https://cdn.example/file".into(),
            mirror: Some(Mirror { page: "https://vikingfile.com/f/abc".into(), host: "VikingFile".into() }),
            ..Default::default()
        };
        assert!(same_request(&going, "https://vikingfile.com/f/abc", true));
        assert!(!same_request(&going, "https://vikingfile.com/f/def", true));
    }

    /// kryo.to's own answer, as `/api/games/<slug>/downloads` gives it.
    #[test]
    fn a_file_or_mirror_is_placed_by_kryo_to() {
        let json = serde_json::json!({
            "links": [{ "url": "https://vikingfile.com/f/game", "host": "VikingFile" }],
            "addons": [{
                "label": "DLC: Firebird",
                "links": [{ "name": "Forts DLC - Firebird - Kryoto.7z" }],
                "pages": ["https://vikingfile.com/f/dlc"],
                "hashes": [{ "name": "Forts DLC - Firebird - Kryoto.7z", "sha256": "ab" }],
            }],
            "releases": [{ "primary": true, "links": [{ "url": "https://mocha.my/share/game", "name": "Forts - Kryoto.7z" }] }],
        });
        let addon = || FileKind::Addon("DLC: Firebird".into());
        assert_eq!(file_kind(&json, None, Some("https://vikingfile.com/f/dlc")), addon());
        assert_eq!(file_kind(&json, Some("forts dlc - firebird - kryoto.7z"), None), addon());
        assert_eq!(file_kind(&json, None, Some("https://vikingfile.com/f/game")), FileKind::Game);
        assert_eq!(file_kind(&json, None, Some("https://mocha.my/share/game")), FileKind::Game);
        assert_eq!(file_kind(&json, Some("Forts - Kryoto.7z"), None), FileKind::Game);
        assert_eq!(file_kind(&json, None, Some("https://elsewhere.example/x")), FileKind::Unknown);
        assert_eq!(file_kind(&serde_json::json!({}), None, Some("x")), FileKind::Unknown);
    }

    /// A DLC out of kryo.to's reach is still an add-on, by its file's name.
    #[test]
    fn a_dlc_file_names_its_label() {
        assert_eq!(dlc_label("Forts DLC - Firebird - Kryoto.7z").as_deref(), Some("DLC: Firebird"));
        assert_eq!(dlc_label("Forts - Kryoto.7z"), None);
    }

    /// Fetched from a mirror that renamed it, an add-on is still found by the page.
    #[test]
    fn a_renamed_add_on_is_found_by_its_mirror_page() {
        let sha = "ab".repeat(32);
        let json = serde_json::json!({ "addons": [{ "label": "DLC: Firebird", "pages": ["https://vikingfile.com/f/dlc"], "hashes": [{ "name": "Forts DLC - Firebird - Kryoto.7z", "sha256": sha }] }] });
        let hint = FileHint { size: None, version: None, page: Some("https://vikingfile.com/f/dlc") };
        let found = match_file_with(&json, "random_upload.7z", &hint);
        assert_eq!(found.addon.as_deref(), Some("DLC: Firebird"));
        assert_eq!(found.sha.as_deref(), Some(sha.as_str()));
    }

    /// dl.kryo.to's link carries the name the file saves as.
    #[test]
    fn the_link_names_its_file() {
        // {"k":"a/b","exp":1,"n":"Hades II - Kryoto.7z"}, as lib/dl-token.ts mints it.
        let body = "eyJrIjoiYS9iIiwiZXhwIjoxLCJuIjoiSGFkZXMgSUkgLSBLcnlvdG8uN3oifQ";
        let url = format!("https://dl.kryo.to/d/{body}.c2ln");
        assert_eq!(file_name_in_link(&url).as_deref(), Some("Hades II - Kryoto.7z"));
        assert_eq!(file_name_in_link("https://dl.kryo.to/x/abc"), None);
        assert_eq!(file_name_in_link("https://dl.kryo.to/d/%%%"), None);
        assert_eq!(base64url("aGk").as_deref(), Some(&b"hi"[..]));
    }

    /// The family the link is bound to, so the download connects over it.
    #[test]
    fn the_link_says_which_network_it_is_for() {
        // A real 0.2.3 report's link: bound to 113.203.49.0/24.
        let v4 = "https://dl.kryo.to/d/eyJrIjoiMnQvMnRWNFJOM2FmNkxNME43eS43eiIsImV4cCI6MTc5MDgxNzQ5NywibiI6IkJhbGR1cidzIEdhdGUgMy43eiIsImlwIjoiMTEzLjIwMy40OS4wLzI0In0.sig";
        assert_eq!(link_family(v4), Some(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)));
        // And one bound to 2a04:4a43:953f:fc03::/64.
        let v6 = "https://dl.kryo.to/d/eyJrIjoiamovSkprdWY4VmlXejkwTzFaRS43eiIsImV4cCI6MTc5MDc4NzE1NywibiI6IkRBUksgU09VTFMgSUkgLSBLcnlvdG8uN3oiLCJpcCI6IjJhMDQ6NGE0Mzo5NTNmOmZjMDM6Oi82NCJ9.sig";
        assert_eq!(link_family(v6), Some(std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)));
        // No binding (a mirror or preview link): nothing to match.
        let none = "https://dl.kryo.to/d/eyJrIjoiYS9iIiwiZXhwIjoxLCJuIjoiSGFkZXMgSUkgLSBLcnlvdG8uN3oifQ.c2ln";
        assert_eq!(link_family(none), None);
    }

    #[test]
    fn names_are_safe_folder_names() {
        assert_eq!(safe_name("Half-Life: Alyx?"), "Half-Life Alyx");
        assert_eq!(safe_name("..\\..\\x"), "x");
        assert_eq!(safe_name("   "), "Game");
    }

    #[test]
    fn a_crawling_download_gets_new_connections_and_a_slow_line_does_not() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + std::time::Duration::from_secs(s);
        let mb = 1024 * 1024;
        let left = 2_000 * mb;
        // Fast, then an eighth of that and less for 40 s: time for new ones.
        let mut w = SlowWatch::new(t0);
        assert!(!w.note(at(5), 20 * mb, left), "warming up");
        assert!(!w.note(at(25), 20 * mb, left));
        assert!(!w.note(at(30), mb, left));
        assert!(!w.note(at(60), mb, left));
        assert!(w.note(at(71), mb, left));
        // A dip that recovers starts the clock again.
        let mut w = SlowWatch::new(t0);
        w.note(at(25), 20 * mb, left);
        w.note(at(30), mb, left);
        w.note(at(50), 10 * mb, left);
        assert!(!w.note(at(75), mb, left));
        // A line that was never fast, and a download nearly done, are left alone.
        let mut w = SlowWatch::new(t0);
        w.note(at(25), 600 * 1024, left);
        assert!(!w.note(at(100), 50 * 1024, left));
        let mut w = SlowWatch::new(t0);
        w.note(at(25), 20 * mb, left);
        w.note(at(30), mb, 10 * mb);
        assert!(!w.note(at(100), mb, 10 * mb));
    }

    #[test]
    fn a_replaced_upload_of_the_same_build_is_an_update() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let json = serde_json::json!({
            "hashes": [{ "name": "Game - Kryoto.7z", "sha256": b }],
            "releases": [
                { "version": "1.0", "primary": true, "hashes": [{ "name": "Game - Kryoto.7z", "sha256": b }] },
                { "version": "0.9", "primary": false, "hashes": [{ "name": "Game - Kryoto.7z", "sha256": a }] }
            ]
        });
        assert!(replaced(&json, Some(&a), None), "the current file is another one");
        assert!(!replaced(&json, Some(&b), None), "the same file");
        assert!(!replaced(&json, Some(&a), Some("0.9")), "the older build asked for is that file");
        assert!(!replaced(&json, None, None), "an install from before hashes were kept");
        let none = serde_json::json!({ "releases": [{ "version": "", "primary": true, "hashes": [] }] });
        assert!(!replaced(&none, Some(&a), None), "no hash to compare with");
        let games = vec![LibraryGame { slug: Some("g".into()), version: Some("1.0".into()), build_sha: Some(a.clone()), ..Default::default() }];
        let mut d = Download { slug: Some("g".into()), meta: CatalogMeta { version: Some("1.0".into()), ..Default::default() }, ..Default::default() };
        assert!(installed_build_in(&games, &d).is_some());
        d.sha256 = Some(b.clone());
        assert!(installed_build_in(&games, &d).is_none(), "fetched, and it is another file");
    }

    #[test]
    fn new_install_folders_are_plain_letters() {
        assert_eq!(plain_name("Baldur’s Gate 3", None), "Baldur's Gate 3");
        assert_eq!(plain_name("Ōkami HD", None), "Okami HD");
        assert_eq!(plain_name("Pokémon™ Legends – Z-A", None), "Pokemon Legends - Z-A");
        assert_eq!(plain_name("Mörkö: Straße… 2", None), "Morko Strasse... 2");
        assert_eq!(plain_name("ドラゴンクエスト", Some("dragon-quest")), "dragon-quest");
        assert_eq!(plain_name("ドラゴンクエスト", None), "Game");
        assert_eq!(plain_name("Half-Life: Alyx?", None), "Half-Life Alyx");
    }

    #[test]
    fn only_kryoto_downloads_are_taken_over() {
        let settings = crate::settings::Settings::default();
        assert!(is_ours(&"https://dl.kryo.to/d/abc".parse().unwrap(), &settings));
        assert!(is_ours(&"https://kryo.to/api/download/v/1".parse().unwrap(), &settings));
        assert!(!is_ours(&"https://evil-kryo.to/x".parse().unwrap(), &settings));
        assert!(!is_ours(&"http://dl.kryo.to/d/abc".parse().unwrap(), &settings));
        assert!(!is_ours(&"https://vikingfile.com/f/x".parse().unwrap(), &settings));
        let local = crate::settings::Settings { catalog_endpoint: "http://localhost:3000".into(), ..Default::default() };
        assert!(is_ours(&"http://localhost:3000/api/download/v/1".parse().unwrap(), &local));
        assert!(!is_ours(&"http://localhost:3001/api/download/v/1".parse().unwrap(), &local));
    }

    #[test]
    fn archive_entries_cannot_escape() {
        let d = Path::new("/lib/Game");
        assert!(safe_join(d, "../../evil.dll").is_none());
        assert!(safe_join(d, "bin/ok.dll").is_some());
    }

    #[test]
    fn meta_reads_the_game_api() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"title":"Captain Hardcore","steam_appid":"1190600","cover_vertical":"https://x/v.jpg",
               "game_executable_path":"Captain Hardcore.exe","game_executable_args":null,"source":"Steam (DRM-free)",
               "version":"b123","download_size_bytes":1234,
               "game_launch_options":{"available":[
                 {"type":"vr","oslist":"windows","arguments":"","executable":"Captain Hardcore.exe","workingdir":"","description":""},
                 {"type":"default","oslist":"windows","arguments":"-nohmd","executable":"Captain Hardcore.exe","workingdir":"","description":"Captain Hardcore Desktop Mode"},
                 {"oslist":"macos","arguments":"","executable":"x.app","workingdir":"","description":""}]}}"#,
        )
        .unwrap();
        let m = meta_from_json(&json);
        assert_eq!(m.entries.len(), 2, "the macOS entry is dropped");
        assert_eq!(m.entries[1].arguments, "-nohmd");
        assert_eq!(m.entries[0].kind, "vr");
        assert_eq!(m.size_bytes, Some(1234));
        assert!(m.hero.unwrap().contains("1190600/library_hero"));
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryoto-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_exe_decides_the_root_whether_or_not_there_is_a_top_folder() {
        let d = scratch("locate");
        std::fs::create_dir_all(d.join("Captain Hardcore/bin")).unwrap();
        std::fs::write(d.join("Captain Hardcore/Captain Hardcore.exe"), b"x").unwrap();
        std::fs::write(d.join("Captain Hardcore/UnityCrashHandler64.exe"), b"xxxxxxxx").unwrap();
        let (root, exe) = locate(&d, "Captain Hardcore.exe");
        assert_eq!(root, d.join("Captain Hardcore"));
        assert_eq!(exe, "Captain Hardcore.exe");
        // Nothing named: the non-junk exe at the top of the single folder.
        let (root, exe) = locate(&d, "");
        assert_eq!(root, d.join("Captain Hardcore"));
        assert_eq!(exe, "Captain Hardcore.exe");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_game_that_ships_a_start_script_starts_from_it() {
        let d = scratch("runme");
        std::fs::create_dir_all(d.join("Game/bin")).unwrap();
        std::fs::write(d.join("Game/Biggest Tool.exe"), vec![0u8; 4096]).unwrap();
        std::fs::write(d.join("Game/bin/game.exe"), b"x").unwrap();
        std::fs::write(d.join("Game/Install.bat"), b"x").unwrap();
        // No start script: the biggest exe, and an installer's script is not one.
        assert_eq!(locate(&d, "").1, "Biggest Tool.exe");
        std::fs::write(d.join("Game/Run Me.bat"), b"x").unwrap();
        std::fs::write(d.join("Game/start.bat"), b"x").unwrap();
        let (root, exe) = locate(&d, "");
        assert_eq!(root, d.join("Game"));
        assert_eq!(exe, "Run Me.bat");
        // What the release names always wins, a script or an exe.
        assert_eq!(locate(&d, "bin/game.exe").1, "bin/game.exe");
        assert_eq!(locate(&d, "start.bat").1, "start.bat");
        let _ = std::fs::remove_dir_all(d);
    }

    /// A real 7-Zip archive, made by 7-Zip the way Forge makes them (`-mx1
    /// -mmt`), unpacked by `extract`. Skipped where 7-Zip is not installed.
    #[cfg(windows)]
    #[test]
    fn a_forge_style_archive_unpacks() {
        let seven = Path::new(r"C:\Program Files\7-Zip\7z.exe");
        if !seven.exists() {
            return;
        }
        let d = scratch("extract");
        let src = d.join("src/Captain Hardcore");
        std::fs::create_dir_all(src.join("Data")).unwrap();
        std::fs::copy(r"C:\Windows\System32\cmd.exe", src.join("Captain Hardcore.exe")).unwrap();
        std::fs::write(src.join("Data/level.bin"), vec![7u8; 300_000]).unwrap();
        let archive = d.join("Captain Hardcore - Kryoto.7z");
        let status = std::process::Command::new(seven)
            .arg("a")
            .arg(&archive)
            .arg(d.join("src").join("*"))
            .args(["-mx1", "-mmt", "-y"])
            .output()
            .unwrap();
        assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stdout));
        let out = d.join("out");
        let seen = std::cell::Cell::new(0u64);
        extract(&archive, &out, &|done, _| seen.set(done)).unwrap();
        let (root, exe) = locate(&out, "Captain Hardcore.exe");
        assert!(root.join(&exe).is_file());
        assert_eq!(std::fs::read(root.join("Data/level.bin")).unwrap().len(), 300_000);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn checksum_errors_alone_are_a_warning() {
        let tar = "UntilThen.pck: 7-Zip bad CRC 0xe05db56a should be 0x1fbc7295: Unknown error\ntar.exe: Error exit delayed from previous errors.";
        assert_eq!(damaged_files(tar), Some(vec!["UntilThen.pck".to_string()]));
        let seven = "ERROR: CRC Failed : data/UntilThen.pck\nSub items Errors: 1\nArchives with Errors: 1";
        assert_eq!(damaged_files(seven), Some(vec!["data/UntilThen.pck".to_string()]));
        // Anything else is a real failure.
        assert_eq!(damaged_files("Game.pck: Write failed: No space left on device\ntar.exe: Error exit delayed from previous errors."), None);
        assert_eq!(damaged_files("tar.exe: Error opening archive: Unrecognized archive format"), None);
        assert!(damage_warning(&["a.pck".into(), "b.pck".into()], true).unwrap().contains("a.pck and 1 more"));
    }

    #[test]
    fn seven_zip_progress_lines_are_read() {
        assert_eq!(seven_zip_percent("  42% 118 - Game\\data.pak"), Some(42));
        assert_eq!(seven_zip_percent("100%"), Some(100));
        assert_eq!(seven_zip_percent("  7%"), Some(7));
        assert_eq!(seven_zip_percent("Everything is Ok"), None);
        assert_eq!(seven_zip_percent("120%"), None);
        assert_eq!(seven_zip_percent(""), None);
    }

    /// Packs a small game with the system's 7-Zip and unpacks it through
    /// `extract`, which hands it to that same 7-Zip. Skipped where there is none.
    #[test]
    fn seven_zip_unpacks_with_progress() {
        let dir = scratch("7zip");
        let src = dir.join("src").join("Game");
        std::fs::create_dir_all(src.join("bin")).unwrap();
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        std::fs::write(src.join("bin").join("Game.exe"), &data).unwrap();
        std::fs::write(src.join("readme.txt"), b"hi").unwrap();
        let archive = dir.join("Game - Kryoto.7z");
        let Some(tool) = seven_zip_tools(true).into_iter().find(|t| {
            std::process::Command::new(t)
                .current_dir(dir.join("src"))
                .arg("a")
                .arg(&archive)
                .arg("Game")
                .output()
                .is_ok_and(|o| o.status.success())
        }) else {
            eprintln!("no 7-Zip here; skipped");
            return;
        };
        let out = dir.join("out");
        let seen = std::cell::RefCell::new(Vec::new());
        let damaged = extract_with_7zip(&tool, &archive, &out, &|done, total| seen.borrow_mut().push((done, total)), &AtomicU8::new(RUN)).unwrap().unwrap();
        assert!(damaged.is_empty());
        assert_eq!(std::fs::read(out.join("Game").join("bin").join("Game.exe")).unwrap(), data);
        let total = data.len() as u64 + 2;
        assert_eq!(seen.borrow().last(), Some(&(total, Some(total))));

        // A byte flipped in the packed data: named as damaged, not a failure.
        let mut bytes = std::fs::read(&archive).unwrap();
        let mid = bytes.len() / 3;
        bytes[mid] ^= 0xff;
        let broken = dir.join("broken.7z");
        std::fs::write(&broken, bytes).unwrap();
        let out = dir.join("out2");
        match extract_with_7zip(&tool, &broken, &out, &|_, _| {}, &AtomicU8::new(RUN)).unwrap() {
            Ok(damaged) => assert!(!damaged.is_empty()),
            Err(e) => assert!(e.contains("7-Zip"), "{e}"),
        }
    }

    #[test]
    fn content_ranges_are_read() {
        assert_eq!(
            parse_content_range("bytes 100-199/1000"),
            Some(ContentRange { start: Some(100), end: Some(199), total: Some(1000) })
        );
        assert_eq!(parse_content_range("bytes */1000"), Some(ContentRange { start: None, end: None, total: Some(1000) }));
        assert_eq!(parse_content_range("bytes 0-0/*"), Some(ContentRange { start: Some(0), end: Some(0), total: None }));
        assert_eq!(parse_content_range("items 1-2/3"), None);
        assert_eq!(parse_content_range("bytes x-2/3"), None);
    }

    #[test]
    fn only_a_difference_says_the_file_changed() {
        assert!(same_file(Some("a-3"), Some(10), Some("a-3"), Some(10)));
        assert!(same_file(Some("a-3"), Some(10), None, None));
        assert!(same_file(None, None, Some("b"), Some(11)));
        assert!(!same_file(Some("a-3"), Some(10), Some("b-3"), Some(10)));
        assert!(!same_file(Some("a-3"), Some(10), Some("a-3"), Some(11)));
    }

    #[test]
    fn retries_back_off_to_a_minute() {
        assert_eq!((1..=7).map(backoff).collect::<Vec<_>>(), vec![2, 4, 8, 16, 30, 60, 60]);
    }

    /// A one-file HTTP server whose answer to `bytes=a-z` is up to `answer`.
    fn serve(rt: &tokio::runtime::Runtime, answer: fn(u64, u64) -> (String, Vec<u8>)) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        rt.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                loop {
                    let Ok((mut sock, _)) = listener.accept().await else { return };
                    tokio::spawn(async move {
                        let mut req = Vec::new();
                        let mut b = [0u8; 1024];
                        while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                            let Ok(n) = sock.read(&mut b).await else { return };
                            if n == 0 {
                                return;
                            }
                            req.extend_from_slice(&b[..n]);
                        }
                        let text = String::from_utf8_lossy(&req).to_string();
                        let range = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(String::from)).unwrap();
                        let (a, z) = range.trim().split_once('-').unwrap();
                        let (head, body) = answer(a.parse().unwrap(), z.parse().unwrap());
                        let _ = sock.write_all(format!("{head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await;
                        let _ = sock.write_all(&body).await;
                    });
                }
            });
            format!("http://{addr}/file")
        })
    }

    /// The test file: every byte is its own offset, so a byte in the wrong
    /// place shows.
    fn byte_at(i: u64) -> u8 {
        (i % 251) as u8
    }

    fn fetch_one(rt: &tokio::runtime::Runtime, url: &str, name: &str, seg: Segment, total: u64) -> (Result<(), Transfer>, Vec<u8>) {
        let path = scratch(name).join("file.bin");
        std::fs::File::create(&path).unwrap().set_len(total).unwrap();
        let (progress, arrived) = (vec![AtomicU64::new(0)], vec![AtomicU64::new(0)]);
        let (stop, limiter) = (AtomicU8::new(RUN), Limiter::new(0));
        let expect = Expected { etag: Some("v1".into()), total: Some(total) };
        let client = reqwest::Client::new();
        let res = rt.block_on(fetch_range(&client, url, &path, 0, seg, &progress, &arrived, &stop, &limiter, &expect));
        (res, std::fs::read(&path).unwrap())
    }

    /// The speed reads `arrived`, which moves with every chunk; `progress` waits for
    /// the 8 MB flush, so a slow line would show nothing for seconds at a time.
    #[test]
    fn bytes_count_for_the_speed_as_they_arrive_and_for_resume_once_flushed() {
        use tokio::io::AsyncReadExt;
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let (url, release) = rt.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (release, wait) = tokio::sync::oneshot::channel::<()>();
            tokio::spawn(async move {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut req = [0u8; 2048];
                let _ = sock.read(&mut req).await.unwrap();
                sock.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-999/1000\r\nETag: \"v1\"\r\nContent-Length: 1000\r\n\r\n").await.unwrap();
                sock.write_all(&[7u8; 400]).await.unwrap();
                sock.flush().await.unwrap();
                let _ = wait.await;
                let _ = sock.write_all(&[7u8; 600]).await;
            });
            (format!("http://{addr}/file"), release)
        });
        let path = scratch("arrived").join("file.bin");
        std::fs::File::create(&path).unwrap().set_len(1000).unwrap();
        let (progress, arrived) = (Arc::new(vec![AtomicU64::new(0)]), Arc::new(vec![AtomicU64::new(0)]));
        let task = {
            let (path, progress, arrived, url) = (path.clone(), progress.clone(), arrived.clone(), url.clone());
            rt.spawn(async move {
                let (stop, limiter) = (AtomicU8::new(RUN), Limiter::new(0));
                let expect = Expected { etag: Some("v1".into()), total: Some(1000) };
                fetch_range(&reqwest::Client::new(), &url, &path, 0, Segment { start: 0, end: 999, done: 0 }, &progress, &arrived, &stop, &limiter, &expect).await.is_ok()
            })
        };
        let started = Instant::now();
        while arrived[0].load(Ordering::SeqCst) < 400 {
            assert!(started.elapsed() < std::time::Duration::from_secs(10), "the first bytes never counted");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Come in, but not on the disk yet: a resume must not count them.
        assert_eq!(progress[0].load(Ordering::SeqCst), 0);
        release.send(()).unwrap();
        assert!(rt.block_on(task).unwrap());
        assert_eq!((arrived[0].load(Ordering::SeqCst), progress[0].load(Ordering::SeqCst)), (1000, 1000));
    }

    #[test]
    fn a_range_lands_at_its_own_offset() {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let url = serve(&rt, |a, z| {
            (format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{z}/1000\r\nETag: \"v1\""), (a..=z).map(byte_at).collect())
        });
        let (res, file) = fetch_one(&rt, &url, "range-ok", Segment { start: 300, end: 599, done: 0 }, 1000);
        assert!(res.is_ok());
        assert!((300..600).all(|i| file[i] == byte_at(i as u64)));
        assert!(file[..300].iter().all(|b| *b == 0) && file[600..].iter().all(|b| *b == 0));
    }

    #[test]
    fn a_range_from_the_wrong_place_or_file_starts_over() {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        // Answers every range from the start of the file.
        let shifted = serve(&rt, |a, z| {
            (format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/1000\r\nETag: \"v1\"", z - a), (0..=z - a).map(byte_at).collect())
        });
        let (res, file) = fetch_one(&rt, &shifted, "range-shifted", Segment { start: 300, end: 599, done: 0 }, 1000);
        assert!(matches!(res, Err(Transfer::Restart(_))));
        assert!(file.iter().all(|b| *b == 0), "nothing written");

        let replaced = serve(&rt, |a, z| {
            (format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{z}/1000\r\nETag: \"v2\""), (a..=z).map(byte_at).collect())
        });
        let (res, _) = fetch_one(&rt, &replaced, "range-replaced", Segment { start: 300, end: 599, done: 0 }, 1000);
        assert!(matches!(res, Err(Transfer::Restart(_))));

        let whole = serve(&rt, |_, _| ("HTTP/1.1 200 OK".into(), (0..1000).map(byte_at).collect()));
        let (res, _) = fetch_one(&rt, &whole, "range-whole", Segment { start: 300, end: 599, done: 0 }, 1000);
        assert!(matches!(res, Err(Transfer::NoRanges)));
    }

    /// Throughput of the range fetcher against a local server that sends as
    /// fast as it can: what the client itself costs, with no network in the
    /// way. `cargo test --release -- --ignored range_throughput --nocapture`
    #[test]
    #[ignore]
    fn range_throughput() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            const TOTAL: u64 = 1024 * 1024 * 1024;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                loop {
                    let Ok((mut sock, _)) = listener.accept().await else { return };
                    tokio::spawn(async move {
                        loop {
                            let mut req = Vec::new();
                            let mut b = [0u8; 1024];
                            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                                let Ok(n) = sock.read(&mut b).await else { return };
                                if n == 0 {
                                    return;
                                }
                                req.extend_from_slice(&b[..n]);
                            }
                            let text = String::from_utf8_lossy(&req).to_string();
                            let range = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(String::from)).unwrap();
                            let (a, z) = range.trim().split_once('-').unwrap();
                            let (a, z): (u64, u64) = (a.parse().unwrap(), z.parse().unwrap());
                            let len = z - a + 1;
                            let head = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {len}\r\nContent-Range: bytes {a}-{z}/{TOTAL}\r\n\r\n");
                            if sock.write_all(head.as_bytes()).await.is_err() {
                                return;
                            }
                            let block = vec![7u8; 256 * 1024];
                            let mut left = len;
                            while left > 0 {
                                let n = left.min(block.len() as u64) as usize;
                                if sock.write_all(&block[..n]).await.is_err() {
                                    return;
                                }
                                left -= n as u64;
                            }
                        }
                    });
                }
            });
            let path = scratch("bench").join("file.bin");
            std::fs::File::create(&path).unwrap().set_len(TOTAL).unwrap();
            let client = reqwest::Client::builder().tcp_nodelay(true).build().unwrap();
            let url = format!("http://{addr}/file");
            for conns in [8u32, 16] {
                let segs = Arc::new(split(TOTAL, conns));
                let progress: Arc<Vec<AtomicU64>> = Arc::new(segs.iter().map(|_| AtomicU64::new(0)).collect());
                let arrived: Arc<Vec<AtomicU64>> = Arc::new(segs.iter().map(|_| AtomicU64::new(0)).collect());
                let (stop, limiter, next) = (Arc::new(AtomicU8::new(RUN)), Arc::new(Limiter::new(0)), Arc::new(std::sync::atomic::AtomicUsize::new(0)));
                let t = Instant::now();
                let mut tasks = Vec::new();
                for _ in 0..conns {
                    let (client, url, path, progress, arrived, stop, limiter, segs, next) =
                        (client.clone(), url.clone(), path.clone(), progress.clone(), arrived.clone(), stop.clone(), limiter.clone(), segs.clone(), next.clone());
                    tasks.push(tokio::spawn(async move {
                        loop {
                            let i = next.fetch_add(1, Ordering::SeqCst);
                            let Some(seg) = segs.get(i) else { break };
                            let expect = Expected { etag: None, total: Some(TOTAL) };
                            assert!(fetch_range(&client, &url, &path, i, seg.clone(), &progress, &arrived, &stop, &limiter, &expect).await.is_ok());
                        }
                    }));
                }
                for t in tasks {
                    t.await.unwrap();
                }
                let secs = t.elapsed().as_secs_f64();
                let got: u64 = progress.iter().map(|p| p.load(Ordering::SeqCst)).sum();
                assert_eq!(got, TOTAL);
                println!("{conns} connections: {:.0} MB/s", TOTAL as f64 / 1048576.0 / secs);
            }
        });
    }
}
