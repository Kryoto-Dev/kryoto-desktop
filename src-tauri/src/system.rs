//! The client's life on the desktop: one copy running at a time, the tray icon,
//! closing to the tray, and starting with the computer.

use serde::Serialize;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, Runtime, WindowEvent,
};

/* ── One copy at a time ───────────────────────────────────── */

/// Debug builds use their own port, so a development copy runs beside an
/// installed one instead of handing over to it.
const INSTANCE_PORT: u16 = if cfg!(debug_assertions) { 47_632 } else { 47_631 };

/// The port, which a debug build can move with `KRYOTO_INSTANCE_PORT` so two
/// development copies (two sessions testing at once) never hand links or
/// "come to the front" to each other. A release build always uses its own.
fn instance_port() -> u16 {
    if cfg!(debug_assertions) {
        if let Some(p) = std::env::var("KRYOTO_INSTANCE_PORT").ok().and_then(|v| v.parse().ok()) {
            return p;
        }
    }
    INSTANCE_PORT
}
const HELLO: &str = "kryoto-desktop show";
const ANSWER: &str = "kryoto-desktop ok";

/// Take the instance port, or - when a Kryoto is already running - ask it to
/// come to the front and say so. Anything else on the port that does not
/// answer like a Kryoto is ignored, and this copy runs without the guard.
pub enum Instance {
    First(TcpListener),
    AlreadyRunning,
    Unguarded,
}

pub fn claim_instance() -> Instance {
    match TcpListener::bind(("127.0.0.1", instance_port())) {
        Ok(l) => Instance::First(l),
        Err(_) => {
            let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], instance_port()).into(), Duration::from_millis(600))
            else {
                return Instance::Unguarded;
            };
            let _ = s.set_read_timeout(Some(Duration::from_millis(1200)));
            // A `kryoto://` link this copy was started for rides on the same
            // line, so the running copy opens it (see links.rs).
            let hello = match crate::links::from_args() {
                Some(link) => format!("{HELLO} {link}"),
                None => HELLO.to_string(),
            };
            if writeln!(s, "{hello}").is_err() {
                return Instance::Unguarded;
            }
            let mut line = String::new();
            let _ = BufReader::new(&s).read_line(&mut line);
            if line.trim() == ANSWER { Instance::AlreadyRunning } else { Instance::Unguarded }
        }
    }
}

/// Answer later copies: bring the window back, and open the link one carried.
pub fn serve_instance<R: Runtime>(app: AppHandle<R>, listener: TcpListener) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
            let mut line = String::new();
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let line = line.trim();
            let link = match line.strip_prefix(HELLO) {
                Some("") => None,
                Some(rest) if rest.starts_with(' ') => Some(rest.trim().to_string()),
                _ => continue,
            };
            let mut w = &stream;
            let _ = writeln!(w, "{ANSWER}");
            match link {
                Some(url) => crate::links::deliver(&app, &url),
                None => show_main(&app),
            }
        }
    });
}

pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        rest_views(&w, false);
    }
}

/* ── Views at rest while nobody can see them ──────────────── */

/// The views put to rest, by label: only these wake again, so a Store the
/// shell had hidden (the Library is open) stays hidden.
#[cfg(windows)]
static RESTING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A minimized window, or one closed to the tray, kept drawing its pages in
/// WebView2: kryo.to's moving covers ran at full speed with nobody looking,
/// and loud fans were the result. A browser hides a minimized window's tabs;
/// this does the same, telling each view that was showing that it is not, so
/// Chromium stops drawing it and the page reads `document.hidden`.
#[cfg(windows)]
pub fn rest_views<R: Runtime>(window: &tauri::Window<R>, rest: bool) {
    if rest {
        if RESTING.lock().is_ok_and(|r| !r.is_empty()) {
            return;
        }
        for view in window.webviews() {
            let label = view.label().to_string();
            let _ = view.with_webview(move |w| unsafe {
                let controller = w.controller();
                let mut visible = windows::core::BOOL(0);
                // A view hidden already (the Store behind the Library) is not
                // ours to wake later.
                if controller.IsVisible(&mut visible).is_ok() && visible.as_bool() && controller.SetIsVisible(false).is_ok() {
                    if let Ok(mut resting) = RESTING.lock() {
                        resting.push(label);
                    }
                }
            });
        }
    } else {
        let labels = RESTING.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default();
        for view in window.webviews().into_iter().filter(|v| labels.iter().any(|l| l == v.label())) {
            let _ = view.with_webview(|w| unsafe {
                let _ = w.controller().SetIsVisible(true);
            });
        }
    }
}

#[cfg(not(windows))]
pub fn rest_views<R: Runtime>(_window: &tauri::Window<R>, _rest: bool) {}

/* ── Tray, and closing to it ──────────────────────────────── */

/// Set once the shell is past sign-in. Before that, closing the window quits:
/// hiding a sign-in screen in the tray would only look like it failed to close.
pub static SHELL_READY: AtomicBool = AtomicBool::new(false);

/// Whether the tray icon exists. Without one (a Linux desktop with no tray),
/// closing must quit: a hidden window with no icon to bring it back is lost.
pub static TRAY_OK: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub fn shell_ready(ready: bool) {
    SHELL_READY.store(ready, Ordering::SeqCst);
}

/// A system notification (the corner pop-up and the notification centre).
///
/// Sent from here and not through the notification plugin on purpose: that
/// plugin swaps `window.Notification` for its own in every frame of every web
/// view, the Store's pages and Cloudflare's challenge frame included, and
/// Turnstile reads a replaced browser API as a tampered browser. Downloads then
/// failed with "Could not verify this browser".
#[tauri::command(async)]
pub fn os_notify(app: AppHandle, title: String, body: Option<String>) {
    let mut note = notify_rust::Notification::new();
    note.summary(&title);
    if let Some(body) = body.filter(|b| !b.trim().is_empty()) {
        note.body(&body);
    }
    note.auto_icon();
    // Windows files a toast under the installed app's ID; a build run from
    // `target/` has no shortcut carrying that ID, so it leaves the default.
    #[cfg(windows)]
    if !cfg!(debug_assertions) {
        note.app_id(&app.config().identifier);
    }
    #[cfg(not(windows))]
    let _ = &app;
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = note.show() {
            crate::logging::error("notify", &e.to_string());
        }
    });
}

/// The system's file or folder picker, over the main window. Our own for the
/// same reason as `os_notify`: the dialog plugin also swaps `alert` and
/// `confirm` in every frame of the Store.
#[tauri::command]
pub async fn pick_path(
    window: tauri::Window,
    title: Option<String>,
    directory: bool,
    default_path: Option<String>,
    extensions: Option<Vec<String>>,
) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let parent = window.clone();
    // Built on the main thread (GTK insists), awaited off it.
    window
        .run_on_main_thread(move || {
            let mut dialog = rfd::AsyncFileDialog::new().set_parent(&parent).set_can_create_directories(true);
            if let Some(title) = title {
                dialog = dialog.set_title(title);
            }
            if let Some(dir) = default_path.filter(|p| std::path::Path::new(p).is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            if let Some(ext) = extensions.filter(|e| !e.is_empty()) {
                let ext: Vec<&str> = ext.iter().map(String::as_str).collect();
                dialog = dialog.add_filter("Games", &ext);
            }
            type Picked = std::pin::Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>>;
            let picked: Picked = if directory { Box::pin(dialog.pick_folder()) } else { Box::pin(dialog.pick_file()) };
            std::thread::spawn(move || {
                let path = tauri::async_runtime::block_on(picked).map(|h| h.path().display().to_string());
                let _ = tx.send(path);
            });
        })
        .map_err(|e| e.to_string())?;
    rx.await.map_err(|_| "The picker closed unexpectedly.".to_string())
}

#[tauri::command]
pub fn app_exit(app: AppHandle) {
    crate::presence::disconnect(&app);
    crate::logging::info("app", "exit");
    app.exit(0);
}

pub fn build_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    // First, because on Linux the tray only opens its menu - a click on the
    // icon itself is not delivered there.
    let open = MenuItem::with_id(app, "tray-open", "Open Kryoto", true, None::<&str>)?;
    let store = MenuItem::with_id(app, "tray-store", "Store", true, None::<&str>)?;
    let library = MenuItem::with_id(app, "tray-library", "Library", true, None::<&str>)?;
    let downloads = MenuItem::with_id(app, "tray-downloads", "Downloads", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "tray-settings", "Settings", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "tray-exit", "Exit Kryoto", true, None::<&str>)?;
    let sep_top = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &sep_top, &store, &library, &downloads, &settings, &sep, &quit])?;
    let mut builder = TrayIconBuilder::with_id("kryoto")
        .tooltip(if cfg!(debug_assertions) { "Kryoto Desktop Dev" } else { "Kryoto Desktop" })
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if id == "tray-exit" {
                app.exit(0);
                return;
            }
            show_main(app);
            if id != "tray-open" {
                let _ = app.emit_to("main", "tray-go", id.trim_start_matches("tray-"));
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    TRAY_OK.store(true, Ordering::SeqCst);
    Ok(())
}

/// Unread chat messages, on the tray icon's tooltip and (Windows) as a dot on
/// the taskbar button; elsewhere as the dock/launcher badge where supported.
pub fn set_unread_badge<R: Runtime>(app: &AppHandle<R>, unread: u32) {
    let base = if cfg!(debug_assertions) { "Kryoto Desktop Dev" } else { "Kryoto Desktop" };
    if let Some(tray) = app.tray_by_id("kryoto") {
        let tip = match unread {
            0 => base.to_string(),
            1 => format!("{base} - 1 unread message"),
            n => format!("{base} - {n} unread messages"),
        };
        let _ = tray.set_tooltip(Some(tip));
    }
    // `get_window`: the main window holds several views, and
    // `get_webview_window` finds none for it.
    let Some(window) = app.get_window("main") else { return };
    #[cfg(windows)]
    {
        let _ = window.set_overlay_icon((unread > 0).then(unread_dot));
    }
    #[cfg(not(windows))]
    {
        let _ = window.set_badge_count((unread > 0).then_some(i64::from(unread)));
    }
}

/// A 16 px round dot for the taskbar overlay, drawn here (no asset to ship).
#[cfg(windows)]
fn unread_dot() -> tauri::image::Image<'static> {
    const N: u32 = 16;
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    let c = (N as f32 - 1.0) / 2.0;
    for y in 0..N {
        for x in 0..N {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            // White ring, then red fill, soft edge.
            let (r, g, b) = if d > c - 2.0 { (255, 255, 255) } else { (229, 57, 53) };
            let a = ((c + 0.5 - d).clamp(0.0, 1.0) * 255.0) as u8;
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    tauri::image::Image::new_owned(rgba, N, N)
}

/// The main window's close button: to the tray once signed in (Steam's
/// default), unless Settings says quit. A move or resize closes an open menu
/// and tells the shell when the window became (or stopped being) maximized or
/// full screen, so the title bar never has to ask on every resize.
pub fn on_main_window_event<R: Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
    let app = window.app_handle();
    match event {
        WindowEvent::CloseRequested { api, .. } => {
            let to_tray = SHELL_READY.load(Ordering::SeqCst) && TRAY_OK.load(Ordering::SeqCst);
            if to_tray && crate::settings::load(app).close_to_tray {
                api.prevent_close();
                crate::menus::close(app);
                let _ = window.hide();
                rest_views(window, true);
            } else {
                app.exit(0);
            }
        }
        // Only a real move or resize: Linux window managers send the same
        // geometry again on focus and stacking changes.
        WindowEvent::Moved(p) if changed(&LAST_POS, (p.x, p.y)) => crate::menus::close(app),
        WindowEvent::Resized(s) if changed(&LAST_SIZE, (s.width as i32, s.height as i32)) => {
            crate::menus::close(app);
            report_window_state(window);
            // Minimizing is a resize too; so is coming back.
            rest_views(window, window.is_minimized().unwrap_or(false));
        }
        // Back from the taskbar or the tray without a resize.
        WindowEvent::Focused(true) => rest_views(window, false),
        _ => {}
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    maximized: bool,
    fullscreen: bool,
}

static LAST_STATE: Mutex<Option<WindowState>> = Mutex::new(None);

fn window_state<R: Runtime>(window: &tauri::Window<R>) -> WindowState {
    WindowState {
        maximized: window.is_maximized().unwrap_or(false),
        fullscreen: window.is_fullscreen().unwrap_or(false),
    }
}

fn report_window_state<R: Runtime>(window: &tauri::Window<R>) {
    let now = window_state(window);
    let Ok(mut last) = LAST_STATE.lock() else { return };
    if last.replace(now) != Some(now) {
        let _ = window.app_handle().emit_to("main", "window-state", now);
    }
}

/// Whether the main window is maximized or full screen right now.
#[tauri::command]
pub fn window_state_get(window: tauri::Window) -> WindowState {
    window_state(&window)
}

static LAST_POS: Mutex<Option<(i32, i32)>> = Mutex::new(None);
static LAST_SIZE: Mutex<Option<(i32, i32)>> = Mutex::new(None);

/// Record `now` and say whether it differs from what was recorded before.
fn changed(last: &Mutex<Option<(i32, i32)>>, now: (i32, i32)) -> bool {
    let Ok(mut last) = last.lock() else { return true };
    let was = last.replace(now);
    was != Some(now)
}

/* ── Window corners ───────────────────────────────────────── */

/// Windows 11 rounds a frameless window's corners and draws its shadow
/// itself when asked - the same corners as every other window, with no
/// see-through edge to paint. `small` is the tighter radius Windows uses for
/// menus. Older Windows keeps square corners. Linux has no such call: its window
/// is see-through (tauri.linux.conf.json) and the page rounds itself (styles.css).
pub fn round_corners<R: Runtime>(window: &tauri::Window<R>, small: bool) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE};
        if let Ok(hwnd) = window.hwnd() {
            // DWMWCP_ROUND = 2, DWMWCP_ROUNDSMALL = 3.
            let pref: i32 = if small { 3 } else { 2 };
            // SAFETY: a live window handle and a pointer to a 4-byte value.
            unsafe {
                DwmSetWindowAttribute(
                    hwnd.0 as _,
                    DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                    (&pref as *const i32).cast(),
                    std::mem::size_of::<i32>() as u32,
                );
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::gdk::WindowState;
        use gtk::glib::Propagation;
        use gtk::prelude::*;
        // GNOME's own radius for a window it rounds.
        const RADIUS: f64 = 12.0;
        const SQUARE: WindowState = WindowState::MAXIMIZED
            .union(WindowState::FULLSCREEN)
            .union(WindowState::TILED);
        let _ = small;
        if let Ok(win) = window.gtk_window() {
            // The window is see-through (tauri.linux.conf.json) so the page can
            // round its own corners, but the views in it are separate surfaces:
            // where one is transparent (the menu's corners over the Store) it
            // shows the window, and a see-through window shows the desktop.
            // So the window is filled, in the shell's colour, inside the same
            // rounded shape the page draws.
            win.connect_draw(|win, cr| {
                let fill = WINDOW_FILL.lock().map(|f| *f).unwrap_or_default();
                let square = win.window().is_none_or(|g| g.state().intersects(SQUARE));
                let (w, h) = (f64::from(win.allocated_width()), f64::from(win.allocated_height()));
                let r = if square { 0.0 } else { RADIUS };
                let q = std::f64::consts::FRAC_PI_2;
                cr.new_sub_path();
                cr.arc(w - r, r, r, -q, 0.0);
                cr.arc(w - r, h - r, r, 0.0, q);
                cr.arc(r, h - r, r, q, 2.0 * q);
                cr.arc(r, r, r, 2.0 * q, 3.0 * q);
                cr.close_path();
                let rgb = |c: [u8; 3]| (f64::from(c[0]) / 255.0, f64::from(c[1]) / 255.0, f64::from(c[2]) / 255.0);
                let (sr, sg, sb) = rgb(fill.store);
                cr.set_source_rgb(sr, sg, sb);
                let _ = cr.fill_preserve();
                // Above the Store's top edge it is the shell's own colour (the
                // nav bar), so a menu's see-through corners over it match.
                cr.clip();
                let (hr, hg, hb) = rgb(fill.shell);
                cr.set_source_rgb(hr, hg, hb);
                cr.rectangle(0.0, 0.0, w, fill.store_top.clamp(0.0, h));
                let _ = cr.fill();
                Propagation::Proceed
            });
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = (window, small);
}

/// What the Linux window is filled with behind its views: the shell's colour
/// down to where the Store starts, the Store's colour below it.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Default)]
struct Fill {
    shell: [u8; 3],
    store: [u8; 3],
    store_top: f64,
}

#[cfg(target_os = "linux")]
static WINDOW_FILL: Mutex<Fill> = Mutex::new(Fill { shell: [10; 3], store: [10; 3], store_top: 0.0 });

#[cfg(target_os = "linux")]
pub fn set_window_fill<R: Runtime>(window: &tauri::Window<R>, shell: [u8; 3], store: [u8; 3]) {
    use gtk::prelude::WidgetExt;
    if let Ok(mut f) = WINDOW_FILL.lock() {
        f.shell = shell;
        f.store = store;
    }
    if let Ok(win) = window.gtk_window() {
        win.queue_draw();
    }
}

/// Where the Store's view starts, from the top of the window.
#[cfg(target_os = "linux")]
pub fn set_fill_split(store_top: f64) {
    if let Ok(mut f) = WINDOW_FILL.lock() {
        f.store_top = store_top;
    }
}

/* ── Starting with the computer ───────────────────────────── */

#[cfg(windows)]
pub fn set_autostart(on: bool) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = wide(if cfg!(debug_assertions) { "Kryoto Desktop Dev" } else { "Kryoto Desktop" });
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let value = wide(&format!("\"{}\" --tray", exe.display()));
        // SAFETY: NUL-terminated UTF-16 strings; the size is the value's bytes.
        let rc = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        if rc != 0 {
            return Err(format!("Windows refused (error {rc})"));
        }
    } else {
        // SAFETY: NUL-terminated UTF-16 strings.
        let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
        // 2 = it was not there, which is the state we wanted.
        if rc != 0 && rc != 2 {
            return Err(format!("Windows refused (error {rc})"));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn set_autostart(on: bool) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "No home folder.".to_string())?;
    let dir = std::path::Path::new(&home).join(".config/autostart");
    let file = dir.join("kryoto-desktop.desktop");
    if on {
        // An AppImage runs from a temporary mount; $APPIMAGE is the file itself.
        let exe = match std::env::var_os("APPIMAGE") {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_exe().map_err(|e| e.to_string())?,
        };
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(
            &file,
            format!("[Desktop Entry]\nType=Application\nName=Kryoto Desktop\nExec=\"{}\" --tray\nX-GNOME-Autostart-enabled=true\n", exe.display()),
        )
        .map_err(|e| e.to_string())
    } else {
        match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

/* ── The computer's own theme ─────────────────────────────── */

/// Whether the computer is set to dark mode, for the "System" theme. The
/// webview can't tell: its prefers-color-scheme reports the window's theme,
/// which the page sets. None when it can't be read.
#[cfg(windows)]
pub fn os_dark() -> Option<bool> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: NUL-terminated UTF-16 strings; `value` holds the DWORD asked for.
    let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut value as *mut u32).cast(), &mut size) };
    (rc == 0).then_some(value == 0)
}

/// GNOME's (and most desktops') color-scheme preference.
#[cfg(not(windows))]
pub fn os_dark() -> Option<bool> {
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "color-scheme"]).output().ok()?;
    let scheme = String::from_utf8_lossy(&out.stdout);
    match scheme.trim().trim_matches('\'') {
        "prefer-dark" => Some(true),
        "prefer-light" | "default" => Some(false),
        _ => None,
    }
}
