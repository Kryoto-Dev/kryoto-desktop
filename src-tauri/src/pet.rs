//! The desktop pet: Kryoto's mascot (Telly unless the player picked another)
//! in a small see-through window of its own, always on top, that they can drag
//! anywhere on their screen - beside the window, on the taskbar, on another
//! monitor. It stays when the main window closes to the tray.
//!
//! A window rather than a view in the main one, so it can sit outside Kryoto.
//! It loads the shell's own page, which draws `PetApp` for this label
//! (src/pet). On or off, which mascot and where it stands are settings
//! (`pet`, `pet_mascot`, `pet_pos`); its right-click menu is native, so it is
//! never clipped by the pet's tiny window.
//!
//! It must never stand in anyone's way (KB-0369: on a ROG Ally it sat top
//! left and nothing else on the screen took a click or a tap until it was
//! hidden). Its window took input on every pixel, see-through ones included,
//! and nothing held it to its size: a shell that stretches new windows over
//! the screen (a handheld's full screen mode) made it a sheet over
//! everything, with the 144 px page drawn in its top left corner. Now:
//! - on Windows the window lets every click and tap through
//!   (`set_ignore_cursor_events`) except while the pointer is on the pet's
//!   drawn pixels, which the page tells it (`pet_hit`);
//! - it is held to 144 px and put back if anything stretches it, and kept
//!   inside the work area of a monitor that exists, wherever its monitor
//!   query fails;
//! - it steps below another app's full screen window (Windows);
//! - "Hide the pet" is in the tray too, which opens when nothing else can be
//!   clicked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::window::Color;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Runtime, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub const LABEL: &str = "pet";

/// Three screen pixels for each of the mascot's 48.
const SIZE: f64 = 144.0;

pub const MASCOTS: [(&str, &str); 4] = [("telly", "Telly"), ("kubi", "Kubi"), ("yuki", "Yuki"), ("blip", "Blip")];

pub fn is_mascot(id: &str) -> bool {
    MASCOTS.iter().any(|(m, _)| *m == id)
}

/// Open or close the pet to match the settings.
pub fn sync<R: Runtime>(app: &AppHandle<R>) {
    let s = crate::settings::load(app);
    let open = app.get_webview_window(LABEL);
    match (s.pet, open) {
        (true, None) => {
            if let Err(e) = create(app, s.pet_pos) {
                crate::logging::error("pet", &e);
            }
        }
        (true, Some(w)) => {
            let _ = w.emit_to(LABEL, "pet-mascot", &s.pet_mascot);
        }
        (false, Some(w)) => {
            let _ = w.close();
        }
        (false, None) => {}
    }
}

fn create<R: Runtime>(app: &AppHandle<R>, saved: Option<[i32; 2]>) -> Result<(), String> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::default())
        .title("Kryoto pet")
        .inner_size(SIZE, SIZE)
        .min_inner_size(SIZE, SIZE)
        .max_inner_size(SIZE, SIZE)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .transparent(true)
        .background_color(Color(0, 0, 0, 0))
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        // Shown by the page once it has drawn, so it never flashes a square.
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    // A small helper window, not an application window: X11 desktops and
    // XWayland keep such windows floating rather than tiling them (KB-0336).
    #[cfg(target_os = "linux")]
    let _ = window.with_webview(|w| {
        use gtk::prelude::*;
        if let Some(top) = w.inner().toplevel().and_then(|t| t.downcast::<gtk::Window>().ok()) {
            top.set_type_hint(gtk::gdk::WindowTypeHint::Utility);
            top.set_skip_pager_hint(true);
        }
    });
    // Windows: every click and tap goes through until the page says the
    // pointer is on the pet itself (`pet_hit`). Not on Linux, where the
    // pointer cannot be asked for under Wayland and the pet would never take
    // a click again; its small window there is held to its size all the same.
    #[cfg(windows)]
    let _ = window.set_ignore_cursor_events(true);
    let pos = saved.filter(|p| on_a_monitor(app, *p)).or_else(|| corner(app));
    if let Some([x, y]) = pos {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    keep_in(&window);
    let handle = app.clone();
    window.on_menu_event(move |_, event| on_menu(&handle, event.id().as_ref()));
    watch_memory(app);
    Ok(())
}

/// Memory this full and the pet eats some (suggestion #52); it is hungry
/// again once use has dropped below `EASED`.
const FULL: f64 = 0.92;
const EASED: f64 = 0.86;
/// How often memory is read, and how often a full PC is eaten from again.
const MEMORY_EVERY: Duration = Duration::from_secs(5);
const SNACK_EVERY: Duration = Duration::from_secs(90);

static WATCHING: AtomicBool = AtomicBool::new(false);

/// Still full? With a gap between the two marks, so use hovering at the line
/// does not set it eating every few seconds. Pure, for the tests.
pub fn full_now(was_full: bool, used: u64, total: u64) -> bool {
    if total == 0 {
        return false;
    }
    let share = used as f64 / total as f64;
    if was_full { share >= EASED } else { share >= FULL }
}

/// How often the pet looks for another app's full screen window.
const TICK: Duration = Duration::from_secs(1);

/// While the pet is open, on a thread of its own: every second, whether
/// another app has a full screen window in front (Windows; the pet then stops
/// being always on top), and every few seconds the memory (one cheap system
/// call, no process list), telling the pet when it is nearly full:
/// `pet-hungry` on the way in, then again now and then while it stays full.
/// Ends by itself when the pet is put away.
fn watch_memory<R: Runtime>(app: &AppHandle<R>) {
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // Debug builds can act it out on a PC with memory to spare.
        let pretend = cfg!(debug_assertions) && std::env::var_os("KRYOTO_DEV_PET_HUNGRY").is_some();
        let mut sys = sysinfo::System::new();
        let mut full = false;
        let mut fed: Option<Instant> = None;
        let mut read_at: Option<Instant> = None;
        let mut on_top = true;
        while let Some(w) = app.get_webview_window(LABEL) {
            let top = !someone_else_full_screen();
            if top != on_top {
                on_top = top;
                let _ = w.set_always_on_top(top);
            }
            if read_at.is_none_or(|at| at.elapsed() >= MEMORY_EVERY) {
                read_at = Some(Instant::now());
                sys.refresh_memory();
                full = pretend || full_now(full, sys.used_memory(), sys.total_memory());
                if !full {
                    fed = None;
                } else if fed.is_none_or(|at| at.elapsed() >= SNACK_EVERY) {
                    fed = Some(Instant::now());
                    let _ = w.emit_to(LABEL, "pet-hungry", ());
                }
            }
            std::thread::sleep(TICK);
        }
        WATCHING.store(false, Ordering::SeqCst);
    });
}

/// The window in front is another app's and covers its whole monitor (a game,
/// a video, a handheld's full screen shell). The desktop itself does not count.
#[cfg(windows)]
fn someone_else_full_screen() -> bool {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId};
    // SAFETY: plain queries on the foreground window, with out-pointers to
    // locals of the sizes given.
    unsafe {
        let w = GetForegroundWindow();
        if w.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, &mut pid);
        if pid == std::process::id() {
            return false;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(w, class.as_mut_ptr(), class.len() as i32);
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        if class == "Progman" || class == "WorkerW" {
            return false;
        }
        let monitor = MonitorFromWindow(w, MONITOR_DEFAULTTONULL);
        if monitor.is_null() {
            return false;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let mut r: RECT = std::mem::zeroed();
        if GetMonitorInfoW(monitor, &mut info) == 0 || GetWindowRect(w, &mut r) == 0 {
            return false;
        }
        let m = info.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

#[cfg(not(windows))]
fn someone_else_full_screen() -> bool {
    false
}

/// A remembered spot is only used while a monitor still covers it.
fn on_a_monitor<R: Runtime>(app: &AppHandle<R>, [x, y]: [i32; 2]) -> bool {
    let Ok(monitors) = app.available_monitors() else { return true };
    monitors.iter().any(|m| {
        let (p, s) = (m.position(), m.size());
        x >= p.x - 40 && y >= p.y - 40 && x < p.x + s.width as i32 - 40 && y < p.y + s.height as i32 - 40
    })
}

/// The primary monitor, or any monitor at all when Windows names none.
fn some_monitor<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::Monitor> {
    app.primary_monitor().ok().flatten().or_else(|| app.available_monitors().ok()?.into_iter().next())
}

/// First run: the bottom-right corner of the screen, above the taskbar.
fn corner<R: Runtime>(app: &AppHandle<R>) -> Option<[i32; 2]> {
    let m = some_monitor(app)?;
    let area = m.work_area();
    let size = (SIZE * m.scale_factor()) as i32;
    Some([area.position.x + area.size.width as i32 - size - 24, area.position.y + area.size.height as i32 - size])
}

/// `at` moved so a `size` square stands wholly inside the work area at
/// `area` (its corner) of `area_size`. A work area smaller than the pet (a
/// monitor that reports nonsense) keeps its top left corner. Pure, for the tests.
pub fn clamped(at: [i32; 2], size: i32, area: [i32; 2], area_size: [i32; 2]) -> [i32; 2] {
    let fit = |v: i32, lo: i32, len: i32| if len <= size { lo } else { v.clamp(lo, lo + len - size) };
    [fit(at[0], area[0], area_size[0]), fit(at[1], area[1], area_size[1])]
}

/// Hold the pet to its size and inside the work area of the monitor it is
/// on (else the primary one, else any): whatever stretched it or put it
/// off screen, it goes back.
fn keep_in<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    let app = window.app_handle();
    let Some(m) = window.current_monitor().ok().flatten().or_else(|| some_monitor(app)) else { return };
    let scale = m.scale_factor();
    let size = (SIZE * scale).round() as i32;
    if window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
        let _ = window.set_fullscreen(false);
        let _ = window.unmaximize();
    }
    let now = window.outer_size().ok();
    if now.is_none_or(|s| (s.width as i32 - size).abs() > 2 || (s.height as i32 - size).abs() > 2) {
        let _ = window.set_size(LogicalSize::new(SIZE, SIZE));
    }
    let area = m.work_area();
    let at = window.outer_position().map(|p| [p.x, p.y]).unwrap_or([area.position.x, area.position.y]);
    let to = clamped(at, size, [area.position.x, area.position.y], [area.size.width as i32, area.size.height as i32]);
    if to != at {
        let _ = window.set_position(PhysicalPosition::new(to[0], to[1]));
    }
}

/// Put back at most this often: a shell that keeps stretching it is not
/// fought in a loop (it lets clicks through anyway).
static KEPT_AT: Mutex<Option<Instant>> = Mutex::new(None);

static LAST_POS: Mutex<Option<[i32; 2]>> = Mutex::new(None);
static SAVING: AtomicBool = AtomicBool::new(false);
static MOVED_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Remember where it was put: once the moving stops, not on every pixel of a
/// drag. Stretched (KB-0369), it goes back to its size and place.
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
    if let WindowEvent::Resized(size) = event {
        let scale = window.scale_factor().unwrap_or(1.0);
        let want = (SIZE * scale).round() as i32;
        let stretched = (size.width as i32 - want).abs() > 2 || (size.height as i32 - want).abs() > 2;
        let due = KEPT_AT.lock().map(|mut at| {
            let due = at.is_none_or(|t| t.elapsed() > Duration::from_millis(500));
            if due {
                *at = Some(Instant::now());
            }
            due
        });
        if stretched && due.unwrap_or(false) {
            crate::logging::warn("pet", &format!("its window was made {}x{}; put back to {want}x{want}", size.width, size.height));
            if let Some(w) = window.app_handle().get_webview_window(LABEL) {
                keep_in(&w);
            }
        }
        return;
    }
    let WindowEvent::Moved(p) = event else { return };
    if let Ok(mut at) = MOVED_AT.lock() {
        *at = Some(Instant::now());
    }
    if let Ok(mut last) = LAST_POS.lock() {
        *last = Some([p.x, p.y]);
    }
    if SAVING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = window.app_handle().clone();
    std::thread::spawn(move || {
        let mut seen = None;
        loop {
            std::thread::sleep(Duration::from_millis(700));
            let now = LAST_POS.lock().ok().and_then(|l| *l);
            if now == seen {
                break;
            }
            seen = now;
        }
        SAVING.store(false, Ordering::SeqCst);
        let mut s = crate::settings::load(&app);
        if s.pet_pos != seen {
            s.pet_pos = seen;
            if let Err(e) = crate::settings::write(&app, &s) {
                crate::logging::error("pet", &e);
            }
        }
    });
}

fn on_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let mut s = crate::settings::load(app);
    if let Some(m) = id.strip_prefix("pet-mascot-").filter(|m| is_mascot(m)) {
        s.pet_mascot = m.into();
    } else if id == "pet-open" {
        crate::system::show_main(app);
        return;
    } else if id == "pet-hide" {
        s.pet = false;
    } else {
        return;
    }
    save_and_sync(app, &s);
}

fn save_and_sync<R: Runtime>(app: &AppHandle<R>, s: &crate::settings::Settings) {
    if let Err(e) = crate::settings::write(app, s) {
        crate::logging::error("pet", &e);
    }
    // Settings may be open in the main window: it shows the new choice.
    let _ = app.emit("pet-settings", serde_json::json!({ "pet": s.pet, "petMascot": s.pet_mascot }));
    sync(app);
}

/// Put the pet away, as Settings' switch does: for the tray, which opens
/// when nothing else on the screen can be clicked (KB-0369).
pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    let mut s = crate::settings::load(app);
    s.pet = false;
    save_and_sync(app, &s);
}

/// The pet is out, for the tray's "Hide the pet".
#[tauri::command]
pub fn pet_shown(app: AppHandle) -> bool {
    app.get_webview_window(LABEL).is_some()
}

/// The pointer is on the pet's drawn pixels (`over`), or not: only then does
/// its window take clicks; otherwise they go to whatever is under it.
#[tauri::command]
pub fn pet_hit(window: tauri::WebviewWindow, over: bool) -> Result<(), String> {
    if window.label() != LABEL {
        return Err("Not the pet.".into());
    }
    window.set_ignore_cursor_events(!over).map_err(|e| e.to_string())
}

/// The right-click menu: which mascot, open Kryoto, or put the pet away.
#[tauri::command]
pub fn pet_menu(app: AppHandle) -> Result<(), String> {
    let window = app.get_webview_window(LABEL).ok_or("The pet is not out.")?;
    let current = crate::settings::load(&app).pet_mascot;
    let e = |e: tauri::Error| e.to_string();
    let menu = Menu::new(&app).map_err(e)?;
    for (id, name) in MASCOTS {
        let item = CheckMenuItem::with_id(&app, format!("pet-mascot-{id}"), name, true, id == current, None::<&str>).map_err(e)?;
        menu.append(&item).map_err(e)?;
    }
    menu.append(&PredefinedMenuItem::separator(&app).map_err(e)?).map_err(e)?;
    menu.append(&MenuItem::with_id(&app, "pet-open", "Open Kryoto", true, None::<&str>).map_err(e)?).map_err(e)?;
    menu.append(&MenuItem::with_id(&app, "pet-hide", "Hide the pet", true, None::<&str>).map_err(e)?).map_err(e)?;
    window.popup_menu(&menu).map_err(e)
}

/// Picked up. The system moves the window from here, and `start_dragging`
/// returns as soon as it has handed the move over, long before the button
/// comes up; the page hears "pet-dropped" when it does, so the pet dangles
/// its legs the whole way, as on the site.
#[tauri::command]
pub fn pet_drag(window: tauri::WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())?;
    let app = window.app_handle().clone();
    std::thread::spawn(move || {
        wait_for_release();
        let _ = app.emit_to(LABEL, "pet-dropped", ());
        // Dropped half off the screen: back inside it.
        if let Some(w) = app.get_webview_window(LABEL) {
            keep_in(&w);
        }
    });
    Ok(())
}

/// Until the primary button is up (or two minutes, whatever happened).
#[cfg(windows)]
fn wait_for_release() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
    // This reads the physical buttons: swapped, the primary one is the right.
    let button = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
    let until = Instant::now() + Duration::from_secs(120);
    while Instant::now() < until && unsafe { GetAsyncKeyState(button.into()) } < 0 {
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// No button state to read here: the drag is over once the window has stood
/// still for a moment. The page also lands it at the first pointer event after.
#[cfg(not(windows))]
fn wait_for_release() {
    let start = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let moved = MOVED_AT.lock().ok().and_then(|at| *at).filter(|at| *at > start).unwrap_or(start);
        if moved.elapsed() > Duration::from_millis(900) || start.elapsed() > Duration::from_secs(120) {
            return;
        }
    }
}

/// Which mascot to draw, for the pet's page when it starts.
#[tauri::command]
pub fn pet_mascot(app: AppHandle) -> String {
    crate::settings::load(&app).pet_mascot
}

/// The sizes the pet can be drawn at (Settings > Desktop pet).
pub const SIZES: [&str; 3] = ["normal", "small", "tiny"];

/// How big to draw the pet, for its page when it starts.
#[tauri::command]
pub fn pet_size(app: AppHandle) -> String {
    crate::settings::load(&app).pet_size
}

#[cfg(test)]
mod tests {
    use super::{clamped, full_now};

    /// KB-0369: wherever it was put or found, it stands inside the work area.
    #[test]
    fn the_pet_is_kept_inside_the_work_area() {
        // A 1080p handheld at 150%: the pet is 216 px, the taskbar 72.
        let (area, size) = ([0, 0], [1920, 1008]);
        assert_eq!(clamped([1680, 792], 216, area, size), [1680, 792]);
        assert_eq!(clamped([-500, -40], 216, area, size), [0, 0]);
        assert_eq!(clamped([5000, 5000], 216, area, size), [1704, 792]);
        // A second monitor to the left, and a portrait panel.
        assert_eq!(clamped([-100, 300], 216, [-1080, 0], [1080, 1920]), [-216, 300]);
        // A work area smaller than the pet (a monitor reporting nonsense).
        assert_eq!(clamped([300, 300], 216, [0, 0], [0, 0]), [0, 0]);
    }

    #[test]
    fn eats_when_nearly_full_and_stops_once_eased() {
        assert!(!full_now(false, 90, 100));
        assert!(full_now(false, 93, 100));
        // Hovering just under the mark keeps it full: no snack every 5 s.
        assert!(full_now(true, 90, 100));
        assert!(!full_now(true, 80, 100));
        assert!(!full_now(false, 1, 0));
    }
}
