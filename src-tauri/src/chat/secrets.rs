//! Chat's secrets: in the OS secure store (Windows Credential Manager, macOS
//! Keychain, or the Secret Service - GNOME Keyring, KWallet - on Linux), or,
//! where there is no secure store, in a file sealed with a passphrase the
//! person types at each start.
//!
//! Two kinds of entry, never anything else:
//! - `session`: the device sign-in token chat connects with.
//! - `db-key-<account id>`: the random key the chat database is encrypted with.
//!
//! The passphrase file (`chat/secrets.sealed`) is XChaCha20-Poly1305 under a
//! key from Argon2id (128 MiB, 3 passes); the passphrase itself is never
//! stored, and the derived key lives in memory only while the app runs. There
//! is no silent fallback to an unprotected file.
//!
//! Error messages here never contain a secret.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use keyring::{Entry, Error};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Debug builds keep their own entries, like their own app data folder.
const SERVICE: &str = if cfg!(debug_assertions) { "to.kryo.desktop.dev.chat" } else { "to.kryo.desktop.chat" };

pub const SESSION: &str = "session";

/// Shown when something needs the passphrase and it has not been typed yet.
pub const LOCKED: &str = "Chat is locked. Type your chat passphrase to unlock it.";

const FILE_AAD: &[u8] = b"KRYOTO-DESKTOP-SECRETS-V1";
const MIN_PASSPHRASE: usize = 8;

/// Where the passphrase file lives (set once at start-up).
static FILE: OnceLock<PathBuf> = OnceLock::new();
/// The key derived from the passphrase, while unlocked.
static UNLOCKED: Mutex<Option<Zeroizing<[u8; 32]>>> = Mutex::new(None);
/// Whether the OS secure store works, decided once (probing writes an entry).
static KEYRING: OnceLock<bool> = OnceLock::new();

pub fn set_file_location(path: PathBuf) {
    let _ = FILE.set(path);
}

fn file_path() -> Result<&'static PathBuf, String> {
    FILE.get().ok_or_else(|| "The chat folder is not known yet.".to_string())
}

/// Is there a working OS secure store? Writes, reads back and deletes a probe.
pub fn keyring_available() -> bool {
    *KEYRING.get_or_init(|| {
        const PROBE: &str = "probe";
        let ok = keyring_write(PROBE, "1").is_ok() && keyring_read(PROBE).ok().flatten().is_some_and(|v| v.as_str() == "1");
        let _ = keyring_remove(PROBE);
        ok
    })
}

/// Where secrets go: the OS store, or the passphrase file.
pub enum Backend {
    Keyring,
    /// No OS store. `exists`: a passphrase was set before (unlock), else one
    /// has to be chosen (create). `unlocked`: typed in this run.
    Passphrase { exists: bool, unlocked: bool },
}

pub fn backend() -> Backend {
    if keyring_available() {
        return Backend::Keyring;
    }
    let exists = file_path().map(|p| p.exists()).unwrap_or(false);
    let unlocked = UNLOCKED.lock().expect("secrets").is_some();
    Backend::Passphrase { exists, unlocked }
}

// ---- OS store -----------------------------------------------------------------

fn entry(name: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, name).map_err(|e| format!("The secure store is not available: {e}"))
}

fn keyring_read(name: &str) -> Result<Option<Zeroizing<String>>, String> {
    match entry(name)?.get_password() {
        Ok(v) => Ok(Some(Zeroizing::new(v))),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read from the secure store: {e}")),
    }
}

fn keyring_write(name: &str, value: &str) -> Result<(), String> {
    entry(name)?.set_password(value).map_err(|e| format!("Could not save to the secure store: {e}"))
}

fn keyring_remove(name: &str) -> Result<(), String> {
    match entry(name)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Could not remove from the secure store: {e}")),
    }
}

// ---- passphrase file --------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct SealedFile {
    v: u32,
    salt: String,
    nonce: String,
    ct: String,
}

type Entries = BTreeMap<String, String>;

fn derive(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    let params = argon2::Params::new(128 * 1024, 3, 1, Some(32)).map_err(|e| e.to_string())?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon.hash_password_into(passphrase.as_bytes(), salt, key.as_mut_slice()).map_err(|e| e.to_string())?;
    Ok(key)
}

fn seal(key: &[u8; 32], salt: &[u8], entries: &Entries) -> Result<String, String> {
    let plain = Zeroizing::new(serde_json::to_vec(entries).map_err(|e| e.to_string())?);
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| "The system random source failed.".to_string())?;
    let ct = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| e.to_string())?
        .encrypt(&XNonce::from(nonce), Payload { msg: &plain, aad: FILE_AAD })
        .map_err(|_| "Could not seal the chat secrets.".to_string())?;
    serde_json::to_string(&SealedFile { v: 1, salt: hex::encode(salt), nonce: hex::encode(nonce), ct: hex::encode(ct) }).map_err(|e| e.to_string())
}

/// Open a sealed file with an already derived key (`None` = wrong key).
fn open_with(key: &[u8; 32], text: &str) -> Result<Option<Entries>, String> {
    let f: SealedFile = serde_json::from_str(text).map_err(|_| "The chat secrets file is damaged.".to_string())?;
    let nonce: [u8; 24] = hex::decode(&f.nonce).ok().and_then(|n| n.try_into().ok()).ok_or("The chat secrets file is damaged.")?;
    let ct = hex::decode(&f.ct).map_err(|_| "The chat secrets file is damaged.".to_string())?;
    let Ok(plain) = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| e.to_string())?
        .decrypt(&XNonce::from(nonce), Payload { msg: &ct, aad: FILE_AAD })
    else {
        return Ok(None);
    };
    let plain = Zeroizing::new(plain);
    serde_json::from_slice(&plain).map(Some).map_err(|_| "The chat secrets file is damaged.".to_string())
}

fn salt_of(text: &str) -> Result<Vec<u8>, String> {
    let f: SealedFile = serde_json::from_str(text).map_err(|_| "The chat secrets file is damaged.".to_string())?;
    hex::decode(f.salt).map_err(|_| "The chat secrets file is damaged.".to_string())
}

/// Type the passphrase: unlock the existing file, or create it with this
/// passphrase if there is none yet.
pub fn unlock(passphrase: &str) -> Result<(), String> {
    let path = file_path()?;
    if path.exists() {
        let text = std::fs::read_to_string(path).map_err(|e| format!("Could not read the chat secrets: {e}"))?;
        let key = derive(passphrase, &salt_of(&text)?)?;
        if open_with(&key, &text)?.is_none() {
            return Err("That passphrase is not right.".into());
        }
        *UNLOCKED.lock().expect("secrets") = Some(key);
        return Ok(());
    }
    if passphrase.chars().count() < MIN_PASSPHRASE {
        return Err(format!("Choose a passphrase of at least {MIN_PASSPHRASE} characters."));
    }
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| "The system random source failed.".to_string())?;
    let key = derive(passphrase, &salt)?;
    let text = seal(&key, &salt, &Entries::new())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, text).map_err(|e| format!("Could not save the chat secrets: {e}"))?;
    *UNLOCKED.lock().expect("secrets") = Some(key);
    Ok(())
}

fn with_file<T>(f: impl FnOnce(&mut Entries) -> T, write_back: bool) -> Result<T, String> {
    let key = UNLOCKED.lock().expect("secrets").clone().ok_or(LOCKED)?;
    let path = file_path()?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("Could not read the chat secrets: {e}"))?;
    let mut entries = open_with(&key, &text)?.ok_or("The chat secrets no longer open with this passphrase.")?;
    let out = f(&mut entries);
    if write_back {
        let sealed = seal(&key, &salt_of(&text)?, &entries)?;
        // Write next to it, then replace: a crash never leaves half a file.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, sealed).map_err(|e| format!("Could not save the chat secrets: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("Could not save the chat secrets: {e}"))?;
    }
    for v in entries.values_mut() {
        zeroize::Zeroize::zeroize(v);
    }
    Ok(out)
}

/// Forget everything: the file and the in-memory key (chat removed).
pub fn wipe_file() {
    *UNLOCKED.lock().expect("secrets") = None;
    if let Ok(p) = file_path() {
        let _ = std::fs::remove_file(p);
    }
}

// ---- the API the rest of chat uses ---------------------------------------------

pub fn read(name: &str) -> Result<Option<Zeroizing<String>>, String> {
    match backend() {
        Backend::Keyring => keyring_read(name),
        Backend::Passphrase { exists: false, .. } => Ok(None),
        Backend::Passphrase { .. } => with_file(|e| e.get(name).map(|v| Zeroizing::new(v.clone())), false),
    }
}

pub fn write(name: &str, value: &str) -> Result<(), String> {
    match backend() {
        Backend::Keyring => keyring_write(name, value),
        Backend::Passphrase { .. } => with_file(|e| drop(e.insert(name.to_string(), value.to_string())), true),
    }
}

pub fn remove(name: &str) -> Result<(), String> {
    match backend() {
        Backend::Keyring => keyring_remove(name),
        Backend::Passphrase { exists: false, .. } => Ok(()),
        Backend::Passphrase { .. } => with_file(|e| drop(e.remove(name)), true),
    }
}

pub fn db_key_name(user_id: u64) -> String {
    format!("db-key-{user_id}")
}

/// The chat database key for an account: made on first use, then reused.
pub fn db_key(user_id: u64) -> Result<Zeroizing<[u8; 32]>, String> {
    let name = db_key_name(user_id);
    if let Some(hex_key) = read(&name)? {
        let mut key = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(hex_key.as_bytes(), key.as_mut_slice()).map_err(|_| "The saved chat key is damaged.".to_string())?;
        return Ok(key);
    }
    let mut key = Zeroizing::new([0u8; 32]);
    getrandom::fill(key.as_mut_slice()).map_err(|_| "The system random source failed.".to_string())?;
    write(&name, &Zeroizing::new(hex::encode(key.as_slice())))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_file_opens_only_with_its_key_and_holds_no_plaintext() {
        let salt = [3u8; 16];
        let key = derive("correct horse battery", &salt).unwrap();
        let mut e = Entries::new();
        e.insert("session".into(), "secret-token-value".into());
        let text = seal(&key, &salt, &e).unwrap();
        assert!(!text.contains("secret-token-value"));
        assert_eq!(open_with(&key, &text).unwrap().unwrap().get("session").map(String::as_str), Some("secret-token-value"));
        let wrong = derive("wrong horse battery", &salt).unwrap();
        assert!(open_with(&wrong, &text).unwrap().is_none());
        assert_eq!(salt_of(&text).unwrap(), salt.to_vec());
    }
}
