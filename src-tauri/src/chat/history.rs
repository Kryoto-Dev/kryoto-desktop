//! Past messages from one of this account's devices to another (KB-0254).
//!
//! Nothing is kept on the server, so a new device starts empty. It asks the
//! account's other devices (`HistoryRequest`: stored once for devices that
//! are offline, then live again: on each reconnect, every `ASK_AGAIN_MS`
//! while it waits or the transfer stalls, and sooner when another of its
//! devices says it is in use). A device keeps each request it was asked
//! (`history-offers`) until it is answered, done or expired, so closing it
//! before answering loses nothing. A device whose player says yes sends its
//! rows in chunks (km-core `history`), live and to that one device only, one
//! at a time: each waits for the new device's `HistoryAck`. The last ack,
//! `done`, goes stored to every device so the others stop asking. The same
//! protocol and rules as the web chat (src/web/engine.ts, history-rules.ts).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
/// While it waits, the new device asks its online devices again this often (live, never stored).
const ASK_AGAIN_MS: u64 = 45_000;
/// Sooner when one of them says it is in use, but never more often than this.
const ASK_AGAIN_MIN_MS: u64 = 10_000;
/// A transfer that brought nothing for this long has stalled: ask again, and another device may take over.
const STALL_MS: u64 = 60_000;
/// Nobody answered for this long: the waiting line says what to do.
const QUIET_MS: u64 = 2 * 60_000;
/// A request is given up after this long, on both ends.
const EXPIRE_MS: u64 = 7 * 24 * 3600 * 1000;
/// Between two chunks at least this long: 20 sends in the gateway's 10 s
/// window, 10 left for chatting.
const PACE: Duration = Duration::from_millis(500);
/// How long a chunk waits for its acknowledgement, and how many tries
/// before the new device counts as gone (until it asks again).
const ACK_WAIT: Duration = Duration::from_secs(15);
const TRIES: u32 = 3;
/// How long "History sent to ..." stays in the list.
const DONE_SHOWN: Duration = Duration::from_secs(60);
/// How often the loop beside a connection looks at the clock (ask again, quiet, expiry).
const TICK: Duration = Duration::from_secs(15);
/// At most this many kept requests.
const KEPT: usize = 20;

const WANT: &str = "history-want";
const START: &str = "history-start";
const SENT: &str = "history-sent";
const CLOSED: &str = "history-closed";
const OFFERS: &str = "history-offers";

/// On the new device: the history it asked for, as kept in its store.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Want {
    pub transfer_id: String,
    /// "waiting", "receiving" or "done".
    pub state: String,
    pub from: Option<String>,
    pub received: u32,
    pub total: u32,
    /// The stored request went out (later ones are live only).
    #[serde(default)]
    pub announced: bool,
    /// When it first asked (requests from 0.3.1 have none: counted from the first start that sees them).
    #[serde(default)]
    pub asked_at_ms: Option<u64>,
    /// The device it takes the history from: the first that sends.
    #[serde(default)]
    pub from_device: Option<u64>,
    /// When the last part from that device came.
    #[serde(default)]
    pub last_at_ms: Option<u64>,
}

/// What the new device shows (lib/chat.ts `HistoryIncoming`).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Incoming {
    pub transfer_id: String,
    pub state: String,
    pub from: Option<String>,
    pub received: u32,
    pub total: u32,
    pub asked_at_ms: u64,
    /// Waiting and nobody answered for a while, or receiving and it stalled.
    pub quiet: bool,
    /// Waited so long it no longer asks.
    pub expired: bool,
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

/// On an older device: a request it was asked, kept until it is answered, done or expired.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SavedOffer {
    pub transfer_id: String,
    pub device_id: String,
    pub device_name: String,
    pub created_at_ms: i64,
    /// When this device first saw it.
    pub seen_at_ms: u64,
    /// "Not now": not asked again for this request (a new request from that device is).
    pub later: bool,
}

// ---- the rules (the same as src/web/history-rules.ts) ------------------------

fn expired(w: &Want, now: u64) -> bool {
    w.state != "done" && now.saturating_sub(w.asked_at_ms.unwrap_or(now)) >= EXPIRE_MS
}

fn incoming_view(w: &Want, now: u64) -> Incoming {
    let asked = w.asked_at_ms.unwrap_or(now);
    let quiet = match w.state.as_str() {
        "waiting" => now.saturating_sub(asked) >= QUIET_MS,
        "receiving" => now.saturating_sub(w.last_at_ms.unwrap_or(asked)) >= STALL_MS,
        _ => false,
    };
    Incoming {
        transfer_id: w.transfer_id.clone(),
        state: w.state.clone(),
        from: w.from.clone(),
        received: w.received,
        total: w.total,
        asked_at_ms: asked,
        quiet,
        expired: expired(w, now),
    }
}

/// Whether the new device asks its online devices again now.
fn ask_again_due(w: Option<&Want>, now: u64, last_ask: u64, nudged: bool) -> bool {
    let Some(w) = w else { return false };
    if w.state == "done" || expired(w, now) {
        return false;
    }
    if w.state == "receiving" && now.saturating_sub(w.last_at_ms.unwrap_or(0)) < STALL_MS {
        return false;
    }
    let gap = now.saturating_sub(last_ask);
    gap >= ASK_AGAIN_MS || (nudged && gap >= ASK_AGAIN_MIN_MS)
}

#[derive(Debug, PartialEq, Eq)]
struct ChunkStep {
    /// It counts (the device the history comes from, or the first, or one
    /// taking over from a stalled one); otherwise a second device said yes
    /// too: its rows are merged by id and it is told it can stop.
    progress: bool,
    done: bool,
    ack_next: u32,
    want: Want,
}

fn on_chunk(w: Option<&Want>, transfer_id: &str, sender: u64, me: Option<u64>, index: u32, total: u32, now: u64) -> Option<ChunkStep> {
    let w = w?;
    if w.transfer_id != transfer_id || w.state == "done" || me == Some(sender) || total < 1 || index >= total {
        return None;
    }
    if let Some(lock) = w.from_device {
        if lock != sender && now.saturating_sub(w.last_at_ms.unwrap_or(0)) < STALL_MS {
            return Some(ChunkStep { progress: false, done: false, ack_next: total, want: w.clone() });
        }
    }
    let done = index + 1 >= total;
    let want = Want {
        state: if done { "done".into() } else { "receiving".into() },
        from: if w.from_device == Some(sender) { w.from.clone() } else { None },
        from_device: Some(sender),
        received: index + 1,
        total,
        last_at_ms: Some(now),
        ..w.clone()
    };
    Some(ChunkStep { progress: true, done, ack_next: index + 1, want })
}

/// The kept requests still worth asking, and the expired ones (which go on the closed list).
fn kept_offers(saved: Vec<SavedOffer>, closed: &[String], now: u64) -> (Vec<SavedOffer>, Vec<String>) {
    let mut keep = Vec::new();
    let mut gone = Vec::new();
    for s in saved {
        if closed.contains(&s.transfer_id) {
            continue;
        }
        if now.saturating_sub(s.seen_at_ms) >= EXPIRE_MS {
            gone.push(s.transfer_id);
        } else {
            keep.push(s);
        }
    }
    (keep, gone)
}

fn with_offer(mut saved: Vec<SavedOffer>, o: SavedOffer) -> Vec<SavedOffer> {
    saved.retain(|s| s.transfer_id != o.transfer_id);
    saved.push(o);
    let skip = saved.len().saturating_sub(KEPT);
    saved.split_off(skip)
}

#[derive(Debug, PartialEq, Eq)]
enum Repeat {
    /// Agreed, but nothing cut yet (a restart lost it).
    Start,
    /// Carry on from the last acknowledged part.
    Resume,
    /// Told to stop (another device sent) or ran out, yet still asked: from the start.
    Restart,
    None,
}

fn on_repeat_request(state: &str, has_chunks: bool, next: u32, total: u32) -> Repeat {
    if state != "sending" && state != "waiting" {
        return Repeat::None;
    }
    if !has_chunks {
        return Repeat::Start;
    }
    if next >= total {
        Repeat::Restart
    } else {
        Repeat::Resume
    }
}

struct Offer {
    view: Outgoing,
    device: u64,
    /// "Not now": not asked again for this request (kept, so not after a restart either).
    later: bool,
    seen_at_ms: u64,
    chunks: Option<Vec<HistoryChunk>>,
    next: u32,
    tries: u32,
    /// When it finished: "History sent" stays a minute, then goes.
    done_at: Option<std::time::Instant>,
}

impl Offer {
    fn new(s: &SavedOffer, device: u64) -> Self {
        let view = Outgoing {
            transfer_id: s.transfer_id.clone(),
            device_id: s.device_id.clone(),
            device_name: s.device_name.clone(),
            created_at_ms: s.created_at_ms,
            fresh: now_ms().saturating_sub(s.created_at_ms.max(0) as u64) < FRESH_MS,
            state: "asking".into(),
            sent: 0,
            total: 0,
        };
        Offer { view, device, later: s.later, seen_at_ms: s.seen_at_ms, chunks: None, next: 0, tries: 0, done_at: None }
    }

    fn saved(&self) -> SavedOffer {
        SavedOffer {
            transfer_id: self.view.transfer_id.clone(),
            device_id: self.view.device_id.clone(),
            device_name: self.view.device_name.clone(),
            created_at_ms: self.view.created_at_ms,
            seen_at_ms: self.seen_at_ms,
            later: self.later,
        }
    }
}

/// The transfers this device is part of while chat runs.
#[derive(Default)]
pub struct HistoryRuntime {
    offers: StdMutex<HashMap<String, Offer>>,
    /// Something to send (an answer, a device back online, one in use).
    kick: tokio::sync::Notify,
    /// An acknowledgement came in.
    acked: tokio::sync::Notify,
    /// When this (new) device last asked.
    asked_at: AtomicU64,
    /// Another of this account's devices said it is in use since then.
    nudged: AtomicBool,
    /// What the list last showed of the request (to tell it when it turns quiet or expires).
    shown: StdMutex<String>,
}

impl HistoryRuntime {
    /// Another of this account's devices is in use: a request still waiting goes to it now.
    pub fn nudge(&self) {
        self.nudged.store(true, Ordering::Relaxed);
        self.kick.notify_one();
    }
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

    async fn want(&self) -> Option<Want> {
        self.setting_json::<Want>(WANT).await
    }

    async fn saved_offers(&self) -> Vec<SavedOffer> {
        self.setting_json(OFFERS).await.unwrap_or_default()
    }

    async fn save_offer(&self, tid: &str) {
        let Some(s) = self.offers().get(tid).map(Offer::saved) else { return };
        let list = with_offer(self.saved_offers().await, s);
        self.set_setting_json(OFFERS, &list).await;
    }

    async fn unsave_offer(&self, tid: &str) {
        let list = self.saved_offers().await;
        if list.iter().any(|s| s.transfer_id == tid) {
            let rest: Vec<SavedOffer> = list.into_iter().filter(|s| s.transfer_id != tid).collect();
            self.set_setting_json(OFFERS, &rest).await;
        }
    }

    pub async fn history_view(&self) -> HistoryView {
        let incoming = self.want().await.map(|w| incoming_view(&w, now_ms()));
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
        // Kept: "Not now" holds after a restart too.
        self.save_offer(transfer_id).await;
        if send {
            self.start_offer(transfer_id).await?;
        }
        self.emit_history().await;
        Ok(())
    }

    /// On the new device, when nobody answered: ask every other device again,
    /// as a new request, so one whose player said "Not now" asks once more.
    pub async fn history_ask_again(&self, device_name: &str) -> Result<(), String> {
        if self.want().await.is_some_and(|w| w.state != "waiting") {
            return Ok(());
        }
        let w = Want {
            transfer_id: hex::encode(new_message_id()),
            state: "waiting".into(),
            from: None,
            received: 0,
            total: 0,
            announced: false,
            asked_at_ms: Some(now_ms()),
            from_device: None,
            last_at_ms: None,
        };
        self.set_setting_json(WANT, &w).await;
        self.emit_history().await;
        if self.client().is_some() {
            self.history_ask(&w, device_name).await;
        }
        Ok(())
    }

    pub async fn history_dismiss(&self, transfer_id: &str) {
        if self.want().await.is_some_and(|w| w.transfer_id == transfer_id && (w.state == "done" || expired(&w, now_ms()))) {
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
    /// waiting), pick up kept requests, then send whatever has been agreed and
    /// look at the clock now and then, until the connection ends.
    pub(super) async fn history_loop(self: Arc<Self>, device_name: String) {
        self.history_announce(&device_name).await;
        self.restore_offers().await;
        let resume: Vec<String> = {
            let mut offers = self.offers();
            let mut start = Vec::new();
            for (tid, o) in offers.iter_mut() {
                if o.view.state == "waiting" {
                    o.view.state = "sending".into();
                    o.tries = 0;
                    if o.chunks.is_none() {
                        start.push(tid.clone());
                    }
                }
            }
            start
        };
        for tid in resume {
            if let Err(e) = self.start_offer(&tid).await {
                logging::warn("chat", &format!("could not resume sending history: {e}"));
            }
        }
        loop {
            self.history_pump().await;
            let _ = tokio::time::timeout(TICK, self.history.kick.notified()).await;
            self.history_tick(&device_name).await;
        }
    }

    /// Ask again when due, say when a request turns quiet, let old requests and "History sent" go.
    async fn history_tick(&self, device_name: &str) {
        let now = now_ms();
        let want = self.want().await;
        let nudged = self.history.nudged.load(Ordering::Relaxed);
        if ask_again_due(want.as_ref(), now, self.history.asked_at.load(Ordering::Relaxed), nudged) {
            if let Some(w) = &want {
                self.history_ask(w, device_name).await;
            }
        } else {
            self.history.nudged.store(false, Ordering::Relaxed);
        }
        let shown = want.as_ref().map(|w| incoming_view(w, now)).map(|v| format!("{}:{}:{}:{}", v.transfer_id, v.state, v.quiet, v.expired)).unwrap_or_default();
        let mut changed = {
            let mut last = self.history.shown.lock().expect("history shown");
            let changed = *last != shown;
            *last = shown;
            changed
        };
        let (finished, stale) = {
            let mut offers = self.offers();
            let before = offers.len();
            offers.retain(|_, o| o.done_at.is_none_or(|t| t.elapsed() < DONE_SHOWN));
            let stale: Vec<String> = offers.iter().filter(|(_, o)| o.view.state == "asking" && now.saturating_sub(o.seen_at_ms) >= EXPIRE_MS).map(|(t, _)| t.clone()).collect();
            for t in &stale {
                offers.remove(t);
            }
            (before != offers.len() + stale.len(), stale)
        };
        for t in &stale {
            self.remember(CLOSED, t).await;
            self.unsave_offer(t).await;
        }
        changed |= finished || !stale.is_empty();
        if changed {
            self.emit_history().await;
        }
    }

    /// On each connect: a new device starts asking; one still waiting asks again.
    async fn history_announce(&self, device_name: &str) {
        let mut want = self.want().await;
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
                let w = Want {
                    transfer_id: hex::encode(new_message_id()),
                    state: "waiting".into(),
                    from: None,
                    received: 0,
                    total: 0,
                    announced: false,
                    asked_at_ms: Some(now_ms()),
                    from_device: None,
                    last_at_ms: None,
                };
                self.set_setting_json(WANT, &w).await;
                want = Some(w);
                self.emit_history().await;
            }
        }
        // A request from 0.3.1 has no time: it counts from now.
        if let Some(w) = want.as_mut().filter(|w| w.asked_at_ms.is_none()) {
            w.asked_at_ms = Some(now_ms());
            self.set_setting_json(WANT, &*w).await;
        }
        if let Some(w) = want {
            self.history_ask(&w, device_name).await;
        }
    }

    /// Ask this account's other devices: stored the first time (for devices offline now), live after that.
    async fn history_ask(&self, w: &Want, device_name: &str) {
        if w.state == "done" || expired(w, now_ms()) {
            return;
        }
        let Ok(transfer_id) = hex::decode(&w.transfer_id) else { return };
        self.history.asked_at.store(now_ms(), Ordering::Relaxed);
        self.history.nudged.store(false, Ordering::Relaxed);
        let me = self.engine.lock().await.user_id;
        let name: String = device_name.chars().filter(|c| !c.is_control()).take(kh::DEVICE_NAME_LIMIT).collect();
        let content = self.content(Body::HistoryRequest(HistoryRequest { transfer_id, device_name: name })).await;
        match self.deliver(&[me], &content, w.announced, None).await {
            Ok(()) if !w.announced => {
                if let Some(now) = self.want().await.filter(|n| n.transfer_id == w.transfer_id) {
                    self.set_setting_json(WANT, &Want { announced: true, ..now }).await;
                }
            }
            Ok(()) => {}
            Err(e) => logging::warn("chat", &format!("could not ask for history: {e}")),
        }
    }

    /// Requests this device was asked before it closed: asked again (or sent on, when the player had said yes).
    async fn restore_offers(&self) {
        let saved = self.saved_offers().await;
        if saved.is_empty() {
            return;
        }
        let closed: Vec<String> = self.setting_json(CLOSED).await.unwrap_or_default();
        let count = saved.len();
        let (keep, gone) = kept_offers(saved, &closed, now_ms());
        for t in &gone {
            self.remember(CLOSED, t).await;
        }
        if keep.len() != count {
            self.set_setting_json(OFFERS, &keep).await;
        }
        let missing: Vec<SavedOffer> = {
            let offers = self.offers();
            keep.into_iter().filter(|s| !offers.contains_key(&s.transfer_id)).collect()
        };
        if missing.is_empty() {
            return;
        }
        let Ok(devices) = self.my_devices().await else { return };
        let mut start = Vec::new();
        for s in missing {
            let Some(device) = s.device_id.parse::<u64>().ok().filter(|_| devices.iter().any(|d| d.device_id == s.device_id)) else {
                // That device left the account: nothing to send it.
                self.unsave_offer(&s.transfer_id).await;
                continue;
            };
            if self.listed(SENT, &s.transfer_id).await {
                start.push(s.transfer_id.clone());
            }
            self.offers().entry(s.transfer_id.clone()).or_insert_with(|| Offer::new(&s, device));
        }
        for tid in start {
            if let Err(e) = self.start_offer(&tid).await {
                logging::warn("chat", &format!("could not resume sending history: {e}"));
            }
        }
        self.emit_history().await;
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
        let repeat = {
            let mut offers = self.offers();
            match offers.get_mut(&tid) {
                Some(o) if o.device != device => return,
                Some(o) => {
                    // It is still waiting (or back): carry on, or start over when it never got the rest.
                    let step = on_repeat_request(&o.view.state, o.chunks.is_some(), o.next, o.view.total);
                    match step {
                        Repeat::Resume | Repeat::Restart => {
                            if step == Repeat::Restart {
                                o.next = 0;
                                o.view.sent = 0;
                            }
                            o.view.state = "sending".into();
                            o.tries = 0;
                        }
                        Repeat::Start | Repeat::None => {}
                    }
                    Some(step)
                }
                None => None,
            }
        };
        match repeat {
            Some(Repeat::Start) => {
                if let Err(e) = self.start_offer(&tid).await {
                    logging::warn("chat", &format!("could not resume sending history: {e}"));
                }
                self.emit_history().await;
                return;
            }
            Some(Repeat::Resume | Repeat::Restart) => {
                self.history.kick.notify_one();
                self.emit_history().await;
                return;
            }
            Some(Repeat::None) => return,
            None => {}
        }
        // Kept from before this device restarted (its answer, its first sighting), if it got here first.
        let kept = self.saved_offers().await.into_iter().find(|s| s.transfer_id == tid);
        // Only to a device that is still on the account.
        let Some(dev) = self.my_devices().await.ok().and_then(|l| l.into_iter().find(|d| d.device_id == device.to_string())) else { return };
        let name = if dev.name.trim().is_empty() { r.device_name.clone() } else { dev.name.clone() };
        let saved = SavedOffer {
            transfer_id: tid.clone(),
            device_id: device.to_string(),
            device_name: if name.trim().is_empty() { "your new device".into() } else { name.chars().take(48).collect() },
            created_at_ms: dev.created_at_ms,
            seen_at_ms: kept.as_ref().map_or_else(now_ms, |k| k.seen_at_ms),
            later: kept.as_ref().is_some_and(|k| k.later),
        };
        let device_name = saved.device_name.clone();
        let later = saved.later;
        {
            let mut offers = self.offers();
            if offers.contains_key(&tid) {
                return;
            }
            offers.insert(tid.clone(), Offer::new(&saved, device));
        }
        self.save_offer(&tid).await;
        // Said yes before the app restarted: no second question.
        if self.listed(SENT, &tid).await {
            if let Err(e) = self.start_offer(&tid).await {
                logging::warn("chat", &format!("could not resume sending history: {e}"));
            }
            self.emit_history().await;
            return;
        }
        self.emit_history().await;
        if kept.is_none() && !later {
            (self.emit)(ChatEvent::HistoryAsk { device_name });
        }
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
                    self.unsave_offer(&tid).await;
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
            self.unsave_offer(&tid).await;
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
            // `next` = all of them: another device is sending it already, this one can stop.
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
        if !kh::chunk_valid(&c) {
            return;
        }
        let want = self.want().await;
        let Some(step) = on_chunk(want.as_ref(), &hex::encode(&c.transfer_id), device, own, c.index, c.total, now_ms()) else { return };
        let conversations: Vec<String> = {
            let mut v: Vec<String> = c.items.iter().map(|i| hex::encode(&i.conversation_id)).collect();
            v.sort();
            v.dedup();
            v
        };
        // Merged by message id: what two devices both send is kept once.
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
        if step.progress {
            let from = match step.want.from.clone() {
                Some(f) => Some(f),
                None => self.my_devices().await.ok().and_then(|l| l.into_iter().find(|d| d.device_id == device.to_string())).map(|d| d.name),
            };
            let next = Want { from: from.filter(|f| !f.trim().is_empty()).or_else(|| Some("your other device".into())), ..step.want.clone() };
            self.set_setting_json(WANT, &next).await;
        }
        if written > 0 {
            (self.emit)(ChatEvent::HistoryMerged { conversations });
        }
        self.emit_history().await;
        let Ok(transfer_id) = hex::decode(&step.want.transfer_id) else { return };
        let ack = self.content(Body::HistoryAck(HistoryAck { transfer_id, next: step.ack_next, done: step.done })).await;
        let sent = if step.done {
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

    const T0: u64 = 1_800_000_000_000;

    fn waiting() -> Want {
        Want {
            transfer_id: "aa".into(),
            state: "waiting".into(),
            from: None,
            received: 0,
            total: 0,
            announced: true,
            asked_at_ms: Some(T0),
            from_device: None,
            last_at_ms: None,
        }
    }

    #[test]
    fn the_shell_gets_camel_case_and_never_the_stored_fields() {
        let v = HistoryView {
            incoming: Some(incoming_view(&Want { state: "receiving".into(), from: Some("PC".into()), received: 2, total: 5, last_at_ms: Some(T0), ..waiting() }, T0)),
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
        assert!(s.contains(r#""transferId":"aa""#) && s.contains(r#""createdAtMs":1"#) && s.contains(r#""deviceName":"Web browser""#), "{s}");
        assert!(s.contains(r#""askedAtMs":1800000000000"#) && s.contains(r#""quiet":false"#) && s.contains(r#""expired":false"#), "{s}");
        assert!(!s.contains("announced") && !s.contains("fromDevice"), "{s}");
        // A request kept by 0.3.1 still reads, with no time and not announced.
        let back: Want = serde_json::from_str(r#"{"transferId":"ab","state":"waiting","from":null,"received":0,"total":0}"#).unwrap();
        assert!(!back.announced && back.asked_at_ms.is_none());
    }

    #[test]
    fn a_waiting_device_asks_again_by_itself_until_someone_answers() {
        let w = waiting();
        assert!(!ask_again_due(Some(&w), T0 + 1000, T0, false));
        assert!(ask_again_due(Some(&w), T0 + ASK_AGAIN_MS, T0, false));
        assert!(ask_again_due(Some(&w), T0 + ASK_AGAIN_MIN_MS, T0, true));
        assert!(!ask_again_due(Some(&w), T0 + ASK_AGAIN_MIN_MS - 1, T0, true));
        assert!(!ask_again_due(None, T0 + ASK_AGAIN_MS, T0, true));
        assert!(!ask_again_due(Some(&Want { state: "done".into(), ..waiting() }), T0 + ASK_AGAIN_MS, T0, true));
        assert!(!ask_again_due(Some(&w), T0 + EXPIRE_MS, T0 + EXPIRE_MS - ASK_AGAIN_MS, true));
        let receiving = Want { state: "receiving".into(), from_device: Some(7), last_at_ms: Some(T0 + 5000), ..waiting() };
        assert!(!ask_again_due(Some(&receiving), T0 + 5000 + STALL_MS - 1, T0, false));
        assert!(ask_again_due(Some(&receiving), T0 + 5000 + STALL_MS, T0, false));
    }

    #[test]
    fn the_waiting_line_turns_useful_and_says_when_it_gave_up() {
        let w = waiting();
        assert!(!incoming_view(&w, T0 + 1000).quiet);
        assert!(incoming_view(&w, T0 + QUIET_MS).quiet && !incoming_view(&w, T0 + QUIET_MS).expired);
        assert!(incoming_view(&w, T0 + EXPIRE_MS).expired);
        let old = incoming_view(&Want { asked_at_ms: None, ..waiting() }, T0);
        assert_eq!((old.asked_at_ms, old.quiet, old.expired), (T0, false, false));
        let done = incoming_view(&Want { state: "done".into(), ..waiting() }, T0 + 2 * EXPIRE_MS);
        assert_eq!((done.quiet, done.expired), (false, false));
    }

    #[test]
    fn two_devices_saying_yes_make_one_transfer() {
        let w = waiting();
        let a = on_chunk(Some(&w), "aa", 1, Some(9), 0, 3, T0).unwrap();
        assert_eq!((a.progress, a.done, a.ack_next, a.want.from_device, a.want.received), (true, false, 1, Some(1), 1));
        let b = on_chunk(Some(&a.want), "aa", 2, Some(9), 0, 2, T0 + 1000).unwrap();
        assert_eq!((b.progress, b.done, b.ack_next), (false, false, 2));
        assert_eq!(b.want, a.want);
        assert!(!on_chunk(Some(&a.want), "aa", 2, Some(9), 1, 2, T0 + 1000).unwrap().done);
        let a2 = on_chunk(Some(&a.want), "aa", 1, Some(9), 2, 3, T0 + 2000).unwrap();
        assert!(a2.progress && a2.done && a2.want.state == "done");
        assert!(on_chunk(Some(&a2.want), "aa", 2, Some(9), 1, 2, T0 + 3000).is_none());
        assert!(on_chunk(Some(&w), "bb", 1, Some(9), 0, 1, T0).is_none());
        assert!(on_chunk(Some(&w), "aa", 9, Some(9), 0, 1, T0).is_none());
        assert!(on_chunk(Some(&w), "aa", 1, Some(9), 3, 3, T0).is_none());
        // The first one stalled: another takes over.
        let c = on_chunk(Some(&a.want), "aa", 2, Some(9), 0, 2, T0 + STALL_MS).unwrap();
        assert_eq!((c.progress, c.want.from_device, c.want.from.clone()), (true, Some(2), None));
    }

    fn offer(id: &str, seen: u64, later: bool) -> SavedOffer {
        SavedOffer { transfer_id: id.into(), device_id: "9".into(), device_name: "Kryoto Mobile".into(), created_at_ms: seen as i64, seen_at_ms: seen, later }
    }

    #[test]
    fn an_old_device_keeps_a_request_until_answered_done_or_expired() {
        let saved = vec![offer("x", T0, false), offer("y", T0 - EXPIRE_MS, false), offer("z", T0, true), offer("w", T0, false)];
        let (keep, gone) = kept_offers(saved, &["w".to_string()], T0 + 1000);
        assert_eq!(keep.iter().map(|s| (s.transfer_id.as_str(), s.later)).collect::<Vec<_>>(), vec![("x", false), ("z", true)]);
        assert_eq!(gone, vec!["y".to_string()]);
        let list = with_offer(with_offer(vec![offer("x", T0, false)], offer("x", T0, true)), offer("v", T0, false));
        assert_eq!(list.iter().map(|s| (s.transfer_id.as_str(), s.later)).collect::<Vec<_>>(), vec![("x", true), ("v", false)]);
        let mut many = Vec::new();
        for i in 0..30 {
            many = with_offer(many, offer(&i.to_string(), T0, false));
        }
        assert_eq!((many.len(), many[19].transfer_id.as_str()), (20, "29"));
    }

    #[test]
    fn a_request_that_comes_again_picks_up_where_it_stood() {
        assert_eq!(on_repeat_request("asking", false, 0, 0), Repeat::None);
        assert_eq!(on_repeat_request("done", true, 3, 3), Repeat::None);
        assert_eq!(on_repeat_request("sending", false, 0, 0), Repeat::Start);
        assert_eq!(on_repeat_request("waiting", true, 1, 3), Repeat::Resume);
        assert_eq!(on_repeat_request("sending", true, 3, 3), Repeat::Restart);
    }
}
