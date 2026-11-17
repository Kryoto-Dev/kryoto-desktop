//! Buzzheavier: the file page carries an htmx `hx-get` path with a token; a
//! request for it (sent the way htmx sends it) answers with the file's
//! address in `hx-redirect`. The page's cookies go along with the download.

use super::Resolved;
use regex::Regex;
use reqwest::cookie::CookieStore;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

static HOST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^|\.)(buzzheavier\.com|bzzhr\.(?:to|co))$").unwrap());
static ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/([A-Za-z0-9]{4,})").unwrap());
static TITLE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<title>([^<]+)</title>").unwrap());
static HXGET_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"hx-get="(/[A-Za-z0-9]+/download\?t=[^"]+)""#).unwrap());

pub fn matches(url: &str) -> bool {
    super::host_matches(url, &HOST_RE)
}

/// The download paths on the page, the primary one first (`alt=true` is the
/// page's slower fallback server).
fn download_paths(html: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in HXGET_RE.captures_iter(html) {
        let p = c[1].replace("&amp;", "&");
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out.sort_by_key(|p| p.contains("alt=true"));
    out
}

fn absolute(base: &str, value: &str) -> Option<String> {
    let url = url::Url::parse(base).ok()?.join(value).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

pub async fn resolve(url: &str) -> Result<Resolved, String> {
    let parsed = url::Url::parse(url).map_err(|_| "not a Buzzheavier link")?;
    if !ID_RE.is_match(parsed.path()) {
        return Err("not a Buzzheavier file link".into());
    }
    let jar = Arc::new(reqwest::cookie::Jar::default());
    let build = |redirects: bool| {
        let b = crate::net::builder()
            .user_agent(super::UA)
            .cookie_provider(jar.clone())
            .timeout(std::time::Duration::from_secs(30));
        let b = if redirects { b } else { b.redirect(reqwest::redirect::Policy::none()) };
        b.build().unwrap_or_default()
    };
    let page = build(true)
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .map_err(|e| format!("Buzzheavier did not answer ({})", crate::logging::why(&e)))?;
    let status = page.status().as_u16();
    if status == 404 {
        return Err("the file is gone from Buzzheavier".into());
    }
    if status != 200 {
        return Err(format!("Buzzheavier answered {status}"));
    }
    let page_url = page.url().to_string();
    let html = page.text().await.unwrap_or_default();
    let Some(path) = download_paths(&html).into_iter().next() else {
        return Err(if html.contains("Just a moment") || html.contains("cf_chl_opt") {
            "Buzzheavier is behind a Cloudflare check".into()
        } else {
            "the Buzzheavier page has no download".into()
        });
    };
    let origin = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or("buzzheavier.com"));
    let tokened = format!("{origin}{path}");
    let res = build(false)
        .get(&tokened)
        .header("Referer", &page_url)
        .header("hx-request", "true")
        .header("hx-current-url", &page_url)
        .send()
        .await
        .map_err(|e| format!("Buzzheavier did not answer ({})", crate::logging::why(&e)))?;
    let header = |name: &str| res.headers().get(name).and_then(|v| v.to_str().ok()).filter(|v| !v.is_empty()).map(str::to_string);
    let direct = header("hx-redirect")
        .or_else(|| header("location"))
        .and_then(|v| absolute(&tokened, &v))
        .ok_or_else(|| format!("Buzzheavier gave no file (answered {})", res.status().as_u16()))?;

    let mut headers = HashMap::from([("User-Agent".to_string(), super::UA.to_string()), ("Referer".to_string(), page_url.clone())]);
    if let Some(cookie) = url::Url::parse(&page_url).ok().and_then(|u| jar.cookies(&u)).and_then(|v| v.to_str().ok().map(str::to_string)) {
        headers.insert("Cookie".into(), cookie);
    }
    let file_name = TITLE_RE.captures(&html).map(|c| c[1].trim().to_string()).filter(|t| !t.is_empty());
    Ok(Resolved { url: direct, file_name, size: None, headers, connections: 8 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_download_path_comes_first() {
        let html = r#"<a hx-get="/AbCd/download?t=1&amp;alt=true">b</a><a hx-get="/AbCd/download?t=1">a</a>"#;
        assert_eq!(download_paths(html), vec!["/AbCd/download?t=1", "/AbCd/download?t=1&alt=true"]);
    }

    #[test]
    fn relative_redirects_join_and_scripts_do_not() {
        assert_eq!(absolute("https://bzzhr.co/A/download?t=1", "/d/f.7z").as_deref(), Some("https://bzzhr.co/d/f.7z"));
        assert!(absolute("https://bzzhr.co/A", "javascript:alert(1)").is_none());
    }
}
