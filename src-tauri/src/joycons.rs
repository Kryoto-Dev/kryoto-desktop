//! Two Joy-Cons as one controller (suggestion #79).
//!
//! A Nintendo Switch Joy-Con (L) and (R) pair over Bluetooth as two separate
//! pads (Nintendo 0x057E, products 0x2006 and 0x2007; the Charging Grip over
//! USB is 0x200E and names its halves "(L)" and "(R)"). When both are
//! connected, Settings > Controller offers to use them as one controller, and
//! remembers the answer for that pair (by the halves' serials, their Bluetooth
//! addresses, when the system gives them; else by model).
//!
//! Who does the combining depends on how SDL reads the halves:
//!
//! - **SDL's own HIDAPI driver** (Windows always; Linux when the hidraw
//!   devices are readable, as Steam's udev rules make them): SDL combines a
//!   pair itself, with its stick calibration, into one "Joy-Con (L/R)" pad
//!   (product 0x2008). Kryoto switches that on and off with
//!   `SDL_JOYSTICK_HIDAPI_COMBINE_JOY_CONS`, starting with it off so the
//!   halves can be seen and offered first ([`sdl_combine`]).
//! - **The Linux kernel's hid-nintendo driver** (evdev, no hidraw access):
//!   SDL lists the halves apart and maps each one sideways. Kryoto reads their
//!   raw buttons by the layout hid-nintendo gives them ([`half_from_raw`]) and
//!   merges the two into one pad for its own navigation and keyboard mode
//!   ([`merge`]).
//! - joycond's combined device (Linux, bus 0x06) is one pad already: SDL's
//!   database maps it ([`is_pair_device`]).
//!
//! Games get the pair as one controller where they can: on Windows through the
//! virtual Xbox controller every non-Xbox pad gets (`pad.rs`, `virt`), on
//! Linux through SDL's own combining switched on in the game's environment
//! ([`game_env`]), which SDL games, Wine and Proton follow. Other games read
//! the two halves themselves and see two controllers. Kryoto installs no
//! driver for this.
//!
//! The IMU is not read, rumble is left to each half (Kryoto's haptics buzz the
//! left one), and a pair's battery is its lower half's.

use padmap::Control;
use serde::Serialize;

pub const NINTENDO: u16 = 0x057e;
pub const LEFT: u16 = 0x2006;
pub const RIGHT: u16 = 0x2007;
pub const GRIP: u16 = 0x200e;
/// What SDL (and joycond) call a combined pair.
pub const PAIR: u16 = 0x2008;

/// The GUID Kryoto gives a pair it merges itself: SDL's layout (bus, CRC,
/// vendor, product, version, driver), Nintendo 0x2008, driver `k`. Settings
/// for the pair (layout, button changes) are kept under it like any pad's, and
/// a game told to leave it out hides SDL's own combined pair (same ids).
pub const MERGED_GUID: &str = "000000007e0500000820000000006b00";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

/// Which half of a pair this pad is, if it is one.
pub fn side_of(vendor: Option<u16>, product: Option<u16>, name: &str) -> Option<Side> {
    if vendor != Some(NINTENDO) {
        return None;
    }
    match product? {
        LEFT => Some(Side::Left),
        RIGHT => Some(Side::Right),
        GRIP if name.contains("(L)") => Some(Side::Left),
        GRIP if name.contains("(R)") => Some(Side::Right),
        _ => None,
    }
}

/// A pair that is one pad already: SDL's combined Joy-Cons, or joycond's.
pub fn is_pair_device(vendor: Option<u16>, product: Option<u16>, name: &str) -> bool {
    vendor == Some(NINTENDO) && (product == Some(PAIR) || (product.is_some_and(|p| p == LEFT || p == RIGHT) && name.contains("(L/R)")))
}

/// How a pair of halves can become one controller here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Way {
    /// SDL's HIDAPI driver combines them.
    Sdl,
    /// Kryoto merges them (hid-nintendo halves on Linux).
    Kryoto,
    /// Read some other way, with buttons Kryoto cannot tell apart.
    None,
}

/// SDL's GUID says which of its drivers reads a pad: byte 14, `h` for HIDAPI.
fn hidapi(guid: &str) -> bool {
    guid.get(28..30) == Some("68")
}

/// The hid-nintendo layout [`half_from_raw`] reads: evdev (no driver byte), the
/// version the kernel driver reports (0x8001, as in SDL's database), and enough
/// buttons and axes for the table.
fn evdev_layout(side: Side, guid: &str, path: &str, buttons: u32, axes: u32) -> bool {
    let wanted = match side {
        Side::Left => LEFT_RAW.iter().map(|(i, _)| *i).max(),
        Side::Right => RIGHT_RAW.iter().map(|(i, _)| *i).max(),
    }
    .unwrap_or(0);
    path.starts_with("/dev/input/event") && guid.get(24..28) == Some("0180") && guid.get(28..32) == Some("0000") && buttons > wanted && axes >= 2
}

/// How this half would join a pair.
pub fn way_of(side: Side, guid: &str, path: &str, buttons: u32, axes: u32) -> Way {
    if hidapi(guid) {
        Way::Sdl
    } else if evdev_layout(side, guid, path, buttons, axes) {
        Way::Kryoto
    } else {
        Way::None
    }
}

/// How two halves can be one: both read the same way.
pub fn pair_way(left: Way, right: Way) -> Way {
    if left == right { left } else { Way::None }
}

/// One half, as far as telling pairs apart goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HalfId {
    pub guid: String,
    /// Its serial (the Bluetooth address over Bluetooth); empty when the system
    /// does not say.
    pub serial: String,
}

impl HalfId {
    fn ident(&self) -> String {
        let serial = self.serial.trim().to_lowercase();
        if serial.is_empty() { self.guid.to_lowercase() } else { serial }
    }
}

/// The key a pair's choice is kept under: these two halves, or, when the
/// system gives no serials, these two models.
pub fn pair_key(left: &HalfId, right: &HalfId) -> String {
    format!("{}+{}", left.ident(), right.ident())
}

/// The pairs among the halves connected, by SDL instance id: the first left
/// with the first right, and so on, in connection order (as SDL pairs them).
pub fn find_pairs(halves: &[(u32, Side)]) -> Vec<(u32, u32)> {
    let mut sorted = halves.to_vec();
    sorted.sort_by_key(|(id, _)| *id);
    let lefts = sorted.iter().filter(|(_, s)| *s == Side::Left).map(|(id, _)| *id);
    let rights = sorted.iter().filter(|(_, s)| *s == Side::Right).map(|(id, _)| *id);
    lefts.zip(rights).collect()
}

/// Whether SDL's combining should be on: a pair SDL reads is to be one, or the
/// pair it combined still is. Off otherwise, so a newly connected pair shows
/// as two halves and is offered first.
pub fn sdl_combine(sdl_pair_wanted: bool, combined_still_wanted: bool) -> bool {
    sdl_pair_wanted || combined_still_wanted
}

/* ── The merge ─────────────────────────────────────────────── */

/// A physical control on one half, held upright.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    // Left half.
    Up,
    Down,
    Left,
    Right,
    L,
    Zl,
    Minus,
    Capture,
    LeftStick,
    /// The small buttons on the rail, on each half.
    LeftSl,
    LeftSr,
    // Right half.
    A,
    B,
    X,
    Y,
    R,
    Zr,
    Plus,
    Home,
    RightStick,
    RightSl,
    RightSr,
}

impl Part {
    pub fn side(self) -> Side {
        use Part::*;
        match self {
            Up | Down | Left | Right | L | Zl | Minus | Capture | LeftStick | LeftSl | LeftSr => Side::Left,
            A | B | X | Y | R | Zr | Plus | Home | RightStick | RightSl | RightSr => Side::Right,
        }
    }

    /// The control it is on a full controller, by place (padmap's naming:
    /// the bottom face button is South, which Nintendo prints B on). The
    /// triggers are ZL and ZR, given as the trigger axes instead. SL and SR
    /// are the paddles, as SDL's own combined pair has them.
    fn control(self) -> Option<Control> {
        use Part::*;
        Some(match self {
            Up => Control::DpadUp,
            Down => Control::DpadDown,
            Left => Control::DpadLeft,
            Right => Control::DpadRight,
            L => Control::LeftBumper,
            Minus => Control::Select,
            Capture => Control::Misc,
            LeftStick => Control::LeftStick,
            LeftSl => Control::PaddleUpperLeft,
            LeftSr => Control::PaddleLowerLeft,
            B => Control::South,
            A => Control::East,
            Y => Control::West,
            X => Control::North,
            R => Control::RightBumper,
            Plus => Control::Start,
            Home => Control::Guide,
            RightStick => Control::RightStick,
            RightSr => Control::PaddleUpperRight,
            RightSl => Control::PaddleLowerRight,
            Zl | Zr => return None,
        })
    }
}

/// What one half has down, and its stick (-1..1, y up), held upright.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HalfState {
    pub pressed: Vec<Part>,
    pub x: f32,
    pub y: f32,
}

/// The two halves as one controller: buttons by place, sticks -1..1 with y
/// up, triggers 0..1, as `pad.rs` hands any pad to the shell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Merged {
    pub pressed: Vec<Control>,
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
    pub lt: f32,
    pub rt: f32,
}

/// How far a Joy-Con stick may rest off centre: they drift more than most,
/// and SDL's evdev path gives them no deadzone.
pub const STICK_REST: f32 = 0.15;

/// A stick past its resting zone, rescaled so the edge of the zone is 0 and
/// full travel is still 1.
pub fn rest(x: f32, y: f32) -> (f32, f32) {
    let r = (x * x + y * y).sqrt();
    if !r.is_finite() || r <= STICK_REST {
        return (0.0, 0.0);
    }
    let scale = ((r - STICK_REST) / (1.0 - STICK_REST)).min(1.0) / r;
    ((x * scale).clamp(-1.0, 1.0), (y * scale).clamp(-1.0, 1.0))
}

/// The pair as one controller: the left half gives the left stick, d-pad,
/// minus, capture, L and ZL; the right half the right stick, A B X Y, plus,
/// home, R and ZR. A half that is missing (disconnected) gives nothing, so its
/// controls read released and its stick centred. A part reported by the wrong
/// half is ignored.
pub fn merge(left: Option<&HalfState>, right: Option<&HalfState>) -> Merged {
    let mut out = Merged::default();
    for (half, side) in [(left, Side::Left), (right, Side::Right)] {
        let Some(h) = half else { continue };
        for part in h.pressed.iter().copied().filter(|p| p.side() == side) {
            match part {
                Part::Zl => out.lt = 1.0,
                Part::Zr => out.rt = 1.0,
                p => {
                    if let Some(c) = p.control() {
                        if !out.pressed.contains(&c) {
                            out.pressed.push(c);
                        }
                    }
                }
            }
        }
        let (x, y) = rest(h.x, h.y);
        match side {
            Side::Left => (out.lx, out.ly) = (x, y),
            Side::Right => (out.rx, out.ry) = (x, y),
        }
    }
    out
}

/// hid-nintendo's left half, by SDL's button index. SDL numbers evdev buttons
/// in key-code order; these are the indices SDL's own database maps for this
/// device (050000007e0500000620000001800000), held upright: BTN_Z is capture,
/// BTN_TL/TL2 are L/ZL, BTN_TR/TR2 the rail's SL/SR, BTN_SELECT minus,
/// BTN_THUMBL the stick, BTN_DPAD_* the d-pad.
const LEFT_RAW: [(u32, Part); 11] = [
    (4, Part::Capture),
    (5, Part::L),
    (6, Part::LeftSl),
    (7, Part::Zl),
    (8, Part::LeftSr),
    (9, Part::Minus),
    (12, Part::LeftStick),
    (14, Part::Up),
    (15, Part::Down),
    (16, Part::Left),
    (17, Part::Right),
];

/// hid-nintendo's right half (050000007e0500000720000001800000): BTN_SOUTH,
/// EAST, NORTH, WEST are B, A, X, Y by place, BTN_TL/TL2 the rail's SL/SR,
/// BTN_TR/TR2 R/ZR, BTN_START plus, BTN_MODE home, BTN_THUMBR the stick.
const RIGHT_RAW: [(u32, Part); 11] = [
    (0, Part::B),
    (1, Part::A),
    (2, Part::X),
    (3, Part::Y),
    (4, Part::RightSl),
    (5, Part::R),
    (6, Part::RightSr),
    (7, Part::Zr),
    (8, Part::Plus),
    (9, Part::Home),
    (10, Part::RightStick),
];

/// One hid-nintendo half's raw state, upright: `buttons` by SDL index, `axes`
/// -1..1 as SDL reads them (x right, y down). Both halves put their stick on
/// axes 0 and 1.
pub fn half_from_raw(side: Side, buttons: &[bool], axes: &[f32]) -> HalfState {
    let table: &[(u32, Part)] = match side {
        Side::Left => &LEFT_RAW,
        Side::Right => &RIGHT_RAW,
    };
    let pressed = table.iter().filter(|(i, _)| buttons.get(*i as usize).copied().unwrap_or(false)).map(|(_, p)| *p).collect();
    let axis = |i: usize| axes.get(i).copied().filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-1.0, 1.0);
    HalfState { pressed, x: axis(0), y: -axis(1) }
}

/* ── Games ─────────────────────────────────────────────────── */

/// What a Linux game's environment gets while a pair is set to be one
/// controller: SDL's own Joy-Con driver and its combining, on even where
/// `pad::sdl_env` turns the rest of HIDAPI off. SDL 2.24 and newer follow it,
/// and so do Wine and Proton through their SDL backend. Values in the game's
/// own launch options still win (`launch::set_default`).
pub fn game_env() -> Vec<(String, String)> {
    vec![
        ("SDL_JOYSTICK_HIDAPI_JOY_CONS".into(), "1".into()),
        ("SDL_JOYSTICK_HIDAPI_COMBINE_JOY_CONS".into(), "1".into()),
    ]
}

/// In plain words, where a pair set to one controller works, for this OS.
pub fn reach(windows: bool) -> &'static str {
    if windows {
        "Works in Kryoto, and in games through the virtual Xbox controller (it needs ViGEmBus). A game that reads Joy-Cons itself may still see two controllers."
    } else {
        "Works in Kryoto and in games that use SDL, Wine or Proton; other games see two controllers."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L_EVDEV: &str = "050000007e0500000620000001800000";
    const R_EVDEV: &str = "050000007e0500000720000001800000";
    const L_HIDAPI: &str = "050000007e0500000620000001006800";

    #[test]
    fn halves_are_told_by_their_ids() {
        assert_eq!(side_of(Some(NINTENDO), Some(LEFT), "Joy-Con (L)"), Some(Side::Left));
        assert_eq!(side_of(Some(NINTENDO), Some(RIGHT), "Nintendo Switch Right Joy-Con"), Some(Side::Right));
        assert_eq!(side_of(Some(NINTENDO), Some(GRIP), "Nintendo Switch Joy-Con (L)"), Some(Side::Left));
        assert_eq!(side_of(Some(NINTENDO), Some(GRIP), "Nintendo Switch Joy-Con (R)"), Some(Side::Right));
        // The grip without a side, a Pro Controller, a pair, another maker.
        assert_eq!(side_of(Some(NINTENDO), Some(GRIP), "Charging Grip"), None);
        assert_eq!(side_of(Some(NINTENDO), Some(0x2009), "Pro Controller"), None);
        assert_eq!(side_of(Some(NINTENDO), Some(PAIR), "Joy-Con (L/R)"), None);
        assert_eq!(side_of(Some(0x045e), Some(LEFT), "Joy-Con (L)"), None);
        assert_eq!(side_of(None, None, "Joy-Con (L)"), None);
        // One pad already: SDL's combined pair and joycond's.
        assert!(is_pair_device(Some(NINTENDO), Some(PAIR), "Nintendo Switch Joy-Con (L/R)"));
        assert!(is_pair_device(Some(NINTENDO), Some(LEFT), "Nintendo Switch Combined Joy-Cons (L/R)"));
        assert!(!is_pair_device(Some(NINTENDO), Some(LEFT), "Joy-Con (L)"));
    }

    #[test]
    fn pairs_go_first_left_with_first_right() {
        let halves = [(7, Side::Right), (3, Side::Left), (9, Side::Left), (4, Side::Right), (12, Side::Left)];
        assert_eq!(find_pairs(&halves), vec![(3, 4), (9, 7)]);
        assert!(find_pairs(&[(1, Side::Left), (2, Side::Left)]).is_empty());
        assert!(find_pairs(&[]).is_empty());
    }

    #[test]
    fn a_pair_is_kept_by_its_serials_else_its_models() {
        let l = HalfId { guid: L_HIDAPI.into(), serial: "98:B6:E9:00:00:01".into() };
        let r = HalfId { guid: "050000007e0500000720000001006800".into(), serial: " 98:b6:e9:00:00:02 ".into() };
        assert_eq!(pair_key(&l, &r), "98:b6:e9:00:00:01+98:b6:e9:00:00:02");
        let l = HalfId { guid: L_EVDEV.to_uppercase(), serial: String::new() };
        let r = HalfId { guid: R_EVDEV.into(), serial: String::new() };
        assert_eq!(pair_key(&l, &r), format!("{L_EVDEV}+{R_EVDEV}"));
    }

    #[test]
    fn who_combines() {
        assert_eq!(way_of(Side::Left, L_HIDAPI, "\\\\?\\hid#vid_057e&pid_2006", 20, 4), Way::Sdl);
        assert_eq!(way_of(Side::Left, L_EVDEV, "/dev/input/event21", 18, 2), Way::Kryoto);
        assert_eq!(way_of(Side::Right, R_EVDEV, "/dev/input/event22", 11, 2), Way::Kryoto);
        // Too few buttons for the table, another kernel driver's version, not evdev.
        assert_eq!(way_of(Side::Left, L_EVDEV, "/dev/input/event21", 12, 2), Way::None);
        assert_eq!(way_of(Side::Left, "050000007e050000062000000f060000", "/dev/input/event21", 18, 2), Way::None);
        assert_eq!(way_of(Side::Left, L_EVDEV, "", 18, 2), Way::None);
        assert_eq!(pair_way(Way::Sdl, Way::Sdl), Way::Sdl);
        assert_eq!(pair_way(Way::Sdl, Way::Kryoto), Way::None);
        // SDL combines while a pair it reads is wanted, or its pair still is.
        assert!(sdl_combine(true, false) && sdl_combine(false, true) && !sdl_combine(false, false));
    }

    fn half(pressed: &[Part], x: f32, y: f32) -> HalfState {
        HalfState { pressed: pressed.to_vec(), x, y }
    }

    #[test]
    fn every_button_lands_on_its_place() {
        use Part::*;
        let cases: [(Part, Control); 20] = [
            (Up, Control::DpadUp),
            (Down, Control::DpadDown),
            (Left, Control::DpadLeft),
            (Right, Control::DpadRight),
            (L, Control::LeftBumper),
            (Minus, Control::Select),
            (Capture, Control::Misc),
            (LeftStick, Control::LeftStick),
            (LeftSl, Control::PaddleUpperLeft),
            (LeftSr, Control::PaddleLowerLeft),
            (B, Control::South),
            (A, Control::East),
            (Y, Control::West),
            (X, Control::North),
            (R, Control::RightBumper),
            (Plus, Control::Start),
            (Home, Control::Guide),
            (RightStick, Control::RightStick),
            (RightSr, Control::PaddleUpperRight),
            (RightSl, Control::PaddleLowerRight),
        ];
        for (part, control) in cases {
            let h = half(&[part], 0.0, 0.0);
            let m = match part.side() {
                Side::Left => merge(Some(&h), None),
                Side::Right => merge(None, Some(&h)),
            };
            assert_eq!(m.pressed, vec![control], "{part:?}");
            assert_eq!((m.lt, m.rt), (0.0, 0.0), "{part:?}");
        }
        // ZL and ZR are the triggers.
        let m = merge(Some(&half(&[Zl], 0.0, 0.0)), Some(&half(&[Zr], 0.0, 0.0)));
        assert!(m.pressed.is_empty());
        assert_eq!((m.lt, m.rt), (1.0, 1.0));
    }

    #[test]
    fn both_halves_at_once_and_misfiled_parts() {
        use Part::*;
        let m = merge(Some(&half(&[L, Up, A], 0.0, 0.0)), Some(&half(&[A, R, Up], 0.0, 0.0)));
        // A from the left half and Up from the right are not theirs.
        assert_eq!(m.pressed, vec![Control::LeftBumper, Control::DpadUp, Control::East, Control::RightBumper]);
    }

    #[test]
    fn both_sticks_and_their_rest() {
        let m = merge(Some(&half(&[], 1.0, 0.0)), Some(&half(&[], 0.0, -1.0)));
        assert_eq!((m.lx, m.ly, m.rx, m.ry), (1.0, 0.0, 0.0, -1.0));
        // Drift inside the rest zone is nothing; just past it is small.
        let m = merge(Some(&half(&[], 0.1, 0.08)), Some(&half(&[], 0.0, 0.2)));
        assert_eq!((m.lx, m.ly), (0.0, 0.0));
        assert!(m.ry > 0.0 && m.ry < 0.1, "{}", m.ry);
        // Past full travel stays within -1..1.
        let (x, y) = rest(1.2, 1.2);
        assert!((-1.0..=1.0).contains(&x) && (-1.0..=1.0).contains(&y));
        assert_eq!(rest(f32::NAN, 0.0), (0.0, 0.0));
    }

    #[test]
    fn one_half_missing() {
        use Part::*;
        let left = half(&[Up, Zl], 0.0, 1.0);
        let m = merge(Some(&left), None);
        assert_eq!(m.pressed, vec![Control::DpadUp]);
        assert_eq!((m.lt, m.rt, m.ly, m.rx, m.ry), (1.0, 0.0, 1.0, 0.0, 0.0));
        let right = half(&[A], -1.0, 0.0);
        let m = merge(None, Some(&right));
        assert_eq!(m.pressed, vec![Control::East]);
        assert_eq!((m.lx, m.ly, m.rx), (0.0, 0.0, -1.0));
        assert_eq!(merge(None, None), Merged::default());
    }

    #[test]
    fn raw_hid_nintendo_halves() {
        let mut buttons = vec![false; 18];
        for i in [4, 5, 7, 14] {
            buttons[i] = true;
        }
        let h = half_from_raw(Side::Left, &buttons, &[0.5, -0.25]);
        assert_eq!(h.pressed, vec![Part::Capture, Part::L, Part::Zl, Part::Up]);
        // SDL's y grows downwards; the half's is up.
        assert_eq!((h.x, h.y), (0.5, 0.25));

        let mut buttons = vec![false; 11];
        for i in [0, 1, 5, 7, 9] {
            buttons[i] = true;
        }
        let h = half_from_raw(Side::Right, &buttons, &[]);
        assert_eq!(h.pressed, vec![Part::B, Part::A, Part::R, Part::Zr, Part::Home]);
        assert_eq!((h.x, h.y), (0.0, 0.0));
        // Fewer buttons than the table: the rest read released.
        assert!(half_from_raw(Side::Left, &[true; 3], &[f32::NAN, 2.0]).pressed.is_empty());
        let m = merge(Some(&half_from_raw(Side::Left, &[], &[0.0, -1.0])), None);
        assert_eq!(m.ly, 1.0);
    }

    /// Two uinput devices that look like hid-nintendo's halves (Bluetooth,
    /// 057E:2006 and 057E:2007, version 0x8001, its key codes), read through
    /// SDL as `pad.rs` reads them: SDL's GUIDs, its database's sideways mapping
    /// (which names the same button indices as [`half_from_raw`]), the way
    /// Kryoto would combine them, and the merge of real presses. Root in WSL:
    /// it makes uinput devices. Proves the layout against SDL's database, not
    /// against a real Joy-Con.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn hid_nintendo_halves_merge_on_linux_live() {
        use sdl2::controller::Button;
        use std::fs::{File, OpenOptions};
        use std::io::Write;
        use std::os::fd::AsRawFd;
        use std::time::Duration;

        // hid-nintendo's key codes: the left half registers these 18, the right 11.
        const LEFT_KEYS: [u16; 18] = [0x130, 0x131, 0x133, 0x134, 0x135, 0x136, 0x137, 0x138, 0x139, 0x13a, 0x13b, 0x13c, 0x13d, 0x13e, 0x220, 0x221, 0x222, 0x223];
        const RIGHT_KEYS: [u16; 11] = [0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x138, 0x139, 0x13b, 0x13c, 0x13e];
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
        struct Half(File);
        impl Half {
            fn new(name: &str, product: u16, keys: &[u16]) -> Self {
                let file = OpenOptions::new().write(true).open("/dev/uinput").expect("root, for /dev/uinput");
                let fd = file.as_raw_fd();
                unsafe {
                    libc::ioctl(fd, 0x4004_5564, 1); // EV_KEY
                    libc::ioctl(fd, 0x4004_5564, 3); // EV_ABS
                    for k in keys {
                        libc::ioctl(fd, 0x4004_5565, libc::c_int::from(*k));
                    }
                    for a in [0u16, 1] {
                        libc::ioctl(fd, 0x4004_5567, libc::c_int::from(a));
                    }
                    let mut setup = Setup { bustype: 0x05, vendor: NINTENDO, product, version: 0x8001, name: [0; 80], ff_effects_max: 0 };
                    setup.name[..name.len()].copy_from_slice(name.as_bytes());
                    assert!(libc::ioctl(fd, 0x405c_5503, &setup as *const Setup) >= 0);
                    for a in [0u16, 1] {
                        let abs = AbsSetup { code: a, value: 0, minimum: -32767, maximum: 32767, fuzz: 0, flat: 0, resolution: 0 };
                        assert!(libc::ioctl(fd, 0x401c_5504, &abs as *const AbsSetup) >= 0);
                    }
                    assert!(libc::ioctl(fd, 0x5501) >= 0);
                }
                Self(file)
            }
            fn send(&mut self, kind: u16, code: u16, value: i32) {
                for (kind, code, value) in [(kind, code, value), (0, 0, 0)] {
                    let mut e = [0u8; 24];
                    e[16..18].copy_from_slice(&kind.to_ne_bytes());
                    e[18..20].copy_from_slice(&code.to_ne_bytes());
                    e[20..24].copy_from_slice(&value.to_ne_bytes());
                    self.0.write_all(&e).unwrap();
                }
            }
        }
        impl Drop for Half {
            fn drop(&mut self) {
                unsafe { libc::ioctl(self.0.as_raw_fd(), 0x5502) };
            }
        }

        let mut left = Half::new("Nintendo Switch Left Joy-Con", LEFT, &LEFT_KEYS);
        let mut right = Half::new("Nintendo Switch Right Joy-Con", RIGHT, &RIGHT_KEYS);
        std::thread::sleep(Duration::from_millis(500));
        // As pad.rs: by place, with SDL's own HIDAPI out of the way (uinput has no hidraw).
        sdl2::hint::set("SDL_GAMECONTROLLER_USE_BUTTON_LABELS", "0");
        sdl2::hint::set("SDL_JOYSTICK_HIDAPI", "0");
        let sdl = sdl2::init().unwrap();
        let js = sdl.joystick().unwrap();
        let gc = sdl.game_controller().unwrap();
        let mut pump = sdl.event_pump().unwrap();
        let find = |side: Side| {
            (0..js.num_joysticks().unwrap())
                .find(|i| {
                    let vendor = unsafe { sdl2::sys::SDL_JoystickGetDeviceVendor(*i as i32) };
                    let product = unsafe { sdl2::sys::SDL_JoystickGetDeviceProduct(*i as i32) };
                    side_of(Some(vendor), Some(product), &js.name_for_index(*i).unwrap_or_default()) == Some(side)
                })
                .expect("SDL lists the half")
        };
        let (li, ri) = (find(Side::Left), find(Side::Right));
        let path = |i: u32| unsafe { std::ffi::CStr::from_ptr(sdl2::sys::SDL_JoystickPathForIndex(i as i32)) }.to_string_lossy().to_string();
        let (lguid, rguid) = (js.device_guid(li).unwrap().string(), js.device_guid(ri).unwrap().string());
        println!("left {lguid} at {}, right {rguid} at {}", path(li), path(ri));
        // SDL puts a CRC of the name in bytes 2-3; its database leaves them 0.
        let no_crc = |g: &str| format!("{}0000{}", &g[..4], &g[8..]);
        assert_eq!(no_crc(&lguid), L_EVDEV);
        assert_eq!(no_crc(&rguid), R_EVDEV);
        let (lj, rj) = (js.open(li).unwrap(), js.open(ri).unwrap());
        assert_eq!((lj.num_buttons(), rj.num_buttons()), (18, 11));
        assert_eq!(way_of(Side::Left, &lguid, &path(li), lj.num_buttons(), lj.num_axes()), Way::Kryoto);
        assert_eq!(way_of(Side::Right, &rguid, &path(ri), rj.num_buttons(), rj.num_axes()), Way::Kryoto);
        // SDL's database maps them sideways, by the same indices.
        let (lc, rc) = (gc.open(li).expect("SDL maps the left half"), gc.open(ri).expect("SDL maps the right half"));

        let read = |j: &sdl2::joystick::Joystick, side: Side| {
            let buttons: Vec<bool> = (0..j.num_buttons()).map(|b| j.button(b).unwrap_or(false)).collect();
            let axes: Vec<f32> = (0..j.num_axes().min(2)).map(|a| j.axis(a).map(|v| v as f32 / 32767.0).unwrap_or(0.0)).collect();
            half_from_raw(side, &buttons, &axes)
        };
        let settle = |pump: &mut sdl2::EventPump| {
            std::thread::sleep(Duration::from_millis(150));
            for _ in pump.poll_iter() {}
        };

        // D-pad left on the left half: the bottom face button held sideways, as SDL maps it.
        left.send(1, 0x222, 1);
        settle(&mut pump);
        assert!(lc.button(Button::A), "SDL's sideways mapping: d-pad left is the bottom button");
        assert_eq!(read(&lj, Side::Left).pressed, vec![Part::Left]);
        left.send(1, 0x222, 0);
        // A (BTN_EAST) on the right half: sideways it is the bottom button.
        right.send(1, 0x131, 1);
        settle(&mut pump);
        assert!(rc.button(Button::A), "SDL's sideways mapping: A is the bottom button");
        right.send(1, 0x131, 0);

        // As one controller: L, d-pad up and the stick pushed up on the left;
        // A and ZR on the right.
        left.send(1, 0x136, 1);
        left.send(1, 0x220, 1);
        left.send(3, 1, -32767);
        right.send(1, 0x131, 1);
        right.send(1, 0x139, 1);
        settle(&mut pump);
        let m = merge(Some(&read(&lj, Side::Left)), Some(&read(&rj, Side::Right)));
        println!("merged: {m:?}");
        assert_eq!(m.pressed, vec![Control::LeftBumper, Control::DpadUp, Control::East]);
        assert_eq!((m.rt, m.lt), (1.0, 0.0));
        assert!(m.ly > 0.99, "pushed up is up: {}", m.ly);
        // The right half goes: the left keeps working alone.
        drop(rc);
        drop(rj);
        drop(right);
        settle(&mut pump);
        let m = merge(Some(&read(&lj, Side::Left)), None);
        assert_eq!(m.pressed, vec![Control::LeftBumper, Control::DpadUp]);
        assert_eq!(m.rt, 0.0);
        let _ = &mut left;
    }

    #[test]
    fn games_are_told_to_combine() {
        let env = game_env();
        assert!(env.contains(&("SDL_JOYSTICK_HIDAPI_COMBINE_JOY_CONS".into(), "1".into())));
        assert!(env.contains(&("SDL_JOYSTICK_HIDAPI_JOY_CONS".into(), "1".into())));
        assert_eq!(MERGED_GUID.len(), 32);
        assert!(MERGED_GUID.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
