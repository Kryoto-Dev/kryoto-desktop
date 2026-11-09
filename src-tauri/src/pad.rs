//! Controllers: reading them, telling them apart, setting them up and
//! remapping them for games. Behind kryo.to's `controller` feature flag:
//! nothing here runs until the shell, seeing the flag on the account, calls
//! `pad_start`.
//!
//! - Reading is SDL2 (statically linked), the same input library Steam and
//!   most PC games use: XInput, RawInput, HIDAPI and DirectInput on Windows,
//!   evdev on Linux, with the SDL_GameControllerDB of known pads built in. It
//!   reads in the background too; Kryoto only acts on presses while its own
//!   window is in front.
//! - A pad SDL does not know (or one it maps wrong) is set up once with the
//!   step-by-step setup in Settings > Controller: it watches the raw buttons,
//!   axes and hats (`pad_capture`) and saves an SDL mapping for that model
//!   (`pad_mapping_set`), which Kryoto and SDL games then use.
//! - Telling pads apart, naming their buttons and remapping are kryoto-padmap,
//!   our open-source resolver (../../kryoto-padmap).
//! - A pad never seen before gets a system notification: "Kryoto detected a
//!   controller ... Configure", which opens Settings > Controller on it.
//! - Haptics: a light tick as the selection moves, a firmer one on select,
//!   a bump at the edge (`pad_haptic`).
//! - Games get the setup and the remaps:
//!   - Linux: as `SDL_GAMECONTROLLERCONFIG`, which SDL games read, and so do
//!     Wine and Proton, whose controller driver is built on SDL ([`game_env`]).
//!   - Windows: through a virtual Xbox controller (ViGEmBus) while a game
//!     runs, so every game that reads Xbox controllers gets the pad as set up
//!     and remapped, rumble included ([`virt`]).
//! - What a pad does in each game (off, a game's own remap, keyboard and
//!   mouse) is `pad_games.rs`, which this thread drives while a game runs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use padmap::{Control, Family, Model, Remap};
use sdl2::controller::{Axis as SAxis, Button as SButton, GameController};
use sdl2::event::Event;
use sdl2::joystick::{HatState, Joystick};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// What is plugged in, and the reader thread's mailbox.
#[derive(Default)]
pub struct Pads {
    list: Mutex<Vec<PadInfo>>,
    tx: Mutex<Option<mpsc::Sender<Req>>>,
    /// Whether the virtual-controller driver answered, when last asked.
    #[cfg_attr(not(windows), allow(dead_code))]
    driver: Mutex<Option<bool>>,
}

enum Req {
    Start,
    #[cfg(windows)]
    Refresh,
    Stop,
    Rumble(u32),
    Haptic(u32, Haptic),
    /// Report the raw inputs of this pad (the setup), or stop.
    Capture(Option<u32>),
    /// The saved setup of this model changed: apply it.
    Mapping(String),
    /// A game asked the virtual controller to rumble.
    #[cfg_attr(not(windows), allow(dead_code))]
    VirtualRumble(u32, u8, u8),
}

/// The feel of moving around Kryoto by pad.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Haptic {
    /// The selection moved.
    Tick,
    /// Something was pressed, or the page changed.
    Select,
    /// Nothing further that way.
    Edge,
}

/// One connected pad, as the shell sees it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PadInfo {
    /// SDL's instance id: this connection's, for rumble and input events.
    id: u32,
    /// SDL's GUID: the same for every pad of one model, and the key its
    /// settings are kept under.
    guid: String,
    /// The name to show: the pad's own, or its maker's when the system only
    /// gave a placeholder ("HID-compliant game controller").
    name: String,
    /// What the system calls it.
    system_name: String,
    vendor: Option<u16>,
    product: Option<u16>,
    /// What padmap made of it.
    model: Model,
    /// SDL knows its buttons (its database, or a setup saved here).
    mapped: bool,
    /// Its buttons come from a setup saved here.
    custom: bool,
    /// An Xbox pad: games read it natively, so no virtual controller.
    xinput: bool,
    rumble: bool,
    battery: Option<u8>,
    charging: bool,
    /// What the setup can read: raw buttons, axes and hats.
    buttons: u32,
    axes: u32,
    hats: u32,
}

/// Settings > Controller, in `controllers.json`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PadConfig {
    /// The account has the feature (set by `pad_start` / `pad_stop`), so a
    /// game started while it is off gets nothing from here.
    pub enabled: bool,
    /// Move around Kryoto with a pad.
    pub navigate: bool,
    /// Say so when a pad is plugged in for the first time.
    pub notify: bool,
    /// Hand the setup and remaps to games.
    pub games: bool,
    /// Feel the selection move (light rumble).
    pub haptics: bool,
    /// Windows: play as a virtual Xbox controller while a game runs (needs
    /// the ViGEmBus driver; pads that are Xbox pads already are left alone).
    pub virtual_pad: bool,
    /// By GUID.
    pub pads: BTreeMap<String, PadPrefs>,
}

impl Default for PadConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            navigate: true,
            notify: false,
            games: true,
            haptics: true,
            virtual_pad: true,
            pads: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PadPrefs {
    /// The name it had when last seen, for listing it while unplugged.
    pub name: String,
    /// Drawn and named as this family instead of the one it was told to be.
    pub family: Option<Family>,
    pub remap: Remap,
    /// The setup saved here: an SDL mapping for this model, used by Kryoto,
    /// by SDL games (Linux) and by the virtual controller (Windows).
    pub mapping: Option<String>,
    /// SDL's own mapping for it (its database), as last seen: the base a
    /// remap rewrites when there is no setup, and what Reset goes back to.
    pub base: Option<String>,
    /// The SDL mapping games get: the setup or the base, under `remap`.
    pub sdl: Option<String>,
}

fn config_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("controllers.json"))
}

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

pub fn load_config<R: Runtime>(app: &AppHandle<R>) -> PadConfig {
    let mut cfg: PadConfig = config_path(app)
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    // A hand-edited file could hold a remap that is not a permutation.
    for p in cfg.pads.values_mut() {
        p.remap = Remap::from_pairs(p.remap.changes().collect::<Vec<_>>());
    }
    cfg
}

fn save_config<R: Runtime>(app: &AppHandle<R>, cfg: &PadConfig) -> Result<(), String> {
    let path = config_path(app).ok_or("No app data folder.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Change the config under the lock, save it and return it.
fn update_config<R: Runtime>(app: &AppHandle<R>, f: impl FnOnce(&mut PadConfig)) -> Result<PadConfig, String> {
    let _guard = CONFIG_LOCK.lock().map_err(|_| "Controller settings are busy.")?;
    let mut cfg = load_config(app);
    f(&mut cfg);
    save_config(app, &cfg)?;
    Ok(cfg)
}

/// The mapping games get for a pad: its setup, or SDL's own under a remap.
fn sdl_line(prefs: &PadPrefs) -> Option<String> {
    let source = prefs.mapping.as_deref().or(prefs.base.as_deref())?;
    if prefs.mapping.is_none() && prefs.remap.is_identity() {
        return None; // SDL already knows it, unchanged
    }
    let parsed = padmap::SdlMapping::parse(source).ok()?;
    Some(parsed.remapped(&prefs.remap).to_line())
}

/// What Play adds to a game's environment on Linux (a setup or a remap saved):
///
/// - `SDL_GAMECONTROLLERCONFIG`: the mappings, which SDL reads before its own
///   database.
/// - `SDL_JOYSTICK_HIDAPI=0`: SDL's HIDAPI drivers address PlayStation and
///   Switch pads by another GUID than evdev's, so the mapping would not reach
///   them; this keeps SDL on evdev, where it does.
/// - `PROTON_PREFER_SDL=1`: likewise for Proton, which otherwise hands some
///   pads to Windows games raw (hidraw), around SDL.
///
/// Windows games get the virtual controller instead ([`virt`]). Values in the
/// game's own launch options win (`launch::set_default`). A game set to off, or
/// to keyboard and mouse, gets none of it; one with its own remap gets that
/// instead of the pad's (`pad_games.rs`).
pub fn game_env<R: Runtime>(app: &AppHandle<R>, game: &str) -> Vec<(String, String)> {
    if cfg!(windows) {
        return Vec::new();
    }
    let cfg = load_config(app);
    if !cfg.enabled || !cfg.games {
        return Vec::new();
    }
    let profiles = crate::pad_games::load(app);
    let profile = profiles.of(game);
    if profile.mode != crate::pad_games::Mode::Gamepad {
        return Vec::new();
    }
    sdl_env(&cfg.pads, profile)
}

/// A pad's USB ids, out of its SDL GUID (vendor at byte 4, product at byte 8,
/// both little-endian), as SDL's ignore list writes them.
fn ids_of_guid(guid: &str) -> Option<String> {
    let word = |at: usize| Some(u16::from_str_radix(guid.get(at + 2..at + 4)?, 16).ok()? << 8 | u16::from_str_radix(guid.get(at..at + 2)?, 16).ok()?);
    Some(format!("0x{:04X}/0x{:04X}", word(8)?, word(16)?))
}

/// The environment for a game in controller mode. SDL goes by model, so each
/// model gets one line: its own remap in this game, or the game's, or the
/// pad's. A model taken out of the game is put on SDL's ignore list, which
/// hides it from SDL's controller support (not from a game reading devices
/// another way).
pub(crate) fn sdl_env(pads: &BTreeMap<String, PadPrefs>, profile: &crate::pad_games::GameProfile) -> Vec<(String, String)> {
    let mut lines: Vec<String> = Vec::new();
    let mut ignore: Vec<String> = Vec::new();
    for (guid, prefs) in pads {
        let Some(for_pad) = profile.for_pad(guid) else {
            ignore.extend(ids_of_guid(guid));
            continue;
        };
        let remap = [for_pad, profile].into_iter().find(|p| !p.remap.is_identity()).map(|p| &p.remap);
        match remap {
            Some(r) => lines.extend(prefs.mapping.as_deref().or(prefs.base.as_deref()).and_then(|m| padmap::SdlMapping::parse(m).ok()).map(|m| m.remapped(r).to_line())),
            None => lines.extend(prefs.sdl.clone()),
        }
    }
    let mut env: Vec<(String, String)> = Vec::new();
    if !lines.is_empty() {
        env.push(("SDL_GAMECONTROLLERCONFIG".into(), padmap::config_env(lines.iter().map(String::as_str))));
        env.push(("SDL_JOYSTICK_HIDAPI".into(), "0".into()));
        env.push(("PROTON_PREFER_SDL".into(), "1".into()));
    }
    if !ignore.is_empty() {
        env.push(("SDL_GAMECONTROLLER_IGNORE_DEVICES".into(), ignore.join(",")));
    }
    env
}

/// The pads plugged in now, by [`crate::pad_games::pad_keys`], and whether the
/// virtual-controller driver answered, when last asked.
pub fn connected<R: Runtime>(app: &AppHandle<R>) -> (Vec<crate::pad_games::PadFacts>, Option<bool>) {
    let pads = app.state::<Pads>();
    let list = pads.list.lock().map(|l| l.clone()).unwrap_or_default();
    let keys = crate::pad_games::pad_keys(list.iter().map(|p| p.guid.as_str()));
    // Two pads of one name are numbered so they can be told apart. (On Windows
    // two Xbox pads can differ in GUID and still share a name.)
    let mut names: HashMap<&str, u32> = HashMap::new();
    let facts = list
        .iter()
        .zip(keys)
        .map(|(p, key)| {
            let n = names.entry(p.name.as_str()).or_insert(0);
            *n += 1;
            let name = if *n == 1 { p.name.clone() } else { format!("{} ({n})", p.name) };
            crate::pad_games::PadFacts { key, name, xinput: p.xinput, mapped: p.mapped }
        })
        .collect();
    let driver = pads.driver.lock().ok().and_then(|d| *d);
    (facts, driver)
}

/* ── Commands ─────────────────────────────────────────────── */

fn send(pads: &State<'_, Pads>, req: Req) {
    if let Ok(slot) = pads.tx.lock() {
        if let Some(tx) = slot.as_ref() {
            let _ = tx.send(req);
        }
    }
}

/// Start reading pads (the account has the flag). Returns what is plugged in.
///
/// One thread for the app's life: SDL may only be used from the thread that
/// started it, so stopping pauses that thread rather than ending it.
#[tauri::command]
pub fn pad_start(app: AppHandle, pads: State<'_, Pads>) -> Vec<PadInfo> {
    let _ = update_config(&app, |c| c.enabled = true);
    if let Ok(mut slot) = pads.tx.lock() {
        match slot.as_ref() {
            Some(tx) => {
                let _ = tx.send(Req::Start);
            }
            None => {
                let (tx, rx) = mpsc::channel();
                let app2 = app.clone();
                let back = tx.clone();
                if std::thread::Builder::new().name("pads".into()).spawn(move || run(&app2, rx, back)).is_ok() {
                    *slot = Some(tx);
                }
            }
        }
    }
    pads.list.lock().map(|l| l.clone()).unwrap_or_default()
}

/// Stop (the flag went off, or the account signed out).
#[tauri::command]
pub fn pad_stop(app: AppHandle, pads: State<'_, Pads>) {
    let _ = update_config(&app, |c| c.enabled = false);
    send(&pads, Req::Stop);
    if let Ok(mut l) = pads.list.lock() {
        l.clear();
    }
}

#[tauri::command]
pub fn pad_list(pads: State<'_, Pads>) -> Vec<PadInfo> {
    pads.list.lock().map(|l| l.clone()).unwrap_or_default()
}

#[tauri::command]
pub fn pad_config_get(app: AppHandle) -> PadConfig {
    load_config(&app)
}

/// Save Settings > Controller. Remaps are kept permutations, the setup and
/// base mappings are kept as they were (only `pad_mapping_set` and the reader
/// change those), and what games get is rewritten here, so Play only reads.
#[tauri::command]
pub fn pad_config_set(app: AppHandle, config: PadConfig) -> Result<PadConfig, String> {
    update_config(&app, |c| {
        let enabled = c.enabled;
        let before = std::mem::take(&mut c.pads);
        *c = config;
        c.enabled = enabled;
        for (guid, p) in c.pads.iter_mut() {
            let old = before.get(guid);
            p.mapping = old.and_then(|o| o.mapping.clone());
            p.base = old.and_then(|o| o.base.clone());
            p.remap = Remap::from_pairs(p.remap.changes().collect::<Vec<_>>());
            p.sdl = sdl_line(p);
        }
    })
}

/// A short buzz, to tell which pad is which.
#[tauri::command]
pub fn pad_rumble(pads: State<'_, Pads>, id: u32) {
    send(&pads, Req::Rumble(id));
}

/// A haptic pulse on one pad, as the shell moves around.
#[tauri::command]
pub fn pad_haptic(pads: State<'_, Pads>, id: u32, kind: Haptic) {
    send(&pads, Req::Haptic(id, kind));
}

/// A press (or the sticks) for the Store's page: kryo.to moves around by
/// controller on its own (components/gamepad/gamepad-layer.tsx) and takes the
/// app's input as `kryo-pad` events, so the player's own setup works there
/// too. A press also gives the Store the keyboard, so its selection shows.
#[tauri::command]
pub fn pad_forward(app: AppHandle, detail: serde_json::Value) {
    let Some(view) = app.get_webview(crate::STORE) else { return };
    let Ok(json) = serde_json::to_string(&detail) else { return };
    if detail["type"] == "button" && detail["pressed"] == true {
        let _ = view.set_focus();
    }
    let _ = view.eval(format!("window.dispatchEvent(new CustomEvent('kryo-pad',{{detail:{json}}}))"));
}

/// The setup listens to this pad's raw inputs (`pad-raw`), or stops.
#[tauri::command]
pub fn pad_capture(pads: State<'_, Pads>, id: Option<u32>) {
    send(&pads, Req::Capture(id));
}

/// What the setup can save for one control: a button (`b3`), a hat
/// direction (`h0.4`), an axis (`a2`), half an axis (`+a2`, `-a2`) or an
/// inverted one (`a1~`).
fn valid_source(s: &str) -> bool {
    let (body, rest) = match s.strip_prefix(['+', '-']) {
        Some(r) => (r, true),
        None => (s, false),
    };
    let body = body.strip_suffix('~').unwrap_or(body);
    let num = |n: &str| !n.is_empty() && n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit());
    match body.split_at_checked(1) {
        Some(("b", n)) => !rest && num(n),
        Some(("a", n)) => num(n),
        Some(("h", n)) => !rest && n.split_once('.').is_some_and(|(h, m)| num(h) && matches!(m, "1" | "2" | "4" | "8")),
        _ => false,
    }
}

/// Save (or, with `None`, forget) the setup of one model of pad: SDL field
/// name -> source, as the setup heard it. Kryoto uses it at once.
#[tauri::command]
pub fn pad_mapping_set(
    app: AppHandle,
    pads: State<'_, Pads>,
    guid: String,
    name: String,
    bindings: Option<BTreeMap<String, String>>,
) -> Result<PadConfig, String> {
    if guid.len() != 32 || !guid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("That controller has no valid id.".into());
    }
    let line = match bindings {
        None => None,
        Some(b) => {
            const AXES: [&str; 4] = ["leftx", "lefty", "rightx", "righty"];
            let mut out = format!("{guid},{},", name.replace(',', " ").trim());
            for (key, value) in &b {
                let known = Control::from_sdl_name(key).is_some() || AXES.contains(&key.as_str());
                if !known || !valid_source(value) {
                    return Err(format!("The setup sent {key}:{value}, which is not something a pad has."));
                }
                out.push_str(&format!("{key}:{value},"));
            }
            if !b.keys().any(|k| k == "a") {
                return Err("The setup needs at least the bottom face button.".into());
            }
            Some(out)
        }
    };
    let cfg = update_config(&app, |c| {
        let p = c.pads.entry(guid.clone()).or_default();
        p.mapping = line;
        p.sdl = sdl_line(p);
    })?;
    send(&pads, Req::Mapping(guid));
    Ok(cfg)
}

/// Windows: is the virtual-controller driver (ViGEmBus) there? Elsewhere
/// always false: Linux games get the setup through SDL instead.
#[tauri::command]
pub fn pad_virtual_driver(pads: State<'_, Pads>) -> bool {
    #[cfg(windows)]
    {
        let found = vigem_client::Client::connect().is_ok();
        if let Ok(mut d) = pads.driver.lock() {
            *d = Some(found);
        }
        send(&pads, Req::Refresh);
        found
    }
    #[cfg(not(windows))]
    {
        let _ = pads;
        false
    }
}

/* ── The reader thread ────────────────────────────────────── */

fn control_of(b: SButton) -> Control {
    match b {
        SButton::A => Control::South,
        SButton::B => Control::East,
        SButton::X => Control::West,
        SButton::Y => Control::North,
        SButton::Back => Control::Select,
        SButton::Guide => Control::Guide,
        SButton::Start => Control::Start,
        SButton::LeftStick => Control::LeftStick,
        SButton::RightStick => Control::RightStick,
        SButton::LeftShoulder => Control::LeftBumper,
        SButton::RightShoulder => Control::RightBumper,
        SButton::DPadUp => Control::DpadUp,
        SButton::DPadDown => Control::DpadDown,
        SButton::DPadLeft => Control::DpadLeft,
        SButton::DPadRight => Control::DpadRight,
        SButton::Misc1 => Control::Misc,
        SButton::Paddle1 => Control::PaddleUpperRight,
        SButton::Paddle2 => Control::PaddleUpperLeft,
        SButton::Paddle3 => Control::PaddleLowerRight,
        SButton::Paddle4 => Control::PaddleLowerLeft,
        SButton::Touchpad => Control::Touchpad,
    }
}

/// Sticks and triggers, -1..1 (sticks, y up) and 0..1 (triggers).
#[derive(Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Axes {
    id: u32,
    lx: f32,
    ly: f32,
    rx: f32,
    ry: f32,
    lt: f32,
    rt: f32,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ButtonEvent {
    id: u32,
    control: Control,
    pressed: bool,
}

/// A raw input, for the setup: `button` (index, 1 = down), `axis` (index,
/// -1..1) or `hat` (index, SDL's direction bits).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawEvent {
    id: u32,
    kind: &'static str,
    index: u32,
    value: f32,
}

/// Where every raw input rests, when the setup starts listening: so a
/// trigger resting at -1 is not taken for a pull.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RawState {
    id: u32,
    axes: Vec<f32>,
    hats: Vec<u8>,
    buttons: Vec<bool>,
}

struct Dev {
    joy: Joystick,
    ctl: Option<GameController>,
    info: PadInfo,
    /// The last value sent for each raw axis while capturing.
    raw_axes: Vec<f32>,
    /// Where the system has it, lowercased; empty when SDL does not say.
    path: String,
}

/// The system's path for a device, lowercased: the same pad reads the same,
/// whichever of SDL's drivers lists it.
fn joystick_path(index: i32) -> String {
    let raw = unsafe { sdl2::sys::SDL_JoystickPathForIndex(index) };
    if raw.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(raw) }.to_string_lossy().to_lowercase()
}

/// By its index: `Joystick::guid` panics on a device whose GUID reads as zero,
/// which took the whole app down on some Windows PCs. Empty is "not saved under anything".
fn guid_of(js: &sdl2::JoystickSubsystem, index: u32) -> String {
    js.device_guid(index).map(|g| g.string()).unwrap_or_default()
}

/// Windows briefly enumerates a dead WGI copy of the wired Stratus+ while
/// removing its XInput interface. Keep its working XInput interface; the
/// Bluetooth model (0x0111) and other WGI controllers remain available.
#[cfg(any(windows, test))]
fn inactive_stratus_interface(vendor: u16, product: u16, guid: &str) -> bool {
    vendor == 0x1038 && product == 0x1434 && guid.get(28..30) == Some("77")
}

fn battery_of(joy: &Joystick) -> (Option<u8>, bool) {
    use sdl2::joystick::PowerLevel::*;
    match joy.power_level() {
        Ok(Empty) => (Some(5), false),
        Ok(Low) => (Some(20), false),
        Ok(Medium) => (Some(55), false),
        Ok(Full) => (Some(100), false),
        _ => (None, false),
    }
}

/// The name to show: a model padmap knows by its ids, else the pad's own
/// name (SDL's database name when it has one), else its maker's.
fn display_name(model: &Model, sdl_name: &str) -> String {
    if model.matched == padmap::Matched::Ids || padmap::is_generic_name(sdl_name) {
        model.name.clone()
    } else {
        sdl_name.trim().to_string()
    }
}

struct Reader<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    js: sdl2::JoystickSubsystem,
    gc: sdl2::GameControllerSubsystem,
    devs: HashMap<u32, Dev>,
    /// Our own virtual controllers, as SDL sees them: never listed.
    ignored: HashSet<u32>,
    capture: Option<u32>,
    focused: bool,
    /// What the running game asks of the pads (`pad_games.rs`).
    games: crate::pad_games::Driver,
    #[cfg(windows)]
    virt: virt::Virtual,
}

impl<R: Runtime> Reader<'_, R> {
    /// The pads in connection order, each with its key: player 1 stays player 1.
    fn keyed(&self) -> Vec<(u32, String)> {
        let mut ids: Vec<u32> = self.devs.keys().copied().collect();
        ids.sort_unstable();
        let keys = crate::pad_games::pad_keys(ids.iter().map(|id| self.devs[id].info.guid.as_str()));
        ids.into_iter().zip(keys).collect()
    }

    /// What each pad has down right now.
    fn pad_states(&self) -> Vec<(String, crate::pad_games::PadState)> {
        const BUTTONS: [SButton; 15] = [
            SButton::A,
            SButton::B,
            SButton::X,
            SButton::Y,
            SButton::Back,
            SButton::Guide,
            SButton::Start,
            SButton::LeftStick,
            SButton::RightStick,
            SButton::LeftShoulder,
            SButton::RightShoulder,
            SButton::DPadUp,
            SButton::DPadDown,
            SButton::DPadLeft,
            SButton::DPadRight,
        ];
        self.keyed()
            .into_iter()
            .filter_map(|(id, key)| {
                let c = self.devs.get(&id)?.ctl.as_ref()?;
                let a = self.axes_of(id)?;
                let pressed = BUTTONS.into_iter().filter(|b| c.button(*b)).map(control_of).collect();
                Some((key, crate::pad_games::PadState { pressed, lx: a.lx, ly: a.ly, rx: a.rx, ry: a.ry, lt: a.lt, rt: a.rt }))
            })
            .collect()
    }

    /// Keys and mouse for the game in front. A short buzz says the pause
    /// chord took.
    fn feed_games(&mut self) {
        self.games.refresh(self.app);
        let keyboard = *self.games.plan() == crate::pad_games::Plan::Keyboard;
        let states = if keyboard { self.pad_states() } else { Vec::new() };
        if self.games.feed(self.app, states) {
            let ids: Vec<u32> = self.devs.keys().copied().collect();
            for id in ids {
                self.rumble(id, 0x5000, 0x3000, 120);
            }
        }
    }

    fn publish(&self) {
        let mut list: Vec<PadInfo> = self.devs.values().map(|d| d.info.clone()).collect();
        list.sort_by_key(|p| p.id);
        if let Ok(mut l) = self.app.state::<Pads>().list.lock() {
            *l = list.clone();
        }
        let _ = self.app.emit("pad-list", list);
    }

    /// Open device `index` (SDL's enumeration index), unless it is open or ours.
    fn open(&mut self, index: u32, announce: bool) -> Option<u32> {
        let i = index as i32;
        let id = unsafe { sdl2::sys::SDL_JoystickGetDeviceInstanceID(i) };
        if id < 0 {
            return None;
        }
        let id = id as u32;
        if self.devs.contains_key(&id) || self.ignored.contains(&id) {
            return None;
        }
        let vendor = unsafe { sdl2::sys::SDL_JoystickGetDeviceVendor(i) };
        let product = unsafe { sdl2::sys::SDL_JoystickGetDeviceProduct(i) };
        #[cfg(windows)]
        if self.virt.is_ours(vendor, product) {
            self.ignored.insert(id);
            return None;
        }
        // A device change (our own virtual pads cause them) can make SDL list
        // a pad it already has again, under a new id: that would be a second
        // virtual controller, and a player who does not exist.
        let path = joystick_path(i);
        if !path.is_empty() && self.devs.values().any(|d| d.path == path) {
            crate::logging::info("pads", &format!("listed twice, ignored: {path}"));
            self.ignored.insert(id);
            return None;
        }
        let kind = unsafe { sdl2::sys::SDL_GameControllerTypeForIndex(i) };
        let joy = self.js.open(index).ok()?;
        let ctl = if self.gc.is_game_controller(index) { self.gc.open(index).ok() } else { None };
        let system_name = ctl.as_ref().map(|c| c.name()).unwrap_or_else(|| joy.name());
        let some = |v: u16| (v != 0).then_some(v);
        let model = padmap::identify(some(vendor), some(product), &system_name);
        let guid = guid_of(&self.js, index);
        #[cfg(windows)]
        if inactive_stratus_interface(vendor, product, &guid) {
            self.ignored.insert(id);
            return None;
        }
        let custom = load_config(self.app).pads.get(&guid).is_some_and(|p| p.mapping.is_some());
        let (battery, charging) = battery_of(&joy);
        use sdl2::sys::SDL_GameControllerType::*;
        let info = PadInfo {
            id,
            guid: guid.clone(),
            name: display_name(&model, &system_name),
            system_name: system_name.clone(),
            vendor: some(vendor),
            product: some(product),
            model,
            mapped: ctl.is_some(),
            custom,
            xinput: matches!(kind, SDL_CONTROLLER_TYPE_XBOX360 | SDL_CONTROLLER_TYPE_XBOXONE),
            rumble: ctl.as_ref().map(|c| c.has_rumble()).unwrap_or_else(|| joy.has_rumble()),
            battery,
            charging,
            buttons: joy.num_buttons(),
            axes: joy.num_axes(),
            hats: joy.num_hats(),
        };
        // Remember SDL's own mapping (not a setup saved here) as the base.
        let base = if custom { None } else { ctl.as_ref().map(|c| c.mapping()) };
        {
            let _ = update_config(self.app, |c| {
                let p = c.pads.entry(guid.clone()).or_default();
                p.name = info.name.clone();
                if base.is_some() {
                    p.base = base.clone();
                }
                p.sdl = sdl_line(p);
            });
        }
        let axes = joy.num_axes() as usize;
        crate::logging::info("pads", &format!("opened {} ({}) at {path}", info.name, info.guid));
        self.devs.insert(id, Dev { joy, ctl, info: info.clone(), raw_axes: vec![0.0; axes], path });
        if announce && load_config(self.app).notify {
            ask_to_configure(self.app, &info);
        }
        Some(id)
    }

    fn open_all(&mut self) {
        let mut changed = false;
        let gone: Vec<u32> = self.devs.iter().filter(|(_, d)| !d.joy.attached()).map(|(id, _)| *id).collect();
        for id in gone {
            self.devs.remove(&id);
            #[cfg(windows)]
            self.virt.unplug(id);
            if self.capture == Some(id) { self.capture = None; }
            changed = true;
        }
        let n = self.js.num_joysticks().unwrap_or(0);
        for i in 0..n {
            changed |= self.open(i, true).is_some();
        }
        if changed { self.publish(); }
    }

    fn close_all(&mut self) {
        self.devs.clear();
        self.capture = None;
        #[cfg(windows)]
        self.virt.unplug_all();
        self.publish();
    }

    /// A setup was saved (or forgotten) for `guid`: tell SDL, and open the
    /// pads of that model as controllers if they were not yet.
    fn apply_mapping(&mut self, guid: &str) {
        let prefs = load_config(self.app).pads.get(guid).cloned().unwrap_or_default();
        // SDL cannot forget a mapping; going back means re-adding its own.
        if let Some(line) = prefs.mapping.as_deref().or(prefs.base.as_deref()) {
            if let Err(e) = self.gc.add_mapping(line) {
                crate::logging::error("pads", &format!("mapping refused: {e}"));
            }
        }
        let ids: Vec<u32> = self.devs.iter().filter(|(_, d)| d.info.guid == guid).map(|(id, _)| *id).collect();
        for id in ids {
            // Reopen: the controller (and its name, and whether it is mapped)
            // follows the new mapping.
            self.devs.remove(&id);
            let n = self.js.num_joysticks().unwrap_or(0);
            if let Some(index) = (0..n).find(|i| unsafe { sdl2::sys::SDL_JoystickGetDeviceInstanceID(*i as i32) } as u32 == id) {
                // A mapping change is not a new connection.
                self.open(index, false);
            }
        }
        self.publish();
    }

    fn rumble(&mut self, id: u32, low: u16, high: u16, ms: u32) {
        if let Some(d) = self.devs.get_mut(&id) {
            let _ = match d.ctl.as_mut() {
                Some(c) => c.set_rumble(low, high, ms),
                None => d.joy.set_rumble(low, high, ms),
            };
        }
    }

    fn axes_of(&self, id: u32) -> Option<Axes> {
        let c = self.devs.get(&id)?.ctl.as_ref()?;
        let n = |v: i16| (v as f32 / 32767.0).clamp(-1.0, 1.0);
        Some(Axes {
            id,
            lx: n(c.axis(SAxis::LeftX)),
            ly: -n(c.axis(SAxis::LeftY)),
            rx: n(c.axis(SAxis::RightX)),
            ry: -n(c.axis(SAxis::RightY)),
            lt: n(c.axis(SAxis::TriggerLeft)).max(0.0),
            rt: n(c.axis(SAxis::TriggerRight)).max(0.0),
        })
    }

    fn raw_state(&self, id: u32) -> Option<RawState> {
        let d = self.devs.get(&id)?;
        Some(RawState {
            id,
            axes: (0..d.joy.num_axes()).map(|a| d.joy.axis(a).map(|v| v as f32 / 32767.0).unwrap_or(0.0)).collect(),
            hats: (0..d.joy.num_hats()).map(|h| d.joy.hat(h).map(|s| s as u8).unwrap_or(0)).collect(),
            buttons: (0..d.joy.num_buttons()).map(|b| d.joy.button(b).unwrap_or(false)).collect(),
        })
    }

    fn handle(&mut self, ev: Event, dirty: &mut HashSet<u32>) {
        match ev {
            Event::JoyDeviceAdded { which, .. } if self.open(which, true).is_some() => self.publish(),
            Event::JoyDeviceRemoved { which, .. } => {
                self.ignored.remove(&which);
                if self.devs.remove(&which).is_some() {
                    #[cfg(windows)]
                    self.virt.unplug(which);
                    if self.capture == Some(which) {
                        self.capture = None;
                    }
                    self.publish();
                }
            }
            Event::ControllerButtonDown { which, button, .. } | Event::ControllerButtonUp { which, button, .. } => {
                let pressed = matches!(ev, Event::ControllerButtonDown { .. });
                if self.focused && self.devs.contains_key(&which) {
                    let _ = self.app.emit("pad-button", ButtonEvent { id: which, control: control_of(button), pressed });
                }
            }
            Event::ControllerAxisMotion { which, .. } => {
                dirty.insert(which);
            }
            Event::JoyButtonDown { which, button_idx, .. } | Event::JoyButtonUp { which, button_idx, .. }
                if self.capture == Some(which) =>
            {
                let value = if matches!(ev, Event::JoyButtonDown { .. }) { 1.0 } else { 0.0 };
                let _ = self.app.emit("pad-raw", RawEvent { id: which, kind: "button", index: button_idx as u32, value });
            }
            Event::JoyAxisMotion { which, axis_idx, value, .. } if self.capture == Some(which) => {
                let v = value as f32 / 32767.0;
                if let Some(d) = self.devs.get_mut(&which) {
                    let last = d.raw_axes.get(axis_idx as usize).copied().unwrap_or(0.0);
                    // Only real movement: a resting stick wobbles.
                    if (v - last).abs() > 0.08 {
                        if let Some(slot) = d.raw_axes.get_mut(axis_idx as usize) {
                            *slot = v;
                        }
                        let _ = self.app.emit("pad-raw", RawEvent { id: which, kind: "axis", index: axis_idx as u32, value: v });
                    }
                }
            }
            Event::JoyHatMotion { which, hat_idx, state, .. } if self.capture == Some(which) => {
                let bits = match state {
                    HatState::Centered => 0,
                    s => s as u8,
                };
                let _ = self.app.emit("pad-raw", RawEvent { id: which, kind: "hat", index: hat_idx as u32, value: bits as f32 });
            }
            _ => {}
        }
    }
}

/// Each connection: "Kryoto detected a controller", with a
/// Configure button that opens Settings > Controller on it.
fn ask_to_configure<R: Runtime>(app: &AppHandle<R>, pad: &PadInfo) {
    // Visible inside Kryoto even if Windows notifications are disabled.
    let _ = app.emit_to("main", "pad-connected", pad);
    let mut note = notify_rust::Notification::new();
    note.summary("Kryoto detected a controller");
    if pad.mapped {
        note.body(&format!("{} is connected. Would you like to configure this controller?", pad.name));
    } else {
        note.body(&format!("{} is connected. Set it up once so its buttons work in Kryoto and your games.", pad.name));
    }
    note.action("default", "Configure");
    note.action("configure", "Configure");
    note.auto_icon();
    // As `system::os_notify`: a toast filed under the installed app's ID.
    #[cfg(windows)]
    if !cfg!(debug_assertions) {
        note.app_id(&app.config().identifier);
    }
    let app = app.clone();
    let guid = pad.guid.clone();
    tauri::async_runtime::spawn_blocking(move || match note.show() {
        Ok(handle) => handle.wait_for_action(|action| {
            if action == "configure" || action == "default" {
                crate::system::show_main(&app);
                let _ = app.emit("pad-configure", guid);
            }
        }),
        Err(e) => crate::logging::error("notify", &e.to_string()),
    });
}

fn run<R: Runtime>(app: &AppHandle<R>, rx: mpsc::Receiver<Req>, back: mpsc::Sender<Req>) {
    // Read in the background too: the virtual controller needs the pad while
    // a game has the focus. Kryoto itself still only acts while in front.
    sdl2::hint::set("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");
    // Real-machine Stratus+ test: RawInput enumerates the wired pad but loses
    // its buttons/sticks; XInput reads all controls. HIDAPI and DirectInput
    // still handle other controllers (including Bluetooth) as before.
    #[cfg(windows)]
    sdl2::hint::set("SDL_JOYSTICK_RAWINPUT", "0");
    let sdl = match sdl2::init() {
        Ok(s) => s,
        Err(e) => return crate::logging::error("pads", &format!("controllers unavailable: {e}")),
    };
    let (js, gc, mut pump) = match (sdl.joystick(), sdl.game_controller(), sdl.event_pump()) {
        (Ok(j), Ok(g), Ok(p)) => (j, g, p),
        _ => return crate::logging::error("pads", "controllers unavailable: SDL did not start"),
    };
    // Setups saved here, before any pad is opened.
    for prefs in load_config(app).pads.values() {
        if let Some(line) = prefs.mapping.as_deref() {
            let _ = gc.add_mapping(line);
        }
    }
    let _ = &back;
    let mut r = Reader {
        app,
        js,
        gc,
        devs: HashMap::new(),
        ignored: HashSet::new(),
        capture: None,
        focused: false,
        games: crate::pad_games::Driver::default(),
        #[cfg(windows)]
        virt: virt::Virtual::new(back),
    };
    r.open_all();
    r.publish();

    let mut active = true;
    let mut focus_at = Instant::now() - Duration::from_secs(1);
    let mut sent: HashMap<u32, Axes> = HashMap::new();
    let mut sent_at = Instant::now();
    let mut dirty: HashSet<u32> = HashSet::new();
    let mut scan_at = Instant::now();

    loop {
        // Asks from the shell.
        loop {
            let req = if active {
                match rx.try_recv() {
                    Ok(r) => r,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            } else {
                // Paused: wait for Start without spinning.
                match rx.recv() {
                    Ok(r) => r,
                    Err(_) => return,
                }
            };
            match req {
                Req::Start => {
                    if !active {
                        active = true;
                        for _ in pump.poll_iter() {}
                    }
                    r.open_all();
                    r.publish();
                }
                Req::Stop => {
                    active = false;
                    r.games.stop(app);
                    r.close_all();
                }
                _ if !active => {}
                #[cfg(windows)]
                Req::Refresh => {
                    r.open_all();
                    r.virt.refresh_driver();
                }
                Req::Rumble(id) => r.rumble(id, 0xb000, 0xb000, 300),
                Req::Haptic(id, kind) => {
                    let (low, high, ms) = match kind {
                        Haptic::Tick => (0, 0x5000, 40),
                        Haptic::Select => (0x5000, 0x3000, 60),
                        Haptic::Edge => (0x9000, 0, 110),
                    };
                    r.rumble(id, low, high, ms);
                }
                Req::Capture(id) => {
                    r.capture = id;
                    if let Some(state) = id.and_then(|i| r.raw_state(i)) {
                        if let Some(d) = r.devs.get_mut(&state.id) {
                            d.raw_axes = state.axes.clone();
                        }
                        let _ = app.emit("pad-raw-state", state);
                    }
                }
                Req::Mapping(guid) => r.apply_mapping(&guid),
                Req::VirtualRumble(id, large, small) => {
                    r.rumble(id, u16::from(large) * 257, u16::from(small) * 257, 250);
                }
            }
        }
        if !active {
            continue;
        }
        // Reconcile missed hotplug/driver-change notifications without restarting.
        if scan_at.elapsed() >= Duration::from_secs(2) {
            scan_at = Instant::now();
            r.open_all();
        }

        if focus_at.elapsed() > Duration::from_millis(200) {
            focus_at = Instant::now();
            r.focused = app.get_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
        }

        if let Some(ev) = pump.wait_event_timeout(8) {
            r.handle(ev, &mut dirty);
            for ev in pump.poll_iter() {
                r.handle(ev, &mut dirty);
            }
        }

        // Sticks and triggers at most 30 times a second, while in front.
        if r.focused && sent_at.elapsed() >= Duration::from_millis(33) && !dirty.is_empty() {
            sent_at = Instant::now();
            for id in dirty.drain() {
                if let Some(a) = r.axes_of(id) {
                    if sent.get(&id) != Some(&a) {
                        sent.insert(id, a);
                        let _ = app.emit("pad-axes", a);
                    }
                }
            }
        }

        r.feed_games();
        #[cfg(windows)]
        r.feed_virtual();
    }
}

#[cfg(windows)]
impl<R: Runtime> Reader<'_, R> {
    /// While a game runs: every set-up, non-Xbox pad plays as a virtual Xbox
    /// controller, with the game's remap or its own, unless the game is set
    /// otherwise (`pad_games.rs`). Unplugged again when the games close.
    fn feed_virtual(&mut self) {
        let cfg_on = self.virt.check_settings(|| load_config(self.app).virtual_pad);
        let playing = self.app.state::<crate::library::Running>().0.lock().map(|m| !m.is_empty()).unwrap_or(false);
        if *self.games.plan() != crate::pad_games::Plan::Gamepad || !(cfg_on && playing) {
            self.virt.unplug_all();
            return;
        }
        let cfg = self.virt.cached_config(|| load_config(self.app));
        // In connection order: Windows numbers the virtual pads as they are
        // plugged in, so this is what keeps player 1 player 1.
        let mut live: HashSet<u32> = HashSet::new();
        let mut problem = None;
        for (id, key) in self.keyed() {
            let Some(d) = self.devs.get(&id) else { continue };
            let Some(ctl) = d.ctl.as_ref() else { continue };
            if d.info.xinput {
                continue;
            }
            // Taken out of this game: no virtual pad for it.
            let Some(remap) = self.games.pad_remap(&key) else { continue };
            let own = remap.is_some();
            let remap = remap.or_else(|| cfg.pads.get(&d.info.guid).map(|p| p.remap.clone())).unwrap_or_default();
            live.insert(id);
            if !self.virt.feed(id, ctl, &remap) && (own || !remap.is_identity()) {
                problem = Some(format!("{} could not be plugged in as a virtual Xbox controller, so its remap does not reach this game. Check ViGEmBus in Settings, Devices.", d.info.name));
            }
        }
        self.virt.retain(&live);
        if let Some(why) = problem {
            self.games.tell(self.app, &why);
        }
        if let Some(found) = self.virt.driver() {
            if let Ok(mut d) = self.app.state::<Pads>().driver.lock() {
                *d = Some(found);
            }
        }
    }
}

/// The virtual Xbox controller (Windows, ViGEmBus).
#[cfg(windows)]
mod virt {
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use padmap::{Control, Remap};
    use sdl2::controller::{Axis as SAxis, Button as SButton, GameController};
    use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};

    use super::{PadConfig, Req};

    pub struct Virtual {
        client: Option<Rc<Client>>,
        /// No driver: do not try again for a while.
        retry_at: Instant,
        driver: Option<bool>,
        targets: HashMap<u32, (Xbox360Wired<Rc<Client>>, XGamepad)>,
        /// Plugged in, not yet seen by SDL: the next such Xbox 360 pad is ours.
        expecting: Vec<Instant>,
        back: mpsc::Sender<Req>,
        settings: (Instant, bool),
        config: Option<(Instant, PadConfig)>,
    }

    impl Virtual {
        pub fn new(back: mpsc::Sender<Req>) -> Self {
            let past = Instant::now() - Duration::from_secs(60);
            Self {
                client: None,
                retry_at: past,
                driver: None,
                targets: HashMap::new(),
                expecting: Vec::new(),
                back,
                settings: (past, false),
                config: None,
            }
        }

        /// The setting, read at most once a second.
        pub fn check_settings(&mut self, read: impl FnOnce() -> bool) -> bool {
            if self.settings.0.elapsed() > Duration::from_secs(1) {
                self.settings = (Instant::now(), read());
            }
            self.settings.1
        }

        /// The remaps, read at most once a second while playing.
        pub fn cached_config(&mut self, read: impl FnOnce() -> PadConfig) -> PadConfig {
            match &self.config {
                Some((at, c)) if at.elapsed() < Duration::from_secs(1) => c.clone(),
                _ => {
                    let c = read();
                    self.config = Some((Instant::now(), c.clone()));
                    c
                }
            }
        }

        pub fn driver(&self) -> Option<bool> {
            self.driver
        }

        pub fn refresh_driver(&mut self) {
            self.retry_at = Instant::now();
            let _ = self.client();
        }

        /// Our own targets appear to SDL as Xbox 360 pads a moment after
        /// being plugged in; those are not listed or read.
        pub fn is_ours(&mut self, vendor: u16, product: u16) -> bool {
            self.expecting.retain(|t| t.elapsed() < Duration::from_secs(3));
            if vendor == 0x045e && product == 0x028e && !self.expecting.is_empty() {
                self.expecting.remove(0);
                return true;
            }
            false
        }

        fn client(&mut self) -> Option<Rc<Client>> {
            if self.client.is_none() && Instant::now() >= self.retry_at {
                match Client::connect() {
                    Ok(c) => {
                        self.client = Some(Rc::new(c));
                        self.driver = Some(true);
                    }
                    Err(_) => {
                        self.driver = Some(false);
                        self.retry_at = Instant::now() + Duration::from_secs(10);
                    }
                }
            }
            self.client.clone()
        }

        /// False when this pad has no virtual controller to feed.
        pub fn feed(&mut self, id: u32, ctl: &GameController, remap: &Remap) -> bool {
            let report = report(ctl, remap);
            if !self.targets.contains_key(&id) {
                let Some(client) = self.client() else { return false };
                let mut target = Xbox360Wired::new(client, TargetId::XBOX360_WIRED);
                if target.plugin().is_err() || target.wait_ready().is_err() {
                    return false;
                }
                self.expecting.push(Instant::now());
                // Rumble from the game goes to the real pad.
                if let Ok(note) = target.request_notification() {
                    let back = self.back.clone();
                    note.spawn_thread(move |_, n| {
                        let _ = back.send(Req::VirtualRumble(id, n.large_motor, n.small_motor));
                    });
                }
                self.targets.insert(id, (target, XGamepad::default()));
            }
            if let Some((target, last)) = self.targets.get_mut(&id) {
                if *last != report && target.update(&report).is_ok() {
                    *last = report;
                }
            }
            true
        }

        pub fn unplug(&mut self, id: u32) {
            if let Some((mut target, _)) = self.targets.remove(&id) {
                let _ = target.unplug();
            }
        }

        pub fn retain(&mut self, live: &HashSet<u32>) {
            let gone: Vec<u32> = self.targets.keys().filter(|id| !live.contains(id)).copied().collect();
            for id in gone {
                self.unplug(id);
            }
        }

        pub fn unplug_all(&mut self) {
            let all: Vec<u32> = self.targets.keys().copied().collect();
            for id in all {
                self.unplug(id);
            }
        }
    }

    fn button_of(c: Control) -> Option<SButton> {
        Some(match c {
            Control::South => SButton::A,
            Control::East => SButton::B,
            Control::West => SButton::X,
            Control::North => SButton::Y,
            Control::Select => SButton::Back,
            Control::Start => SButton::Start,
            Control::Guide => SButton::Guide,
            Control::LeftStick => SButton::LeftStick,
            Control::RightStick => SButton::RightStick,
            Control::LeftBumper => SButton::LeftShoulder,
            Control::RightBumper => SButton::RightShoulder,
            Control::DpadUp => SButton::DPadUp,
            Control::DpadDown => SButton::DPadDown,
            Control::DpadLeft => SButton::DPadLeft,
            Control::DpadRight => SButton::DPadRight,
            _ => return None,
        })
    }

    /// How far a physical control is pressed, 0..1.
    fn amount(ctl: &GameController, c: Control) -> f32 {
        match c {
            Control::LeftTrigger => (ctl.axis(SAxis::TriggerLeft) as f32 / 32767.0).clamp(0.0, 1.0),
            Control::RightTrigger => (ctl.axis(SAxis::TriggerRight) as f32 / 32767.0).clamp(0.0, 1.0),
            other => button_of(other).map(|b| if ctl.button(b) { 1.0 } else { 0.0 }).unwrap_or(0.0),
        }
    }

    /// The Xbox report for a pad: each job done by the control the remap
    /// gives it, sticks as they are.
    pub fn report(ctl: &GameController, remap: &Remap) -> XGamepad {
        let job = |c: Control| amount(ctl, remap.done_by(c));
        const BITS: [(Control, u16); 15] = [
            (Control::South, XButtons::A),
            (Control::East, XButtons::B),
            (Control::West, XButtons::X),
            (Control::North, XButtons::Y),
            (Control::LeftBumper, XButtons::LB),
            (Control::RightBumper, XButtons::RB),
            (Control::Select, XButtons::BACK),
            (Control::Start, XButtons::START),
            (Control::Guide, XButtons::GUIDE),
            (Control::LeftStick, XButtons::LTHUMB),
            (Control::RightStick, XButtons::RTHUMB),
            (Control::DpadUp, XButtons::UP),
            (Control::DpadDown, XButtons::DOWN),
            (Control::DpadLeft, XButtons::LEFT),
            (Control::DpadRight, XButtons::RIGHT),
        ];
        let mut raw = 0u16;
        for (c, bit) in BITS {
            if job(c) > 0.5 {
                raw |= bit;
            }
        }
        // SDL's y grows downwards, XInput's upwards.
        let flip = |v: i16| v.saturating_neg().max(-32767);
        XGamepad {
            buttons: XButtons { raw },
            left_trigger: (job(Control::LeftTrigger) * 255.0) as u8,
            right_trigger: (job(Control::RightTrigger) * 255.0) as u8,
            thumb_lx: ctl.axis(SAxis::LeftX),
            thumb_ly: flip(ctl.axis(SAxis::LeftY)),
            thumb_rx: ctl.axis(SAxis::RightX),
            thumb_ry: flip(ctl.axis(SAxis::RightY)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_inactive_wired_stratus_interface_is_filtered() {
        assert!(inactive_stratus_interface(0x1038, 0x1434, "0300a408381000003414000000007701"));
        assert!(!inactive_stratus_interface(0x1038, 0x1434, "0300fa67381000003414000000007801"));
        assert!(!inactive_stratus_interface(0x0111, 0x1434, "0300a408110100003414000000007701"));
        assert!(!inactive_stratus_interface(0x045e, 0x028e, "0300a4085e0400008e02000000007701"));
        assert!(!inactive_stratus_interface(0x1038, 0x1434, "invalid"));
    }

    #[test]
    fn config_defaults_and_old_files() {
        let c: PadConfig = serde_json::from_str("{}").unwrap();
        assert!(!c.enabled && c.navigate && !c.notify && c.games && c.haptics && c.virtual_pad);
        let c: PadConfig = serde_json::from_str(r#"{"pads":{"abc":{"name":"Pad","family":"nintendo","remap":{"a":"b","b":"a"}}}}"#).unwrap();
        let p = &c.pads["abc"];
        assert_eq!(p.family, Some(Family::Nintendo));
        assert_eq!(p.remap.acts_as(Control::South), Control::East);
    }

    #[test]
    fn what_games_get() {
        let base = "030000004c050000e60c000011810000,DualSense,a:b0,b:b1,x:b3,y:b2,platform:Linux,".to_string();
        // SDL knows it and nothing changed: nothing to hand over.
        let mut p = PadPrefs { base: Some(base.clone()), ..Default::default() };
        assert_eq!(sdl_line(&p), None);
        // A remap rewrites SDL's own mapping.
        p.remap.assign(Control::South, Control::East);
        let m = padmap::SdlMapping::parse(&sdl_line(&p).unwrap()).unwrap();
        assert_eq!((m.get("a"), m.get("b")), (Some("b1"), Some("b0")));
        // A setup saved here is handed over even without a remap.
        let p = PadPrefs { mapping: Some("03008231110100003414000000000000,SteelSeries,a:b0,b:b1,".into()), ..Default::default() };
        assert!(sdl_line(&p).unwrap().contains("a:b0"));
    }

    #[test]
    fn each_model_gets_its_own_line_and_a_removed_one_is_ignored() {
        use crate::pad_games::{GameProfile, PadUse};
        let sense = "030000004c050000e60c000011810000";
        let other = "03000000d62000000228000010010000";
        let line = |g: &str| format!("{g},Pad,a:b0,b:b1,x:b3,y:b2,platform:Linux,");
        let pads: BTreeMap<String, PadPrefs> = [sense, other].map(|g| (g.to_string(), PadPrefs { base: Some(line(g)), ..Default::default() })).into();
        let mut swap = Remap::new();
        swap.assign(Control::South, Control::East);
        let value = |env: &[(String, String)], k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());

        // Nothing changed, nothing to hand over.
        assert!(sdl_env(&pads, &GameProfile::default()).is_empty());

        // One controller's own remap: only its model's line changes.
        let mut p = GameProfile::default();
        p.pads.insert(sense.into(), PadUse { off: false, own: Some(Box::new(GameProfile { remap: swap.clone(), ..Default::default() })) });
        let env = sdl_env(&pads, &p);
        let config = value(&env, "SDL_GAMECONTROLLERCONFIG").unwrap();
        assert_eq!(config.lines().count(), 1);
        let m = padmap::SdlMapping::parse(&config).unwrap();
        assert_eq!((m.get("a"), m.get("b")), (Some("b1"), Some("b0")));

        // The game's remap reaches both; the first's own beats it.
        p.remap = swap.clone();
        assert_eq!(value(&sdl_env(&pads, &p), "SDL_GAMECONTROLLERCONFIG").unwrap().lines().count(), 2);

        // Taken out: on the ignore list, by its USB ids, and off the config.
        let mut p = GameProfile { remap: swap, ..Default::default() };
        p.pads.insert(sense.into(), PadUse { off: true, own: None });
        let env = sdl_env(&pads, &p);
        assert_eq!(value(&env, "SDL_GAMECONTROLLER_IGNORE_DEVICES").unwrap(), "0x054C/0x0CE6");
        assert_eq!(value(&env, "SDL_GAMECONTROLLERCONFIG").unwrap().lines().count(), 1);
    }

    #[test]
    fn setup_sources() {
        for ok in ["b0", "b15", "h0.1", "h0.8", "a2", "+a2", "-a5", "a1~", "+a3~"] {
            assert!(valid_source(ok), "{ok}");
        }
        for bad in ["", "b", "x1", "h0.3", "+b1", "a", "b1;rm", "a99999", "h.1"] {
            assert!(!valid_source(bad), "{bad}");
        }
    }

    #[test]
    fn a_placeholder_name_gives_way() {
        let model = padmap::identify(Some(0x0111), Some(0x1434), "As: 6 knop: 16 gamepad met kapschakelaar");
        assert_eq!(display_name(&model, "As: 6 knop: 16 gamepad met kapschakelaar"), "SteelSeries Stratus+");
        let model = padmap::identify(None, None, "8BitDo Pro 2");
        assert_eq!(display_name(&model, "8BitDo Pro 2"), "8BitDo Pro 2");
    }
}
