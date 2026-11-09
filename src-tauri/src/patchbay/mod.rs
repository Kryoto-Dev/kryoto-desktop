//! Patchbay: the player's own services plugged in where Kryoto Desktop would
//! use kryo.to's or none.
//!
//! - A cloud saves server of their own (`saves/api.rs` talks to it instead of
//!   kryo.to, with the same API and a bearer token).
//! - An achievements server of their own (`achievements.rs` here, beside the
//!   Store page's kryo.to sync in `achievements_cloud.rs`).
//! - A debrid service in front of mirror downloads: Real-Debrid (`debrid.rs`)
//!   or TorBox (`torbox.rs`).
//!
//! Whatever goes to the player's own server goes there and nowhere else:
//! kryo.to never sees those saves or achievements, and never sees a token.
//! Tokens are kept in the OS secure store (`secrets.rs`), never in
//! settings.json. The protocol a server has to speak is in
//! `docs/patchbay.md`.

pub mod achievements;
pub mod debrid;
mod secrets;
pub mod torbox;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Runtime};
use zeroize::Zeroizing;

/// What a token belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Slot {
    Saves,
    Achievements,
    RealDebrid,
    Torbox,
}

impl Slot {
    fn key(self) -> &'static str {
        match self {
            Slot::Saves => "saves-token",
            Slot::Achievements => "achievements-token",
            Slot::RealDebrid => "real-debrid-token",
            Slot::Torbox => "torbox-token",
        }
    }
}

/// One of the player's own servers, as Settings has it.
#[derive(Clone)]
pub struct Own {
    /// No trailing slash: paths like `/api/saves` go straight after it.
    pub base: String,
    /// What to call it in messages: the address's host.
    pub host: String,
    pub token: Option<Zeroizing<String>>,
}

fn own(base: &str, slot: Slot) -> Option<Own> {
    let base = crate::settings::normalize_own_endpoint(base).ok().filter(|b| !b.is_empty())?;
    let host = url::Url::parse(&base).ok()?.host_str()?.to_string();
    Some(Own { base, host, token: secrets::read(slot.key()) })
}

/// The player's own cloud saves server, when Settings > Patchbay has one.
pub fn saves<R: Runtime>(app: &AppHandle<R>) -> Option<Own> {
    own(&crate::settings::load(app).saves_endpoint, Slot::Saves)
}

/// The player's own achievements server, when Settings > Patchbay has one.
pub fn achievements<R: Runtime>(app: &AppHandle<R>) -> Option<Own> {
    own(&crate::settings::load(app).achievements_endpoint, Slot::Achievements)
}

/// A debrid service's token, when one is kept.
pub fn token(slot: Slot) -> Option<Zeroizing<String>> {
    secrets::read(slot.key())
}

/// A short, stable name for a server, for the files that remember what each
/// one holds: switching servers (or back to kryo.to) never mixes them up.
pub fn ledger_key(base: &str) -> String {
    hex::encode(&Sha256::digest(base.as_bytes())[..6])
}

/// Settings changed a server: the Cloud pane and game pages show the new one
/// at once, and the achievements loop starts on it without waiting a round.
pub fn endpoints_changed<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit("cloud-saves", crate::saves::current_status(app));
    let _ = app.emit("patchbay", ());
    achievements::wake();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotStatus {
    endpoint: Option<String>,
    token: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    saves: SlotStatus,
    achievements: SlotStatus,
    debrid: Option<String>,
    debrid_token: bool,
    torbox_token: bool,
}

fn status<R: Runtime>(app: &AppHandle<R>) -> Status {
    let s = crate::settings::load(app);
    let slot = |endpoint: &str, slot: Slot| SlotStatus {
        endpoint: Some(endpoint.to_string()).filter(|e| !e.is_empty()),
        token: secrets::read(slot.key()).is_some(),
    };
    Status {
        saves: slot(&s.saves_endpoint, Slot::Saves),
        achievements: slot(&s.achievements_endpoint, Slot::Achievements),
        debrid: Some(s.debrid.clone()).filter(|d| !d.is_empty()),
        debrid_token: secrets::read(Slot::RealDebrid.key()).is_some(),
        torbox_token: secrets::read(Slot::Torbox.key()).is_some(),
    }
}

#[tauri::command(async)]
pub fn patchbay_status(app: AppHandle) -> Status {
    status(&app)
}

/// Keep a token (or forget it, with none). The value never comes back out to
/// the page: `patchbay_status` only says whether there is one.
#[tauri::command(async)]
pub fn patchbay_set_token(app: AppHandle, slot: Slot, token: Option<String>) -> Result<Status, String> {
    let token = token.map(|t| Zeroizing::new(t.trim().to_string())).filter(|t| !t.is_empty());
    match token {
        Some(t) => {
            if t.len() > 4096 || t.chars().any(char::is_control) {
                return Err("That does not look like a token.".into());
            }
            secrets::write(slot.key(), &t)?
        }
        None => secrets::remove(slot.key())?,
    }
    if !matches!(slot, Slot::RealDebrid | Slot::Torbox) {
        endpoints_changed(&app);
    }
    Ok(status(&app))
}

/// What a test of one slot found, in a sentence.
#[tauri::command]
pub async fn patchbay_test(app: AppHandle, slot: Slot) -> Result<String, String> {
    match slot {
        Slot::Saves => {
            let api = crate::saves::api::Api::new(&app).map_err(|e| e.to_string())?;
            let list = api.list().await.map_err(|e| e.to_string())?;
            Ok(format!("Connected. {} game{} stored there.", list.games.len(), if list.games.len() == 1 { "" } else { "s" }))
        }
        Slot::Achievements => {
            let own = achievements(&app).ok_or("Set an achievements server first.")?;
            let n = achievements::check(&own).await?;
            Ok(format!("Connected. {n} game{} with achievements there.", if n == 1 { "" } else { "s" }))
        }
        Slot::RealDebrid => {
            let t = token(Slot::RealDebrid).ok_or("Paste your Real-Debrid API token first.")?;
            let a = debrid::account(&t).await?;
            Ok(a.describe())
        }
        Slot::Torbox => {
            let t = token(Slot::Torbox).ok_or("Paste your TorBox API key first.")?;
            torbox::account(&t).await
        }
    }
}

/// A connection for the player's own servers: no cookies, no redirects to
/// elsewhere (a token must not follow one), the app's own name.
pub(crate) fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

pub(crate) fn authorize(req: reqwest::RequestBuilder, own: &Own) -> reqwest::RequestBuilder {
    match &own.token {
        Some(t) => req.bearer_auth(t.as_str()),
        None => req,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ledger_keys_are_short_and_stable() {
        let a = super::ledger_key("https://saves.example.com");
        assert_eq!(a.len(), 12);
        assert_eq!(a, super::ledger_key("https://saves.example.com"));
        assert_ne!(a, super::ledger_key("https://other.example.com"));
    }
}
