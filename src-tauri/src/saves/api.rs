//! Talking to kryo.to's cloud saves (`/api/saves`) as the account signed in
//! to the Store, the way `phone.rs` does: the Store view's cookies on a
//! request from here. Storage itself is reached through the short-lived
//! addresses kryo.to hands out, with a separate client that carries no
//! cookies.

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
    pub max_backup_bytes: u64,
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
    cookie: String,
}

fn ua() -> &'static str {
    concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION"))
}

/// A client for storage addresses: no cookies, no redirects to elsewhere.
pub fn storage_client() -> reqwest::Client {
    reqwest::Client::builder()
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
        let base = crate::settings::catalog_endpoint(&crate::settings::load(app));
        let url = url::Url::parse(&base).map_err(|e| ApiError::Other(e.to_string()))?;
        let cookie = app
            .get_webview(crate::STORE)
            .and_then(|view| view.cookies_for_url(url).ok())
            .map(|jar| jar.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; "))
            .filter(|c| !c.is_empty())
            .ok_or(ApiError::SignedOut)?;
        let http = reqwest::Client::builder()
            .user_agent(ua())
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| ApiError::Other(e.to_string()))?;
        Ok(Api { http, base, cookie })
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<serde_json::Value, ApiError> {
        let res = req.header("cookie", &self.cookie).send().await.map_err(|e| ApiError::Other(format!("Could not reach kryo.to: {e}")))?;
        let status = res.status().as_u16();
        let body: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
        let message = || body.get("error").and_then(|e| e.as_str()).unwrap_or("kryo.to could not do that.").to_string();
        match status {
            200..=299 => Ok(body),
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
        let v = self.send(self.http.get(self.url("/api/saves"))).await?;
        serde_json::from_value(v).map_err(|e| ApiError::Other(e.to_string()))
    }

    pub async fn head(&self, game_key: &str) -> Result<Head, ApiError> {
        let v = self.send(self.http.get(self.url("/api/saves")).query(&[("gameKey", game_key)])).await?;
        serde_json::from_value(v).map_err(|e| ApiError::Other(e.to_string()))
    }

    pub async fn prepare(&self, body: serde_json::Value) -> Result<Prepared, ApiError> {
        let v = self.send(self.http.post(self.url("/api/saves/prepare")).json(&body)).await?;
        serde_json::from_value(v).map_err(|e| ApiError::Other(e.to_string()))
    }

    pub async fn claim(&self, upload_id: &str) -> Result<Version, ApiError> {
        let v = self.send(self.http.post(self.url("/api/saves/claim")).json(&json!({ "uploadId": upload_id }))).await?;
        serde_json::from_value(v.get("version").cloned().unwrap_or_default()).map_err(|e| ApiError::Other(e.to_string()))
    }

    pub async fn link(&self, version_id: &str) -> Result<Link, ApiError> {
        let v = self.send(self.http.post(self.url("/api/saves/link")).json(&json!({ "versionId": version_id }))).await?;
        serde_json::from_value(v).map_err(|e| ApiError::Other(e.to_string()))
    }

    pub async fn rules(&self, keys: &[String]) -> Result<HashMap<String, Vec<kryoto_saves::Rule>>, ApiError> {
        let v = self.send(self.http.get(self.url("/api/saves/rules")).query(&[("gameKeys", keys.join(","))])).await?;
        let rules = v.get("rules").cloned().unwrap_or_default();
        // A rule this version cannot read is skipped, not the whole answer.
        let map: HashMap<String, Vec<serde_json::Value>> = serde_json::from_value(rules).unwrap_or_default();
        Ok(map.into_iter().map(|(k, list)| (k, list.into_iter().filter_map(|r| serde_json::from_value(r).ok()).collect())).collect())
    }

    /// `traced`: Windows saw the game write these (precise finding).
    pub async fn report_rules(&self, game_key: &str, rules: &[kryoto_saves::Rule], traced: bool) -> Result<(), ApiError> {
        let rules: Vec<_> = rules.iter().take(20).map(|r| json!({ "kind": r.kind, "root": r.root, "path": r.path, "excludes": r.excludes, "source": "detected" })).collect();
        self.send(self.http.post(self.url("/api/saves/rules/report")).json(&json!({ "gameKey": game_key, "rules": rules, "traced": traced }))).await.map(|_| ())
    }
}

/// PUT a backup to the address kryo.to gave, with exactly the headers it
/// signed (the MD5 and length), so storage takes these bytes and no others.
pub async fn upload(url: &str, headers: &HashMap<String, String>, file: &Path, size: u64, sent: impl Fn(u64) + Send + Sync + 'static) -> Result<(), String> {
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
pub async fn download(url: &str, out: &Path, max: u64, on_got: impl Fn(u64, u64)) -> Result<(), String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let res = storage_client().get(url).send().await.map_err(|e| format!("Could not download the backup: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Could not download the backup ({}).", res.status()));
    }
    let tmp = out.with_extension("part");
    let total = res.content_length().unwrap_or(0);
    let mut f = tokio::fs::File::create(&tmp).await.map_err(|e| e.to_string())?;
    let mut got = 0u64;
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("The download stopped: {e}"))?;
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
