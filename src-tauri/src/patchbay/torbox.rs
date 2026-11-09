//! TorBox in front of mirror downloads, the second debrid service beside
//! Real-Debrid (suggestion #42). A host link TorBox supports becomes a web
//! download on the player's TorBox account, and once TorBox has the file its
//! direct link is what Kryoto downloads. Anything that goes wrong, or a file
//! TorBox has not fetched within a minute, falls through to the usual way
//! (resolvers/mod.rs), so it only ever makes a download faster.
//!
//! API: <https://api.torbox.app/v1/api> (`/webdl/hosters`, `/webdl/createwebdownload`,
//! `/webdl/mylist`, `/webdl/requestdl`, `/user/me`). Every answer is
//! `{ success, error, detail, data }`. The token goes to TorBox only.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::resolvers::{Resolved, UA};

const API: &str = "https://api.torbox.app/v1/api";
/// How long the list of supported hosts stands before asking again.
const HOSTERS_FOR: Duration = Duration::from_secs(12 * 3600);
/// How long to wait for TorBox to fetch a file it does not have yet.
const READY_WITHIN: Duration = Duration::from_secs(60);
const POLL_EVERY: Duration = Duration::from_secs(3);

static HOSTERS: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// One TorBox call: the body's `data` on success, its error in words otherwise.
async fn call(req: reqwest::RequestBuilder) -> Result<Value, String> {
    let res = req.send().await.map_err(|e| format!("TorBox could not be reached: {e}"))?;
    let status = res.status().as_u16();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    data_of(status, body)
}

pub fn data_of(status: u16, body: Value) -> Result<Value, String> {
    if (200..300).contains(&status) && body.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(body.get("data").cloned().unwrap_or(Value::Null));
    }
    Err(error_text(status, &body))
}

/// The hosts TorBox can fetch from right now, asked for twice a day. An old
/// list is better than none when it can't be reached.
async fn hosters(token: &str) -> Option<Vec<String>> {
    let cached = HOSTERS.lock().ok().and_then(|d| d.clone());
    if let Some((at, list)) = &cached {
        if at.elapsed() < HOSTERS_FOR {
            return Some(list.clone());
        }
    }
    let fresh = call(client().get(format!("{API}/webdl/hosters")).bearer_auth(token)).await.ok().map(|d| parse_hosters(&d));
    match fresh {
        Some(list) if !list.is_empty() => {
            if let Ok(mut slot) = HOSTERS.lock() {
                *slot = Some((Instant::now(), list.clone()));
            }
            Some(list)
        }
        _ => cached.map(|(_, list)| list),
    }
}

/// The domains of every hoster that is up.
pub fn parse_hosters(data: &Value) -> Vec<String> {
    let Some(list) = data.as_array() else { return Vec::new() };
    list.iter()
        .filter(|h| h.get("status").and_then(Value::as_bool).unwrap_or(true))
        .flat_map(|h| h.get("domains").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter_map(|d| d.as_str().map(|d| d.trim().to_ascii_lowercase()))
        .filter(|d| !d.is_empty())
        .collect()
}

/// Try TorBox first. `None` when no token is kept or it does not support
/// this host: the usual resolvers take it.
pub async fn try_resolve(url: &str) -> Option<Result<Resolved, String>> {
    let token = super::token(super::Slot::Torbox)?;
    if !super::debrid::supports(&hosters(&token).await?, url) {
        return None;
    }
    Some(fetch(&token, url).await)
}

async fn fetch(token: &str, link: &str) -> Result<Resolved, String> {
    let created = call(client().post(format!("{API}/webdl/createwebdownload")).bearer_auth(token).form(&[("link", link)])).await?;
    let id = parse_created(&created)?;
    let until = Instant::now() + READY_WITHIN;
    let file = loop {
        let item = call(client().get(format!("{API}/webdl/mylist?id={id}&bypass_cache=true")).bearer_auth(token)).await?;
        if let Some(file) = parse_ready(&item)? {
            break file;
        }
        if Instant::now() >= until {
            return Err("TorBox is still fetching the file.".into());
        }
        tokio::time::sleep(POLL_EVERY).await;
    };
    // `requestdl` takes the token in the address, as TorBox's API asks; it is
    // never logged, and the link it answers with carries no token.
    let q: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("token", token)
        .append_pair("web_id", &id.to_string())
        .append_pair("file_id", &file.id.to_string())
        .finish();
    let link = call(client().get(format!("{API}/webdl/requestdl?{q}"))).await?;
    Ok(Resolved {
        url: parse_link(&link)?,
        file_name: file.name,
        size: file.size,
        headers: HashMap::from([("User-Agent".into(), UA.into())]),
        connections: 8,
    })
}

pub fn parse_created(data: &Value) -> Result<u64, String> {
    data.get("webdownload_id").or_else(|| data.get("id")).and_then(Value::as_u64).ok_or_else(|| "TorBox did not take the link.".into())
}

#[derive(Debug, PartialEq)]
pub struct File {
    pub id: u64,
    pub name: Option<String>,
    pub size: Option<u64>,
}

/// The web download's biggest file once TorBox has it all, `None` while it
/// is still fetching. A failed fetch is an error.
pub fn parse_ready(item: &Value) -> Result<Option<File>, String> {
    let state = item.get("download_state").and_then(Value::as_str).unwrap_or("");
    if state.starts_with("failed") || state == "error" {
        return Err(format!("TorBox could not fetch the file ({state})."));
    }
    let done = item.get("download_finished").and_then(Value::as_bool).unwrap_or(false) || item.get("download_present").and_then(Value::as_bool).unwrap_or(false);
    if !done {
        return Ok(None);
    }
    let files = item.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
    let best = files.iter().filter(|f| f.get("id").and_then(Value::as_u64).is_some()).max_by_key(|f| f.get("size").and_then(Value::as_u64).unwrap_or(0));
    let Some(f) = best else { return Err("TorBox fetched nothing it can hand over.".into()) };
    Ok(Some(File {
        id: f.get("id").and_then(Value::as_u64).unwrap_or(0),
        name: f.get("short_name").or_else(|| f.get("name")).and_then(Value::as_str).map(|n| n.rsplit('/').next().unwrap_or(n).to_string()).filter(|n| !n.is_empty()),
        size: f.get("size").and_then(Value::as_u64),
    }))
}

pub fn parse_link(data: &Value) -> Result<String, String> {
    data.as_str().filter(|u| u.starts_with("https://")).map(str::to_string).ok_or_else(|| "TorBox gave no download link.".into())
}

/// TorBox's error codes, in words a player can act on.
pub fn error_text(status: u16, body: &Value) -> String {
    match body.get("error").and_then(Value::as_str).unwrap_or("") {
        "BAD_TOKEN" | "AUTH_ERROR" | "NO_AUTH" => "TorBox refused the token. Paste a new one in Settings > Patchbay.".into(),
        "PLAN_RESTRICTED_FEATURE" => "Your TorBox plan does not include host downloads.".into(),
        "ACTIVE_LIMIT" => "Too many downloads running on TorBox at once.".into(),
        "MONTHLY_LIMIT" | "COOLDOWN_LIMIT" => "Your TorBox download limit is used up for now.".into(),
        "DOWNLOAD_TOO_LARGE" => "The file is too big for your TorBox plan.".into(),
        "UNSUPPORTED_SITE" => "TorBox does not support this host.".into(),
        "ITEM_NOT_FOUND" => "The file is gone from the host.".into(),
        _ if status == 401 || status == 403 => "TorBox refused the token. Paste a new one in Settings > Patchbay.".into(),
        _ if status == 429 => "TorBox asked to slow down. Try again in a minute.".into(),
        other => {
            let why = body
                .get("detail")
                .and_then(Value::as_str)
                .filter(|d| !d.is_empty())
                .map(str::to_string)
                .or_else(|| (!other.is_empty()).then(|| other.to_string()))
                .unwrap_or_else(|| format!("HTTP {status}"));
            format!("TorBox could not fetch the link ({why}).")
        }
    }
}

/// `/user/me`, in a sentence for the Test button.
pub fn describe(user: &Value) -> String {
    let who = user.get("email").and_then(Value::as_str).unwrap_or("your account");
    let plan = user.get("plan").and_then(Value::as_u64).unwrap_or(0);
    let until = user.get("premium_expires_at").and_then(Value::as_str).and_then(|d| d.get(..10)).filter(|d| !d.is_empty());
    match (plan, until) {
        (0, _) => format!("Signed in as {who}: free plan, so host downloads will not work."),
        (_, Some(day)) => format!("Signed in as {who}: paid plan until {day}."),
        _ => format!("Signed in as {who}: paid plan."),
    }
}

pub async fn account(token: &str) -> Result<String, String> {
    let user = call(client().get(format!("{API}/user/me")).bearer_auth(token)).await?;
    Ok(describe(&user))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_hosters_that_are_up_count() {
        let d = parse_hosters(&json!([
            { "name": "1fichier", "domains": ["1fichier.com", "Alterupload.com"], "status": true },
            { "name": "down", "domains": ["down.example"], "status": false },
            { "name": "mediafire", "domains": ["mediafire.com"] }
        ]));
        assert_eq!(d, vec!["1fichier.com", "alterupload.com", "mediafire.com"]);
        assert!(super::super::debrid::supports(&d, "https://www.mediafire.com/file/x"));
        assert!(!super::super::debrid::supports(&d, "https://down.example/x"));
        assert!(parse_hosters(&json!(null)).is_empty());
    }

    #[test]
    fn answers_unwrap_or_say_what_to_do() {
        assert_eq!(data_of(200, json!({ "success": true, "data": { "webdownload_id": 7 } })).unwrap()["webdownload_id"], 7);
        let bad = data_of(403, json!({ "success": false, "error": "BAD_TOKEN", "detail": "Invalid token" })).unwrap_err();
        assert!(bad.contains("Settings > Patchbay"));
        assert!(data_of(200, json!({ "success": false, "error": "ACTIVE_LIMIT" })).unwrap_err().contains("at once"));
        assert!(data_of(500, Value::Null).unwrap_err().contains("HTTP 500"));
        assert!(error_text(400, &json!({ "error": "WEIRD", "detail": "Something odd." })).contains("Something odd."));
    }

    #[test]
    fn a_web_download_is_ready_once_finished() {
        assert_eq!(parse_created(&json!({ "webdownload_id": 42, "hash": "h", "auth_id": "a" })), Ok(42));
        assert!(parse_created(&json!({})).is_err());
        assert_eq!(parse_ready(&json!({ "download_state": "downloading", "download_finished": false, "files": [] })), Ok(None));
        let ready = parse_ready(&json!({
            "download_state": "completed", "download_finished": true,
            "files": [
                { "id": 1, "name": "Game/readme.txt", "short_name": "readme.txt", "size": 10 },
                { "id": 2, "name": "Game/Game - kryoto.7z", "short_name": "Game - kryoto.7z", "size": 1234567 }
            ]
        }))
        .unwrap()
        .unwrap();
        assert_eq!(ready, File { id: 2, name: Some("Game - kryoto.7z".into()), size: Some(1234567) });
        assert!(parse_ready(&json!({ "download_state": "failed (host offline)" })).is_err());
        assert!(parse_ready(&json!({ "download_finished": true, "files": [] })).is_err());
    }

    #[test]
    fn only_https_links_are_downloaded() {
        assert_eq!(parse_link(&json!("https://store-1.tb-cdn.st/zip/abc")).unwrap(), "https://store-1.tb-cdn.st/zip/abc");
        assert!(parse_link(&json!("http://plain.example/x")).is_err());
        assert!(parse_link(&json!(null)).is_err());
    }

    #[test]
    fn the_test_button_says_the_plan() {
        assert!(describe(&json!({ "email": "p@x", "plan": 0 })).contains("free plan"));
        assert!(describe(&json!({ "email": "p@x", "plan": 2, "premium_expires_at": "2027-01-31T00:00:00Z" })).contains("until 2027-01-31"));
        assert!(describe(&json!({ "plan": 1 })).contains("paid plan."));
    }
}
