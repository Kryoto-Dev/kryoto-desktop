//! This PC's chat device and its live connection to the gateway.
//!
//! Everything cryptographic is km-core's; this file moves its outputs over the
//! WebSocket and keeps the encrypted store up to date. It never logs a token,
//! a key or a message.
//!
//! One connection runs three things side by side:
//! - a **reader** that hands each answer to whoever asked for it (by request
//!   id) and queues deliveries;
//! - a **processor** that takes deliveries one at a time, in order: decrypt,
//!   store, report, acknowledge;
//! - requests from anywhere (sending a message, fetching keys) through
//!   [`Client::request`], which works while deliveries keep arriving.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use futures_util::stream::SplitStream;
use futures_util::{SinkExt, StreamExt};
use km_core::{DeviceKind, LocalDevice, MasterKey, TrustStore, TrustedDevice};
use km_proto::gateway::{self as gw, client_frame::Kind as C, server_frame::Kind as S, ClientFrame, ServerFrame};
use km_store_sqlcipher::Store;
use prost::Message as _;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use zeroize::Zeroizing;

use super::types::{ChatEvent, ChatStatus, Context, Emit};
use crate::logging;

/// Keep at least this many one-time keys on the server; top up to TARGET.
const LOW_WATER: u32 = 30;
const TARGET: u32 = 100;
/// The gateway pings every 25 s; silence this long means the line is dead.
const SILENCE: Duration = Duration::from_secs(90);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// This account's chat state, behind one lock.
pub struct Engine {
    pub user_id: u64,
    pub(super) store: Store,
    pub(super) device: LocalDevice,
    pub(super) master: MasterKey,
    pub(super) trust: TrustStore,
    /// Each user's checked devices, fetched when first needed.
    pub(super) devices: HashMap<u64, Vec<TrustedDevice>>,
    /// Groups this account is in: members from the gateway.
    pub(super) groups: HashMap<u64, super::messaging::GroupCache>,
}

impl Engine {
    /// Open (or create) this account's chat state in its encrypted database.
    pub fn open(dir: &Path, user_id: u64, key: &[u8; 32]) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not create the chat folder: {e}"))?;
        let store = Store::open(&dir.join(format!("{user_id}.db")), key).map_err(|e| e.to_string())?;
        let device = match store.load_device().map_err(|e| e.to_string())? {
            Some(d) if d.user_id() == user_id => d,
            Some(_) => return Err("The chat database belongs to another account.".into()),
            None => {
                let d = LocalDevice::new(user_id, DeviceKind::Desktop);
                store.save_device(&d).map_err(|e| e.to_string())?;
                d
            }
        };
        let master = match store.load_master_key().map_err(|e| e.to_string())? {
            Some(m) => m,
            None => {
                let m = MasterKey::generate();
                store.save_master_key(&m).map_err(|e| e.to_string())?;
                m
            }
        };
        let mut trust = store.load_trust().map_err(|e| e.to_string())?;
        trust.pin_own(user_id, master.public_key());
        Ok(Self { user_id, store, device, master, trust, devices: HashMap::new(), groups: HashMap::new() })
    }

    /// Persist the device (sessions) and trust pins.
    pub(super) fn save(&self) -> Result<(), String> {
        self.store.save_device(&self.device).map_err(|e| e.to_string())?;
        self.store.save_trust(&self.trust).map_err(|e| e.to_string())
    }
}

/// A refusal from the gateway, by its stable code.
#[derive(Debug, Clone)]
pub struct Refused {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, w: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        w.write_str(&self.message)
    }
}

/// A live connection's request side.
pub struct Client {
    out: mpsc::Sender<Message>,
    pending: StdMutex<HashMap<u32, oneshot::Sender<S>>>,
    next: AtomicU32,
}

impl Client {
    fn new(out: mpsc::Sender<Message>) -> Self {
        Self { out, pending: StdMutex::new(HashMap::new()), next: AtomicU32::new(0) }
    }

    /// Send a request and wait for its answer.
    pub async fn request(&self, kind: C) -> Result<S, Refused> {
        let id = loop {
            let id = self.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            if id != 0 {
                break id;
            }
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending").insert(id, tx);
        let frame = ClientFrame { request_id: id, kind: Some(kind) };
        if self.out.send(Message::Binary(frame.encode_to_vec().into())).await.is_err() {
            self.pending.lock().expect("pending").remove(&id);
            return Err(Refused { code: "offline".into(), message: "Not connected.".into() });
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(S::Error(e))) => Err(Refused { code: e.code, message: e.message }),
            Ok(Ok(kind)) => Ok(kind),
            _ => {
                self.pending.lock().expect("pending").remove(&id);
                Err(Refused { code: "timeout".into(), message: "The chat server did not answer.".into() })
            }
        }
    }

    /// A frame that needs no answer (acknowledgements).
    pub async fn tell(&self, kind: C) {
        let frame = ClientFrame { request_id: 0, kind: Some(kind) };
        let _ = self.out.send(Message::Binary(frame.encode_to_vec().into())).await;
    }

    fn answer(&self, id: u32, kind: S) {
        if let Some(tx) = self.pending.lock().expect("pending").remove(&id) {
            let _ = tx.send(kind);
        }
    }
}

/// Everything the app holds about chat while it is turned on.
pub struct Chat {
    pub engine: Arc<Mutex<Engine>>,
    pub client: StdMutex<Option<Arc<Client>>>,
    pub emit: Emit,
    pub context: StdMutex<Context>,
    /// History transfers to and from this account's other devices (KB-0254).
    pub history: super::history::HistoryRuntime,
}

impl Chat {
    pub fn new(engine: Engine, emit: Emit) -> Arc<Self> {
        Arc::new(Self {
            engine: Arc::new(Mutex::new(engine)),
            client: StdMutex::new(None),
            emit,
            context: StdMutex::new(Context::default()),
            history: Default::default(),
        })
    }

    pub fn client(&self) -> Option<Arc<Client>> {
        self.client.lock().expect("client").clone()
    }

    pub(super) fn status(&self, status: ChatStatus) {
        (self.emit)(ChatEvent::Status(status));
    }
}

/// Why a connection ended.
#[derive(Debug)]
enum Outcome {
    /// Try again after a backoff.
    Retry(String),
    Stop(ChatStatus),
}

fn refusal(code: &str, message: &str) -> Outcome {
    match code {
        "unauthorized" => Outcome::Stop(ChatStatus::SignInNeeded),
        "device_unknown" => Outcome::Stop(ChatStatus::DeviceRemoved),
        "anonymous_mode" => Outcome::Stop(ChatStatus::AnonymousMode),
        "chat_disabled" => Outcome::Stop(ChatStatus::Disabled),
        "unsupported_version" | "conflict" => Outcome::Stop(ChatStatus::Unavailable { reason: message.into() }),
        _ => Outcome::Retry(format!("gateway refused: {code}")),
    }
}

impl From<Refused> for Outcome {
    fn from(r: Refused) -> Self {
        refusal(&r.code, &r.message)
    }
}

async fn send_raw(ws: &mut Ws, kind: C) -> Result<(), Outcome> {
    let f = ClientFrame { request_id: 0, kind: Some(kind) };
    ws.send(Message::Binary(f.encode_to_vec().into())).await.map_err(|e| Outcome::Retry(format!("send failed: {e}")))
}

async fn read_raw(ws: &mut Ws) -> Result<S, Outcome> {
    loop {
        let next = tokio::time::timeout(SILENCE, ws.next()).await.map_err(|_| Outcome::Retry("gateway went silent".into()))?;
        match next {
            Some(Ok(Message::Binary(b))) => {
                let f = ServerFrame::decode(b.as_ref()).map_err(|_| Outcome::Retry("unreadable frame".into()))?;
                if let Some(kind) = f.kind {
                    return Ok(kind);
                }
            }
            Some(Ok(Message::Close(_))) | None => return Err(Outcome::Retry("connection closed".into())),
            Some(Err(e)) => return Err(Outcome::Retry(format!("connection error: {e}"))),
            _ => {}
        }
    }
}

pub(super) fn host_name() -> String {
    sysinfo::System::host_name().map(|h| h.chars().take(48).collect()).unwrap_or_else(|| "This PC".into())
}

/// Connect, prove (or register) this device, and return the Ready answer.
async fn handshake(engine: &Mutex<Engine>, token: &str, url: &str) -> Result<(Ws, gw::Ready), Outcome> {
    let (mut ws, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(url))
        .await
        .map_err(|_| Outcome::Retry("connect timed out".into()))?
        .map_err(|e| Outcome::Retry(format!("connect failed: {e}")))?;
    let device_id = engine.lock().await.device.device_id();
    send_raw(
        &mut ws,
        C::Hello(gw::Hello { token: token.into(), protocol_version: gw::PROTOCOL_VERSION, device_id: device_id.unwrap_or(0) }),
    )
    .await?;
    let nonce = match read_raw(&mut ws).await? {
        S::Challenge(c) => c.nonce,
        S::Error(e) => return Err(refusal(&e.code, &e.message)),
        _ => return Err(Outcome::Retry("unexpected answer to hello".into())),
    };
    match device_id {
        Some(id) => {
            let signature = engine.lock().await.device.sign_gateway_challenge(&nonce, id).to_vec();
            send_raw(&mut ws, C::Proof(gw::Proof { signature })).await?;
        }
        None => {
            let frame = {
                let eng = engine.lock().await;
                let k = eng.device.keys();
                C::RegisterDevice(gw::RegisterDevice {
                    kind: 1,
                    name: host_name(),
                    ed25519: k.ed25519.to_vec(),
                    curve25519: k.curve25519.to_vec(),
                    seal_key: k.seal_key.to_vec(),
                    signature: eng.device.sign_gateway_challenge(&nonce, 0).to_vec(),
                })
            };
            send_raw(&mut ws, frame).await?;
            match read_raw(&mut ws).await? {
                S::Registered(r) => {
                    let mut eng = engine.lock().await;
                    eng.device.set_device_id(r.device_id).map_err(|e| Outcome::Stop(ChatStatus::Unavailable { reason: e.to_string() }))?;
                    eng.save().map_err(|e| Outcome::Stop(ChatStatus::Unavailable { reason: e }))?;
                    super::history::mark_new_device(&eng.store);
                    logging::info("chat", "registered this PC as a chat device");
                }
                S::Error(e) => return Err(refusal(&e.code, &e.message)),
                _ => return Err(Outcome::Retry("unexpected answer to registration".into())),
            }
        }
    }
    match read_raw(&mut ws).await? {
        S::Ready(r) => Ok((ws, r)),
        S::Error(e) => Err(refusal(&e.code, &e.message)),
        _ => Err(Outcome::Retry("expected ready".into())),
    }
}

/// Make the server's view of this device right: identity, certificate, keys.
async fn settle(engine: &Mutex<Engine>, client: &Client, ready: &gw::Ready) -> Result<(), Outcome> {
    let (ours, device_id) = {
        let eng = engine.lock().await;
        (eng.master.public_key(), eng.device.device_id().unwrap_or(0))
    };
    if ready.master_key.is_empty() {
        client.request(C::PublishMasterKey(gw::PublishMasterKey { public_key: ours.to_vec() })).await?;
        logging::info("chat", "published this account's master key");
    } else if ready.master_key.as_slice() != ours {
        // Another device holds this account's identity: this one has to be
        // linked from it (QR, milestone 8) before anyone can reach it.
        return Err(Outcome::Stop(ChatStatus::NeedsLink));
    }
    if !ready.certified || ready.master_key.is_empty() {
        let signature = {
            let eng = engine.lock().await;
            eng.master.certify(&eng.device.keys()).msk_signature.to_vec()
        };
        client.request(C::CertifyDevice(gw::CertifyDevice { device_id, signature })).await?;
    }
    if ready.one_time_keys < LOW_WATER || !ready.has_fallback_key {
        let prekeys = {
            let mut eng = engine.lock().await;
            let n = TARGET.saturating_sub(ready.one_time_keys) as usize;
            let keys = eng.device.publish_prekeys(n, !ready.has_fallback_key).map_err(|e| Outcome::Retry(e.to_string()))?;
            // Saved BEFORE upload: the server must never hold a key whose
            // secret we could lose in a crash.
            eng.save().map_err(|e| Outcome::Stop(ChatStatus::Unavailable { reason: e }))?;
            keys
        };
        let prekeys = prekeys
            .into_iter()
            .map(|p| gw::Prekey { key_id: p.key_id, key: p.key.to_vec(), fallback: p.fallback, signature: p.signature.to_vec() })
            .collect();
        client.request(C::KeysUpload(gw::KeysUpload { prekeys })).await?;
    }
    Ok(())
}

/// Read frames until the connection ends: answers to their askers,
/// deliveries to the processor.
async fn read_loop(
    mut stream: SplitStream<Ws>,
    client: Arc<Client>,
    deliveries: mpsc::UnboundedSender<gw::Deliver>,
    groups: mpsc::UnboundedSender<u64>,
    me: u64,
) -> Outcome {
    loop {
        let next = match tokio::time::timeout(SILENCE, stream.next()).await {
            Ok(n) => n,
            Err(_) => return Outcome::Retry("gateway went silent".into()),
        };
        let bytes = match next {
            Some(Ok(Message::Binary(b))) => b,
            Some(Ok(Message::Close(_))) | None => return Outcome::Retry("connection closed".into()),
            Some(Err(e)) => return Outcome::Retry(format!("connection error: {e}")),
            Some(Ok(_)) => continue,
        };
        let Ok(frame) = ServerFrame::decode(bytes.as_ref()) else { continue };
        match (frame.request_id, frame.kind) {
            (id, Some(kind)) if id != 0 => client.answer(id, kind),
            (_, Some(S::Deliver(d))) => {
                let _ = deliveries.send(d);
            }
            (_, Some(S::Event(e))) if e.kind == gw::EventKind::MasterKeyChanged as i32 && e.user_id == me => {
                // Someone published a different identity for this account:
                // reconnect, which re-checks it.
                return Outcome::Retry("own master key changed".into());
            }
            (_, Some(S::Event(e))) if e.kind == gw::EventKind::GroupChanged as i32 && e.group_id != 0 => {
                let _ = groups.send(e.group_id);
            }
            (_, Some(S::Error(e))) => return refusal(&e.code, &e.message),
            _ => {}
        }
    }
}

async fn session(chat: &Arc<Chat>, token: &str, url: &str) -> Outcome {
    let (ws, ready) = match handshake(&chat.engine, token, url).await {
        Ok(v) => v,
        Err(o) => return o,
    };
    let (mut sink, stream) = ws.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(256);
    let _writer = Owned(tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            if sink.send(m).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    }));
    let client = Arc::new(Client::new(out_tx));
    let (deliver_tx, mut deliver_rx) = mpsc::unbounded_channel();
    let (group_tx, mut group_rx) = mpsc::unbounded_channel::<u64>();
    let (outcome_tx, outcome_rx) = oneshot::channel();
    let reader_client = client.clone();
    let _reader = Owned(tokio::spawn(async move {
        let _ = outcome_tx.send(read_loop(stream, reader_client, deliver_tx, group_tx, ready.user_id).await);
    }));

    if let Err(o) = settle(&chat.engine, &client, &ready).await {
        return o;
    }
    *chat.client.lock().expect("client") = Some(client.clone());
    chat.status(ChatStatus::Online { user_id: ready.user_id.to_string(), device_id: ready.device_id.to_string() });
    logging::info("chat", "connected");

    // Anything that was waiting to go out (sent while offline) goes now.
    let _resend = {
        let chat = chat.clone();
        Owned(tokio::spawn(async move { chat.resend_pending().await }))
    };

    // Past messages to or from this account's other devices (KB-0254).
    let _history = Owned(tokio::spawn(chat.clone().history_loop(host_name())));

    // Group changes (someone added or removed): refresh, tell the shell.
    let _groups = {
        let chat = chat.clone();
        Owned(tokio::spawn(async move {
            while let Some(g) = group_rx.recv().await {
                chat.on_group_changed(g).await;
            }
        }))
    };

    // Deliveries, strictly one after another, acknowledged once handled.
    let _processor = {
        let chat = chat.clone();
        let client = client.clone();
        Owned(tokio::spawn(async move {
            while let Some(d) = deliver_rx.recv().await {
                chat.handle_delivery(&d).await;
                if d.seq != 0 {
                    client.tell(C::Ack(gw::Ack { up_to_seq: d.seq })).await;
                }
            }
        }))
    };

    // Whatever ends the connection ends everything above with it (Owned).
    let outcome = outcome_rx.await.unwrap_or_else(|_| Outcome::Retry("reader stopped".into()));
    *chat.client.lock().expect("client") = None;
    outcome
}

/// A spawned task that ends with whatever owns this: dropping the session
/// (chat turned off, app closing, reconnect) stops its reader, writer and
/// processor instead of leaving them running on an old connection.
struct Owned(tokio::task::JoinHandle<()>);

impl Drop for Owned {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn jitter(secs: u64) -> Duration {
    let mut b = [0u8; 2];
    let _ = getrandom::fill(&mut b);
    let spread = u64::from(u16::from_be_bytes(b)) % (secs * 250 + 1);
    Duration::from_millis(secs * 1000 - (secs * 125) + spread)
}

/// Stay connected until told to stop: reconnect with backoff (1 s to 60 s).
pub async fn run(chat: Arc<Chat>, token: Zeroizing<String>, url: String) {
    let mut backoff = 1u64;
    loop {
        chat.status(ChatStatus::Connecting);
        let started = Instant::now();
        match session(&chat, &token, &url).await {
            Outcome::Stop(status) => {
                logging::warn("chat", &format!("stopped: {status:?}"));
                chat.status(status);
                return;
            }
            Outcome::Retry(reason) => {
                if started.elapsed() > Duration::from_secs(60) {
                    backoff = 1;
                }
                logging::warn("chat", &format!("disconnected ({reason}); retrying in about {backoff}s"));
                chat.status(ChatStatus::Offline { retry_in_secs: backoff });
                tokio::time::sleep(jitter(backoff)).await;
                backoff = (backoff * 2).min(60);
            }
        }
    }
}

/// A short connection for one-off work while the live one is stopped
/// (restoring from a backup, resetting the identity): handshake, then
/// requests one after another. Deliveries that arrive meanwhile are left
/// unacknowledged and come again on the live connection.
pub struct OneOff {
    ws: Ws,
    next: u32,
}

fn outcome_text(o: Outcome) -> String {
    match o {
        Outcome::Retry(reason) => format!("Could not reach the chat server ({reason})."),
        Outcome::Stop(ChatStatus::Unavailable { reason }) => reason,
        Outcome::Stop(s) => format!("The chat server refused: {s:?}"),
    }
}

impl OneOff {
    pub async fn open(engine: &Mutex<Engine>, token: &str, url: &str) -> Result<Self, String> {
        let (ws, _ready) = handshake(engine, token, url).await.map_err(outcome_text)?;
        Ok(Self { ws, next: 0 })
    }

    /// One request and its answer; a refusal comes back as its message.
    pub async fn ask(&mut self, kind: C) -> Result<S, String> {
        self.next += 1;
        let id = self.next;
        let frame = ClientFrame { request_id: id, kind: Some(kind) };
        self.ws.send(Message::Binary(frame.encode_to_vec().into())).await.map_err(|e| format!("Could not reach the chat server ({e})."))?;
        loop {
            let next = tokio::time::timeout(REQUEST_TIMEOUT, self.ws.next())
                .await
                .map_err(|_| "The chat server did not answer.".to_string())?;
            let bytes = match next {
                Some(Ok(Message::Binary(b))) => b,
                Some(Ok(Message::Close(_))) | None => return Err("The chat server closed the connection.".into()),
                Some(Err(e)) => return Err(format!("Connection error: {e}")),
                Some(Ok(_)) => continue,
            };
            let Ok(f) = ServerFrame::decode(bytes.as_ref()) else { continue };
            if f.request_id != id {
                continue;
            }
            return match f.kind {
                Some(S::Error(e)) => Err(e.message),
                Some(kind) => Ok(kind),
                None => Err("The chat server sent an empty answer.".into()),
            };
        }
    }

    pub async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}

/// Remove this device from the account on the server (best effort).
pub async fn revoke_self(engine: &Mutex<Engine>, token: &str, url: &str) -> Result<(), String> {
    let device_id = engine.lock().await.device.device_id().ok_or("This PC was never registered.")?;
    let (mut ws, _) = handshake(engine, token, url).await.map_err(|o| format!("{o:?}"))?;
    send_raw(&mut ws, C::RevokeDevice(gw::RevokeDevice { device_id })).await.map_err(|o| format!("{o:?}"))?;
    // The gateway answers, then closes.
    let _ = tokio::time::timeout(Duration::from_secs(5), read_raw(&mut ws)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_jitter_stays_near_the_target() {
        for secs in [1u64, 2, 8, 60] {
            for _ in 0..50 {
                let d = jitter(secs).as_millis() as u64;
                assert!(d >= secs * 875 && d <= secs * 1125, "{secs}s -> {d}ms");
            }
        }
    }

    #[test]
    fn refusals_map_to_what_the_person_must_do() {
        assert!(matches!(refusal("unauthorized", "m"), Outcome::Stop(ChatStatus::SignInNeeded)));
        assert!(matches!(refusal("device_unknown", "m"), Outcome::Stop(ChatStatus::DeviceRemoved)));
        assert!(matches!(refusal("anonymous_mode", "m"), Outcome::Stop(ChatStatus::AnonymousMode)));
        assert!(matches!(refusal("chat_disabled", "m"), Outcome::Stop(ChatStatus::Disabled)));
        assert!(matches!(refusal("internal", "m"), Outcome::Retry(_)));
    }

    #[test]
    fn state_survives_reopening_and_stays_one_identity() {
        let dir = std::env::temp_dir().join(format!("kryoto-chat-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = [9u8; 32];
        let (master, ed) = {
            let e = Engine::open(&dir, 77, &key).unwrap();
            (e.master.public_key(), e.device.keys().ed25519)
        };
        let e = Engine::open(&dir, 77, &key).unwrap();
        assert_eq!(e.master.public_key(), master, "the master key is reused, not regenerated");
        assert_eq!(e.device.keys().ed25519, ed, "the device keys are reused");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
