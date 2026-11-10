//! Past messages from one of this account's devices to another (KB-0254).
//!
//! Nothing is kept on the server, so a new device starts empty. It asks the
//! account's other devices (`HistoryRequest`: stored once for devices that
//! are offline, live again on each reconnect while it waits). A device whose
//! player says yes sends its rows in chunks (km-core `history`), live and to
//! that one device only, one at a time: each waits for the new device's
//! `HistoryAck`. The last ack, `done`, goes stored to every device so the
//! others stop asking. The same protocol as the web chat (src/web/engine.ts).

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use km_core::history::{self as kh, own_conversation};
use km_core::{new_message_id, SignedPrekey};
use km_proto::gateway::{self as gw, client_frame::Kind as C, server_frame::Kind as S, SendStatus};
use km_proto::{content::Body, Content, HistoryAck, HistoryChunk, HistoryGroup, HistoryRequest};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use super::engine::Chat;
use super::messaging::{now_ms, SendError};
use super::types::ChatEvent;
use crate::logging;

/// A new device counts as just set up for this long ("Send" is the default).
const FRESH_MS: u64 = 10 * 60 * 1000;
/// Between two chunks at least this long: 20 sends in the gateway's 10 s
/// window, 10 left for chatting.
const PACE: Duration = Duration::from_millis(500);
/// How long a chunk waits for its acknowledgement, and how many tries
/// before the new device counts as gone (until it asks again).
const ACK_WAIT: Duration = Duration::from_secs(15);
const TRIES: u32 = 3;
/// How long "History sent to ..." stays in the list.
const DONE_SHOWN: Duration = Duration::from_secs(60);

const WANT: &str = "history-want";
const START: &str = "history-start";
const SENT: &str = "history-sent";
const CLOSED: &str = "history-closed";

/// On the new device: the history it asked for.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Incoming {
    pub transfer_id: String,
    /// "waiting", "receiving" or "done".
    pub state: String,
    pub from: Option<String>,
    pub received: u32,
    pub total: u32,
    /// The stored request went out (later ones are live only).
    #[serde(default, skip_serializing)]
    pub announced: bool,
}

/// On an older device: a new device asked for this one's history.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Outgoing {
    pub transfer_id: String,
    pub device_id: String,
    pub device_name: String,
    pub created_at_ms: i64,
    pub fresh: bool,
    /// "asking", "sending", "waiting" or "done".
    pub state: String,
    pub sent: u32,
    pub total: u32,
}

/// What the shell shows (`chat-history`).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryView {
    pub incoming: Option<Incoming>,
    pub outgoing: Vec<Outgoing>,
}

struct Offer {
    view: Outgoing,
    device: u64,
    /// "Not now": hidden until the app starts again.
    later: bool,
    chunks: Option<Vec<HistoryChunk>>,
    next: u32,
    tries: u32,
    /// When it finished: "History sent" stays a minute, then goes.
    done_at: Option<std::time::Instant>,
}

/// The transfers this device is part of while chat runs.
#[derive(Default)]
pub struct HistoryRuntime {
    offers: StdMutex<HashMap<String, Offer>>,
    /// Something to send (an answer, a device back online).
    kick: tokio::sync::Notify,
    /// An acknowledgement came in.
    acked: tokio::sync::Notify,
}

/// A new chat device here (registered, or restored with the recovery code):
/// once online it asks the account's other devices for their history.
pub(super) fn mark_new_device(store: &km_store_sqlcipher::Store) {
    let _ = store.set_setting(WANT, "");
    let _ = store.set_setting(START, "1");
}

/// A new identity: the old devices cannot vouch for this one, so no history comes.
pub(super) fn forget(store: &km_store_sqlcipher::Store) {
    let _ = store.set_setting(WANT, "");
    let _ = store.set_setting(START, "");
}

impl Chat {
    fn offers(&self) -> std::sync::MutexGuard<'_, HashMap<String, Offer>> {
        self.history.offers.lock().expect("history offers")
    }

    async fn setting_json<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.engine.lock().await.store.setting(key).ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
    }

    async fn set_setting_json<T: Serialize>(&self, key: &str, value: &T) {
        if let Ok(s) = serde_json::to_string(value) {
            let _ = self.engine.lock().await.store.set_setting(key, &s);
        }
    }

    async fn remember(&self, key: &str, transfer_id: &str) {
        let mut list: Vec<String> = self.setting_json(key).await.unwrap_or_default();
        if !list.iter().any(|t| t == transfer_id) {
            list.push(transfer_id.to_string());
            let skip = list.len().saturating_sub(50);
            self.set_setting_json(key, &list[skip..].to_vec()).await;
        }
    }

    async fn listed(&self, key: &str, transfer_id: &str) -> bool {
        self.setting_json::<Vec<String>>(key).await.unwrap_or_default().iter().any(|t| t == transfer_id)
    }

    pub async fn history_view(&self) -> HistoryView {
        let incoming = self.setting_json::<Incoming>(WANT).await;
        let mut outgoing: Vec<Outgoing> = self.offers().values().filter(|o| !o.later).map(|o| o.view.clone()).collect();
        outgoing.sort_by(|a, b| a.transfer_id.cmp(&b.transfer_id));
        HistoryView { incoming, outgoing }
    }

    async fn emit_history(&self) {
        let view = self.history_view().await;
        (self.emit)(ChatEvent::History(view));
    }

    /// The player's answer to "Send your chat history to ...?"
    pub async fn history_answer(&self, transfer_id: &str, send: bool) -> Result<(), String> {
        {
            let mut offers = self.offers();
            let o = offers.get_mut(transfer_id).ok_or("That request is gone.")?;
            o.later = !send;
        }
        if send {
            self.start_offer(transfer_id).await?;
        }
        self.emit_history().await;
        Ok(())
    }

    pub async fn history_dismiss(&self, transfer_id: &str) {
        if self.setting_json::<Incoming>(WANT).await.is_some_and(|w| w.transfer_id == transfer_id && w.state == "done") {
            let _ = self.engine.lock().await.store.set_setting(WANT, "");
        }
        {
            let mut offers = self.offers();
            if offers.get(transfer_id).is_some_and(|o| o.view.state == "done") {
                offers.remove(transfer_id);
            }
        }
        self.emit_history().await;
    }


    async fn content(&self, body: Body) -> Content {
        let me = self.engine.lock().await.user_id;
        Content { msg_id: new_message_id().to_vec(), conversation_id: own_conversation(me), sent_at_ms: now_ms(), body: Some(body) }
    }

    /// Runs beside a live connection: ask (a new device), ask again (still
    /// waiting), then send whatever has been agreed, until the connection ends.
    pub(super) async fn history_loop(self: Arc<Self>, device_name: String) {
        self.history_announce(&device_name).await;
        for o in self.offers().values_mut() {
            if o.view.state == "waiting" {
                o.view.state = "sending".into();
                o.tries = 0;
            }
        }
        loop {
            self.history_pump().await;
            let _ = tokio::time::timeout(Duration::from_secs(20), self.history.kick.notified()).await;
            let gone = {
                let mut offers = self.offers();
                let before = offers.len();
                offers.retain(|_, o| o.done_at.is_none_or(|t| t.elapsed() < DONE_SHOWN));
                before != offers.len()
            };
            if gone {
                self.emit_history().await;
            }
        }
    }

    async fn history_announce(&self, device_name: &str) {
        let me = self.engine.lock().await.user_id;
        let mut want = self.setting_json::<Incoming>(WANT).await;
        // "History restored" was said when it happened; it does not come back on the next start.
        if want.as_ref().is_some_and(|w| w.state == "done") {
            let _ = self.engine.lock().await.store.set_setting(WANT, "");
            want = None;
            self.emit_history().await;
        }
        let start = self.engine.lock().await.store.setting(START).ok().flatten().is_some_and(|s| s == "1");
        if want.is_none() && start {
            let _ = self.engine.lock().await.store.set_setting(START, "");
            let others = self.my_devices().await.map(|l| l.into_iter().filter(|d| !d.current && d.certified).count()).unwrap_or(0);
            // The account's first chat device has nothing to restore.
            if others > 0 {
                let w = Incoming {
                    transfer_id: hex::encode(new_message_id()),
                    state: "waiting".into(),
                    from: None,
                    received: 0,
                    total: 0,
                    announced: false,
                };
                self.set_setting_json(WANT, &w).await;
                want = Some(w);
                self.emit_history().await;
            }
        }
        let Some(w) = want.filter(|w| w.state != "done") else { return };
        let Ok(transfer_id) = hex::decode(&w.transfer_id) else { return };
        let name: String = device_name.chars().filter(|c| !c.is_control()).take(kh::DEVICE_NAME_LIMIT).collect();
        let content = self.content(Body::HistoryRequest(HistoryRequest { transfer_id, device_name: name })).await;
        // Stored once, for devices that are offline now; live after that.
        match self.deliver(&[me], &content, w.announced, None).await {
            Ok(()) if !w.announced => self.set_setting_json(WANT, &Incoming { announced: true, ..w }).await,
            Ok(()) => {}
            Err(e) => logging::warn("chat", &format!("could not ask for history: {e}")),
        }
    }

    pub(super) async fn on_history_request(&self, device: u64, r: HistoryRequest) {
        let own = self.engine.lock().await.device.device_id();
        if !kh::request_valid(&r) || own == Some(device) {
            return;
        }
        let tid = hex::encode(&r.transfer_id);
        if self.listed(CLOSED, &tid).await {
            return;
        }
        let resumed = {
            let mut offers = self.offers();
            match offers.get_mut(&tid) {
                Some(o) if o.device != device => return,
                Some(o) if matches!(o.view.state.as_str(), "waiting" | "sending") => {
                    // It is back: carry on where it stopped.
                    o.view.state = "sending".into();
                    o.tries = 0;
                    true
                }
                Some(_) => return,
                None => false,
            }
        };
        if resumed {
            self.history.kick.notify_one();
            self.emit_history().await;
            return;
        }
        // Only to a device that is still on the account.
        let Some(dev) = self.my_devices().await.ok().and_then(|l| l.into_iter().find(|d| d.device_id == device.to_string())) else { return };
        let name = if dev.name.trim().is_empty() { r.device_name.clone() } else { dev.name.clone() };
        let view = Outgoing {
            transfer_id: tid.clone(),
            device_id: device.to_string(),
            device_name: if name.trim().is_empty() { "your new device".into() } else { name.chars().take(48).collect() },
            created_at_ms: dev.created_at_ms,
            fresh: now_ms().saturating_sub(dev.created_at_ms.max(0) as u64) < FRESH_MS,
            state: "asking".into(),
            sent: 0,
            total: 0,
        };
        let device_name = view.device_name.clone();
        self.offers().insert(tid.clone(), Offer { view, device, later: false, chunks: None, next: 0, tries: 0, done_at: None });
        // Said yes before the app restarted: no second question.
        if self.listed(SENT, &tid).await {
            if let Err(e) = self.start_offer(&tid).await {
                logging::warn("chat", &format!("could not resume sending history: {e}"));
            }
            self.emit_history().await;
            return;
        }
        self.emit_history().await;
        (self.emit)(ChatEvent::HistoryAsk { device_name });
    }

    async fn start_offer(&self, tid: &str) -> Result<(), String> {
        self.remember(SENT, tid).await;
        let client = self.client().ok_or("Chat is offline. Try again when it is connected.")?;
        let mut groups = Vec::new();
        if let Ok(S::Groups(list)) = client.request(C::GroupList(gw::Empty {})).await {
            let eng = self.engine.lock().await;
            for g in list.groups {
                if let Ok(Some(name)) = eng.store.setting(&format!("group-name-{}", g.group_id)) {
                    groups.push(HistoryGroup { group_id: g.group_id, name });
                }
            }
        }
        let (me, items) = {
            let eng = self.engine.lock().await;
            (eng.user_id, eng.store.history_items().map_err(|e| e.to_string())?)
        };
        let transfer_id = hex::decode(tid).map_err(|_| "Bad request.")?;
        let chunks = kh::chunks(me, &transfer_id, items, groups);
        {
            let mut offers = self.offers();
            let o = offers.get_mut(tid).ok_or("That request is gone.")?;
            o.view.total = chunks.len() as u32;
            o.view.sent = 0;
            o.view.state = "sending".into();
            o.chunks = Some(chunks);
            o.next = 0;
            o.tries = 0;
        }
        self.history.kick.notify_one();
        Ok(())
    }

    /// One message to one of this account's own devices, live.
    async fn send_to_own(&self, device: u64, content: &Content) -> Result<(), SendError> {
        let client = self.client().ok_or(SendError::Offline)?;
        let me = self.engine.lock().await.user_id;
        let mut target = self.devices_of(&client, me, false).await?.into_iter().find(|d| d.keys().device_id == device);
        if target.is_none() {
            target = self.devices_of(&client, me, true).await?.into_iter().find(|d| d.keys().device_id == device);
        }
        let target = target.ok_or(SendError::NoChat)?;
        let has_session = self.engine.lock().await.device.has_session(&target);
        let mut prekey: Option<SignedPrekey> = None;
        if !has_session {
            if let Ok(S::KeysBundle(b)) = client.request(C::KeysClaim(gw::KeysClaim { user_id: me })).await {
                prekey = b.keys.into_iter().find(|k| k.device_id == device).and_then(|k| k.prekey).and_then(|p| super::messaging::prekey(&p));
            }
        }
        let envelope = {
            let mut eng = self.engine.lock().await;
            let e = eng.device.encrypt(&target, prekey.as_ref(), content).map_err(|e| SendError::Failed(e.to_string()))?;
            let _ = eng.save();
            e
        };
        let items = vec![gw::SendItem { recipient_device_id: device, envelope }];
        let answer = client
            .request(C::Send(gw::Send { client_msg_id: content.msg_id.clone(), items, ephemeral: true, group_id: 0, quiet: true }))
            .await
            .map_err(|e| if e.code == "offline" { SendError::Offline } else { SendError::Failed(e.message) })?;
        let S::SendAck(ack) = answer else { return Err(SendError::Failed("Unexpected answer.".into())) };
        match SendStatus::try_from(ack.status).unwrap_or(SendStatus::Unspecified) {
            SendStatus::Accepted => Ok(()),
            SendStatus::RateLimited => Err(SendError::RateLimited),
            _ => {
                // Its keys changed or it left the account: look again next time.
                self.engine.lock().await.devices.remove(&me);
                Err(SendError::Failed("The device did not take it.".into()))
            }
        }
    }

    /// Send every agreed offer's chunks, each once the previous one is acknowledged.
    async fn history_pump(&self) {
        let mut last_send: Option<Instant> = None;
        loop {
            let job = {
                let offers = self.offers();
                offers.iter().find_map(|(tid, o)| match &o.chunks {
                    Some(c) if o.view.state == "sending" && (o.next as usize) < c.len() => Some((tid.clone(), o.device, o.next, c[o.next as usize].clone())),
                    _ => None,
                })
            };
            let Some((tid, device, index, chunk)) = job else { return };
            if let Some(t) = last_send {
                tokio::time::sleep_until(t + PACE).await;
            }
            let content = self.content(Body::HistoryChunk(chunk)).await;
            let sent = self.send_to_own(device, &content).await;
            last_send = Some(Instant::now());
            let acked = match sent {
                Ok(()) => self.wait_ack(&tid, index).await,
                Err(SendError::NoChat) => {
                    // No longer one of this account's devices.
                    self.offers().remove(&tid);
                    self.emit_history().await;
                    continue;
                }
                Err(SendError::Offline) => {
                    if let Some(o) = self.offers().get_mut(&tid) {
                        o.view.state = "waiting".into();
                    }
                    self.emit_history().await;
                    return;
                }
                Err(e) => {
                    logging::warn("chat", &format!("a history chunk did not go: {e}"));
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    false
                }
            };
            if !acked {
                if let Some(o) = self.offers().get_mut(&tid) {
                    o.tries += 1;
                    if o.tries >= TRIES {
                        o.view.state = "waiting".into();
                    }
                }
            }
            self.emit_history().await;
        }
    }

    /// Until the chunk at `index` is acknowledged (true) or the wait runs out.
    async fn wait_ack(&self, tid: &str, index: u32) -> bool {
        let deadline = Instant::now() + ACK_WAIT;
        loop {
            {
                let offers = self.offers();
                match offers.get(tid) {
                    None => return true,
                    Some(o) if o.next > index || o.view.state != "sending" => {
                        return true;
                    }
                    _ => {}
                }
            }
            if tokio::time::timeout_at(deadline, self.history.acked.notified()).await.is_err() {
                return false;
            }
        }
    }

    pub(super) async fn on_history_ack(&self, device: u64, a: HistoryAck) {
        if !kh::ack_valid(&a) {
            return;
        }
        let tid = hex::encode(&a.transfer_id);
        if a.done {
            // It has its history (from here or another device): nothing more to ask or send.
            self.remember(CLOSED, &tid).await;
            let mut offers = self.offers();
            let ours = offers.get(&tid).is_some_and(|o| o.device == device && o.chunks.is_some());
            if ours {
                if let Some(o) = offers.get_mut(&tid) {
                    o.view.state = "done".into();
                    o.view.sent = o.view.total;
                    o.next = o.view.total;
                    o.done_at = Some(std::time::Instant::now());
                }
            } else {
                offers.remove(&tid);
            }
        } else {
            let mut offers = self.offers();
            let Some(o) = offers.get_mut(&tid).filter(|o| o.device == device && o.chunks.is_some()) else { return };
            o.next = a.next.min(o.view.total);
            o.view.sent = o.next;
            o.tries = 0;
        }
        self.history.acked.notify_one();
        self.emit_history().await;
    }

    pub(super) async fn on_history_chunk(&self, device: u64, c: HistoryChunk) {
        let (me, own) = {
            let eng = self.engine.lock().await;
            (eng.user_id, eng.device.device_id())
        };
        let Some(want) = self.setting_json::<Incoming>(WANT).await else { return };
        if !kh::chunk_valid(&c) || own == Some(device) || want.state == "done" || hex::encode(&c.transfer_id) != want.transfer_id {
            return;
        }
        let conversations: Vec<String> = {
            let mut v: Vec<String> = c.items.iter().map(|i| hex::encode(&i.conversation_id)).collect();
            v.sort();
            v.dedup();
            v
        };
        let written = {
            let eng = self.engine.lock().await;
            for g in &c.groups {
                let key = format!("group-name-{}", g.group_id);
                let name: String = g.name.trim().chars().take(km_core::chat::GROUP_NAME_LIMIT).collect();
                if g.group_id != 0 && !name.is_empty() && eng.store.setting(&key).ok().flatten().is_none_or(|s| s.is_empty()) {
                    let _ = eng.store.set_setting(&key, &name);
                }
            }
            eng.store.merge_history(me, c.items).unwrap_or_else(|e| {
                logging::warn("chat", &format!("could not store history: {e}"));
                0
            })
        };
        let done = c.index + 1 >= c.total;
        let from = match want.from.clone() {
            Some(f) => Some(f),
            None => self.my_devices().await.ok().and_then(|l| l.into_iter().find(|d| d.device_id == device.to_string())).map(|d| d.name),
        };
        let next = Incoming {
            state: if done { "done".into() } else { "receiving".into() },
            from: from.or_else(|| Some("your other device".into())),
            received: c.index + 1,
            total: c.total,
            ..want.clone()
        };
        self.set_setting_json(WANT, &next).await;
        if written > 0 {
            (self.emit)(ChatEvent::HistoryMerged { conversations });
        }
        self.emit_history().await;
        let Ok(transfer_id) = hex::decode(&want.transfer_id) else { return };
        let ack = self.content(Body::HistoryAck(HistoryAck { transfer_id, next: c.index + 1, done })).await;
        let sent = if done {
            // Stored, to every device: the others stop asking their players.
            self.deliver(&[me], &ack, false, None).await
        } else {
            self.send_to_own(device, &ack).await
        };
        if let Err(e) = sent {
            logging::warn("chat", &format!("could not acknowledge history: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_gets_camel_case_and_never_the_announce_flag() {
        let v = HistoryView {
            incoming: Some(Incoming { transfer_id: "ab".into(), state: "receiving".into(), from: Some("PC".into()), received: 2, total: 5, announced: true }),
            outgoing: vec![Outgoing {
                transfer_id: "cd".into(),
                device_id: "7".into(),
                device_name: "Web browser".into(),
                created_at_ms: 1,
                fresh: true,
                state: "asking".into(),
                sent: 0,
                total: 0,
            }],
        };
        let s = serde_json::to_string(&v).unwrap();
        assert!(s.contains(r#""transferId":"ab""#) && s.contains(r#""createdAtMs":1"#) && s.contains(r#""deviceName":"Web browser""#), "{s}");
        assert!(!s.contains("announced"), "{s}");
        let back: Incoming = serde_json::from_str(r#"{"transferId":"ab","state":"waiting","from":null,"received":0,"total":0}"#).unwrap();
        assert!(!back.announced);
    }
}
