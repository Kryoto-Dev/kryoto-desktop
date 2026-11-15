//! Gofile: a guest account from its API, then the folder's contents with the
//! website token its own page sends (a SHA-256 of the user agent, language,
//! account token and a four-hour window). The file downloads with the guest
//! token as a cookie.

use super::Resolved;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::LazyLock;
use tokio::sync::Mutex;

static HOST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(^|\.)gofile\.io$").unwrap());
static ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:/d/|/)([A-Za-z0-9]{4,})").unwrap());

const LANG: &str = "en-US";
const WT_SALT: &str = "9844d94d963d30";
const WT_WINDOW_SECS: u64 = 14_400;
/// A guest token is good for a long while; one per half day is plenty.
const TOKEN_TTL: std::time::Duration = std::time::Duration::from_secs(12 * 60 * 60);

static TOKEN: LazyLock<Mutex<Option<(String, std::time::Instant)>>> = LazyLock::new(|| Mutex::new(None));

pub fn matches(url: &str) -> bool {
    super::host_matches(url, &HOST_RE)
}

fn content_id(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if let Some(c) = ID_RE.captures(u.path()) {
        return Some(c[1].to_string());
    }
    u.query_pairs().find(|(k, _)| k == "c" || k == "id").map(|(_, v)| v.to_string()).filter(|v| v.len() >= 4)
}

fn wt_hash(ua: &str, lang: &str, token: &str, window: u64) -> String {
    let digest = Sha256::digest(format!("{ua}::{lang}::{token}::{window}::{WT_SALT}").as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn window() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / WT_WINDOW_SECS).unwrap_or(0)
}

async fn guest_token(client: &reqwest::Client) -> Result<String, String> {
    let mut cached = TOKEN.lock().await;
    if let Some((t, at)) = cached.as_ref() {
        if at.elapsed() < TOKEN_TTL {
            return Ok(t.clone());
        }
    }
    let res = client.post("https://api.gofile.io/accounts").send().await.map_err(|e| format!("Gofile did not answer ({})", crate::logging::why(&e)))?;
    let json: serde_json::Value = res.json().await.map_err(|_| "Gofile's answer was unreadable")?;
    let token = json["data"]["token"].as_str().ok_or("Gofile would not start a guest session")?.to_string();
    *cached = Some((token.clone(), std::time::Instant::now()));
    Ok(token)
}

/// The files in a contents answer: the node itself, or a folder's children,
/// sorted by name so parts come in order.
fn files(json: &serde_json::Value) -> Vec<(String, Option<String>, Option<u64>)> {
    let file = |n: &serde_json::Value| {
        (n["type"].as_str() == Some("file")).then(|| n["link"].as_str().map(|l| (l.to_string(), n["name"].as_str().map(str::to_string), n["size"].as_u64())))?
    };
    let data = &json["data"];
    if let Some(f) = file(data) {
        return vec![f];
    }
    let mut out: Vec<_> = data["children"].as_object().into_iter().flat_map(|c| c.values()).filter_map(file).collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

pub async fn resolve(url: &str) -> Result<Resolved, String> {
    let id = content_id(url).ok_or("not a Gofile link")?;
    let client = super::client(true);
    let token = guest_token(&client).await?;
    let res = client
        .get(format!("https://api.gofile.io/contents/{id}?page=1&pageSize=1000&sortField=createTime&sortDirection=-1"))
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Website-Token", wt_hash(super::UA, LANG, &token, window()))
        .header("X-BL", LANG)
        .send()
        .await
        .map_err(|e| format!("Gofile did not answer ({})", crate::logging::why(&e)))?;
    let json: serde_json::Value = res.json().await.map_err(|_| "Gofile's answer was unreadable")?;
    match json["status"].as_str() {
        Some("ok") => {}
        Some("error-notFound") => return Err("the file is gone from Gofile".into()),
        other => {
            // A stale token is the usual cause; the next try starts a new one.
            *TOKEN.lock().await = None;
            return Err(format!("Gofile said {}", other.unwrap_or("no")));
        }
    }
    let mut found = files(&json);
    if found.len() != 1 {
        return Err(if found.is_empty() {
            "the Gofile folder is empty".into()
        } else {
            format!("the Gofile folder holds {} files, not one archive", found.len())
        });
    }
    let (link, name, size) = found.remove(0);
    Ok(Resolved {
        url: link,
        file_name: name,
        size,
        headers: HashMap::from([
            ("Cookie".into(), format!("accountToken={token}")),
            ("User-Agent".into(), super::UA.into()),
        ]),
        // Guest downloads are capped per connection and throttled beyond a few.
        connections: 4,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn website_token_matches_the_reference() {
        assert_eq!(
            wt_hash("test-ua", "en-US", "TOKEN123", 100000),
            "6bf97fa87e76a4b9d8b9965bca1483888f57a76bd72a319909e9bb4d0c4ba61f"
        );
    }

    #[test]
    fn content_id_reads_folder_links() {
        assert_eq!(content_id("https://gofile.io/d/dc1V9W").as_deref(), Some("dc1V9W"));
        assert_eq!(content_id("https://gofile.io/"), None);
    }

    #[test]
    fn a_folder_lists_its_files_in_name_order() {
        let json = serde_json::json!({ "status": "ok", "data": { "type": "folder", "children": {
            "a": { "type": "file", "name": "b.7z", "link": "https://x/2", "size": 2 },
            "b": { "type": "file", "name": "a.7z", "link": "https://x/1", "size": 1 },
            "c": { "type": "folder" }
        }}});
        let f = files(&json);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].1.as_deref(), Some("a.7z"));
    }
}
