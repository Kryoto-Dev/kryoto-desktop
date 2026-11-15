//! Mocha: our own files, straight from Mocha's storage.
//!
//! Mocha's share page wants a Turnstile check for every download. For files
//! kryo.to owns, dl.kryo.to asks Mocha's API for a presigned link instead
//! (`/mirror/mocha/<share>`, which resolves the share and signs it with our
//! key), so the download starts with no page and no check. A share that is not
//! ours, or dl.kryo.to not answering, falls back to Mocha's page.

use super::Resolved;
use std::collections::HashMap;

const HOTLINK: &str = "https://dl.kryo.to/mirror/mocha/";

/// The token in `https://mocha.my/share/<token>`.
pub fn share_token(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    if host != "mocha.my" && host != "www.mocha.my" {
        return None;
    }
    let mut parts = u.path_segments()?.filter(|s| !s.is_empty());
    (parts.next()? == "share").then_some(())?;
    let token = parts.next()?;
    (token.len() >= 8 && token.len() <= 80 && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .then(|| token.to_string())
}

pub async fn hotlink(url: &str) -> Result<Resolved, String> {
    let token = share_token(url).ok_or("not a Mocha share link")?;
    let res = super::client(true)
        .get(format!("{HOTLINK}{token}"))
        .send()
        .await
        .map_err(|e| format!("dl.kryo.to did not answer ({})", crate::logging::why(&e)))?;
    let status = res.status().as_u16();
    let json: serde_json::Value = res.json().await.unwrap_or_default();
    if status != 200 {
        return Err(format!("no direct link ({})", json["error"].as_str().unwrap_or("error")));
    }
    let link = json["url"].as_str().filter(|u| u.starts_with("https://")).ok_or("no direct link in the answer")?;
    Ok(Resolved {
        url: link.to_string(),
        file_name: json["name"].as_str().map(str::to_string),
        size: None,
        headers: HashMap::from([("User-Agent".into(), super::UA.into())]),
        // Mocha's own storage: the same as its CDN downloads, many connections.
        connections: 8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_share_tokens_only_from_mocha() {
        assert_eq!(share_token("https://mocha.my/share/pvpgaoYk-6So_SEvo").as_deref(), Some("pvpgaoYk-6So_SEvo"));
        assert_eq!(share_token("https://www.mocha.my/share/pvpgaoYk-6So_SEvo/").as_deref(), Some("pvpgaoYk-6So_SEvo"));
        assert_eq!(share_token("https://mocha.my/files"), None);
        assert_eq!(share_token("https://evil.example/share/pvpgaoYk-6So_SEvo"), None);
    }
}
