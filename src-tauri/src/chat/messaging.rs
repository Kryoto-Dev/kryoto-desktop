//! Sending and receiving messages: what happens between a key press and the
//! other person's screen.
//!
//! A message to someone is encrypted once per device of theirs and once per
//! other device of yours (so your own devices show what you sent), and goes
//! out as one Send. Receipts, edits, deletes and reactions are messages too,
//! end-to-end encrypted like the text they refer to. Typing indicators are
//! the only thing sent "ephemeral": delivered to devices online right now,
//! never stored anywhere.
//!
//! Group chats (up to 32 people) work the same way: a message is encrypted
//! for every device of every member, and the Send names the group so the
//! gateway can check membership instead of friendship. The member list comes
//! from the gateway; the group's name travels encrypted (`GroupMeta`).

use km_core::chat::{
    dm_conversation_id, dm_peer, group_conversation_id, group_of, receive_rules, text_limit, validate_outgoing, Shown, EDIT_WINDOW_MS,
};
use km_core::{new_message_id, CoreError, DeviceKeys, DeviceKind, SignedDevice, SignedPrekey, TrustedDevice, UserDevices};
use km_proto::gateway::{self as gw, client_frame::Kind as C, server_frame::Kind as S, SendStatus};
use km_proto::{content::Body, Attachment, Call, CallKind, Content, Delete, Edit, Gif, GroupMeta, Invite, Reaction, Receipt, ReceiptKind, Text, Typing};
use km_store_sqlcipher::{MessageRow, NewMessage, Status};
use serde::{Deserialize, Serialize};

use super::engine::{Chat, Client};
use super::types::{ChatEvent, Settings};
use crate::logging;

/// A GIF as stored and shown: where to load it from, never the file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GifBody {
    pub provider: String,
    pub id: String,
    pub url: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub title: String,
}

/// A game invite as stored and shown.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InviteBody {
    pub slug: String,
    pub title: String,
    /// Steam lobby to join (Kryoto Online games), or empty.
    #[serde(default)]
    pub steam_lobby: String,
    #[serde(default)]
    pub host_steam_id: String,
    pub expires_at: u64,
}

impl InviteBody {
    fn to_proto(&self) -> Invite {
        Invite {
            slug: self.slug.clone(),
            title: self.title.clone(),
            steam_lobby: self.steam_lobby.clone(),
            host_steam_id: self.host_steam_id.clone(),
            expires_at_ms: self.expires_at,
        }
    }

    fn from_proto(i: &Invite) -> Self {
        Self {
            slug: i.slug.clone(),
            title: i.title.clone(),
            steam_lobby: i.steam_lobby.clone(),
            host_steam_id: i.host_steam_id.clone(),
            expires_at: i.expires_at_ms,
        }
    }
}

/// Who a message is for: one person, or a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Dm(u64),
    Group(u64),
}

impl Target {
    /// "123" for a person, "g:123" for a group (how the shell names them).
    pub fn parse(s: &str) -> Option<Self> {
        match s.strip_prefix("g:") {
            Some(g) => g.parse().ok().map(Target::Group),
            None => s.parse().ok().map(Target::Dm),
        }
    }

    pub fn conversation(&self, me: u64) -> Vec<u8> {
        match *self {
            Target::Dm(peer) => dm_conversation_id(me, peer),
            Target::Group(g) => group_conversation_id(g),
        }
    }

    pub fn from_conversation(conversation_id: &[u8], me: u64) -> Option<Self> {
        group_of(conversation_id).map(Target::Group).or_else(|| dm_peer(conversation_id, me).map(Target::Dm))
    }

    fn group(&self) -> Option<u64> {
        match *self {
            Target::Group(g) => Some(g),
            Target::Dm(_) => None,
        }
    }

    fn peer(&self) -> Option<u64> {
        match *self {
            Target::Dm(p) => Some(p),
            Target::Group(_) => None,
        }
    }
}

/// A group as this app knows it: the members from the gateway, the name from
/// the members' own (encrypted) messages.
#[derive(Clone, Debug, Default)]
pub struct GroupCache {
    pub members: Vec<(u64, String)>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMemberView {
    pub user_id: String,
    pub role: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub id: String,
    pub name: String,
    pub members: Vec<GroupMemberView>,
    pub my_role: String,
}

fn group_name_key(group: u64) -> String {
    format!("group-name-{group}")
}

/// A file as stored: where its ciphertext is and what opens it. The key is
/// only ever in the encrypted local database and in the encrypted message.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileBody {
    pub id: String,
    pub token: String,
    pub key: String,
    pub sha256: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

impl FileBody {
    fn to_proto(&self) -> Option<Attachment> {
        Some(Attachment {
            id: self.id.parse().ok()?,
            download_token: self.token.clone(),
            key: hex::decode(&self.key).ok()?,
            sha256: hex::decode(&self.sha256).ok()?,
            name: self.name.clone(),
            mime: self.mime.clone(),
            size: self.size,
            width: self.width,
            height: self.height,
        })
    }

    fn from_proto(a: &Attachment) -> Self {
        Self {
            id: a.id.to_string(),
            token: a.download_token.clone(),
            key: hex::encode(&a.key),
            sha256: hex::encode(&a.sha256),
            name: a.name.clone(),
            mime: a.mime.clone(),
            size: a.size,
            width: a.width,
            height: a.height,
        }
    }

    /// The same file, with what the shell may see (no key, no token).
    pub fn public_json(&self) -> String {
        serde_json::json!({ "name": self.name, "mime": self.mime, "size": self.size, "width": self.width, "height": self.height }).to_string()
    }
}

/// A guess at the type from the file name, for previews and the save dialog.
pub fn mime_of(name: &str) -> &'static str {
    match name.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("txt" | "log") => "text/plain",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    }
}

/// How long an invite stays joinable: Steam lobbies do not last.
pub const INVITE_TTL_MS: u64 = 15 * 60 * 1000;

/// Why something could not be sent, in words for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    Offline,
    NoChat,
    IdentityChanged,
    Forbidden,
    RateLimited,
    Invalid(String),
    Failed(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SendError::Offline => "You are offline. It will be sent when you are back.",
            SendError::NoChat => "They have not turned on chat yet.",
            SendError::IdentityChanged => "Their security key changed. Check it before sending.",
            SendError::Forbidden => "You cannot message this person.",
            SendError::RateLimited => "Slow down a little.",
            SendError::Invalid(m) | SendError::Failed(m) => m,
        })
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn to_user_devices(u: &gw::UserDevicesInfo) -> Option<UserDevices> {
    Some(UserDevices {
        user_id: u.user_id,
        master_key: u.master_key.as_slice().try_into().ok()?,
        devices: u
            .devices
            .iter()
            .filter_map(|d| {
                Some(SignedDevice {
                    keys: DeviceKeys {
                        user_id: u.user_id,
                        device_id: d.device_id,
                        kind: if d.kind == 1 { DeviceKind::Desktop } else { DeviceKind::Web },
                        ed25519: d.ed25519.as_slice().try_into().ok()?,
                        curve25519: d.curve25519.as_slice().try_into().ok()?,
                        seal_key: d.seal_key.as_slice().try_into().ok()?,
                    },
                    msk_signature: d.msk_signature.as_slice().try_into().ok()?,
                })
            })
            .collect(),
    })
}

fn prekey(p: &gw::Prekey) -> Option<SignedPrekey> {
    Some(SignedPrekey {
        key_id: p.key_id.clone(),
        key: p.key.as_slice().try_into().ok()?,
        fallback: p.fallback,
        signature: p.signature.as_slice().try_into().ok()?,
    })
}

fn invalid(e: CoreError, supporter: bool) -> SendError {
    match e {
        CoreError::TooLarge => SendError::Invalid(format!(
            "Messages can be {} characters long{}.",
            text_limit(supporter),
            if supporter { "" } else { " (8,000 for supporters)" }
        )),
        _ => SendError::Invalid("That cannot be sent.".into()),
    }
}

impl Chat {
    async fn my_id(&self) -> u64 {
        self.engine.lock().await.user_id
    }

    fn supporter(&self, user: u64) -> bool {
        self.context.lock().expect("context").people.get(&user).is_some_and(|p| p.supporter)
    }

    /// Their message request is waiting for this account's answer.
    fn awaiting_answer(&self, user: u64) -> bool {
        self.context.lock().expect("context").people.get(&user).is_some_and(|p| p.pending)
    }

    fn me_supporter(&self) -> bool {
        self.context.lock().expect("context").me_supporter
    }

    pub async fn settings(&self) -> Settings {
        let eng = self.engine.lock().await;
        eng.store
            .setting("chat")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub async fn set_settings(&self, s: &Settings) -> Result<(), String> {
        let json = serde_json::to_string(s).map_err(|e| e.to_string())?;
        self.engine.lock().await.store.set_setting("chat", &json).map_err(|e| e.to_string())
    }

    /// What the reader sees for a stored message: the supporter-only rules
    /// applied against the sender's current supporter status.
    pub fn shown(&self, mut row: MessageRow) -> MessageRow {
        if row.kind == "file" {
            if let Ok(f) = serde_json::from_str::<FileBody>(&row.body) {
                row.body = f.public_json();
            }
            return row;
        }
        // Invites are checked field by field when they arrive (invite_valid).
        if row.outgoing || row.deleted || row.kind == "invite" {
            return row;
        }
        let sender: u64 = row.sender_user.parse().unwrap_or(0);
        let supporter = self.supporter(sender);
        let body = match row.kind.as_str() {
            "gif" => serde_json::from_str::<GifBody>(&row.body).ok().map(|g| {
                Body::Gif(Gif { provider: g.provider, id: g.id, url: g.url, width: g.width, height: g.height, title: g.title })
            }),
            _ => Some(Body::Text(Text { text: row.body.clone(), reply_to: vec![] })),
        };
        let content = Content { msg_id: vec![], conversation_id: vec![], sent_at_ms: 0, body };
        match receive_rules(&content, supporter) {
            Shown::Ok => {}
            Shown::Truncated(t) => row.body = t,
            Shown::GifAsText(label) => {
                row.kind = "text".into();
                row.body = label;
            }
        }
        row
    }

    /// Every conversation, newest first, with its last message as shown.
    pub async fn conversations(&self) -> Result<Vec<km_store_sqlcipher::ConversationRow>, String> {
        let rows = self.engine.lock().await.store.conversations().map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|mut c| {
                c.last = c.last.map(|m| self.shown(m));
                c
            })
            .collect())
    }

    /// Messages in a conversation, oldest first; `before` (ms) pages back.
    pub async fn messages_with(&self, target: Target, before: Option<u64>) -> Result<Vec<MessageRow>, String> {
        let eng = self.engine.lock().await;
        let conv = target.conversation(eng.user_id);
        let rows = eng.store.messages(&conv, before, 60).map_err(|e| e.to_string())?;
        drop(eng);
        Ok(rows.into_iter().map(|m| self.shown(m)).collect())
    }

    /// Search this PC's messages. Nothing is searched anywhere else.
    pub async fn search(&self, query: &str) -> Result<Vec<MessageRow>, String> {
        let q = query.trim();
        if q.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let rows = self.engine.lock().await.store.search(q, 50).map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|m| self.shown(m)).collect())
    }

    /// A user's checked devices, from the cache unless `refresh`.
    pub(super) async fn devices_of(&self, client: &Client, user: u64, refresh: bool) -> Result<Vec<TrustedDevice>, SendError> {
        if !refresh {
            // "No devices" is never trusted from the cache: they may have
            // turned chat on, or a message request may have opened, since.
            if let Some(d) = self.engine.lock().await.devices.get(&user).filter(|d| !d.is_empty()) {
                return Ok(d.clone());
            }
        }
        let answer = client
            .request(C::DevicesQuery(gw::DevicesQuery { user_ids: vec![user] }))
            .await
            .map_err(|e| SendError::Failed(e.message))?;
        let S::Devices(found) = answer else { return Err(SendError::Failed("Unexpected answer.".into())) };
        let mut eng = self.engine.lock().await;
        let own_device = eng.device.device_id().unwrap_or(0);
        let devices = match found.users.first().and_then(to_user_devices) {
            None => Vec::new(),
            Some(u) => match eng.trust.check(&u) {
                Ok(checked) => checked.devices.into_iter().filter(|d| d.keys().device_id != own_device).collect(),
                Err(CoreError::IdentityChanged { .. }) => {
                    drop(eng);
                    (self.emit)(ChatEvent::IdentityChanged { user_id: user.to_string() });
                    return Err(SendError::IdentityChanged);
                }
                Err(_) => Vec::new(),
            },
        };
        eng.devices.insert(user, devices.clone());
        let _ = eng.save();
        Ok(devices)
    }

    /// Encrypt `content` for every device of `users` and send it. In a group
    /// (`group`), members who cannot be reached right now (no chat devices,
    /// a block, a security key waiting to be accepted) are left out instead
    /// of stopping the message for everyone.
    async fn deliver(&self, users: &[u64], content: &Content, ephemeral: bool, group: Option<u64>) -> Result<(), SendError> {
        let client = self.client().ok_or(SendError::Offline)?;
        let me = self.my_id().await;
        for attempt in 0..2 {
            let refresh = attempt > 0;
            let mut items = Vec::new();
            for &user in users {
                let devices = match self.devices_of(&client, user, refresh).await {
                    Ok(d) => d,
                    Err(SendError::IdentityChanged) if group.is_some() => continue,
                    Err(e) => return Err(e),
                };
                if user != me && devices.is_empty() {
                    if group.is_some() {
                        continue;
                    }
                    return Err(SendError::NoChat);
                }
                let missing: Vec<TrustedDevice> = {
                    let eng = self.engine.lock().await;
                    devices.iter().filter(|d| !eng.device.has_session(d)).cloned().collect()
                };
                let mut claimed: Vec<gw::ClaimedKey> = Vec::new();
                if !missing.is_empty() {
                    if let Ok(S::KeysBundle(b)) = client.request(C::KeysClaim(gw::KeysClaim { user_id: user })).await {
                        claimed = b.keys;
                    }
                }
                let mut eng = self.engine.lock().await;
                for d in &devices {
                    let pk = claimed
                        .iter()
                        .find(|k| k.device_id == d.keys().device_id)
                        .and_then(|k| k.prekey.as_ref())
                        .and_then(prekey);
                    match eng.device.encrypt(d, pk.as_ref(), content) {
                        Ok(envelope) => items.push(gw::SendItem { recipient_device_id: d.keys().device_id, envelope }),
                        // A device with no session and no prekey cannot be reached.
                        Err(e) => logging::warn("chat", &format!("skipped a device: {e}")),
                    }
                }
                let _ = eng.save();
            }
            if items.is_empty() {
                // Only ourselves, with no other devices (or a group nobody
                // else in can be reached right now): nothing to send.
                return if group.is_some() || users.iter().all(|u| *u == me) { Ok(()) } else { Err(SendError::NoChat) };
            }
            let answer = client
                .request(C::Send(gw::Send { client_msg_id: content.msg_id.clone(), items, ephemeral, group_id: group.unwrap_or(0) }))
                .await
                .map_err(|e| if e.code == "offline" { SendError::Offline } else { SendError::Failed(e.message) })?;
            let S::SendAck(ack) = answer else { return Err(SendError::Failed("Unexpected answer.".into())) };
            match SendStatus::try_from(ack.status).unwrap_or(SendStatus::Unspecified) {
                SendStatus::Accepted => return Ok(()),
                SendStatus::DeviceMismatch => continue,
                SendStatus::Forbidden => return Err(SendError::Forbidden),
                SendStatus::RateLimited => return Err(SendError::RateLimited),
                SendStatus::Unspecified => return Err(SendError::Failed("Unexpected answer.".into())),
            }
        }
        Err(SendError::Failed("Their devices kept changing. Try again.".into()))
    }

    fn emit_row(&self, new: bool, row: MessageRow) {
        let row = self.shown(row);
        (self.emit)(if new { ChatEvent::Message(row) } else { ChatEvent::Updated(row) });
    }

    /// Everyone a message to `target` goes to (us included, for our other
    /// devices), and the group it is in.
    async fn recipients(&self, target: Target) -> Result<(Vec<u64>, Option<u64>), SendError> {
        let me = self.my_id().await;
        match target {
            Target::Dm(peer) => Ok((vec![peer, me], None)),
            Target::Group(g) => {
                let group = self.group_cache(g, false).await.ok_or(SendError::Invalid("You are not in that group.".into()))?;
                if !group.members.iter().any(|(u, _)| *u == me) {
                    return Err(SendError::Invalid("You are not in that group.".into()));
                }
                Ok((group.members.iter().map(|(u, _)| *u).collect(), Some(g)))
            }
        }
    }

    async fn store_outgoing(&self, target: Target, content: &Content, kind: &str, body: &str, reply_to: Option<&[u8]>) -> Result<MessageRow, SendError> {
        let eng = self.engine.lock().await;
        let device = eng.device.device_id().unwrap_or(0);
        eng.store
            .insert_message(&NewMessage {
                msg_id: &content.msg_id,
                conversation_id: &content.conversation_id,
                peer_user_id: target.peer(),
                sender_user: eng.user_id,
                sender_device: device,
                outgoing: true,
                sent_at: content.sent_at_ms,
                received_at: content.sent_at_ms,
                kind,
                body,
                reply_to,
                status: Status::Sending,
            })
            .map_err(|e| SendError::Failed(e.to_string()))?;
        eng.store.message(&content.msg_id).ok().flatten().ok_or(SendError::Failed("Could not store it.".into()))
    }

    /// Send a message and record how it went. Offline, it stays "sending" and
    /// goes out on reconnect.
    async fn send_new(&self, target: Target, content: Content, kind: &str, body: &str, reply_to: Option<&[u8]>) -> Result<MessageRow, SendError> {
        let supporter = self.me_supporter();
        validate_outgoing(&content, supporter).map_err(|e| invalid(e, supporter))?;
        let (users, group) = self.recipients(target).await?;
        let row = self.store_outgoing(target, &content, kind, body, reply_to).await?;
        self.emit_row(true, row);
        let result = self.deliver(&users, &content, false, group).await;
        let status = match &result {
            Ok(()) => Some(Status::Sent),
            Err(SendError::Offline) => None,
            Err(_) => Some(Status::Failed),
        };
        let row = {
            let eng = self.engine.lock().await;
            if let Some(s) = status {
                let _ = eng.store.advance_status(&content.msg_id, s);
            }
            eng.store.message(&content.msg_id).ok().flatten()
        };
        if let Some(row) = row.clone() {
            self.emit_row(false, row);
        }
        match result {
            Ok(()) | Err(SendError::Offline) => row.ok_or(SendError::Failed("Could not store it.".into())),
            Err(e) => Err(e),
        }
    }

    pub async fn send_text(&self, target: Target, text: String, reply_to: Option<Vec<u8>>) -> Result<MessageRow, SendError> {
        let me = self.my_id().await;
        let reply = reply_to.filter(|r| r.len() == 16);
        let content = Content {
            msg_id: new_message_id().to_vec(),
            conversation_id: target.conversation(me),
            sent_at_ms: now_ms(),
            body: Some(Body::Text(Text { text: text.clone(), reply_to: reply.clone().unwrap_or_default() })),
        };
        self.send_new(target, content, "text", &text, reply.as_deref()).await
    }

    pub async fn send_gif(&self, target: Target, gif: GifBody) -> Result<MessageRow, SendError> {
        let me = self.my_id().await;
        let body = serde_json::to_string(&gif).map_err(|e| SendError::Failed(e.to_string()))?;
        let content = Content {
            msg_id: new_message_id().to_vec(),
            conversation_id: target.conversation(me),
            sent_at_ms: now_ms(),
            body: Some(Body::Gif(Gif {
                provider: gif.provider,
                id: gif.id,
                url: gif.url,
                width: gif.width,
                height: gif.height,
                title: gif.title,
            })),
        };
        self.send_new(target, content, "gif", &body, None).await
    }

    /// Invite them to a game (with the Steam lobby the host is in, if known).
    pub async fn send_invite(&self, target: Target, mut invite: InviteBody) -> Result<MessageRow, SendError> {
        let me = self.my_id().await;
        invite.expires_at = now_ms() + INVITE_TTL_MS;
        let body = serde_json::to_string(&invite).map_err(|e| SendError::Failed(e.to_string()))?;
        let content = Content {
            msg_id: new_message_id().to_vec(),
            conversation_id: target.conversation(me),
            sent_at_ms: now_ms(),
            body: Some(Body::Invite(invite.to_proto())),
        };
        self.send_new(target, content, "invite", &body, None).await
    }

    /// Encrypt a file here, upload only the ciphertext to the gateway
    /// (`http_base`: its https origin), then send the message that holds
    /// the key.
    pub async fn send_file(&self, target: Target, http_base: &str, name: String, bytes: Vec<u8>) -> Result<MessageRow, SendError> {
        if bytes.len() > km_core::MAX_ATTACHMENT {
            return Err(SendError::Invalid("Files can be up to 25 MB.".into()));
        }
        let client = self.client().ok_or(SendError::Offline)?;
        let size = bytes.len() as u64;
        let sealed = tokio::task::spawn_blocking(move || km_core::seal_attachment(&bytes))
            .await
            .map_err(|e| SendError::Failed(e.to_string()))?
            .map_err(|e| SendError::Failed(e.to_string()))?;
        let ticket = match client
            .request(C::AttachmentTicket(gw::AttachmentTicket { size: sealed.ciphertext.len() as u64 }))
            .await
            .map_err(|e| SendError::Failed(e.message))?
        {
            S::AttachmentUpload(t) => t,
            _ => return Err(SendError::Failed("Unexpected answer.".into())),
        };
        let url = format!("{http_base}/v1/attachments/{}?t={}&s={}", ticket.id, ticket.upload_token, sealed.ciphertext.len());
        let res = reqwest::Client::new()
            .put(url)
            .body(sealed.ciphertext)
            .send()
            .await
            .map_err(|e| SendError::Failed(format!("Could not upload the file: {e}")))?;
        if !res.status().is_success() {
            return Err(SendError::Failed(format!("The upload was refused ({}).", res.status())));
        }
        let name: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\')).take(200).collect();
        let file = FileBody {
            id: ticket.id.to_string(),
            token: ticket.download_token,
            key: hex::encode(sealed.key.as_slice()),
            sha256: hex::encode(sealed.sha256),
            mime: mime_of(&name).into(),
            name: if name.trim().is_empty() { "file".into() } else { name },
            size,
            width: 0,
            height: 0,
        };
        let body = serde_json::to_string(&file).map_err(|e| SendError::Failed(e.to_string()))?;
        let me = self.my_id().await;
        let content = Content {
            msg_id: new_message_id().to_vec(),
            conversation_id: target.conversation(me),
            sent_at_ms: now_ms(),
            body: Some(Body::Attachment(file.to_proto().ok_or(SendError::Failed("Bad file.".into()))?)),
        };
        self.send_new(target, content, "file", &body, None).await
    }

    /// Download, check and decrypt a file from a message on this PC.
    pub async fn fetch_file(&self, msg_id: &[u8], http_base: &str) -> Result<(FileBody, zeroize::Zeroizing<Vec<u8>>), String> {
        let row = self.engine.lock().await.store.message(msg_id).ok().flatten().ok_or("No such message.")?;
        if row.kind != "file" || row.deleted {
            return Err("That message has no file.".into());
        }
        let file: FileBody = serde_json::from_str(&row.body).map_err(|_| "The file details are damaged.".to_string())?;
        let url = format!("{http_base}/v1/attachments/{}?t={}", file.id, file.token);
        let res = reqwest::Client::new().get(url).send().await.map_err(|e| format!("Could not download the file: {e}"))?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err("This file is no longer available (files are kept for 30 days).".into());
        }
        if !res.status().is_success() {
            return Err(format!("The download was refused ({}).", res.status()));
        }
        let bytes = res.bytes().await.map_err(|e| format!("Could not download the file: {e}"))?;
        let key = hex::decode(&file.key).map_err(|_| "The file details are damaged.".to_string())?;
        let sha = hex::decode(&file.sha256).map_err(|_| "The file details are damaged.".to_string())?;
        let plain = km_core::open_attachment(&key, &sha, &bytes).map_err(|_| "The file did not check out; it may have been changed.".to_string())?;
        Ok((file, plain))
    }

    /// Voice call signalling to someone (WebRTC offer/answer/ICE, hang up).
    /// Ephemeral: to their devices that are online right now, never stored.
    pub async fn send_call(&self, peer: u64, call_id: Vec<u8>, kind: CallKind, payload: String) -> Result<(), SendError> {
        if self.awaiting_answer(peer) {
            return Err(SendError::Invalid("Accept their message request first.".into()));
        }
        let me = self.my_id().await;
        let content = Content {
            msg_id: new_message_id().to_vec(),
            conversation_id: dm_conversation_id(me, peer),
            sent_at_ms: now_ms(),
            body: Some(Body::Call(Call { call_id, kind: kind as i32, payload })),
        };
        let supporter = self.me_supporter();
        validate_outgoing(&content, supporter).map_err(|e| invalid(e, supporter))?;
        self.deliver(&[peer], &content, true, None).await
    }

    /// One of your own messages, if it may still be edited or deleted.
    async fn own_recent(&self, msg_id: &[u8]) -> Result<MessageRow, SendError> {
        let row = self.engine.lock().await.store.message(msg_id).ok().flatten();
        match row {
            Some(r) if r.outgoing && !r.deleted && now_ms().saturating_sub(r.sent_at) <= EDIT_WINDOW_MS => Ok(r),
            Some(r) if r.outgoing && !r.deleted => Err(SendError::Invalid("Messages can be changed for 24 hours.".into())),
            _ => Err(SendError::Invalid("That message cannot be changed.".into())),
        }
    }

    fn target_of(&self, row: &MessageRow, me: u64) -> Result<Target, SendError> {
        hex::decode(&row.conversation_id)
            .ok()
            .and_then(|c| Target::from_conversation(&c, me))
            .ok_or(SendError::Invalid("Not a conversation you are in.".into()))
    }

    async fn control(&self, target: Target, body: Body) -> Result<(), SendError> {
        let me = self.my_id().await;
        let content = Content { msg_id: new_message_id().to_vec(), conversation_id: target.conversation(me), sent_at_ms: now_ms(), body: Some(body) };
        let supporter = self.me_supporter();
        validate_outgoing(&content, supporter).map_err(|e| invalid(e, supporter))?;
        let (users, group) = self.recipients(target).await?;
        self.deliver(&users, &content, false, group).await
    }

    pub async fn edit(&self, msg_id: Vec<u8>, text: String) -> Result<MessageRow, SendError> {
        let row = self.own_recent(&msg_id).await?;
        if row.kind != "text" {
            return Err(SendError::Invalid("Only text can be edited.".into()));
        }
        let me = self.my_id().await;
        let target = self.target_of(&row, me)?;
        self.control(target, Body::Edit(Edit { target: msg_id.clone(), text: text.clone() })).await?;
        let eng = self.engine.lock().await;
        let _ = eng.store.apply_edit(&msg_id, me, &text, now_ms());
        let updated = eng.store.message(&msg_id).ok().flatten().ok_or(SendError::Failed("Gone.".into()))?;
        drop(eng);
        self.emit_row(false, updated.clone());
        Ok(updated)
    }

    pub async fn delete(&self, msg_id: Vec<u8>) -> Result<(), SendError> {
        let row = self.own_recent(&msg_id).await?;
        let me = self.my_id().await;
        let target = self.target_of(&row, me)?;
        self.control(target, Body::Delete(Delete { target: msg_id.clone() })).await?;
        let eng = self.engine.lock().await;
        let _ = eng.store.apply_delete(&msg_id, me);
        let updated = eng.store.message(&msg_id).ok().flatten();
        drop(eng);
        if let Some(u) = updated {
            self.emit_row(false, u);
        }
        Ok(())
    }

    pub async fn react(&self, msg_id: Vec<u8>, emoji: String, remove: bool) -> Result<(), SendError> {
        let me = self.my_id().await;
        let row = self.engine.lock().await.store.message(&msg_id).ok().flatten().ok_or(SendError::Invalid("No such message.".into()))?;
        let target = self.target_of(&row, me)?;
        self.control(target, Body::Reaction(Reaction { target: msg_id.clone(), emoji: emoji.clone(), remove })).await?;
        let eng = self.engine.lock().await;
        let _ = eng.store.apply_reaction(&msg_id, me, &emoji, remove);
        let updated = eng.store.message(&msg_id).ok().flatten();
        drop(eng);
        if let Some(u) = updated {
            self.emit_row(false, u);
        }
        Ok(())
    }

    /// "typing..." for someone. Not sent if typing indicators are off, never
    /// stored, dropped if they are offline.
    pub async fn typing(&self, target: Target, active: bool) -> Result<(), SendError> {
        if !self.settings().await.typing || target.peer().is_some_and(|p| self.awaiting_answer(p)) {
            return Ok(());
        }
        let me = self.my_id().await;
        let content = Content {
            msg_id: new_message_id().to_vec(),
            conversation_id: target.conversation(me),
            sent_at_ms: now_ms(),
            body: Some(Body::Typing(Typing { active })),
        };
        let (users, group) = self.recipients(target).await?;
        let others: Vec<u64> = users.into_iter().filter(|u| *u != me).collect();
        self.deliver(&others, &content, true, group).await
    }

    /// Mark a conversation read; tells them, if read receipts are on.
    pub async fn mark_read(&self, target: Target) -> Result<(), SendError> {
        let me = self.my_id().await;
        let conv = target.conversation(me);
        let ids = self.engine.lock().await.store.mark_read(&conv, now_ms()).unwrap_or_default();
        if ids.is_empty() {
            return Ok(());
        }
        let content = Content {
            msg_id: new_message_id().to_vec(),
            conversation_id: conv,
            sent_at_ms: now_ms(),
            body: Some(Body::Receipt(Receipt { kind: ReceiptKind::Read as i32, msg_ids: ids })),
        };
        match target {
            // Groups have no read receipts; our other devices still learn
            // the conversation was read.
            Target::Group(g) => self.deliver(&[me], &content, false, Some(g)).await,
            Target::Dm(peer) => {
                if !self.settings().await.read_receipts || self.awaiting_answer(peer) {
                    return Ok(());
                }
                // Our other devices get it too (the gateway insists on that
                // for anything stored), and mark the conversation read there.
                self.deliver(&[peer, me], &content, false, None).await
            }
        }
    }

    /// Send again what never reached the server.
    pub async fn resend_pending(&self) {
        let pending = self.engine.lock().await.store.pending_outgoing().unwrap_or_default();
        let me = self.my_id().await;
        for row in pending {
            let Ok(msg_id) = hex::decode(&row.msg_id) else { continue };
            let Ok(conv) = hex::decode(&row.conversation_id) else { continue };
            let Some(target) = Target::from_conversation(&conv, me) else { continue };
            let body = match row.kind.as_str() {
                "gif" => match serde_json::from_str::<GifBody>(&row.body) {
                    Ok(g) => Body::Gif(Gif { provider: g.provider, id: g.id, url: g.url, width: g.width, height: g.height, title: g.title }),
                    Err(_) => continue,
                },
                "invite" => match serde_json::from_str::<InviteBody>(&row.body) {
                    Ok(i) => Body::Invite(i.to_proto()),
                    Err(_) => continue,
                },
                "file" => match serde_json::from_str::<FileBody>(&row.body).ok().and_then(|f| f.to_proto()) {
                    Some(a) => Body::Attachment(a),
                    None => continue,
                },
                _ => Body::Text(Text {
                    text: row.body.clone(),
                    reply_to: row.reply_to.as_deref().and_then(|r| hex::decode(r).ok()).unwrap_or_default(),
                }),
            };
            // The same message id: anyone who already got it drops the copy.
            let content = Content { msg_id: msg_id.clone(), conversation_id: conv, sent_at_ms: row.sent_at, body: Some(body) };
            let Ok((users, group)) = self.recipients(target).await else { continue };
            let status = match self.deliver(&users, &content, false, group).await {
                Ok(()) => Status::Sent,
                Err(SendError::Offline) => return,
                Err(_) => Status::Failed,
            };
            let updated = {
                let eng = self.engine.lock().await;
                let _ = eng.store.advance_status(&msg_id, status);
                eng.store.message(&msg_id).ok().flatten()
            };
            if let Some(u) = updated {
                self.emit_row(false, u);
            }
        }
    }

    /// One delivery: decrypt, apply, report. Errors are logged and the
    /// delivery dropped (the server's copy is acknowledged either way: a
    /// message nobody can decrypt does not get better by trying again).
    pub async fn handle_delivery(&self, d: &gw::Deliver) {
        let me = self.my_id().await;
        let inbound = {
            let attempt = {
                let mut eng = self.engine.lock().await;
                let cache = eng.devices.clone();
                eng.device.decrypt(&d.envelope, |u, dev| cache.get(&u).and_then(|l| l.iter().find(|t| t.keys().device_id == dev).cloned()))
            };
            match attempt {
                Err(CoreError::UnknownSender { user_id, .. }) => {
                    // A device we have not seen: look it up and try once more.
                    let Some(client) = self.client() else { return };
                    if self.devices_of(&client, user_id, true).await.is_err() {
                        return;
                    }
                    let mut eng = self.engine.lock().await;
                    let cache = eng.devices.clone();
                    eng.device.decrypt(&d.envelope, |u, dev| cache.get(&u).and_then(|l| l.iter().find(|t| t.keys().device_id == dev).cloned()))
                }
                other => other,
            }
        };
        match inbound {
            Ok(i) => self.apply(me, i).await,
            Err(CoreError::Duplicate) => {}
            Err(e) => logging::warn("chat", &format!("dropped a message that did not decrypt: {e}")),
        }
        // Sessions moved forward: saved only now, after the message is stored
        // and before it is acknowledged. Stopped in between, the server sends
        // it again, the old session decrypts it again, and the store keeps
        // one copy; saved first, a stop would lose it for good.
        let _ = self.engine.lock().await.save();
    }

    /// Store and act on one decrypted message.
    async fn apply(&self, me: u64, inbound: km_core::Inbound) {
        let sender = inbound.sender.user_id;
        let content = inbound.content;
        // It must belong to a conversation between this sender and us (or be
        // a copy from one of our own devices), or to a group they are in.
        let Some(target) = Target::from_conversation(&content.conversation_id, me) else { return };
        match target {
            Target::Dm(peer) if sender != me && peer != sender => return,
            Target::Group(g) if !self.is_member(g, sender).await => return,
            _ => {}
        }
        let from_me = sender == me;
        // Edits, deletes and reactions only touch messages in the same conversation.
        let same_conversation = |id: &[u8], eng: &super::engine::Engine| {
            eng.store.message(id).ok().flatten().and_then(|m| hex::decode(m.conversation_id).ok()).as_deref() == Some(&content.conversation_id[..])
        };
        match content.body {
            Some(Body::Invite(ref i)) if !km_core::chat::invite_valid(i) => {}
            Some(Body::Attachment(ref a)) if !km_core::chat::attachment_valid(a) => {}
            Some(Body::Text(_)) | Some(Body::Gif(_)) | Some(Body::Invite(_)) | Some(Body::Attachment(_)) => {
                let (kind, body, reply) = match &content.body {
                    Some(Body::Text(t)) => ("text", t.text.clone(), (t.reply_to.len() == 16).then(|| t.reply_to.clone())),
                    Some(Body::Gif(g)) => (
                        "gif",
                        serde_json::to_string(&GifBody {
                            provider: g.provider.clone(),
                            id: g.id.clone(),
                            url: g.url.clone(),
                            width: g.width,
                            height: g.height,
                            title: g.title.clone(),
                        })
                        .unwrap_or_default(),
                        None,
                    ),
                    Some(Body::Invite(i)) => ("invite", serde_json::to_string(&InviteBody::from_proto(i)).unwrap_or_default(), None),
                    Some(Body::Attachment(a)) => ("file", serde_json::to_string(&FileBody::from_proto(a)).unwrap_or_default(), None),
                    _ => unreachable!(),
                };
                let row = {
                    let eng = self.engine.lock().await;
                    let inserted = eng
                        .store
                        .insert_message(&NewMessage {
                            msg_id: &content.msg_id,
                            conversation_id: &content.conversation_id,
                            peer_user_id: target.peer(),
                            sender_user: sender,
                            sender_device: inbound.sender.device_id,
                            outgoing: from_me,
                            sent_at: content.sent_at_ms,
                            received_at: now_ms(),
                            kind,
                            body: &body,
                            reply_to: reply.as_deref(),
                            status: if from_me { Status::Sent } else { Status::Received },
                        })
                        .unwrap_or(false);
                    if inserted { eng.store.message(&content.msg_id).ok().flatten() } else { None }
                };
                let Some(row) = row else { return };
                let shown = self.shown(row);
                (self.emit)(ChatEvent::Message(shown.clone()));
                if !from_me {
                    let muted = self.context.lock().expect("context").people.get(&sender).is_some_and(|p| p.muted);
                    if !muted {
                        let preview = (self.settings().await.notification_content == "full").then(|| match shown.kind.as_str() {
                            "invite" => serde_json::from_str::<InviteBody>(&shown.body)
                                .map(|i| format!("Invites you to play {}", i.title))
                                .unwrap_or_else(|_| "Sent a game invite".into()),
                            "gif" => "Sent a GIF".into(),
                            "file" => serde_json::from_str::<serde_json::Value>(&shown.body)
                                .ok()
                                .and_then(|v| v["name"].as_str().map(|n| format!("Sent a file: {n}")))
                                .unwrap_or_else(|| "Sent a file".into()),
                            _ => shown.body.chars().take(140).collect(),
                        });
                        let group_name = match target {
                            Target::Group(g) => Some(self.group_name(g).await),
                            Target::Dm(_) => None,
                        };
                        (self.emit)(ChatEvent::Notify {
                            conversation_id: shown.conversation_id.clone(),
                            sender_user: sender.to_string(),
                            preview,
                            group_name,
                        });
                    }
                    // Delivered (always once they may talk to us; read
                    // receipts are the optional ones). Not in groups.
                    if self.awaiting_answer(sender) || target.group().is_some() {
                        return;
                    }
                    let receipt = Content {
                        msg_id: new_message_id().to_vec(),
                        conversation_id: content.conversation_id.clone(),
                        sent_at_ms: now_ms(),
                        body: Some(Body::Receipt(Receipt { kind: ReceiptKind::Delivered as i32, msg_ids: vec![content.msg_id.clone()] })),
                    };
                    if let Err(e) = self.deliver(&[sender, me], &receipt, false, None).await {
                        logging::warn("chat", &format!("delivery receipt not sent: {e}"));
                    }
                }
            }
            Some(Body::Receipt(r)) if from_me && ReceiptKind::try_from(r.kind) == Ok(ReceiptKind::Read) => {
                // Our other device read this conversation: it is read here too.
                let _ = self.engine.lock().await.store.mark_read(&content.conversation_id, now_ms());
                (self.emit)(ChatEvent::ReadElsewhere { conversation_id: hex::encode(&content.conversation_id) });
            }
            Some(Body::Receipt(r)) if !from_me => {
                let status = match ReceiptKind::try_from(r.kind).unwrap_or(ReceiptKind::Unspecified) {
                    ReceiptKind::Delivered => Status::Delivered,
                    // Read receipts work both ways: with yours off, theirs are not shown either.
                    ReceiptKind::Read if self.settings().await.read_receipts => Status::Read,
                    _ => return,
                };
                for id in r.msg_ids.iter().take(500) {
                    let updated = {
                        let eng = self.engine.lock().await;
                        // Only receipts about our own messages in this conversation.
                        match eng.store.message(id).ok().flatten() {
                            Some(m) if m.outgoing && hex::decode(&m.conversation_id).ok().as_deref() == Some(&content.conversation_id[..]) => {
                                if eng.store.advance_status(id, status).unwrap_or(false) { eng.store.message(id).ok().flatten() } else { None }
                            }
                            _ => None,
                        }
                    };
                    if let Some(u) = updated {
                        self.emit_row(false, u);
                    }
                }
            }
            Some(Body::Typing(t)) if !from_me && self.settings().await.typing => {
                (self.emit)(ChatEvent::Typing {
                    conversation_id: hex::encode(&content.conversation_id),
                    user_id: sender.to_string(),
                    active: t.active,
                });
            }
            Some(Body::Edit(e)) => {
                let updated = {
                    let eng = self.engine.lock().await;
                    if same_conversation(&e.target, &eng) && eng.store.apply_edit(&e.target, sender, &e.text, content.sent_at_ms).unwrap_or(false) {
                        eng.store.message(&e.target).ok().flatten()
                    } else {
                        None
                    }
                };
                if let Some(u) = updated {
                    self.emit_row(false, u);
                }
            }
            Some(Body::Delete(x)) => {
                let updated = {
                    let eng = self.engine.lock().await;
                    if same_conversation(&x.target, &eng) && eng.store.apply_delete(&x.target, sender).unwrap_or(false) {
                        eng.store.message(&x.target).ok().flatten()
                    } else {
                        None
                    }
                };
                if let Some(u) = updated {
                    self.emit_row(false, u);
                }
            }
            Some(Body::Reaction(r)) => {
                let updated = {
                    let eng = self.engine.lock().await;
                    if same_conversation(&r.target, &eng) && eng.store.apply_reaction(&r.target, sender, &r.emoji, r.remove).unwrap_or(false) {
                        eng.store.message(&r.target).ok().flatten()
                    } else {
                        None
                    }
                };
                if let Some(u) = updated {
                    self.emit_row(false, u);
                }
            }
            Some(Body::Call(c)) if !from_me => {
                // Calls are one-to-one, from people who may talk to us.
                if let (Target::Dm(_), true, false) = (target, km_core::chat::call_valid(&c), self.awaiting_answer(sender)) {
                    let kind = match CallKind::try_from(c.kind) {
                        Ok(CallKind::Offer) => "offer",
                        Ok(CallKind::Answer) => "answer",
                        Ok(CallKind::Ice) => "ice",
                        Ok(CallKind::Hangup) => "hangup",
                        Ok(CallKind::Decline) => "decline",
                        Ok(CallKind::Busy) => "busy",
                        Ok(CallKind::Restart) => "restart",
                        Ok(CallKind::Restarted) => "restarted",
                        Ok(CallKind::Muted) => "muted",
                        Ok(CallKind::Unmuted) => "unmuted",
                        _ => return,
                    };
                    (self.emit)(ChatEvent::Call {
                        user_id: sender.to_string(),
                        call_id: hex::encode(&c.call_id),
                        kind: kind.into(),
                        payload: c.payload,
                    });
                }
            }
            Some(Body::GroupMeta(m)) => {
                if let Target::Group(g) = target {
                    let name: String = m.name.trim().chars().take(km_core::chat::GROUP_NAME_LIMIT).collect();
                    if !name.is_empty() {
                        let _ = self.engine.lock().await.store.set_setting(&group_name_key(g), &name);
                        (self.emit)(ChatEvent::GroupChanged { group_id: g.to_string() });
                    }
                }
            }
            _ => {}
        }
    }

    // ---- groups ------------------------------------------------------------------

    /// The group's members, from the cache unless `refresh` (or not cached).
    /// None when this account is not (or no longer) in it.
    pub(super) async fn group_cache(&self, group: u64, refresh: bool) -> Option<GroupCache> {
        if !refresh {
            if let Some(g) = self.engine.lock().await.groups.get(&group) {
                return Some(g.clone());
            }
        }
        let client = self.client()?;
        match client.request(C::GroupGet(gw::GroupRef { group_id: group })).await {
            Ok(S::Group(info)) => Some(self.remember_group(info).await),
            Err(e) if e.code == "not_found" => {
                self.engine.lock().await.groups.remove(&group);
                None
            }
            _ => self.engine.lock().await.groups.get(&group).cloned(),
        }
    }

    async fn remember_group(&self, info: gw::GroupInfo) -> GroupCache {
        let cache = GroupCache { members: info.members.into_iter().map(|m| (m.user_id, m.role)).collect() };
        self.engine.lock().await.groups.insert(info.group_id, cache.clone());
        cache
    }

    async fn is_member(&self, group: u64, user: u64) -> bool {
        if let Some(g) = self.engine.lock().await.groups.get(&group) {
            if g.members.iter().any(|(u, _)| *u == user) {
                return true;
            }
        }
        // Someone just added, or a group we have not looked at yet.
        self.group_cache(group, true).await.is_some_and(|g| g.members.iter().any(|(u, _)| *u == user))
    }

    pub async fn group_name(&self, group: u64) -> String {
        self.engine
            .lock()
            .await
            .store
            .setting(&group_name_key(group))
            .ok()
            .flatten()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Group chat".into())
    }

    async fn view(&self, group: u64, cache: &GroupCache) -> GroupView {
        let me = self.my_id().await;
        GroupView {
            id: group.to_string(),
            name: self.group_name(group).await,
            members: cache.members.iter().map(|(u, r)| GroupMemberView { user_id: u.to_string(), role: r.clone() }).collect(),
            my_role: cache.members.iter().find(|(u, _)| *u == me).map(|(_, r)| r.clone()).unwrap_or_default(),
        }
    }

    /// Every group this account is in, fresh from the gateway.
    pub async fn groups(&self) -> Result<Vec<GroupView>, String> {
        let client = self.client().ok_or("Chat is offline.")?;
        let S::Groups(list) = client.request(C::GroupList(gw::Empty {})).await.map_err(|e| e.message)? else {
            return Err("Unexpected answer.".into());
        };
        let ids: Vec<u64> = list.groups.iter().map(|g| g.group_id).collect();
        self.engine.lock().await.groups.retain(|id, _| ids.contains(id));
        let mut out = Vec::new();
        for info in list.groups {
            let id = info.group_id;
            let cache = self.remember_group(info).await;
            out.push(self.view(id, &cache).await);
        }
        Ok(out)
    }

    /// Tell the members the group's name (encrypted; the server never sees it).
    async fn send_group_name(&self, group: u64, name: &str) -> Result<(), SendError> {
        let name: String = name.trim().chars().take(km_core::chat::GROUP_NAME_LIMIT).collect();
        self.engine.lock().await.store.set_setting(&group_name_key(group), &name).map_err(|e| SendError::Failed(e.to_string()))?;
        self.control(Target::Group(group), Body::GroupMeta(GroupMeta { name })).await
    }

    pub async fn group_create(&self, name: &str, members: Vec<u64>) -> Result<GroupView, String> {
        if name.trim().is_empty() {
            return Err("Give the group a name.".into());
        }
        let client = self.client().ok_or("Chat is offline.")?;
        let S::Group(info) = client.request(C::GroupCreate(gw::GroupCreate { member_ids: members })).await.map_err(|e| e.message)? else {
            return Err("Unexpected answer.".into());
        };
        let id = info.group_id;
        let cache = self.remember_group(info).await;
        self.send_group_name(id, name).await.map_err(|e| e.to_string())?;
        Ok(self.view(id, &cache).await)
    }

    pub async fn group_rename(&self, group: u64, name: &str) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("Give the group a name.".into());
        }
        self.send_group_name(group, name).await.map_err(|e| e.to_string())?;
        (self.emit)(ChatEvent::GroupChanged { group_id: group.to_string() });
        Ok(())
    }

    pub async fn group_add(&self, group: u64, users: Vec<u64>) -> Result<GroupView, String> {
        let client = self.client().ok_or("Chat is offline.")?;
        let S::Group(info) = client.request(C::GroupAdd(gw::GroupAdd { group_id: group, user_ids: users })).await.map_err(|e| e.message)? else {
            return Err("Unexpected answer.".into());
        };
        let cache = self.remember_group(info).await;
        // The new people learn the name the same way everyone did.
        let name = self.group_name(group).await;
        let _ = self.send_group_name(group, &name).await;
        Ok(self.view(group, &cache).await)
    }

    /// Remove someone, or (`user` = us) leave. History stays on this PC.
    pub async fn group_remove(&self, group: u64, user: u64) -> Result<(), String> {
        let client = self.client().ok_or("Chat is offline.")?;
        match client.request(C::GroupRemove(gw::GroupRemove { group_id: group, user_id: user })).await.map_err(|e| e.message)? {
            S::Group(info) => {
                self.remember_group(info).await;
            }
            _ => {
                self.engine.lock().await.groups.remove(&group);
            }
        }
        (self.emit)(ChatEvent::GroupChanged { group_id: group.to_string() });
        Ok(())
    }

    /// The gateway says a group changed: refresh it and tell the shell.
    pub async fn on_group_changed(&self, group: u64) {
        let _ = self.group_cache(group, true).await;
        (self.emit)(ChatEvent::GroupChanged { group_id: group.to_string() });
    }
}
