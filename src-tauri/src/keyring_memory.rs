//! Whether this PC's secure store remembers what it is given, and a sealed
//! file for when it does not (KB-0368).
//!
//! On Linux the secure store is whatever answers the Secret Service: GNOME
//! Keyring, KWallet, KeePassXC. Some answer and then forget at the next
//! restart: SteamOS in Game Mode (KWallet not opened, or the default alias on a
//! session-only collection), and the token for the player's own Patchbay server
//! had to be typed again after every reboot. Saving gave no error, so nothing
//! could tell at the time.
//!
//! So the store is tested over restarts: a marker entry is written once, and a
//! later start that finds it gone (while this app's own note says it was
//! written) means the store forgets. From then on:
//! - Patchbay's tokens go to a file sealed with a key made from this machine's
//!   id (`MachineSealed`, 0600). Weaker than a real secure store (anyone who can
//!   read this account's files and the machine id can open it), but it is kept.
//! - Chat, whose secrets matter more (its sign-in and its database key), uses
//!   its passphrase file instead, as on a PC with no store at all.
//!
//! A later start that finds the marker kept (a working store again) turns this
//! off, and Patchbay's tokens move back into the store. Windows' Credential
//! Manager keeps what it is given: only Linux is tested.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// Debug builds keep their own entries, like their own app data folder.
const SERVICE: &str = if cfg!(debug_assertions) {
    "to.kryo.desktop.dev.keyring-check"
} else if crate::channel::PREVIEW {
    "to.kryo.desktop.preview.keyring-check"
} else {
    "to.kryo.desktop.keyring-check"
};
const MARKER: &str = "kept-across-restarts";
const NOTE: &str = "keyring-check.json";

static DIR: OnceLock<PathBuf> = OnceLock::new();
static FORGETS: OnceLock<bool> = OnceLock::new();

/// This app's data folder, set once at start.
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

pub fn dir() -> Option<&'static PathBuf> {
    DIR.get()
}

#[derive(Default, Serialize, Deserialize)]
struct Note {
    /// The marker went into the store in an earlier run.
    written: bool,
    /// The store lost it since: it forgets across restarts.
    forgets: bool,
}

/// Whether the secure store forgets across restarts. Asked once a run
/// (it talks to the store): not from the window's own thread.
pub fn forgets() -> bool {
    // Not known before the folder is: asked again later rather than kept as no.
    if DIR.get().is_none() {
        return false;
    }
    *FORGETS.get_or_init(|| {
        if !cfg!(target_os = "linux") {
            return false;
        }
        let Some(dir) = DIR.get() else { return false };
        let entry = keyring::Entry::new(SERVICE, MARKER);
        let kept = entry.as_ref().ok().and_then(|e| e.get_password().ok()).is_some();
        let note_path = dir.join(NOTE);
        let note: Note = std::fs::read_to_string(&note_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let written = kept || entry.as_ref().is_ok_and(|e| e.set_password("1").is_ok());
        let forgets = decide(&note, kept);
        if forgets != note.forgets {
            crate::logging::warn(
                "secrets",
                if forgets {
                    "this PC's secure store forgot what it was given at the last restart: Patchbay's tokens are kept in a sealed file instead, and chat asks for its passphrase"
                } else {
                    "this PC's secure store keeps what it is given again: secrets go back into it"
                },
            );
        }
        let next = Note { written: written || note.written, forgets };
        if let Ok(text) = serde_json::to_string(&next) {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(&note_path, text);
        }
        forgets
    })
}

/// The store forgets when the marker was written before and is gone now; a
/// marker found means it keeps. Pure, for the tests.
fn decide(note: &Note, kept: bool) -> bool {
    if kept {
        false
    } else {
        note.written || note.forgets
    }
}

/* ── the machine-sealed file ─────────────────────────────────────────── */

const AAD: &[u8] = b"KRYOTO-DESKTOP-MACHINE-SEALED-V1";

#[derive(Serialize, Deserialize)]
struct Sealed {
    v: u32,
    salt: String,
    nonce: String,
    ct: String,
}

/// Name/value pairs in a file only this machine opens.
pub struct MachineSealed {
    path: PathBuf,
    machine: String,
}

impl MachineSealed {
    pub fn at(path: PathBuf) -> Self {
        Self { path, machine: machine_id() }
    }

    #[cfg(test)]
    fn with_machine(path: PathBuf, machine: &str) -> Self {
        Self { path, machine: machine.to_string() }
    }

    fn key(&self, salt: &[u8]) -> Zeroizing<[u8; 32]> {
        let mut h = Sha256::new();
        h.update(b"kryoto-machine-sealed-v1:");
        h.update(self.machine.as_bytes());
        h.update(b":");
        h.update(salt);
        Zeroizing::new(h.finalize().into())
    }

    pub fn read_all(&self) -> BTreeMap<String, String> {
        self.try_read().unwrap_or_default()
    }

    fn try_read(&self) -> Option<BTreeMap<String, String>> {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD;
        let file: Sealed = serde_json::from_str(&std::fs::read_to_string(&self.path).ok()?).ok()?;
        let salt = b64.decode(file.salt).ok()?;
        let nonce = b64.decode(file.nonce).ok()?;
        let ct = b64.decode(file.ct).ok()?;
        let nonce: [u8; 24] = nonce.try_into().ok()?;
        let key = self.key(&salt);
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref()).ok()?;
        let plain = Zeroizing::new(cipher.decrypt(&XNonce::from(nonce), Payload { msg: &ct, aad: AAD }).ok()?);
        serde_json::from_slice(&plain).ok()
    }

    pub fn write_all(&self, entries: &BTreeMap<String, String>) -> Result<(), String> {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD;
        if entries.is_empty() {
            return match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 24];
        getrandom::fill(&mut salt).map_err(|e| e.to_string())?;
        getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
        let key = self.key(&salt);
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref()).map_err(|e| e.to_string())?;
        let plain = Zeroizing::new(serde_json::to_vec(entries).map_err(|e| e.to_string())?);
        let ct = cipher.encrypt(&XNonce::from(nonce), Payload { msg: &plain, aad: AAD }).map_err(|_| "Could not seal the file.".to_string())?;
        let text = serde_json::to_string(&Sealed { v: 1, salt: b64.encode(salt), nonce: b64.encode(nonce), ct: b64.encode(ct) }).map_err(|e| e.to_string())?;
        write_private(&self.path, text.as_bytes())
    }
}

/// Written whole, readable by this account only.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write;
        let mut f = options.open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// This machine's id (systemd's, or D-Bus's), or the host's name.
fn machine_id() -> String {
    for p in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(id) = std::fs::read_to_string(p) {
            let id = id.trim();
            if !id.is_empty() {
                return id.to_string();
            }
        }
    }
    std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME")).unwrap_or_else(|_| "kryoto".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_that_lost_the_marker_forgets_and_one_that_kept_it_does_not() {
        // First run: nothing known yet.
        assert!(!decide(&Note::default(), false));
        // Written last run, gone now: it forgets.
        assert!(decide(&Note { written: true, forgets: false }, false));
        // Still forgetting.
        assert!(decide(&Note { written: true, forgets: true }, false));
        // The marker survived a restart: it keeps, whatever was noted before.
        assert!(!decide(&Note { written: true, forgets: true }, true));
    }

    #[test]
    fn the_sealed_file_opens_on_this_machine_only_and_is_private() {
        let dir = std::env::temp_dir().join(format!("kryoto-sealed-{}", std::process::id()));
        let path = dir.join("patchbay-secrets.sealed");
        let here = MachineSealed::with_machine(path.clone(), "machine-a");
        let mut entries = BTreeMap::new();
        entries.insert("own-saves".to_string(), "tok-123".to_string());
        here.write_all(&entries).unwrap();
        assert_eq!(here.read_all(), entries);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("tok-123"), "the token is never in the clear");
        // Another machine's id: nothing comes out.
        assert!(MachineSealed::with_machine(path.clone(), "machine-b").read_all().is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Emptied: the file goes.
        here.write_all(&BTreeMap::new()).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
