//! Save versions in a folder on this PC, for everyone: no account, no perk.
//!
//! The same versions, restore and keep-N as kryo.to's cloud saves and a
//! Patchbay server, answering the same calls (`api.rs` hands them here), laid
//! out like `tools/patchbay-server.mjs`: `index.json` beside `files/<id>.ksave`.
//! A backup is "uploaded" by copying it into `files/` and "downloaded" by
//! copying it back out, through `file:` addresses, so sync.rs runs unchanged.

use super::api::{Allowance, ApiError, CloudGame, Head, Link, List, Prepared, Usage, Version};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Bigger than any backup kryo.to takes: on this PC, the disk is the limit.
const MAX_BACKUP: u64 = 8 << 30;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Game {
    aliases: Vec<String>,
    slug: Option<String>,
    title: String,
    /// Newest first.
    versions: Vec<Version>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Index {
    games: BTreeMap<String, Game>,
}

/// A backup announced by `prepare`, waiting for its bytes and its `claim`.
#[derive(Debug, Clone)]
struct Pending {
    game_key: String,
    aliases: Vec<String>,
    slug: Option<String>,
    title: String,
    size: u64,
    sha256: String,
    md5: String,
    machine: Option<String>,
    parent: Option<String>,
    manifest: Option<serde_json::Value>,
}

/// One writer at a time for the index, whichever store asks.
static INDEX: Mutex<()> = Mutex::new(());
static PENDING: Mutex<Option<HashMap<String, Pending>>> = Mutex::new(None);

pub struct LocalStore {
    dir: PathBuf,
    keep: u32,
}

fn key_ok(k: &str) -> bool {
    !k.is_empty() && k.len() <= 200 && k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

fn id_ok(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A fresh id: unique per backup, never a path.
fn new_id(game_key: &str) -> String {
    use sha2::Digest;
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let digest = sha2::Sha256::digest(format!("{game_key}|{nanos}|{n}|{}", std::process::id()).as_bytes());
    hex::encode(&digest[..16])
}

fn io(e: impl std::fmt::Display) -> ApiError {
    ApiError::Other(format!("Could not use the saves folder: {e}"))
}

impl LocalStore {
    pub fn new(dir: PathBuf, keep: u32) -> Self {
        LocalStore { dir, keep: keep.clamp(1, 100) }
    }

    fn files(&self) -> PathBuf {
        self.dir.join("files")
    }

    fn read(&self) -> Index {
        std::fs::read(self.dir.join("index.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn write(&self, index: &Index) -> Result<(), ApiError> {
        super::write_json(&self.dir.join("index.json"), index).map_err(io)
    }

    fn allowance(&self, used: u64) -> Allowance {
        let room = crate::storage::disk_space(&self.dir).map(|(_, free)| free).unwrap_or(0);
        Allowance { total_bytes: used + room, versions_per_game: self.keep, max_backup_bytes: MAX_BACKUP }
    }

    fn file_url(path: &Path) -> Result<String, ApiError> {
        url::Url::from_file_path(path).map(String::from).map_err(|_| ApiError::Other(format!("{} is not a folder Kryoto can use.", path.display())))
    }

    pub fn list(&self) -> Result<List, ApiError> {
        let index = self.read();
        let games: Vec<CloudGame> = index
            .games
            .iter()
            .map(|(k, g)| CloudGame {
                game_key: k.clone(),
                aliases: g.aliases.clone(),
                slug: g.slug.clone(),
                title: g.title.clone(),
                bytes: g.versions.iter().map(|v| v.size).sum(),
                versions: g.versions.clone(),
            })
            .collect();
        let bytes = games.iter().map(|g| g.bytes).sum();
        let usage = Usage { bytes, games: games.len() as u32, versions: games.iter().map(|g| g.versions.len() as u32).sum() };
        Ok(List { perk: true, allowance: self.allowance(bytes), usage, games })
    }

    pub fn head(&self, game_key: &str) -> Result<Head, ApiError> {
        let index = self.read();
        let used = index.games.values().flat_map(|g| &g.versions).map(|v| v.size).sum();
        Ok(Head { perk: true, allowance: self.allowance(used), head: index.games.get(game_key).and_then(|g| g.versions.first().cloned()) })
    }

    /// As a server's `prepare`: check the backup's shape and that this PC
    /// saw the newest version (else a conflict), then hand out where to put it.
    pub fn prepare(&self, body: serde_json::Value) -> Result<Prepared, ApiError> {
        let s = |k: &str| body.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let game_key = s("gameKey").filter(|k| key_ok(k)).ok_or_else(|| ApiError::Other("Which game?".into()))?;
        let size = body.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        let sha256 = s("sha256").filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).ok_or_else(|| ApiError::Other("The backup has no checksum.".into()))?;
        if size == 0 {
            return Err(ApiError::Other("The backup is empty.".into()));
        }
        if size > MAX_BACKUP {
            return Err(ApiError::TooBig("This backup is too big.".into()));
        }
        let parent = s("parentVersion");
        let head = self.read().games.get(&game_key).and_then(|g| g.versions.first().cloned());
        if head.as_ref().map(|h| h.id.clone()) != parent {
            return Err(ApiError::Conflict(head.map(Box::new)));
        }
        std::fs::create_dir_all(self.files()).map_err(io)?;
        let id = new_id(&game_key);
        let pending = Pending {
            aliases: body.get("aliases").and_then(|a| serde_json::from_value(a.clone()).ok()).unwrap_or_default(),
            slug: s("slug"),
            title: s("title").unwrap_or_default(),
            size,
            sha256,
            md5: s("md5").unwrap_or_default(),
            machine: s("machine"),
            parent,
            manifest: body.get("manifest").cloned(),
            game_key,
        };
        PENDING.lock().map_err(|_| io("lock"))?.get_or_insert_with(HashMap::new).insert(id.clone(), pending);
        Ok(Prepared { url: Self::file_url(&self.files().join(format!("{id}.part")))?, upload_id: id, headers: HashMap::new() })
    }

    /// As a server's `claim`: the copy must be exactly what was announced; then
    /// it is the newest version, and versions past the newest `keep` go.
    pub fn claim(&self, upload_id: &str) -> Result<Version, ApiError> {
        if !id_ok(upload_id) {
            return Err(ApiError::Rejected("That upload never arrived.".into()));
        }
        let pending = PENDING.lock().map_err(|_| io("lock"))?.get_or_insert_with(HashMap::new).remove(upload_id);
        let part = self.files().join(format!("{upload_id}.part"));
        let Some(p) = pending else {
            let _ = std::fs::remove_file(&part);
            return Err(ApiError::Rejected("That upload never arrived.".into()));
        };
        let arrived = kryoto_saves::bundle::digest(&part).ok();
        if arrived.as_ref().is_none_or(|w| w.size != p.size || !w.sha256.eq_ignore_ascii_case(&p.sha256)) {
            let _ = std::fs::remove_file(&part);
            return Err(ApiError::Rejected("The backup did not arrive as it was sent.".into()));
        }
        let _guard = INDEX.lock().map_err(|_| io("lock"))?;
        let mut index = self.read();
        let game = index.games.entry(p.game_key.clone()).or_default();
        let head = game.versions.first().cloned();
        if head.as_ref().map(|h| h.id.clone()) != p.parent {
            let _ = std::fs::remove_file(&part);
            return Err(ApiError::Conflict(head.map(Box::new)));
        }
        std::fs::rename(&part, self.files().join(format!("{upload_id}.ksave"))).map_err(io)?;
        let version = Version {
            id: upload_id.to_string(),
            size: p.size,
            sha256: p.sha256.to_ascii_lowercase(),
            md5: p.md5,
            file_count: p.manifest.as_ref().and_then(|m| m.get("files")).and_then(|f| f.as_array()).map(|f| f.len() as u64).unwrap_or(0),
            machine: p.machine,
            created_at: now(),
            parent_id: p.parent,
            manifest: p.manifest,
        };
        if !p.aliases.is_empty() {
            game.aliases = p.aliases;
        }
        game.slug = p.slug.or(game.slug.take());
        if !p.title.is_empty() {
            game.title = p.title;
        }
        game.versions.insert(0, version.clone());
        let keep = self.keep as usize;
        for old in game.versions.split_off(keep.min(game.versions.len())) {
            let _ = std::fs::remove_file(self.files().join(format!("{}.ksave", old.id)));
        }
        self.write(&index)?;
        Ok(version)
    }

    pub fn link(&self, version_id: &str) -> Result<Link, ApiError> {
        let index = self.read();
        let v = index.games.values().flat_map(|g| &g.versions).find(|v| v.id == version_id && id_ok(&v.id)).ok_or_else(|| ApiError::Other("No such backup.".into()))?;
        let file = self.files().join(format!("{}.ksave", v.id));
        if !file.is_file() {
            return Err(ApiError::Other(format!("That backup is not in {} any more.", self.dir.display())));
        }
        Ok(Link { url: Self::file_url(&file)?, size: v.size, sha256: v.sha256.clone() })
    }
}

/// A `file:` address from a local store, as a path; None for anything else.
pub(crate) fn path_of(url: &str) -> Option<PathBuf> {
    url.starts_with("file:").then(|| url::Url::parse(url).ok()?.to_file_path().ok()).flatten()
}

/// Copy `from` to `to` in chunks, telling `progress` the bytes so far.
pub(crate) async fn copy(from: &Path, to: &Path, max: u64, progress: impl Fn(u64)) -> Result<u64, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut src = tokio::fs::File::open(from).await.map_err(|e| format!("Could not read {}: {e}", from.display()))?;
    if let Some(dir) = to.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    }
    let tmp = to.with_extension("copying");
    let mut dst = tokio::fs::File::create(&tmp).await.map_err(|e| format!("Could not write {}: {e}", to.display()))?;
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = src.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > max {
            drop(dst);
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err("That backup is bigger than it should be.".into());
        }
        dst.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
        progress(done);
    }
    dst.sync_all().await.map_err(|e| e.to_string())?;
    drop(dst);
    tokio::fs::rename(&tmp, to).await.map_err(|e| e.to_string())?;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryoto-local-saves-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Prepare, put the bytes where it says, claim: what sync.rs does.
    fn back_up(store: &LocalStore, key: &str, bytes: &[u8], parent: Option<&str>) -> Result<Version, ApiError> {
        let src = store.dir.join("src.ksave");
        std::fs::write(&src, bytes).unwrap();
        let w = kryoto_saves::bundle::digest(&src).unwrap();
        let p = store.prepare(serde_json::json!({ "gameKey": key, "title": "Hades", "size": w.size, "sha256": w.sha256, "md5": w.md5, "parentVersion": parent, "manifest": { "files": [{}, {}] } }))?;
        std::fs::copy(&src, path_of(&p.url).unwrap()).unwrap();
        store.claim(&p.upload_id)
    }

    #[test]
    fn versions_newest_first_and_only_the_newest_kept() {
        let dir = temp("keep");
        let store = LocalStore::new(dir.clone(), 2);
        let a = back_up(&store, "slug:hades", b"one", None).unwrap();
        let b = back_up(&store, "slug:hades", b"two", Some(&a.id)).unwrap();
        let c = back_up(&store, "slug:hades", b"three", Some(&b.id)).unwrap();
        let list = store.list().unwrap();
        let ids: Vec<_> = list.games[0].versions.iter().map(|v| v.id.clone()).collect();
        assert_eq!(ids, vec![c.id.clone(), b.id.clone()]);
        assert_eq!((list.games[0].title.as_str(), list.allowance.versions_per_game, c.file_count), ("Hades", 2, 2));
        // The oldest one's file went with it.
        assert!(!dir.join("files").join(format!("{}.ksave", a.id)).exists());
        // Back out of the store, byte for byte.
        let link = store.link(&b.id).unwrap();
        assert_eq!(std::fs::read(path_of(&link.url).unwrap()).unwrap(), b"two");
        assert_eq!(store.head("slug:hades").unwrap().head.map(|h| h.id), Some(c.id));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn another_pc_first_is_a_conflict_and_a_bad_copy_is_refused() {
        let dir = temp("conflict");
        let store = LocalStore::new(dir.clone(), 10);
        let a = back_up(&store, "custom:abc", b"one", None).unwrap();
        // A PC that never saw `a` (parent none) is told what is newest.
        match back_up(&store, "custom:abc", b"other", None) {
            Err(ApiError::Conflict(Some(head))) => assert_eq!(head.id, a.id),
            other => panic!("{other:?}"),
        }
        // Bytes that are not the ones announced never become a version.
        let src = dir.join("x.ksave");
        std::fs::write(&src, b"announced").unwrap();
        let w = kryoto_saves::bundle::digest(&src).unwrap();
        let p = store.prepare(serde_json::json!({ "gameKey": "custom:abc", "size": w.size, "sha256": w.sha256, "parentVersion": a.id })).unwrap();
        std::fs::write(path_of(&p.url).unwrap(), b"something else").unwrap();
        assert!(matches!(store.claim(&p.upload_id), Err(ApiError::Rejected(_))));
        assert_eq!(store.list().unwrap().games[0].versions.len(), 1);
        // Names that are paths are refused.
        assert!(store.prepare(serde_json::json!({ "gameKey": "../x", "size": 1, "sha256": "0".repeat(64) })).is_err());
        assert!(store.link("../index").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
