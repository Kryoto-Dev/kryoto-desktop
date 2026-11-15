//! Pixeldrain: a public API. `/api/file/<id>/info` names the file and
//! `/api/file/<id>?download` is the file.

use super::Resolved;
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

static HOST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(^|\.)pixeldrain\.(com|net)$").unwrap());
static PATH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/(u|api/file)/([A-Za-z0-9_-]{4,40})").unwrap());

pub fn matches(url: &str) -> bool {
    super::host_matches(url, &HOST_RE)
}

fn file_id(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    PATH_RE.captures(u.path()).map(|c| c[2].to_string())
}

pub async fn resolve(url: &str) -> Result<Resolved, String> {
    let id = file_id(url).ok_or("not a Pixeldrain file link")?;
    let res = super::client(true)
        .get(format!("https://pixeldrain.com/api/file/{id}/info"))
        .send()
        .await
        .map_err(|e| format!("Pixeldrain did not answer ({})", crate::logging::why(&e)))?;
    match res.status().as_u16() {
        200 => {}
        404 => return Err("the file is gone from Pixeldrain".into()),
        s => return Err(format!("Pixeldrain answered {s}")),
    }
    let info: serde_json::Value = res.json().await.map_err(|_| "Pixeldrain's answer was unreadable")?;
    Ok(Resolved {
        url: format!("https://pixeldrain.com/api/file/{id}?download"),
        file_name: info["name"].as_str().map(str::to_string),
        size: super::num(info.get("size")),
        headers: HashMap::from([("User-Agent".into(), super::UA.into())]),
        // Pixeldrain rate-limits by IP; a few connections is plenty.
        connections: 4,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_id_from_page_and_api_links() {
        assert_eq!(file_id("https://pixeldrain.com/u/AbCd1234").as_deref(), Some("AbCd1234"));
        assert_eq!(file_id("https://pixeldrain.com/api/file/AbCd1234?download").as_deref(), Some("AbCd1234"));
        assert_eq!(file_id("https://pixeldrain.com/"), None);
    }
}
