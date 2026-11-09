//! filez.stream: an encrypted file host. A link is `https://filez.stream/<id>#<key>`;
//! the key is in the fragment, which no browser sends to a server, and the
//! host only ever holds ciphertext. Its own page decrypts in the browser.
//!
//! Here the same happens natively, by the host's own published format
//! (`shared/format.ts`, `src/lib/crypto.ts` in its repo):
//!
//! * the plaintext is cut into 1 MiB chunks, each sealed on its own with
//!   AES-256-GCM (16-byte tag), so any chunk can be fetched and opened alone,
//!   which is what makes the download parallel and resumable by chunk;
//! * the content and metadata keys are HKDF-SHA256 of the 32-byte link key,
//!   salt `filez.stream/v1`, info `content` / `meta`;
//! * chunk `i`'s nonce is four zero bytes then `i` as a big-endian u64, and
//!   its additional data is `[1]` for the last chunk and `[0]` otherwise, so a
//!   file cut short fails to open instead of ending early;
//! * `encMeta` is base64url(iv12 || ciphertext) of `{"n":name,"t":type,"s":size}`.
//!
//! Getting the ciphertext is the host's own way too: `GET /api/files/:id`
//! for the size and sealed name, `POST /api/grant` for permission to fetch
//! each stretch (its daily allowance and slow lane), and ranged
//! `GET /api/files/:id/data`. The data route wants the host's verification
//! (a Turnstile pass, kept as a cookie for a day): that is passed in its own
//! page, in Kryoto's page window (browser.rs `filez_access`), never skipped.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, Runtime};

pub const CHUNK: u64 = 1024 * 1024;
pub const TAG: u64 = 16;
pub const ENC_CHUNK: u64 = CHUNK + TAG;

/// What a download of one filez file needs past its address: the content key,
/// the sizes, and the host's verification. Only in memory, never written to
/// disk: the key is the file.
#[derive(Clone)]
pub struct FilezFile {
    pub origin: String,
    pub id: String,
    pub content: Aes256Gcm,
    pub cipher_size: u64,
    pub size: u64,
    pub name: String,
}

/// The resolved data address of each filez download, to its key (see `FilezFile`).
static FOUND: LazyLock<Mutex<HashMap<String, FilezFile>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The filez file behind a resolved address, when it is one.
pub fn file_for(data_url: &str) -> Option<FilezFile> {
    FOUND.lock().ok()?.get(data_url).cloned()
}

pub fn matches(url: &str) -> bool {
    let host = super::hostname(url);
    super::domain_match(&host, "filez.stream") || dev_origin().is_some_and(|o| url.starts_with(&o))
}

/// Debug builds only: a local filez.stream (`wrangler dev`) stands in for the
/// real one, so the whole path can be run without the real host.
fn dev_origin() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        std::env::var("KRYOTO_DEV_FILEZ_ORIGIN").ok().map(|o| o.trim_end_matches('/').to_string()).filter(|o| !o.is_empty())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// A file link: its origin, id and key. `None` for any other filez address
/// (the home page, `/dl/` links), or one whose key is missing or malformed.
pub fn parse(url: &str) -> Option<(String, String, [u8; 32])> {
    let u = url::Url::parse(url).ok()?;
    let mut segs = u.path_segments()?.filter(|s| !s.is_empty());
    let id = segs.next()?.to_string();
    if segs.next().is_some() || id.len() < 4 || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(u.fragment()?.trim_end_matches('=')).ok()?;
    let key: [u8; 32] = raw.try_into().ok()?;
    Some((u.origin().ascii_serialization(), id, key))
}

fn derive(raw: &[u8; 32], info: &str) -> Aes256Gcm {
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(b"filez.stream/v1"), raw);
    let mut okm = [0u8; 32];
    hk.expand(info.as_bytes(), &mut okm).expect("32 bytes is a valid HKDF-SHA256 length");
    Aes256Gcm::new_from_slice(&okm).expect("a 32-byte key")
}

/// The name and size sealed in `encMeta`. `None` for the wrong key.
pub fn open_meta(raw: &[u8; 32], sealed: &str) -> Option<(String, u64)> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(sealed.trim_end_matches('=')).ok()?;
    if bytes.len() < 12 + 16 {
        return None;
    }
    let (iv, ct) = bytes.split_at(12);
    let plain = derive(raw, "meta").decrypt(Nonce::from_slice(iv), ct).ok()?;
    let m: serde_json::Value = serde_json::from_slice(&plain).ok()?;
    let name = m["n"].as_str().unwrap_or("download").to_string();
    let size = m["s"].as_u64().or_else(|| m["s"].as_f64().map(|f| f as u64))?;
    Some((name, size))
}

pub fn chunk_count(size: u64) -> u64 {
    size.div_ceil(CHUNK).max(1)
}

/// The plaintext of chunk `index` out of its sealed bytes, or `None` when it
/// does not open (damaged, cut short, or the wrong key).
pub fn open_chunk(key: &Aes256Gcm, index: u64, last: bool, sealed: &[u8]) -> Option<Vec<u8>> {
    let mut iv = [0u8; 12];
    iv[4..].copy_from_slice(&index.to_be_bytes());
    key.decrypt(Nonce::from_slice(&iv), Payload { msg: sealed, aad: &[u8::from(last)] }).ok()
}

/// Bytes [from, to] (inclusive) of the ciphertext for chunks `c0..=c1`.
pub fn cipher_range(cipher_size: u64, c0: u64, c1: u64) -> (u64, u64) {
    (c0 * ENC_CHUNK, ((c1 + 1) * ENC_CHUNK).min(cipher_size) - 1)
}

/// Ask the host for `want` more bytes, waiting in its slow lane when the
/// day's allowance is spent. Gives up (`Err`) when stopped or refused.
pub async fn grant(client: &reqwest::Client, f: &FilezFile, mut want: u64, stopped: &(dyn Fn() -> bool + Sync)) -> Result<(), String> {
    let mut failures = 0u32;
    while want > 0 {
        if stopped() {
            return Err(crate::downloads::STOPPED.into());
        }
        let ask = want.min(512 * CHUNK);
        let res = client
            .post(format!("{}/api/grant", f.origin))
            .json(&serde_json::json!({ "want": ask, "file": f.id }))
            .send()
            .await;
        let wait_ms = match res {
            Ok(r) if r.status().is_success() => {
                failures = 0;
                let j: serde_json::Value = r.json().await.unwrap_or_default();
                let granted = j["granted"].as_u64().unwrap_or(0).min(ask);
                want -= granted;
                if granted == 0 { j["retryAfter"].as_u64().unwrap_or(1000).clamp(100, 10_000) } else { 0 }
            }
            Ok(r) if r.status().as_u16() == 403 => return Err(format!("filez.stream refused the download ({})", r.status().as_u16())),
            _ => {
                failures += 1;
                if failures > 5 {
                    return Err("filez.stream did not give permission to download".into());
                }
                500 * (1 << failures)
            }
        };
        if wait_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
        }
    }
    Ok(())
}

/// The link to the file: its size and name (out of the sealed metadata),
/// and the host's verification passed in its own page. The download then
/// fetches `/api/files/<id>/data` with that pass.
pub async fn resolve<R: Runtime>(app: &AppHandle<R>, page: &str) -> Result<super::Resolved, String> {
    let (origin, id, raw) = parse(page).ok_or("this filez.stream link has no key (the part after #). Copy the whole link from kryo.to.")?;
    let info: serde_json::Value = super::client(true)
        .get(format!("{origin}/api/files/{id}"))
        .send()
        .await
        .map_err(|e| format!("filez.stream did not answer ({e})"))?
        .error_for_status()
        .map_err(|e| match e.status().map(|s| s.as_u16()) {
            Some(404 | 410) => "the file is gone from filez.stream".to_string(),
            s => format!("filez.stream answered {}", s.unwrap_or(0)),
        })?
        .json()
        .await
        .map_err(|_| "filez.stream's answer was unreadable".to_string())?;
    if info["status"].as_str().is_some_and(|s| s != "ready") {
        return Err("the file is still being uploaded to filez.stream. Try again later.".into());
    }
    let cipher_size = info["cipherSize"].as_u64().ok_or("filez.stream did not say how big the file is")?;
    let (name, size) = open_meta(&raw, info["encMeta"].as_str().unwrap_or("")).ok_or("the link's key does not open this file (the link was cut short?)")?;
    let access = super::browser::filez_access(app, page, &origin).await?;
    let data = format!("{origin}/api/files/{id}/data");
    let file = FilezFile { origin: origin.clone(), id, content: derive(&raw, "content"), cipher_size, size, name: name.clone() };
    if let Ok(mut found) = FOUND.lock() {
        found.insert(data.clone(), file);
    }
    let mut headers = HashMap::from([("User-Agent".to_string(), access.user_agent), ("Referer".to_string(), format!("{origin}/"))]);
    if !access.cookie.is_empty() {
        headers.insert("Cookie".into(), access.cookie);
    }
    Ok(super::Resolved { url: data, file_name: Some(name), size: Some(size), headers, connections: 4 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::AeadInPlace;

    /// Seal like the host's browser code does (`sealChunk`), for the round trip.
    fn seal(key: &Aes256Gcm, index: u64, last: bool, plain: &[u8]) -> Vec<u8> {
        let mut iv = [0u8; 12];
        iv[4..].copy_from_slice(&index.to_be_bytes());
        let mut buf = plain.to_vec();
        key.encrypt_in_place(Nonce::from_slice(&iv), &[u8::from(last)], &mut buf).unwrap();
        buf
    }

    #[test]
    fn reads_id_and_key_out_of_a_link() {
        let key = [7u8; 32];
        let k = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key);
        assert_eq!(parse(&format!("https://filez.stream/Ab12Cd34#{k}")), Some(("https://filez.stream".into(), "Ab12Cd34".into(), key)));
        assert_eq!(parse("https://filez.stream/Ab12Cd34"), None, "no key, no file");
        assert_eq!(parse(&format!("https://filez.stream/dl/Ab12Cd34/{k}/x.7z")), None);
        assert_eq!(parse("https://filez.stream/Ab12Cd34#short"), None);
        assert!(matches("https://filez.stream/x"));
        assert!(!matches("https://filez.stream.evil.net/x"));
    }

    #[test]
    fn chunks_open_in_order_and_a_cut_file_does_not() {
        let raw = [3u8; 32];
        let key = derive(&raw, "content");
        let a = seal(&key, 0, false, b"first chunk");
        let b = seal(&key, 1, true, b"last chunk");
        assert_eq!(open_chunk(&key, 0, false, &a).as_deref(), Some(&b"first chunk"[..]));
        assert_eq!(open_chunk(&key, 1, true, &b).as_deref(), Some(&b"last chunk"[..]));
        // The first chunk passed off as the last (a file cut short) fails.
        assert_eq!(open_chunk(&key, 0, true, &a), None);
        // In another place in the file, it fails too.
        assert_eq!(open_chunk(&key, 1, false, &a), None);
        // The metadata key does not open content.
        assert_eq!(open_chunk(&derive(&raw, "meta"), 0, false, &a), None);
    }

    #[test]
    fn metadata_opens_with_the_meta_key() {
        let raw = [9u8; 32];
        let iv = [1u8; 12];
        let body = br#"{"n":"Game - kryoto.7z","t":"application/x-7z-compressed","s":3145728}"#;
        let ct = derive(&raw, "meta").encrypt(Nonce::from_slice(&iv), &body[..]).unwrap();
        let sealed = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([&iv[..], &ct].concat());
        assert_eq!(open_meta(&raw, &sealed), Some(("Game - kryoto.7z".into(), 3_145_728)));
        assert_eq!(open_meta(&[0u8; 32], &sealed), None, "the wrong key");
    }

    #[test]
    fn ranges_cover_whole_sealed_chunks() {
        let size = 3 * CHUNK + 10;
        let cipher = size + TAG * chunk_count(size);
        assert_eq!(chunk_count(size), 4);
        assert_eq!(cipher_range(cipher, 0, 0), (0, ENC_CHUNK - 1));
        assert_eq!(cipher_range(cipher, 3, 3), (3 * ENC_CHUNK, cipher - 1));
        assert_eq!(cipher_range(cipher, 1, 3).1, cipher - 1);
        assert_eq!(chunk_count(0), 1);
    }
}
