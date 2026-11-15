//! MediaFire: the file page links the file directly (sometimes base64'd in
//! `data-scrambled-url`).

use super::Resolved;
use base64::Engine;
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

static HOST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(^|\.)mediafire\.com$").unwrap());
static DIRECT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https://download\d+\.mediafire\.com/[^"'\s<>\\]+"#).unwrap());
static SCRAMBLED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-scrambled-url="([^"]+)""#).unwrap());

pub fn matches(url: &str) -> bool {
    super::host_matches(url, &HOST_RE)
}

fn unscramble(cap: &str) -> Option<String> {
    let s = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(cap).ok()?).ok()?;
    if s.starts_with("http") {
        Some(s)
    } else {
        s.starts_with("//").then(|| format!("https:{s}"))
    }
}

fn direct_in(html: &str) -> Option<String> {
    DIRECT_RE
        .find(html)
        .map(|m| m.as_str().to_string())
        .or_else(|| SCRAMBLED_RE.captures(html).and_then(|c| unscramble(&c[1])))
}

pub async fn resolve(url: &str) -> Result<Resolved, String> {
    let res = super::client(true).get(url).send().await.map_err(|e| format!("MediaFire did not answer ({})", crate::logging::why(&e)))?;
    if !res.status().is_success() {
        return Err(format!("MediaFire answered {}", res.status().as_u16()));
    }
    let html = res.text().await.unwrap_or_default();
    let direct = direct_in(&html).ok_or("the MediaFire page has no download")?;
    Ok(Resolved {
        file_name: super::last_segment(&direct),
        url: direct,
        size: None,
        headers: HashMap::from([("User-Agent".into(), super::UA.into())]),
        connections: 8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_plain_and_scrambled_links() {
        assert_eq!(
            direct_in(r#"<a href="https://download123.mediafire.com/x/y/game.7z">"#).as_deref(),
            Some("https://download123.mediafire.com/x/y/game.7z")
        );
        let scrambled = base64::engine::general_purpose::STANDARD.encode("//download9.mediafire.com/a/b.7z");
        assert_eq!(
            direct_in(&format!(r#"data-scrambled-url="{scrambled}""#)).as_deref(),
            Some("https://download9.mediafire.com/a/b.7z")
        );
    }
}
