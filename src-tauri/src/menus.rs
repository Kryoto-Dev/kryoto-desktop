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

/// Where the Store's view sits in the main window (logical pixels), so an
/// anchor in the shell's page can be turned into one in the Store's.
static STORE_ORIGIN: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));

pub fn set_store_origin(x: f64, y: f64) {
    if let Ok(mut o) = STORE_ORIGIN.lock() {
        *o = (x, y);
    }
    #[cfg(target_os = "linux")]
    crate::system::set_fill_split(y);
}

/// Draw a plain menu in the Store's own page (BROWSER_STATE_SCRIPT,
/// `__kryoMenu`), when a kryo.to page is in it. That is a part of the page: a
/// second web view over the Store could not be see-through on Linux, which is
/// why its corners were square or grey. False when the page cannot take it
/// (not loaded, not kryo.to) and the menu view should draw it instead.
fn draw_in_store<R: Runtime>(app: &AppHandle<R>, payload: &serde_json::Value, anchor: Anchor, right: bool) -> bool {
    let Some(store) = app.get_webview(crate::STORE) else { return false };
    if !store.url().map(|u| crate::is_kryoto(&u, app)).unwrap_or(false) {
        return false;
    }
    let (ox, oy) = STORE_ORIGIN.lock().map(|o| *o).unwrap_or((0.0, 0.0));
    // The page only reaches down from the Store's top edge. A trigger well above
    // it (the title bar's menus, the account) cannot have its menu drawn next to
    // it from in here: the menu view does that one.
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
    });
    store.eval(format!("window.__kryoMenu&&window.__kryoMenu.open({message})")).is_ok()
}

/// Hide the menu, if one is showing, and tell the shell.
pub fn close<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<Menus>() else { return };
    let Some(menu) = state.payload.lock().ok().and_then(|mut p| menu_id(&p.take())) else { return };
    if let Some(view) = app.get_webview(LABEL) {
        let _ = view.hide();
    }
    if let Some(store) = app.get_webview(crate::STORE) {
        let _ = store.eval("window.__kryoMenu&&window.__kryoMenu.close()");
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
    if payload["kind"] == "menu" && draw_in_store(&app, &payload, anchor, right) {
        // A menu from the other kind of view may still be up.
        if let Some(view) = app.get_webview(LABEL) {
            let _ = view.hide();
        }
        return Ok(());
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
        // Linux: see-through, so the panel's rounded corners (styles.css) show
        // the window's fill behind them (system.rs, `round_corners`).
        .background_color(Color(10, 10, 10, if cfg!(target_os = "linux") { 0 } else { 255 }))
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
pub async fn menu_ready(
    app: AppHandle,
    menu: String,
    width: f64,
    height: f64,
    scale: Option<f64>,
    radius: Option<f64>,
    from: Option<f64>,
    span: Option<f64>,
) -> Result<(), String> {
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
    let inner = window.inner_size().map_err(|e| e.to_string())?.to_logical::<f64>(scale);
    let (anchor, right) = *app.state::<Menus>().anchor.lock().map_err(|_| "menu lock")?;
    // The menu measured itself in its own CSS pixels. Where its ratio differs
    // from the shell's (a monitor with another scale it has not caught up
    // with), sized in the shell's units it came out narrow and cut its text.
    let ratio = menu_scale.map_or(1.0, |menu| menu / scale);
    let (width, height) = ((width * ratio).ceil().max(40.0), (height * ratio).ceil().max(20.0));
    let x = if right { anchor.right - width } else { anchor.left };
    let x = x.min(inner.width - width - GAP).max(GAP);
    let y = (anchor.bottom + GAP).min(inner.height - height - GAP).max(GAP);
    crate::placement::place(&view, x, y, width, height, css)?;
    #[cfg(target_os = "linux")]
    crate::linux_overlay::place(&view, x, y, width, height);
    // Before it is shown, so the view never appears at full size for a frame.
    // The page says how round its panel is (its CSS px), and how it opens.
    #[cfg(windows)]
    raise(&view, radius.unwrap_or(10.0).max(0.0) * scale, from.unwrap_or(0.9).clamp(0.5, 1.0), span.unwrap_or(200.0).max(1.0));
    view.show().map_err(|e| e.to_string())?;
    let _ = view.set_focus();
    Ok(())
}

/// The newest opening, so an animation still running for a menu that has
/// since closed or been replaced stops.
#[cfg(windows)]
static OPENING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Keep the menu above the Store, and shape it. A child web view is a child
/// window on Windows, and one shown again keeps its old place in the stack. It
/// cannot be see-through over the Store's view (its corners came out black), so
/// the window itself is cut to the shape the page draws (styles.css), and that
/// cut is what opens: it grows from `from` (a fraction) to full size, from the top
/// edge, over `span_ms`, in step with the page's own scale. `radius` is in
/// physical pixels.
#[cfg(windows)]
fn raise(view: &tauri::Webview, radius: f64, from: f64, span_ms: f64) {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    /// The page starts its animation once the view is up, a few ms after this.
    const LEAD_MS: u64 = 30;
    // The shape at scale `s` of the full size, hung from the top edge's middle.
    unsafe fn shape(hwnd: isize, w: f64, h: f64, radius: f64, s: f64) {
        let (sw, sh) = ((w * s).round(), (h * s).round());
        let x = ((w - sw) / 2.0).round();
        let d = (radius * s * 2.0).round() as i32;
        // The system owns the region after this call.
        let rgn = CreateRoundRectRgn(x as i32, 0, (x + sw) as i32 + 1, sh as i32 + 1, d, d);
        SetWindowRgn(hwnd as _, rgn, 1);
    }
    let generation = OPENING.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = view.with_webview(move |w| unsafe {
        let mut hwnd = windows::Win32::Foundation::HWND::default();
        if w.controller().ParentWindow(&mut hwnd).is_err() {
            return;
        }
        let hwnd = hwnd.0 as isize;
        SetWindowPos(hwnd as _, HWND_TOP, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd as _, &mut r) == 0 {
            return;
        }
        let (w, h) = ((r.right - r.left) as f64, (r.bottom - r.top) as f64);
        shape(hwnd, w, h, radius, from);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(LEAD_MS));
            let start = std::time::Instant::now();
            loop {
                if OPENING.load(Ordering::SeqCst) != generation {
                    return;
                }
                let t = (start.elapsed().as_secs_f64() * 1000.0 / span_ms).min(1.0);
                // Ease-out, like the page's own curve.
                let s = from + (1.0 - from) * (1.0 - (1.0 - t).powi(4));
                shape(hwnd, w, h, radius, s);
                if t >= 1.0 {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        });
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
