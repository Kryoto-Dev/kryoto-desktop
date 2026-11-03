//! End-to-end encrypted chat: this PC as a chat device.
//!
//! The cryptography is kryoto-messaging's (km-core); keys never reach a web
//! view. The shell's own view (`main`) may call the commands below
//! (permissions/chat.toml, capabilities/chat.json); the Store's pages may not.
//! The shell only ever sees plaintext for display.
//!
//! See FRIENDS-AND-CHAT.md at the top of the workspace.

mod auth;
mod engine;
mod identity;
mod messaging;
mod secrets;
mod types;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use zeroize::Zeroizing;

use crate::{logging, settings};
use engine::{Chat, Engine};
use messaging::{GifBody, GroupView, InviteBody, Target};
pub use types::ChatStatus;
use types::{ChatEvent, Person, Settings};

#[derive(Default)]
pub struct ChatState {
    status: Mutex<Option<ChatStatus>>,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    chat: Mutex<Option<Arc<Chat>>>,
    /// The shell's latest people context, kept so a restart of chat gets it.
    people: Mutex<(Vec<Person>, bool)>,
    /// One enable/remove at a time.
    busy: tokio::sync::Mutex<()>,
}

fn set_status<R: Runtime>(app: &AppHandle<R>, status: ChatStatus) {
    if let Some(state) = app.try_state::<ChatState>() {
        *state.status.lock().expect("chat state") = Some(status.clone());
    }
    let _ = app.emit_to("main", "chat-status", &status);
}

/// The account id chat was last turned on for. Not a secret; lets chat start
/// at launch without asking kryo.to first.
#[derive(Serialize, Deserialize)]
struct Account {
    user_id: u64,
}

fn chat_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("chat"))
}

fn account_file<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(chat_dir(app)?.join("account.json"))
}

fn api_base<R: Runtime>(app: &AppHandle<R>) -> String {
    settings::catalog_endpoint(&settings::load(app)).trim_end_matches('/').to_string()
}

/// Where the gateway is. Production: ws.kryo.to. A debug build pointed at
/// another catalog endpoint can name its own with KRYOTO_GATEWAY_URL.
fn gateway_url() -> String {
    if cfg!(debug_assertions) {
        if let Ok(url) = std::env::var("KRYOTO_GATEWAY_URL") {
            return url;
        }
    }
    "wss://ws.kryo.to/v1/ws".to_string()
}

/// The gateway's https origin (attachments), from its WebSocket address.
fn gateway_http() -> String {
    let ws = gateway_url();
    let base = ws.strip_suffix("/v1/ws").unwrap_or(&ws);
    if let Some(rest) = base.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = base.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        base.to_string()
    }
}

/// A native file dialog (built on the main thread, awaited off it).
async fn file_dialog(window: &tauri::Window, save_as: Option<String>) -> Result<Option<PathBuf>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let parent = window.clone();
    window
        .run_on_main_thread(move || {
            let dialog = rfd::AsyncFileDialog::new().set_parent(&parent);
            type Picked = std::pin::Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>>;
            let picked: Picked = match save_as {
                Some(name) => Box::pin(dialog.set_title("Save file").set_file_name(name).save_file()),
                None => Box::pin(dialog.set_title("Send a file (up to 25 MB)").pick_file()),
            };
            std::thread::spawn(move || {
                let _ = tx.send(tauri::async_runtime::block_on(picked).map(|h| h.path().to_path_buf()));
            });
        })
        .map_err(|e| e.to_string())?;
    rx.await.map_err(|_| "The file dialog closed unexpectedly.".to_string())
}

/// What the engine reports, turned into app events and notifications.
fn emitter(app: AppHandle) -> types::Emit {
    Arc::new(move |event: ChatEvent| match event {
        ChatEvent::Status(s) => set_status(&app, s),
        ChatEvent::Message(m) => {
            let _ = app.emit_to("main", "chat-message", &m);
            if !m.outgoing {
                refresh_badge(&app);
            }
        }
        ChatEvent::Updated(m) => {
            let _ = app.emit_to("main", "chat-updated", &m);
        }
        ChatEvent::Typing { conversation_id, user_id, active } => {
            let _ = app.emit_to(
                "main",
                "chat-typing",
                serde_json::json!({ "conversationId": conversation_id, "userId": user_id, "active": active }),
            );
        }
        ChatEvent::ReadElsewhere { conversation_id } => {
            let _ = app.emit_to("main", "chat-read", serde_json::json!({ "conversationId": conversation_id }));
            refresh_badge(&app);
        }
        ChatEvent::IdentityChanged { user_id } => {
            let _ = app.emit_to("main", "chat-identity-changed", serde_json::json!({ "userId": user_id }));
        }
        ChatEvent::GroupChanged { group_id } => {
            let _ = app.emit_to("main", "chat-group-changed", serde_json::json!({ "groupId": group_id }));
        }
        ChatEvent::Call { user_id, call_id, kind, payload } => {
            // Once per call: one ringing call is one notice, however many
            // offers it takes to connect.
            if kind == "offer" && first_ring(&call_id) {
                // Ring: bring the window up, and tell the person if it was hidden.
                // `get_window`, not `get_webview_window`: the main window holds
                // several views, and the latter finds none for it.
                let focused = app.get_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
                if !focused {
                    let state = app.state::<ChatState>();
                    let name = state.people.lock().expect("people").0.iter().find(|p| p.id == user_id).map(|p| p.name.clone());
                    crate::system::os_notify(app.clone(), format!("{} is calling", name.unwrap_or_else(|| "Someone".into())), None);
                    crate::system::show_main(&app);
                }
            }
            let _ = app.emit_to(
                "main",
                "chat-call",
                serde_json::json!({ "userId": user_id, "callId": call_id, "kind": kind, "payload": payload }),
            );
        }
        ChatEvent::Notify { conversation_id, sender_user, preview, group_name } => {
            // Not while the window is in front: the message is on screen.
            let focused = app.get_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
            let _ = app.emit_to("main", "chat-unread", serde_json::json!({ "conversationId": conversation_id }));
            if focused {
                return;
            }
            let state = app.state::<ChatState>();
            let name = state.people.lock().expect("people").0.iter().find(|p| p.id == sender_user).map(|p| p.name.clone());
            let chat = state.chat.lock().expect("chat").clone();
            let app2 = app.clone();
            tauri::async_runtime::spawn(async move {
                let mode = match &chat {
                    Some(c) => c.settings().await.notification_content,
                    None => "name".into(),
                };
                let who = name.unwrap_or_else(|| "New message".into());
                let title = match (mode.as_str(), group_name) {
                    ("none", _) => "New message on Kryoto".to_string(),
                    (_, Some(group)) => format!("{who} in {group}"),
                    (_, None) => who,
                };
                let body = if mode == "full" { preview } else { None };
                crate::system::os_notify(app2, title, body);
            });
        }
    })
}

/// Whether this call has not rung here before (and remember it).
fn first_ring(call_id: &str) -> bool {
    static RUNG: std::sync::Mutex<std::collections::VecDeque<String>> = std::sync::Mutex::new(std::collections::VecDeque::new());
    let Ok(mut rung) = RUNG.lock() else { return true };
    if rung.iter().any(|c| c == call_id) {
        return false;
    }
    rung.push_back(call_id.to_string());
    if rung.len() > 64 {
        rung.pop_front();
    }
    true
}

/// Count unread messages (people you muted left out) onto the tray and
/// taskbar badge.
fn refresh_badge<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    let Some(chat) = app.state::<ChatState>().chat.lock().expect("chat state").clone() else {
        crate::system::set_unread_badge(&app, 0);
        return;
    };
    tauri::async_runtime::spawn(async move {
        let muted: std::collections::HashSet<String> = {
            let ctx = chat.context.lock().expect("context");
            ctx.people.values().filter(|p| p.muted).map(|p| p.id.clone()).collect()
        };
        let total: u64 = chat
            .conversations()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|c| c.peer_user_id.as_ref().is_none_or(|p| !muted.contains(p)))
            .map(|c| c.unread)
            .sum();
        crate::system::set_unread_badge(&app, total.min(u64::from(u32::MAX)) as u32);
    });
}

fn stop_task(state: &ChatState) {
    if let Some(task) = state.task.lock().expect("chat state").take() {
        task.abort();
    }
    *state.chat.lock().expect("chat state") = None;
}

/// Open the account's chat store and stay connected in the background.
fn start(app: &AppHandle, token: Zeroizing<String>, user_id: u64) -> Result<(), String> {
    let state = app.state::<ChatState>();
    stop_task(&state);
    let key = secrets::db_key(user_id)?;
    let chat = Chat::new(Engine::open(&chat_dir(app)?, user_id, &key)?, emitter(app.clone()));
    {
        let (people, me_supporter) = state.people.lock().expect("people").clone();
        let mut ctx = chat.context.lock().expect("context");
        ctx.people = people.into_iter().filter_map(|p| Some((p.id.parse().ok()?, p))).collect();
        ctx.me_supporter = me_supporter;
    }
    *state.chat.lock().expect("chat state") = Some(chat.clone());
    let handle = tauri::async_runtime::spawn(engine::run(chat, token, gateway_url()));
    *state.task.lock().expect("chat state") = Some(handle);
    Ok(())
}

/// At launch: if chat was turned on here before, connect again.
pub fn autostart(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        locate_secrets(&app);
        let Ok(path) = account_file(&app) else { return };
        let Ok(text) = std::fs::read_to_string(path) else {
            set_status(&app, ChatStatus::Off);
            return;
        };
        let Ok(account) = serde_json::from_str::<Account>(&text) else { return };
        // No secure store here: wait for the passphrase (chat_unlock starts it).
        if let secrets::Backend::Passphrase { exists, unlocked: false } = secrets::backend() {
            set_status(&app, if exists { ChatStatus::Locked { creating: false } } else { ChatStatus::SignInNeeded });
            return;
        }
        match secrets::read(secrets::SESSION) {
            Ok(Some(token)) => {
                if let Err(e) = start(&app, token, account.user_id) {
                    logging::error("chat", &e);
                    set_status(&app, ChatStatus::Unavailable { reason: e });
                }
            }
            Ok(None) => set_status(&app, ChatStatus::SignInNeeded),
            Err(e) => set_status(&app, ChatStatus::Unavailable { reason: e }),
        }
    });
}

fn locate_secrets<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(dir) = chat_dir(app) {
        secrets::set_file_location(dir.join("secrets.sealed"));
    }
}

/// Type the chat passphrase (systems with no secure store): unlocks the
/// sealed secrets, or sets the passphrase the first time. Chat that was on
/// here before connects again; otherwise the shell goes on to turn it on.
#[tauri::command]
pub async fn chat_unlock(app: AppHandle, passphrase: String) -> Result<ChatStatus, String> {
    locate_secrets(&app);
    let passphrase = Zeroizing::new(passphrase);
    // Argon2id takes a moment on purpose: not on the async runtime's threads.
    tauri::async_runtime::spawn_blocking(move || secrets::unlock(&passphrase)).await.map_err(|e| e.to_string())??;
    let account = std::fs::read_to_string(account_file(&app)?).ok().and_then(|t| serde_json::from_str::<Account>(&t).ok());
    match (account, secrets::read(secrets::SESSION)?) {
        (Some(a), Some(token)) => {
            start(&app, token, a.user_id)?;
            Ok(ChatStatus::Connecting)
        }
        _ => {
            set_status(&app, ChatStatus::Off);
            Ok(ChatStatus::Off)
        }
    }
}

fn current(state: &State<'_, ChatState>) -> Result<Arc<Chat>, String> {
    state.chat.lock().expect("chat state").clone().ok_or_else(|| "Chat is not on.".to_string())
}

fn user_id(s: &str) -> Result<u64, String> {
    s.parse().map_err(|_| "Not an account id.".to_string())
}

/// A conversation as the shell names it: "123" (a person) or "g:123" (a group).
fn target(s: &str) -> Result<Target, String> {
    Target::parse(s).ok_or_else(|| "Not a conversation.".to_string())
}

fn msg_id(s: &str) -> Result<Vec<u8>, String> {
    hex::decode(s).ok().filter(|b| b.len() == 16).ok_or_else(|| "Not a message id.".to_string())
}

#[tauri::command]
pub fn chat_status(state: State<'_, ChatState>) -> ChatStatus {
    state.status.lock().expect("chat state").clone().unwrap_or(ChatStatus::Off)
}

/// Turn chat on for this PC (the shell asks the person first).
///
/// Reuses a still-valid sign-in; otherwise starts kryo.to's device sign-in and
/// has the Store page, which is signed in, approve exactly that code.
#[tauri::command]
pub async fn chat_enable(app: AppHandle) -> Result<ChatStatus, String> {
    let state = app.state::<ChatState>();
    let _guard = state.busy.lock().await;
    locate_secrets(&app);
    // No OS secure store (Linux without GNOME Keyring/KWallet): a passphrase
    // protects chat's keys instead. Ask for it first.
    if let secrets::Backend::Passphrase { exists, unlocked: false } = secrets::backend() {
        set_status(&app, ChatStatus::Locked { creating: !exists });
        return Err(secrets::LOCKED.into());
    }
    let base = api_base(&app);

    let existing = secrets::read(secrets::SESSION)?;
    let valid = match &existing {
        Some(t) => auth::me(&base, t).await?.is_some(),
        None => false,
    };
    let token = match existing {
        Some(t) if valid => t,
        _ => {
            let view = crate::store(&app).map_err(|_| "Open the Store and sign in to kryo.to first.".to_string())?;
            let base_url = url::Url::parse(&base).map_err(|e| e.to_string())?;
            let signed_in_here = view.url().map(|u| u.origin() == base_url.origin()).unwrap_or(false);
            if !signed_in_here {
                return Err("Open kryo.to in the Store and sign in first.".into());
            }
            let started = auth::start(&base, &format!("Chat on {}", whoami())).await?;
            view.eval(auth::approve_script(&started.user_code)?).map_err(|e| e.to_string())?;
            let token = auth::wait_for_token(&base, &started).await?;
            secrets::write(secrets::SESSION, &token)?;
            token
        }
    };

    let me = auth::me(&base, &token).await?.ok_or_else(|| "kryo.to did not accept the new sign-in.".to_string())?;
    if me.anonymous {
        set_status(&app, ChatStatus::AnonymousMode);
        return Err("You need to turn off anonymous mode to use this feature.".into());
    }
    let dir = chat_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(account_file(&app)?, serde_json::to_string(&Account { user_id: me.id }).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    start(&app, token, me.id)?;
    logging::info("chat", "turned on for this PC");
    Ok(ChatStatus::Connecting)
}

/// Take this PC out of chat: revoke the device on the server, sign its
/// session out, delete its keys and encrypted database. Message history on
/// this PC is gone afterwards; the shell says so before calling this.
#[tauri::command]
pub async fn chat_remove_device(app: AppHandle) -> Result<(), String> {
    let state = app.state::<ChatState>();
    let _guard = state.busy.lock().await;
    let base = api_base(&app);
    let token = secrets::read(secrets::SESSION)?;
    let chat = state.chat.lock().expect("chat state").clone();
    // Wait for the connection task to really end: on Windows an open handle
    // to the database would stop the folder from being deleted below.
    let task = state.task.lock().expect("chat state").take();
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
    stop_task(&state);

    if let (Some(token), Some(chat)) = (&token, &chat) {
        if let Err(e) = engine::revoke_self(&chat.engine, token, &gateway_url()).await {
            logging::warn("chat", &format!("could not revoke this device on the server: {e}"));
        }
    }
    if let Some(token) = &token {
        auth::sign_out(&base, token).await;
    }
    let user_id = match &chat {
        Some(c) => Some(c.engine.lock().await.user_id),
        None => std::fs::read_to_string(account_file(&app)?)
            .ok()
            .and_then(|t| serde_json::from_str::<Account>(&t).ok())
            .map(|a| a.user_id),
    };
    drop(chat);
    secrets::remove(secrets::SESSION)?;
    if let Some(uid) = user_id {
        secrets::remove(&secrets::db_key_name(uid))?;
    }
    secrets::wipe_file();
    let dir = chat_dir(&app)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("Could not delete the chat folder: {e}"))?;
    }
    set_status(&app, ChatStatus::Off);
    logging::info("chat", "removed from this PC");
    Ok(())
}

/// The shell's people context: names, supporter status and mutes from the
/// friends list, and whether this account is a supporter.
#[tauri::command]
pub fn chat_set_context(state: State<'_, ChatState>, people: Vec<Person>, me_supporter: bool) {
    *state.people.lock().expect("people") = (people.clone(), me_supporter);
    if let Some(chat) = state.chat.lock().expect("chat state").clone() {
        let mut ctx = chat.context.lock().expect("context");
        ctx.people = people.into_iter().filter_map(|p| Some((p.id.parse().ok()?, p))).collect();
        ctx.me_supporter = me_supporter;
    }
}

#[tauri::command]
pub async fn chat_conversations(state: State<'_, ChatState>) -> Result<Vec<km_store_sqlcipher::ConversationRow>, String> {
    current(&state)?.conversations().await
}

/// Messages with someone, oldest first; `before` (ms) pages back.
#[tauri::command]
pub async fn chat_messages(
    state: State<'_, ChatState>,
    peer: String,
    before: Option<u64>,
) -> Result<Vec<km_store_sqlcipher::MessageRow>, String> {
    current(&state)?.messages_with(target(&peer)?, before).await
}

#[tauri::command]
pub async fn chat_send(
    state: State<'_, ChatState>,
    peer: String,
    text: String,
    reply_to: Option<String>,
) -> Result<km_store_sqlcipher::MessageRow, String> {
    let chat = current(&state)?;
    let reply = reply_to.as_deref().map(msg_id).transpose()?;
    chat.send_text(target(&peer)?, text, reply).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_send_gif(state: State<'_, ChatState>, peer: String, gif: GifBody) -> Result<km_store_sqlcipher::MessageRow, String> {
    let chat = current(&state)?;
    chat.send_gif(target(&peer)?, gif).await.map_err(|e| e.to_string())
}

/// Invite them to play a game (with the Steam lobby you are in, if known).
#[tauri::command]
pub async fn chat_send_invite(state: State<'_, ChatState>, peer: String, invite: InviteBody) -> Result<km_store_sqlcipher::MessageRow, String> {
    current(&state)?.send_invite(target(&peer)?, invite).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_edit(state: State<'_, ChatState>, msg: String, text: String) -> Result<km_store_sqlcipher::MessageRow, String> {
    let chat = current(&state)?;
    chat.edit(msg_id(&msg)?, text).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_delete(state: State<'_, ChatState>, msg: String) -> Result<(), String> {
    let chat = current(&state)?;
    chat.delete(msg_id(&msg)?).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_react(state: State<'_, ChatState>, msg: String, emoji: String, remove: bool) -> Result<(), String> {
    let chat = current(&state)?;
    chat.react(msg_id(&msg)?, emoji, remove).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_typing(state: State<'_, ChatState>, peer: String, active: bool) -> Result<(), String> {
    let chat = current(&state)?;
    chat.typing(target(&peer)?, active).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_mark_read(app: AppHandle, state: State<'_, ChatState>, peer: String) -> Result<(), String> {
    let chat = current(&state)?;
    let r = chat.mark_read(target(&peer)?).await.map_err(|e| e.to_string());
    refresh_badge(&app);
    r
}

/// Search this PC's messages. Nothing is searched anywhere else.
#[tauri::command]
pub async fn chat_search(state: State<'_, ChatState>, query: String) -> Result<Vec<km_store_sqlcipher::MessageRow>, String> {
    current(&state)?.search(&query).await
}

#[tauri::command]
pub async fn chat_settings_get(state: State<'_, ChatState>) -> Result<Settings, String> {
    Ok(current(&state)?.settings().await)
}

#[tauri::command]
pub async fn chat_settings_set(state: State<'_, ChatState>, settings: Settings) -> Result<(), String> {
    if !["full", "name", "none"].contains(&settings.notification_content.as_str()) {
        return Err("notificationContent must be full, name or none.".into());
    }
    current(&state)?.set_settings(&settings).await
}

/// Search GIFs through kryo.to (supporters only; kryo.to holds the GIF
/// provider's key and keeps your IP and searches from the provider).
#[tauri::command]
pub async fn chat_gif_search(app: AppHandle, query: String) -> Result<serde_json::Value, String> {
    let token = secrets::read(secrets::SESSION)?.ok_or("Turn chat on first.")?;
    auth::gif_search(&api_base(&app), &token, &query).await
}

/// Ask to message someone who is not a friend, by username.
#[tauri::command]
pub async fn chat_message_request(app: AppHandle, username: String) -> Result<serde_json::Value, String> {
    let token = secrets::read(secrets::SESSION)?.ok_or("Turn chat on first.")?;
    auth::message_request(&api_base(&app), &token, &username).await
}

/// Accept or decline a message request someone sent you.
#[tauri::command]
pub async fn chat_message_request_respond(app: AppHandle, user: String, accept: bool) -> Result<(), String> {
    let token = secrets::read(secrets::SESSION)?.ok_or("Turn chat on first.")?;
    auth::message_request_respond(&api_base(&app), &token, user_id(&user)?, accept).await
}

// ---- verification, backup, devices, identity --------------------------------

#[tauri::command]
pub async fn chat_verify_info(state: State<'_, ChatState>, peer: String) -> Result<identity::VerifyInfo, String> {
    current(&state)?.verify_info(user_id(&peer)?).await
}

#[tauri::command]
pub async fn chat_verify_mark(state: State<'_, ChatState>, peer: String, verified: bool) -> Result<(), String> {
    current(&state)?.verify_mark(user_id(&peer)?, verified).await
}

/// Accept someone's changed security key after the warning.
#[tauri::command]
pub async fn chat_identity_ack(state: State<'_, ChatState>, peer: String) -> Result<(), String> {
    current(&state)?.identity_ack(user_id(&peer)?).await
}

#[tauri::command]
pub async fn chat_backup_status(state: State<'_, ChatState>) -> Result<identity::BackupStatus, String> {
    current(&state)?.backup_status().await
}

/// Make a key backup; returns the recovery code (shown once).
#[tauri::command]
pub async fn chat_backup_create(state: State<'_, ChatState>) -> Result<String, String> {
    current(&state)?.backup_create().await
}

#[tauri::command]
pub async fn chat_backup_delete(state: State<'_, ChatState>) -> Result<(), String> {
    current(&state)?.backup_delete().await
}

#[tauri::command]
pub async fn chat_devices(state: State<'_, ChatState>) -> Result<Vec<identity::MyDevice>, String> {
    current(&state)?.my_devices().await
}

#[tauri::command]
pub async fn chat_device_revoke(state: State<'_, ChatState>, device: String) -> Result<(), String> {
    current(&state)?.revoke_device(user_id(&device)?).await
}

/// What the shell sends with a report.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportInput {
    /// The person reported.
    peer: String,
    /// Message ids, read from this PC's store.
    msgs: Vec<String>,
    reason: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    block: bool,
    /// "g:<id>" when reporting from a group; else the 1:1 conversation.
    #[serde(default)]
    conversation: Option<String>,
}

/// Report messages from a conversation to Kryoto staff. The messages are
/// read from this PC's store by id (the shell names them, it does not supply
/// their text), decrypted only here, and sent as the person chose.
#[tauri::command]
pub async fn chat_report(app: AppHandle, state: State<'_, ChatState>, report: ReportInput) -> Result<(), String> {
    let ReportInput { peer, msgs, reason, note, block, conversation } = report;
    let chat = current(&state)?;
    let peer_id = user_id(&peer)?;
    // In a group, the report is about one member: their messages and yours.
    let place = match conversation.as_deref() {
        Some(c) => target(c)?,
        None => Target::Dm(peer_id),
    };
    if msgs.is_empty() || msgs.len() > 30 {
        return Err("Pick between 1 and 30 messages.".into());
    }
    let wanted: std::collections::HashSet<String> = msgs.into_iter().collect();
    let picked: Vec<serde_json::Value> = chat
        .messages_with(place, None)
        .await?
        .into_iter()
        .filter(|m| wanted.contains(&m.msg_id) && !m.deleted && (m.outgoing || m.sender_user == peer))
        .map(|m| serde_json::json!({ "sentAt": m.sent_at, "mine": m.outgoing, "kind": m.kind, "text": m.body }))
        .collect();
    if picked.is_empty() {
        return Err("Those messages are not on this PC.".into());
    }
    let token = secrets::read(secrets::SESSION)?.ok_or("Turn chat on first.")?;
    let payload = serde_json::json!({ "userId": peer, "reason": reason, "note": note, "messages": picked, "block": block });
    auth::report(&api_base(&app), &token, &payload).await
}

// ---- voice calls -----------------------------------------------------------------

/// Send call signalling (the shell's WebRTC side does the media).
#[tauri::command]
pub async fn chat_call_signal(state: State<'_, ChatState>, peer: String, call_id: String, kind: String, payload: String) -> Result<(), String> {
    use km_proto::CallKind;
    let kind = match kind.as_str() {
        "offer" => CallKind::Offer,
        "answer" => CallKind::Answer,
        "ice" => CallKind::Ice,
        "hangup" => CallKind::Hangup,
        "decline" => CallKind::Decline,
        "busy" => CallKind::Busy,
        "restart" => CallKind::Restart,
        "restarted" => CallKind::Restarted,
        "muted" => CallKind::Muted,
        "unmuted" => CallKind::Unmuted,
        _ => return Err("Not a call signal.".into()),
    };
    let id = hex::decode(&call_id).ok().filter(|b| b.len() == 16).ok_or("Not a call id.")?;
    current(&state)?.send_call(user_id(&peer)?, id, kind, payload).await.map_err(|e| e.to_string())
}

/// Put a finished call in the conversation (the caller's side sends it).
#[tauri::command]
pub async fn chat_call_log(state: State<'_, ChatState>, peer: String, call_id: String, answered: bool, seconds: u32) -> Result<(), String> {
    let id = hex::decode(&call_id).ok().filter(|b| b.len() == 16).ok_or("Not a call id.")?;
    current(&state)?.send_call_log(user_id(&peer)?, id, answered, seconds).await.map(|_| ()).map_err(|e| e.to_string())
}

/// Someone is using Kryoto here: this account's other devices keep quiet.
#[tauri::command]
pub async fn chat_active(state: State<'_, ChatState>) -> Result<(), String> {
    current(&state)?.send_active().await.map_err(|e| e.to_string())
}

/// STUN/TURN servers for calls, from kryo.to (TURN credentials are short-lived).
#[tauri::command]
pub async fn chat_ice_servers(app: AppHandle) -> Result<serde_json::Value, String> {
    let token = secrets::read(secrets::SESSION)?.ok_or("Turn chat on first.")?;
    auth::api(&api_base(&app), &token, reqwest::Method::GET, "/api/chat/ice", None).await
}

// ---- the public room (not end-to-end encrypted) ---------------------------------

/// Messages in the public room (newest, or only those after `after`).
#[tauri::command]
pub async fn room_list(app: AppHandle, after: Option<String>) -> Result<serde_json::Value, String> {
    let token = secrets::read(secrets::SESSION)?.ok_or("Turn chat on first.")?;
    let path = match after.filter(|a| !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit())) {
        Some(a) => format!("/api/chat/room?after={a}"),
        None => "/api/chat/room".to_string(),
    };
    auth::api(&api_base(&app), &token, reqwest::Method::GET, &path, None).await
}

#[tauri::command]
pub async fn room_post(app: AppHandle, text: String) -> Result<serde_json::Value, String> {
    let token = secrets::read(secrets::SESSION)?.ok_or("Turn chat on first.")?;
    auth::api(&api_base(&app), &token, reqwest::Method::POST, "/api/chat/room", Some(serde_json::json!({ "text": text }))).await
}

#[tauri::command]
pub async fn room_delete(app: AppHandle, id: String) -> Result<(), String> {
    let token = secrets::read(secrets::SESSION)?.ok_or("Turn chat on first.")?;
    let id = user_id(&id)?;
    auth::api(&api_base(&app), &token, reqwest::Method::DELETE, &format!("/api/chat/room/{id}"), None).await.map(|_| ())
}

// ---- files ---------------------------------------------------------------------

/// Pick a file and send it, end-to-end encrypted. None when cancelled.
#[tauri::command]
pub async fn chat_send_file(window: tauri::Window, state: State<'_, ChatState>, peer: String) -> Result<Option<km_store_sqlcipher::MessageRow>, String> {
    let chat = current(&state)?;
    let to = target(&peer)?;
    let Some(path) = file_dialog(&window, None).await? else { return Ok(None) };
    let meta = std::fs::metadata(&path).map_err(|e| format!("Could not read the file: {e}"))?;
    if meta.len() > km_core::MAX_ATTACHMENT as u64 {
        return Err("Files can be up to 25 MB.".into());
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("Could not read the file: {e}"))?;
    let row = chat.send_file(to, &gateway_http(), name, bytes).await.map_err(|e| e.to_string())?;
    Ok(Some(chat.shown(row)))
}

/// Download a file from a message and save it where the person picks.
#[tauri::command]
pub async fn chat_file_save(window: tauri::Window, state: State<'_, ChatState>, msg: String) -> Result<Option<String>, String> {
    let chat = current(&state)?;
    let (file, bytes) = chat.fetch_file(&msg_id(&msg)?, &gateway_http()).await?;
    let Some(path) = file_dialog(&window, Some(file.name.clone())).await? else { return Ok(None) };
    tokio::fs::write(&path, bytes.as_slice()).await.map_err(|e| format!("Could not save it: {e}"))?;
    Ok(Some(path.display().to_string()))
}

/// An image from a message, decrypted, as a data: URL to show inline.
#[tauri::command]
pub async fn chat_file_preview(state: State<'_, ChatState>, msg: String) -> Result<String, String> {
    use base64::Engine as _;
    let chat = current(&state)?;
    let (file, bytes) = chat.fetch_file(&msg_id(&msg)?, &gateway_http()).await?;
    let mime = messaging::mime_of(&file.name);
    if !mime.starts_with("image/") || bytes.len() > 10 * 1024 * 1024 {
        return Err("No preview for this file.".into());
    }
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes.as_slice())))
}

// ---- groups --------------------------------------------------------------------

/// Names for people in your groups who are not your friends.
#[tauri::command]
pub async fn chat_people(app: AppHandle, ids: Vec<String>) -> Result<serde_json::Value, String> {
    let ids = ids.iter().map(|i| user_id(i)).collect::<Result<Vec<_>, _>>()?;
    let token = secrets::read(secrets::SESSION)?.ok_or("Turn chat on first.")?;
    auth::people(&api_base(&app), &token, &ids).await
}

#[tauri::command]
pub async fn chat_groups(state: State<'_, ChatState>) -> Result<Vec<GroupView>, String> {
    current(&state)?.groups().await
}

/// Start a group with these friends; you are its owner.
#[tauri::command]
pub async fn chat_group_create(state: State<'_, ChatState>, name: String, members: Vec<String>) -> Result<GroupView, String> {
    let ids = members.iter().map(|m| user_id(m)).collect::<Result<Vec<_>, _>>()?;
    current(&state)?.group_create(&name, ids).await
}

#[tauri::command]
pub async fn chat_group_rename(state: State<'_, ChatState>, group: String, name: String) -> Result<(), String> {
    current(&state)?.group_rename(user_id(&group)?, &name).await
}

#[tauri::command]
pub async fn chat_group_add(state: State<'_, ChatState>, group: String, members: Vec<String>) -> Result<GroupView, String> {
    let ids = members.iter().map(|m| user_id(m)).collect::<Result<Vec<_>, _>>()?;
    current(&state)?.group_add(user_id(&group)?, ids).await
}

/// Remove someone from a group (owners and admins), or `user` = you: leave.
#[tauri::command]
pub async fn chat_group_remove(state: State<'_, ChatState>, group: String, user: String) -> Result<(), String> {
    current(&state)?.group_remove(user_id(&group)?, user_id(&user)?).await
}

/// Save every conversation on this PC to a JSON file the person picks. The
/// file is plain text (not encrypted): the shell says so before calling.
/// Returns where it was saved, or None if the save dialog was cancelled.
#[tauri::command]
pub async fn chat_export_history(window: tauri::Window, state: State<'_, ChatState>) -> Result<Option<String>, String> {
    let chat = current(&state)?;
    let me = chat.engine.lock().await.user_id;
    let names: std::collections::HashMap<String, String> =
        state.people.lock().expect("people").0.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
    let mut conversations = Vec::new();
    for conv in chat.conversations().await? {
        let Some(peer) = conv.peer_user_id.as_deref().and_then(|p| p.parse::<u64>().ok()) else { continue };
        // Page back to the beginning.
        let mut all = Vec::new();
        let mut before = None;
        loop {
            let page = chat.messages_with(Target::Dm(peer), before).await?;
            let Some(oldest) = page.first().map(|m| m.sent_at) else { break };
            let full = page.len() >= 60;
            all.splice(0..0, page);
            if !full {
                break;
            }
            before = Some(oldest);
        }
        let messages: Vec<serde_json::Value> = all
            .into_iter()
            .filter(|m| !m.deleted)
            .map(|m| {
                serde_json::json!({
                    "sentAt": m.sent_at,
                    "from": if m.outgoing { "me".to_string() } else { names.get(&m.sender_user).cloned().unwrap_or_else(|| m.sender_user.clone()) },
                    "kind": m.kind,
                    "text": m.body,
                    "edited": m.edited_at.is_some(),
                })
            })
            .collect();
        conversations.push(serde_json::json!({
            "with": names.get(&peer.to_string()).cloned().unwrap_or_else(|| peer.to_string()),
            "userId": peer.to_string(),
            "messages": messages,
        }));
    }
    let doc = serde_json::json!({ "account": me.to_string(), "exportedAt": chrono_now(), "conversations": conversations });
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    let parent = window.clone();
    window
        .run_on_main_thread(move || {
            let dialog = rfd::AsyncFileDialog::new()
                .set_parent(&parent)
                .set_title("Save your chat history")
                .set_file_name("kryoto-chat-history.json")
                .add_filter("JSON", &["json"]);
            let picked = dialog.save_file();
            std::thread::spawn(move || {
                let _ = tx.send(tauri::async_runtime::block_on(picked).map(|h| h.path().to_path_buf()));
            });
        })
        .map_err(|e| e.to_string())?;
    let Some(path) = rx.await.map_err(|_| "The save dialog closed unexpectedly.".to_string())? else { return Ok(None) };
    std::fs::write(&path, text).map_err(|e| format!("Could not save it: {e}"))?;
    logging::info("chat", "exported the chat history to a file");
    Ok(Some(path.display().to_string()))
}

/// Now as an ISO 8601 UTC string, without pulling in a date crate.
fn chrono_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Stop the live connection, run `work` on the engine with a one-off
/// connection, then connect again (with whatever identity `work` left).
async fn with_live_stopped<F, Fut>(app: &AppHandle, work: F) -> Result<(), String>
where
    F: FnOnce(Arc<Chat>, Zeroizing<String>) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let state = app.state::<ChatState>();
    let _guard = state.busy.lock().await;
    let chat = current(&state)?;
    let token = secrets::read(secrets::SESSION)?.ok_or("Turn chat on first.")?;
    let task = state.task.lock().expect("chat state").take();
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
    let user_id = chat.engine.lock().await.user_id;
    let result = work(chat, token.clone()).await;
    // Connect again either way: after a failure the account is where it was.
    start(app, token, user_id)?;
    result
}

/// This account has chat on another device: take over its identity with the
/// recovery code of its key backup.
#[tauri::command]
pub async fn chat_restore(app: AppHandle, code: String) -> Result<(), String> {
    with_live_stopped(&app, |chat, token| async move { identity::restore(&chat.engine, &token, &gateway_url(), &code).await }).await
}

/// Start a new chat identity for the whole account (contacts are warned; the
/// account's other devices must be set up again).
#[tauri::command]
pub async fn chat_reset_identity(app: AppHandle) -> Result<(), String> {
    with_live_stopped(&app, |chat, token| async move { identity::reset_identity(&chat.engine, &token, &gateway_url()).await }).await
}

fn whoami() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "Kryoto Desktop".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_dates_are_iso_utc() {
        let now = chrono_now();
        assert_eq!(now.len(), 20, "{now}");
        assert!(now.starts_with("20") && now.ends_with('Z') && now.as_bytes()[10] == b'T', "{now}");
    }

    #[test]
    fn status_reaches_the_shell_in_camel_case() {
        let s = serde_json::to_string(&ChatStatus::Online { user_id: "1".into(), device_id: "2".into() }).unwrap();
        assert_eq!(s, r#"{"state":"online","userId":"1","deviceId":"2"}"#);
        let s = serde_json::to_string(&ChatStatus::Offline { retry_in_secs: 4 }).unwrap();
        assert_eq!(s, r#"{"state":"offline","retryInSecs":4}"#);
        assert_eq!(serde_json::to_string(&ChatStatus::AnonymousMode).unwrap(), r#"{"state":"anonymousMode"}"#);
    }

    #[test]
    fn ids_from_the_shell_are_checked() {
        assert!(user_id("123").is_ok());
        assert!(user_id("12a").is_err());
        assert!(msg_id(&"ab".repeat(16)).is_ok());
        assert!(msg_id("abcd").is_err());
        assert!(msg_id(&"zz".repeat(16)).is_err());
    }
}
