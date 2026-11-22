//! Patchbay's tokens, in the OS secure store (Windows Credential Manager, or
//! the Secret Service - GNOME Keyring, KWallet - on Linux), under a service
//! of their own beside chat's. Never in settings.json, never sent to kryo.to.
//!
//! Read on every request to the player's own servers, so they are kept in
//! memory once read; writing or removing one updates that copy too.
//!
//! A store that forgets across restarts (SteamOS in Game Mode, KB-0368) is
//! found out by `keyring_memory`, and the tokens then go to a file sealed for
//! this machine (`patchbay-secrets.sealed`) so they are kept. Once the store
//! keeps things again, they move back into it.

use std::collections::HashMap;
use std::sync::Mutex;

use keyring::{Entry, Error};
use zeroize::Zeroizing;

/// Debug builds keep their own entries, like their own app data folder.
const SERVICE: &str = if cfg!(debug_assertions) {
    "to.kryo.desktop.dev.patchbay"
} else if crate::channel::PREVIEW {
    "to.kryo.desktop.preview.patchbay"
} else {
    "to.kryo.desktop.patchbay"
};

/// What the store holds for each slot, once read: `None` for a slot with no token.
type Cached = HashMap<&'static str, Option<Zeroizing<String>>>;

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

fn entry(name: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, name).map_err(|e| no_store(&e))
}

fn no_store(e: &Error) -> String {
    if cfg!(target_os = "linux") {
        format!("This PC has no secure store to keep the token in. Install GNOME Keyring or KWallet and sign in again. ({e})")
    } else {
        format!("The token could not be kept in this PC's secure store: {e}")
    }
}

/// The machine-sealed file, where the store forgets.
fn sealed() -> Option<crate::keyring_memory::MachineSealed> {
    crate::keyring_memory::dir().map(|d| crate::keyring_memory::MachineSealed::at(d.join("patchbay-secrets.sealed")))
}

fn file_set(name: &str, value: Option<&str>) -> Result<(), String> {
    let file = sealed().ok_or("The app's folder is not known yet.")?;
    let mut all = file.read_all();
    match value {
        Some(v) => all.insert(name.to_string(), v.to_string()),
        None => all.remove(name),
    };
    file.write_all(&all)
}

pub fn read(name: &'static str) -> Option<Zeroizing<String>> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = cache.get_or_insert_with(HashMap::new);
    if let Some(v) = map.get(name) {
        return v.clone();
    }
    let in_file = sealed().and_then(|f| f.read_all().remove(name)).map(Zeroizing::new);
    let v = if crate::keyring_memory::forgets() {
        in_file
    } else {
        let kept = entry(name).ok().and_then(|e| e.get_password().ok()).map(Zeroizing::new);
        match (kept, in_file) {
            (Some(v), _) => Some(v),
            // Kept in the file while the store forgot: back into the store.
            (None, Some(v)) => {
                if entry(name).and_then(|e| e.set_password(&v).map_err(|e| no_store(&e))).is_ok() {
                    let _ = file_set(name, None);
                }
                Some(v)
            }
            (None, None) => None,
        }
    };
    map.insert(name, v.clone());
    v
}

pub fn write(name: &'static str, value: &str) -> Result<(), String> {
    if crate::keyring_memory::forgets() {
        file_set(name, Some(value))?;
    } else {
        entry(name)?.set_password(value).map_err(|e| no_store(&e))?;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.get_or_insert_with(HashMap::new).insert(name, Some(Zeroizing::new(value.to_string())));
    Ok(())
}

pub fn remove(name: &'static str) -> Result<(), String> {
    let _ = file_set(name, None);
    match entry(name)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => {}
        Err(e) if crate::keyring_memory::forgets() => crate::logging::info("patchbay", &format!("the secure store did not take the removal: {e}")),
        Err(e) => return Err(no_store(&e)),
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.get_or_insert_with(HashMap::new).insert(name, None);
    Ok(())
}
