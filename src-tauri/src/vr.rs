//! VR: which runtimes this PC has, and getting one ready before a VR game.
//!
//! Steam's behaviour, matched: a game Steam marks VR (kryo.to's `vr` flag,
//! from Steam's "VR Supported"/"VR Only" categories) or a launch entry of
//! Steam's `vr` kind starts with SteamVR already running, so the headset is
//! awake when the game looks for it. Nothing here touches a flat game.
//!
//! Windows (first): SteamVR from every Steam library (`libraryfolders.vdf`),
//! the system's OpenXR runtime (`HKLM\SOFTWARE\Khronos\OpenXR\1`), Meta's
//! runtime (Oculus / Meta Quest Link), and OpenVR's own record of where
//! SteamVR is (`openvrpaths.vrpath`). Linux is experimental: OpenXR only,
//! the runtime manifests Monado and WiVRn install, handed to the game as
//! `XR_RUNTIME_JSON`; SteamVR under Proton is not promised.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Runtime};

/// kryo.to's flag, null-safe: anything but the two values is a flat game.
pub fn kind_of(v: &serde_json::Value) -> Option<String> {
    v.as_str().filter(|s| matches!(*s, "vr_supported" | "vr_only")).map(String::from)
}

/// One way a headset can be driven.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VrRuntime {
    /// Stable id for Settings: `steamvr`, `meta`, `system`, `monado`, `wivrn`, ...
    pub id: String,
    pub name: String,
    /// The OpenXR manifest a game is pointed at with `XR_RUNTIME_JSON`.
    pub manifest: Option<String>,
    /// Starts it (SteamVR's `vrstartup.exe`).
    pub starter: Option<String>,
    /// The system's current OpenXR runtime.
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VrStatus {
    pub runtimes: Vec<VrRuntime>,
    pub steamvr_installed: bool,
    pub steamvr_running: bool,
    /// Linux: OpenXR only, experimental.
    pub experimental: bool,
}

/// What Settings > VR shows.
#[tauri::command(async)]
pub fn vr_status() -> VrStatus {
    let runtimes = detect();
    VrStatus {
        steamvr_installed: runtimes.iter().any(|r| r.id == "steamvr" && r.starter.is_some()),
        steamvr_running: steamvr_running(),
        experimental: !cfg!(windows),
        runtimes,
    }
}

/// Every runtime found, SteamVR first, each once.
pub fn detect() -> Vec<VrRuntime> {
    let mut out: Vec<VrRuntime> = Vec::new();
    let active = active_manifest();
    for root in steam_roots() {
        for lib in library_folders(&root) {
            let dir = lib.join("steamapps").join("common").join("SteamVR");
            if let Some(r) = steamvr_at(&dir, active.as_deref()) {
                push(&mut out, r);
            }
        }
    }
    for dir in openvr_runtime_paths() {
        if let Some(r) = steamvr_at(&dir, active.as_deref()) {
            push(&mut out, r);
        }
    }
    #[cfg(windows)]
    {
        // The Meta Quest Link app: "Meta Horizon" today, "Oculus" before.
        for base in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"].iter().filter_map(|v| std::env::var(v).ok()) {
            for app in ["Meta Horizon", "Oculus"] {
                let json = PathBuf::from(&base).join(app).join(r"Support\oculus-runtime\oculus_openxr_64.json");
                if json.is_file() {
                    push(&mut out, runtime("meta", "Meta Quest Link", Some(&json), None, active.as_deref()));
                }
            }
        }
    }
    #[cfg(not(windows))]
    for (id, name, file) in [
        ("monado", "Monado", "openxr_monado.json"),
        ("wivrn", "WiVRn", "openxr_wivrn.json"),
    ] {
        for dir in ["/usr/share/openxr/1", "/usr/local/share/openxr/1", "/etc/xdg/openxr/1"] {
            let json = Path::new(dir).join(file);
            if json.is_file() {
                push(&mut out, runtime(id, name, Some(&json), None, active.as_deref()));
            }
        }
    }
    // The system's own choice, when it is none of the above.
    if let Some(json) = active.as_deref().filter(|a| !out.iter().any(|r| r.manifest.as_deref().is_some_and(|m| same(m, a)))) {
        push(&mut out, runtime("system", &format!("System OpenXR runtime ({})", file_stem(json)), Some(Path::new(json)), None, Some(json)));
    }
    out
}

fn push(out: &mut Vec<VrRuntime>, r: VrRuntime) {
    if !out.iter().any(|o| o.id == r.id) {
        out.push(r);
    }
}

fn runtime(id: &str, name: &str, manifest: Option<&Path>, starter: Option<&Path>, active: Option<&str>) -> VrRuntime {
    // Steam's registry path is written with forward slashes; shown as Windows writes paths.
    let tidy = |p: &Path| if cfg!(windows) { p.to_string_lossy().replace('/', "\\") } else { p.to_string_lossy().into_owned() };
    let manifest = manifest.map(tidy);
    let starter = starter.map(tidy);
    VrRuntime {
        id: id.into(),
        name: name.into(),
        active: manifest.as_deref().zip(active).is_some_and(|(m, a)| same(m, a)),
        manifest,
        starter,
    }
}

fn steamvr_at(dir: &Path, active: Option<&str>) -> Option<VrRuntime> {
    #[cfg(windows)]
    let (starter, manifest) = (dir.join(r"bin\win64\vrstartup.exe"), dir.join("steamxr_win64.json"));
    #[cfg(not(windows))]
    let (starter, manifest) = (dir.join("bin/vrstartup.sh"), dir.join("steamxr_linux64.json"));
    if !starter.is_file() && !manifest.is_file() {
        return None;
    }
    Some(runtime(
        "steamvr",
        "SteamVR",
        manifest.is_file().then_some(manifest.as_path()),
        (cfg!(windows) && starter.is_file()).then_some(starter.as_path()),
        active,
    ))
}

fn file_stem(p: &str) -> String {
    Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.into())
}

fn same(a: &str, b: &str) -> bool {
    let n = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
    n(a) == n(b)
}

/// The system's OpenXR runtime manifest, as OpenXR's loader finds it.
fn active_manifest() -> Option<String> {
    if let Some(v) = std::env::var_os("XR_RUNTIME_JSON").filter(|v| !v.is_empty()) {
        return Some(v.to_string_lossy().into_owned());
    }
    #[cfg(windows)]
    {
        reg_string(windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE, r"SOFTWARE\Khronos\OpenXR\1", "ActiveRuntime")
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".config")));
        let candidates = config.into_iter().map(|c| c.join("openxr/1/active_runtime.json")).chain([PathBuf::from("/etc/xdg/openxr/1/active_runtime.json")]);
        for c in candidates {
            if c.exists() {
                let real = std::fs::canonicalize(&c).unwrap_or(c);
                return Some(real.to_string_lossy().into_owned());
            }
        }
        None
    }
}

/// Where Steam keeps itself: the registry on Windows, the usual folders on Linux.
pub(crate) fn steam_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut roots: Vec<PathBuf> = reg_string(windows_sys::Win32::System::Registry::HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath").map(PathBuf::from).into_iter().collect();
        if let Ok(p) = std::env::var("ProgramFiles(x86)") {
            roots.push(PathBuf::from(p).join("Steam"));
        }
        roots.retain(|r| r.is_dir());
        roots.dedup_by(|a, b| same(&a.to_string_lossy(), &b.to_string_lossy()));
        roots
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        crate::compat::steam_roots(&home).into_iter().filter(|r| r.is_dir()).collect()
    }
}

/// Every library a Steam install knows: itself, and each `"path"` in
/// `steamapps/libraryfolders.vdf`.
fn library_folders(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    if let Ok(text) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
        out.extend(vdf_paths(&text).into_iter().map(PathBuf::from));
    }
    out
}

/// The `"path"` values out of a `libraryfolders.vdf`, unescaped.
pub fn vdf_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.split('"').filter(|p| !p.trim().is_empty());
        if parts.next().map(str::trim) == Some("path") {
            if let Some(v) = parts.next() {
                out.push(v.replace("\\\\", "\\"));
            }
        }
    }
    out
}

/// SteamVR's folders as OpenVR records them (`openvrpaths.vrpath`).
fn openvr_runtime_paths() -> Vec<PathBuf> {
    #[cfg(windows)]
    let file = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(r"openvr\openvrpaths.vrpath"));
    #[cfg(not(windows))]
    let file = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/openvr/openvrpaths.vrpath"));
    let Some(text) = file.and_then(|f| std::fs::read_to_string(f).ok()) else { return Vec::new() };
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    json["runtime"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).map(PathBuf::from).collect()
}

#[cfg(windows)]
fn reg_string(root: windows_sys::Win32::System::Registry::HKEY, key: &str, value: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (key, value) = (wide(key), wide(value));
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: NUL-terminated key and value names, a live buffer and its byte length.
    let status = unsafe { RegGetValueW(root, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
    if status != 0 {
        return None;
    }
    let chars = (len as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..chars])).filter(|s| !s.is_empty())
}

/// SteamVR's server or monitor is running.
pub fn steamvr_running() -> bool {
    let sys = sysinfo::System::new_with_specifics(sysinfo::RefreshKind::nothing().with_processes(sysinfo::ProcessRefreshKind::nothing()));
    sys.processes().values().any(|p| {
        let name = p.name().to_string_lossy().to_ascii_lowercase();
        name.starts_with("vrserver") || name.starts_with("vrmonitor")
    })
}

/// Whether this start is a VR one: the entry is Steam's `vr` kind (or says
/// VR in its name), or the game only plays in VR and a plain Play was
/// pressed. A flat entry of a VR-supported game is not.
pub fn wants_vr(game_vr: Option<&str>, entry: Option<&crate::launch::LaunchEntry>, asked: bool) -> bool {
    if asked {
        return true;
    }
    match entry {
        Some(e) => e.kind.eq_ignore_ascii_case("vr") || e.description.split_whitespace().any(|w| w.eq_ignore_ascii_case("vr")),
        None => game_vr == Some("vr_only"),
    }
}

/// Get the headset's runtime ready, and say which one the game should use.
/// Returns the environment to add (`XR_RUNTIME_JSON` for a runtime picked in
/// Settings) and a note for the game's log. Never fails the launch: a game
/// that finds no headset says so itself.
pub fn prepare<R: Runtime>(_app: &AppHandle<R>, settings: &crate::settings::Settings) -> (Vec<(String, String)>, String) {
    let runtimes = detect();
    let picked = runtimes.iter().find(|r| !settings.vr_runtime.is_empty() && r.id == settings.vr_runtime);
    let mut env = Vec::new();
    let mut notes = Vec::new();
    if let Some(json) = picked.and_then(|r| r.manifest.clone()) {
        notes.push(format!("VR: {} (XR_RUNTIME_JSON={json})", picked.map(|r| r.name.as_str()).unwrap_or("?")));
        env.push(("XR_RUNTIME_JSON".to_string(), json));
    }
    // SteamVR first, unless another runtime was picked on purpose.
    let steamvr = runtimes.iter().find(|r| r.id == "steamvr" && r.starter.is_some());
    let want_steamvr = settings.vr_start_steamvr && picked.is_none_or(|p| p.id == "steamvr");
    if let (true, Some(s)) = (want_steamvr, steamvr) {
        if steamvr_running() {
            notes.push("VR: SteamVR already running".into());
        } else if let Some(starter) = &s.starter {
            match std::process::Command::new(starter).current_dir(Path::new(starter).parent().unwrap_or(Path::new("."))).spawn() {
                Ok(_) => {
                    let started = Instant::now();
                    while started.elapsed() < Duration::from_secs(25) && !steamvr_running() {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    notes.push(if steamvr_running() {
                        format!("VR: started SteamVR in {} s", started.elapsed().as_secs())
                    } else {
                        "VR: SteamVR did not come up in 25 s; starting the game anyway".into()
                    });
                }
                Err(e) => notes.push(format!("VR: could not start SteamVR: {e}")),
            }
        }
    } else if steamvr.is_none() && picked.is_none() {
        notes.push("VR: no SteamVR on this PC; the game uses the system's OpenXR runtime".into());
    }
    (env, notes.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_steam_library() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
        assert_eq!(vdf_paths(vdf), vec![r"C:\Program Files (x86)\Steam".to_string(), r"D:\SteamLibrary".to_string()]);
    }

    /// What this machine has: `cargo test --lib vr_detect_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn vr_detect_live() {
        let status = vr_status();
        println!("{}", serde_json::to_string_pretty(&status).unwrap());
        assert_eq!(status.experimental, !cfg!(windows));
    }

    #[test]
    fn the_vr_flag_is_null_safe() {
        assert_eq!(kind_of(&serde_json::json!("vr_only")).as_deref(), Some("vr_only"));
        assert_eq!(kind_of(&serde_json::json!("vr_supported")).as_deref(), Some("vr_supported"));
        assert_eq!(kind_of(&serde_json::json!(null)), None);
        assert_eq!(kind_of(&serde_json::json!("yes")), None);
        assert_eq!(kind_of(&serde_json::json!(true)), None);
    }

    #[test]
    fn only_vr_starts_prepare_a_headset() {
        use crate::launch::LaunchEntry;
        let vr = LaunchEntry { kind: "vr".into(), ..Default::default() };
        let flat = LaunchEntry { description: "Desktop Mode".into(), ..Default::default() };
        let named = LaunchEntry { description: "Launch in VR".into(), ..Default::default() };
        assert!(wants_vr(None, Some(&vr), false));
        assert!(wants_vr(None, Some(&named), false));
        assert!(!wants_vr(Some("vr_supported"), Some(&flat), false));
        assert!(!wants_vr(Some("vr_supported"), None, false), "a VR-supported game's plain Play is flat");
        assert!(wants_vr(Some("vr_only"), None, false));
        assert!(wants_vr(Some("vr_supported"), None, true), "Play in VR");
        assert!(!wants_vr(None, None, false));
    }
}
