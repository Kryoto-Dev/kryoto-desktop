//! Menus over the Store.
//!
//! The Store is a native web view laid over the shell, so nothing the shell
//! draws can appear on top of it. Menus (the account's, the nav tabs', the
//! bell's) and tooltips that land over the Store are drawn in one more web view
//! inside the main window, stacked above the Store, or in the Store's own page
//! (`draw_in_store`). It is part of the window, not a window of its own, so no
//! window manager decides where it goes, whether it may take focus or when it
//! is "in front": it moves, hides and closes with the client on every system.
//!
//! The shell sends what to draw and the anchor; the menu page draws it and
//! says how big it came out (`menu_ready`), which is when it is placed and
//! shown. A pick (`menu_pick`) or anything that closes it is reported back to
//! the shell as `menu-pick` and `menu-closed`.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::webview::WebviewBuilder;
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl};

/// The menu's web view. `capabilities/default.json` lets it call the shell's
/// commands; it loads the shell's own page and draws `PopupApp`.
pub const LABEL: &str = "popup";

/// Gap between the anchor and the menu, and between the menu and the edge of
/// the window, in logical pixels.
const GAP: f64 = 6.0;

/// How long a menu may take to come up before its view is made again. A dead
/// renderer answers nothing, and only the menus in this view would stop working.
const READY_WAIT: Duration = Duration::from_millis(2500);
/// The same for a view just made: the page has the whole shell to load.
const FIRST_WAIT: Duration = Duration::from_secs(8);

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
    /// Counts menu requests, so a wait for one never acts on the next.
    asked: AtomicU64,
    /// The menu asked for is on screen.
    shown: AtomicBool,
    /// The view's page has answered once, so a long wait now means it is stuck.
    answered: AtomicBool,
    /// Held while the view is made: `menu_warm` and `menu_open` can arrive together.
    making: Mutex<()>,
    /// The view was closed holding the keyboard. Hidden, it would keep it: the
    /// window coming back gives focus to whichever view had it last.
    left_focus: AtomicBool,
}

#[derive(Clone, Serialize)]
struct Closed {
    menu: Option<String>,
    /// Closed from the keyboard: the shell gives it back to the trigger.
    back: bool,
}

fn menu_id(payload: &Option<serde_json::Value>) -> Option<String> {
    payload.as_ref().and_then(|p| p["menu"].as_str()).map(String::from)
}

fn open_id<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    app.try_state::<Menus>()?.payload.lock().ok().and_then(|p| menu_id(&p))
}

/// Where the Store's view sits in the main window (logical pixels), so an
/// anchor in the shell's page can be turned into one in the Store's.
static STORE_ORIGIN: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));
/// Whether the Store is on screen. A menu drawn in its page while it is hidden
/// (the Library, a dialog) would open where nobody can see it.
static STORE_SHOWN: AtomicBool = AtomicBool::new(false);

pub fn set_store_shown(shown: bool) {
    STORE_SHOWN.store(shown, Ordering::Relaxed);
}

pub fn set_store_origin(x: f64, y: f64) {
    if let Ok(mut o) = STORE_ORIGIN.lock() {
        *o = (x, y);
    }
    #[cfg(target_os = "linux")]
    crate::system::set_fill_split(y * crate::text_size::css_to_logical());
}

/// Draw a plain menu in the Store's own page (BROWSER_STATE_SCRIPT,
/// `__kryoMenu`), when a kryo.to page is in it. That is a part of the page: a
/// second web view over the Store could not be see-through on Linux, which is
/// why its corners were square or grey. False when the page cannot take it
/// (not loaded, not kryo.to) and the menu view should draw it instead.
fn draw_in_store<R: Runtime>(app: &AppHandle<R>, payload: &serde_json::Value, anchor: Anchor, right: bool) -> bool {
    if !STORE_SHOWN.load(Ordering::Relaxed) {
        return false;
    }
    let Some(store) = app.get_webview(crate::STORE) else { return false };
    if !store.url().map(|u| crate::is_kryoto(&u, app)).unwrap_or(false) {
        return false;
    }
    let (ox, oy) = STORE_ORIGIN.lock().map(|o| *o).unwrap_or((0.0, 0.0));
    // The page only reaches down from the Store's top edge. A trigger well above
    // it cannot have its menu drawn next to it from in here: the menu view does that one.
    if anchor.bottom + GAP * 4.0 < oy {
        return false;
    }
    let message = serde_json::json!({
        "items": payload["items"],
        "minWidth": payload["minWidth"],
        "colors": payload["look"]["colors"],
        "x": anchor.left - ox,
        "anchorRight": anchor.right - ox,
        "y": (anchor.bottom - oy + GAP).max(0.0),
        "right": right,
        "focus": payload["focus"],
    });
    let drawn = store.eval(format!("window.__kryoMenu&&window.__kryoMenu.open({message})")).is_ok();
    // Opened from the keyboard: the arrows have to reach the page.
    if drawn && payload["focus"] == true {
        let _ = store.set_focus();
    }
    drawn
}

/// Hide the menu, if one is showing, and tell the shell.
pub fn close<R: Runtime>(app: &AppHandle<R>) {
    close_from(app, false);
}

fn close_from<R: Runtime>(app: &AppHandle<R>, back: bool) {
    let Some(state) = app.try_state::<Menus>() else { return };
    let Some(menu) = state.payload.lock().ok().and_then(|mut p| menu_id(&p.take())) else { return };
    if let Some(view) = app.get_webview(LABEL) {
        // Gone with the window (another app in front): nothing else has the keyboard.
        let away = app.get_window("main").and_then(|w| w.is_focused().ok()).is_some_and(|f| !f);
        state.left_focus.store(menu != "tip" && !back && away, Ordering::SeqCst);
        let _ = view.hide();
    }
    if let Some(store) = app.get_webview(crate::STORE) {
        let _ = store.eval("window.__kryoMenu&&window.__kryoMenu.close()");
    }
    if back {
        if let Some(shell) = app.get_webview("main") {
            let _ = shell.set_focus();
        }
    }
    let _ = app.emit_to("main", "menu-closed", Closed { menu: Some(menu), back });
}

/// Give the keyboard to what is on screen: the Store if it is showing, else the shell.
fn focus_page<R: Runtime>(app: &AppHandle<R>) {
    let label = if STORE_SHOWN.load(Ordering::Relaxed) { crate::STORE } else { "main" };
    if let Some(view) = app.get_webview(label) {
        let _ = view.set_focus();
    }
}

/// The window came back. If the menu view held the keyboard when it closed, it
/// must not get it back while hidden, or every key goes nowhere until a click.
pub fn window_focused<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<Menus>() else { return };
    if state.left_focus.swap(false, Ordering::SeqCst) && open_id(app).is_none() {
        focus_page(app);
    }
}

/// Make the menu's web view, hidden, unless it is there already. Its page asks
/// for the menu once it has loaded (`menu_payload`), so nothing is lost to an
/// event sent before anything was listening.
fn make_view(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<Menus>();
    let _making = state.making.lock().map_err(|_| "menu lock")?;
    if app.get_webview(LABEL).is_some() {
        return Ok(());
    }
    state.answered.store(false, Ordering::SeqCst);
    let window = app.get_window("main").ok_or("The main window is gone.")?;
    let builder = WebviewBuilder::new(LABEL, WebviewUrl::default())
        // The shell's own background, for the moment before the menu paints.
        // Opaque everywhere: see-through views drew dark pixels as clear on
        // WebKitGTK 2.54 (KB-0006). Linux rounds the view with a shape instead.
        .background_color(Color(10, 10, 10, 255))
        .focused(false);
    let (position, size) = crate::placement::rect(0.0, 0.0, 320.0, 480.0, None);
    let view = window.add_child(builder, position, size).map_err(|e| e.to_string())?;
    crate::display_env::watch_crashes(&view);
    let _ = view.hide();
    Ok(())
}

/// Make the view ahead of its first menu (the pointer reached a trigger), so
/// that menu does not wait for a whole page to load.
#[tauri::command]
pub async fn menu_warm(app: AppHandle) -> Result<(), String> {
    make_view(&app)
}

/// Whether request `asked` is still the open menu and has not come up.
fn still_waiting(app: &AppHandle, asked: u64) -> bool {
    let state = app.state::<Menus>();
    state.asked.load(Ordering::SeqCst) == asked && !state.shown.load(Ordering::SeqCst) && open_id(app).is_some()
}

/// The menu never came up: its page is gone or stuck. Make the view again, once;
/// the new page picks the menu up by itself. If that one does not answer either,
/// the menu closes and says so. Nothing here may fail in silence.
fn watch_ready(app: AppHandle, asked: u64) {
    let wait = if app.state::<Menus>().answered.load(Ordering::SeqCst) { READY_WAIT } else { FIRST_WAIT };
    std::thread::spawn(move || {
        std::thread::sleep(wait);
        if !still_waiting(&app, asked) {
            return;
        }
        crate::logging::warn("menu", "the menu view did not answer; making it again");
        if let Some(view) = app.get_webview(LABEL) {
            let _ = view.close();
        }
        std::thread::sleep(Duration::from_millis(300));
        let made = make_view(&app);
        std::thread::sleep(FIRST_WAIT);
        if still_waiting(&app, asked) {
            crate::logging::error("menu", &format!("the menu view still did not answer ({made:?})"));
            close(&app);
            let note = crate::downloads::Notice::new("That menu did not open", "Try it again. If it keeps happening, restart Kryoto.", None);
            let _ = app.emit_to("main", "notify-fallback", note);
        }
    });
}

/// Open a menu under `anchor` (logical pixels in the main window). With
/// `right`, its right edge lines up with the anchor's instead of its left.
/// True when it is on screen already; otherwise `menu-shown` says when it is.
#[tauri::command]
pub async fn menu_open(
    app: AppHandle,
    anchor: Anchor,
    right: bool,
    payload: serde_json::Value,
    scale: Option<f64>,
    viewport_width: Option<f64>,
) -> Result<bool, String> {
    let window = app.get_window("main").ok_or("The main window is gone.")?;
    let scale = crate::placement::shell_scale(&window, scale, viewport_width)?;
    let state = app.state::<Menus>();
    let previous = open_id(&app);
    let asked = state.asked.fetch_add(1, Ordering::SeqCst) + 1;
    state.shown.store(false, Ordering::SeqCst);
    *state.scale.lock().map_err(|_| "menu lock")? = crate::placement::css_scale(scale);
    *state.payload.lock().map_err(|_| "menu lock")? = Some(payload.clone());
    *state.anchor.lock().map_err(|_| "menu lock")? = (anchor, right);
    // Another menu was up (hovering from tab to tab): it closed.
    if previous.is_some() {
        let _ = app.emit_to("main", "menu-closed", Closed { menu: previous, back: false });
    }
    if payload["kind"] == "menu" && draw_in_store(&app, &payload, anchor, right) {
        // A menu from the other kind of view may still be up.
        if let Some(view) = app.get_webview(LABEL) {
            let _ = view.hide();
        }
        state.shown.store(true, Ordering::SeqCst);
        return Ok(true);
    }
    make_view(&app)?;
    // A tooltip that never shows is nobody's problem; a menu that never shows is.
    if payload["kind"] != "tip" {
        watch_ready(app.clone(), asked);
    }
    if let Some(view) = app.get_webview(LABEL) {
        // Drawn again, hidden, then shown by `menu_ready` at its new size.
        let _ = view.hide();
    }
    app.emit_to(LABEL, "menu-show", payload).map_err(|e| e.to_string())?;
    Ok(false)
}

#[tauri::command]
pub fn menu_payload(state: tauri::State<'_, Menus>) -> Option<serde_json::Value> {
    state.payload.lock().ok().and_then(|p| p.clone())
}

/// The menu has drawn itself at this size: place it, kept inside the window,
/// then show it and give it the keyboard.
#[tauri::command]
pub async fn menu_ready(app: AppHandle, menu: String, width: f64, height: f64, scale: Option<f64>) -> Result<(), String> {
    app.state::<Menus>().answered.store(true, Ordering::SeqCst);
    let menu_scale = crate::placement::css_scale(scale);
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
    // The measured shell scale includes display scaling and text zoom on Linux.
    // Older callers without a viewport measurement retain the text-size fallback.
    let z = crate::text_size::css_to_logical();
    let inner = if cfg!(target_os = "linux") && css.is_none() {
        let l = window.inner_size().map_err(|e| e.to_string())?.to_logical::<f64>(window.scale_factor().map_err(|e| e.to_string())?);
        tauri::LogicalSize::new(l.width / z, l.height / z)
    } else {
        window.inner_size().map_err(|e| e.to_string())?.to_logical::<f64>(scale)
    };
    let (anchor, right) = *app.state::<Menus>().anchor.lock().map_err(|_| "menu lock")?;
    // The menu measured itself in its own CSS pixels. Where its ratio differs
    // from the shell's (a monitor with another scale it has not caught up
    // with), sized in the shell's units it came out narrow and cut its text.
    let ratio = if cfg!(target_os = "linux") {
        // Both views have the same text zoom. Their raw DPR may omit that zoom.
        1.0
    } else {
        menu_scale.map_or(1.0, |menu| menu / scale)
    };
    let (width, height) = ((width * ratio).ceil().max(40.0), (height * ratio).ceil().max(20.0));
    let tip = menu == "tip";
    let x = match (tip, right) {
        (true, _) => (anchor.left + anchor.right - width) / 2.0,
        (_, true) => anchor.right - width,
        _ => anchor.left,
    };
    let x = x.min(inner.width - width - GAP).max(GAP);
    let y = (anchor.bottom + GAP).min(inner.height - height - GAP).max(GAP);
    crate::placement::place(&view, x, y, width, height, css)?;
    #[cfg(target_os = "linux")]
    {
        crate::linux_overlay::place(&view, x, y, width, height, css);
        // Rounded like the panel it draws, at the reader's corner style
        // (styles.css), cut by a shape so the view itself can stay opaque (KB-0006).
        let _ = view.with_webview(move |v| {
            use gtk::prelude::*;
            let (_, _, w, h) = crate::linux_overlay::rect(0.0, 0.0, width, height, css, v.inner().scale_factor());
            crate::system::linux_shape::view(v.inner().upcast_ref::<gtk::Widget>(), w, h, crate::system::corner_drawn());
        });
    }
    view.show().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    raise(&view);
    // A tooltip is only looked at: the keyboard stays where it is.
    if !tip {
        let _ = view.set_focus();
    }
    app.state::<Menus>().shown.store(true, Ordering::SeqCst);
    let _ = app.emit_to("main", "menu-shown", menu);
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
    focus_page(&app);
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
pub fn menu_close(app: AppHandle, menu: Option<String>, back: Option<bool>) {
    if let Some(menu) = menu {
        if open_id(&app).as_deref() != Some(menu.as_str()) {
            return;
        }
    }
    close_from(&app, back.unwrap_or(false));
}

/// The pointer went into (or left) the menu, for menus that open on hover.
#[tauri::command]
pub fn menu_hover(app: AppHandle, inside: bool) {
    let _ = app.emit_to("main", "menu-hover", inside);
}
