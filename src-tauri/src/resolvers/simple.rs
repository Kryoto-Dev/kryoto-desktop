//! Hosts whose page names the file in one line of script.

use super::Resolved;
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

static FF_HOST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(^|\.)fuckingfast\.co$").unwrap());
static FF_LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"window\.open\("(https://fuckingfast\.co/dl/[^"]*)"\)"#).unwrap());

pub fn fuckingfast_matches(url: &str) -> bool {
    super::host_matches(url, &FF_HOST_RE)
}

pub async fn fuckingfast(url: &str) -> Result<Resolved, String> {
    let res = super::client(true).get(url).send().await.map_err(|e| format!("FuckingFast did not answer ({})", crate::logging::why(&e)))?;
    if !res.status().is_success() {
        return Err(format!("FuckingFast answered {}", res.status().as_u16()));
    }
    let html = res.text().await.unwrap_or_default();
    if html.contains("File Not Found Or Deleted") {
        return Err("the file is gone from FuckingFast".into());
    }
    let direct = FF_LINK_RE.captures(&html).map(|c| c[1].to_string()).ok_or("the FuckingFast page has no download")?;
    let file_name = url::Url::parse(&direct)
        .ok()
        .and_then(|u| u.fragment().filter(|f| !f.is_empty()).map(|f| percent_encoding::percent_decode_str(f).decode_utf8_lossy().to_string()))
        .or_else(|| super::last_segment(&direct));
    Ok(Resolved {
        url: direct,
        file_name,
        size: None,
        headers: HashMap::from([("User-Agent".into(), super::UA.into())]),
        connections: 4,
    })
}
