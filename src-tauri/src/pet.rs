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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, Runtime, WebviewUrl, WebviewWindowBuilder, WindowEvent};

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
        .resizable(false)
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
    let pos = saved.filter(|p| on_a_monitor(app, *p)).or_else(|| corner(app));
    if let Some([x, y]) = pos {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
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

/// While the pet is open, read memory every few seconds on a thread of its
/// own (one cheap system call, no process list) and tell the pet when it is
/// nearly full: `pet-hungry` on the way in, then again now and then while it
/// stays full. Ends by itself when the pet is put away.
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
        while let Some(w) = app.get_webview_window(LABEL) {
            sys.refresh_memory();
            full = pretend || full_now(full, sys.used_memory(), sys.total_memory());
            if !full {
                fed = None;
            } else if fed.is_none_or(|at| at.elapsed() >= SNACK_EVERY) {
                fed = Some(Instant::now());
                let _ = w.emit_to(LABEL, "pet-hungry", ());
            }
            std::thread::sleep(MEMORY_EVERY);
        }
        WATCHING.store(false, Ordering::SeqCst);
    });
}

/// A remembered spot is only used while a monitor still covers it.
fn on_a_monitor<R: Runtime>(app: &AppHandle<R>, [x, y]: [i32; 2]) -> bool {
    let Ok(monitors) = app.available_monitors() else { return true };
    monitors.iter().any(|m| {
        let (p, s) = (m.position(), m.size());
        x >= p.x - 40 && y >= p.y - 40 && x < p.x + s.width as i32 - 40 && y < p.y + s.height as i32 - 40
    })
}

/// First run: the bottom-right corner of the screen, above the taskbar.
fn corner<R: Runtime>(app: &AppHandle<R>) -> Option<[i32; 2]> {
    let m = app.primary_monitor().ok().flatten()?;
    let area = m.work_area();
    let size = (SIZE * m.scale_factor()) as i32;
    Some([area.position.x + area.size.width as i32 - size - 24, area.position.y + area.size.height as i32 - size])
}

static LAST_POS: Mutex<Option<[i32; 2]>> = Mutex::new(None);
static SAVING: AtomicBool = AtomicBool::new(false);
static MOVED_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Remember where it was put: once the moving stops, not on every pixel of a drag.
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
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
    if let Err(e) = crate::settings::write(app, &s) {
        crate::logging::error("pet", &e);
    }
    // Settings may be open in the main window: it shows the new choice.
    let _ = app.emit("pet-settings", serde_json::json!({ "pet": s.pet, "petMascot": s.pet_mascot }));
    sync(app);
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

#[cfg(test)]
mod tests {
    use super::full_now;

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
