//! Game art, kept on this PC.
//!
//! Every image the shell draws for a game (covers, banners, logos) is asked
//! for as `kimg://localhost/<url>` (`http://kimg.localhost/<url>` on Windows).
//! The first time, it is fetched and written to the cache folder; after that
//! it comes from disk, so the Library, a game's page and Downloads look the
//! same with no connection at all. A failure is logged once per address, with
//! the address and why, instead of an image that quietly never appears.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};
use tauri::http::{Request, Response, StatusCode};
use tauri::{AppHandle, Manager, Runtime};
use crate::managed_art::{managed_key, mime, MAX_BYTES};

pub const SCHEME: &str = "kimg";

fn custom_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("custom-art"))
}

/// Copy a selected raster picture into durable app data. The original is untouched.
#[tauri::command(async)]
pub fn art_import(app: AppHandle, path: String) -> Result<String, String> {
    let bytes = crate::managed_art::read_image(std::path::Path::new(&path))?;
    let hash = Sha256::digest(&bytes);
    let key: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    crate::managed_art::store(&custom_dir(&app)?, &bytes, &key)
}

fn dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let d = app.path().app_cache_dir().ok()?.join("art");
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn key(url: &str) -> String {
    let hash = Sha256::digest(url.as_bytes());
    hash.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// The remote address in a `kimg` request: everything after the host,
/// percent-decoded.
fn target(request: &Request<Vec<u8>>) -> Option<url::Url> {
    let raw = request.uri().path().trim_start_matches('/');
    let decoded = percent_decode(raw)?;
    let url = url::Url::parse(&decoded).ok()?;
    (matches!(url.scheme(), "http" | "https") || managed_key(&decoded).is_some()).then_some(url)
}

fn percent_decode(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default()
    })
}

/// Log a failed address once per run.
///
/// A warning, not a report to kryo.to. Art that is missing (Steam moved a
/// game's images to hashed addresses and the old ones answer 404) or that
/// could not be fetched (no connection) is expected, has a fallback in every
/// place that draws it, and made up most of what the reports carried - so the
/// failures worth reading were buried under thousands of these.
fn report(url: &str, why: &str) {
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let Ok(mut seen) = SEEN.lock() else { return };
    if seen.get_or_insert_with(HashSet::new).insert(url.to_string()) {
        crate::logging::warn("art", &format!("{url}: {why}"));
    }
}

/// `(appid, file)` for a legacy Steam CDN address
/// (`.../steam/apps/<appid>/header.jpg`), the shape that answers 404 for games
/// whose art Steam now keeps only at hashed addresses.
fn legacy_steam(url: &str) -> Option<(String, String)> {
    let re = regex::Regex::new(r"steamstatic\.com/steam/apps/(\d+)/([a-z0-9_]+\.jpg)").ok()?;
    let caps = re.captures(url)?;
    Some((caps[1].to_string(), caps[2].to_string()))
}

/// Where Steam keeps that image now, from its own store API. Only the header
/// and capsule have a field there; anything else stays missing, and the
/// window falls back to the next picture it was given.
async fn current_steam_address(appid: &str, file: &str) -> Option<String> {
    let field = match file {
        "header.jpg" => "header_image",
        "capsule_231x87.jpg" | "capsule_sm_120.jpg" => "capsule_image",
        "capsule_616x353.jpg" => "capsule_imagev5",
        _ => return None,
    };
    let res = client()
        .get(format!("https://kryo.to/api/steam/art?appid={appid}"))
        .send()
        .await
        .ok()?;
    let json: serde_json::Value = res.json().await.ok()?;
    let url = json.get(field)?.as_str()?.to_string();
    (url.starts_with("https://") && legacy_steam(&url).is_none()).then_some(url)
}

fn respond(status: StatusCode, bytes: Vec<u8>) -> Response<Vec<u8>> {
    let mut builder = Response::builder().status(status).header("Access-Control-Allow-Origin", "*");
    if status.is_success() {
        builder = builder.header("Content-Type", mime(&bytes)).header("Cache-Control", "max-age=31536000, immutable");
    }
    builder.body(bytes).unwrap_or_default()
}

/// Answer one `kimg` request: from disk, else from the network (and keep it).
pub async fn serve<R: Runtime>(app: AppHandle<R>, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let Some(url) = target(&request) else {
        return respond(StatusCode::BAD_REQUEST, Vec::new());
    };
    if let Some(key) = managed_key(url.as_str()) {
        return match custom_dir(&app).and_then(|d| crate::managed_art::read_image(&d.join(key))) {
            Ok(bytes) => respond(StatusCode::OK, bytes),
            Err(_) => respond(StatusCode::NOT_FOUND, Vec::new()),
        };
    }
    let file = dir(&app).map(|d| d.join(key(url.as_str())));
    if let Some(bytes) = file.as_ref().and_then(|f| std::fs::read(f).ok()).filter(|b| !b.is_empty()) {
        return respond(StatusCode::OK, bytes);
    }
    let mut fetched = fetch(url.as_str()).await;
    if matches!(&fetched, Err(why) if why.contains("404")) {
        if let Some((appid, file)) = legacy_steam(url.as_str()) {
            if let Some(now) = current_steam_address(&appid, &file).await {
                // Kept under the OLD address's key, so the next ask is a disk hit.
                fetched = fetch(&now).await;
            }
        }
    }
    match fetched {
        Ok(bytes) => {
            if let Some(f) = &file {
                let tmp = f.with_extension("part");
                if std::fs::write(&tmp, &bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, f);
                }
            }
            respond(StatusCode::OK, bytes)
        }
        Err(why) => {
            report(url.as_str(), &why);
            respond(StatusCode::NOT_FOUND, Vec::new())
        }
    }
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let res = client().get(network_image_url(url)).send().await.map_err(|e| {
        if e.is_connect() || e.is_timeout() {
            "no connection".to_string()
        } else {
            e.to_string()
        }
    })?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("answered {status}"));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(format!("{} bytes", bytes.len()));
    }
    if mime(&bytes) == "application/octet-stream" {
        return Err("not an image".into());
    }
    Ok(bytes.to_vec())
}

/// The cache keeps the original URL as its key; network loads go through Kryoto.
fn network_image_url(raw: &str) -> String {
    static HOSTS: OnceLock<serde_json::Value> = OnceLock::new();
    let hosts = HOSTS.get_or_init(|| serde_json::from_str(include_str!("../../src/lib/media-hosts.json")).expect("media host policy"));
    let Ok(mut url) = url::Url::parse(raw) else { return raw.into() };
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return raw.into();
    }
    let host = url.host_str().unwrap_or_default();
    let exact = ["steamHosts", "mediaHosts"].iter().any(|key| {
        hosts[*key].as_array().is_some_and(|list| list.iter().any(|v| v.as_str() == Some(host)))
    });
    let domain = ["steamDomains", "mediaDomains"].iter().any(|key| {
        hosts[*key].as_array().is_some_and(|list| list.iter().filter_map(|v| v.as_str()).any(|d| host == d || host.ends_with(&format!(".{d}"))))
    });
    if !exact && !domain { return raw.into() }
    let _ = url.set_scheme("https");
    url.set_fragment(None);
    let mut proxy = url::Url::parse("https://kryo.to/api/steam/image.jpg").expect("proxy URL");
    proxy.query_pairs_mut().append_pair("url", url.as_str());
    proxy.to_string()
}

/// Fetch a game's art into the cache ahead of time (when it is installed), so
/// it is there offline even if its page was never opened.
pub fn prefetch<R: Runtime>(app: &AppHandle<R>, urls: Vec<String>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(d) = dir(&app) else { return };
        for url in urls.into_iter().filter(|u| u.starts_with("http")) {
            let f = d.join(key(&url));
            if f.is_file() {
                continue;
            }
            match fetch(&url).await {
                Ok(bytes) => {
                    let _ = std::fs::write(&f, bytes);
                }
                Err(why) => report(&url, &why),
            }
        }
    });
}

/// Keep the cache under a size: oldest files go first. Run once at start.
pub fn prune<R: Runtime>(app: &AppHandle<R>, max_bytes: u64) {
    let Some(d) = dir(app) else { return };
    let Ok(read) = std::fs::read_dir(&d) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = read
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_name_their_remote_address() {
        let req = Request::builder()
            .uri("kimg://localhost/https%3A%2F%2Fshared.akamai.steamstatic.com%2Fa%2Fheader.jpg%3Ft%3D1")
            .body(Vec::new())
            .unwrap();
        assert_eq!(target(&req).unwrap().as_str(), "https://shared.akamai.steamstatic.com/a/header.jpg?t=1");
        let bad = Request::builder().uri("kimg://localhost/file%3A%2F%2F%2Fetc%2Fpasswd").body(Vec::new()).unwrap();
        assert!(target(&bad).is_none());
    }

    #[test]
    fn images_are_known_by_their_bytes() {
        assert_eq!(mime(&[0xFF, 0xD8, 0xFF]), "image/jpeg");
        assert_eq!(mime(b"\x89PNG\r\n"), "image/png");
        assert_eq!(mime(b"<html>"), "application/octet-stream");
    }

    #[test]
    fn blocked_media_uses_kryoto_even_when_prefetched() {
        for raw in [
            "https://shared.fastly.steamstatic.com/community_assets/images/apps/620/icon.jpg",
            "https://cdn.discordapp.com/avatars/123/a.png?size=256",
            "https://media2.giphy.com/media/abc/giphy.webp",
        ] {
            let proxy = url::Url::parse(&network_image_url(raw)).unwrap();
            assert_eq!(proxy.host_str(), Some("kryo.to"));
            assert_eq!(proxy.query_pairs().find(|(k, _)| k == "url").unwrap().1, raw);
        }
        for raw in ["https://dl.kryo.to/d/file", "https://steamstatic.com.evil.test/a.jpg", "http://127.0.0.1/a.jpg"] {
            assert_eq!(network_image_url(raw), raw);
        }
    }
}
