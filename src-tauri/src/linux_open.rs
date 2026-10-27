//! Opening a folder, file or link with the desktop's own app, on Linux.
//!
//! A bare `xdg-open` failed in three ways players hit (kryoto-bugs KB-0005,
//! KB-0009: "Browse local files does nothing"):
//!
//! * From the AppImage, the child inherits the AppImage's library paths
//!   (`LD_LIBRARY_PATH`, `GIO_MODULE_DIR`, `GTK_PATH`, `XDG_DATA_DIRS`...), so
//!   the file manager or the helper xdg-open picks loads our bundled GTK and
//!   dies before it draws.
//! * On KDE, xdg-open asks `qtpaths`, which many installs lack
//!   ("xdg-mime: qtpaths: command not found"), and gives up.
//! * Nothing checked the result: `spawn()` succeeds as soon as xdg-open starts,
//!   so the button just did nothing.
//!
//! So: a clean environment, then xdg-open, gio, kde-open and finally the
//! desktop portal over D-Bus, each given a moment to fail before the next one
//! is tried. The first that starts the handler (or exits 0) wins.

use std::ffi::OsString;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long an opener gets to fail. Most exit 0 within a few hundred ms; one
/// still running after this has started the app and is waiting on it.
const PATIENCE: Duration = Duration::from_millis(2500);

/// Variables the AppImage runtime and `display_env` set for this process only.
const OURS: [&str; 6] = ["APPDIR", "APPIMAGE", "ARGV0", "OWD", "WEBKIT_DISABLE_DMABUF_RENDERER", "WEBKIT_DISABLE_COMPOSITING_MODE"];

/// `path`-style lists the AppImage prefixes with its own folders.
const LISTS: [&str; 10] = [
    "LD_LIBRARY_PATH",
    "PATH",
    "XDG_DATA_DIRS",
    "GTK_PATH",
    "GIO_MODULE_DIR",
    "GSETTINGS_SCHEMA_DIR",
    "GDK_PIXBUF_MODULE_FILE",
    "GTK_EXE_PREFIX",
    "GTK_DATA_PREFIX",
    "PYTHONPATH",
];

/// The environment for a child that should behave like one the desktop started.
fn clean(cmd: &mut Command) {
    let appdir = std::env::var_os("APPDIR").filter(|d| !d.is_empty());
    for key in OURS {
        cmd.env_remove(key);
    }
    // Forced for our own window (display_env.rs); a file manager on Wayland
    // should pick its own backend.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var("GDK_BACKEND").is_ok_and(|v| v == "x11") {
        cmd.env_remove("GDK_BACKEND");
    }
    let Some(appdir) = appdir else { return };
    let appdir = appdir.to_string_lossy().into_owned();
    for key in LISTS {
        let Some(value) = std::env::var_os(key) else { continue };
        let kept: Vec<String> = value
            .to_string_lossy()
            .split(':')
            .filter(|part| !part.is_empty() && !part.starts_with(&appdir))
            .map(str::to_owned)
            .collect();
        if kept.is_empty() {
            cmd.env_remove(key);
        } else {
            cmd.env(key, OsString::from(kept.join(":")));
        }
    }
}

/// Run one opener. `Some(true)`: it worked; `Some(false)`: it ran and failed;
/// `None`: it is not installed.
fn attempt(program: &str, args: &[&str]) -> Option<bool> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    clean(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(false),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if start.elapsed() >= PATIENCE => {
                // Still going: it has handed off to (or is) the app. Reap it
                // later so it does not linger as a zombie.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Some(true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return Some(false),
        }
    }
}

/// A path as the `file://` URI the portal and gio expect.
fn uri(target: &str) -> String {
    if target.contains("://") {
        return target.to_owned();
    }
    url::Url::from_file_path(target).map(|u| u.to_string()).unwrap_or_else(|_| format!("file://{target}"))
}

/// Open a URL or a path with the desktop's app for it.
pub fn open(target: &str) -> Result<(), String> {
    let as_uri = uri(target);
    let portal_arg = format!("'{}'", as_uri.replace('\'', "%27"));
    let tries: [(&str, Vec<&str>); 5] = [
        ("xdg-open", vec![target]),
        ("gio", vec!["open", &as_uri]),
        ("kde-open", vec![target]),
        ("kde-open5", vec![target]),
        (
            "gdbus",
            vec![
                "call",
                "--session",
                "--dest",
                "org.freedesktop.portal.Desktop",
                "--object-path",
                "/org/freedesktop/portal/desktop",
                "--method",
                "org.freedesktop.portal.OpenURI.OpenURI",
                "''",
                &portal_arg,
                "{}",
            ],
        ),
    ];
    let mut tried = Vec::new();
    for (program, args) in &tries {
        match attempt(program, args) {
            Some(true) => return Ok(()),
            Some(false) => tried.push(*program),
            None => {}
        }
    }
    Err(if tried.is_empty() {
        "No app to open it with: install xdg-utils.".into()
    } else {
        format!("Could not open it (tried {}).", tried.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::uri;

    /// On a real Linux desktop (or WSL with a handler registered):
    /// `KRYOTO_OPEN_TEST=/path cargo test linux_open -- --ignored`. Pretends to
    /// be the AppImage, so the handler must not see its library paths.
    #[test]
    #[ignore]
    fn opens_through_the_desktop_with_a_clean_environment() {
        let Ok(target) = std::env::var("KRYOTO_OPEN_TEST") else { return };
        std::env::set_var("APPDIR", "/tmp/.mount_fake");
        std::env::set_var("LD_LIBRARY_PATH", "/tmp/.mount_fake/usr/lib:/opt/keep");
        assert_eq!(super::open(&target), Ok(()));
    }

    #[test]
    fn paths_become_file_uris_and_links_stay_links() {
        assert_eq!(uri("/home/a b/Games"), "file:///home/a%20b/Games");
        assert_eq!(uri("https://kryo.to/game/x"), "https://kryo.to/game/x");
    }
}
