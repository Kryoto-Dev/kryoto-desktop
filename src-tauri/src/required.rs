//! The oldest Kryoto Desktop kryo.to still allows, for a version with a
//! problem serious enough that nobody may keep using it (KB-0375: 0.3.11 could
//! delete a whole games folder on uninstall).
//!
//! The owner sets it in kryo.to's Admin > Settings. kryo.to enforces it on its
//! own for every version (it knows each one by its user agent: the Store shows
//! "Update Kryoto Desktop", download links are refused). From this version on
//! the app also asks `GET /api/desktop/required` (`{ min, reason, latest }`) at
//! start, every few hours and when the Store's page says so
//! (`store_required_check`), and while it is older than `min` the shell shows
//! a screen that cannot be dismissed (src/shell/RequiredUpdate.tsx): Update
//! now, Download it yourself, Quit, and Play for games already installed. What
//! can do harm is refused here as well, whatever asks for it (the phone,
//! Remote): new downloads, uninstalling and moving a game (`refuse`).
//!
//! Never a brick:
//! - kryo.to out of reach: the last answer holds for at most 7 days, then
//!   there is no requirement until kryo.to answers again;
//! - an answer whose `min` is above `latest` (an update to a version that is
//!   not out, or a tampered answer) is ignored, as kryo.to itself does;
//! - a Preview build (`0.4.0-preview.12`) counts as the release it previews:
//!   its own channel only ever offers more previews.

use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview};

/// How long the last answer holds while kryo.to cannot be reached.
pub const KEEP_FOR_SECS: u64 = 7 * 24 * 3600;
const FILE: &str = "required.json";

/// kryo.to's answer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub min: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub latest: Option<String>,
}

/// The last answer and when it came (Unix seconds), kept on disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Kept {
    pub answer: Answer,
    pub at: u64,
}

/// What the shell is told.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Required {
    /// This copy is older than the oldest allowed: only updating, quitting and playing.
    pub blocked: bool,
    pub min: Option<String>,
    pub reason: Option<String>,
    pub latest: Option<String>,
    pub current: String,
    /// The answer is a kept one: kryo.to did not answer this time.
    pub offline: bool,
}

/// `0.4.0`, `v0.4.0`, `0.4.0-preview.12`: the numbers and the pre-release parts.
fn parse(value: &str) -> Option<([u64; 3], Vec<String>)> {
    let v = value.trim().trim_start_matches('v');
    let v = v.split('+').next()?;
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let nums: Vec<u64> = core.split('.').map(|n| if !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit()) { n.parse().ok() } else { None }).collect::<Option<_>>()?;
    let &[a, b, c] = nums.as_slice() else { return None };
    let pre: Vec<String> = match pre {
        Some(p) => {
            let parts: Vec<String> = p.split('.').map(String::from).collect();
            if parts.iter().any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
                return None;
            }
            parts
        }
        None => Vec::new(),
    };
    Some(([a, b, c], pre))
}

/// Semantic version order; `None` when either is not a version.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (x, xp) = parse(a)?;
    let (y, yp) = parse(b)?;
    let core = x.cmp(&y);
    if core != Ordering::Equal {
        return Some(core);
    }
    Some(match (xp.is_empty(), yp.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            for (p, q) in xp.iter().zip(yp.iter()) {
                let order = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => p.cmp(q),
                };
                if order != Ordering::Equal {
                    return Some(order);
                }
            }
            xp.len().cmp(&yp.len())
        }
    })
}

/// Whether `version` is older than `min`, a pre-release judged as the release it previews.
pub fn below(version: &str, min: &str) -> bool {
    match (parse(version), parse(min)) {
        (Some((v, _)), Some((m, _))) => v < m,
        _ => false,
    }
}

/// The minimum that applies: a plain release number, not above the newest release.
pub fn effective_min(answer: &Answer) -> Option<&str> {
    let min = answer.min.as_deref().map(str::trim).filter(|m| !m.is_empty())?;
    match parse(min) {
        Some((_, pre)) if pre.is_empty() && !min.starts_with('v') => {}
        _ => return None,
    }
    if let Some(latest) = answer.latest.as_deref() {
        if compare(min, latest) == Some(Ordering::Greater) {
            return None;
        }
    }
    Some(min)
}

/// What applies to `current`, from the answer kept at `kept.at`, at `now`.
pub fn decide(current: &str, kept: Option<&Kept>, now: u64, fresh: bool) -> Required {
    let base = Required { current: current.to_string(), offline: !fresh, ..Default::default() };
    // Too old to hold (offline a week), or from a clock that has since gone back a day.
    let Some(kept) = kept.filter(|k| now.saturating_sub(k.at) <= KEEP_FOR_SECS && k.at <= now + 86_400) else { return base };
    let answer = &kept.answer;
    let Some(min) = effective_min(answer) else { return Required { latest: answer.latest.clone(), ..base } };
    Required {
        blocked: below(current, min),
        min: Some(min.to_string()),
        reason: answer.reason.as_deref().map(|r| r.chars().filter(|c| !c.is_control()).take(300).collect::<String>()).filter(|r| !r.trim().is_empty()),
        latest: answer.latest.clone(),
        ..base
    }
}

static NOW: Mutex<Option<Required>> = Mutex::new(None);

/// For the commands that can do harm: refused while this copy must update.
pub fn refuse() -> Result<(), String> {
    refusal(NOW.lock().unwrap_or_else(|e| e.into_inner()).as_ref())
}

fn refusal(now: Option<&Required>) -> Result<(), String> {
    if now.is_some_and(|r| r.blocked) {
        return Err("Update Kryoto Desktop first: this version has a serious problem, so downloads, uninstalling and moving games wait until it is updated.".into());
    }
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(FILE))
}

fn load<R: Runtime>(app: &AppHandle<R>) -> Option<Kept> {
    serde_json::from_slice(&std::fs::read(path(app)?).ok()?).ok()
}

fn save<R: Runtime>(app: &AppHandle<R>, kept: &Kept) {
    let Some(file) = path(app) else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec(kept) {
        let tmp = file.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &file);
        }
    }
}

/// `GET /api/desktop/required`, the way every request to kryo.to goes from
/// here: past a network that blocks its name (net.rs), signed in or not.
async fn fetch<R: Runtime>(app: &AppHandle<R>) -> Result<Answer, String> {
    let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(app));
    let url = format!("{}/api/desktop/required", endpoint.trim_end_matches('/'));
    let client = crate::net::kryoto_tls(&url, crate::net::builder().user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION"))))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client.get(&url).send().await.map_err(|e| {
        let why = crate::logging::why(&e);
        crate::net::note_failure(&url, &why);
        why
    })?;
    if !res.status().is_success() {
        return Err(format!("kryo.to answered {}", res.status().as_u16()));
    }
    let answer: Answer = res.json().await.map_err(|e| e.to_string())?;
    // Only what could be a version and one short sentence is kept.
    let clip = |v: Option<String>, n: usize| v.map(|s| s.trim().chars().take(n).collect::<String>()).filter(|s| !s.is_empty());
    Ok(Answer { min: clip(answer.min, 32), reason: clip(answer.reason, 300), latest: clip(answer.latest, 32) })
}

/// Ask kryo.to now, keep the answer, and tell the shell when it changes.
pub async fn check<R: Runtime>(app: &AppHandle<R>) -> Required {
    let current = app.package_info().version.to_string();
    let now = now_secs();
    let (kept, fresh) = match fetch(app).await {
        Ok(answer) => {
            let kept = Kept { answer, at: now };
            save(app, &kept);
            (Some(kept), true)
        }
        Err(e) => {
            crate::logging::warn("required", &format!("could not ask kryo.to for the oldest version allowed: {e}"));
            (load(app), false)
        }
    };
    let required = decide(&current, kept.as_ref(), now, fresh);
    let changed = {
        let mut held = NOW.lock().unwrap_or_else(|e| e.into_inner());
        let changed = held.as_ref().map(|r| (r.blocked, &r.min)) != Some((required.blocked, &required.min));
        *held = Some(required.clone());
        changed
    };
    if changed {
        if required.blocked {
            crate::logging::warn("required", &format!("this version ({current}) is older than the oldest allowed ({}): only updating and playing", required.min.as_deref().unwrap_or("?")));
        }
        let _ = app.emit_to("main", "required-state", &required);
    }
    required
}

/// The shell asks at start and every few hours. Its own view only.
#[tauri::command]
pub async fn required_check(webview: Webview) -> Result<Required, String> {
    if webview.label() != "main" {
        return Err("Only the app itself asks.".into());
    }
    Ok(check(webview.app_handle()).await)
}

static STORE_ASKED: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// kryo.to's "Update Kryoto Desktop" page says this copy must update: ask
/// now rather than at the next check. At most once a minute; kryo.to only.
#[tauri::command]
pub fn store_required_check(app: AppHandle, webview: Webview) -> Result<(), String> {
    crate::from_kryoto_store(&app, &webview)?;
    {
        let mut asked = STORE_ASKED.lock().unwrap_or_else(|e| e.into_inner());
        if asked.is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
            return Ok(());
        }
        *asked = Some(std::time::Instant::now());
    }
    tauri::async_runtime::spawn(async move {
        check(&app).await;
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(min: Option<&str>, latest: Option<&str>, at: u64) -> Kept {
        Kept { answer: Answer { min: min.map(String::from), reason: Some("It can delete files.".into()), latest: latest.map(String::from) }, at }
    }

    #[test]
    fn orders_versions_as_numbers_with_pre_releases_first() {
        assert_eq!(compare("0.3.11", "0.4.0"), Some(Ordering::Less));
        assert_eq!(compare("0.3.9", "0.3.11"), Some(Ordering::Less));
        assert_eq!(compare("0.10.0", "0.9.9"), Some(Ordering::Greater));
        assert_eq!(compare("v0.4.0", "0.4.0"), Some(Ordering::Equal));
        assert_eq!(compare("0.4.0-preview.3", "0.4.0"), Some(Ordering::Less));
        assert_eq!(compare("0.4.0-preview.10", "0.4.0-preview.9"), Some(Ordering::Greater));
        assert_eq!(compare("0.4.0-alpha", "0.4.0-alpha.1"), Some(Ordering::Less));
        assert_eq!(compare("0.4.0-1", "0.4.0-alpha"), Some(Ordering::Less));
        assert_eq!(compare("0.4.0+5", "0.4.0"), Some(Ordering::Equal));
        for bad in ["", "0.4", "0.4.0.1", "x.y.z", "0.4.0-", "0.4.0-a..b"] {
            assert_eq!(compare(bad, "0.4.0"), None, "{bad}");
        }
    }

    #[test]
    fn a_preview_build_counts_as_the_release_it_previews() {
        assert!(below("0.3.11", "0.4.0"));
        assert!(!below("0.4.0", "0.4.0"));
        assert!(!below("0.4.1", "0.4.0"));
        assert!(!below("0.4.0-preview.2", "0.4.0"));
        assert!(below("0.3.12-preview.40", "0.4.0"));
        assert!(!below("nonsense", "0.4.0"));
    }

    #[test]
    fn blocks_only_a_version_older_than_the_minimum() {
        let now = 1_000_000;
        let k = kept(Some("0.4.0"), Some("0.4.0"), now);
        let r = decide("0.3.11", Some(&k), now, true);
        assert!(r.blocked);
        assert_eq!(r.min.as_deref(), Some("0.4.0"));
        assert_eq!(r.reason.as_deref(), Some("It can delete files."));
        assert!(!decide("0.4.0", Some(&k), now, true).blocked);
        assert!(!decide("0.5.0", Some(&k), now, true).blocked);
        assert!(!decide("0.4.0-preview.1", Some(&k), now, true).blocked);
        assert!(!decide("0.3.11", Some(&kept(None, Some("0.4.0"), now)), now, true).blocked);
        assert!(!decide("0.3.11", None, now, false).blocked);
    }

    #[test]
    fn ignores_a_minimum_above_the_newest_release_or_not_a_release() {
        let now = 1_000_000;
        assert!(!decide("0.3.11", Some(&kept(Some("0.4.0"), Some("0.3.11"), now)), now, true).blocked);
        assert!(!decide("0.3.11", Some(&kept(Some("99.0.0"), Some("0.4.0"), now)), now, true).blocked);
        assert!(!decide("0.3.11", Some(&kept(Some("0.4.0-rc.1"), Some("0.4.0"), now)), now, true).blocked);
        assert!(!decide("0.3.11", Some(&kept(Some("soon"), Some("0.4.0"), now)), now, true).blocked);
        // The newest unknown to kryo.to: the minimum stands, as on the site.
        assert!(decide("0.3.11", Some(&kept(Some("0.4.0"), None, now)), now, true).blocked);
    }

    #[test]
    fn offline_the_last_answer_holds_for_a_week_then_never_bricks() {
        let at = 1_000_000;
        let k = kept(Some("0.4.0"), Some("0.4.0"), at);
        let day = 86_400;
        let r = decide("0.3.11", Some(&k), at + 3 * day, false);
        assert!(r.blocked && r.offline);
        assert!(decide("0.3.11", Some(&k), at + 7 * day, false).blocked);
        assert!(!decide("0.3.11", Some(&k), at + 7 * day + 1, false).blocked);
        // A kept answer dated far in the future (the clock went back) is not trusted.
        assert!(!decide("0.3.11", Some(&k), at - 2 * day, false).blocked);
    }

    #[test]
    fn only_harmful_actions_are_refused_while_blocked() {
        assert!(refusal(Some(&Required { blocked: true, ..Default::default() })).is_err());
        assert!(refusal(Some(&Required { blocked: false, ..Default::default() })).is_ok());
        // Never asked yet (the first check still running): nothing is held up.
        assert!(refusal(None).is_ok());
    }

    #[test]
    fn reads_the_answer_kryo_to_sends() {
        let a: Answer = serde_json::from_str(r#"{"min":"0.4.0","reason":"x","latest":"0.4.1"}"#).unwrap();
        assert_eq!(effective_min(&a), Some("0.4.0"));
        let none: Answer = serde_json::from_str(r#"{"min":null,"reason":null,"latest":null}"#).unwrap();
        assert_eq!(effective_min(&none), None);
        let older_site: Answer = serde_json::from_str("{}").unwrap();
        assert_eq!(effective_min(&older_site), None);
    }
}
