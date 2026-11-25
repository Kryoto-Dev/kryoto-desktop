//! Talking to kryo.to's cloud saves (`/api/saves`) as the account signed in
//! to the Store, the way `phone.rs` does: the Store view's cookies on a
//! request from here. Storage itself is reached through the short-lived
//! addresses kryo.to hands out, with a separate client that carries no
//! cookies.
//!
//! With a saves server of the player's own in Settings > Patchbay
//! (patchbay/), the same API is asked of that server instead, with its
//! bearer token and no kryo.to cookie: kryo.to then sees none of it.
//!
//! With save versions kept on this PC (Settings > Cloud, local.rs), the same
//! calls are answered from that folder, and nothing goes over the network.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager, Runtime};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Allowance {
    pub total_bytes: u64,
    pub versions_per_game: u32,
    /// The biggest one backup file can be, packed.
    pub max_backup_bytes: u64,
    /// The most save data one backup can hold before packing (kryo.to: four
    /// times the packed cap). 0 when the server does not say: then it is the
    /// packed cap (`sync::limits`).
    pub max_raw_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Usage {
    pub bytes: u64,
    pub games: u32,
    pub versions: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Version {
    pub id: String,
    pub size: u64,
    pub sha256: String,
    pub md5: String,
    pub file_count: u64,
    pub machine: Option<String>,
    pub created_at: u64,
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<serde_json::Value>,
}

impl Version {
    /// The saves' content hash, as Kryoto Desktop sent it.
    pub fn content_hash(&self) -> Option<String> {
        self.manifest.as_ref()?.get("contentHash")?.as_str().map(str::to_string)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CloudGame {
    pub game_key: String,
    pub aliases: Vec<String>,
    pub slug: Option<String>,
    pub title: String,
    pub bytes: u64,
    pub versions: Vec<Version>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct List {
    pub perk: bool,
    pub allowance: Allowance,
    pub usage: Usage,
    pub games: Vec<CloudGame>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Head {
    pub perk: bool,
    pub allowance: Allowance,
    pub head: Option<Version>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub upload_id: String,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub url: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// Cloud saves are not on for this account (the feature flag).
    Off,
    SignedOut,
    /// Saving needs the supporter perk.
    NoPerk,
    /// Another PC backed up since this one last synced.
    Conflict(Option<Box<Version>>),
    /// Over the account's space or a backup's size.
    TooBig(String),
    Limited,
    /// The upload did not arrive as it was sent (kryo.to deleted it).
    Rejected(String),
    Other(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&match self {
            ApiError::Off => "Cloud saves are not available on your account yet.".into(),
            ApiError::SignedOut => "Sign in to kryo.to in Kryoto Desktop first.".into(),
            ApiError::NoPerk => "Cloud saves are a supporter perk.".into(),
            ApiError::Conflict(_) => "Another PC backed up this game's saves since this one last did.".into(),
            ApiError::TooBig(m) | ApiError::Rejected(m) | ApiError::Other(m) => m.clone(),
            ApiError::Limited => "Too many backups in a short time. Try again in a while.".into(),
        })
    }
}

pub struct Api {
    http: reqwest::Client,
    base: String,
    auth: Auth,
    /// Who answers, for messages: kryo.to, or the own server's host.
    place: String,
}

enum Auth {
    /// The Store view's kryo.to cookies.
    Cookie(String),
    /// The player's own server (Patchbay), with or without a token.
    Own(crate::patchbay::Own),
    /// A folder on this PC.
    Local(super::local::LocalStore),
}

fn ua() -> &'static str {
    concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION"))
}

/// A client for storage addresses: no cookies, no redirects to elsewhere.
pub fn storage_client() -> reqwest::Client {
    crate::net::builder()
        .user_agent(ua())
        .connect_timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .unwrap_or_default()
}

impl Api {
    /// As the account signed in to the Store. Call from async code or a
    /// worker thread, never a synchronous command on the main thread.
    pub fn new<R: Runtime>(app: &AppHandle<R>) -> Result<Api, ApiError> {
        match super::target(app) {
            super::Target::Own(own) => {
                return Ok(Api { http: crate::patchbay::client(), base: own.base.clone(), place: own.host.clone(), auth: Auth::Own(own) });
            }
            super::Target::Local(dir) => {
                let keep = crate::settings::load(app).saves_local_keep;
                let place = dir.to_string_lossy().into_owned();
                return Ok(Api { http: crate::net::builder().build().unwrap_or_default(), base: String::new(), place, auth: Auth::Local(super::local::LocalStore::new(dir, keep)) });
            }
            super::Target::Account => {}
        }
        let base = crate::settings::catalog_endpoint(&crate::settings::load(app));
        let url = url::Url::parse(&base).map_err(|e| ApiError::Other(e.to_string()))?;
        let cookie = app
            .get_webview(crate::STORE)
            .and_then(|view| view.cookies_for_url(url).ok())
            .map(|jar| jar.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; "))
            .filter(|c| !c.is_empty())
            .ok_or(ApiError::SignedOut)?;
        // kryo.to's name encrypted where that works (net.rs `kryoto_tls`, KB-0347).
        let http = crate::net::kryoto_tls(&base, crate::net::builder())
            .user_agent(ua())
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| ApiError::Other(e.to_string()))?;
        Ok(Api { http, base, auth: Auth::Cookie(cookie), place: "kryo.to".into() })
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<serde_json::Value, ApiError> {
        let req = match &self.auth {
            Auth::Cookie(cookie) => req.header("cookie", cookie),
            Auth::Own(own) => crate::patchbay::authorize(req, own),
            Auth::Local(_) => return Err(ApiError::Other("Saves kept on this PC need no network.".into())),
        };
        let res = req.send().await.map_err(|e| ApiError::Other(format!("Could not reach {}: {e}", self.place)))?;
        let status = res.status().as_u16();
        let body: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
        let message = || body.get("error").and_then(|e| e.as_str()).map(str::to_string).unwrap_or_else(|| format!("{} could not do that.", self.place));
        let own = matches!(self.auth, Auth::Own(_));
        match status {
            200..=299 => Ok(body),
            401 | 403 if own => Err(ApiError::Other(format!("{} refused the token. Check it in Settings > Patchbay.", self.place))),
            404 if own => Err(ApiError::Other(format!("{} has no cloud saves at this address. Check it in Settings > Patchbay.", self.place))),
            401 => Err(ApiError::SignedOut),
            403 if body.get("perk").and_then(|p| p.as_bool()) == Some(false) => Err(ApiError::NoPerk),
            404 => Err(ApiError::Off),
            409 => Err(ApiError::Conflict(body.get("head").cloned().and_then(|h| serde_json::from_value(h).ok()).map(Box::new))),
            413 => Err(ApiError::TooBig(message())),
            422 => Err(ApiError::Rejected(message())),
            429 => Err(ApiError::Limited),
            _ => Err(ApiError::Other(message())),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub async fn list(&self) -> Result<List, ApiError> {
        if let Auth::Local(store) = &self.auth {
            return store.list();
        }
        let v = self.send(self.http.get(self.url("/api/saves"))).await?;
        serde_json::from_value(v).map_err(|e| ApiError::Other(e.to_string()))
    }

    pub async fn head(&self, game_key: &str) -> Result<Head, ApiError> {
        if let Auth::Local(store) = &self.auth {
            return store.head(game_key);
        }
        let v = self.send(self.http.get(self.url("/api/saves")).query(&[("gameKey", game_key)])).await?;
        serde_json::from_value(v).map_err(|e| ApiError::Other(e.to_string()))
    }

    pub async fn prepare(&self, body: serde_json::Value) -> Result<Prepared, ApiError> {
        if let Auth::Local(store) = &self.auth {
            return store.prepare(body);
        }
        let v = self.send(self.http.post(self.url("/api/saves/prepare")).json(&body)).await?;
        let prepared: Prepared = serde_json::from_value(v).map_err(|e| ApiError::Other(e.to_string()))?;
        web_address(&prepared.url, &self.place)?;
        Ok(prepared)
    }

    pub async fn claim(&self, upload_id: &str) -> Result<Version, ApiError> {
        if let Auth::Local(store) = &self.auth {
            return store.claim(upload_id);
        }
        let v = self.send(self.http.post(self.url("/api/saves/claim")).json(&json!({ "uploadId": upload_id }))).await?;
        serde_json::from_value(v.get("version").cloned().unwrap_or_default()).map_err(|e| ApiError::Other(e.to_string()))
    }

    pub async fn link(&self, version_id: &str) -> Result<Link, ApiError> {
        if let Auth::Local(store) = &self.auth {
            return store.link(version_id);
        }
        let v = self.send(self.http.post(self.url("/api/saves/link")).json(&json!({ "versionId": version_id }))).await?;
        let link: Link = serde_json::from_value(v).map_err(|e| ApiError::Other(e.to_string()))?;
        web_address(&link.url, &self.place)?;
        Ok(link)
    }

    pub async fn rules(&self, keys: &[String]) -> Result<HashMap<String, Vec<kryoto_saves::Rule>>, ApiError> {
        // On this PC there are no other players' locations to learn from.
        if matches!(self.auth, Auth::Local(_)) {
            return Ok(HashMap::new());
        }
        let v = self.send(self.http.get(self.url("/api/saves/rules")).query(&[("gameKeys", keys.join(","))])).await?;
        let rules = v.get("rules").cloned().unwrap_or_default();
        // A rule this version cannot read is skipped, not the whole answer.
        let map: HashMap<String, Vec<serde_json::Value>> = serde_json::from_value(rules).unwrap_or_default();
        Ok(map.into_iter().map(|(k, list)| (k, list.into_iter().filter_map(|r| serde_json::from_value(r).ok()).collect())).collect())
    }

    /// `traced`: Windows saw the game write these (precise finding).
    pub async fn report_rules(&self, game_key: &str, rules: &[kryoto_saves::Rule], traced: bool) -> Result<(), ApiError> {
        if matches!(self.auth, Auth::Local(_)) {
            return Ok(());
        }
        let rules: Vec<_> = rules.iter().take(20).map(|r| json!({ "kind": r.kind, "root": r.root, "path": r.path, "excludes": r.excludes, "source": "detected" })).collect();
        self.send(self.http.post(self.url("/api/saves/rules/report")).json(&json!({ "gameKey": game_key, "rules": rules, "traced": traced }))).await.map(|_| ())
    }

    /// Put a backup where `prepare` said: copied into the folder kept on this
    /// PC, else sent to the web address (`upload_to_web`).
    pub async fn upload(&self, url: &str, headers: &HashMap<String, String>, file: &Path, size: u64, sent: impl Fn(u64) + Send + Sync + 'static) -> Result<(), String> {
        match self.file_in_store(url)? {
            Some(dest) => super::local::copy(file, &dest, size, sent).await.map(|_| ()),
            None => upload_to_web(url, headers, file, size, sent).await,
        }
    }

    /// Fetch a backup from where `link` said to `out`, refusing anything
    /// bigger than `max`. `on_got(bytes so far, bytes in all)` follows it.
    pub async fn download(&self, url: &str, out: &Path, max: u64, on_got: impl Fn(u64, u64)) -> Result<(), String> {
        match self.file_in_store(url)? {
            Some(src) => {
                let total = std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
                super::local::copy(&src, out, max, |done| on_got(done, total.max(done))).await.map(|_| ())
            }
            None => download_from_web(url, out, max, on_got).await,
        }
    }

    /// The file a backup's `file:` address names, for the folder kept on this
    /// PC only. Anywhere else the address must be a web one, so nothing a
    /// server says can make this PC read or write one of its own files.
    fn file_in_store(&self, url: &str) -> Result<Option<std::path::PathBuf>, String> {
        if let (Auth::Local(_), Some(path)) = (&self.auth, super::local::path_of(url)) {
            return Ok(Some(path));
        }
        web_address(url, &self.place).map(|()| None).map_err(|e| e.to_string())
    }
}

/// Where a server says to put or fetch a backup. Only a web address is taken
/// from a server: a `file:` address names a file on this PC, and only the
/// folder kept on this PC (local.rs) hands those out.
fn web_address(url: &str, place: &str) -> Result<(), ApiError> {
    match url::Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "https" | "http") => Ok(()),
        _ => Err(ApiError::Other(format!("{place} answered with a storage address that is not a web address."))),
    }
}

/// PUT a backup to the address kryo.to gave, with exactly the headers it
/// signed (the MD5 and length), so storage takes these bytes and no others.
async fn upload_to_web(url: &str, headers: &HashMap<String, String>, file: &Path, size: u64, sent: impl Fn(u64) + Send + Sync + 'static) -> Result<(), String> {
    let client = storage_client();
    let sent = std::sync::Arc::new(sent);
    let mut last = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(2 * attempt)).await;
        }
        let body = tokio::fs::File::open(file).await.map_err(|e| e.to_string())?;
        let mut req = client.put(url).body(counted(body, sent.clone())).header("content-length", size.to_string());
        for (k, v) in headers {
            if !k.eq_ignore_ascii_case("content-length") {
                req = req.header(k.as_str(), v.as_str());
            }
        }
        match req.send().await {
            Ok(res) if res.status().is_success() => return Ok(()),
            Ok(res) => {
                last = format!("Storage refused the backup ({}).", res.status());
                // A signed address that was refused will be refused again.
                if res.status().is_client_error() {
                    break;
                }
            }
            Err(e) => last = format!("Could not upload the backup: {e}"),
        }
    }
    Err(last)
}

/// A file as an upload body that says how many bytes have gone out.
fn counted(file: tokio::fs::File, sent: std::sync::Arc<impl Fn(u64) + Send + Sync + 'static>) -> reqwest::Body {
    use tokio::io::AsyncReadExt;
    // (file, bytes so far, finished)
    let stream = futures_util::stream::unfold((file, 0u64, false), move |(mut f, so_far, finished)| {
        let sent = sent.clone();
        async move {
            if finished {
                return None;
            }
            let mut buf = vec![0u8; 256 * 1024];
            match f.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => {
                    buf.truncate(n);
                    let total = so_far + n as u64;
                    sent(total);
                    Some((Ok::<Vec<u8>, std::io::Error>(buf), (f, total, false)))
                }
                Err(e) => Some((Err(e), (f, so_far, true))),
            }
        }
    });
    reqwest::Body::wrap_stream(stream)
}

/// Download a backup to `out`, refusing anything bigger than `max`.
/// `on_got(bytes so far, bytes in all)` follows it.
async fn download_from_web(url: &str, out: &Path, max: u64, on_got: impl Fn(u64, u64)) -> Result<(), String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let res = storage_client().get(url).send().await.map_err(|e| format!("Could not download the backup: {}", crate::logging::why(&e)))?;
    if !res.status().is_success() {
        return Err(format!("Could not download the backup ({}).", res.status()));
    }
    let tmp = out.with_extension("part");
    let total = res.content_length().unwrap_or(0);
    let mut f = tokio::fs::File::create(&tmp).await.map_err(|e| e.to_string())?;
    let mut got = 0u64;
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("The download stopped: {}", crate::logging::why(&e)))?;
        got += chunk.len() as u64;
        on_got(got, total.max(got));
        if got > max {
            drop(f);
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err("That backup is bigger than it should be.".into());
        }
        f.write_all(&chunk).await.map_err(|e| e.to_string())?;
    }
    f.flush().await.map_err(|e| e.to_string())?;
    drop(f);
    tokio::fs::rename(&tmp, out).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A save server of the player's own that answers one request with `reply`.
    async fn own_server(reply: serde_json::Value) -> Api {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            // The whole request first: closing on unread bytes can reset the
            // connection, and the test would pass on that error instead.
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got).to_ascii_lowercase();
                let whole = text.find("\r\n\r\n").is_some_and(|end| {
                    let length = text[..end].lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                    got.len() >= end + 4 + length
                });
                if n == 0 || whole {
                    break;
                }
            }
            let body = reply.to_string();
            let res = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            socket.write_all(res.as_bytes()).await.unwrap();
        });
        let own = crate::patchbay::Own { base: base.clone(), host: "the save server".into(), token: None };
        Api { http: reqwest::Client::builder().no_proxy().build().unwrap(), base, place: own.host.clone(), auth: Auth::Own(own) }
    }

    #[tokio::test]
    async fn a_server_cannot_send_a_backup_to_a_file_on_this_pc() {
        let file = "file:///C:/Users/player/Documents/thesis.docx";
        let refused = |r: Result<String, ApiError>| matches!(r, Err(ApiError::Other(m)) if m == "the save server answered with a storage address that is not a web address.");
        assert!(refused(own_server(json!({ "uploadId": "x", "url": file })).await.prepare(json!({})).await.map(|p| p.url)));
        assert!(refused(own_server(json!({ "url": file, "size": 4 })).await.link("x").await.map(|l| l.url)));
        // The same server with a web address is taken at its word.
        let web = "http://192.168.1.20:8787/upload/x?key=abc";
        assert_eq!(own_server(json!({ "uploadId": "x", "url": web })).await.prepare(json!({})).await.unwrap().url, web);
        assert_eq!(own_server(json!({ "url": web, "size": 4 })).await.link("x").await.unwrap().url, web);
    }

    /// Only the folder kept on this PC moves backups by `file:` address. For
    /// a server, one is refused before any file is read or written, even if
    /// it got past `prepare` and `link`.
    #[tokio::test]
    async fn only_the_folder_on_this_pc_moves_backups_by_file_address() {
        let dir = std::env::temp_dir().join(format!("kryoto-api-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mine = dir.join("thesis.docx");
        std::fs::write(&mine, b"mine").unwrap();
        let address = |p: &Path| url::Url::from_file_path(p).unwrap().to_string();
        let own = crate::patchbay::Own { base: "http://127.0.0.1:9".into(), host: "the save server".into(), token: None };
        let server = Api { http: reqwest::Client::new(), base: own.base.clone(), place: own.host.clone(), auth: Auth::Own(own) };
        let refused = Err("the save server answered with a storage address that is not a web address.".to_string());
        assert_eq!(server.download(&address(&mine), &dir.join("got"), 1 << 20, |_, _| {}).await, refused);
        assert_eq!(server.upload(&address(&dir.join("put")), &HashMap::new(), &mine, 4, |_| {}).await, refused);
        assert!(!dir.join("got").exists() && !dir.join("put").exists());
        // The folder on this PC copies them in and out as before.
        let here = Api { http: reqwest::Client::new(), base: String::new(), place: dir.to_string_lossy().into_owned(), auth: Auth::Local(crate::saves::local::LocalStore::new(dir.clone(), 10)) };
        here.download(&address(&mine), &dir.join("got"), 1 << 20, |_, _| {}).await.unwrap();
        here.upload(&address(&dir.join("put")), &HashMap::new(), &mine, 4, |_| {}).await.unwrap();
        assert_eq!(std::fs::read(dir.join("got")).unwrap(), b"mine");
        assert_eq!(std::fs::read(dir.join("put")).unwrap(), b"mine");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_web_addresses_are_taken_from_a_server() {
        for url in ["https://storage.kryo.to/saves/x?sig=1", "http://192.168.1.20:8787/download/x?key=abc"] {
            assert_eq!(web_address(url, "kryo.to"), Ok(()), "{url}");
        }
        for url in ["file:///C:/Windows/win.ini", "FILE:///home/player/.bashrc", "file://server/share/x", r"C:\Users\player\x", "/upload/x", "ftp://example.com/x", ""] {
            assert!(web_address(url, "kryo.to").is_err(), "{url}");
        }
    }
}
