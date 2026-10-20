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
    }
}

/* ── Tray, and closing to it ──────────────────────────────── */

/// Set once the shell is past sign-in. Before that, closing the window quits:
/// hiding a sign-in screen in the tray would only look like it failed to close.
pub static SHELL_READY: AtomicBool = AtomicBool::new(false);

/// Whether the tray icon exists. Without one (a desktop with no tray),
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
    // First, because some desktop shells only open the menu from a tray click;
    // the icon click handler below makes the window return everywhere else.
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
    let Some(window) = app.get_webview_window("main") else { return };
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
            } else {
                app.exit(0);
            }
        }
        // Only a real move or resize: desktop window managers can send the
        // same geometry again on focus and stacking changes.
        WindowEvent::Moved(p) if changed(&LAST_POS, (p.x, p.y)) => crate::menus::close(app),
        WindowEvent::Resized(s) if changed(&LAST_SIZE, (s.width as i32, s.height as i32)) => {
            crate::menus::close(app);
            report_window_state(window);
        }
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
/// menus. Older Windows and non-Windows hosts keep square corners.
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
    #[cfg(not(windows))]
    let _ = (window, small);
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

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "macos")]
pub fn set_autostart(on: bool) -> Result<(), String> {
    let home = std::env::var_os("HOME").ok_or("No home folder.")?;
    let dir = std::path::PathBuf::from(home).join("Library/LaunchAgents");
    let label = if cfg!(debug_assertions) {
        "to.kryo.desktop.dev"
    } else {
        "to.kryo.desktop"
    };
    let file = dir.join(format!("{label}.plist"));
    if !on {
        return match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let xml = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    };
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{label}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--tray</string></array>\n<key>RunAtLoad</key><true/>\n<key>LimitLoadToSessionType</key><string>Aqua</string>\n</dict></plist>\n",
        xml(&exe.to_string_lossy())
    );
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(file, plist).map_err(|e| e.to_string())
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn set_autostart(_: bool) -> Result<(), String> {
    Err("Starting with the system is not supported on this operating system.".into())
}
