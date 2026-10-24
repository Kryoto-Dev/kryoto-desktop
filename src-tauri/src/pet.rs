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
use std::time::Duration;
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
    Ok(())
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

/// Remember where it was put: once the moving stops, not on every pixel of a drag.
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
    let WindowEvent::Moved(p) = event else { return };
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

/// A double-click on the pet brings Kryoto up.
#[tauri::command]
pub fn pet_open_app(app: AppHandle) {
    crate::system::show_main(&app);
}

/// Which mascot to draw, for the pet's page when it starts.
#[tauri::command]
pub fn pet_mascot(app: AppHandle) -> String {
    crate::settings::load(&app).pet_mascot
}
