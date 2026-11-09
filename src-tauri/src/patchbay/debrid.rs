//! Real-Debrid in front of mirror downloads: with a token in Settings >
//! Patchbay, a link on a host Real-Debrid supports is turned into its own
//! fast direct link first, before any resolver or hidden page. Anything that
//! goes wrong falls through to the usual way (resolvers/mod.rs), so a debrid
//! outage only makes a download slower, never stops it.
//!
//! API: <https://api.real-debrid.com/> (`/unrestrict/link`, `/user`, and the
//! public `/hosts/domains`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{AppHandle, Runtime};

use crate::resolvers::{domain_match, hostname, Resolved, UA};

const API: &str = "https://api.real-debrid.com/rest/1.0";
/// How long the list of supported hosts stands before asking again.
const DOMAINS_FOR: Duration = Duration::from_secs(12 * 3600);

static DOMAINS: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// The hosts Real-Debrid supports, asked for twice a day. An old list is
/// better than none when it can't be reached.
async fn domains() -> Option<Vec<String>> {
    let cached = DOMAINS.lock().ok().and_then(|d| d.clone());
    if let Some((at, list)) = &cached {
        if at.elapsed() < DOMAINS_FOR {
            return Some(list.clone());
        }
    }
    let fresh: Option<Vec<String>> = async { client().get(format!("{API}/hosts/domains")).send().await.ok()?.error_for_status().ok()?.json().await.ok() }.await;
    match fresh {
        Some(list) if !list.is_empty() => {
            let list: Vec<String> = list.into_iter().map(|d| d.to_ascii_lowercase()).collect();
            if let Ok(mut slot) = DOMAINS.lock() {
                *slot = Some((Instant::now(), list.clone()));
            }
            Some(list)
        }
        _ => cached.map(|(_, list)| list),
    }
}

pub fn supports(domains: &[String], url: &str) -> bool {
    let host = hostname(url);
    !host.is_empty() && domains.iter().any(|d| domain_match(&host, d))
}

/// Try the debrid service Settings picked (Real-Debrid, or TorBox in
/// torbox.rs), with its name for the log. `None` when none is set up or it
/// does not support this host: the usual resolvers take it.
pub async fn try_resolve<R: Runtime>(app: &AppHandle<R>, url: &str) -> Option<(&'static str, Result<Resolved, String>)> {
    match crate::settings::load(app).debrid.as_str() {
        "real-debrid" => {
            let token = super::token(super::Slot::RealDebrid)?;
            if !supports(&domains().await?, url) {
                return None;
            }
            Some(("Real-Debrid", unrestrict(&token, url).await))
        }
        "torbox" => super::torbox::try_resolve(url).await.map(|r| ("TorBox", r)),
        _ => None,
    }
}

async fn unrestrict(token: &str, link: &str) -> Result<Resolved, String> {
    let res = client()
        .post(format!("{API}/unrestrict/link"))
        .bearer_auth(token)
        .form(&[("link", link)])
        .send()
        .await
        .map_err(|e| format!("Real-Debrid could not be reached: {e}"))?;
    let status = res.status().as_u16();
    let body: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
    if !(200..300).contains(&status) {
        return Err(error_text(status, &body));
    }
    parse_unrestricted(&body)
}

pub fn parse_unrestricted(body: &serde_json::Value) -> Result<Resolved, String> {
    let url = body.get("download").and_then(|v| v.as_str()).filter(|u| u.starts_with("https://")).ok_or("Real-Debrid gave no download link.")?;
    Ok(Resolved {
        url: url.to_string(),
        file_name: body.get("filename").and_then(|v| v.as_str()).map(str::to_string).filter(|n| !n.is_empty()),
        size: crate::resolvers::num(body.get("filesize")),
        headers: HashMap::from([("User-Agent".into(), UA.into())]),
        // Real-Debrid's download servers take many connections at once.
        connections: 16,
    })
}

/// Real-Debrid's error codes, in words a player can act on.
pub fn error_text(status: u16, body: &serde_json::Value) -> String {
    let code = body.get("error_code").and_then(|c| c.as_i64());
    match code {
        Some(8) => "Real-Debrid refused the token. Paste a new one in Settings > Patchbay.".into(),
        Some(9 | 14) => "Real-Debrid says your account is locked or not allowed to do that.".into(),
        Some(16) => "Real-Debrid does not support this host.".into(),
        Some(17 | 19) => "This host is down at Real-Debrid for now.".into(),
        Some(18 | 23 | 36) => "Your Real-Debrid traffic for this host is used up for today.".into(),
        Some(20) => "This host needs Real-Debrid premium.".into(),
        Some(21) => "Too many downloads running on Real-Debrid at once.".into(),
        Some(22) => "Real-Debrid does not allow downloads from this network (VPN or server address).".into(),
        Some(24) => "The file is gone from the host.".into(),
        Some(5 | 34) => "Real-Debrid asked to slow down. Try again in a minute.".into(),
        Some(35) => "Real-Debrid will not unlock this file.".into(),
        _ if status == 401 => "Real-Debrid refused the token. Paste a new one in Settings > Patchbay.".into(),
        _ => format!(
            "Real-Debrid could not unlock the link ({}).",
            body.get("error").and_then(|e| e.as_str()).unwrap_or(&format!("HTTP {status}"))
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct Account {
    pub username: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Seconds of premium left.
    #[serde(default)]
    pub premium: u64,
}

impl Account {
    pub fn describe(&self) -> String {
        if self.kind == "premium" && self.premium > 0 {
            let days = self.premium.div_ceil(86_400);
            format!("Signed in as {}: premium, {days} day{} left.", self.username, if days == 1 { "" } else { "s" })
        } else {
            format!("Signed in as {}: not premium, so most hosts will not unlock.", self.username)
        }
    }
}

pub async fn account(token: &str) -> Result<Account, String> {
    let res = client().get(format!("{API}/user")).bearer_auth(token).send().await.map_err(|e| format!("Real-Debrid could not be reached: {e}"))?;
    let status = res.status().as_u16();
    let body: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
    if !(200..300).contains(&status) {
        return Err(error_text(status, &body));
    }
    serde_json::from_value(body).map_err(|e| format!("Real-Debrid answered oddly: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unrestricted_link_becomes_a_download() {
        let r = parse_unrestricted(&json!({
            "id": "X", "filename": "Game - kryoto.7z", "filesize": 1234567, "link": "https://1fichier.com/?abc",
            "host": "1fichier.com", "download": "https://abc.download.real-debrid.com/d/X/Game%20-%20kryoto.7z"
        }))
        .unwrap();
        assert!(r.url.starts_with("https://abc.download.real-debrid.com/"));
        assert_eq!(r.file_name.as_deref(), Some("Game - kryoto.7z"));
        assert_eq!(r.size, Some(1234567));
        assert!(parse_unrestricted(&json!({ "download": "http://plain.example/x" })).is_err());
        assert!(parse_unrestricted(&json!({})).is_err());
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(error_text(401, &json!({ "error": "bad_token", "error_code": 8 })).contains("Settings > Patchbay"));
        assert!(error_text(503, &json!({ "error_code": 19 })).contains("down"));
        assert!(error_text(500, &serde_json::Value::Null).contains("HTTP 500"));
    }

    #[test]
    fn only_supported_hosts_go_through_it() {
        let d = vec!["1fichier.com".to_string(), "mediafire.com".to_string()];
        assert!(supports(&d, "https://1fichier.com/?abc"));
        assert!(supports(&d, "https://www.mediafire.com/file/x"));
        assert!(!supports(&d, "https://notmediafire.com/file/x"));
        assert!(!supports(&d, "https://pixeldrain.com/u/x"));
    }

    #[test]
    fn premium_days_round_up() {
        let a = Account { username: "p".into(), kind: "premium".into(), premium: 86_401 };
        assert!(a.describe().contains("2 days"));
        let f = Account { username: "p".into(), kind: "free".into(), premium: 0 };
        assert!(f.describe().contains("not premium"));
    }
}
