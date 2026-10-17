//! How the Linux build sets up WebKitGTK before GTK starts.
//!
//! WebKitGTK draws pages through its DMA-BUF renderer by default. That is the
//! fast path, and on Intel, AMD and nouveau it works. On NVIDIA's proprietary
//! driver it draws blank, torn or offset pages, or the web process dies at
//! start ("Failed to create GBM buffer"), and without a GPU render node (most
//! virtual machines, some remote sessions) it has nothing to share buffers
//! through. Turning it off everywhere, as the client used to, cost every
//! AMD and Intel machine its hardware acceleration; turning it off only for
//! NVIDIA (PR #7) left the other broken cases broken. So:
//!
//! * **Automatic** (the default): DMA-BUF rendering off, GPU compositing on.
//!   Measured on WebKitGTK 2.52 (Oct 2026) with kryo.to in the page: with
//!   DMA-BUF on, scrolling ran at 6 fps and opening a menu froze for 300 ms,
//!   the "really bad" lag people filmed whenever they opened anything in the
//!   Store; with it off, 60-70 fps and no long frames. It is also what
//!   NVIDIA's driver, machines without a render node and the AppImage needed.
//! * **Compatible**: turned off, and compositing too. The last resort that
//!   draws on anything, but slow: every frame is drawn on the CPU.
//! * **Full**: on, whatever the machine. For people who know theirs works.
//!
//! The AppImage carries its own WebKit, built on an older Ubuntu, while the
//! graphics drivers are the system's. On a rolling distribution with a newer
//! Mesa (Fedora 44 with Mesa 26 on AMD, Oct 2026) that WebKit's DMA-BUF
//! renderer aborted the web process on a C++ assertion, and the window went
//! blank or closed. Without DMA-BUF it still composites on the GPU. The .deb
//! and .rpm use the system's own WebKit and keep it.
//!
//! And it heals itself: every start leaves a marker that the shell removes
//! once it has drawn its first frame (`display_rendered`), and a web process
//! that crashes leaves one too (`note_renderer_crash`). A start that finds it
//! falls back to Compatible and stays there until the person picks another
//! mode. Environment variables the user sets always win.
//!
//! The decision is a pure function (`plan`) so it is tested on every
//! platform; only reading the machine and setting the variables is Linux.

// The decision is used by the Linux build only, and tested everywhere.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Auto,
    Compatible,
    Full,
}

impl Mode {
    fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "automatic" => Some(Mode::Auto),
            "compatible" | "safe" | "compat" => Some(Mode::Compatible),
            "full" | "fast" => Some(Mode::Full),
            _ => None,
        }
    }
}

/// What the machine looks like.
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts {
    /// NVIDIA's proprietary kernel module is loaded (not nouveau).
    pub nvidia: bool,
    /// `/dev/dri/renderD*` exists: a GPU WebKit can share buffers through.
    pub render_node: bool,
    /// A Wayland session.
    pub wayland: bool,
    /// The previous start never drew its first frame, or its renderer crashed.
    pub last_start_failed: bool,
    /// DMA-BUF rendering has failed on this machine before (sticky).
    pub dmabuf_failed: bool,
    /// Running from the AppImage, with its own WebKit and the system's drivers.
    pub appimage: bool,
}

/// What to set, and why, in words for the log and Settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub mode: Mode,
    /// The mode was switched to Compatible because a start failed to draw.
    pub healed: bool,
    pub set: Vec<(&'static str, &'static str)>,
    pub reason: String,
}

/// The decision. `chosen` is the saved mode (`KRYOTO_GPU` overrides it),
/// `is_set` says whether the user's environment already has a variable.
pub fn plan(chosen: Mode, env_mode: Option<&str>, is_set: impl Fn(&str) -> bool, facts: Facts) -> Plan {
    let mut mode = env_mode.and_then(Mode::parse).unwrap_or(chosen);
    let mut healed = false;
    // Automatic already has DMA-BUF off, so a start that failed to draw is
    // left with one step: drawing on the CPU.
    if mode == Mode::Auto && facts.last_start_failed {
        mode = Mode::Compatible;
        healed = true;
    }

    let mut set = Vec::new();
    // The client moves, centres and resizes its own frameless window, which a
    // Wayland client may not do; through XWayland it can.
    if facts.wayland && !is_set("GDK_BACKEND") {
        set.push(("GDK_BACKEND", "x11"));
    }

    let (dmabuf_off, compositing_off, reason) = match mode {
        Mode::Full => (false, false, "Full: hardware rendering, as chosen".to_string()),
        Mode::Compatible if healed => (
            true,
            true,
            "Compatible: the last start did not draw its window, so hardware rendering was turned off".to_string(),
        ),
        Mode::Compatible => (true, true, "Compatible: hardware rendering off, as chosen".to_string()),
        Mode::Auto if facts.nvidia => (true, false, "Automatic: NVIDIA driver, DMA-BUF rendering off".to_string()),
        Mode::Auto if !facts.render_node => (true, false, "Automatic: no GPU render node, DMA-BUF rendering off".to_string()),
        Mode::Auto if facts.dmabuf_failed => (
            true,
            false,
            "Automatic: DMA-BUF rendering failed here before, so it stays off (GPU compositing stays on)".to_string(),
        ),
        Mode::Auto if facts.appimage => (
            true,
            false,
            "Automatic: AppImage, DMA-BUF rendering off (its WebKit and this system's graphics drivers can disagree)".to_string(),
        ),
        Mode::Auto => (true, false, "Automatic: GPU compositing, DMA-BUF rendering off (with it on, pages stutter when they open)".to_string()),
    };
    if dmabuf_off && !is_set("WEBKIT_DISABLE_DMABUF_RENDERER") {
        set.push(("WEBKIT_DISABLE_DMABUF_RENDERER", "1"));
    }
    if compositing_off && !is_set("WEBKIT_DISABLE_COMPOSITING_MODE") {
        set.push(("WEBKIT_DISABLE_COMPOSITING_MODE", "1"));
    }
    let overridden = ["WEBKIT_DISABLE_DMABUF_RENDERER", "WEBKIT_DISABLE_COMPOSITING_MODE"].iter().any(|k| is_set(k));
    let reason = if overridden { format!("{reason} (your environment variables take precedence)") } else { reason };
    Plan { mode, healed, set, reason }
}

/// `/proc/modules` text has NVIDIA's proprietary module (`nvidia`, not `nouveau`).
pub fn modules_have_nvidia(modules: &str) -> bool {
    modules.lines().any(|l| l.split_whitespace().next() == Some("nvidia"))
}

// ---- The machine (Linux) ------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayState {
    /// The mode saved in Settings.
    pub mode: Mode,
    /// What this run uses and why.
    pub reason: String,
    pub healed: bool,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    pub static STATE: OnceLock<DisplayState> = OnceLock::new();

    /// The app's data folder, worked out before Tauri exists (the same place
    /// Tauri's `app_data_dir` gives later: `$XDG_DATA_HOME/<identifier>`).
    pub fn data_dir() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))?;
        Some(base.join("to.kryo.desktop"))
    }

    pub fn mode_file() -> Option<PathBuf> {
        data_dir().map(|d| d.join("display-mode"))
    }

    pub fn probe_file() -> Option<PathBuf> {
        data_dir().map(|d| d.join("display-probe"))
    }

    /// DMA-BUF failed here once; Automatic keeps it off from then on.
    pub fn dmabuf_failed_file() -> Option<PathBuf> {
        data_dir().map(|d| d.join("display-dmabuf-failed"))
    }

    /// A web view's renderer crashed during the last session. Unlike the probe,
    /// a page that reloads and draws again does not clear it; the next start does.
    pub fn crashed_file() -> Option<PathBuf> {
        data_dir().map(|d| d.join("display-crashed"))
    }

    /// Written once the gentler healing below has been applied to this machine.
    fn v2_file() -> Option<PathBuf> {
        data_dir().map(|d| d.join("display-v2"))
    }

    pub fn saved_mode() -> Mode {
        mode_file()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| Mode::parse(&s))
            .unwrap_or_default()
    }

    /// Once per machine: a Compatible mode the old healing chose gets one more
    /// chance at the GPU.
    ///
    /// Before 0.2.23 a single failed start switched to Compatible for good,
    /// which draws everything on the CPU. A saved "compatible" from then cannot
    /// be told apart from one somebody picked, so it goes back to Automatic
    /// with DMA-BUF off - the step that fixes nearly every machine - and if
    /// that fails to draw too, the next start is Compatible again.
    fn migrate() {
        let Some(marker) = v2_file() else { return };
        if marker.exists() {
            return;
        }
        if let Some(f) = mode_file() {
            if std::fs::read_to_string(&f).is_ok_and(|s| Mode::parse(&s) == Some(Mode::Compatible)) {
                let _ = std::fs::write(&f, "auto");
                if let Some(d) = dmabuf_failed_file() {
                    let _ = std::fs::write(d, "1");
                }
            }
        }
        let _ = std::fs::create_dir_all(marker.parent().unwrap_or(&marker));
        let _ = std::fs::write(marker, "1");
    }

    fn facts() -> Facts {
        let nvidia = std::path::Path::new("/sys/module/nvidia").exists()
            || std::fs::read_to_string("/proc/modules").map(|m| modules_have_nvidia(&m)).unwrap_or(false);
        let render_node = std::fs::read_dir("/dev/dri")
            .map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("renderD")))
            .unwrap_or(false);
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        let last_start_failed = probe_file().is_some_and(|p| p.exists()) || crashed_file().is_some_and(|p| p.exists());
        let dmabuf_failed = dmabuf_failed_file().is_some_and(|p| p.exists());
        let appimage = std::env::var_os("APPIMAGE").is_some_and(|v| !v.is_empty());
        Facts { nvidia, render_node, wayland, last_start_failed, dmabuf_failed, appimage }
    }

    pub fn apply() {
        migrate();
        let saved = saved_mode();
        let env_mode = std::env::var("KRYOTO_GPU").ok();
        let is_set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        let p = plan(saved, env_mode.as_deref(), is_set, facts());
        for (k, v) in &p.set {
            std::env::set_var(k, v);
        }
        if p.healed {
            // Sticky: a machine that failed twice keeps the mode that works,
            // until the person picks another one in Settings.
            if let Some(f) = mode_file() {
                let _ = std::fs::create_dir_all(f.parent().unwrap_or(&f));
                let _ = std::fs::write(f, "compatible");
            }
        }
        // Counted now; the crash belonged to the last session.
        if let Some(f) = crashed_file() {
            let _ = std::fs::remove_file(f);
        }
        // This start's marker; the shell removes it once it has drawn.
        if let Some(f) = probe_file() {
            let _ = std::fs::create_dir_all(f.parent().unwrap_or(&f));
            let _ = std::fs::write(f, "starting");
        }
        let mode = if p.healed { Mode::Compatible } else { saved };
        let _ = STATE.set(DisplayState { mode, reason: p.reason, healed: p.healed });
    }
}

/// Before GTK starts: pick the renderer and set the variables (Linux only).
pub fn apply() {
    #[cfg(target_os = "linux")]
    linux::apply();
}

/// For the log, once logging is up.
#[cfg(target_os = "linux")]
pub fn describe() -> Option<String> {
    linux::STATE.get().map(|s| s.reason.clone())
}
#[cfg(not(target_os = "linux"))]
pub fn describe() -> Option<String> {
    None
}

/// A web view's renderer crashed (WebKit's `web-process-terminated`).
///
/// Counted like a start that never drew, so the next start takes the next
/// healing step. Kept in its own marker: the reloaded page draws again and
/// clears the probe, which must not erase the crash. The view itself is
/// reloaded by the caller; a crash mid-session should cost a reload, not the app.
pub fn note_renderer_crash() {
    #[cfg(target_os = "linux")]
    if let Some(f) = linux::crashed_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap_or(&f));
        let _ = std::fs::write(f, "renderer crashed");
    }
}

/// Watch one web view for a crashed renderer: note it for the next start and
/// reload the page, at most a few times a minute so a page that crashes at
/// once does not spin.
#[cfg(target_os = "linux")]
pub fn watch_crashes(view: &tauri::Webview) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use webkit2gtk::{WebProcessTerminationReason, WebViewExt};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let label = view.label().to_string();
    let _ = view.with_webview(move |w| {
        let wv = w.inner();
        wv.connect_web_process_terminated(move |wv, reason| {
            if reason == WebProcessTerminationReason::TerminatedByApi {
                return;
            }
            note_renderer_crash();
            crate::logging::error(
                "display",
                &format!("the {label} view's web process ended ({reason:?}); reloading it, and the next start turns DMA-BUF rendering off"),
            );
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now.saturating_sub(LAST.swap(now, Ordering::Relaxed)) >= 20 {
                wv.reload();
            }
        });
    });
}
#[cfg(not(target_os = "linux"))]
pub fn watch_crashes(_view: &tauri::Webview) {}

/// The shell drew its first frame: this start worked.
#[tauri::command]
pub fn display_rendered() {
    #[cfg(target_os = "linux")]
    if let Some(f) = linux::probe_file() {
        let _ = std::fs::remove_file(f);
    }
}

/// What Settings shows: the saved mode and what this run uses. None off Linux.
#[cfg(target_os = "linux")]
#[tauri::command]
pub fn display_state() -> Option<DisplayState> {
    linux::STATE.get().cloned().map(|mut s| {
        s.mode = linux::saved_mode();
        s
    })
}
#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub fn display_state() -> Option<DisplayState> {
    None
}

/// Save a mode for the next start.
#[cfg(target_os = "linux")]
#[tauri::command]
pub fn display_set_mode(mode: Mode) -> Result<(), String> {
    let f = linux::mode_file().ok_or("No data folder")?;
    std::fs::create_dir_all(f.parent().unwrap_or(&f)).map_err(|e| e.to_string())?;
    let text = match mode {
        Mode::Auto => "auto",
        Mode::Compatible => "compatible",
        Mode::Full => "full",
    };
    std::fs::write(f, text).map_err(|e| e.to_string())
}
#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub fn display_set_mode(mode: Mode) -> Result<(), String> {
    let _ = mode;
    Err("Only the Linux build has graphics modes.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> bool {
        false
    }
    fn vars(p: &Plan) -> Vec<&str> {
        p.set.iter().map(|(k, _)| *k).collect()
    }

    #[test]
    fn automatic_keeps_the_gpu_but_not_dmabuf() {
        // AMD and Intel too: DMA-BUF made kryo.to stutter in the Store.
        let p = plan(Mode::Auto, None, none, Facts { render_node: true, ..Facts::default() });
        assert_eq!(vars(&p), ["WEBKIT_DISABLE_DMABUF_RENDERER"], "{p:?}");
        assert!(p.reason.contains("GPU compositing"));
    }

    #[test]
    fn nvidia_and_no_render_node_turn_dmabuf_off() {
        let p = plan(Mode::Auto, None, none, Facts { nvidia: true, render_node: true, ..Facts::default() });
        assert_eq!(vars(&p), ["WEBKIT_DISABLE_DMABUF_RENDERER"]);
        let p = plan(Mode::Auto, None, none, Facts::default());
        assert_eq!(vars(&p), ["WEBKIT_DISABLE_DMABUF_RENDERER"]);
    }

    #[test]
    fn a_failed_start_heals_to_compatible() {
        let p = plan(Mode::Auto, None, none, Facts { render_node: true, last_start_failed: true, ..Facts::default() });
        assert!(p.healed);
        assert_eq!(p.mode, Mode::Compatible);
        assert_eq!(vars(&p), ["WEBKIT_DISABLE_DMABUF_RENDERER", "WEBKIT_DISABLE_COMPOSITING_MODE"]);
        // A mode the person chose is theirs: no healing over it.
        let p = plan(Mode::Full, None, none, Facts { nvidia: true, last_start_failed: true, ..Facts::default() });
        assert!(!p.healed);
        assert!(vars(&p).is_empty());
    }

    #[test]
    fn the_environment_wins() {
        let set = |k: &str| k == "WEBKIT_DISABLE_DMABUF_RENDERER" || k == "GDK_BACKEND";
        let p = plan(Mode::Auto, None, set, Facts { nvidia: true, wayland: true, ..Facts::default() });
        assert!(vars(&p).is_empty(), "{p:?}");
        assert!(p.reason.contains("environment"));
        let p = plan(Mode::Auto, Some("safe"), none, Facts { render_node: true, ..Facts::default() });
        assert_eq!(p.mode, Mode::Compatible);
        let p = plan(Mode::Compatible, Some("full"), none, Facts { nvidia: true, ..Facts::default() });
        assert!(vars(&p).is_empty());
    }

    #[test]
    fn the_appimage_keeps_dmabuf_off_but_the_gpu_on() {
        let p = plan(Mode::Auto, None, none, Facts { render_node: true, appimage: true, ..Facts::default() });
        assert_eq!(vars(&p), ["WEBKIT_DISABLE_DMABUF_RENDERER"]);
        assert!(p.reason.contains("AppImage"));
        // Full means full, AppImage or not.
        let p = plan(Mode::Full, None, none, Facts { render_node: true, appimage: true, ..Facts::default() });
        assert!(vars(&p).is_empty());
    }

    #[test]
    fn wayland_goes_through_xwayland() {
        let p = plan(Mode::Auto, None, none, Facts { wayland: true, render_node: true, ..Facts::default() });
        assert_eq!(p.set, [("GDK_BACKEND", "x11"), ("WEBKIT_DISABLE_DMABUF_RENDERER", "1")]);
    }

    #[test]
    fn nouveau_is_not_nvidia() {
        assert!(modules_have_nvidia("nvidia_drm 1 0 - Live\nnvidia 5 2 nvidia_drm, Live\n"));
        assert!(!modules_have_nvidia("nouveau 2 0 - Live\nnvidia_uvm 1 0 - Live\n"));
    }
}
