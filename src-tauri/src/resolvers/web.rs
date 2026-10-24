//! Any other web link. Asked once, like a browser would: when what comes back
//! is a file, that is the download; when it is a page (a GitHub release, a
//! developer's site), the page opens in the mirror window, shown, and the
//! download the person starts there is taken over (`browser.rs`).

use super::{browser, Resolved, UA};
use reqwest::header::{HeaderMap, CONTENT_DISPOSITION, CONTENT_RANGE, CONTENT_TYPE};
use std::collections::HashMap;
use tauri::{AppHandle, Runtime};

pub async fn resolve<R: Runtime>(app: &AppHandle<R>, url: &str, host: &str) -> Result<Resolved, String> {
    match file_at(url).await {
        Some(file) => Ok(file),
        None => browser::solve(app, url, host, browser::Mode::Open).await,
    }
}

/// The file at `url`, when it is one. A page, an error or no answer is `None`:
/// the page is opened instead, where whatever went wrong can be seen.
async fn file_at(url: &str) -> Option<Resolved> {
    let res = super::client(true).get(url).header("Range", "bytes=0-0").send().await.ok()?;
    if !res.status().is_success() || !is_file(res.headers()) {
        return None;
    }
    let headers = res.headers();
    let size = headers
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|total| total.trim().parse::<u64>().ok())
        .or_else(|| (res.status().as_u16() == 200).then(|| res.content_length()).flatten());
    let file_name = headers
        .get(CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(disposition_name)
        .or_else(|| super::last_segment(res.url().as_str()));
    Some(Resolved {
        // The link as given, not where it led: a redirect to a signed address
        // runs out, and the link is asked again when the download resumes.
        url: url.to_string(),
        file_name,
        size: size.filter(|s| *s > 0),
        headers: HashMap::from([("User-Agent".into(), UA.into())]),
        connections: 4,
    })
}

/// Whether an answer is a file rather than something for a browser to show.
fn is_file(headers: &HeaderMap) -> bool {
    let header = |name: reqwest::header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    if header(CONTENT_DISPOSITION).starts_with("attachment") {
        return true;
    }
    let kind = header(CONTENT_TYPE);
    let kind = kind.split(';').next().unwrap_or("").trim();
    // No type says nothing either way, and a page shows what it is.
    !(kind.is_empty()
        || kind.starts_with("text/")
        || matches!(kind, "application/json" | "application/xml" | "application/xhtml+xml" | "application/javascript"))
}

/// The file name a `Content-Disposition` gives, `filename*=` first.
fn disposition_name(value: &str) -> Option<String> {
    let mut plain = None;
    for part in value.split(';').map(str::trim) {
        let Some((key, val)) = part.split_once('=') else { continue };
        let val = val.trim().trim_matches('"');
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                let encoded = val.split_once("''").map_or(val, |(_, rest)| rest);
                let name = percent_encoding::percent_decode_str(encoded).decode_utf8_lossy().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
            "filename" if !val.is_empty() => plain = Some(val.to_string()),
            _ => {}
        }
    }
    plain
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(pairs: &[(reqwest::header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(k.clone(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn files_are_told_from_pages() {
        assert!(is_file(&headers(&[(CONTENT_TYPE, "application/octet-stream")])));
        assert!(is_file(&headers(&[(CONTENT_TYPE, "application/x-msdownload")])));
        assert!(is_file(&headers(&[(CONTENT_TYPE, "text/html"), (CONTENT_DISPOSITION, "attachment; filename=a.zip")])));
        assert!(!is_file(&headers(&[(CONTENT_TYPE, "text/html; charset=utf-8")])));
        assert!(!is_file(&headers(&[(CONTENT_TYPE, "application/json")])));
        assert!(!is_file(&headers(&[])));
    }

    #[test]
    fn disposition_names() {
        assert_eq!(disposition_name("attachment; filename=Setup.exe").as_deref(), Some("Setup.exe"));
        assert_eq!(disposition_name(r#"attachment; filename="My Game.zip""#).as_deref(), Some("My Game.zip"));
        assert_eq!(
            disposition_name("attachment; filename=\"x.zip\"; filename*=UTF-8''Caf%C3%A9.zip").as_deref(),
            Some("Café.zip")
        );
        assert_eq!(disposition_name("inline"), None);
    }
}
