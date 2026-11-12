//! What the chat engine tells the rest of the app, and what it is told.
//! No Tauri in here: the engine runs the same under the test harness.

use std::collections::HashMap;
use std::sync::Arc;

use km_store_sqlcipher::MessageRow;
use serde::{Deserialize, Serialize};

/// What chat is doing, for the shell to show.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChatStatus {
    /// Never turned on, or removed from this PC.
    Off,
    Connecting,
    Online { user_id: String, device_id: String },
    /// Lost the connection; trying again on its own.
    Offline { retry_in_secs: u64 },
    /// The sign-in expired: turn chat on again (one click while signed in to the Store).
    SignInNeeded,
    /// "You need to turn off anonymous mode to use this feature."
    AnonymousMode,
    /// Not rolled out to this account yet.
    Disabled,
    /// The account already has chat on another device; link this one from there.
    NeedsLink,
    /// This PC was removed from the account's chat devices.
    DeviceRemoved,
    /// No OS secure store here: chat's keys are sealed under a passphrase,
    /// which has to be typed (or, `creating`, chosen) first.
    Locked { creating: bool },
    Unavailable { reason: String },
}

/// Everything the engine reports.
#[derive(Clone, Debug)]
pub enum ChatEvent {
    Status(ChatStatus),
    /// A new message in a conversation (incoming, or sent from another of your devices).
    Message(MessageRow),
    /// A message changed: status, edit, delete, reaction.
    Updated(MessageRow),
    Typing { conversation_id: String, user_id: String, active: bool },
    /// Someone's security key changed: the conversation shows a warning and
    /// nothing is sent until the person acknowledges it.
    IdentityChanged { user_id: String },
    /// Another of this account's devices read a conversation.
    ReadElsewhere { conversation_id: String },
    /// A group's members or name changed.
    GroupChanged { group_id: String },
    /// Voice call signalling from someone (offer, answer, ice, hangup, decline, busy).
    Call { user_id: String, call_id: String, kind: String, payload: String },
    /// Past messages between this account's devices (KB-0254): where it stands.
    History(super::history::HistoryView),
    /// History from another device was stored in these conversations.
    HistoryMerged { conversations: Vec<String> },
    /// A new device of this account asks for this one's history.
    HistoryAsk { device_name: String },
    /// Something worth a desktop notification (already filtered for mutes).
    Notify { conversation_id: String, sender_user: String, preview: Option<String>, group_name: Option<String> },
}

pub type Emit = Arc<dyn Fn(ChatEvent) + Send + Sync>;

/// What the shell knows about the people chat involves, from the friends
/// list: names for notifications, supporter status for the supporter-only
/// rules, who is muted.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub supporter: bool,
    #[serde(default)]
    pub muted: bool,
    /// They sent a message request this account has not accepted yet: no
    /// receipts or typing go back until it is (the gateway would refuse them).
    #[serde(default)]
    pub pending: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Context {
    pub people: HashMap<u64, Person>,
    /// This account is a supporter (longer messages, GIFs).
    pub me_supporter: bool,
}

/// The person's chat settings, kept in the encrypted store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Tell people when you have read their messages (and see theirs).
    pub read_receipts: bool,
    /// Show "typing..." both ways.
    pub typing: bool,
    /// What a desktop notification shows: "full" (name and message),
    /// "name" (only who it is from), "none" (only that something arrived).
    pub notification_content: String,
    /// Load GIFs straight away (the GIF provider then sees your IP), or only
    /// when clicked.
    pub gifs_auto: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { read_receipts: true, typing: true, notification_content: "name".into(), gifs_auto: true }
    }
}
