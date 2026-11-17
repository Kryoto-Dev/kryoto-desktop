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
    /// Like an outage, only once it has lasted (`FAILS` over `GRACE`), and
    /// never while the status feed says the site itself is down.
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
    /// Rounds in a row that looked like this network blocking kryo.to, and
    /// since when: counted apart from `fails`, because the Store page reaching
    /// kryo.to is a block for this app and no failure for the Store.
    blocks: u32,
    first_block: Option<Instant>,
    blocked: bool,
}

impl Watch {
    /// Takes this round's word on a block: this app got no answer from kryo.to
    /// while the Store page or a neutral address did. Like a failure, it is
    /// said only once it has lasted. Any other round ends it: kryo.to answered,
    /// or nothing else did either, so it is not this network.
    fn step_block(&mut self, looks_blocked: bool, now: Instant) {
        if looks_blocked {
            self.blocks += 1;
            let since = *self.first_block.get_or_insert(now);
            self.blocked = self.blocks >= FAILS && now.duration_since(since) >= GRACE;
        } else {
            self.blocks = 0;
            self.first_block = None;
            self.blocked = false;
        }
    }

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

    /// Anything less than calm, or a block not yet said: look again soon.
    fn watchful(&self) -> bool {
        self.fails > 0 || self.state != State::Ok || !self.seen_parts.is_empty() || (self.blocks > 0 && !self.blocked)
    }

    fn report(&self, status_url: String) -> Health {
        Health {
            state: self.state,
            parts: self.parts.clone(),
            answering: self.fails == 0 || self.oks > 0,
            status_up: !matches!(self.feed, Some(Feed::Dead) | None),
            status_url,
            // The feed saying the site is down outranks it: then it is not this network.
            blocked: self.blocked && !matches!(self.feed, Some(Feed::Says { site_bad: true, .. })),
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
/// reqwest's own text names only the address).
async fn ask_site(client: &reqwest::Client, endpoint: &str) -> Result<Site, String> {
    let res = client.get(format!("{endpoint}/api/health")).send().await.map_err(|e| crate::logging::why(&e))?;
    let code = res.status().as_u16();
    Ok(read_site(code, &res.text().await.unwrap_or_default()))
}

/// What the Store page got for `/api/health` (lib.rs `report_catalog_state`).
pub fn page_answered(code: u16, body: String) {
    *PAGE.lock().unwrap_or_else(|e| e.into_inner()) = Some((code, body));
    PAGE_ANSWERED.notify_one();
}

/// Whether the internet answers from this PC at all, asked only when kryo.to
/// did not: two neutral addresses, either is enough.
async fn internet_answers(client: &reqwest::Client) -> Result<(), String> {
    let mut why = Vec::new();
    for url in ["http://www.msftconnecttest.com/connecttest.txt", "https://www.cloudflare.com/cdn-cgi/trace"] {
        match client.get(url).timeout(Duration::from_secs(6)).send().await {
            Ok(res) if res.status().is_success() => return Ok(()),
            Ok(res) => why.push(format!("{url}: {}", res.status())),
            Err(e) => why.push(format!("{url}: {}", crate::logging::why(&e))),
        }
    }
    Err(why.join("; "))
}

/// The Store page's own reading of the site, when it is on kryo.to and
/// answers in time; `None` when it cannot say.
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

pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let Ok(client) = reqwest::Client::builder()
            .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
            .timeout(TIMEOUT)
            .build()
        else {
            return;
        };
        let mut watch = Watch::default();
        let mut round = 0u32;
        let mut owed_checked = false;
        let mut outage_since: Option<Instant> = None;
        // Said once a run of it: the page reaches kryo.to and this app does not.
        let mut said_split = false;
        loop {
            let settings = crate::settings::load(&app);
            let endpoint = crate::settings::catalog_endpoint(&settings);
            let status = status_url(!settings.catalog_endpoint.trim().is_empty());
            let own = ask_site(&client, &endpoint).await;
            let page = if own.is_err() { ask_page(&app).await } else { None };
            match (&own, &page) {
                (Err(e), Some(site)) if *site != Site::Failed && !said_split => {
                    said_split = true;
                    crate::logging::error("health", &format!("kryo.to answers the Store but not this app's own requests: {e}"));
                }
                (Ok(_), _) => said_split = false,
                _ => {}
            }
            let page_reached = page.as_ref().is_some_and(|p| *p != Site::Failed);
            // kryo.to failed here: does anything else answer? Then it is this network.
            let internet = if own.is_err() && !page_reached { Some(internet_answers(&client).await) } else { None };
            let looks_blocked = own.is_err() && (page_reached || matches!(internet, Some(Ok(()))));
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
            let at = Instant::now();
            watch.step(site, feed, at);
            watch.step_block(looks_blocked, at);
            let health = watch.report(status);
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

    #[test]
    fn the_site_saying_a_database_is_down_is_partial_not_an_outage() {
        let body = r#"{"status":"degraded","checks":{"accounts":{"ok":true,"ms":3},"messaging":{"ok":false,"ms":4000}}}"#;
        assert_eq!(read_site(503, body), Site::Degraded(vec!["Chat".to_string()]));
        assert_eq!(read_site(502, "<html>Bad gateway</html>"), Site::Failed);
        assert_eq!(read_site(429, "slow down"), Site::Fine);
    }

    /// One round as `start` runs it: what was read, and whether this app
    /// missed kryo.to while the Store page or a neutral address answered.
    fn round(watch: &mut Watch, start: Instant, at: u64, site: Site, feed: Option<&str>, looks_blocked: bool) -> Health {
        let now = start + Duration::from_secs(at);
        watch.step(site, feed.map(read_feed), now);
        watch.step_block(looks_blocked, now);
        watch.report(String::new())
    }

    /// A blip (waking from sleep, one slow answer) while the rest of the
    /// internet answers is no blocked network: nobody is told to get a VPN.
    #[test]
    fn one_round_that_looks_blocked_says_nothing() {
        let (mut w, t) = (Watch::default(), Instant::now());
        assert!(!round(&mut w, t, 0, Site::Failed, Some(FINE), true).blocked);
        assert!(!round(&mut w, t, 10, Site::Fine, Some(FINE), false).blocked);
        assert!(!round(&mut w, t, 40, Site::Failed, Some(FINE), true).blocked);
    }

    #[test]
    fn a_block_is_said_once_it_has_lasted_and_goes_with_an_answer() {
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in [0, 10, 20] {
            assert!(!round(&mut w, t, at, Site::Failed, Some(FINE), true).blocked, "at {at}");
        }
        assert!(round(&mut w, t, 30, Site::Failed, Some(FINE), true).blocked);
        assert!(!round(&mut w, t, 40, Site::Fine, Some(FINE), false).blocked);
    }

    /// The status page saying the site is down outranks it: "Kryoto itself is
    /// up, try a VPN" is never said while Kryoto is down. Once the feed says
    /// the site is back and kryo.to still does not answer here, it is this network.
    #[test]
    fn the_status_page_saying_down_is_never_called_a_block() {
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in (0..=60).step_by(10) {
            assert!(!round(&mut w, t, at, Site::Failed, Some(SITE_DOWN), true).blocked, "at {at}");
        }
        assert_eq!(w.state, State::Down);
        let health = round(&mut w, t, 70, Site::Failed, Some(FINE), true);
        assert_eq!(health.state, State::Down);
        assert!(health.blocked);
    }

    /// The Store page reaching kryo.to while this app cannot: the Store stays
    /// up, the watcher looks again soon while it might be a block, and the
    /// block on this app's own requests is said after the same wait.
    #[test]
    fn the_store_page_reaching_kryoto_is_a_block_once_it_has_lasted() {
        let (mut w, t) = (Watch::default(), Instant::now());
        for at in [0, 10, 20] {
            let health = round(&mut w, t, at, Site::Fine, Some(FINE), true);
            assert!(!health.blocked, "at {at}");
            assert_eq!(health.state, State::Ok);
            assert!(w.watchful(), "at {at}");
        }
        let health = round(&mut w, t, 30, Site::Fine, Some(FINE), true);
        assert!(health.blocked);
        assert_eq!(health.state, State::Ok);
        assert!(!w.watchful());
    }
}
