//! Text size (Settings > General, Ctrl + / Ctrl - / Ctrl 0): the whole app
//! drawn larger or smaller, as a browser's zoom does it.
//!
//! It is the web views' own zoom, not a font size in the pages: most of the
//! shell's sizes are fixed pixels (`text-[11px]`, icons, rails), which a root
//! font size would leave behind, so only some words grew and lines broke. The
//! zoom scales every view Kryoto draws text in at once: the shell (Library,
//! game pages, Settings, chat, Big Picture, dialogs), the Store, the menu
//! view and the page-solver window. The pet is left alone: it draws no words,
//! and its window is exactly the size of the mascot.
//!
//! Every view is zoomed as it is made (`apply`, from `on_webview_ready`), before
//! its page loads, so nothing is ever drawn at 100% first.
//!
//! The shell measures where the Store and menus go in its own CSS pixels.
//! WebView2 counts the zoom in `devicePixelRatio`, so the conversion to
//! physical pixels in placement.rs stays right on Windows. Linux places views
//! in logical pixels and never uses that ratio (WebKitGTK 2.54 counts the zoom
//! in it, older versions do not): there a CSS pixel is `factor()` logical ones
//! (`css_to_logical`).

use std::sync::atomic::{AtomicU32, Ordering};
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview};

/// The sizes offered, in percent. Ctrl + and Ctrl - step through them.
pub const STEPS: [u32; 5] = [90, 100, 110, 125, 150];
pub const DEFAULT: u32 = 100;

/// The size in use, in percent; read by placement on every move, so kept here
/// rather than in the settings file.
static PERCENT: AtomicU32 = AtomicU32::new(0);

/// The nearest offered size to `percent` (an old or hand-edited file).
pub fn clean(percent: u32) -> u32 {
    STEPS.iter().copied().min_by_key(|s| s.abs_diff(percent)).unwrap_or(DEFAULT)
}

/// One step up (`1`), down (`-1`), or back to 100% (`0`), stopping at the ends.
pub fn stepped(percent: u32, step: i32) -> u32 {
    let at = STEPS.iter().position(|s| *s == clean(percent)).unwrap_or(1);
    match step.signum() {
        0 => DEFAULT,
        1 => STEPS[(at + 1).min(STEPS.len() - 1)],
        _ => STEPS[at.saturating_sub(1)],
    }
}

fn percent<R: Runtime>(app: &AppHandle<R>) -> u32 {
    match PERCENT.load(Ordering::Relaxed) {
        0 => {
            let p = clean(crate::settings::load(app).text_size);
            PERCENT.store(p, Ordering::Relaxed);
            p
        }
        p => p,
    }
}

/// The zoom in use, 1.0 at 100%.
pub fn factor() -> f64 {
    match PERCENT.load(Ordering::Relaxed) {
        0 => 1.0,
        p => f64::from(p) / 100.0,
    }
}

/// Logical pixels per CSS pixel of the shell's page (see the top of this file).
pub fn css_to_logical() -> f64 {
    if cfg!(target_os = "linux") {
        factor()
    } else {
        1.0
    }
}

/// Zoom one view to the text size, if it is one Kryoto draws words in.
pub fn apply<R: Runtime>(view: &Webview<R>) {
    if view.label() == crate::pet::LABEL {
        return;
    }
    let zoom = f64::from(percent(view.app_handle())) / 100.0;
    if let Err(e) = view.set_zoom(zoom) {
        crate::logging::warn("text-size", &format!("{}: {e}", view.label()));
    }
    #[cfg(windows)]
    keep_zoom(view);
}

/// The splash box (tauri.conf.json's 340 by 380) at the text size, before it is
/// first shown: the shell sizes it the same way (src/lib/window.ts), but only
/// once its page runs, and until then the splash was drawn cramped.
pub fn size_start_window<R: Runtime>(window: &tauri::Window<R>) {
    let z = f64::from(percent(window.app_handle())) / 100.0;
    if (z - 1.0).abs() < 0.001 {
        return;
    }
    let size = tauri::LogicalSize::new((340.0 * z).round(), (380.0 * z).round());
    let _ = window.set_min_size(Some(size));
    let _ = window.set_max_size(Some(size));
    let _ = window.set_size(size);
    let _ = window.center();
}

/// WebView2 remembers a zoom per site, and going to a site it has another
/// zoom for puts that one back. Held to the text size instead. Setting the
/// zoom from here does not raise the event again.
#[cfg(windows)]
fn keep_zoom<R: Runtime>(view: &Webview<R>) {
    let _ = view.with_webview(|w| unsafe {
        use webview2_com::ZoomFactorChangedEventHandler;
        let controller = w.controller();
        let handler = ZoomFactorChangedEventHandler::create(Box::new(|sender, _| {
            let Some(controller) = sender else { return Ok(()) };
            let want = factor();
            let mut now = 0.0;
            controller.ZoomFactor(&mut now)?;
            if (now - want).abs() > 0.001 {
                controller.SetZoomFactor(want)?;
            }
            Ok(())
        }));
        let mut token = Default::default();
        let _ = controller.add_ZoomFactorChanged(&handler, &mut token);
    });
}

/// Use `percent` everywhere now, save it, and tell the pages.
fn set<R: Runtime>(app: &AppHandle<R>, percent: u32) -> Result<u32, String> {
    let percent = clean(percent);
    let mut settings = crate::settings::load(app);
    if settings.text_size != percent {
        settings.text_size = percent;
        crate::settings::write(app, &settings)?;
    }
    if PERCENT.swap(percent, Ordering::Relaxed) != percent {
        for view in app.webviews().values() {
            apply(view);
        }
        // A shape cut around the window's corners follows the page's corners.
        if let Some(main) = app.get_window("main") {
            crate::system::apply_corners(&main);
        }
        // The shell measures again at the new size (its views move with it),
        // and Settings shows the new choice when a key changed it.
        let _ = app.emit("text-size", percent);
    }
    Ok(percent)
}

#[tauri::command]
pub fn text_size_get(app: AppHandle) -> u32 {
    percent(&app)
}

/// Settings > General picked a size.
#[tauri::command]
pub fn text_size_set(app: AppHandle, percent: u32) -> Result<u32, String> {
    set(&app, percent)
}

/// Ctrl + (`1`), Ctrl - (`-1`) or Ctrl 0 (`0`), from the shell, a menu or a Store
/// page (BROWSER_STATE_SCRIPT). Only a step: a page cannot pick a size of its own.
#[tauri::command]
pub fn text_size_step(app: AppHandle, step: i32) -> Result<u32, String> {
    let now = percent(&app);
    set(&app, stepped(now, step))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_values_become_an_offered_size() {
        assert_eq!(clean(100), 100);
        assert_eq!(clean(0), 90);
        assert_eq!(clean(118), 125);
        assert_eq!(clean(400), 150);
    }

    #[test]
    fn steps_stop_at_the_ends_and_zero_resets() {
        assert_eq!(stepped(100, 1), 110);
        assert_eq!(stepped(110, 1), 125);
        assert_eq!(stepped(150, 1), 150);
        assert_eq!(stepped(100, -1), 90);
        assert_eq!(stepped(90, -1), 90);
        assert_eq!(stepped(125, 0), 100);
        assert_eq!(stepped(125, -5), 110);
    }

    #[test]
    fn an_old_settings_file_reads_as_100() {
        let old: crate::settings::Settings = serde_json::from_str(r#"{"connections":16}"#).unwrap();
        assert_eq!(old.text_size, 100);
    }
}
