//! Whether Kryoto is answering, for the shell to say so calmly.
//!
//! One failed request means nothing: this reads the site's `/api/health` and
//! status.kryo.to's feed on a timer, and only calls it an outage after several
//! failures over a grace period. Then:
//!
//! - **down**: the site is not answering here and the status feed agrees.
//! - **unreachable**: the site is not answering here, and the feed is dead too
//!   or still says the site is fine. Either way this PC cannot tell it is
//!   Kryoto: the player's connection may be the reason.
//! - **partial**: the site answers, but something on it is not right.
//!
//! When this app's own request fails, the Store page is asked to fetch
//! `/api/health` too (`ask_page`), and its answer counts: on a PC whose
//! antivirus or proxy lets the web view through and not this app's own
//! requests, kryo.to is answering, and the Store it shows is never hidden
//! behind "Kryoto is not answering" (KB-0323).
//!
//! The site's `/api/health` answer is cached for a few seconds there, so this
//! costs it very little (kryoto-bugs KB-0002 is what polling can do): every 30
//! seconds while the window is up, every 3 minutes while it is hidden, and
//! faster only while something looks wrong.
//!
//! The same patience goes for "this network blocks kryo.to": one request of
//! this app's that got no answer says nothing. Desktop 0.3.10 called it on the
//! first one, and a kept connection that a router or antivirus had quietly
//! dropped was enough: 222 Windows PCs and 21 Linux ones were told their
//! network blocks kryo.to in its first two days, many of them every few
//! minutes while the Store beside it worked (KB-0328). So each look is made on
//! a connection of its own, asked a second time before it counts as failed,
//! and the network is only blamed after several looks in a row over the grace
//! period.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::Notify;

/// The public status page, on its own server so it outlives the site.
const STATUS_URL: &str = "https://status.kryo.to";
/// Failures in a row, and how long they must span, before anything is said.
const FAILS: u32 = 3;
const GRACE: Duration = Duration::from_secs(30);
/// How long the feed gets to catch up before "it says the site is fine" is believed.
const FEED_LAG: Duration = Duration::from_secs(120);
/// Good answers in a row before an outage is over.
const OKS: u32 = 2;
/// Reads in a row before a partial problem is shown, or taken away.
const PART_READS: u32 = 2;

const CALM: Duration = Duration::from_secs(30);
const HIDDEN: Duration = Duration::from_secs(180);
const WATCHFUL: Duration = Duration::from_secs(10);
/// Once an outage has gone on a while, asking less leaves a recovering site alone.
const PATIENT: Duration = Duration::from_secs(20);
const PATIENT_AFTER: Duration = Duration::from_secs(300);
const TIMEOUT: Duration = Duration::from_secs(8);
/// Neutral addresses, each with text only its real answer carries.
/// msftconnecttest is what Windows itself asks to tell a Wi-Fi sign-in page
/// from the internet.
const NEUTRAL: [(&str, &str); 2] = [
    ("http://www.msftconnecttest.com/connecttest.txt", "Microsoft Connect Test"),
    ("https://www.cloudflare.com/cdn-cgi/trace", "h=www.cloudflare.com"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    #[default]
    Ok,
    Partial,
    Down,
    Unreachable,
}

impl State {
    fn outage(self) -> bool {
        matches!(self, State::Down | State::Unreachable)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub state: State,
    /// What is affected, in the player's words, for `partial`.
    pub parts: Vec<String>,
    /// The last request to the site got an answer.
    pub answering: bool,
    /// The status page answered last time it was asked.
    pub status_up: bool,
    pub status_url: String,
    /// kryo.to does not answer this app while the internet does (the Store
    /// page reaches kryo.to, or a neutral address answers): this network is
    /// blocking kryo.to, as internet providers in some countries do for sites
    /// on piracy lists. The player is told so and what gets around it.
    pub blocked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Site {
    Fine,
    Degraded(Vec<String>),
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Feed {
    Dead,
    Says { site_bad: bool, parts: Vec<String> },
}

/// The feed's and the site's names for things, as a player knows them.
fn part_name(id: &str) -> Option<&'static str> {
    Some(match id {
        "website" => "kryo.to",
        "accounts" => "Sign-in",
        "catalog" | "games" => "Game pages",
        "community" | "requests" => "Community",
        "downloads" => "Downloads",
        "chat" | "messaging" => "Chat",
        "voice" => "Voice chat",
        _ => return None,
    })
}

fn add(parts: &mut Vec<String>, name: &str) {
    if !parts.iter().any(|p| p == name) {
        parts.push(name.to_string());
    }
}

/// What `/api/health` answered. Only a 5xx without the site's own body, or no
/// answer at all, is a failure: a 503 with one is the site saying what is wrong,
/// and anything else (a refusal, a rate limit) means something is there.
fn read_site(code: u16, body: &str) -> Site {
    if code < 500 {
        return Site::Fine;
    }
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else { return Site::Failed };
    let Some(checks) = json["checks"].as_object() else { return Site::Failed };
    let mut parts = Vec::new();
    for (id, check) in checks {
        if check["ok"].as_bool() == Some(false) {
            add(&mut parts, part_name(id).unwrap_or("Some of Kryoto"));
        }
    }
    if parts.is_empty() {
        add(&mut parts, "Some of Kryoto");
    }
    Site::Degraded(parts)
}

/// status.kryo.to's `/api/status`.
fn read_feed(body: &str) -> Feed {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else { return Feed::Dead };
    let Some(groups) = json["groups"].as_array() else { return Feed::Dead };
    let mut site_bad = false;
    let mut parts = Vec::new();
    for component in groups.iter().filter_map(|g| g["components"].as_array()).flatten() {
        let id = component["id"].as_str().unwrap_or("");
        let status = component["status"].as_str().unwrap_or("unknown");
        if status == "operational" || status == "unknown" {
            continue;
        }
        if matches!(id, "website" | "main" | "proxy") {
            site_bad = true;
        }
        if let Some(name) = part_name(id) {
            add(&mut parts, name);
        }
    }
    Feed::Says { site_bad, parts }
}

/// A neutral address answering as itself. A Wi-Fi that wants a sign-in first
/// (a hotel's, an airport's) answers plain HTTP with its own page, and that
/// is no internet yet: nobody there is helped by being told to get a VPN.
fn neutral_answered(code: u16, body: &str, mark: &str) -> bool {
    (200..300).contains(&code) && body.contains(mark)
}

/// What has been seen so far, and what it adds up to.
#[derive(Default)]
struct Watch {
    fails: u32,
    first_fail: Option<Instant>,
    oks: u32,
    feed: Option<Feed>,
    state: State,
    parts: Vec<String>,
    seen_parts: Vec<String>,
    part_reads: u32,
    /// This app's own requests to kryo.to that failed in a row while the
    /// internet (or the Store page) answered, and when the run began.
    own_fails: u32,
    own_first_fail: Option<Instant>,
}

impl Watch {
    fn step(&mut self, site: Site, feed: Option<Feed>, now: Instant) {
        if feed.is_some() {
            self.feed = feed;
        }
        let failed = site == Site::Failed;
        if failed {
            self.fails += 1;
            self.first_fail.get_or_insert(now);
            self.oks = 0;
        } else {
            self.oks += 1;
            // A single good answer clears a run of failures, but not an outage.
            if !self.state.outage() || self.oks >= OKS {
                self.fails = 0;
                self.first_fail = None;
                if self.state.outage() {
                    self.state = State::Ok;
                }
            }
        }
        if let Some(since) = self.first_fail {
            let long = now.duration_since(since);
            if self.fails >= FAILS && long >= GRACE {
                match &self.feed {
                    Some(Feed::Says { site_bad: true, .. }) => {
                        self.state = State::Down;
                        self.parts.clear();
                    }
                    // Its word is taken once it has had time to notice, or
                    // once an outage is already called: then it stays called.
                    Some(Feed::Says { site_bad: false, .. }) if long < FEED_LAG || self.state == State::Down => {}
                    _ => {
                        if self.state != State::Down {
                            self.state = State::Unreachable;
                            self.parts.clear();
                        }
                    }
                }
            }
            return;
        }
        if self.state.outage() {
            return;
        }
        let mut parts = match site {
            Site::Degraded(parts) => parts,
            _ => Vec::new(),
        };
        if let Some(Feed::Says { parts: said, .. }) = &self.feed {
            for part in said {
                add(&mut parts, part);
            }
        }
        if parts == self.seen_parts {
            self.part_reads += 1;
        } else {
            self.seen_parts = parts.clone();
            self.part_reads = 1;
        }
        if self.part_reads >= PART_READS {
            self.state = if parts.is_empty() { State::Ok } else { State::Partial };
            self.parts = parts;
        }
    }

    /// Whether this network blocks kryo.to, as opposed to Kryoto being down.
    /// Called after `step`, so the status page's word for this look is in.
    ///
    /// This app's own request failed (`own_failed`), `FAILS` looks in a row
    /// over `GRACE`, and one of:
    /// - the Store page reaches kryo.to (`page_reached`): the site is plainly
    ///   up, and only this app's requests are stopped;
    /// - the rest of the internet answers (`internet`) and the status page,
    ///   which watches the site from outside, has had `FEED_LAG` to notice an
    ///   outage and still says the site is fine, or cannot be reached either
    ///   (it is under kryo.to too, so a block takes both).
    ///
    /// The status page saying the site is bad is Kryoto down, never a block;
    /// and before it has had time to notice, nobody is blamed (0.3.10 said
    /// "blocked" through the first minutes of every real outage, KB-0328).
    /// One good answer of this app's own ends it.
    fn blocked(&mut self, own_failed: bool, page_reached: bool, internet: bool, now: Instant) -> bool {
        if !own_failed {
            self.own_fails = 0;
            self.own_first_fail = None;
            return false;
        }
        self.own_fails += 1;
        let long = now.duration_since(*self.own_first_fail.get_or_insert(now));
        if self.own_fails < FAILS || long < GRACE {
            return false;
        }
        if page_reached {
            return true;
        }
        if !internet {
            return false;
        }
        match &self.feed {
            Some(Feed::Says { site_bad: true, .. }) => false,
            Some(Feed::Says { site_bad: false, .. }) => long >= FEED_LAG,
            Some(Feed::Dead) | None => true,
        }
    }

    /// Anything less than calm: look again soon.
    fn watchful(&self) -> bool {
        self.fails > 0 || self.own_fails > 0 || self.state != State::Ok || !self.seen_parts.is_empty()
    }

    fn report(&self, status_url: String) -> Health {
        Health {
            state: self.state,
            parts: self.parts.clone(),
            answering: self.fails == 0 || self.oks > 0,
            status_up: !matches!(self.feed, Some(Feed::Dead) | None),
            status_url,
            blocked: false,
        }
    }
}

/// The status page. A test against a stand-in names its own, and only with the
/// developer's catalog address set too: a player's install always asks the real one.
fn status_url(testing: bool) -> String {
    if testing {
        if let Ok(url) = std::env::var("KRYOTO_STATUS_URL") {
            return url.trim_end_matches('/').to_string();
        }
    }
    STATUS_URL.to_string()
}

static NOW: Mutex<Option<Health>> = Mutex::new(None);
static POKE: Notify = Notify::const_new();
/// The Store page's own `/api/health` answer (`page_answered`): status (0 for
/// none at all) and body.
static PAGE: Mutex<Option<(u16, String)>> = Mutex::new(None);
static PAGE_ANSWERED: Notify = Notify::const_new();

/// `/api/health`, or why there was no answer (every cause in the chain:
/// reqwest's own text names only the address). Asked a second time, a moment
/// later, before no answer is believed: a single request lost on the way is
/// common and means nothing.
async fn ask_site(client: &reqwest::Client, endpoint: &str) -> Result<Site, String> {
    let mut why = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        match client.get(format!("{endpoint}/api/health")).send().await {
            Ok(res) => {
                let code = res.status().as_u16();
                return Ok(read_site(code, &res.text().await.unwrap_or_default()));
            }
            Err(e) => why = crate::logging::why(&e),
        }
    }
    // A block page answering for kryo.to on this network's DNS: the next look
    // asks a public resolver where kryo.to is (net.rs `note_failure`).
    crate::net::note_failure(endpoint, &why);
    Err(why)
}

/// What the Store page got for `/api/health` (lib.rs `report_catalog_state`).
pub fn page_answered(code: u16, body: String) {
    *PAGE.lock().unwrap_or_else(|e| e.into_inner()) = Some((code, body));
    PAGE_ANSWERED.notify_one();
}

/// The Store page's own reading of the site, when it is on kryo.to and
/// answers in time; `None` when it cannot say.
/// Whether the internet answers from this PC at all, asked only when kryo.to
/// did not: two neutral addresses, either is enough.
async fn internet_answers(client: &reqwest::Client) -> Result<(), String> {
    let mut why = Vec::new();
    for (url, mark) in NEUTRAL {
        match client.get(url).timeout(Duration::from_secs(6)).send().await {
            Ok(res) => {
                let code = res.status();
                match res.text().await {
                    Ok(body) if neutral_answered(code.as_u16(), &body, mark) => return Ok(()),
                    Ok(_) if code.is_success() => why.push(format!("{url}: {code} from another page (a Wi-Fi sign-in?)")),
                    Ok(_) => why.push(format!("{url}: {code}")),
                    Err(e) => why.push(format!("{url}: {code}, then {}", crate::logging::why(&e))),
                }
            }
            Err(e) => why.push(format!("{url}: {}", crate::logging::why(&e))),
        }
    }
    Err(why.join("; "))
}

async fn ask_page<R: Runtime>(app: &AppHandle<R>) -> Option<Site> {
    let store = app.get_webview(crate::STORE)?;
    PAGE.lock().unwrap_or_else(|e| e.into_inner()).take();
    store.eval("window.__kryoHealthTick && window.__kryoHealthTick()").ok()?;
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        if let Some((code, body)) = PAGE.lock().unwrap_or_else(|e| e.into_inner()).take() {
            return Some(if code == 0 { Site::Failed } else { read_site(code, &body) });
        }
        // A wake-up left over from an earlier answer finds nothing: wait on.
        tokio::time::timeout_at(deadline, PAGE_ANSWERED.notified()).await.ok()?;
    }
}

/// This app's reading, or the Store page's when this app got no answer.
fn reading(own: &Result<Site, String>, page: Option<Site>) -> Site {
    match (own, page) {
        (Ok(site), _) => site.clone(),
        (Err(_), Some(site)) => site,
        (Err(_), None) => Site::Failed,
    }
}

async fn ask_feed(client: &reqwest::Client, status: &str) -> Feed {
    let Ok(res) = client.get(format!("{status}/api/status")).send().await else { return Feed::Dead };
    if !res.status().is_success() {
        return Feed::Dead;
    }
    read_feed(&res.text().await.unwrap_or_default())
}

fn window_up<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
}

/// A little either side of the interval, so PCs do not ask in step.
fn jitter(wait: Duration) -> Duration {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    wait.mul_f64(0.9 + f64::from(nanos % 1000) / 5000.0)
}

/// Windows: how many window-manager objects (windows, icons, menus, cursors)
/// this process holds, of the 10,000 it may. Some PCs run out after long use,
/// and then no window can open (the tray panel, a download's page, the pet;
/// KB-0335); what uses them up is not known. Said once past each mark, with
/// what the app is doing, so the next report names it.
#[cfg(windows)]
fn watch_window_objects<R: Runtime>(app: &AppHandle<R>, said: &mut u32) {
    #[link(name = "user32")]
    extern "system" {
        fn GetGuiResources(process: *mut std::ffi::c_void, flags: u32) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }
    // SAFETY: both only read this process's own counters.
    let (user, gdi) = unsafe { (GetGuiResources(GetCurrentProcess(), 1), GetGuiResources(GetCurrentProcess(), 0)) };
    let mark = [8000, 5000, 2500, 1000].into_iter().find(|m| user >= *m).unwrap_or(0);
    if mark <= *said {
        return;
    }
    *said = mark;
    let running = app.try_state::<crate::library::Running>().and_then(|r| r.0.lock().ok().map(|m| m.len())).unwrap_or(0);
    let line = format!(
        "window objects: {user} of 10000 in use (GDI {gdi}); views open: {}, games running: {running}, controllers: {}, pet: {}",
        app.webviews().len(),
        crate::pad::connected(app).0.len(),
        app.get_webview_window(crate::pet::LABEL).is_some(),
    );
    if mark >= 5000 {
        crate::logging::error("windows", &line);
    } else {
        crate::logging::warn("windows", &line);
    }
}

pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let base = || {
            crate::net::builder()
                .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
                .timeout(TIMEOUT)
                // A connection of its own for every look. Kept between looks, one
                // that a router, a VPN or an antivirus had dropped without a word
                // (or that died while the PC slept) took the next request with it:
                // no answer for the whole timeout, on a network that was fine.
                .pool_max_idle_per_host(0)
        };
        let Ok(client) = base().build() else {
            return;
        };
        // The request to kryo.to itself, with its name encrypted where that
        // works (net.rs `kryoto_tls`, KB-0347): built again when that changes.
        let mut own_client: Option<(u64, String, reqwest::Client)> = None;
        let mut own_failed = false;
        let mut watch = Watch::default();
        let mut round = 0u32;
        let mut owed_checked = false;
        let mut outage_since: Option<Instant> = None;
        // Said once a run of it: the page reaches kryo.to and this app does not.
        let mut said_split = false;
        #[cfg(windows)]
        let mut objects_said = 0u32;
        loop {
            #[cfg(windows)]
            watch_window_objects(&app, &mut objects_said);
            let settings = crate::settings::load(&app);
            let endpoint = crate::settings::catalog_endpoint(&settings);
            let status = status_url(!settings.catalog_endpoint.trim().is_empty());
            if crate::net::ech_due(own_failed) {
                crate::net::check_ech().await;
            }
            let generation = crate::net::ech_generation();
            if !own_client.as_ref().is_some_and(|(g, e, _)| *g == generation && *e == endpoint) {
                own_client = crate::net::kryoto_tls(&endpoint, base()).build().ok().map(|c| (generation, endpoint.clone(), c));
            }
            let own = ask_site(own_client.as_ref().map_or(&client, |(_, _, c)| c), &endpoint).await;
            own_failed = own.is_err();
            let page = if own.is_err() { ask_page(&app).await } else { None };
            let page_reached = page.as_ref().is_some_and(|p| *p != Site::Failed);
            // kryo.to failed here: does anything else answer? Then it is this
            // network, once it has gone on (`Watch::blocked`).
            let internet = if own.is_err() && !page_reached { Some(internet_answers(&client).await) } else { None };
            // For the log: what this round actually saw, in words.
            let seen = format!(
                "own request: {}; Store page: {}",
                match &own {
                    Ok(s) => format!("{s:?}"),
                    Err(e) => e.clone(),
                },
                match &page {
                    Some(p) => format!("{p:?}"),
                    None => "not asked or no answer".into(),
                }
            ) + &match &internet {
                Some(Ok(())) => "; the rest of the internet answers".to_string(),
                Some(Err(e)) => format!("; the rest of the internet does not either ({e})"),
                None => String::new(),
            };
            let site = reading(&own, page);
            // The feed every other round while all is calm, every round otherwise.
            let feed = if watch.watchful() || site != Site::Fine || round.is_multiple_of(2) {
                Some(ask_feed(&client, &status).await)
            } else {
                None
            };
            round = round.wrapping_add(1);
            let was = watch.state;
            watch.step(site, feed, Instant::now());
            let blocked = watch.blocked(own.is_err(), page_reached, matches!(internet, Some(Ok(()))), Instant::now());
            match &own {
                // Reported once a run of it, and only once it has lasted: a
                // single miss is a line in the log, not an error.
                Err(e) if blocked && page_reached && !said_split => {
                    said_split = true;
                    crate::logging::error("health", &format!("kryo.to answers the Store but not this app's own requests: {e}"));
                }
                Err(e) if !blocked => crate::logging::info("health", &format!("a look at kryo.to got no answer: {e}")),
                Ok(_) => said_split = false,
                _ => {}
            }
            let mut health = watch.report(status);
            health.blocked = blocked;
            let changed = {
                let mut now = NOW.lock().expect("health");
                let changed = now.as_ref() != Some(&health);
                *now = Some(health.clone());
                changed
            };
            if changed {
                // Every change, with what was seen and how long the last state
                // lasted, so a report of "not answering" says why and for how long.
                let lasted = outage_since.map(|t| format!(", after {} s", t.elapsed().as_secs())).unwrap_or_default();
                let line = format!(
                    "{:?} {:?}, status page up: {}, kryo.to blocked on this network: {}{lasted} ({seen})",
                    health.state, health.parts, health.status_up, health.blocked
                );
                if health.state.outage() || health.blocked {
                    crate::logging::warn("health", &line);
                } else {
                    crate::logging::info("health", &line);
                }
                let _ = app.emit("service-health", &health);
            }
            let reachable = !watch.state.outage() && watch.fails == 0;
            // Backups the site could not take: on the way out of an outage, and
            // the first time it answers after a start (the PC may have been off).
            if reachable && (was.outage() || !owed_checked) {
                owed_checked = true;
                crate::saves::catch_up(app.clone());
            }
            outage_since = if watch.state.outage() { outage_since.or(Some(Instant::now())) } else { None };
            let wait = if !watch.watchful() {
                if window_up(&app) { CALM } else { HIDDEN }
            } else if outage_since.is_some_and(|t| t.elapsed() > PATIENT_AFTER) {
                PATIENT
            } else {
                WATCHFUL
            };
            tokio::select! {
                () = tokio::time::sleep(jitter(wait)) => {}
                () = POKE.notified() => {}
            }
        }
    });
}

#[tauri::command]
pub fn health_get() -> Option<Health> {
    NOW.lock().expect("health").clone()
}

/// Something failed, or the player pressed Try again: look now, not on the timer.
#[tauri::command]
pub fn health_poke() {
    POKE.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;

    const FINE: &str = r#"{"groups":[{"components":[{"id":"website","status":"operational"},{"id":"chat","status":"operational"}]}]}"#;
    const SITE_DOWN: &str = r#"{"groups":[{"components":[{"id":"website","status":"down"},{"id":"chat","status":"operational"}]}]}"#;
    const CHAT_DOWN: &str = r#"{"groups":[{"components":[{"id":"website","status":"operational"},{"id":"chat","status":"degraded"},{"id":"discord","status":"down"}]}]}"#;

    /// This app's request failing (an antivirus scanning HTTPS, say) while
    /// the Store page reaches kryo.to is no outage: the page's answer counts.
    #[test]
    fn the_store_page_answering_is_kryoto_answering() {
        let failed: Result<Site, String> = Err("error sending request: invalid peer certificate: UnknownIssuer".into());
        assert_eq!(reading(&failed, Some(Site::Fine)), Site::Fine);
        assert_eq!(reading(&failed, Some(Site::Failed)), Site::Failed);
        assert_eq!(reading(&failed, None), Site::Failed);
        assert_eq!(reading(&Ok(Site::Fine), Some(Site::Failed)), Site::Fine);
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in (0..=300).step_by(10) {
            assert_eq!(run(&mut w, t, at, reading(&failed, Some(Site::Fine)), Some("<html>")), State::Ok, "at {at}");
        }
    }

    /// One request of this app's that got no answer (a dropped connection, a
    /// PC just awake) is not a blocked network, however well the Store page
    /// answers: 0.3.10 said so at once (KB-0328).
    #[test]
    fn one_missed_request_is_not_a_blocked_network() {
        let (mut w, t) = (Watch::default(), Instant::now());
        let at = |s: u64| t + Duration::from_secs(s);
        assert!(!w.blocked(true, true, false, at(0)));
        assert!(w.watchful(), "it looks again soon");
        assert!(!w.blocked(false, true, false, at(10)));
        assert!(!w.watchful());
        // Every other look failing, as those PCs had it for hours: never.
        for round in 0..40 {
            assert!(!w.blocked(round % 2 == 0, true, false, at(20 + round * 30)), "round {round}");
        }
    }

    /// The Store page reaches kryo.to and this app's requests keep failing.
    #[test]
    fn a_network_that_keeps_refusing_this_app_is_called_blocked() {
        let (mut w, t) = (Watch::default(), Instant::now());
        let at = |s: u64| t + Duration::from_secs(s);
        assert!(!w.blocked(true, true, false, at(0)));
        assert!(!w.blocked(true, true, false, at(10)));
        // Three in a row, but not yet over the grace period.
        assert!(!w.blocked(true, true, false, at(20)));
        assert!(w.blocked(true, true, false, at(30)));
        assert!(w.blocked(true, true, false, at(40)));
        // Its own request answered: over at once.
        assert!(!w.blocked(false, true, false, at(50)));
        assert!(!w.blocked(true, true, false, at(60)));
    }

    /// One whole look: the site's answer, the feed's, then the question.
    fn look(w: &mut Watch, t: Instant, at: u64, feed: &str, internet: bool) -> (State, bool) {
        let now = t + Duration::from_secs(at);
        w.step(Site::Failed, Some(read_feed(feed)), now);
        (w.state, w.blocked(true, false, internet, now))
    }

    /// Kryoto really down: the rest of the internet answers here too, and
    /// that must never read as this network blocking it.
    #[test]
    fn kryoto_down_is_never_called_blocked() {
        let (mut w, t) = (Watch::default(), Instant::now());
        // The status page has not noticed yet: nobody is blamed.
        for at in (0..=110).step_by(10) {
            assert_eq!(look(&mut w, t, at, FINE, true), (State::Ok, false), "at {at}");
        }
        // It has: down, not blocked, for as long as it lasts.
        for at in (120..=900).step_by(20) {
            assert_eq!(look(&mut w, t, at, SITE_DOWN, true), (State::Down, false), "at {at}");
        }
    }

    /// The status page, from outside, keeps saying the site is fine while
    /// this PC reaches the internet and not kryo.to: this network.
    #[test]
    fn the_site_fine_from_outside_and_unreachable_here_is_blocked() {
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in (0..=110).step_by(10) {
            assert!(!look(&mut w, t, at, FINE, true).1, "at {at}");
        }
        assert_eq!(look(&mut w, t, 120, FINE, true), (State::Unreachable, true));
        // The status page is under kryo.to too: a block usually takes both.
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in [0, 10, 20] {
            assert!(!look(&mut w, t, at, "<html>", true).1, "at {at}");
        }
        assert!(look(&mut w, t, 30, "<html>", true).1);
    }

    #[test]
    fn no_internet_at_all_is_not_a_blocked_network() {
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in (0..=300).step_by(10) {
            assert!(!look(&mut w, t, at, "<html>", false).1, "at {at}");
        }
    }

    fn run(watch: &mut Watch, start: Instant, at: u64, site: Site, feed: Option<&str>) -> State {
        watch.step(site, feed.map(read_feed), start + Duration::from_secs(at));
        watch.state
    }

    #[test]
    fn one_failed_request_does_nothing() {
        let (mut w, t) = (Watch::default(), Instant::now());
        assert_eq!(run(&mut w, t, 0, Site::Fine, Some(FINE)), State::Ok);
        assert_eq!(run(&mut w, t, 30, Site::Failed, Some(FINE)), State::Ok);
        assert_eq!(run(&mut w, t, 40, Site::Fine, Some(FINE)), State::Ok);
        assert!(!w.watchful());
    }

    #[test]
    fn an_outage_needs_the_grace_period_and_the_feed() {
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in [0, 10, 20] {
            assert_eq!(run(&mut w, t, at, Site::Failed, Some(SITE_DOWN)), State::Ok, "at {at}");
        }
        assert_eq!(run(&mut w, t, 30, Site::Failed, Some(SITE_DOWN)), State::Down);
    }

    #[test]
    fn the_feed_lagging_holds_it_back_then_it_is_only_unreachable() {
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in (0..=110).step_by(10) {
            assert_eq!(run(&mut w, t, at, Site::Failed, Some(FINE)), State::Ok, "at {at}");
        }
        assert_eq!(run(&mut w, t, 120, Site::Failed, Some(FINE)), State::Unreachable);
    }

    #[test]
    fn with_the_status_page_dead_too_nobody_is_blamed() {
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in [0, 10, 20] {
            run(&mut w, t, at, Site::Failed, Some("<html>"));
        }
        assert_eq!(run(&mut w, t, 30, Site::Failed, Some("<html>")), State::Unreachable);
        assert!(!w.report(String::new()).status_up);
    }

    #[test]
    fn it_takes_two_good_answers_to_end_an_outage() {
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in [0, 10, 20, 30] {
            run(&mut w, t, at, Site::Failed, Some(SITE_DOWN));
        }
        assert_eq!(run(&mut w, t, 40, Site::Fine, Some(SITE_DOWN)), State::Down);
        assert_eq!(run(&mut w, t, 50, Site::Failed, Some(SITE_DOWN)), State::Down);
        assert_eq!(run(&mut w, t, 60, Site::Fine, Some(FINE)), State::Down);
        assert_eq!(run(&mut w, t, 70, Site::Fine, Some(FINE)), State::Ok);
    }

    #[test]
    fn a_partial_problem_is_named_after_two_reads_and_clears_the_same_way() {
        let (mut w, t) = (Watch::default(), Instant::now());
        assert_eq!(run(&mut w, t, 0, Site::Fine, Some(CHAT_DOWN)), State::Ok);
        assert_eq!(run(&mut w, t, 10, Site::Fine, Some(CHAT_DOWN)), State::Partial);
        assert_eq!(w.parts, vec!["Chat"]);
        assert_eq!(run(&mut w, t, 20, Site::Fine, Some(FINE)), State::Partial);
        assert_eq!(run(&mut w, t, 30, Site::Fine, Some(FINE)), State::Ok);
    }

    /// A Wi-Fi sign-in page answering for the neutral addresses is no internet,
    /// so kryo.to failing behind it is not called a block.
    #[test]
    fn a_wifi_sign_in_page_is_not_the_internet_answering() {
        // What the two addresses really answer.
        let [(_, msft), (_, cloudflare)] = NEUTRAL;
        assert!(neutral_answered(200, "Microsoft Connect Test", msft));
        assert!(neutral_answered(200, "fl=60f44\nh=www.cloudflare.com\nip=203.0.113.7\nvisit_scheme=https\n", cloudflare));
        let portal = "<html><title>Guest Wi-Fi</title><form action=/login>Accept the terms</form></html>";
        assert!(!neutral_answered(200, portal, msft));
        assert!(!neutral_answered(200, portal, cloudflare));
        assert!(!neutral_answered(511, "Microsoft Connect Test", msft));
    }

    #[test]
    fn the_site_saying_a_database_is_down_is_partial_not_an_outage() {
        let body = r#"{"status":"degraded","checks":{"accounts":{"ok":true,"ms":3},"messaging":{"ok":false,"ms":4000}}}"#;
        assert_eq!(read_site(503, body), Site::Degraded(vec!["Chat".to_string()]));
        assert_eq!(read_site(502, "<html>Bad gateway</html>"), Site::Failed);
        assert_eq!(read_site(429, "slow down"), Site::Fine);
    }
}
