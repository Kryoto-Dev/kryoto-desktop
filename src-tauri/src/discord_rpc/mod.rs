//! "Playing Hades" in Discord, through the Discord client on this PC.
//!
//! Native, over Discord's local socket (`ipc.rs`), with no page script. One
//! background task keeps the connection, and only while there is something to
//! show. Everything else just says what should be showing and returns, so
//! starting a game never waits for Discord: if Discord is closed the task
//! tries again quietly, and picks up when it opens.
//!
//! What is shown comes from `activity.rs`. Games another launcher keeps are
//! left out: Kryoto cannot tell when they end, and that launcher tells Discord
//! itself.

mod activity;
mod ipc;

use crate::library::LibraryGame;
use activity::Who;
use ipc::{frame, Frames, Pipe, OP_CLOSE, OP_FRAME, OP_HANDSHAKE};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

/// The Kryoto application. Public: it is in every Discord client that shows it.
const CLIENT_ID: &str = "1519811642521817258";
/// How long between looks for a closed Discord.
const RETRY: Duration = Duration::from_secs(15);
const ANSWER: Duration = Duration::from_secs(5);
/// Discord takes five updates in twenty seconds.
const GAP: Duration = Duration::from_secs(4);

#[derive(Default)]
struct State {
    /// What is running, oldest first; the newest shows.
    playing: Vec<(String, u64)>,
    username: Option<String>,
    anonymous: bool,
    opened: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static WANTED: OnceLock<watch::Sender<Option<Value>>> = OnceLock::new();
static LINK: Mutex<Option<String>> = Mutex::new(None);
/// Set while Discord is connected and answered: who it is signed in as.
static CONNECTED: Mutex<bool> = Mutex::new(false);

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(s.get_or_insert_with(State::default))
}

pub fn start<R: Runtime>(app: AppHandle<R>) {
    let (tx, rx) = watch::channel(None);
    if WANTED.set(tx).is_err() {
        return;
    }
    state(|s| s.opened = now());
    let told = app.clone();
    tauri::async_runtime::spawn(run(
        move || {
            let _ = told.emit("discord-status", ());
        },
        rx,
    ));
    refresh(&app);
    // A button taking its turn. Nothing goes to Discord unless the turn changed.
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(turn_seconds().min(30))).await;
            if state(|s| !s.playing.is_empty()) {
                refresh(&app);
            }
        }
    });
}

fn turn_seconds() -> u64 {
    // Debug builds only: a test does not wait five minutes for a turn.
    #[cfg(debug_assertions)]
    if let Some(n) = std::env::var("KRYOTO_DEV_DISCORD_TURN").ok().and_then(|v| v.parse().ok()) {
        return n;
    }
    activity::TURN_SECONDS
}

/// A game Kryoto started. Returns at once.
pub fn started<R: Runtime>(app: &AppHandle<R>, id: &str, at: u64) {
    state(|s| {
        s.playing.retain(|(g, _)| g != id);
        s.playing.push((id.to_string(), at));
    });
    refresh(app);
}

pub fn ended<R: Runtime>(app: &AppHandle<R>, id: &str) {
    state(|s| s.playing.retain(|(g, _)| g != id));
    refresh(app);
}

pub fn set_account<R: Runtime>(app: &AppHandle<R>, username: Option<&str>, anonymous: bool) {
    let same = state(|s| {
        let same = s.username.as_deref() == username && s.anonymous == anonymous;
        s.username = username.map(String::from);
        s.anonymous = anonymous;
        same
    });
    if !same {
        refresh(app);
    }
}

/// Work out what should show, off the caller's thread.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(tx) = WANTED.get() else { return };
        let wanted = desired(&app);
        tx.send_if_modified(|current| {
            let changed = *current != wanted;
            if changed {
                current.clone_from(&wanted);
            }
            changed
        });
        let _ = app.emit("discord-status", ());
    });
}

fn who() -> Who {
    state(|s| Who::new(s.username.as_deref(), s.anonymous))
}

fn desired<R: Runtime>(app: &AppHandle<R>) -> Option<Value> {
    let settings = crate::settings::load(app).discord;
    if !settings.on {
        return None;
    }
    let playing = state(|s| s.playing.clone());
    let opened = state(|s| s.opened);
    let games = crate::library::load(app).unwrap_or_default();
    let shown = playing.iter().rev().find_map(|(id, at)| games.iter().find(|g| &g.id == id && shows(g)).map(|g| (g, *at)));
    match shown {
        Some((g, at)) => Some(activity::for_game(&settings, g, at, activity::turn(at, now(), turn_seconds()), &who())),
        None if settings.browsing => Some(activity::browsing(&settings, opened, &who())),
        None => None,
    }
}

fn shows(g: &LibraryGame) -> bool {
    g.discord_show != Some(false) && g.launcher.is_none() && (!g.hidden || g.discord_show == Some(true))
}

/// How one library game stands with Discord.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameStanding {
    pub id: String,
    pub title: String,
    /// Why it is not shown as itself: `off`, `game`, `hidden` or `adult`.
    pub why: Option<&'static str>,
    pub playing: bool,
    /// It is on Discord this moment.
    pub now: bool,
    /// The player chose something for this game that differs from Settings.
    pub set: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub on: bool,
    /// Discord is open on this PC.
    pub open: bool,
    /// Games another launcher keeps are not here: that launcher tells Discord.
    pub games: Vec<GameStanding>,
}

/// Pure, for the tests. `live` is the game Discord is showing.
fn standings(s: &crate::settings::DiscordPresence, games: &[LibraryGame], playing: &[(String, u64)], connected: bool) -> Vec<GameStanding> {
    let live = playing.iter().rev().find_map(|(id, _)| games.iter().find(|g| &g.id == id && shows(g))).filter(|_| connected && s.on).map(|g| g.id.as_str());
    games
        .iter()
        .filter(|g| g.launcher.is_none())
        .map(|g| GameStanding {
            id: g.id.clone(),
            title: g.title.clone(),
            why: activity::hidden_because(s, g),
            playing: playing.iter().any(|(id, _)| id == &g.id),
            now: live == Some(g.id.as_str()),
            set: g.discord_show.is_some(),
        })
        .collect()
}

/// Whether Discord is open, when nothing is connected to tell. A look at its
/// socket, at most this often.
static LOOKED: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

async fn discord_open() -> bool {
    if *CONNECTED.lock().unwrap_or_else(|e| e.into_inner()) {
        return true;
    }
    if let Some((at, open)) = *LOOKED.lock().unwrap_or_else(|e| e.into_inner()) {
        if at.elapsed() < Duration::from_secs(10) {
            return open;
        }
    }
    let open = ipc::open().await.is_some();
    *LOOKED.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), open));
    open
}

/// The one answer the game page, Properties and Settings all show.
#[tauri::command]
pub async fn discord_games_status(app: AppHandle) -> Overview {
    let s = crate::settings::load(&app).discord;
    let games = crate::library::load(&app).unwrap_or_default();
    let playing = state(|st| st.playing.clone());
    let connected = *CONNECTED.lock().unwrap_or_else(|e| e.into_inner());
    let open = discord_open().await;
    Overview { on: s.on, open, games: standings(&s, &games, &playing, connected) }
}

/// The player flipped a game, wherever they did it.
#[tauri::command(async)]
pub fn discord_game_set(app: AppHandle, id: String, show: bool) -> Result<(), String> {
    let s = crate::settings::load(&app).discord;
    let game = crate::library::update(&app, |games| {
        let g = games.iter_mut().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
        g.discord_show = activity::choice(&s, g, show);
        Ok(g.clone())
    })?;
    crate::library::write_marker(&game);
    refresh(&app);
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Discord answered, and is showing what Kryoto sent or is ready to.
    pub connected: bool,
    /// What the two lines and picture would be, on the game last played.
    pub preview: Option<Value>,
}

/// For Settings: how it stands, and what other people would see.
#[tauri::command(async)]
pub fn discord_presence_status(app: AppHandle) -> Status {
    let s = crate::settings::load(&app).discord;
    let games = crate::library::load(&app).unwrap_or_default();
    // Never an adult game: this draws its art in Settings, unblurred.
    let sample = games.iter().filter(|g| shows(g) && !g.nsfw).max_by_key(|g| g.last_played.unwrap_or(0));
    let preview = sample.map(|g| activity::for_game(&s, g, now(), 0, &who())).or_else(|| s.browsing.then(|| activity::browsing(&s, now(), &who())));
    Status { connected: *CONNECTED.lock().unwrap_or_else(|e| e.into_inner()), preview }
}

fn set_connected(on: bool) {
    *CONNECTED.lock().unwrap_or_else(|e| e.into_inner()) = on;
}

fn note_problem(what: String) {
    let mut last = LINK.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_deref() != Some(what.as_str()) {
        crate::logging::warn("discord", &what);
        *last = Some(what);
    }
}

async fn run(tell: impl Fn() + Send + 'static, mut rx: watch::Receiver<Option<Value>>) {
    loop {
        if rx.borrow_and_update().is_none() {
            if rx.changed().await.is_err() {
                return;
            }
            continue;
        }
        match connect().await {
            Some((pipe, frames)) => {
                set_connected(true);
                tell();
                crate::logging::info("discord", "connected");
                serve(pipe, frames, &mut rx).await;
                set_connected(false);
                tell();
            }
            None => {
                tokio::select! {
                    _ = rx.changed() => {}
                    _ = tokio::time::sleep(RETRY) => {}
                }
            }
        }
    }
}

async fn send(pipe: &mut Pipe, op: u32, body: &Value) -> bool {
    matches!(tokio::time::timeout(ANSWER, pipe.write_all(&frame(op, body))).await, Ok(Ok(())))
}

/// The socket, once Discord has said READY.
async fn connect() -> Option<(Pipe, Frames)> {
    let mut pipe = ipc::open().await?;
    if !send(&mut pipe, OP_HANDSHAKE, &json!({ "v": 1, "client_id": CLIENT_ID })).await {
        return None;
    }
    let mut frames = Frames::default();
    let mut bytes = [0u8; 4096];
    let deadline = Instant::now() + ANSWER;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let n = tokio::time::timeout(left, pipe.read(&mut bytes)).await.ok()?.ok()?;
        if n == 0 {
            return None;
        }
        frames.push(&bytes[..n]);
        while let Some((op, body)) = frames.next().ok()? {
            if op == OP_CLOSE {
                note_problem(format!("Discord refused the connection: {body}"));
                return None;
            }
            if body["evt"] == "READY" {
                return Some((pipe, frames));
            }
        }
    }
}

fn payload(activity: Option<&Value>, extended: bool, nonce: u64) -> Value {
    let mut activity = activity.cloned().unwrap_or(Value::Null);
    if !extended {
        if let Some(a) = activity.as_object_mut() {
            a.remove("status_display_type");
        }
    }
    json!({ "cmd": "SET_ACTIVITY", "args": { "pid": std::process::id(), "activity": activity }, "nonce": nonce.to_string() })
}

/// Keep Discord up to date until it goes, or there is nothing left to show.
async fn serve(mut pipe: Pipe, mut frames: Frames, rx: &mut watch::Receiver<Option<Value>>) {
    let mut bytes = [0u8; 4096];
    let mut nonce = 0u64;
    // An older client that does not know a field refuses the whole update.
    let mut extended = true;
    let mut sent_at: Option<Instant> = None;
    loop {
        if let Some(wait) = sent_at.map(|t| GAP.saturating_sub(t.elapsed())) {
            tokio::time::sleep(wait).await;
        }
        let wanted = rx.borrow_and_update().clone();
        nonce += 1;
        if !send(&mut pipe, OP_FRAME, &payload(wanted.as_ref(), extended, nonce)).await {
            return;
        }
        sent_at = Some(Instant::now());
        // Closing clears it too; the update above just says so at once.
        if wanted.is_none() {
            return;
        }
        loop {
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    break;
                }
                read = pipe.read(&mut bytes) => {
                    let n = match read {
                        Ok(n) if n > 0 => n,
                        _ => return,
                    };
                    frames.push(&bytes[..n]);
                    let mut again = false;
                    loop {
                        match frames.next() {
                            Err(()) => return,
                            Ok(None) => break,
                            Ok(Some((OP_CLOSE, _))) => return,
                            Ok(Some((_, body))) => {
                                if body["evt"] == "ERROR" {
                                    if extended {
                                        extended = false;
                                        again = true;
                                    } else {
                                        note_problem(format!("Discord refused an update: {}", body["data"]));
                                    }
                                }
                            }
                        }
                    }
                    if again {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_another_launcher_keeps_or_the_player_hid_is_not_shown() {
        let g = LibraryGame::default();
        assert!(shows(&g));
        assert!(!shows(&LibraryGame { discord_show: Some(false), ..g.clone() }));
        // A hidden game stays off Discord, unless the player said to show it.
        assert!(!shows(&LibraryGame { hidden: true, ..g.clone() }));
        assert!(shows(&LibraryGame { hidden: true, discord_show: Some(true), ..g.clone() }));
        assert!(!shows(&LibraryGame { launcher: Some("steam".into()), ..g }));
    }

    #[test]
    fn only_the_game_on_discord_reads_as_on_it_now() {
        let s = crate::settings::DiscordPresence::default();
        let game = |id: &str| LibraryGame { id: id.into(), title: id.into(), ..Default::default() };
        let games = [game("a"), game("b"), LibraryGame { discord_show: Some(false), ..game("c") }, LibraryGame { launcher: Some("steam".into()), ..game("d") }];
        let playing = [("a".to_string(), 1), ("b".to_string(), 2)];
        let list = standings(&s, &games, &playing, true);
        assert_eq!(list.len(), 3, "another launcher's game is not listed");
        assert_eq!((list[0].playing, list[0].now, list[1].now, list[2].now), (true, false, true, false));
        assert_eq!((list[2].why, list[2].set), (Some("game"), true));
        // Running, but Discord is closed.
        assert!(!standings(&s, &games, &playing, false).iter().any(|g| g.now));
        // A hidden game running does not take the place of the one on show.
        let playing = [("a".to_string(), 1), ("c".to_string(), 2)];
        assert!(standings(&s, &games, &playing, true)[0].now);
    }

    /// The whole conversation, over a real pipe.
    #[cfg(windows)]
    #[test]
    fn says_hello_shows_the_game_and_clears_it_over_a_pipe() {
        use tokio::net::windows::named_pipe::ServerOptions;
        async fn next(server: &mut tokio::net::windows::named_pipe::NamedPipeServer, frames: &mut Frames) -> (u32, Value) {
            let mut bytes = [0u8; 4096];
            loop {
                if let Some(f) = frames.next().unwrap() {
                    return f;
                }
                let n = server.read(&mut bytes).await.unwrap();
                assert!(n > 0, "the client hung up");
                frames.push(&bytes[..n]);
            }
        }
        tauri::async_runtime::block_on(async {
            std::env::set_var("KRYOTO_DEV_DISCORD_PIPE", "kryoto-unit-ipc");
            let mut server = ServerOptions::new().first_pipe_instance(true).create(r"\\.\pipe\kryoto-unit-ipc-0").unwrap();
            let (tx, rx) = watch::channel(Some(json!({ "details": "Hades" })));
            let task = tauri::async_runtime::spawn(run(|| {}, rx));
            server.connect().await.unwrap();
            let mut frames = Frames::default();
            let (op, hello) = next(&mut server, &mut frames).await;
            assert_eq!((op, hello["client_id"].as_str()), (OP_HANDSHAKE, Some(CLIENT_ID)));
            server.write_all(&frame(OP_FRAME, &json!({ "evt": "READY" }))).await.unwrap();
            let (_, set) = next(&mut server, &mut frames).await;
            assert_eq!((set["cmd"].as_str(), set["args"]["activity"]["details"].as_str()), (Some("SET_ACTIVITY"), Some("Hades")));
            assert_eq!(set["args"]["pid"], std::process::id());
            tx.send(None).unwrap();
            let (_, clear) = next(&mut server, &mut frames).await;
            assert_eq!(clear["args"]["activity"], Value::Null);
            task.abort();
        });
    }

    #[test]
    fn an_older_client_gets_the_update_without_the_newer_field() {
        let a = json!({ "details": "Hades", "status_display_type": 2 });
        assert_eq!(payload(Some(&a), true, 1)["args"]["activity"]["status_display_type"], 2);
        assert!(payload(Some(&a), false, 2)["args"]["activity"].get("status_display_type").is_none());
        assert_eq!(payload(None, true, 3)["args"]["activity"], Value::Null);
    }
}
