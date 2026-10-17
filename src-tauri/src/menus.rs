//! Menus over the Store.
//!
//! The Store is a native web view laid over the shell, so nothing the shell
//! draws can appear on top of it. Menus (the title bar's, the nav tabs', the
//! bell's) are drawn in one more web view inside the main window, stacked
//! above the Store. It is part of the window, not a window of its own, so no
//! window manager decides where it goes, whether it may take focus or when it
//! is "in front": it moves, hides and closes with the client on every system.
//!
//! The shell sends what to draw and the anchor; the menu page draws it and
//! says how big it came out (`menu_ready`), which is when it is placed and
//! shown. A pick (`menu_pick`) or anything that closes it is reported back to
//! the shell as `menu-pick` and `menu-closed`.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::webview::WebviewBuilder;
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl};

/// The menu's web view. `capabilities/default.json` lets it call the shell's
/// commands; it loads the shell's own page and draws `PopupApp`.
pub const LABEL: &str = "popup";

/// Gap between the anchor and the menu, and between the menu and the edge of
/// the window, in logical pixels.
const GAP: f64 = 6.0;

#[derive(Clone, Copy, Default, Deserialize)]
pub struct Anchor {
    left: f64,
    right: f64,
    bottom: f64,
}

#[derive(Default)]
pub struct Menus {
    /// The menu being shown, as the shell sent it.
    payload: Mutex<Option<serde_json::Value>>,
    anchor: Mutex<(Anchor, bool)>,
    /// The shell's devicePixelRatio when the menu was asked for (placement.rs).
    scale: Mutex<Option<f64>>,
}

#[derive(Clone, Serialize)]
struct Closed {
    menu: Option<String>,
}

fn menu_id(payload: &Option<serde_json::Value>) -> Option<String> {
    payload.as_ref().and_then(|p| p["menu"].as_str()).map(String::from)
}

fn open_id<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    app.try_state::<Menus>()?.payload.lock().ok().and_then(|p| menu_id(&p))
}

/// Hide the menu, if one is showing, and tell the shell.
pub fn close<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<Menus>() else { return };
    let Some(menu) = state.payload.lock().ok().and_then(|mut p| menu_id(&p.take())) else { return };
    if let Some(view) = app.get_webview(LABEL) {
        let _ = view.hide();
    }
    let _ = app.emit_to("main", "menu-closed", Closed { menu: Some(menu) });
}

/// Open a menu under `anchor` (logical pixels in the main window). With
/// `right`, its right edge lines up with the anchor's instead of its left.
#[tauri::command]
pub async fn menu_open(
    app: AppHandle,
    anchor: Anchor,
    right: bool,
    payload: serde_json::Value,
    scale: Option<f64>,
) -> Result<(), String> {
    let state = app.state::<Menus>();
    let previous = open_id(&app);
    *state.scale.lock().map_err(|_| "menu lock")? = crate::placement::css_scale(scale);
    *state.payload.lock().map_err(|_| "menu lock")? = Some(payload.clone());
    *state.anchor.lock().map_err(|_| "menu lock")? = (anchor, right);
    // Another menu was up (hovering from tab to tab): it closed.
    if previous.is_some() {
        let _ = app.emit_to("main", "menu-closed", Closed { menu: previous });
    }
    if let Some(view) = app.get_webview(LABEL) {
        // Drawn again, hidden, then shown by `menu_ready` at its new size.
        let _ = view.hide();
        return app.emit_to(LABEL, "menu-show", payload).map_err(|e| e.to_string());
    }
    // The first menu of the session makes the view. Its page asks for the
    // payload once it has loaded (`menu_payload`), so nothing is lost to an
    // event sent before anything was listening.
    let window = app.get_window("main").ok_or("The main window is gone.")?;
    let builder = WebviewBuilder::new(LABEL, WebviewUrl::default())
        // The shell's own background, for the moment before the menu paints.
        .background_color(Color(10, 10, 10, 255))
        .focused(false);
    let (position, size) = crate::placement::rect(anchor.left, anchor.bottom, 320.0, 480.0, scale);
    let view = window.add_child(builder, position, size).map_err(|e| e.to_string())?;
    crate::display_env::watch_crashes(&view);
    let _ = view.hide();
    Ok(())
}

#[tauri::command]
pub fn menu_payload(state: tauri::State<'_, Menus>) -> Option<serde_json::Value> {
    state.payload.lock().ok().and_then(|p| p.clone())
}

/// The menu has drawn itself at this size: place it, kept inside the window,
/// then show it and give it the keyboard.
#[tauri::command]
pub async fn menu_ready(app: AppHandle, menu: String, width: f64, height: f64) -> Result<(), String> {
    // Drawn for a menu that has since closed or been replaced.
    if open_id(&app).as_deref() != Some(menu.as_str()) {
        return Ok(());
    }
    let view = app.get_webview(LABEL).ok_or("no menu")?;
    let window = app.get_window("main").ok_or("The main window is gone.")?;
    // The shell's own ratio when it sent one, so the menu's CSS pixels and
    // the window's size are measured the same way.
    let css = *app.state::<Menus>().scale.lock().map_err(|_| "menu lock")?;
    let scale = match css {
        Some(s) => s,
        None => window.scale_factor().map_err(|e| e.to_string())?,
    };
    let inner = window.inner_size().map_err(|e| e.to_string())?.to_logical::<f64>(scale);
    let (anchor, right) = *app.state::<Menus>().anchor.lock().map_err(|_| "menu lock")?;
    let (width, height) = (width.ceil().max(40.0), height.ceil().max(20.0));
    let x = if right { anchor.right - width } else { anchor.left };
    let x = x.min(inner.width - width - GAP).max(GAP);
    let y = (anchor.bottom + GAP).min(inner.height - height - GAP).max(GAP);
    crate::placement::place(&view, x, y, width, height, css)?;
    #[cfg(target_os = "linux")]
    crate::linux_overlay::place(&view, x, y, width, height);
    view.show().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    raise(&view);
    let _ = view.set_focus();
    Ok(())
}

/// Keep the menu above the Store. A child web view is a child window on
/// Windows, and one shown again keeps its old place in the stack.
#[cfg(windows)]
fn raise(view: &tauri::Webview) {
    let _ = view.with_webview(|w| unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        };
        let mut hwnd = windows::Win32::Foundation::HWND::default();
        if w.controller().ParentWindow(&mut hwnd).is_ok() {
            SetWindowPos(hwnd.0 as _, HWND_TOP, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        }
    });
}

/// Something in the menu was picked. The shell runs it.
#[tauri::command]
pub fn menu_pick(app: AppHandle, id: String) {
    close(&app);
    let _ = app.emit_to("main", "menu-pick", id);
}

/// Close the menu. With `menu`, only if that menu is the one still open.
///
/// Moving from one title bar menu to the next hides the menu view for a moment
/// while it redraws, and the page in it sees that as losing focus. Its close
/// used to land on the menu that had just been asked for, so hovering or
/// clicking from one menu to another shut the second one too. A close that
/// names a menu which is no longer the open one is that stale close, and is
/// ignored.
#[tauri::command]
pub fn menu_close(app: AppHandle, menu: Option<String>) {
    if let Some(menu) = menu {
        if open_id(&app).as_deref() != Some(menu.as_str()) {
            return;
        }
    }
    close(&app);
}

/// The pointer went into (or left) the menu, for menus that open on hover.
#[tauri::command]
pub fn menu_hover(app: AppHandle, inside: bool) {
    let _ = app.emit_to("main", "menu-hover", inside);
}
