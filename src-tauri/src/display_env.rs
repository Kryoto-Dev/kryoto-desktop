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
//! * **Automatic** (the default): shared-memory buffers, GPU compositing on
//!   where supported. `WEBKIT_DMABUF_RENDERER_FORCE_SHM` avoids DMA-BUF buffer
//!   sharing without disabling the accelerated renderer. The old
//!   `WEBKIT_DISABLE_DMABUF_RENDERER` workaround also removed that renderer's
//!   backing store on current WebKitGTK: maximized pages and typing repainted
//!   the Store on the CPU, even though Settings said GPU compositing was on.
//! * **Compatible**: turned off, and compositing too. The last resort that
//!   draws on anything, but slow: every frame is drawn on the CPU.
//! * **Full**: on, whatever the machine. For people who know theirs works.
//!
//! The AppImage carries its own WebKit, built on an older Ubuntu, while the
//! graphics drivers are the system's. On a rolling distribution with a newer
//! Mesa (Fedora 44 with Mesa 26 on AMD, Oct 2026) that WebKit's DMA-BUF
//! renderer aborted the web process on a C++ assertion, and the window went
//! blank or closed. Shared-memory buffers avoid that driver mismatch while
//! keeping the accelerated backing store available.
//!
//! And it heals itself: every start leaves a marker that the shell removes
//! once it has drawn its first frame (`display_rendered`). A start that finds
//! it falls back to Compatible and stays there until the person picks another
//! mode. A web process that crashes after the window drew (`note_renderer_crash`)
//! only keeps DMA-BUF off: until 0.3.1 it counted as a failed start too, and
//! one crash (KB-0116, an assertion inside the AppImage's WebKit that happens
//! with or without the GPU) left the machine drawing every frame on the CPU for
//! good, which is what made the Store's home page crawl at 20 fps (KB-0012).
//! Environment variables the user sets always win.
//!
//! Only a start that is going to show the window arms the marker (KB-0311).
//! Up to 0.3.6 every start wrote it before asking whether Kryoto was already
//! running, so a second launch (the app icon again, a `kryoto://` link, an
//! update's relaunch) left it behind for a window it never had. The next start
//! took that for a failed one and saved Compatible, which draws every frame on
//! the CPU, for good, and Settings called it "as chosen": the Store lagged and
//! the fans spun on any machine that had ever been started twice, while a fresh
//! install, a tester's machine or a clean WSL run never showed it. 0.3.7 arms
//! the marker after the instance check and not for a start to the tray, and
//! gives every saved Compatible one more start on the GPU (`migrate_v4`).
//!
//! A view whose renderer keeps crashing before its page ever loads (KB-0310)
//! is a start that cannot draw: after the third crash in a row the app saves
//! Compatible and restarts itself in it; if it was already drawing on the CPU
//! it stops reloading and says what is wrong instead of animating forever.
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
    // Automatic already bypasses DMA-BUF buffer sharing, so a failed start is
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

    let (shared_memory, compositing_off, reason) = match mode {
        Mode::Full => (false, false, "Full: hardware rendering, as chosen".to_string()),
        Mode::Compatible if healed => (
            false,
            true,
            "Compatible: the last start did not draw its window, so hardware rendering was turned off".to_string(),
        ),
        Mode::Compatible => (false, true, "Compatible: hardware rendering off, as chosen".to_string()),
        Mode::Auto if facts.nvidia => (true, false, "Automatic: NVIDIA driver, shared-memory buffers with GPU compositing".to_string()),
        Mode::Auto if !facts.render_node => (true, false, "Automatic: no GPU render node, shared-memory buffers".to_string()),
        Mode::Auto if facts.dmabuf_failed => (
            true,
            false,
            "Automatic: DMA-BUF buffer sharing failed here before, using shared-memory buffers with GPU compositing".to_string(),
        ),
        Mode::Auto if facts.appimage => (
            true,
            false,
            "Automatic: AppImage, shared-memory buffers (its WebKit and this system's graphics drivers can disagree)".to_string(),
        ),
        Mode::Auto => (true, false, "Automatic: shared-memory buffers with GPU compositing".to_string()),
    };
    if shared_memory && !is_set("WEBKIT_DMABUF_RENDERER_FORCE_SHM") && !is_set("WEBKIT_DISABLE_DMABUF_RENDERER") {
        set.push(("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1"));
    }
    if compositing_off && !is_set("WEBKIT_DISABLE_DMABUF_RENDERER") {
        set.push(("WEBKIT_DISABLE_DMABUF_RENDERER", "1"));
    }
    if compositing_off && !is_set("WEBKIT_DISABLE_COMPOSITING_MODE") {
        set.push(("WEBKIT_DISABLE_COMPOSITING_MODE", "1"));
    }
    let overridden = ["WEBKIT_DMABUF_RENDERER_FORCE_SHM", "WEBKIT_DISABLE_DMABUF_RENDERER", "WEBKIT_DISABLE_COMPOSITING_MODE"].iter().any(|k| is_set(k));
    let reason = if overridden { format!("{reason} (your environment variables take precedence)") } else { reason };
    Plan { mode, healed, set, reason }
}

/// What the markers the last session left mean: (`last_start_failed`, `dmabuf_failed`).
///
/// Only a start that never drew its window heals to Compatible. A renderer that
/// crashed after drawing keeps DMA-BUF off (Automatic has it off already), but
/// does not take the GPU away: on 0.3.0 the crashes went on in Compatible too,
/// and drawing on the CPU only made every page slow.
pub fn read_markers(never_drew: bool, renderer_crashed: bool, dmabuf_failed: bool) -> (bool, bool) {
    (never_drew, dmabuf_failed || renderer_crashed)
}

/// Set in the environment of a copy that the app started to replace itself
/// (`watch_crashes`): its instance check waits for the old copy's port
/// (system.rs), and it never restarts itself a second time.
pub const RELAUNCHED: &str = "KRYOTO_RELAUNCHED";

/// Whether this start should leave the "did it draw?" marker. Only a start
/// that shows its window can remove it: a copy that hands over to the one
/// already running, or a start to the tray, never draws (KB-0311).
pub fn arms_probe(first_copy: bool, to_tray: bool) -> bool {
    first_copy && !to_tray
}

/// What a view's renderer crash leads to (`watch_crashes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashStep {
    /// Load the page again.
    Reload,
    /// A crash soon after the last reload, of a page that had loaded: leave
    /// it (as before 0.3.7), so a page that crashes on something it plays
    /// does not spin.
    Leave,
    /// The page cannot load at all: start again drawing on the CPU.
    RestartCompatible,
    /// It cannot load even on the CPU: stop and say so.
    GiveUp,
}

/// `in_a_row`: crashes since the view last finished loading a page, this one
/// included. `since_reload`: seconds since this view was last reloaded after a
/// crash. `on_cpu`: compositing is already off. `relaunched`: this copy was
/// started by the app to replace itself.
pub fn crash_step(in_a_row: u32, since_reload: u64, on_cpu: bool, relaunched: bool) -> CrashStep {
    if in_a_row >= 3 {
        return if on_cpu || relaunched { CrashStep::GiveUp } else { CrashStep::RestartCompatible };
    }
    // A second crash before the page loaded must be reloaded too, or the
    // view stays dead and nobody learns that it cannot load.
    if in_a_row >= 2 || since_reload >= 20 {
        CrashStep::Reload
    } else {
        CrashStep::Leave
    }
}

// ---- Colour emoji fonts and the AppImage's WebKit (KB-0116) -------------
//
// The AppImage carries WebKitGTK 2.50.4 from Ubuntu 22.04, built with
// libstdc++'s hardening assertions. Its Skia reads the colour stops of a COLRv1
// font's gradients one past the end of a vector, and the assertion aborts the
// web process. Fedora installs its colour emoji as a COLRv1 font
// (`google-noto-color-emoji-fonts`, Noto-COLRv1.ttf), so on Fedora every page
// that drew an emoji could take the view down: the Store "on Download", the
// main view once 0.3.0 put emoji in chat, reactions and notes. Reproduced on
// Fedora 44 with the 0.3.0 AppImage's own WebKit; the system WebKit (2.54, the
// .rpm) draws the same page. So the AppImage alone hides COLRv1 fonts from its
// WebKit through a fontconfig file of its own, and puts a bitmap (CBDT) Noto
// Color Emoji in their place, which it carries (release.yml).

/// The version of a font file's COLR table, if it has one. `bytes` is the start
/// of the file (the table directory, and the table's first two bytes when they
/// are in it); a collection is read by its first font.
pub fn colr_version(bytes: &[u8]) -> Option<u16> {
    let at = colr_offset(bytes)?;
    bytes.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
}

/// Where a font file's COLR table starts, from its table directory (`bytes`,
/// the start of the file). The table itself is usually megabytes further on:
/// Fedora's Noto-COLRv1.ttf has it at 3.8 MB of 5 MB.
pub fn colr_offset(bytes: &[u8]) -> Option<usize> {
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let font = if bytes.get(..4) == Some(b"ttcf") { u32_at(12)? } else { 0 };
    let tables = u16_at(font + 4)? as usize;
    (0..tables.min(512)).find_map(|i| {
        let record = font + 12 + i * 16;
        (bytes.get(record..record + 4)? == b"COLR").then_some(())?;
        u32_at(record + 8)
    })
}

/// A font file's COLR version, read where the table really is. Reading only the
/// file's first 16 KB found no version for Fedora's emoji, so the 0.3.1
/// candidate never hid it and the AppImage still crashed (KB-0116).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn colr_version_of_file(path: &std::path::Path) -> Option<u16> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = vec![0u8; 16 * 1024];
    let n = file.read(&mut head).ok()?;
    head.truncate(n);
    if let Some(v) = colr_version(&head) {
        return Some(v);
    }
    let at = colr_offset(&head)?;
    file.seek(SeekFrom::Start(at as u64)).ok()?;
    let mut v = [0u8; 2];
    file.read_exact(&mut v).ok()?;
    Some(u16::from_be_bytes(v))
}

/// A font file worth opening to look for COLRv1: colour emoji, by name.
pub fn may_be_colour_emoji(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    (n.ends_with(".ttf") || n.ends_with(".otf") || n.ends_with(".ttc")) && (n.contains("emoji") || n.contains("colr"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The fontconfig file: the system's own, plus `extra_dir` (the emoji the
/// AppImage carries), minus the `rejected` font files.
pub fn font_config(system: &str, extra_dir: Option<&str>, rejected: &[String]) -> String {
    let mut out = String::from("<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n<!-- Written by Kryoto Desktop's AppImage at every start (display_env.rs, KB-0116). -->\n<fontconfig>\n");
    out += &format!("  <include ignore_missing=\"yes\">{}</include>\n", xml_escape(system));
    if let Some(dir) = extra_dir {
        out += &format!("  <dir>{}</dir>\n", xml_escape(dir));
    }
    out += "  <selectfont>\n    <rejectfont>\n";
    for file in rejected {
        out += &format!("      <glob>{}</glob>\n", xml_escape(file));
    }
    out += "    </rejectfont>\n  </selectfont>\n</fontconfig>\n";
    out
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
    /// The AppImage, with its own WebKit: it plays no video (ReleaseIntro.tsx, KB-0259).
    pub appimage: bool,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    pub static STATE: OnceLock<DisplayState> = OnceLock::new();

    /// Names of the variables this process set itself, for a copy it starts.
    const SET_BY_US: &str = "KRYOTO_DISPLAY_SET";

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

    /// DMA-BUF failed here once; Automatic keeps using shared-memory buffers.
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

    /// The fontconfig file the AppImage's WebKit reads, when one was needed.
    pub static FONT_SHIM: OnceLock<PathBuf> = OnceLock::new();

    /// COLRv1 font files fontconfig would hand to WebKit, from the usual font
    /// folders. Only colour-emoji-looking names are opened, and only the first
    /// few kilobytes of each, so a start pays next to nothing for it.
    fn colrv1_fonts() -> Vec<String> {
        let mut roots: Vec<PathBuf> = vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into()];
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
        if let Some(d) = data {
            roots.push(d.join("fonts"));
        }
        if let Some(h) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(h).join(".fonts"));
        }
        let mut found = Vec::new();
        let mut seen = 0usize;
        let mut stack: Vec<(PathBuf, u8)> = roots.into_iter().map(|r| (r, 0)).collect();
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                seen += 1;
                if seen > 20_000 {
                    return found;
                }
                let path = e.path();
                let Ok(kind) = e.file_type() else { continue };
                if kind.is_dir() && depth < 5 {
                    stack.push((path, depth + 1));
                } else if may_be_colour_emoji(&e.file_name().to_string_lossy()) && super::colr_version_of_file(&path) == Some(1) {
                    // Through a symlink fontconfig records the real file.
                    let real = std::fs::canonicalize(&path).unwrap_or(path.clone());
                    found.push(path.to_string_lossy().into_owned());
                    if real != path {
                        found.push(real.to_string_lossy().into_owned());
                    }
                }
            }
        }
        found
    }

    /// AppImage only, and not over a FONTCONFIG_FILE the person set.
    fn shim_fonts(appimage: bool) {
        if !appimage || std::env::var_os("FONTCONFIG_FILE").is_some_and(|v| !v.is_empty()) {
            return;
        }
        let system = "/etc/fonts/fonts.conf";
        if !std::path::Path::new(system).is_file() {
            return;
        }
        let rejected = colrv1_fonts();
        if rejected.is_empty() {
            return;
        }
        let carried = std::env::var_os("APPDIR")
            .map(|d| PathBuf::from(d).join("usr/share/kryoto/fonts"))
            .filter(|d| d.is_dir())
            .map(|d| d.to_string_lossy().into_owned());
        let Some(file) = data_dir().map(|d| d.join("fontconfig").join("fonts.conf")) else { return };
        let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
        if std::fs::write(&file, font_config(system, carried.as_deref(), &rejected)).is_ok() {
            std::env::set_var("FONTCONFIG_FILE", &file);
            let _ = FONT_SHIM.set(file);
        }
    }

    /// Written once 0.3.1's reset of crash-made Compatible modes has run here.
    fn v3_file() -> Option<PathBuf> {
        data_dir().map(|d| d.join("display-v3"))
    }

    /// Written once 0.3.7's reset of second-launch-made Compatible modes has run here.
    fn v4_file() -> Option<PathBuf> {
        data_dir().map(|d| d.join("display-v4"))
    }

    /// The saved Compatible was chosen by the app (a start that did not draw,
    /// a view that could not load), not by the person. Picking a mode in
    /// Settings removes it.
    pub fn healed_file() -> Option<PathBuf> {
        data_dir().map(|d| d.join("display-healed"))
    }

    /// Save Compatible for the next start, as the app's own choice.
    pub fn save_healed_compatible() {
        if let Some(f) = mode_file() {
            let _ = std::fs::create_dir_all(f.parent().unwrap_or(&f));
            let _ = std::fs::write(f, "compatible");
        }
        if let Some(f) = healed_file() {
            let _ = std::fs::write(f, "1");
        }
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
    /// with DMA-BUF buffer sharing bypassed, and if
    /// that fails to draw too, the next start is Compatible again.
    fn migrate() {
        migrate_v3();
        migrate_v4();
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

    /// Once per machine, in 0.3.1: a saved Compatible goes back to Automatic.
    ///
    /// Up to 0.3.0 one web-process crash after the window drew (KB-0116) saved
    /// Compatible for good, and the Store drew on the CPU ever after (KB-0012).
    /// That cannot be told apart from a Compatible somebody picked, so, as in
    /// `migrate` above, it gets one more start on the GPU; a window that then
    /// fails to draw heals back to Compatible on the next start.
    fn migrate_v3() {
        let Some(marker) = v3_file() else { return };
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

    /// Once per machine, in 0.3.7: a saved Compatible goes back to Automatic.
    ///
    /// Up to 0.3.6 a second launch while Kryoto ran left the start marker
    /// behind, and the next start saved Compatible (KB-0311). That cannot be
    /// told apart from a Compatible somebody picked, so, as in `migrate_v3`,
    /// it gets one more start on the GPU; a window that then fails to draw
    /// heals back to Compatible on the next start. A probe left by such a
    /// second launch is cleared too: it says nothing about this machine.
    fn migrate_v4() {
        let Some(marker) = v4_file() else { return };
        if marker.exists() {
            return;
        }
        if let Some(f) = mode_file() {
            if std::fs::read_to_string(&f).is_ok_and(|s| Mode::parse(&s) == Some(Mode::Compatible)) {
                let _ = std::fs::write(&f, "auto");
            }
        }
        if let Some(p) = probe_file() {
            let _ = std::fs::remove_file(p);
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
        let (last_start_failed, dmabuf_failed) = read_markers(
            probe_file().is_some_and(|p| p.exists()),
            crashed_file().is_some_and(|p| p.exists()),
            dmabuf_failed_file().is_some_and(|p| p.exists()),
        );
        let appimage = std::env::var_os("APPIMAGE").is_some_and(|v| !v.is_empty());
        Facts { nvidia, render_node, wayland, last_start_failed, dmabuf_failed, appimage }
    }

    pub fn apply(arm_probe: bool) {
        // Variables an earlier copy of Kryoto set, inherited through a restart
        // (an update's relaunch, `watch_crashes`): ours, not the person's, so
        // this start decides them again instead of taking them as an override.
        if let Some(ours) = std::env::var_os(SET_BY_US) {
            for name in ours.to_string_lossy().split(',').filter(|n| n.starts_with("WEBKIT_") || *n == "GDK_BACKEND") {
                std::env::remove_var(name);
            }
            std::env::remove_var(SET_BY_US);
        }
        migrate();
        let saved = saved_mode();
        let env_mode = std::env::var("KRYOTO_GPU").ok();
        let is_set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        let facts = facts();
        let mut p = plan(saved, env_mode.as_deref(), is_set, facts);
        if !p.healed && p.mode == Mode::Compatible && healed_file().is_some_and(|f| f.exists()) {
            p.reason = p.reason.replace("as chosen", "chosen by Kryoto after a start that could not draw (pick Automatic in Settings to try the GPU again)");
        }
        shim_fonts(facts.appimage);
        for (k, v) in &p.set {
            std::env::set_var(k, v);
        }
        if !p.set.is_empty() {
            std::env::set_var(SET_BY_US, p.set.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(","));
        }
        // Not part of the desktop's saved session: KDE (and any XSMP session
        // manager) restored Kryoto at login after a logout with it open, even
        // with Start with my computer off (kryoto-bugs KB-0011). GTK registers
        // only when SESSION_MANAGER is set; starting at login is the autostart
        // entry's job (system.rs), which follows the switch.
        std::env::remove_var("SESSION_MANAGER");
        if p.healed {
            // Sticky: a machine that failed twice keeps the mode that works,
            // until the person picks another one in Settings.
            save_healed_compatible();
        }
        // Counted now; the crash belonged to the last session. DMA-BUF stays
        // off from here on (`dmabuf_failed`), the GPU stays on.
        if let Some(f) = crashed_file() {
            if f.exists() {
                if let Some(d) = dmabuf_failed_file() {
                    let _ = std::fs::write(d, "1");
                }
            }
            let _ = std::fs::remove_file(f);
        }
        // This start's marker; the shell removes it once it has drawn. Only a
        // start that will show its window can do that (KB-0311).
        if let Some(f) = probe_file().filter(|_| arm_probe) {
            let _ = std::fs::create_dir_all(f.parent().unwrap_or(&f));
            let _ = std::fs::write(f, "starting");
        }
        let mode = if p.healed { Mode::Compatible } else { saved };
        let _ = STATE.set(DisplayState { mode, reason: p.reason, healed: p.healed, appimage: facts.appimage });
    }
}

/// Before GTK starts, and after the instance check: pick the renderer and set
/// the variables (Linux only). `arm_probe`: see `arms_probe`.
pub fn apply(arm_probe: bool) {
    #[cfg(target_os = "linux")]
    linux::apply(arm_probe);
    #[cfg(not(target_os = "linux"))]
    let _ = arm_probe;
}

/// For the log, once logging is up.
#[cfg(target_os = "linux")]
pub fn describe() -> Option<String> {
    linux::STATE.get().map(|s| match linux::FONT_SHIM.get() {
        Some(_) => format!("{}; COLRv1 emoji fonts hidden from the AppImage's WebKit", s.reason),
        None => s.reason.clone(),
    })
}

/// A child process (a game, a file manager) should see the system's fonts as
/// they are: the FONTCONFIG_FILE this process set for its own WebKit is not theirs.
pub fn remove_font_shim(cmd: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    if let Some(ours) = linux::FONT_SHIM.get() {
        if std::env::var_os("FONTCONFIG_FILE").is_some_and(|v| std::path::Path::new(&v) == ours.as_path()) {
            cmd.env_remove("FONTCONFIG_FILE");
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = cmd;
}
#[cfg(not(target_os = "linux"))]
pub fn describe() -> Option<String> {
    None
}

/// A web view's renderer crashed (WebKit's `web-process-terminated`).
///
/// The next start keeps DMA-BUF off (`read_markers`); it no longer draws on the
/// CPU for it. The view itself is reloaded by the caller; a crash mid-session
/// should cost a reload, not the app.
pub fn note_renderer_crash() {
    #[cfg(target_os = "linux")]
    if let Some(f) = linux::crashed_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap_or(&f));
        let _ = std::fs::write(f, "renderer crashed");
    }
}

/// Watch one web view for a crashed renderer: note it for the next start and
/// reload the page (`crash_step`). A view that crashes three times without
/// ever loading its page restarts the app drawing on the CPU, and if that
/// was already the case, stops and says what is wrong (KB-0310): before,
/// the start screen kept animating over a Store that would never come.
#[cfg(target_os = "linux")]
pub fn watch_crashes(view: &tauri::Webview) {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tauri::Manager;
    use webkit2gtk::{LoadEvent, WebProcessTerminationReason, WebViewExt};
    /// Per view: crashes since its last finished load, and its last reload.
    static VIEWS: Mutex<Option<HashMap<String, (u32, u64)>>> = Mutex::new(None);
    fn with<T>(label: &str, f: impl FnOnce(&mut (u32, u64)) -> T) -> T {
        let mut all = VIEWS.lock().unwrap_or_else(|e| e.into_inner());
        f(all.get_or_insert_with(HashMap::new).entry(label.to_string()).or_insert((0, 0)))
    }
    let label = view.label().to_string();
    let app = view.app_handle().clone();
    let _ = view.with_webview(move |w| {
        let wv = w.inner();
        let loaded = label.clone();
        wv.connect_load_changed(move |_, event| {
            if event == LoadEvent::Finished {
                with(&loaded, |v| v.0 = 0);
            }
        });
        wv.connect_web_process_terminated(move |wv, reason| {
            if reason == WebProcessTerminationReason::TerminatedByApi {
                return;
            }
            note_renderer_crash();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let (in_a_row, last_reload) = with(&label, |v| {
                v.0 += 1;
                *v
            });
            let on_cpu = std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_some_and(|v| !v.is_empty() && v != "0");
            let relaunched = std::env::var_os(RELAUNCHED).is_some();
            let step = crash_step(in_a_row, now.saturating_sub(last_reload), on_cpu, relaunched);
            crate::logging::error(
                "display",
                &format!(
                    "the {label} view's web process ended ({reason:?}), {in_a_row} in a row: {step:?} ({})",
                    describe().unwrap_or_default()
                ),
            );
            match step {
                CrashStep::Reload => {
                    with(&label, |v| v.1 = now);
                    wv.reload();
                }
                CrashStep::Leave => {}
                CrashStep::RestartCompatible => {
                    linux::save_healed_compatible();
                    crate::logging::error("display", "the page cannot load with hardware rendering: restarting with it off (Compatible)");
                    std::env::set_var(RELAUNCHED, "display");
                    app.restart();
                }
                CrashStep::GiveUp => {
                    if label != "main" {
                        use tauri::Emitter;
                        let url = wv.uri().map(|u| u.to_string()).unwrap_or_default();
                        let _ = app.emit_to(
                            "main",
                            "browser-error",
                            serde_json::json!({ "url": url, "reason": "The page's renderer keeps crashing on this system." }),
                        );
                    }
                    tell_it_cannot_draw(&app, wv, label == "main");
                }
            }
        });
    });
}

/// The last resort, drawn by GTK itself because the page cannot draw: what
/// happened and what to do. Without the main view there is nothing left to
/// use, so closing it quits.
#[cfg(target_os = "linux")]
fn tell_it_cannot_draw(app: &tauri::AppHandle, wv: &webkit2gtk::WebView, quit: bool) {
    use gtk::prelude::*;
    let parent = wv.toplevel().and_then(|w| w.downcast::<gtk::Window>().ok());
    let dialog = gtk::MessageDialog::new(
        parent.as_ref(),
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        gtk::MessageType::Error,
        gtk::ButtonsType::Close,
        "Kryoto Desktop cannot draw its window on this system",
    );
    dialog.set_title("Kryoto Desktop");
    let logs = linux::data_dir().map(|d| d.join("logs").display().to_string()).unwrap_or_default();
    dialog.set_secondary_text(Some(&format!(
        "Its web view (WebKitGTK) crashed three times before it could show anything, with hardware rendering on and off.\n\n\
         Update your system (graphics drivers and webkit2gtk), then start Kryoto Desktop again. \
         If it still happens, ask in #support on Discord and attach the log from {logs}."
    )));
    let app = app.clone();
    dialog.connect_response(move |d, _| {
        d.close();
        if quit {
            app.exit(1);
        }
    });
    dialog.show_all();
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
    // The person's choice from here on, not the app's.
    if let Some(h) = linux::healed_file() {
        let _ = std::fs::remove_file(h);
    }
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
    fn automatic_keeps_the_accelerated_renderer_with_shared_memory_buffers() {
        let p = plan(Mode::Auto, None, none, Facts { render_node: true, ..Facts::default() });
        assert_eq!(p.set, [("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1")], "{p:?}");
        assert!(p.reason.contains("GPU compositing"));
    }

    #[test]
    fn nvidia_and_no_render_node_use_shared_memory_buffers() {
        let p = plan(Mode::Auto, None, none, Facts { nvidia: true, render_node: true, ..Facts::default() });
        assert_eq!(vars(&p), ["WEBKIT_DMABUF_RENDERER_FORCE_SHM"]);
        let p = plan(Mode::Auto, None, none, Facts::default());
        assert_eq!(vars(&p), ["WEBKIT_DMABUF_RENDERER_FORCE_SHM"]);
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
    fn a_crash_after_drawing_keeps_the_gpu() {
        // KB-0012 / KB-0116: one crash used to mean the CPU for good.
        let (failed, dmabuf) = read_markers(false, true, false);
        assert!(!failed && dmabuf);
        let p = plan(Mode::Auto, None, none, Facts { render_node: true, last_start_failed: failed, dmabuf_failed: dmabuf, ..Facts::default() });
        assert!(!p.healed);
        assert_eq!(p.mode, Mode::Auto);
        assert_eq!(vars(&p), ["WEBKIT_DMABUF_RENDERER_FORCE_SHM"]);
        // A window that never drew still heals.
        let (failed, _) = read_markers(true, false, false);
        assert!(plan(Mode::Auto, None, none, Facts { render_node: true, last_start_failed: failed, ..Facts::default() }).healed);
    }

    #[test]
    fn only_a_start_that_shows_its_window_arms_the_probe() {
        // KB-0311: a second launch (icon, kryoto:// link, an update's relaunch)
        // left the marker, and the next start saved Compatible for good.
        assert!(arms_probe(true, false));
        assert!(!arms_probe(false, false));
        assert!(!arms_probe(true, true));
    }

    #[test]
    fn a_view_that_cannot_load_restarts_on_the_cpu_then_gives_up() {
        // The first crash reloads; a page that loaded and crashes again soon is left.
        assert_eq!(crash_step(1, u64::MAX, false, false), CrashStep::Reload);
        assert_eq!(crash_step(1, 5, false, false), CrashStep::Leave);
        assert_eq!(crash_step(1, 25, false, false), CrashStep::Reload);
        // Crashing again before it ever loaded: reload, then the CPU (KB-0310).
        assert_eq!(crash_step(2, 1, false, false), CrashStep::Reload);
        assert_eq!(crash_step(3, 1, false, false), CrashStep::RestartCompatible);
        // Already on the CPU, or already restarted once: say so, no loop.
        assert_eq!(crash_step(3, 1, true, false), CrashStep::GiveUp);
        assert_eq!(crash_step(3, 1, false, true), CrashStep::GiveUp);
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
    fn shared_memory_override_is_respected() {
        let set = |k: &str| k == "WEBKIT_DMABUF_RENDERER_FORCE_SHM";
        let p = plan(Mode::Auto, None, set, Facts { render_node: true, ..Facts::default() });
        assert!(p.set.is_empty());
        assert!(p.reason.contains("environment"));
    }

    #[test]
    fn a_previous_dmabuf_failure_keeps_the_accelerated_renderer() {
        let p = plan(Mode::Auto, None, none, Facts { render_node: true, dmabuf_failed: true, ..Facts::default() });
        assert_eq!(p.set, [("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1")]);
        assert!(!p.healed);
    }

    #[test]
    fn the_appimage_keeps_dmabuf_off_but_the_gpu_on() {
        let p = plan(Mode::Auto, None, none, Facts { render_node: true, appimage: true, ..Facts::default() });
        assert_eq!(vars(&p), ["WEBKIT_DMABUF_RENDERER_FORCE_SHM"]);
        assert!(p.reason.contains("AppImage"));
        // Full means full, AppImage or not.
        let p = plan(Mode::Full, None, none, Facts { render_node: true, appimage: true, ..Facts::default() });
        assert!(vars(&p).is_empty());
    }

    #[test]
    fn wayland_goes_through_xwayland() {
        let p = plan(Mode::Auto, None, none, Facts { wayland: true, render_node: true, ..Facts::default() });
        assert_eq!(p.set, [("GDK_BACKEND", "x11"), ("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1")]);
    }

    /// A font file's start: a table directory with `tables`, then their data.
    fn font(tables: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut head: Vec<u8> = vec![0, 1, 0, 0];
        head.extend((tables.len() as u16).to_be_bytes());
        head.extend([0u8; 6]);
        let mut data: Vec<u8> = Vec::new();
        let start = 12 + 16 * tables.len();
        for (tag, body) in tables {
            head.extend(tag.iter());
            head.extend([0u8; 4]);
            head.extend(((start + data.len()) as u32).to_be_bytes());
            head.extend((body.len() as u32).to_be_bytes());
            data.extend(body.iter());
        }
        head.extend(data);
        head
    }

    #[test]
    fn colrv1_fonts_are_told_from_bitmap_emoji() {
        // Fedora's Noto-COLRv1.ttf: COLR version 1.
        assert_eq!(colr_version(&font(&[(b"CPAL", &[0, 0]), (b"COLR", &[0, 1, 0, 0])])), Some(1));
        // Ubuntu's NotoColorEmoji.ttf: bitmaps (CBDT), no COLR at all.
        assert_eq!(colr_version(&font(&[(b"CBDT", &[0, 3]), (b"CBLC", &[0, 3])])), None);
        // Twemoji's COLRv0 is drawn fine.
        assert_eq!(colr_version(&font(&[(b"COLR", &[0, 0])])), Some(0));
        // Truncated or not a font: nothing, and no panic.
        assert_eq!(colr_version(b"\0\x01\0\0\0\x09"), None);
        assert_eq!(colr_version(b""), None);
        // A collection: its first font starts at 16, so that font's offsets move by 16.
        let shifted = {
            let mut f = font(&[(b"COLR", &[0, 1])]);
            let off = u32::from_be_bytes([f[20], f[21], f[22], f[23]]) + 16;
            f[20..24].copy_from_slice(&off.to_be_bytes());
            let mut t = b"ttcf\0\x01\0\0\0\0\0\x01\0\0\0\x10".to_vec();
            t.extend(f);
            t
        };
        assert_eq!(colr_version(&shifted), Some(1));
        assert!(may_be_colour_emoji("Noto-COLRv1.ttf") && may_be_colour_emoji("NotoColorEmoji.ttf"));
        // The table far into the file, as in the real Noto-COLRv1.ttf (3.8 MB in):
        // the start of the file does not hold the version, the file does (KB-0116).
        let mut far = font(&[(b"CPAL", &[0, 0])]);
        let start = far.len();
        far[12..16].copy_from_slice(b"COLR");
        far[20..24].copy_from_slice(&((start + 40_000) as u32).to_be_bytes());
        far.resize(start + 40_000, 0);
        far.extend([0, 1, 0, 0]);
        assert_eq!(colr_version(&far[..16 * 1024]), None);
        let path = std::env::temp_dir().join(format!("kryoto-colr-{}.ttf", std::process::id()));
        std::fs::write(&path, &far).unwrap();
        assert_eq!(colr_version_of_file(&path), Some(1));
        let _ = std::fs::remove_file(&path);
        assert!(!may_be_colour_emoji("DejaVuSans.ttf") && !may_be_colour_emoji("emoji.txt"));
    }

    #[test]
    fn the_font_file_keeps_the_system_config_and_rejects_only_colrv1() {
        let c = font_config("/etc/fonts/fonts.conf", Some("/tmp/.mount_x/usr/share/kryoto/fonts"), &["/usr/share/fonts/a&b/Noto-COLRv1.ttf".into()]);
        assert!(c.contains("<include ignore_missing=\"yes\">/etc/fonts/fonts.conf</include>"));
        assert!(c.contains("<dir>/tmp/.mount_x/usr/share/kryoto/fonts</dir>"));
        assert!(c.contains("<glob>/usr/share/fonts/a&amp;b/Noto-COLRv1.ttf</glob>"));
    }

    #[test]
    fn nouveau_is_not_nvidia() {
        assert!(modules_have_nvidia("nvidia_drm 1 0 - Live\nnvidia 5 2 nvidia_drm, Live\n"));
        assert!(!modules_have_nvidia("nouveau 2 0 - Live\nnvidia_uvm 1 0 - Live\n"));
    }
}
