//! A controller's job in each game (Properties > Controller), for games
//! started outside Steam: left alone, played as a controller with a remap of
//! their own, or played as keyboard and mouse.
//!
//! What can really reach a game:
//!
//! - **Controller.** Windows: the virtual Xbox controller (`pad.rs`, ViGEmBus,
//!   an optional driver). An Xbox pad goes to the game directly and cannot be
//!   hidden from it without another driver, so a remap does not reach it there.
//!   Linux: an SDL mapping in the game's environment, which SDL, Wine and Proton
//!   read; a game that reads the device some other way does not see it.
//! - **Keyboard and mouse**, for games with no controller support. Windows:
//!   `SendInput` with scan codes, no driver; a game run as administrator refuses
//!   it. Linux: `/dev/uinput`, which is in the kernel but needs write permission
//!   (the `steam-devices` package or a udev rule).
//!
//! Synthetic input only goes to the game. Windows checks the window in front
//! belongs to it. Linux cannot ask that under Wayland, so there it goes out
//! while the game runs and Kryoto is not in front; Select + Start, held, pauses
//! it on both.
//!
//! A game with anti-cheat gets none of this ([`anti_cheat_in`]): no injected
//! input and no virtual controller, which such games may take for cheating.
//!
//! Several controllers on one game: each can be taken out of it or have its
//! own settings ([`PadUse`]), and [`describe`] says what the game will see.
//! Windows gives each non-Xbox pad its own virtual Xbox controller, in
//! connection order. Linux's mappings go by model, so two identical pads
//! cannot differ there. Keys and mouse are one keyboard and one mouse however
//! many pads press them: a game cannot tell players apart that way.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use padmap::{Control, Remap};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/* ── What is saved ────────────────────────────────────────── */

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Kryoto leaves the game alone.
    Off,
    /// The pad as a controller, as set up and remapped.
    #[default]
    Gamepad,
    /// The pad as keyboard and mouse.
    Keyboard,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StickUse {
    #[default]
    Off,
    /// W A S D.
    Wasd,
    /// The arrow keys.
    Arrows,
    /// A and D only: steering.
    Steer,
    /// Moves the mouse.
    Mouse,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameProfile {
    pub mode: Mode,
    /// The layout it started from, by id in [`TEMPLATES`].
    pub template: String,
    /// Controller mode: this game's own remap. Empty uses the pad's own.
    pub remap: Remap,
    /// Keyboard mode: what each control presses, by name in [`KEYS`].
    pub binds: BTreeMap<Control, String>,
    pub left_stick: StickUse,
    pub right_stick: StickUse,
    /// Pixels a second with a stick fully over.
    pub sensitivity: u32,
    /// How far a stick moves before it counts, 0..1.
    pub deadzone: f32,
    pub invert_y: bool,
    /// One controller's own say in this game, by [`pad_keys`]. A controller
    /// not in here does what the rest of this profile says.
    pub pads: BTreeMap<String, PadUse>,
}

/// What one controller does in a game, when it differs from the others.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PadUse {
    /// Taken out of this game.
    pub off: bool,
    /// Its own remap or keys. Its `mode` and `pads` mean nothing here.
    pub own: Option<Box<GameProfile>>,
}

/// A key for each pad, in connection order. SDL gives every pad of one model
/// the same GUID, so the second of a model is told apart by being the second.
pub fn pad_keys<'a>(guids: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen: HashMap<&str, u32> = HashMap::new();
    guids
        .into_iter()
        .map(|guid| {
            let n = seen.entry(guid).or_insert(0);
            *n += 1;
            if *n == 1 {
                guid.to_string()
            } else {
                format!("{guid}#{n}")
            }
        })
        .collect()
}

impl Default for GameProfile {
    fn default() -> Self {
        template("action").unwrap_or_else(blank)
    }
}

fn blank() -> GameProfile {
    GameProfile {
        mode: Mode::Gamepad,
        template: String::new(),
        remap: Remap::new(),
        binds: BTreeMap::new(),
        left_stick: StickUse::Off,
        right_stick: StickUse::Off,
        sensitivity: 1200,
        deadzone: 0.15,
        invert_y: false,
        pads: BTreeMap::new(),
    }
}

impl GameProfile {
    /// A hand-edited file, or a page, cannot hand the mapper nonsense.
    fn clean(mut self) -> Self {
        self.binds.retain(|_, name| key(name).is_some());
        self.remap = Remap::from_pairs(self.remap.changes().collect::<Vec<_>>());
        self.sensitivity = self.sensitivity.clamp(100, 4000);
        self.deadzone = if self.deadzone.is_finite() { self.deadzone.clamp(0.02, 0.6) } else { 0.15 };
        self.pads = std::mem::take(&mut self.pads)
            .into_iter()
            .map(|(k, mut u)| {
                u.own = u.own.map(|g| {
                    let mut g = Box::new(g.clean());
                    g.pads.clear();
                    g
                });
                (k, u)
            })
            .filter(|(_, u)| u.off || u.own.is_some())
            .collect();
        self
    }

    /// What this controller goes by in this game; `None` when it is taken out.
    pub fn for_pad(&self, key: &str) -> Option<&GameProfile> {
        match self.pads.get(key) {
            Some(u) if u.off => None,
            Some(PadUse { own: Some(own), .. }) => Some(own),
            _ => Some(self),
        }
    }
}

/// `controller-games.json`, beside `controllers.json`: its own file, because
/// Settings saves that one whole.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Profiles {
    /// What a game with none of its own gets.
    pub default: GameProfile,
    /// By library id.
    pub games: BTreeMap<String, GameProfile>,
}

impl Profiles {
    pub fn of(&self, game: &str) -> &GameProfile {
        self.games.get(game).unwrap_or(&self.default)
    }
}

fn path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("controller-games.json"))
}

static LOCK: Mutex<()> = Mutex::new(());

pub fn load<R: Runtime>(app: &AppHandle<R>) -> Profiles {
    let mut p: Profiles = path(app).and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    p.default = p.default.clean();
    p.games = std::mem::take(&mut p.games).into_iter().map(|(id, g)| (id, g.clean())).collect();
    p
}

fn update<R: Runtime>(app: &AppHandle<R>, f: impl FnOnce(&mut Profiles)) -> Result<(), String> {
    let _guard = LOCK.lock().map_err(|_| "Controller settings are busy.")?;
    let mut all = load(app);
    f(&mut all);
    let file = path(app).ok_or("No app data folder.")?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&all).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

/// A game taken off the library takes its settings with it.
pub fn forget<R: Runtime>(app: &AppHandle<R>, game: &str) {
    if load(app).games.contains_key(game) {
        let _ = update(app, |p| {
            p.games.remove(game);
        });
    }
}

/* ── Keys ─────────────────────────────────────────────────── */

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyKind {
    /// Scan code set 1 (Windows), extended or not, and Linux's key code.
    Key { scan: u16, extended: bool, linux: u16 },
    /// 0 left, 1 right, 2 middle, 3 back, 4 forward.
    Button(u8),
    /// Notches up (+) or down (-), on each press.
    Wheel(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct KeyDef {
    pub name: &'static str,
    pub label: &'static str,
    #[serde(skip)]
    pub kind: KeyKind,
}

const fn k(name: &'static str, label: &'static str, code: u16) -> KeyDef {
    // The main block's scan codes and Linux's key codes are the same numbers.
    KeyDef { name, label, kind: KeyKind::Key { scan: code, extended: false, linux: code } }
}

const fn x(name: &'static str, label: &'static str, scan: u16, linux: u16) -> KeyDef {
    KeyDef { name, label, kind: KeyKind::Key { scan, extended: true, linux } }
}

const fn b(name: &'static str, label: &'static str, button: u8) -> KeyDef {
    KeyDef { name, label, kind: KeyKind::Button(button) }
}

const fn w(name: &'static str, label: &'static str, notches: i32) -> KeyDef {
    KeyDef { name, label, kind: KeyKind::Wheel(notches) }
}

/// Everything a control can press, in the order the page lists them.
pub const KEYS: &[KeyDef] = &[
    b("mouse1", "Left click", 0),
    b("mouse2", "Right click", 1),
    b("mouse3", "Middle click", 2),
    b("mouse4", "Mouse back", 3),
    b("mouse5", "Mouse forward", 4),
    w("wheelup", "Wheel up", 1),
    w("wheeldown", "Wheel down", -1),
    k("space", "Space", 57),
    k("enter", "Enter", 28),
    k("esc", "Esc", 1),
    k("tab", "Tab", 15),
    k("backspace", "Backspace", 14),
    k("lshift", "Shift", 42),
    k("lctrl", "Ctrl", 29),
    k("lalt", "Alt", 56),
    x("up", "Up arrow", 0x48, 103),
    x("down", "Down arrow", 0x50, 108),
    x("left", "Left arrow", 0x4B, 105),
    x("right", "Right arrow", 0x4D, 106),
    k("a", "A", 30),
    k("b", "B", 48),
    k("c", "C", 46),
    k("d", "D", 32),
    k("e", "E", 18),
    k("f", "F", 33),
    k("g", "G", 34),
    k("h", "H", 35),
    k("i", "I", 23),
    k("j", "J", 36),
    k("k", "K", 37),
    k("l", "L", 38),
    k("m", "M", 50),
    k("n", "N", 49),
    k("o", "O", 24),
    k("p", "P", 25),
    k("q", "Q", 16),
    k("r", "R", 19),
    k("s", "S", 31),
    k("t", "T", 20),
    k("u", "U", 22),
    k("v", "V", 47),
    k("w", "W", 17),
    k("x", "X", 45),
    k("y", "Y", 21),
    k("z", "Z", 44),
    k("1", "1", 2),
    k("2", "2", 3),
    k("3", "3", 4),
    k("4", "4", 5),
    k("5", "5", 6),
    k("6", "6", 7),
    k("7", "7", 8),
    k("8", "8", 9),
    k("9", "9", 10),
    k("0", "0", 11),
    k("f1", "F1", 59),
    k("f2", "F2", 60),
    k("f3", "F3", 61),
    k("f4", "F4", 62),
    k("f5", "F5", 63),
    k("f6", "F6", 64),
    k("f7", "F7", 65),
    k("f8", "F8", 66),
    k("f9", "F9", 67),
    k("f10", "F10", 68),
    k("f11", "F11", 87),
    k("f12", "F12", 88),
];

pub fn key(name: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|d| d.name == name)
}

/* ── Layouts ──────────────────────────────────────────────── */

#[derive(Clone, Copy, Debug, Serialize)]
pub struct TemplateInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
}

/// One layout per kind of game.
pub const TEMPLATES: &[TemplateInfo] = &[
    TemplateInfo { id: "shooter", label: "First-person shooter", hint: "Left stick walks, right stick aims, triggers aim and fire." },
    TemplateInfo { id: "action", label: "Third-person action", hint: "Left stick walks, right stick turns the camera." },
    TemplateInfo { id: "platformer", label: "Platformer", hint: "Move on the left stick or d-pad, jump on the bottom button." },
    TemplateInfo { id: "strategy", label: "Strategy and point-and-click", hint: "Left stick moves the pointer, right stick scrolls the map." },
    TemplateInfo { id: "racing", label: "Racing", hint: "Left stick steers, triggers are throttle and brake. Keys are on or off, so steering is not gradual." },
    TemplateInfo { id: "player2", label: "Second player on one keyboard", hint: "Arrow keys and the keys around them, for games where two players share a keyboard. Give this to the second controller." },
];

/// A layout's keyboard and mouse settings. Controller mode, as for any game.
pub fn template(id: &str) -> Option<GameProfile> {
    use Control::*;
    let (binds, left, right, sensitivity): (&[(Control, &str)], StickUse, StickUse, u32) = match id {
        "shooter" => (
            &[
                (RightTrigger, "mouse1"),
                (LeftTrigger, "mouse2"),
                (South, "space"),
                (East, "lctrl"),
                (West, "r"),
                (North, "e"),
                (LeftBumper, "g"),
                (RightBumper, "q"),
                (LeftStick, "lshift"),
                (RightStick, "v"),
                (Start, "esc"),
                (Select, "tab"),
                (DpadUp, "1"),
                (DpadRight, "2"),
                (DpadDown, "3"),
                (DpadLeft, "4"),
            ],
            StickUse::Wasd,
            StickUse::Mouse,
            1600,
        ),
        "action" => (
            &[
                (RightTrigger, "mouse1"),
                (LeftTrigger, "mouse2"),
                (South, "space"),
                (East, "lctrl"),
                (West, "e"),
                (North, "r"),
                (LeftBumper, "q"),
                (RightBumper, "f"),
                (LeftStick, "lshift"),
                (RightStick, "mouse3"),
                (Start, "esc"),
                (Select, "tab"),
                (DpadUp, "1"),
                (DpadRight, "2"),
                (DpadDown, "3"),
                (DpadLeft, "4"),
            ],
            StickUse::Wasd,
            StickUse::Mouse,
            1200,
        ),
        "platformer" => (
            &[
                (South, "space"),
                (East, "lshift"),
                (West, "e"),
                (North, "f"),
                (LeftBumper, "q"),
                (RightBumper, "r"),
                (Start, "esc"),
                (Select, "tab"),
                (DpadUp, "up"),
                (DpadRight, "right"),
                (DpadDown, "down"),
                (DpadLeft, "left"),
            ],
            StickUse::Wasd,
            StickUse::Off,
            1200,
        ),
        "strategy" => (
            &[
                (South, "mouse1"),
                (East, "mouse2"),
                (West, "lshift"),
                (North, "lctrl"),
                (RightTrigger, "mouse1"),
                (LeftTrigger, "mouse2"),
                (LeftBumper, "wheeldown"),
                (RightBumper, "wheelup"),
                (Start, "esc"),
                (Select, "tab"),
                (DpadUp, "1"),
                (DpadRight, "2"),
                (DpadDown, "3"),
                (DpadLeft, "4"),
            ],
            StickUse::Mouse,
            StickUse::Arrows,
            900,
        ),
        "racing" => (
            &[
                (RightTrigger, "w"),
                (LeftTrigger, "s"),
                (South, "space"),
                (East, "r"),
                (West, "c"),
                (North, "v"),
                (LeftBumper, "q"),
                (RightBumper, "e"),
                (Start, "esc"),
                (Select, "tab"),
            ],
            StickUse::Steer,
            StickUse::Off,
            1200,
        ),
        "player2" => (
            &[
                (South, "enter"),
                (East, "l"),
                (West, "k"),
                (North, "j"),
                (DpadUp, "up"),
                (DpadRight, "right"),
                (DpadDown, "down"),
                (DpadLeft, "left"),
            ],
            StickUse::Arrows,
            StickUse::Off,
            1200,
        ),
        _ => return None,
    };
    Some(GameProfile {
        template: id.to_string(),
        binds: binds.iter().map(|(c, name)| (*c, (*name).to_string())).collect(),
        left_stick: left,
        right_stick: right,
        sensitivity,
        ..blank()
    })
}

/* ── From pad to keys ─────────────────────────────────────── */

/// A pad's state, all pads together. Sticks -1..1 with y up, triggers 0..1.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadState {
    pub pressed: HashSet<Control>,
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
    pub lt: f32,
    pub rt: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Key(&'static KeyDef, bool),
    Move(i32, i32),
    Wheel(i32),
}

/// A trigger counts as pressed from here.
const TRIGGER_ON: f32 = 0.35;
/// A stick used as keys presses here, and lets go a little lower, so a stick
/// held at the edge does not chatter.
const STICK_ON: f32 = 0.5;
const STICK_OFF: f32 = 0.35;

/// Turns pad states into key and mouse events. Knows nothing of the system.
#[derive(Default)]
pub struct Mapper {
    held: HashSet<&'static str>,
    /// What is left over of a pixel, x and y.
    carry: (f32, f32),
}

impl Mapper {
    fn stick_keys(&self, using: StickUse, x: f32, y: f32, want: &mut HashSet<&'static str>) {
        let (up, left, down, right) = match using {
            StickUse::Wasd => ("w", "a", "s", "d"),
            StickUse::Arrows => ("up", "left", "down", "right"),
            StickUse::Steer => ("", "a", "", "d"),
            StickUse::Off | StickUse::Mouse => return,
        };
        let mut side = |name: &'static str, amount: f32| {
            let limit = if self.held.contains(name) { STICK_OFF } else { STICK_ON };
            if !name.is_empty() && amount > limit {
                want.insert(name);
            }
        };
        side(up, y);
        side(down, -y);
        side(left, -x);
        side(right, x);
    }

    /// One tick, `dt` seconds after the last, for each controller in play,
    /// each by its own settings. There is one keyboard and one mouse: a key
    /// two controllers hold stays down until the last lets go, and the sticks
    /// on the mouse add up.
    pub fn step_all(&mut self, pads: &[(&GameProfile, &PadState)], dt: f32, out: &mut Vec<Out>) {
        let mut want: HashSet<&'static str> = HashSet::new();
        for (p, s) in pads {
            for (control, name) in &p.binds {
                let on = match control {
                    Control::LeftTrigger => s.lt > TRIGGER_ON,
                    Control::RightTrigger => s.rt > TRIGGER_ON,
                    c => s.pressed.contains(c),
                };
                if on {
                    if let Some(def) = key(name) {
                        want.insert(def.name);
                    }
                }
            }
            self.stick_keys(p.left_stick, s.lx, s.ly, &mut want);
            self.stick_keys(p.right_stick, s.rx, s.ry, &mut want);
        }

        // Let go before pressing: two keys swapping never overlap the wrong way.
        let gone: Vec<&'static str> = self.held.difference(&want).copied().collect();
        for name in gone {
            self.held.remove(name);
            if let Some(def) = key(name) {
                if !matches!(def.kind, KeyKind::Wheel(_)) {
                    out.push(Out::Key(def, false));
                }
            }
        }
        let new: Vec<&'static str> = want.difference(&self.held).copied().collect();
        for name in new {
            self.held.insert(name);
            if let Some(def) = key(name) {
                match def.kind {
                    KeyKind::Wheel(n) => out.push(Out::Wheel(n)),
                    _ => out.push(Out::Key(def, true)),
                }
            }
        }

        // The mouse: the stick past the deadzone, on a curve, so small moves
        // are fine and a full push is fast. Screen y grows downwards.
        for (p, s) in pads {
            let flip = if p.invert_y { 1.0 } else { -1.0 };
            let speed = p.sensitivity as f32 * dt;
            for (using, sx, sy) in [(p.left_stick, s.lx, s.ly), (p.right_stick, s.rx, s.ry)] {
                if using == StickUse::Mouse {
                    let (dx, dy) = curve(sx, sy, p.deadzone);
                    self.carry.0 += dx * speed;
                    self.carry.1 += dy * speed * flip;
                }
            }
        }
        let (mx, my) = (self.carry.0.trunc(), self.carry.1.trunc());
        self.carry.0 -= mx;
        self.carry.1 -= my;
        if mx != 0.0 || my != 0.0 {
            out.push(Out::Move(mx as i32, my as i32));
        }
    }

    /// One controller's tick.
    #[cfg(test)]
    pub fn step(&mut self, p: &GameProfile, s: &PadState, dt: f32, out: &mut Vec<Out>) {
        self.step_all(&[(p, s)], dt, out);
    }

    /// Everything up: the game lost the front, closed, or was paused.
    pub fn release(&mut self, out: &mut Vec<Out>) {
        for name in self.held.drain() {
            if let Some(def) = key(name) {
                if !matches!(def.kind, KeyKind::Wheel(_)) {
                    out.push(Out::Key(def, false));
                }
            }
        }
        self.carry = (0.0, 0.0);
    }
}

/// A stick past its deadzone, 0..1 again and squared along its direction.
fn curve(x: f32, y: f32, deadzone: f32) -> (f32, f32) {
    let length = (x * x + y * y).sqrt();
    if length <= deadzone {
        return (0.0, 0.0);
    }
    let amount = ((length - deadzone) / (1.0 - deadzone)).min(1.0);
    let scale = amount * amount / length;
    (x * scale, y * scale)
}

/* ── Anti-cheat ───────────────────────────────────────────── */

/// Names in a game's folder that mean anti-cheat, lowercased.
const ANTI_CHEATS: &[(&str, &str)] = &[
    ("easyanticheat", "Easy Anti-Cheat"),
    ("eaanticheat", "EA anticheat"),
    ("battleye", "BattlEye"),
    ("beservice", "BattlEye"),
    ("vgc.exe", "Riot Vanguard"),
    ("vanguard", "Riot Vanguard"),
    ("xigncode", "XIGNCODE3"),
    ("gameguard", "GameGuard"),
    ("pnkbstr", "PunkBuster"),
    ("punkbuster", "PunkBuster"),
    ("denuvo-anti-cheat", "Denuvo Anti-Cheat"),
    ("anticheatexpert", "Anti-Cheat Expert"),
    ("mhyprot", "miHoYo's anti-cheat"),
    ("faceit", "FACEIT"),
];

fn anti_cheat_named(name: &str) -> Option<&'static str> {
    let name = name.to_ascii_lowercase();
    ANTI_CHEATS.iter().find(|(needle, _)| name.contains(needle)).map(|(_, label)| *label)
}

/// The anti-cheat in a game's folder, if any: a look at the top few levels,
/// where these sit, not the whole game. D: can be a slow disk.
pub fn anti_cheat_in(dir: &Path) -> Option<&'static str> {
    walkdir::WalkDir::new(dir).max_depth(3).into_iter().flatten().find_map(|e| anti_cheat_named(&e.file_name().to_string_lossy()))
}

/// What was found in a folder, `None` while the look is running.
type Scan = Option<Option<&'static str>>;

static SCANS: Mutex<Option<HashMap<PathBuf, Scan>>> = Mutex::new(None);

/// What is known of a folder; asks for a look (off this thread) the first time.
fn anti_cheat_known(dir: &Path) -> Option<Option<&'static str>> {
    let mut scans = SCANS.lock().ok()?;
    let map = scans.get_or_insert_with(HashMap::new);
    if let Some(known) = map.get(dir) {
        return *known;
    }
    map.insert(dir.to_path_buf(), None);
    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        let found = anti_cheat_in(&dir);
        if let Ok(mut scans) = SCANS.lock() {
            scans.get_or_insert_with(HashMap::new).insert(dir, Some(found));
        }
    });
    None
}

/* ── Sending it ───────────────────────────────────────────── */

/// On every event Kryoto sends on Windows ("KRYO"), so it can be told from
/// what other software on the PC is doing with the same controller.
#[cfg(windows)]
pub const SIGNATURE: usize = 0x4B52_594F;

/// Why keyboard and mouse cannot reach games here, or `None` when they can.
fn output_problem() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        out::Output::open().err()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(windows)]
mod out {
    use super::{KeyKind, Out};
    use std::mem::size_of;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

    pub struct Output;

    fn key_input(scan: u16, extended: bool, down: bool) -> INPUT {
        let mut flags = KEYEVENTF_SCANCODE;
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !down {
            flags |= KEYEVENTF_KEYUP;
        }
        INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: super::SIGNATURE } } }
    }

    fn mouse_input(dx: i32, dy: i32, data: i32, flags: u32) -> INPUT {
        INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: super::SIGNATURE } } }
    }

    fn button_input(button: u8, down: bool) -> INPUT {
        const XBUTTON1: i32 = 1;
        const XBUTTON2: i32 = 2;
        let (flags, data) = match (button, down) {
            (0, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (0, false) => (MOUSEEVENTF_LEFTUP, 0),
            (1, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (1, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (2, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (2, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (3, true) => (MOUSEEVENTF_XDOWN, XBUTTON1),
            (3, false) => (MOUSEEVENTF_XUP, XBUTTON1),
            (_, true) => (MOUSEEVENTF_XDOWN, XBUTTON2),
            (_, false) => (MOUSEEVENTF_XUP, XBUTTON2),
        };
        mouse_input(0, 0, data, flags)
    }

    impl Output {
        pub fn open() -> Result<Self, String> {
            Ok(Self)
        }

        pub fn send(&mut self, events: &[Out]) -> Result<(), String> {
            let inputs: Vec<INPUT> = events
                .iter()
                .map(|e| match e {
                    Out::Key(def, down) => match def.kind {
                        KeyKind::Key { scan, extended, .. } => key_input(scan, extended, *down),
                        KeyKind::Button(n) => button_input(n, *down),
                        KeyKind::Wheel(_) => mouse_input(0, 0, 0, 0),
                    },
                    Out::Move(x, y) => mouse_input(*x, *y, 0, MOUSEEVENTF_MOVE),
                    Out::Wheel(n) => mouse_input(0, 0, n * 120, MOUSEEVENTF_WHEEL),
                })
                .collect();
            if inputs.is_empty() {
                return Ok(());
            }
            let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
            if sent as usize == inputs.len() {
                Ok(())
            } else {
                Err("Windows refused the input. It does that while an administrator prompt is up, and for a game run as administrator.".into())
            }
        }
    }

    /// The process that owns the window in front, and its program's path.
    pub fn front() -> Option<(u32, std::path::PathBuf)> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::*;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, &mut pid);
            if pid == 0 {
                return None;
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                // An elevated game's path cannot be read; its id still can.
                return Some((pid, std::path::PathBuf::new()));
            }
            let mut buffer = [0u16; 1024];
            let mut size = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size);
            CloseHandle(process);
            let path = if ok != 0 { String::from_utf16_lossy(&buffer[..size as usize]) } else { String::new() };
            Some((pid, path.into()))
        }
    }
}

#[cfg(target_os = "linux")]
mod out {
    use super::{KeyKind, Out, KEYS};
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    const EV_SYN: u16 = 0;
    const EV_KEY: u16 = 1;
    const EV_REL: u16 = 2;
    const REL_X: u16 = 0;
    const REL_Y: u16 = 1;
    const REL_WHEEL: u16 = 8;
    /// BTN_LEFT, then right, middle, side (back) and extra (forward).
    const BTN_LEFT: u16 = 0x110;

    const UI_DEV_CREATE: libc::c_ulong = 0x5501;
    const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
    const UI_DEV_SETUP: libc::c_ulong = 0x405c_5503;
    const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
    const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
    const UI_SET_RELBIT: libc::c_ulong = 0x4004_5566;

    pub const NAME: &str = "Kryoto controller as keyboard and mouse";

    #[repr(C)]
    struct Setup {
        bustype: u16,
        vendor: u16,
        product: u16,
        version: u16,
        name: [u8; 80],
        ff_effects_max: u32,
    }

    /// `struct input_event`: the time is left zero, the kernel stamps it.
    #[repr(C)]
    struct Event {
        time: libc::timeval,
        kind: u16,
        code: u16,
        value: i32,
    }

    fn bytes(e: &Event) -> &[u8] {
        unsafe { std::slice::from_raw_parts((e as *const Event).cast(), std::mem::size_of::<Event>()) }
    }

    /// A virtual keyboard and mouse, for as long as this lives.
    pub struct Output {
        file: File,
    }

    fn set(fd: i32, request: libc::c_ulong, value: u16) -> bool {
        unsafe { libc::ioctl(fd, request, libc::c_int::from(value)) >= 0 }
    }

    impl Output {
        pub fn open() -> Result<Self, String> {
            let file = OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open("/dev/uinput").map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    "This system has no /dev/uinput.".to_string()
                } else {
                    "Kryoto may not write to /dev/uinput. Install your distro's steam-devices package, or add a udev rule giving your user access to it, then sign in again.".to_string()
                }
            })?;
            let fd = file.as_raw_fd();
            let mut ok = set(fd, UI_SET_EVBIT, EV_KEY) && set(fd, UI_SET_EVBIT, EV_REL);
            for def in KEYS {
                if let KeyKind::Key { linux, .. } = def.kind {
                    ok &= set(fd, UI_SET_KEYBIT, linux);
                }
            }
            for button in 0..5 {
                ok &= set(fd, UI_SET_KEYBIT, BTN_LEFT + button);
            }
            for rel in [REL_X, REL_Y, REL_WHEEL] {
                ok &= set(fd, UI_SET_RELBIT, rel);
            }
            let mut setup = Setup { bustype: 0x03, vendor: 0x4b52, product: 0x0001, version: 1, name: [0; 80], ff_effects_max: 0 };
            setup.name[..NAME.len()].copy_from_slice(NAME.as_bytes());
            ok = ok && unsafe { libc::ioctl(fd, UI_DEV_SETUP, &setup as *const Setup) >= 0 && libc::ioctl(fd, UI_DEV_CREATE) >= 0 };
            if !ok {
                return Err("Linux refused the virtual keyboard and mouse.".into());
            }
            Ok(Self { file })
        }

        fn write(&mut self, kind: u16, code: u16, value: i32) -> std::io::Result<()> {
            let event = Event { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, kind, code, value };
            self.file.write_all(bytes(&event))
        }

        pub fn send(&mut self, events: &[Out]) -> Result<(), String> {
            if events.is_empty() {
                return Ok(());
            }
            let mut go = || -> std::io::Result<()> {
                for e in events {
                    match e {
                        Out::Key(def, down) => match def.kind {
                            KeyKind::Key { linux, .. } => self.write(EV_KEY, linux, i32::from(*down))?,
                            KeyKind::Button(n) => self.write(EV_KEY, BTN_LEFT + u16::from(n), i32::from(*down))?,
                            KeyKind::Wheel(_) => {}
                        },
                        Out::Move(x, y) => {
                            self.write(EV_REL, REL_X, *x)?;
                            self.write(EV_REL, REL_Y, *y)?;
                        }
                        Out::Wheel(n) => self.write(EV_REL, REL_WHEEL, *n)?,
                    }
                }
                self.write(EV_SYN, 0, 0)
            };
            go().map_err(|e| format!("The virtual keyboard and mouse stopped working: {e}"))
        }
    }

    impl Drop for Output {
        fn drop(&mut self) {
            unsafe { libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY) };
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod out {
    use super::Out;

    pub struct Output;

    impl Output {
        pub fn open() -> Result<Self, String> {
            Err("Not on this system.".into())
        }

        pub fn send(&mut self, _: &[Out]) -> Result<(), String> {
            Ok(())
        }
    }
}

/* ── While a game runs (the pad thread) ───────────────────── */

/// What the running game asks of the pads, right now.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Plan {
    /// No game, or nothing wanted: no virtual controller, no keys.
    #[default]
    Nothing,
    /// The virtual controller, for each controller in play ([`Driver::pad_remap`]).
    Gamepad,
    /// Keys and mouse.
    Keyboard,
}

/// Whether a program is inside a game's folder, however either path is written.
#[cfg_attr(not(windows), allow(dead_code))]
fn runs_from(exe: &Path, dir: &Path) -> bool {
    let plain = |p: &Path| {
        let text = p.to_string_lossy().to_lowercase().replace('/', "\\");
        text.strip_prefix("\\\\?\\").unwrap_or(&text).trim_end_matches('\\').to_string()
    };
    let (exe, dir) = (plain(exe), plain(dir));
    !dir.is_empty() && exe.starts_with(&format!("{dir}\\"))
}

/// Select + Start held this long pauses keys and mouse, or starts them again.
const PAUSE_HOLD: Duration = Duration::from_millis(600);

pub struct Driver {
    profiles: Profiles,
    read_at: Instant,
    /// Settings > Controller: hand the setup and remaps to games.
    to_games: bool,
    /// Library folders of the running games, by id.
    dirs: HashMap<String, PathBuf>,
    plan: Plan,
    profile: GameProfile,
    game: Option<String>,
    in_front: bool,
    front_at: Instant,
    paused: bool,
    chord_since: Option<Instant>,
    chord_done: bool,
    mapper: Mapper,
    output: Option<out::Output>,
    /// Said once per run of a game: it stays in the log after that.
    told: bool,
    /// Sends refused in a row.
    refused: u32,
    tick: Instant,
}

impl Default for Driver {
    fn default() -> Self {
        let past = Instant::now() - Duration::from_secs(60);
        Self {
            profiles: Profiles::default(),
            read_at: past,
            to_games: true,
            dirs: HashMap::new(),
            plan: Plan::Nothing,
            profile: GameProfile::default(),
            game: None,
            in_front: false,
            front_at: past,
            paused: false,
            chord_since: None,
            chord_done: false,
            mapper: Mapper::default(),
            output: None,
            told: false,
            refused: 0,
            tick: Instant::now(),
        }
    }
}

impl Driver {
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Which running game is in front (Windows); the rest of the time, the
    /// first. Asked a few times a second.
    fn pick_game<R: Runtime>(&mut self, app: &AppHandle<R>, sessions: &[(String, Option<u32>)]) -> Option<String> {
        #[cfg(windows)]
        {
            let front = out::front();
            let found = front.and_then(|(pid, exe)| {
                sessions.iter().find(|(id, session)| *session == Some(pid) || self.dirs.get(id).is_some_and(|d| runs_from(&exe, d))).map(|(id, _)| id.clone())
            });
            self.in_front = found.is_some() || (cfg!(debug_assertions) && std::env::var_os("KRYOTO_DEV_PAD_FRONT").is_some());
            let _ = app;
            found.or_else(|| sessions.first().map(|(id, _)| id.clone()))
        }
        #[cfg(not(windows))]
        {
            // Wayland will not say which window is in front; Kryoto can only
            // tell that it is not itself.
            self.in_front = !app.get_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
            sessions.first().map(|(id, _)| id.clone())
        }
    }

    /// The pads were stopped: nothing stays held down.
    pub fn stop<R: Runtime>(&mut self, app: &AppHandle<R>) {
        let mut events = Vec::new();
        self.mapper.release(&mut events);
        self.send(app, &events);
        self.output = None;
        self.plan = Plan::Nothing;
    }

    /// What the running game asks for. Cheap: the disk at most once a second.
    pub fn refresh<R: Runtime>(&mut self, app: &AppHandle<R>) {
        if self.front_at.elapsed() < Duration::from_millis(200) {
            return;
        }
        self.front_at = Instant::now();
        let sessions: Vec<(String, Option<u32>)> = app
            .state::<crate::library::Running>()
            .0
            .lock()
            .map(|m| {
                let mut all: Vec<(String, Option<u32>)> = m.iter().filter(|(_, s)| !s.stopped()).map(|(id, s)| (id.clone(), s.pid())).collect();
                all.sort();
                all
            })
            .unwrap_or_default();
        if sessions.is_empty() {
            self.game = None;
            self.plan = Plan::Nothing;
            self.dirs.clear();
            self.paused = false;
            self.told = false;
            return;
        }
        if self.read_at.elapsed() > Duration::from_secs(1) || sessions.iter().any(|(id, _)| !self.dirs.contains_key(id)) {
            self.read_at = Instant::now();
            self.profiles = load(app);
            self.to_games = crate::pad::load_config(app).games;
            let folders =crate::settings::all_folders(&crate::settings::load(app));
            let games = crate::library::load(app).unwrap_or_default();
            self.dirs = sessions
                .iter()
                .filter_map(|(id, _)| {
                    let game = games.iter().find(|g| g.id == *id)?;
                    let dir = crate::storage::game_folder(&folders, &game.install_dir).unwrap_or_else(|| PathBuf::from(&game.install_dir));
                    Some((id.clone(), dir))
                })
                .collect();
        }
        let Some(game) = self.pick_game(app, &sessions) else { return };
        self.profile = self.profiles.of(&game).clone();
        let known = self.dirs.get(&game).and_then(|d| anti_cheat_known(d));
        self.plan = match (known, self.profile.mode) {
            _ if !self.to_games => Plan::Nothing,
            (_, Mode::Off) => Plan::Nothing,
            // Not looked at yet, or anti-cheat: inject nothing.
            (None, _) | (Some(Some(_)), _) => Plan::Nothing,
            (Some(None), Mode::Gamepad) => Plan::Gamepad,
            (Some(None), Mode::Keyboard) => Plan::Keyboard,
        };
        self.game = Some(game);
    }

    fn send<R: Runtime>(&mut self, app: &AppHandle<R>, events: &[Out]) {
        if events.is_empty() {
            return;
        }
        if self.output.is_none() {
            match out::Output::open() {
                Ok(o) => self.output = Some(o),
                Err(e) => return self.tell(app, &e),
            }
        }
        if let Some(o) = self.output.as_mut() {
            match o.send(events) {
                Ok(()) => self.refused = 0,
                // A moment of this is normal: Windows refuses all input under
                // an administrator prompt. Only saying so when it lasts.
                Err(e) => {
                    self.refused += 1;
                    if self.refused >= 50 {
                        self.tell(app, &e);
                    }
                }
            }
        }
    }

    /// Said once per run of a game, in a notice; the log has every time.
    pub fn tell<R: Runtime>(&mut self, app: &AppHandle<R>, why: &str) {
        crate::logging::warn("pads", why);
        if !self.told {
            self.told = true;
            let _ = app.emit("notify", crate::downloads::Notice::new("Controller", why, None));
        }
    }

    /// In controller mode, what this pad goes by: `None` when it is taken out
    /// of the game, else the remap for it, inner `None` being the pad's own
    /// from Settings.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn pad_remap(&self, key: &str) -> Option<Option<Remap>> {
        let use_it = self.profile.for_pad(key)?;
        Some([use_it, &self.profile].into_iter().find(|p| !p.remap.is_identity()).map(|p| p.remap.clone()))
    }

    /// One tick of the pads, each by its key. True when the pause chord just
    /// fired: Select and Start held together on one controller.
    pub fn feed<R: Runtime>(&mut self, app: &AppHandle<R>, mut pads: Vec<(String, PadState)>) -> bool {
        let dt = self.tick.elapsed().as_secs_f32().min(0.05);
        self.tick = Instant::now();
        let mut events = Vec::new();
        let mut fired = false;
        if self.plan == Plan::Keyboard {
            let mut chord = false;
            for (_, state) in &mut pads {
                if state.pressed.contains(&Control::Select) && state.pressed.contains(&Control::Start) {
                    // Neither reaches the game while held together.
                    state.pressed.remove(&Control::Select);
                    state.pressed.remove(&Control::Start);
                    chord = true;
                }
            }
            if chord {
                let since = *self.chord_since.get_or_insert_with(Instant::now);
                if !self.chord_done && since.elapsed() >= PAUSE_HOLD {
                    self.chord_done = true;
                    self.paused = !self.paused;
                    fired = true;
                }
            } else {
                self.chord_since = None;
                self.chord_done = false;
            }
        }
        if self.plan == Plan::Keyboard && self.in_front && !self.paused {
            let playing: Vec<(&GameProfile, &PadState)> = pads.iter().filter_map(|(key, state)| Some((self.profile.for_pad(key)?, state))).collect();
            self.mapper.step_all(&playing, dt, &mut events);
        } else {
            self.mapper.release(&mut events);
        }
        self.send(app, &events);
        if self.plan != Plan::Keyboard {
            // Nothing lingers once the game has closed (a virtual device on Linux).
            self.output = None;
        }
        fired
    }
}

/* ── What a game will see ─────────────────────────────────── */

/// A connected pad, as far as saying what a game sees of it goes.
#[derive(Clone, Debug)]
pub struct PadFacts {
    pub key: String,
    pub name: String,
    /// An Xbox pad: Windows hands it to every game as it is.
    pub xinput: bool,
    /// Its buttons are known (SDL's database, or a setup saved here).
    pub mapped: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PadLine {
    key: String,
    name: String,
    /// In plain words, what the game gets from this controller.
    sees: String,
    /// Which player, where Kryoto can tell.
    player: Option<u32>,
    /// It is taken out of this game.
    removed: bool,
    /// Kryoto can really take it out of this game.
    can_remove: bool,
    /// It has its own settings here.
    own: bool,
    /// Its own settings would reach the game.
    can_own: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
    pads: Vec<PadLine>,
    /// The whole picture, in a sentence or two.
    summary: String,
}

/// Windows games read four Xbox controllers at most.
const XINPUT_SLOTS: u32 = 4;

/// What a game gets from the controllers plugged in, with these settings.
pub fn describe(p: &GameProfile, pads: &[PadFacts], windows: bool, driver: Option<bool>) -> Seen {
    let removed = |d: &PadFacts| p.pads.get(&d.key).is_some_and(|u| u.off);
    let own = |d: &PadFacts| p.pads.get(&d.key).is_some_and(|u| u.own.is_some());
    let line = |d: &PadFacts, sees: String, player: Option<u32>, can_remove: bool, can_own: bool| PadLine {
        key: d.key.clone(),
        name: d.name.clone(),
        sees,
        player,
        removed: removed(d),
        can_remove,
        own: own(d),
        can_own,
    };
    if pads.is_empty() {
        return Seen { pads: Vec::new(), summary: "No controller is connected. Connect one to see what this game will get.".into() };
    }
    let in_play = pads.iter().filter(|d| !removed(d)).count();
    match p.mode {
        Mode::Off => Seen {
            pads: pads.iter().map(|d| line(d, "Left alone: the game reads it by itself, if it can.".into(), None, false, false)).collect(),
            summary: "Kryoto does nothing for this game. It reads whatever controllers it can by itself.".into(),
        },
        Mode::Keyboard => {
            let lines = pads
                .iter()
                .map(|d| {
                    let sees = if removed(d) {
                        "Taken out: its presses do nothing here.".to_string()
                    } else if !d.mapped {
                        "Not set up yet, so Kryoto cannot read its buttons. Set it up in Settings, Devices.".to_string()
                    } else if own(d) {
                        "Presses its own keys.".to_string()
                    } else {
                        "Presses the keys below.".to_string()
                    };
                    line(d, sees, None, true, true)
                })
                .collect();
            let summary = if in_play < 2 {
                "The game sees a keyboard and a mouse.".to_string()
            } else if pads.iter().any(|d| !removed(d) && own(d)) {
                "The game sees one keyboard and one mouse, not separate players. Each controller presses its own keys, so two people can play if the game lets two share a keyboard. The mouse is shared.".to_string()
            } else {
                "The game sees one keyboard and one mouse, not separate players: every controller presses the same keys and moves the same mouse. For a game where two people share a keyboard, give one controller its own keys.".to_string()
            };
            Seen { pads: lines, summary }
        }
        Mode::Gamepad if windows => {
            // Windows numbers the Xbox pads it has first; virtual ones follow.
            let mut next = pads.iter().filter(|d| d.xinput).count() as u32;
            let mut virtuals = 0;
            let mut xbox = 0;
            let lines: Vec<PadLine> = pads
                .iter()
                .map(|d| {
                    if d.xinput {
                        xbox += 1;
                        let sees = "The game reads it directly as an Xbox controller. Windows gives it to every game, so a remap cannot reach it and Kryoto cannot take it out.";
                        return line(d, sees.into(), Some(xbox), false, false);
                    }
                    if removed(d) {
                        return line(d, "Taken out: no virtual controller for it. A game that reads it directly still can.".into(), None, true, false);
                    }
                    if driver == Some(false) {
                        return line(d, "No ViGEmBus driver, so no virtual controller: the game reads it as it is, if it can, and remaps do not reach it.".into(), None, false, false);
                    }
                    if !d.mapped {
                        return line(d, "Not set up yet, so Kryoto cannot read its buttons. Set it up in Settings, Devices.".into(), None, false, false);
                    }
                    next += 1;
                    virtuals += 1;
                    if next > XINPUT_SLOTS {
                        return line(d, "No room: Windows games read four Xbox controllers at most.".into(), None, true, true);
                    }
                    line(d, format!("The game sees a virtual Xbox controller, player {next}."), Some(next), true, true)
                })
                .collect();
            let seen = (xbox + virtuals).min(XINPUT_SLOTS);
            let mut summary = match seen {
                0 => "The game gets no Xbox controller from Kryoto.".to_string(),
                1 => "The game sees one Xbox controller.".to_string(),
                n => format!("The game sees {n} Xbox controllers, one per player."),
            };
            if virtuals > 0 {
                summary.push_str(" A game that also reads controllers directly can get the same presses twice; if so, use Keyboard and mouse or take that controller out.");
            }
            if xbox + virtuals > XINPUT_SLOTS {
                summary.push_str(" Windows games read four Xbox controllers at most, so the rest do not reach the game.");
            }
            Seen { pads: lines, summary }
        }
        Mode::Gamepad => {
            let lines = pads
                .iter()
                .map(|d| {
                    if d.key.contains('#') {
                        // SDL's mappings and its ignore list go by model.
                        return line(d, "Same model as the one above: Linux games get one setting per model, so it follows that one.".into(), None, false, false);
                    }
                    if removed(d) {
                        return line(d, "Taken out: hidden from games that use SDL's controller support, with every controller of this model. Others can still read it.".into(), None, true, false);
                    }
                    let sees = if own(d) { "The game reads it directly, with its own remap if it uses SDL, Wine or Proton." } else { "The game reads it directly, remapped if it uses SDL, Wine or Proton." };
                    line(d, sees.into(), None, true, true)
                })
                .collect();
            Seen { pads: lines, summary: "The game reads each controller itself and decides who is player 1. Remaps reach SDL games, Wine and Proton, and take effect from the next start.".into() }
        }
    }
}

/* ── Commands ─────────────────────────────────────────────── */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameView {
    profile: GameProfile,
    /// False while the game follows the shared default.
    own: bool,
    default: GameProfile,
    /// The anti-cheat found in its folder: Kryoto injects nothing into it.
    anti_cheat: Option<&'static str>,
    /// Why keyboard and mouse cannot work here, if so.
    output_problem: Option<String>,
    run_as_admin: bool,
    templates: &'static [TemplateInfo],
    keys: &'static [KeyDef],
    /// The layouts' settings, so picking one needs no trip back.
    layouts: BTreeMap<&'static str, GameProfile>,
}

/// A game's page: looks at its folder again each time it opens.
#[tauri::command(async)]
pub fn pad_game_get(app: AppHandle, game_id: String) -> Result<GameView, String> {
    let game = crate::library::load(&app)?.into_iter().find(|g| g.id == game_id).ok_or("That game is no longer in the library.")?;
    let folders = crate::settings::all_folders(&crate::settings::load(&app));
    let dir = crate::storage::game_folder(&folders, &game.install_dir).unwrap_or_else(|| PathBuf::from(&game.install_dir));
    let anti_cheat = anti_cheat_in(&dir);
    if let Ok(mut scans) = SCANS.lock() {
        scans.get_or_insert_with(HashMap::new).insert(dir, Some(anti_cheat));
    }
    let all = load(&app);
    Ok(GameView {
        profile: all.of(&game_id).clone(),
        own: all.games.contains_key(&game_id),
        default: all.default,
        anti_cheat,
        output_problem: output_problem(),
        run_as_admin: game.run_as_admin,
        templates: TEMPLATES,
        keys: KEYS,
        layouts: TEMPLATES.iter().filter_map(|t| Some((t.id, template(t.id)?))).collect(),
    })
}

/// This game's settings, or `None` to follow the shared default again.
#[tauri::command]
pub fn pad_game_set(app: AppHandle, game_id: String, profile: Option<GameProfile>) -> Result<(), String> {
    update(&app, |p| match profile {
        Some(g) => {
            p.games.insert(game_id, g.clean());
        }
        None => {
            p.games.remove(&game_id);
        }
    })
}

/// What every game without its own settings gets.
#[tauri::command]
pub fn pad_game_default_set(app: AppHandle, profile: GameProfile) -> Result<(), String> {
    update(&app, |p| p.default = profile.clean())
}

/// What a game would see with these settings and the pads plugged in now.
#[tauri::command]
pub fn pad_game_seen(app: AppHandle, profile: GameProfile) -> Seen {
    let (pads, driver) = crate::pad::connected(&app);
    describe(&profile, &pads, cfg!(windows), driver)
}

/// The games with a controller setup of their own, for the right-click menu.
#[tauri::command]
pub fn pad_game_configured(app: AppHandle) -> Vec<String> {
    load(&app).games.into_keys().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(pressed: &[Control]) -> PadState {
        PadState { pressed: pressed.iter().copied().collect(), ..Default::default() }
    }

    fn keys(events: &[Out]) -> Vec<(&'static str, bool)> {
        let mut all: Vec<_> = events.iter().filter_map(|e| if let Out::Key(d, down) = e { Some((d.name, *down)) } else { None }).collect();
        all.sort();
        all
    }

    #[test]
    fn every_layout_only_presses_keys_that_exist() {
        for t in TEMPLATES {
            let p = template(t.id).unwrap();
            assert!(!p.binds.is_empty(), "{}", t.id);
            assert!(p.binds.values().all(|name| key(name).is_some()), "{}", t.id);
        }
        let mut names: Vec<_> = KEYS.iter().map(|k| k.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), KEYS.len());
    }

    #[test]
    fn a_button_presses_once_and_lets_go() {
        let p = template("shooter").unwrap();
        let mut m = Mapper::default();
        let mut out = Vec::new();
        m.step(&p, &state(&[Control::South]), 0.01, &mut out);
        assert_eq!(keys(&out), [("space", true)]);
        out.clear();
        m.step(&p, &state(&[Control::South]), 0.01, &mut out);
        assert!(out.is_empty());
        m.step(&p, &state(&[]), 0.01, &mut out);
        assert_eq!(keys(&out), [("space", false)]);
    }

    #[test]
    fn triggers_click_past_their_threshold() {
        let p = template("shooter").unwrap();
        let mut m = Mapper::default();
        let mut out = Vec::new();
        m.step(&p, &PadState { rt: 0.2, ..Default::default() }, 0.01, &mut out);
        assert!(out.is_empty());
        m.step(&p, &PadState { rt: 0.8, ..Default::default() }, 0.01, &mut out);
        assert_eq!(keys(&out), [("mouse1", true)]);
    }

    #[test]
    fn a_stick_as_keys_does_not_chatter_at_the_edge() {
        let p = template("shooter").unwrap();
        let mut m = Mapper::default();
        let mut out = Vec::new();
        m.step(&p, &PadState { ly: 0.6, ..Default::default() }, 0.01, &mut out);
        assert_eq!(keys(&out), [("w", true)]);
        out.clear();
        // Under the press point, over the release point: still held.
        m.step(&p, &PadState { ly: 0.4, ..Default::default() }, 0.01, &mut out);
        assert!(out.is_empty());
        m.step(&p, &PadState { ly: 0.2, ..Default::default() }, 0.01, &mut out);
        assert_eq!(keys(&out), [("w", false)]);
    }

    #[test]
    fn steering_uses_only_left_and_right() {
        let p = template("racing").unwrap();
        let mut m = Mapper::default();
        let mut out = Vec::new();
        m.step(&p, &PadState { lx: -0.9, ly: 0.9, ..Default::default() }, 0.01, &mut out);
        assert_eq!(keys(&out), [("a", true)]);
    }

    #[test]
    fn the_mouse_moves_with_the_stick_and_keeps_its_fractions() {
        let p = template("shooter").unwrap();
        let mut m = Mapper::default();
        let mut x = 0;
        let mut y = 0;
        // A second of full right, up: `sensitivity` pixels, screen y going up.
        for _ in 0..100 {
            let mut out = Vec::new();
            m.step(&p, &PadState { rx: 1.0, ..Default::default() }, 0.01, &mut out);
            for e in out {
                if let Out::Move(dx, dy) = e {
                    x += dx;
                    y += dy;
                }
            }
        }
        assert!((x - p.sensitivity as i32).abs() <= 1, "{x}");
        assert_eq!(y, 0);
        let mut out = Vec::new();
        m.step(&p, &PadState { ry: 1.0, ..Default::default() }, 0.05, &mut out);
        assert!(matches!(out[0], Out::Move(0, dy) if dy < 0));
        // Inside the deadzone nothing moves.
        let mut out = Vec::new();
        m.step(&p, &PadState { rx: 0.1, ..Default::default() }, 0.05, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn wheels_click_on_each_press_and_never_stay_held() {
        let p = template("strategy").unwrap();
        let mut m = Mapper::default();
        let mut out = Vec::new();
        m.step(&p, &state(&[Control::RightBumper]), 0.01, &mut out);
        assert_eq!(out, [Out::Wheel(1)]);
        out.clear();
        m.step(&p, &state(&[Control::RightBumper]), 0.01, &mut out);
        m.step(&p, &state(&[]), 0.01, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn release_lets_everything_go() {
        let p = template("shooter").unwrap();
        let mut m = Mapper::default();
        let mut out = Vec::new();
        m.step(&p, &state(&[Control::South, Control::West]), 0.01, &mut out);
        out.clear();
        m.release(&mut out);
        assert_eq!(keys(&out), [("r", false), ("space", false)]);
    }

    #[test]
    fn anti_cheat_is_found_by_its_files() {
        assert_eq!(anti_cheat_named("EasyAntiCheat_EOS_Setup.exe"), Some("Easy Anti-Cheat"));
        assert_eq!(anti_cheat_named("BattlEye"), Some("BattlEye"));
        assert_eq!(anti_cheat_named("BEService_x64.exe"), Some("BattlEye"));
        assert_eq!(anti_cheat_named("game.exe"), None);
        let base = std::env::temp_dir().join("kryoto-pad-anticheat");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("Game/EasyAntiCheat")).unwrap();
        std::fs::create_dir_all(base.join("Plain/bin")).unwrap();
        assert_eq!(anti_cheat_in(&base.join("Game")), Some("Easy Anti-Cheat"));
        assert_eq!(anti_cheat_in(&base.join("Plain")), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn identical_pads_are_told_apart_by_order() {
        assert_eq!(pad_keys(["a", "b", "a", "a"]), ["a", "b", "a#2", "a#3"]);
    }

    #[test]
    fn a_key_two_controllers_hold_stays_down_until_the_last_lets_go() {
        let p = template("shooter").unwrap();
        let (down, up) = (state(&[Control::South]), state(&[]));
        let mut m = Mapper::default();
        let mut out = Vec::new();
        m.step_all(&[(&p, &down), (&p, &up)], 0.01, &mut out);
        assert_eq!(keys(&out), [("space", true)]);
        out.clear();
        m.step_all(&[(&p, &down), (&p, &down)], 0.01, &mut out);
        assert!(out.is_empty(), "the second press is not a new press");
        m.step_all(&[(&p, &up), (&p, &down)], 0.01, &mut out);
        assert!(out.is_empty(), "the other controller still holds it");
        m.step_all(&[(&p, &up), (&p, &up)], 0.01, &mut out);
        assert_eq!(keys(&out), [("space", false)]);
    }

    #[test]
    fn each_controller_presses_its_own_keys() {
        let (one, two) = (template("platformer").unwrap(), template("player2").unwrap());
        let pressed = state(&[Control::South]);
        let mut m = Mapper::default();
        let mut out = Vec::new();
        m.step_all(&[(&one, &pressed), (&two, &pressed)], 0.01, &mut out);
        assert_eq!(keys(&out), [("enter", true), ("space", true)]);
        out.clear();
        let left = PadState { lx: -0.9, ..Default::default() };
        m.step_all(&[(&one, &left), (&two, &left)], 0.01, &mut out);
        assert!(keys(&out).contains(&("a", true)) && keys(&out).contains(&("left", true)));
    }

    #[test]
    fn two_sticks_on_the_mouse_add_up() {
        let p = template("shooter").unwrap();
        let right = PadState { rx: 1.0, ..Default::default() };
        let mut m = Mapper::default();
        let mut out = Vec::new();
        m.step_all(&[(&p, &right), (&p, &right)], 0.01, &mut out);
        assert_eq!(out, [Out::Move(32, 0)]);
    }

    #[test]
    fn a_controller_can_be_taken_out_or_have_its_own_settings() {
        let mut p = template("shooter").unwrap();
        let own = template("player2").unwrap();
        p.pads.insert("out".into(), PadUse { off: true, own: None });
        p.pads.insert("own".into(), PadUse { off: false, own: Some(Box::new(own.clone())) });
        assert!(p.for_pad("out").is_none());
        assert_eq!(p.for_pad("own").unwrap().template, "player2");
        assert_eq!(p.for_pad("anyone else").unwrap().template, "shooter");
        // An entry that says nothing, and settings inside settings, go.
        p.pads.insert("nothing".into(), PadUse::default());
        p.pads.get_mut("own").unwrap().own.as_mut().unwrap().pads.insert("x".into(), PadUse { off: true, own: None });
        let clean = p.clean();
        assert!(!clean.pads.contains_key("nothing"));
        assert!(clean.pads["own"].own.as_ref().unwrap().pads.is_empty());
    }

    fn pad(key: &str, xinput: bool) -> PadFacts {
        PadFacts { key: key.into(), name: key.into(), xinput, mapped: true }
    }

    #[test]
    fn it_says_what_a_windows_game_sees_of_two_controllers() {
        let mut p = GameProfile { mode: Mode::Gamepad, ..template("action").unwrap() };
        // An Xbox pad and a DualSense: Windows has the Xbox one first.
        let seen = describe(&p, &[pad("sense", false), pad("xbox", true)], true, Some(true));
        assert!(seen.summary.contains("2 Xbox controllers"), "{}", seen.summary);
        assert!(seen.summary.contains("twice"), "{}", seen.summary);
        assert_eq!(seen.pads[0].player, Some(2));
        assert_eq!(seen.pads[1].player, Some(1));
        assert!(seen.pads[0].can_own && seen.pads[0].can_remove);
        assert!(!seen.pads[1].can_own && !seen.pads[1].can_remove, "an Xbox pad cannot be hidden or remapped");
        // Without the driver nothing is virtual, and it says so.
        let seen = describe(&p, &[pad("sense", false)], true, Some(false));
        assert!(seen.pads[0].sees.contains("ViGEmBus") && !seen.pads[0].can_own);
        // Taken out.
        p.pads.insert("sense".into(), PadUse { off: true, own: None });
        let seen = describe(&p, &[pad("sense", false), pad("other", false)], true, Some(true));
        assert!(seen.pads[0].removed && seen.pads[0].player.is_none());
        assert_eq!(seen.pads[1].player, Some(1));
        assert!(seen.summary.contains("one Xbox controller"), "{}", seen.summary);
        // Five: Windows stops at four.
        let many: Vec<PadFacts> = (0..5).map(|i| pad(&format!("p{i}"), false)).collect();
        let seen = describe(&GameProfile { mode: Mode::Gamepad, ..template("action").unwrap() }, &many, true, Some(true));
        assert!(seen.summary.contains("four"), "{}", seen.summary);
        assert!(seen.pads[4].sees.contains("four"));
    }

    #[test]
    fn it_says_two_controllers_are_one_keyboard() {
        let mut p = GameProfile { mode: Mode::Keyboard, ..template("action").unwrap() };
        let pads = [pad("a", true), pad("b", false)];
        let seen = describe(&p, &pads, true, Some(true));
        assert!(seen.summary.contains("one keyboard and one mouse, not separate players"), "{}", seen.summary);
        assert!(seen.summary.contains("same keys"), "{}", seen.summary);
        assert!(seen.pads.iter().all(|l| l.can_own && l.can_remove), "keys work for any pad");
        p.pads.insert("b".into(), PadUse { off: false, own: template("player2").map(Box::new) });
        assert!(describe(&p, &pads, true, Some(true)).summary.contains("its own keys"));
        p.pads.insert("b".into(), PadUse { off: true, own: None });
        assert_eq!(describe(&p, &pads, true, Some(true)).summary, "The game sees a keyboard and a mouse.");
    }

    #[test]
    fn it_says_linux_goes_by_model() {
        let p = GameProfile { mode: Mode::Gamepad, ..template("action").unwrap() };
        let seen = describe(&p, &[pad("sense", false), pad("sense#2", false)], false, None);
        assert!(seen.pads[0].can_own && seen.pads[0].can_remove);
        assert!(!seen.pads[1].can_own && !seen.pads[1].can_remove);
        assert!(seen.pads[1].sees.contains("per model"));
        assert!(seen.summary.contains("decides who is player 1"));
    }

    #[test]
    fn with_no_controller_or_with_off_it_says_that_too() {
        let p = GameProfile { mode: Mode::Off, ..template("action").unwrap() };
        assert!(describe(&p, &[], true, None).summary.contains("No controller"));
        let seen = describe(&p, &[pad("a", false)], true, None);
        assert!(seen.summary.contains("does nothing") && !seen.pads[0].can_remove);
    }

    #[test]
    fn a_program_belongs_to_its_game_however_the_paths_are_written() {
        let exe = Path::new(r"D:\Kryoto Games\Hades\bin\Hades.exe");
        assert!(runs_from(exe, Path::new("D:/Kryoto Games/Hades")));
        assert!(runs_from(exe, Path::new(r"d:\kryoto games\hades\")));
        assert!(runs_from(exe, Path::new(r"\\?\D:\Kryoto Games\Hades")));
        // A folder that merely starts the same is another game.
        assert!(!runs_from(Path::new(r"D:\Kryoto Games\Hades II\Hades2.exe"), Path::new(r"D:\Kryoto Games\Hades")));
        assert!(!runs_from(exe, Path::new("")));
        assert!(!runs_from(Path::new(""), Path::new(r"D:\Kryoto Games\Hades")));
    }

    /// What Windows says is in front, read without touching anything.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn front_window_reads_live() {
        let (pid, exe) = out::front().expect("a window in front");
        println!("front: {pid} {}", exe.display());
        assert!(pid != 0 && exe.is_absolute());
    }

    #[test]
    fn a_game_without_its_own_settings_follows_the_default() {
        let mut all = Profiles::default();
        all.default.mode = Mode::Keyboard;
        assert_eq!(all.of("g").mode, Mode::Keyboard);
        all.games.insert("g".into(), GameProfile { mode: Mode::Off, ..Default::default() });
        assert_eq!(all.of("g").mode, Mode::Off);
        assert_eq!(all.of("other").mode, Mode::Keyboard);
    }

    #[test]
    fn saved_settings_are_cleaned_and_survive_the_file() {
        let mut p = template("action").unwrap();
        p.binds.insert(Control::South, "not-a-key".into());
        p.sensitivity = 999_999;
        p.deadzone = f32::NAN;
        let clean = p.clean();
        assert!(!clean.binds.contains_key(&Control::South));
        assert_eq!(clean.sensitivity, 4000);
        assert_eq!(clean.deadzone, 0.15);
        let json = serde_json::to_string(&clean).unwrap();
        assert_eq!(serde_json::from_str::<GameProfile>(&json).unwrap(), clean);
    }

    /// Windows' SendInput, end to end and invisibly: a low-level hook in this
    /// test sees each event as injected and swallows it, so nothing reaches
    /// the desktop. `cargo test --lib pad_games -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn sendinput_reaches_windows_live() {
        use std::sync::Mutex;
        use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        static SEEN: Mutex<Vec<(u32, u32, u32)>> = Mutex::new(Vec::new());
        unsafe extern "system" fn keys_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
            let e = unsafe { &*(l as *const KBDLLHOOKSTRUCT) };
            if code >= 0 && e.flags & LLKHF_INJECTED != 0 {
                SEEN.lock().unwrap().push((w as u32, e.scanCode, e.flags & LLKHF_EXTENDED));
                return 1;
            }
            unsafe { CallNextHookEx(std::ptr::null_mut(), code, w, l) }
        }
        unsafe extern "system" fn mouse_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
            let e = unsafe { &*(l as *const MSLLHOOKSTRUCT) };
            if code >= 0 && e.flags & LLMHF_INJECTED != 0 {
                SEEN.lock().unwrap().push((w as u32, e.mouseData >> 16, 0));
                return 1;
            }
            unsafe { CallNextHookEx(std::ptr::null_mut(), code, w, l) }
        }
        unsafe {
            let hooks = [
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(keys_hook), std::ptr::null_mut(), 0),
                SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), std::ptr::null_mut(), 0),
            ];
            assert!(hooks.iter().all(|h| !h.is_null()));
            let w_key = key("w").unwrap();
            let up = key("up").unwrap();
            let click = key("mouse1").unwrap();
            let events = [Out::Key(w_key, true), Out::Key(w_key, false), Out::Key(up, true), Out::Key(up, false), Out::Key(click, true), Out::Key(click, false), Out::Wheel(1)];
            // The hooks run on this thread's message loop, so send from another.
            let sender = std::thread::spawn(move || out::Output::open().unwrap().send(&events));
            let until = Instant::now() + Duration::from_secs(3);
            let mut msg: MSG = std::mem::zeroed();
            while Instant::now() < until && SEEN.lock().unwrap().len() < 7 {
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {}
                std::thread::sleep(Duration::from_millis(5));
            }
            sender.join().unwrap().unwrap();
            for h in hooks {
                UnhookWindowsHookEx(h);
            }
        }
        let seen = SEEN.lock().unwrap().clone();
        println!("{seen:?}");
        assert_eq!(seen[0], (WM_KEYDOWN, 17, 0));
        assert_eq!(seen[1], (WM_KEYUP, 17, 0));
        assert_eq!(seen[2], (WM_KEYDOWN, 0x48, LLKHF_EXTENDED));
        assert_eq!(seen[4].0, WM_LBUTTONDOWN);
        assert_eq!(seen[5].0, WM_LBUTTONUP);
        assert_eq!(seen[6], (WM_MOUSEWHEEL, 120, 0));
    }

    /// The whole path on Windows without a controller: this test plugs in a
    /// virtual Xbox pad (ViGEmBus), which a running dev app reads as a real one
    /// and turns into keys, and a low-level hook here catches and swallows them.
    /// Start the dev app first, with `KRYOTO_DEV_PAD_FRONT=1`, `pad_start` called,
    /// a game running and set to keyboard and mouse with the shooter layout.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn virtual_pad_becomes_keys_live() {
        use std::sync::Mutex;
        use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};
        use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
        unsafe extern "system" fn keys_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
            let e = unsafe { &*(l as *const KBDLLHOOKSTRUCT) };
            if code >= 0 && e.flags & LLKHF_INJECTED != 0 {
                SEEN.lock().unwrap().push(format!("key {} {}", e.scanCode, if w as u32 == WM_KEYDOWN { "down" } else { "up" }));
                return 1;
            }
            unsafe { CallNextHookEx(std::ptr::null_mut(), code, w, l) }
        }
        unsafe extern "system" fn mouse_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
            let e = unsafe { &*(l as *const MSLLHOOKSTRUCT) };
            if code >= 0 && e.flags & LLMHF_INJECTED != 0 {
                SEEN.lock().unwrap().push(match w as u32 {
                    WM_MOUSEMOVE => "move".to_string(),
                    WM_LBUTTONDOWN => "left down".to_string(),
                    WM_LBUTTONUP => "left up".to_string(),
                    other => format!("mouse {other}"),
                });
                return 1;
            }
            unsafe { CallNextHookEx(std::ptr::null_mut(), code, w, l) }
        }
        let pump = |ms: u64| unsafe {
            let until = Instant::now() + Duration::from_millis(ms);
            let mut msg: MSG = std::mem::zeroed();
            while Instant::now() < until {
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {}
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        let hooks = unsafe { [SetWindowsHookExW(WH_KEYBOARD_LL, Some(keys_hook), std::ptr::null_mut(), 0), SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), std::ptr::null_mut(), 0)] };
        let mut pad = Xbox360Wired::new(Client::connect().expect("ViGEmBus"), TargetId::XBOX360_WIRED);
        pad.plugin().unwrap();
        pad.wait_ready().unwrap();
        pump(3500); // the dev app scans for new pads every two seconds
        let mut push = |report: XGamepad, ms: u64| {
            pad.update(&report).unwrap();
            pump(ms);
        };
        push(XGamepad { buttons: XButtons { raw: XButtons::A }, ..Default::default() }, 300);
        push(XGamepad::default(), 300);
        push(XGamepad { right_trigger: 255, ..Default::default() }, 300);
        push(XGamepad::default(), 300);
        push(XGamepad { thumb_ly: 32000, ..Default::default() }, 300);
        push(XGamepad::default(), 300);
        push(XGamepad { thumb_rx: 32000, ..Default::default() }, 400);
        push(XGamepad::default(), 300);
        pad.unplug().unwrap();
        for h in hooks {
            unsafe { UnhookWindowsHookEx(h) };
        }
        let seen = SEEN.lock().unwrap().clone();
        let moves = seen.iter().filter(|s| *s == "move").count();
        let rest: Vec<&String> = seen.iter().filter(|s| *s != "move").collect();
        println!("events {rest:?}, moves {moves}");
        for want in ["key 57 down", "key 57 up", "left down", "left up", "key 17 down", "key 17 up"] {
            assert!(seen.iter().any(|s| s == want), "{want} never came");
        }
        assert!(moves > 5, "the mouse barely moved: {moves}");
    }

    /// Two pads at once, keyboard and mouse, on Windows: prints what Kryoto sent
    /// for each step, swallowed by a hook as above. Dev app as for
    /// `virtual_pad_becomes_keys_live`; the game's settings are the test.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn two_pads_become_keys_live() {
        use std::sync::Mutex;
        use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};
        use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
        unsafe extern "system" fn keys_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
            let e = unsafe { &*(l as *const KBDLLHOOKSTRUCT) };
            if code >= 0 && e.flags & LLKHF_INJECTED != 0 && e.dwExtraInfo == SIGNATURE {
                let name = KEYS.iter().find(|d| matches!(d.kind, KeyKind::Key { scan, .. } if u32::from(scan) == e.scanCode)).map_or("?", |d| d.name);
                SEEN.lock().unwrap().push(format!("{name} {}", if w as u32 == WM_KEYDOWN { "down" } else { "up" }));
                return 1;
            }
            unsafe { CallNextHookEx(std::ptr::null_mut(), code, w, l) }
        }
        unsafe extern "system" fn mouse_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
            let e = unsafe { &*(l as *const MSLLHOOKSTRUCT) };
            if code >= 0 && e.flags & LLMHF_INJECTED != 0 && e.dwExtraInfo == SIGNATURE {
                SEEN.lock().unwrap().push(if w as u32 == WM_MOUSEMOVE { "move".to_string() } else { format!("mouse {w}") });
                return 1;
            }
            unsafe { CallNextHookEx(std::ptr::null_mut(), code, w, l) }
        }
        let pump = |ms: u64| unsafe {
            let until = Instant::now() + Duration::from_millis(ms);
            let mut msg: MSG = std::mem::zeroed();
            while Instant::now() < until {
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {}
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        let told = |what: &str| {
            let events = std::mem::take(&mut *SEEN.lock().unwrap());
            let moves = events.iter().filter(|s| *s == "move").count();
            let rest: Vec<&String> = events.iter().filter(|s| *s != "move").collect();
            println!("{what}: {rest:?}{}", if moves > 0 { format!(", {moves} mouse moves") } else { String::new() });
        };
        let hooks = unsafe { [SetWindowsHookExW(WH_KEYBOARD_LL, Some(keys_hook), std::ptr::null_mut(), 0), SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), std::ptr::null_mut(), 0)] };
        let mut one = Xbox360Wired::new(Client::connect().expect("ViGEmBus"), TargetId::XBOX360_WIRED);
        one.plugin().unwrap();
        one.wait_ready().unwrap();
        pump(400);
        let mut two = Xbox360Wired::new(Client::connect().unwrap(), TargetId::XBOX360_WIRED);
        two.plugin().unwrap();
        two.wait_ready().unwrap();
        pump(7000); // the dev app scans every two seconds, and the settings go in now
        let press = |raw: u16| XGamepad { buttons: XButtons { raw }, ..Default::default() };
        let rest = XGamepad::default();
        one.update(&press(XButtons::A)).unwrap();
        pump(250);
        one.update(&rest).unwrap();
        pump(250);
        told("pad 1 presses A");
        two.update(&press(XButtons::A)).unwrap();
        pump(250);
        two.update(&rest).unwrap();
        pump(250);
        told("pad 2 presses A");
        one.update(&press(XButtons::A)).unwrap();
        pump(200);
        two.update(&press(XButtons::A)).unwrap();
        pump(200);
        one.update(&rest).unwrap();
        pump(300);
        told("both hold A, pad 1 lets go");
        two.update(&rest).unwrap();
        pump(250);
        told("pad 2 lets go");
        one.update(&XGamepad { thumb_rx: 32000, ..Default::default() }).unwrap();
        pump(300);
        one.update(&rest).unwrap();
        pump(150);
        told("pad 1 right stick");
        two.update(&XGamepad { thumb_rx: 32000, ..Default::default() }).unwrap();
        pump(300);
        two.update(&rest).unwrap();
        pump(150);
        told("pad 2 right stick");
        one.update(&press(XButtons::BACK)).unwrap();
        two.update(&press(XButtons::START)).unwrap();
        pump(900);
        one.update(&rest).unwrap();
        two.update(&rest).unwrap();
        pump(200);
        told("Back on pad 1, Start on pad 2, held (must not pause)");
        one.update(&press(XButtons::A)).unwrap();
        pump(200);
        one.update(&rest).unwrap();
        pump(200);
        told("pad 1 presses A (still working)");
        two.update(&press(XButtons::BACK | XButtons::START)).unwrap();
        pump(900);
        two.update(&rest).unwrap();
        pump(200);
        told("Back and Start on pad 2, held (pauses)");
        one.update(&press(XButtons::A)).unwrap();
        pump(200);
        one.update(&rest).unwrap();
        pump(200);
        told("pad 1 presses A (paused: nothing)");
        two.update(&press(XButtons::BACK | XButtons::START)).unwrap();
        pump(900);
        two.update(&rest).unwrap();
        pump(200);
        told("Back and Start on pad 2 again (starts again)");
        one.update(&press(XButtons::A)).unwrap();
        pump(200);
        one.update(&rest).unwrap();
        pump(200);
        told("pad 1 presses A (working again)");
        one.unplug().unwrap();
        two.unplug().unwrap();
        for h in hooks {
            unsafe { UnhookWindowsHookEx(h) };
        }
    }

    /// Two pads at once in controller mode on Windows, as a game sees them: two
    /// virtual DualShock 4s, which the dev app turns into virtual Xbox pads, read
    /// back through XInput, the way a game reads them. Prints what each press
    /// did on each XInput slot.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn two_pads_reach_the_game_live() {
        use vigem_client::{Client, DualShock4Wired, TargetId};
        #[repr(C)]
        #[derive(Default)]
        struct State {
            packet: u32,
            buttons: u16,
            triggers: [u8; 2],
            thumbs: [i16; 4],
        }
        #[link(name = "xinput")]
        extern "system" {
            fn XInputGetState(user: u32, state: *mut State) -> u32;
        }
        let slots = || -> Vec<(u32, u16)> {
            (0..4)
                .filter_map(|i| {
                    let mut s = State::default();
                    (unsafe { XInputGetState(i, &mut s) } == 0).then_some((i, s.buttons))
                })
                .collect()
        };
        let name = |b: u16| match b {
            0 => "nothing".to_string(),
            0x1000 => "A".to_string(),
            0x2000 => "B".to_string(),
            0x4000 => "X".to_string(),
            0x8000 => "Y".to_string(),
            other => format!("{other:#x}"),
        };
        let before: Vec<u32> = slots().into_iter().map(|(i, _)| i).collect();
        println!("xinput slots before: {before:?}");

        // The crate's DS4 report is not the size this driver takes, so it is
        // sent by hand: Size, SerialNo, then the nine bytes of DS4_REPORT.
        use std::ffi::c_void;
        #[repr(C)]
        struct Overlapped {
            internal: [usize; 2],
            offset: [u32; 2],
            event: *mut c_void,
        }
        extern "system" {
            fn DeviceIoControl(device: *mut c_void, code: u32, input: *mut c_void, in_size: u32, output: *mut c_void, out_size: u32, returned: *mut u32, overlapped: *mut Overlapped) -> i32;
            fn GetOverlappedResult(device: *mut c_void, overlapped: *mut Overlapped, transferred: *mut u32, wait: i32) -> i32;
            fn CreateEventW(attributes: *mut c_void, manual: i32, initial: i32, name: *const u16) -> *mut c_void;
            fn CloseHandle(handle: *mut c_void) -> i32;
            fn GetLastError() -> u32;
        }
        const IOCTL_DS4_SUBMIT_REPORT: u32 = 0x2AA80C;
        const SQUARE_NONE: u16 = 8; // the d-pad's nibble at rest
        const CROSS: u16 = 0x20;
        let send = |client: &Client, size: u32, serial: u32, buttons: u16| -> Result<(), u32> {
            let mut report = [0u8; 24];
            report[0..4].copy_from_slice(&size.to_le_bytes());
            report[4..8].copy_from_slice(&serial.to_le_bytes());
            report[8..12].copy_from_slice(&[0x80; 4]);
            report[12..14].copy_from_slice(&buttons.to_le_bytes());
            unsafe {
                // `Client` holds just the driver's handle.
                let device = *(client as *const Client as *const *mut c_void);
                let event = CreateEventW(std::ptr::null_mut(), 0, 0, std::ptr::null());
                let mut overlapped = Overlapped { internal: [0; 2], offset: [0; 2], event };
                let mut moved = 0;
                DeviceIoControl(device, IOCTL_DS4_SUBMIT_REPORT, report.as_mut_ptr().cast(), size, std::ptr::null_mut(), 0, &mut moved, &mut overlapped);
                let done = GetOverlappedResult(device, &mut overlapped, &mut moved, 1);
                let error = GetLastError();
                CloseHandle(event);
                if done != 0 {
                    Ok(())
                } else {
                    Err(error)
                }
            }
        };
        let mut one = DualShock4Wired::new(Client::connect().expect("ViGEmBus"), TargetId::DUALSHOCK4_WIRED);
        one.plugin().unwrap();
        one.wait_ready().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let mut two = DualShock4Wired::new(Client::connect().unwrap(), TargetId::DUALSHOCK4_WIRED);
        two.plugin().unwrap();
        two.wait_ready().unwrap();
        std::thread::sleep(Duration::from_millis(7000));
        // Which serials are ours, once they have settled: the driver takes
        // reports only for this process's pads, and 20 bytes is what it wants.
        const SIZE: u32 = 20;
        let ours: Vec<u32> = (1..=16).filter(|s| send(one.client(), SIZE, *s, SQUARE_NONE).is_ok()).collect();
        println!("driver serials: {ours:?}");
        let &[first, second, ..] = ours.as_slice() else { panic!("no serials for both pads") };
        let added = |all: &[(u32, u16)]| -> Vec<String> { all.iter().filter(|(i, _)| !before.contains(i)).map(|(i, b)| format!("slot {i}: {}", name(*b))).collect() };
        println!("new xinput slots at rest: {:?}", added(&slots()));
        send(one.client(), SIZE, first, SQUARE_NONE | CROSS).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        println!("Cross on pad 1: {:?}", added(&slots()));
        send(one.client(), SIZE, first, SQUARE_NONE).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        send(two.client(), SIZE, second, SQUARE_NONE | CROSS).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        println!("Cross on pad 2: {:?}", added(&slots()));
        send(two.client(), SIZE, second, SQUARE_NONE).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        one.unplug().unwrap();
        two.unplug().unwrap();
    }

    /// Linux's uinput, end to end: makes the device and reads its events back
    /// from evdev. Needs write access to /dev/uinput and read access to
    /// /dev/input (root in WSL).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn uinput_reaches_linux_live() {
        use std::io::Read;
        let mut output = out::Output::open().expect("uinput");
        std::thread::sleep(Duration::from_millis(400));
        let list = std::fs::read_to_string("/proc/bus/input/devices").unwrap();
        let block = list.split("\n\n").find(|b| b.contains(out::NAME)).expect("the device is listed");
        println!("{block}");
        let event = block.split_whitespace().find(|w| w.starts_with("event")).expect("an event node");
        let mut node = std::fs::File::open(format!("/dev/input/{event}")).unwrap();
        let w_key = key("w").unwrap();
        let click = key("mouse1").unwrap();
        output.send(&[Out::Key(w_key, true), Out::Key(w_key, false), Out::Key(click, true), Out::Key(click, false), Out::Move(5, -3), Out::Wheel(1)]).unwrap();
        let mut seen = Vec::new();
        let mut buffer = [0u8; 24];
        while seen.len() < 7 {
            node.read_exact(&mut buffer).unwrap();
            let kind = u16::from_ne_bytes([buffer[16], buffer[17]]);
            let code = u16::from_ne_bytes([buffer[18], buffer[19]]);
            let value = i32::from_ne_bytes([buffer[20], buffer[21], buffer[22], buffer[23]]);
            if kind != 0 && kind != 4 {
                seen.push((kind, code, value));
            }
        }
        println!("{seen:?}");
        assert_eq!(seen, [(1, 17, 1), (1, 17, 0), (1, 0x110, 1), (1, 0x110, 0), (2, 0, 5), (2, 1, -3), (2, 8, 1)]);
    }

    /// Two virtual gamepads on Linux, read through SDL with the environment
    /// Kryoto gives a game: what really reaches a game for two different
    /// models, one taken out, and two identical ones; then keyboard and mouse
    /// with each pad on its own keys. Root in WSL: it makes uinput devices.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn two_pads_on_linux_live() {
        use sdl2::controller::Button;
        use std::fs::{File, OpenOptions};
        use std::io::{Read, Write};
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;

        const BTN_SOUTH: u16 = 0x130;
        const BUTTONS: [u16; 11] = [0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x13a, 0x13b, 0x13c, 0x13d, 0x13e];
        const AXES: [u16; 4] = [0, 1, 3, 4];

        #[repr(C)]
        struct Setup {
            bustype: u16,
            vendor: u16,
            product: u16,
            version: u16,
            name: [u8; 80],
            ff_effects_max: u32,
        }
        #[repr(C)]
        struct AbsSetup {
            code: u16,
            value: i32,
            minimum: i32,
            maximum: i32,
            fuzz: i32,
            flat: i32,
            resolution: i32,
        }
        struct VPad(File);
        impl VPad {
            fn new(name: &str, product: u16) -> Self {
                let file = OpenOptions::new().write(true).open("/dev/uinput").expect("root, for /dev/uinput");
                let fd = file.as_raw_fd();
                unsafe {
                    libc::ioctl(fd, 0x4004_5564, 1); // EV_KEY
                    libc::ioctl(fd, 0x4004_5564, 3); // EV_ABS
                    for b in BUTTONS {
                        libc::ioctl(fd, 0x4004_5565, libc::c_int::from(b));
                    }
                    for a in AXES {
                        libc::ioctl(fd, 0x4004_5567, libc::c_int::from(a));
                    }
                    let mut setup = Setup { bustype: 0x03, vendor: 0x4b52, product, version: 1, name: [0; 80], ff_effects_max: 0 };
                    setup.name[..name.len()].copy_from_slice(name.as_bytes());
                    assert!(libc::ioctl(fd, 0x405c_5503, &setup as *const Setup) >= 0);
                    for a in AXES {
                        let abs = AbsSetup { code: a, value: 0, minimum: -32768, maximum: 32767, fuzz: 0, flat: 0, resolution: 0 };
                        assert!(libc::ioctl(fd, 0x401c_5504, &abs as *const AbsSetup) >= 0);
                    }
                    assert!(libc::ioctl(fd, 0x5501) >= 0);
                }
                Self(file)
            }
            fn button(&mut self, code: u16, down: bool) {
                for (kind, code, value) in [(1u16, code, i32::from(down)), (0, 0, 0)] {
                    let mut e = [0u8; 24];
                    e[16..18].copy_from_slice(&kind.to_ne_bytes());
                    e[18..20].copy_from_slice(&code.to_ne_bytes());
                    e[20..24].copy_from_slice(&value.to_ne_bytes());
                    self.0.write_all(&e).unwrap();
                }
            }
        }
        impl Drop for VPad {
            fn drop(&mut self) {
                unsafe { libc::ioctl(self.0.as_raw_fd(), 0x5502) };
            }
        }
        fn settle() {
            std::thread::sleep(Duration::from_millis(500));
        }
        fn clear_env() {
            for name in ["SDL_GAMECONTROLLERCONFIG", "SDL_JOYSTICK_HIDAPI", "PROTON_PREFER_SDL", "SDL_GAMECONTROLLER_IGNORE_DEVICES"] {
                std::env::remove_var(name);
            }
        }
        fn is_ours(js: &sdl2::JoystickSubsystem, i: u32) -> bool {
            js.name_for_index(i).is_ok_and(|n| n.starts_with("KryotoTestPad"))
        }
        /// SDL's own view of our pads: GUID and mapping, as `pad.rs` keeps them.
        fn learn() -> Vec<(String, String)> {
            clear_env();
            let sdl = sdl2::init().unwrap();
            let js = sdl.joystick().unwrap();
            let gc = sdl.game_controller().unwrap();
            (0..js.num_joysticks().unwrap()).filter(|i| is_ours(&js, *i)).map(|i| (js.device_guid(i).unwrap().string(), gc.open(i).expect("SDL maps it").mapping())).collect()
        }
        /// With `env` set as for a game: each pad's press of its bottom button, as the game's SDL reports it.
        fn as_a_game(pads: &mut [VPad], env: &[(String, String)]) {
            clear_env();
            for (k, v) in env {
                std::env::set_var(k, v);
            }
            let sdl = sdl2::init().unwrap();
            let js = sdl.joystick().unwrap();
            let gc = sdl.game_controller().unwrap();
            let ours: Vec<u32> = (0..js.num_joysticks().unwrap()).filter(|i| is_ours(&js, *i)).collect();
            let opened: Vec<Option<sdl2::controller::GameController>> = ours.iter().map(|i| if gc.is_game_controller(*i) { gc.open(*i).ok() } else { None }).collect();
            println!("  joysticks {}, controllers the game can use {}", ours.len(), opened.iter().flatten().count());
            for (n, pad) in pads.iter_mut().enumerate() {
                pad.button(BTN_SOUTH, true);
                std::thread::sleep(Duration::from_millis(120));
                gc.update();
                let got: Vec<String> = opened
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| {
                        let c = c.as_ref()?;
                        [(Button::A, "A"), (Button::B, "B")].into_iter().find(|(b, _)| c.button(*b)).map(|(_, name)| format!("controller {} says {name}", i + 1))
                    })
                    .collect();
                pad.button(BTN_SOUTH, false);
                std::thread::sleep(Duration::from_millis(120));
                gc.update();
                println!("  pad {} presses its bottom button: {got:?}", n + 1);
            }
            clear_env();
        }
        fn prefs(learned: &[(String, String)]) -> BTreeMap<String, crate::pad::PadPrefs> {
            learned.iter().map(|(guid, mapping)| (guid.clone(), crate::pad::PadPrefs { base: Some(mapping.clone()), ..Default::default() })).collect()
        }
        let own_swap = || {
            let mut remap = Remap::new();
            remap.assign(Control::South, Control::East);
            PadUse { off: false, own: Some(Box::new(GameProfile { remap, ..template("action").unwrap() })) }
        };

        println!("--- two different models, the first with its own remap");
        let mut pads = [VPad::new("KryotoTestPad One", 1), VPad::new("KryotoTestPad Two", 2)];
        settle();
        let learned = learn();
        assert_eq!(learned.len(), 2, "{learned:?}");
        assert_ne!(learned[0].0, learned[1].0);
        let mut p = template("action").unwrap();
        p.pads.insert(learned[0].0.clone(), own_swap());
        let env = crate::pad::sdl_env(&prefs(&learned), &p);
        println!("  env: {:?}", env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>());
        as_a_game(&mut pads, &env);

        println!("--- the first model taken out of the game");
        let mut p = template("action").unwrap();
        p.pads.insert(learned[0].0.clone(), PadUse { off: true, own: None });
        let env = crate::pad::sdl_env(&prefs(&learned), &p);
        println!("  env: {env:?}");
        as_a_game(&mut pads, &env);
        drop(pads);
        settle();

        println!("--- two identical pads, a remap on the first");
        let mut pads = [VPad::new("KryotoTestPad Same", 3), VPad::new("KryotoTestPad Same", 3)];
        settle();
        let learned = learn();
        assert_eq!(learned.len(), 2, "{learned:?}");
        println!("  same GUID: {}", learned[0].0 == learned[1].0);
        let mut p = template("action").unwrap();
        p.pads.insert(learned[0].0.clone(), own_swap());
        let env = crate::pad::sdl_env(&prefs(&learned), &p);
        as_a_game(&mut pads, &env);

        println!("--- keyboard and mouse: identical pads, the second on its own keys");
        let keys_of = pad_keys(learned.iter().map(|(g, _)| g.as_str()));
        println!("  keys: {keys_of:?}");
        let mut p = template("platformer").unwrap();
        p.pads.insert(keys_of[1].clone(), PadUse { off: false, own: template("player2").map(Box::new) });
        let mut output = out::Output::open().expect("uinput");
        settle();
        let list = std::fs::read_to_string("/proc/bus/input/devices").unwrap();
        let block = list.split("\n\n").find(|b| b.contains(out::NAME)).expect("the device is listed");
        let event = block.split_whitespace().find(|w| w.starts_with("event")).expect("an event node");
        let mut node = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(format!("/dev/input/{event}")).unwrap();
        let sdl = sdl2::init().unwrap();
        let js = sdl.joystick().unwrap();
        let gc = sdl.game_controller().unwrap();
        let ours: Vec<u32> = (0..js.num_joysticks().unwrap()).filter(|i| is_ours(&js, *i)).collect();
        let opened: Vec<_> = ours.iter().map(|i| gc.open(*i).unwrap()).collect();
        let mut mapper = Mapper::default();
        for (n, pad) in pads.iter_mut().enumerate().take(2) {
            pad.button(BTN_SOUTH, true);
            std::thread::sleep(Duration::from_millis(120));
            gc.update();
            let states: Vec<(String, PadState)> = opened
                .iter()
                .zip(&keys_of)
                .map(|(c, key)| (key.clone(), PadState { pressed: if c.button(Button::A) { [Control::South].into_iter().collect() } else { HashSet::new() }, ..Default::default() }))
                .collect();
            let playing: Vec<(&GameProfile, &PadState)> = states.iter().filter_map(|(k, s)| Some((p.for_pad(k)?, s))).collect();
            let mut events = Vec::new();
            mapper.step_all(&playing, 0.01, &mut events);
            println!("  pad {} presses its bottom button: sends {:?}", n + 1, keys(&events));
            output.send(&events).unwrap();
            pad.button(BTN_SOUTH, false);
            std::thread::sleep(Duration::from_millis(120));
            gc.update();
            let mut events = Vec::new();
            mapper.step_all(&[], 0.01, &mut events);
            output.send(&events).unwrap();
        }
        std::thread::sleep(Duration::from_millis(100));
        let mut buffer = [0u8; 24];
        let mut seen = Vec::new();
        while node.read_exact(&mut buffer).is_ok() {
            let kind = u16::from_ne_bytes([buffer[16], buffer[17]]);
            let code = u16::from_ne_bytes([buffer[18], buffer[19]]);
            let value = i32::from_ne_bytes([buffer[20], buffer[21], buffer[22], buffer[23]]);
            if kind == 1 {
                seen.push((code, value));
            }
        }
        println!("  the virtual keyboard sent (key, down): {seen:?}");
        assert_eq!(seen, [(57, 1), (57, 0), (28, 1), (28, 0)], "space, then enter");
    }
}
