//! Who is who: safety numbers and verification, the key backup, this
//! account's devices, and the two ways out of "this account has chat on
//! another device" (restore from the backup, or start a new identity).
//!
//! No Tauri in here (the test harness runs it as is). Nothing returned to the
//! shell is a key: safety numbers are fingerprints, and the recovery code is
//! shown once, when it is made, because the person has to write it down.

use km_core::{display_groups, open_backup, safety_number, seal_backup, MasterKey, RecoveryKey, TrustLevel};
use km_proto::gateway::{self as gw, client_frame::Kind as C, server_frame::Kind as S};
use serde::Serialize;
use tokio::sync::Mutex;

use super::engine::{Chat, Engine, OneOff};
use crate::logging;

/// The recovery code, kept in the encrypted chat database so the backup can
/// be refreshed when trust changes (marking someone verified).
const BACKUP_CODE: &str = "backup_code";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyInfo {
    /// Twelve groups of five digits; empty until their key is known.
    pub safety_number: Vec<String>,
    pub verified: bool,
    /// Their key changed and the change is not acknowledged yet: nothing is
    /// sent to them until it is.
    pub pending_change: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub exists: bool,
    pub updated_at_ms: i64,
    /// This PC holds the code and keeps the backup up to date.
    pub kept_here: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyDevice {
    pub device_id: String,
    pub kind: &'static str,
    pub name: String,
    pub certified: bool,
    pub created_at_ms: i64,
    pub last_seen_day_ms: i64,
    pub current: bool,
}

impl Chat {
    fn online(&self) -> Result<std::sync::Arc<super::engine::Client>, String> {
        self.client().ok_or_else(|| "Chat is offline. Try again when it is connected.".to_string())
    }

    pub async fn verify_info(&self, peer: u64) -> Result<VerifyInfo, String> {
        // Look at their devices now: that pins their key the first time, and
        // notices a key that changed since.
        if let Some(client) = self.client() {
            let _ = self.devices_of(&client, peer, true).await;
        }
        let eng = self.engine.lock().await;
        let pending_change = eng.trust.has_pending_change(peer);
        Ok(match eng.trust.pinned(peer) {
            None => VerifyInfo { safety_number: vec![], verified: false, pending_change },
            Some((theirs, level)) => {
                let number = safety_number(eng.user_id, &eng.master.public_key(), peer, &theirs);
                VerifyInfo {
                    safety_number: display_groups(&number).into_iter().map(str::to_string).collect(),
                    verified: level == TrustLevel::Verified,
                    pending_change,
                }
            }
        })
    }

    /// Mark them verified (the numbers matched) or take that back.
    pub async fn verify_mark(&self, peer: u64, verified: bool) -> Result<(), String> {
        {
            let mut eng = self.engine.lock().await;
            let (key, _) = eng.trust.pinned(peer).ok_or("Open the conversation first so their key is known.")?;
            if verified {
                eng.trust.mark_verified(peer, &key).map_err(|e| e.to_string())?;
            } else {
                eng.trust.mark_unverified(peer).map_err(|e| e.to_string())?;
            }
            eng.save()?;
        }
        self.backup_refresh().await;
        Ok(())
    }

    /// The person saw the "security key changed" warning and accepts the new
    /// key. Verification is cleared; sending works again.
    pub async fn identity_ack(&self, peer: u64) -> Result<(), String> {
        let mut eng = self.engine.lock().await;
        eng.trust.acknowledge_change(peer).map_err(|e| e.to_string())?;
        eng.devices.remove(&peer);
        eng.save()
    }

    pub async fn backup_status(&self) -> Result<BackupStatus, String> {
        let client = self.online()?;
        let kept_here = self.engine.lock().await.store.setting(BACKUP_CODE).ok().flatten().is_some_and(|c| !c.is_empty());
        match client.request(C::BackupGet(gw::Empty {})).await.map_err(|e| e.message)? {
            S::Backup(b) => Ok(BackupStatus { exists: !b.blob.is_empty(), updated_at_ms: b.updated_at_ms, kept_here }),
            _ => Err("Unexpected answer.".into()),
        }
    }

    /// Make a new recovery code and upload a backup sealed with it. The code
    /// is returned once, for the person to write down.
    pub async fn backup_create(&self) -> Result<String, String> {
        let client = self.online()?;
        let key = RecoveryKey::generate();
        let code = key.to_code();
        let blob = {
            let eng = self.engine.lock().await;
            seal_backup(&key, eng.user_id, &eng.master, &eng.trust).map_err(|e| e.to_string())?
        };
        client.request(C::BackupPut(gw::BackupPut { blob })).await.map_err(|e| e.message)?;
        self.engine.lock().await.store.set_setting(BACKUP_CODE, &code).map_err(|e| e.to_string())?;
        logging::info("chat", "key backup created");
        Ok(code)
    }

    /// Re-seal the backup with the kept code (trust pins changed). Best effort.
    pub async fn backup_refresh(&self) {
        let Some(client) = self.client() else { return };
        let blob = {
            let eng = self.engine.lock().await;
            let Some(code) = eng.store.setting(BACKUP_CODE).ok().flatten() else { return };
            let Ok(key) = RecoveryKey::from_code(&code) else { return };
            match seal_backup(&key, eng.user_id, &eng.master, &eng.trust) {
                Ok(b) => b,
                Err(_) => return,
            }
        };
        if let Err(e) = client.request(C::BackupPut(gw::BackupPut { blob })).await {
            logging::warn("chat", &format!("could not refresh the key backup: {}", e.message));
        }
    }

    pub async fn backup_delete(&self) -> Result<(), String> {
        let client = self.online()?;
        client.request(C::BackupDelete(gw::Empty {})).await.map_err(|e| e.message)?;
        forget_code(&self.engine).await;
        Ok(())
    }

    pub async fn my_devices(&self) -> Result<Vec<MyDevice>, String> {
        let client = self.online()?;
        let me = self.engine.lock().await.device.device_id().unwrap_or(0);
        match client.request(C::ListMyDevices(gw::Empty {})).await.map_err(|e| e.message)? {
            S::DeviceList(list) => Ok(list
                .devices
                .into_iter()
                .map(|d| MyDevice {
                    device_id: d.device_id.to_string(),
                    kind: if d.kind == 2 { "web" } else { "desktop" },
                    name: d.name,
                    certified: d.certified,
                    created_at_ms: d.created_at_ms,
                    last_seen_day_ms: d.last_seen_day_ms,
                    current: d.device_id == me,
                })
                .collect()),
            _ => Err("Unexpected answer.".into()),
        }
    }

    /// Remove another of this account's devices (this PC is removed with
    /// "Remove chat from this PC", which also deletes the local data).
    pub async fn revoke_device(&self, device_id: u64) -> Result<(), String> {
        let client = self.online()?;
        if self.engine.lock().await.device.device_id() == Some(device_id) {
            return Err("Use \"Remove chat from this PC\" for this one.".into());
        }
        client.request(C::RevokeDevice(gw::RevokeDevice { device_id })).await.map_err(|e| e.message)?;
        let mut eng = self.engine.lock().await;
        let me = eng.user_id;
        eng.devices.remove(&me);
        Ok(())
    }
}

async fn forget_code(engine: &Mutex<Engine>) {
    // An empty value reads as "no code" (setting() returns it, from_code fails).
    let _ = engine.lock().await.store.set_setting(BACKUP_CODE, "");
}

/// Take over the account's identity from its key backup: the code opens the
/// backup, its master key replaces this PC's unused one, and the live
/// connection (started again afterwards) certifies this PC with it.
pub async fn restore(engine: &Mutex<Engine>, token: &str, url: &str, code: &str) -> Result<(), String> {
    let key = RecoveryKey::from_code(code).map_err(|e| e.to_string())?;
    let mut conn = OneOff::open(engine, token, url).await?;
    let answer = conn.ask(C::BackupGet(gw::Empty {})).await;
    conn.close().await;
    let blob = match answer? {
        S::Backup(b) if !b.blob.is_empty() => b.blob,
        S::Backup(_) => return Err("Your account has no key backup. Start a new chat identity instead, or turn on a backup on your other device first.".into()),
        _ => return Err("Unexpected answer.".into()),
    };
    let mut eng = engine.lock().await;
    let user_id = eng.user_id;
    let (master, trust) = open_backup(&key, user_id, &blob).map_err(|e| match e {
        km_core::CoreError::Undecryptable => "That recovery code does not open your backup.".to_string(),
        other => other.to_string(),
    })?;
    adopt(&mut eng, master, Some(trust))?;
    eng.store.set_setting(BACKUP_CODE, &key.to_code()).map_err(|e| e.to_string())?;
    // The past messages come from the account's other devices.
    super::history::mark_new_device(&eng.store);
    logging::info("chat", "restored the chat identity from the key backup");
    Ok(())
}

/// Start over with a new identity for the whole account. Every contact sees
/// "security key changed"; the account's other devices stop working until
/// they are set up again; the old backup is deleted (it holds the old key).
pub async fn reset_identity(engine: &Mutex<Engine>, token: &str, url: &str) -> Result<(), String> {
    let fresh = MasterKey::generate();
    let public = fresh.public_key();
    {
        let mut eng = engine.lock().await;
        adopt(&mut eng, fresh, None)?;
        super::history::forget(&eng.store);
    }
    let mut conn = OneOff::open(engine, token, url).await?;
    let result = async {
        conn.ask(C::PublishMasterKey(gw::PublishMasterKey { public_key: public.to_vec() })).await?;
        let (device_id, signature) = {
            let eng = engine.lock().await;
            (eng.device.device_id().unwrap_or(0), eng.master.certify(&eng.device.keys()).msk_signature.to_vec())
        };
        conn.ask(C::CertifyDevice(gw::CertifyDevice { device_id, signature })).await?;
        let _ = conn.ask(C::BackupDelete(gw::Empty {})).await;
        Ok::<_, String>(())
    }
    .await;
    conn.close().await;
    result?;
    forget_code(engine).await;
    logging::info("chat", "started a new chat identity for this account");
    Ok(())
}

/// Make `master` this PC's identity, keeping existing pins unless a restored
/// set is given.
fn adopt(eng: &mut Engine, master: MasterKey, trust: Option<km_core::TrustStore>) -> Result<(), String> {
    if let Some(t) = trust {
        eng.trust = t;
    }
    let user_id = eng.user_id;
    eng.trust.pin_own(user_id, master.public_key());
    eng.store.save_master_key(&master).map_err(|e| e.to_string())?;
    eng.master = master;
    eng.devices.clear();
    eng.save()
}
