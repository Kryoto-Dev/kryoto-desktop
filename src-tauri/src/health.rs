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

    /// Anything less than calm: look again soon.
    fn watchful(&self) -> bool {
        self.fails > 0 || self.state != State::Ok || !self.seen_parts.is_empty()
    }

    fn report(&self, status_url: String) -> Health {
        Health {
            state: self.state,
            parts: self.parts.clone(),
            answering: self.fails == 0 || self.oks > 0,
            status_up: !matches!(self.feed, Some(Feed::Dead) | None),
            status_url,
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

async fn ask_site(client: &reqwest::Client, endpoint: &str) -> Site {
    let Ok(res) = client.get(format!("{endpoint}/api/health")).send().await else { return Site::Failed };
    let code = res.status().as_u16();
    read_site(code, &res.text().await.unwrap_or_default())
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
        loop {
            let settings = crate::settings::load(&app);
            let endpoint = crate::settings::catalog_endpoint(&settings);
            let status = status_url(!settings.catalog_endpoint.trim().is_empty());
            let site = ask_site(&client, &endpoint).await;
            // The feed every other round while all is calm, every round otherwise.
            let feed = if watch.watchful() || site != Site::Fine || round.is_multiple_of(2) {
                Some(ask_feed(&client, &status).await)
            } else {
                None
            };
            round = round.wrapping_add(1);
            let was = watch.state;
            watch.step(site, feed, Instant::now());
            let health = watch.report(status);
            let changed = {
                let mut now = NOW.lock().expect("health");
                let changed = now.as_ref() != Some(&health);
                *now = Some(health.clone());
                changed
            };
            if changed {
                crate::logging::info("health", &format!("{:?} {:?}, status page up: {}", health.state, health.parts, health.status_up));
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
}
