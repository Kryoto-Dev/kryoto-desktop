//! On Linux, the Wine and Proton builds already on this computer, so a game
//! can be played without anyone typing a path. On macOS, Windows games can use
//! the official Sikarugir Wine engine directly, or an existing Sikarugir
//! wrapper: a Wineskin-style app bundle whose launcher owns the Wine engine,
//! renderers and prefix.
//!
//! Looked for where Steam, Proton-GE and umu put them, newest Proton first:
//! Proton runs these games best, umu-run is Proton outside Steam, and plain
//! Wine is the last resort. Windows has none of this and gets an empty list.
//!
//! Like Steam, the client can also get them itself: `compat_install` puts the
//! newest Proton-GE and umu-launcher in the app's own `compat` folder, so a
//! Linux player never has to install Wine or Proton by hand. Those come first,
//! and a Proton then runs through umu, inside the Steam Linux Runtime. macOS
//! does not use Proton. Kryoto can download the latest Sikarugir Wine engine
//! into its managed compatibility folder and run its `bin/wine` directly;
//! existing wrappers in the standard Sikarugir folder remain discoverable too.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

const SIKARUGIR_ENGINE_LIST: &str = "https://sikarugir-app.github.io/Engines/EngineList.txt";
const SIKARUGIR_ENGINE_REPO: &str = "Sikarugir-App/Engines";
const SIKARUGIR_TEMPLATE_REPO: &str = "Sikarugir-App/Template";

/// Renderer ids understood by the official Sikarugir Template. `wined3d` is
/// the engine's built-in renderer and therefore has no renderer directory.
/// Keep this list stable: values are persisted in a game's library entry.
pub const SIKARUGIR_RENDERERS: &[(&str, &str)] = &[
    ("d3dmetal", "D3DMetal"),
    ("dxvk", "DXVK"),
    ("cnc_ddraw", "CNC-DDRAW"),
    ("d9vk", "D9VK"),
    ("dxmt", "DXMT"),
    ("wined3d", "WineD3D"),
];

// Several Play presses and Settings can need the same missing runtime.
// Serialize writes so they cannot unpack over one another's download.
static INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompatTool {
    /// What to call it: the folder name for a Proton, else the program.
    pub name: String,
    pub path: String,
    /// `proton`, `umu`, `wine` or `sikarugir`; managed macOS Sikarugir Wine
    /// engines use the `wine` kind and wrappers use `sikarugir`.
    pub kind: String,
    /// Put there by Kryoto Desktop, so Settings > Compatibility can remove it.
    #[serde(default)]
    pub managed: bool,
}

/// Where the client keeps the Proton and umu it downloaded.
pub fn managed_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("compat"))
}

#[cfg(unix)]
pub fn on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let appdir = std::env::var_os("APPDIR").map(PathBuf::from);
    std::env::split_paths(&path).filter(|d| !appdir.as_ref().is_some_and(|root| d.starts_with(root))).map(|d| d.join(program)).find(|p| p.is_file())
}

#[cfg(not(unix))]
pub fn on_path(_program: &str) -> Option<PathBuf> {
    None
}

/// umu-run: the client's own, else one on PATH. Linux only; macOS uses the
/// engine and prefix selected by a Sikarugir wrapper.
pub fn umu_run(managed: Option<&Path>) -> Option<PathBuf> {
    if cfg!(any(windows, target_os = "macos")) {
        return None;
    }
    managed
        .map(|m| m.join("umu").join("umu-run"))
        .filter(|p| p.is_file())
        .or_else(|| on_path("umu-run"))
}

/// Sikarugir wrappers in the wild use both names: recent templates call the
/// executable `Sikarugir`, while the Wineskin-compatible template calls it
/// `launcher` (the latter is also the name used in Sikarugir's CLI examples).
const SIKARUGIR_LAUNCHERS: &[&str] = &["Sikarugir", "launcher"];

/// Whether `app` is a Sikarugir/Wineskin wrapper bundle. The launcher is the
/// contract we can check without opening or modifying the bundle; a prefix is
/// allowed to be absent because Sikarugir creates it on first use.
pub fn is_sikarugir_wrapper(app: &Path) -> bool {
    app.is_dir()
        && app.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"))
        && launcher_for_app(app).is_some()
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn launcher_for_app(app: &Path) -> Option<PathBuf> {
    let macos = app.join("Contents/MacOS");
    SIKARUGIR_LAUNCHERS
        .iter()
        .map(|name| macos.join(name))
        .find(|path| executable_file(path))
}

/// The `bin/wine` executable inside a managed Sikarugir engine bundle.
/// Engines are archives containing `wswine.bundle`; they are not wrapper apps.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn is_sikarugir_engine(tool: &Path) -> bool {
    let Some(bin) = tool.parent() else { return false };
    let Some(bundle) = bin.parent() else { return false };
    let Some(engine) = bundle.parent() else { return false };
    tool.file_name().is_some_and(|name| name == "wine")
        && bin.file_name().is_some_and(|name| name == "bin")
        && bundle.file_name().is_some_and(|name| name == "wswine.bundle")
        && engine.parent().and_then(Path::file_name).is_some_and(|name| name == "sikarugir")
        && executable_file(tool)
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn sikarugir_engines_in(managed: &Path) -> Vec<CompatTool> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(managed.join("sikarugir")) else { return out };
    for entry in read.flatten() {
        let root = entry.path();
        if root.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.')) {
            continue;
        }
        let wine = root.join("wswine.bundle/bin/wine");
        if !is_sikarugir_engine(&wine) {
            continue;
        }
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Sikarugir Wine".into());
        out.push(CompatTool {
            name,
            path: wine.to_string_lossy().into_owned(),
            kind: "wine".into(),
            managed: true,
        });
    }
    out.sort_by_key(|tool| std::cmp::Reverse(natural_key(&tool.name)));
    out
}

#[cfg(target_os = "macos")]
fn frameworks_in_app(app: &Path) -> Option<PathBuf> {
    let frameworks = app.join("Contents/Frameworks");
    frameworks.join("libinotify.0.dylib").is_file().then_some(frameworks)
}

#[cfg(target_os = "macos")]
fn runtime_app_for_engine(tool: &Path) -> Option<PathBuf> {
    let engine = tool.parent()?.parent()?.parent()?;
    let runtime = engine.join("runtime");
    let entries = std::fs::read_dir(runtime).ok()?;
    entries
        .flatten()
        .map(|entry| entry.path())
        .find(|app| frameworks_in_app(app).is_some())
}

#[cfg(target_os = "macos")]
fn standard_sikarugir_frameworks() -> Vec<PathBuf> {
    let mut apps = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        apps.push(home.join("Applications/Sikarugir"));
    }
    apps.push(PathBuf::from("/Applications/Sikarugir"));
    if let Some(custom) = std::env::var_os("KRYOTO_SIKARUGIR_DIR").map(PathBuf::from) {
        apps.push(custom);
    }

    let mut out = Vec::new();
    for app_or_dir in apps {
        if let Some(frameworks) = frameworks_in_app(&app_or_dir) {
            out.push(frameworks);
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&app_or_dir) else { continue };
        for entry in entries.flatten() {
            if let Some(frameworks) = frameworks_in_app(&entry.path()) {
                if !out.iter().any(|path| path == &frameworks) {
                    out.push(frameworks);
                }
            }
        }
    }
    out
}

/// Native libraries required by a direct Sikarugir Wine engine. The Engines
/// archive intentionally omits these; official Sikarugir templates provide
/// them under `Contents/Frameworks`.
#[cfg(target_os = "macos")]
pub fn sikarugir_engine_frameworks(tool: &Path) -> Option<PathBuf> {
    if let Some(app) = runtime_app_for_engine(tool) {
        return frameworks_in_app(&app);
    }
    standard_sikarugir_frameworks().into_iter().next()
}

#[cfg(not(target_os = "macos"))]
pub fn sikarugir_engine_frameworks(_tool: &Path) -> Option<PathBuf> {
    None
}

/// Environment needed when the selected backend is the raw Sikarugir Wine
/// engine rather than a wrapper launcher. Wrapper apps establish this
/// themselves before starting Wine.
#[cfg(target_os = "macos")]
pub fn sikarugir_engine_environment(tool: &Path, renderer: Option<&str>) -> Result<Vec<(String, String)>, String> {
    if !is_sikarugir_engine(tool) {
        return Ok(Vec::new());
    }
    let bundle = tool
        .parent()
        .and_then(Path::parent)
        .ok_or("The Sikarugir Wine engine bundle is incomplete.")?;
    // WINEDLLPATH is a list of architecture-specific directories, not the
    // parent `lib/wine` directory. The official launcher exports these three
    // directories in this order; using the parent directories lets Wine find
    // enough files to print its version, but leaves wineboot unable to finish
    // prefix initialization on macOS.
    let windows64 = bundle.join("lib/wine/x86_64-windows");
    let windows32 = bundle.join("lib/wine/i386-windows");
    let unix = bundle.join("lib/wine/x86_64-unix");
    let lib = bundle.join("lib/wine");
    let frameworks = sikarugir_engine_frameworks(tool).ok_or(
        "Sikarugir Wine's native runtime libraries are missing. Get Sikarugir Wine again in Settings > Compatibility.",
    )?;

    let selected = renderer.filter(|value| !value.trim().is_empty() && *value != "auto").unwrap_or("wined3d");
    let renderer_folder = SIKARUGIR_RENDERERS
        .iter()
        .find(|(id, _)| *id == selected)
        .map(|(id, _)| *id)
        .ok_or_else(|| format!("Unknown Sikarugir renderer {selected:?}."))?;
    let contents = frameworks.parent().ok_or("Sikarugir's native runtime is incomplete.")?;
    let runtime_app = contents.parent().ok_or("Sikarugir's native runtime is incomplete.")?;
    let renderer_root = runtime_app.join("Contents/SharedSupport/renderer");

    // The launcher uses the fallback loader path rather than
    // DYLD_LIBRARY_PATH. The latter changes how Wine's child wineserver
    // resolves @rpath/libinotify.0.dylib when an x86_64 engine is spawned by
    // the arm64 desktop app, and leaves a fresh prefix half-initialized.
    // D3DMetal also loads a dylib from its renderer-specific `external`
    // directory.
    let mut dyld_fallback = vec![lib.clone(), lib.join("lib64"), frameworks.clone()];
    if renderer_folder != "wined3d" {
        let folder = renderer_root.join(renderer_folder);
        if !folder.is_dir() {
            return Err(format!("Sikarugir's {selected} renderer is missing. Get Sikarugir Wine again in Settings > Compatibility."));
        }
        if renderer_folder == "d3dmetal" {
            dyld_fallback.push(folder.join("external"));
        }
    }

    // Sikarugir's launcher also keeps the native runtime on the fallback
    // search path and marks the engine as its Wine build. These are important
    // for a fresh prefix: without them `wine --version` works, but the child
    // wineboot process exits before copying the built-in Windows files.
    let gstreamer = frameworks.join("GStreamer.framework/Libraries");
    if gstreamer.is_dir() {
        dyld_fallback.push(gstreamer);
    }
    dyld_fallback.extend([PathBuf::from("/opt/wine/lib"), PathBuf::from("/usr/lib"), PathBuf::from("/usr/libexec"), PathBuf::from("/usr/lib/system")]);
    let join_paths = |paths: &[PathBuf]| paths.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>().join(":");
    let mut env = vec![
        ("DYLD_FALLBACK_LIBRARY_PATH".into(), join_paths(&dyld_fallback)),
        ("WINESERVER".into(), bundle.join("bin/wineserver").to_string_lossy().into_owned()),
        ("WINELOADER".into(), unix.join("wine").to_string_lossy().into_owned()),
        ("WINEDLLPATH".into(), format!("{}:{}:{}", windows64.display(), windows32.display(), unix.display())),
        ("WINEDATADIR".into(), bundle.join("share/wine").to_string_lossy().into_owned()),
        ("WINEDLLDIR0".into(), windows64.to_string_lossy().into_owned()),
        ("WINEDLLDIR1".into(), windows32.to_string_lossy().into_owned()),
        ("WINEDLLDIR2".into(), unix.to_string_lossy().into_owned()),
        ("WINEBOOT_HIDE_DIALOG".into(), "1".into()),
        ("WINEMSYNC".into(), "1".into()),
        ("SikarugirAppWine11".into(), "1".into()),
        ("CX_ROOT".into(), bundle.to_string_lossy().into_owned()),
    ];

    if renderer_folder != "wined3d" {
        let renderer_wine = renderer_root.join(renderer_folder).join("wine");
        if !renderer_wine.is_dir() {
            return Err(format!("Sikarugir's {selected} renderer has no Wine files. Get Sikarugir Wine again in Settings > Compatibility."));
        }
        // These names are the same switches the official launcher uses. The
        // generic prepend makes the direct engine work with older Templates;
        // the renderer-specific variable is consumed by newer launchers and
        // is harmless to Wine builds that do not know it.
        env.push(("WINEDLLPATH_PREPEND".into(), renderer_wine.to_string_lossy().into_owned()));
        let variable = match renderer_folder {
            "d3dmetal" => "WINEDLLPATH_D3DMETAL",
            "dxvk" => "WINEDLLPATH_DXVK",
            "cnc_ddraw" => "WINEDLLPATH_CNCD",
            "d9vk" => "WINEDLLPATH_D9VK",
            "dxmt" => "WINEDLLPATH_DXMT",
            _ => unreachable!(),
        };
        env.push((variable.into(), renderer_wine.to_string_lossy().into_owned()));
        if matches!(renderer_folder, "dxvk" | "d9vk") {
            let icd = runtime_app.join("Contents/Resources/vulkan/icd.d/MoltenVK_icd.json");
            if icd.is_file() {
                env.push(("VK_DRIVER_FILES".into(), icd.to_string_lossy().into_owned()));
            }
        }
    }
    Ok(env)
}

#[cfg(not(target_os = "macos"))]
pub fn sikarugir_engine_environment(_tool: &Path, _renderer: Option<&str>) -> Result<Vec<(String, String)>, String> {
    Ok(Vec::new())
}

/// The launcher inside a Sikarugir wrapper. The standard Sikarugir setup keeps
/// wrappers in `~/Applications/Sikarugir`, while users may also keep them in
/// `/Applications/Sikarugir` or pass a wrapper path of their own. Both the
/// current `Sikarugir` launcher name and the older `launcher` name work.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn sikarugir_in(dirs: &[PathBuf]) -> Vec<CompatTool> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(dir) else { continue };
        for entry in read.flatten() {
            let app = entry.path();
            if !is_sikarugir_wrapper(&app) {
                continue;
            }
            let Some(launcher) = launcher_for_app(&app) else { continue };
            if out.iter().any(|tool: &CompatTool| tool.path == launcher.to_string_lossy()) {
                continue;
            }
            let name = app
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "Sikarugir wrapper".into());
            out.push(CompatTool {
                name,
                path: launcher.to_string_lossy().into_owned(),
                kind: "sikarugir".into(),
                managed: false,
            });
        }
    }
    out.sort_by_key(|tool| tool.name.to_ascii_lowercase());
    out
}

#[cfg(target_os = "macos")]
fn sikarugir_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        out.push(home.join("Applications/Sikarugir"));
    }
    out.push(PathBuf::from("/Applications/Sikarugir"));
    if let Some(custom) = std::env::var_os("KRYOTO_SIKARUGIR_DIR").map(PathBuf::from) {
        if !out.iter().any(|dir| dir == &custom) {
            out.push(custom);
        }
    }
    out
}

/// The wrapper app containing a Sikarugir launcher, if the path is one.
pub fn sikarugir_wrapper(tool: &Path) -> Option<PathBuf> {
    let macos = tool.parent()?;
    if macos.file_name()? != "MacOS" || macos.parent()?.file_name()? != "Contents" {
        return None;
    }
    let app = macos.parent()?.parent()?;
    if !app.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) {
        return None;
    }
    (launcher_for_app(app).as_deref() == Some(tool)).then_some(app.to_path_buf())
}

/// The prefix owned by a Sikarugir wrapper. It is intentionally derived from
/// the wrapper instead of using Kryoto's Linux-style per-game prefix: the
/// Sikarugir launcher sets up this prefix and its renderer environment itself.
pub fn sikarugir_prefix(tool: &Path) -> Option<PathBuf> {
    sikarugir_wrapper(tool).map(|app| {
        let shared = app.join("Contents/SharedSupport/prefix");
        let resources = app.join("Contents/Resources");
        [shared.clone(), resources.join("wineprefix"), resources.join("prefix")]
            .into_iter()
            .find(|prefix| prefix.is_dir())
            .or_else(|| resources.join("drive_c").is_dir().then_some(resources))
            .unwrap_or(shared)
    })
}

/// Every `<dir>/*/proton` under the folders Steam and Proton-GE use.
#[cfg_attr(any(not(unix), target_os = "macos"), allow(dead_code))]
fn protons_in(dirs: &[PathBuf]) -> Vec<CompatTool> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(dir) else { continue };
        for entry in read.flatten() {
            let script = entry.path().join("proton");
            if script.is_file() && proton_matches_arch(&script, std::env::consts::ARCH) {
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = script.to_string_lossy().into_owned();
                if !out.iter().any(|t: &CompatTool| t.path == path) {
                    out.push(CompatTool { name, path, kind: "proton".into(), managed: false });
                }
            }
        }
    }
    // Newest first: "GE-Proton10-3" before "GE-Proton9-27", "Proton 9.0" before "Proton 8.0".
    out.sort_by_key(|t| std::cmp::Reverse(natural_key(&t.name)));
    out
}

/// A sort key that compares the numbers in a name as numbers.
#[cfg_attr(any(not(unix), target_os = "macos"), allow(dead_code))]
fn natural_key(s: &str) -> Vec<(u8, u64, String)> {
    let mut out = Vec::new();
    let mut num = String::new();
    let mut text = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !text.is_empty() {
                out.push((0, 0, std::mem::take(&mut text).to_lowercase()));
            }
            num.push(c);
        } else {
            if !num.is_empty() {
                out.push((1, num.parse().unwrap_or(0), String::new()));
                num.clear();
            }
            text.push(c);
        }
    }
    if !num.is_empty() {
        out.push((1, num.parse().unwrap_or(0), String::new()));
    }
    if !text.is_empty() {
        out.push((0, 0, text.to_lowercase()));
    }
    out
}

#[cfg_attr(any(not(unix), target_os = "macos"), allow(dead_code))]
fn proton_dirs(home: &Path) -> Vec<PathBuf> {
    let steams = steam_roots(home);
    let mut dirs: Vec<PathBuf> = steams.iter().flat_map(|s| [s.join("compatibilitytools.d"), s.join("steamapps/common")]).collect();
    dirs.push(PathBuf::from("/usr/share/steam/compatibilitytools.d"));
    dirs
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn steam_roots(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".steam/steam"),
        home.join(".steam/root"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
}

/// Old clients picked the first archive, which can be ARM on an Intel PC.
/// Check both the directory label and Wine's ELF header for custom installs.
pub fn proton_matches_arch(tool: &Path, arch: &str) -> bool {
    let Some(dir) = tool.parent() else {
        return false;
    };
    let name = dir.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    if (name.contains("aarch64") || name.contains("arm64")) && arch != "aarch64" {
        return false;
    }
    if (name.contains("x86_64") || name.contains("amd64")) && arch != "x86_64" {
        return false;
    }
    for rel in ["files/bin/wine", "files/bin/wine64", "dist/bin/wine", "dist/bin/wine64"] {
        use std::io::Read;
        let mut header = [0; 20];
        if std::fs::File::open(dir.join(rel)).and_then(|mut f| f.read_exact(&mut header)).is_ok() && &header[..4] == b"\x7fELF" {
            let machine = if header[5] == 2 { u16::from_be_bytes([header[18], header[19]]) } else { u16::from_le_bytes([header[18], header[19]]) };
            return matches!((arch, machine), ("x86_64", 62 | 3) | ("aarch64", 183));
        }
    }
    true
}

/// Recover a missing or wrong-architecture saved pick using a working tool.
pub fn resolve_tool(saved: Option<&str>, tools: &[CompatTool]) -> Result<Option<PathBuf>, String> {
    if let Some(saved) = saved {
        let path = PathBuf::from(saved);
        let resolved = if cfg!(target_os = "macos") {
            // A macOS selection must be an actual Sikarugir wrapper launcher
            // or an engine bundle, not a same-named executable copied from
            // somewhere else.
            (path.components().count() != 1 && (sikarugir_wrapper(&path).is_some() || is_sikarugir_engine(&path))).then_some(path.clone())
        } else if path.components().count() == 1 {
            on_path(saved)
        } else {
            path.is_file().then_some(path.clone())
        };
        if let Some(tool) = resolved {
            if kind_of(&tool) != "proton" || proton_matches_arch(&tool, std::env::consts::ARCH) {
                return Ok(Some(tool));
            }
        }
        // A valid replacement avoids leaving existing installations broken.
        if let Some(tool) = tools.iter().find(|tool| {
            !cfg!(target_os = "macos") || sikarugir_wrapper(Path::new(&tool.path)).is_some() || is_sikarugir_engine(Path::new(&tool.path))
        }) {
            return Ok(Some(PathBuf::from(&tool.path)));
        }
        if cfg!(target_os = "macos") {
            return Err(format!("The Sikarugir Wine engine or wrapper {saved} is missing or invalid. Get Sikarugir Wine in Settings > Compatibility, or pick a wrapper."));
        }
        return Err(format!("The compatibility tool {saved} is missing or does not match this computer ({}). Get Proton in Settings > Compatibility, or pick an installed Wine tool.", std::env::consts::ARCH));
    }
    Ok(tools
        .iter()
        .find(|tool| !cfg!(target_os = "macos") || sikarugir_wrapper(Path::new(&tool.path)).is_some() || is_sikarugir_engine(Path::new(&tool.path)))
        .map(|t| PathBuf::from(&t.path)))
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn detect_in(managed: Option<&Path>) -> Vec<CompatTool> {
    let mut out = Vec::new();
    if let Some(m) = managed {
        out.extend(protons_in(&[m.to_path_buf()]).into_iter().map(|t| CompatTool { managed: true, ..t }));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for t in protons_in(&proton_dirs(&home)) {
            if !out.iter().any(|o| o.path == t.path) {
                out.push(t);
            }
        }
    }
    if let Some(p) = umu_run(managed) {
        let own = managed.is_some_and(|m| p.starts_with(m));
        out.push(CompatTool { name: "umu-run".into(), path: p.to_string_lossy().into_owned(), kind: "umu".into(), managed: own });
    }
    for program in ["wine", "wine64"] {
        if let Some(p) = on_path(program) {
            out.push(CompatTool { name: program.into(), path: p.to_string_lossy().into_owned(), kind: "wine".into(), managed: false });
            break;
        }
    }
    out
}

#[cfg(target_os = "macos")]
pub fn detect_in(managed: Option<&Path>) -> Vec<CompatTool> {
    let mut out = managed.map(sikarugir_engines_in).unwrap_or_default();
    out.extend(sikarugir_in(&sikarugir_dirs()));
    out
}

#[cfg(not(unix))]
pub fn detect_in(_managed: Option<&Path>) -> Vec<CompatTool> {
    Vec::new()
}

/// Every tool, the client's own first.
pub fn detect<R: Runtime>(app: &AppHandle<R>) -> Vec<CompatTool> {
    detect_in(managed_dir(app).as_deref())
}

/// What a tool is, from its path: Proton's `proton` script, `umu-run`, or Wine.
pub fn kind_of(tool: &Path) -> &'static str {
    match tool.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()) {
        Some(n) if n == "proton" => "proton",
        Some(n) if n == "umu-run" => "umu",
        #[cfg(target_os = "macos")]
        Some(n) if matches!(n.as_str(), "sikarugir" | "launcher") => "sikarugir",
        _ => "wine",
    }
}

#[tauri::command(async)]
pub fn compat_tools(app: AppHandle) -> Vec<CompatTool> {
    detect(&app)
}

/// What Settings > Compatibility shows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatStatus {
    tools: Vec<CompatTool>,
    /// The native host family, so Settings can explain the right backend even
    /// when the browser user-agent is reduced or overridden.
    platform: &'static str,
    /// umu-run, which runs a Proton inside the Steam Linux Runtime.
    umu: Option<String>,
    mangohud: bool,
    gamemode: bool,
}

#[tauri::command(async)]
pub fn compat_status(app: AppHandle) -> CompatStatus {
    let managed = managed_dir(&app);
    CompatStatus {
        tools: detect_in(managed.as_deref()),
        platform: if cfg!(target_os = "macos") { "macos" } else if cfg!(windows) { "windows" } else { "linux" },
        umu: umu_run(managed.as_deref()).map(|p| p.to_string_lossy().into_owned()),
        mangohud: on_path("mangohud").is_some(),
        gamemode: on_path("gamemoderun").is_some(),
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

fn proton_asset<'a>(release: &'a Release, arch: &str) -> Result<&'a Asset, String> {
    let explicit = match arch {
        "x86_64" | "aarch64" => format!("{}-{arch}.tar.gz", release.tag_name),
        _ => return Err(format!("Proton downloads are not available for {arch}.")),
    };
    release
        .assets
        .iter()
        .find(|a| a.name == explicit)
        .or_else(|| (arch == "x86_64").then(|| release.assets.iter().find(|a| a.name == format!("{}.tar.gz", release.tag_name))).flatten())
        .ok_or_else(|| format!("That Proton-GE release has no download for {arch}."))
}

#[derive(Clone, Serialize)]
struct Progress {
    step: String,
    fraction: Option<f64>,
}

fn progress<R: Runtime>(app: &AppHandle<R>, step: &str, fraction: Option<f64>) {
    use tauri::Emitter;
    let _ = app.emit("compat-progress", Progress { step: step.into(), fraction });
}

async fn latest(client: &reqwest::Client, repo: &str) -> Result<Release, String> {
    client
        .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Could not reach GitHub: {e}"))?
        .json()
        .await
        .map_err(|e| format!("GitHub answered oddly: {e}"))
}

fn first_engine_name(catalog: &str) -> Result<String, String> {
    let name = catalog
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .find(|line| !line.is_empty())
        .ok_or("Sikarugir's engine catalog was empty.")?;
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err("Sikarugir's engine catalog contained an invalid engine name.".into());
    }
    Ok(name.to_string())
}

async fn latest_sikarugir_engine(client: &reqwest::Client) -> Result<(String, Asset), String> {
    let catalog = client
        .get(SIKARUGIR_ENGINE_LIST)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Could not read Sikarugir's engine catalog: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Sikarugir's engine catalog could not be read: {e}"))?;
    let name = first_engine_name(&catalog)?;
    let archive = format!("{name}.tar.xz");
    let release = latest(client, SIKARUGIR_ENGINE_REPO).await?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == archive)
        .cloned()
        .ok_or_else(|| format!("Sikarugir's latest catalog entry {name} has no matching release asset."))?;
    if asset.digest.as_deref().and_then(|digest| digest.strip_prefix("sha256:")).is_none() {
        return Err(format!("Sikarugir's {name} release has no SHA-256 digest."));
    }
    Ok((name, asset))
}

fn template_version(name: &str) -> Option<Vec<u64>> {
    let version = name.strip_prefix("Template-")?.strip_suffix(".tar.xz")?;
    let parts = version.split('.').map(str::parse::<u64>).collect::<Result<Vec<_>, _>>().ok()?;
    (!parts.is_empty()).then_some(parts)
}

async fn latest_sikarugir_template(client: &reqwest::Client) -> Result<(String, Asset), String> {
    let release = latest(client, SIKARUGIR_TEMPLATE_REPO).await?;
    let asset = release
        .assets
        .iter()
        .filter_map(|asset| template_version(&asset.name).map(|version| (version, asset)))
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, asset)| asset.clone())
        .ok_or("Sikarugir's Template release had no Template archive.")?;
    if asset.digest.as_deref().and_then(|digest| digest.strip_prefix("sha256:")).is_none() {
        return Err(format!("Sikarugir's {} release has no SHA-256 digest.", asset.name));
    }
    Ok((asset.name.clone(), asset))
}

/// Stream a file to `to`, saying how far along it is.
async fn fetch<R: Runtime>(app: &AppHandle<R>, client: &reqwest::Client, url: &str, to: &Path, step: &str) -> Result<(), String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let res = client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("{step} failed: {e}"))?;
    let total = res.content_length();
    let mut file = tokio::fs::File::create(to).await.map_err(|e| e.to_string())?;
    let mut got = 0u64;
    let mut last = std::time::Instant::now();
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("{step} failed: {e}"))?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        got += chunk.len() as u64;
        if last.elapsed().as_millis() > 200 {
            last = std::time::Instant::now();
            progress(app, step, total.map(|t| got as f64 / t.max(1) as f64));
        }
    }
    file.flush().await.map_err(|e| e.to_string())
}

/// Unpack with the system's tar, available on the supported desktop hosts.
async fn untar(archive: &Path, into: &Path) -> Result<(), String> {
    let (archive, into) = (archive.to_path_buf(), into.to_path_buf());
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("tar");
        crate::launch::clean_appimage_env(&mut cmd);
        let out = cmd.arg("-xf").arg(&archive).arg("-C").arg(&into).output().map_err(|e| format!("Could not run tar: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("Could not unpack {}: {}", archive.display(), String::from_utf8_lossy(&out.stderr).trim()))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

fn safe_tar_entry(entry: &str) -> bool {
    let entry = entry.trim_end_matches('/');
    !entry.is_empty()
        && !Path::new(entry).components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_)
            )
        })
}

fn validate_tar(archive: &Path) -> Result<(), String> {
    let listed = std::process::Command::new("tar")
        .arg("-tf")
        .arg(archive)
        .output()
        .map_err(|e| format!("Could not inspect {}: {e}", archive.display()))?;
    if !listed.status.success() {
        return Err(format!(
            "Could not inspect {}: {}",
            archive.display(),
            String::from_utf8_lossy(&listed.stderr).trim()
        ));
    }
    for entry in String::from_utf8_lossy(&listed.stdout).lines() {
        if !safe_tar_entry(entry) {
            return Err("Sikarugir's engine archive contained an unsafe path.".into());
        }
    }

    let verbose = std::process::Command::new("tar")
        .arg("-tvf")
        .arg(archive)
        .output()
        .map_err(|e| format!("Could not inspect {}: {e}", archive.display()))?;
    if !verbose.status.success() {
        return Err(format!(
            "Could not inspect {}: {}",
            archive.display(),
            String::from_utf8_lossy(&verbose.stderr).trim()
        ));
    }
    if String::from_utf8_lossy(&verbose.stdout).lines().any(|line| matches!(line.as_bytes().first(), Some(b'l' | b'h' | b'p' | b'c' | b'b' | b's'))) {
        return Err("Sikarugir's engine archive contained a link or special file.".into());
    }
    Ok(())
}

/// Validate an official Template archive before extracting it. Template
/// archives intentionally contain symlinks (framework Versions/Current and
/// renderer aliases), so unlike an engine archive they cannot use
/// [`validate_tar`]'s no-links rule. The archive is hash-checked before this
/// function is called; this check is for path traversal and malformed names.
fn validate_template_tar(archive: &Path) -> Result<(), String> {
    let listed = std::process::Command::new("tar")
        .arg("-tf")
        .arg(archive)
        .output()
        .map_err(|e| format!("Could not inspect {}: {e}", archive.display()))?;
    if !listed.status.success() {
        return Err(format!(
            "Could not inspect {}: {}",
            archive.display(),
            String::from_utf8_lossy(&listed.stderr).trim()
        ));
    }
    let mut has_frameworks = false;
    let mut has_renderer = false;
    for entry in String::from_utf8_lossy(&listed.stdout).lines() {
        if !safe_tar_entry(entry) {
            return Err("Sikarugir's Template archive contained an unsafe path.".into());
        }
        has_frameworks |= entry.ends_with("/Contents/Frameworks/libinotify.0.dylib");
        has_renderer |= entry.contains("/Contents/SharedSupport/renderer/");
    }
    if !has_frameworks || !has_renderer {
        return Err("Sikarugir's Template archive did not contain its native runtime and renderer files.".into());
    }
    Ok(())
}

fn sha512_of(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha512};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha512::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn sha256_of(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(target_os = "macos")]
fn install_rosetta() -> Result<(), String> {
    if std::env::consts::ARCH != "aarch64" {
        return Ok(());
    }
    let output = std::process::Command::new("/usr/sbin/softwareupdate")
        .args(["--install-rosetta", "--agree-to-license"])
        .output()
        .map_err(|e| format!("Rosetta 2 could not be started: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if detail.is_empty() {
            format!("Rosetta 2 installation failed with {}.", output.status)
        } else {
            format!("Rosetta 2 installation failed: {detail}")
        })
    }
}

#[cfg(not(target_os = "macos"))]
fn install_rosetta() -> Result<(), String> {
    Ok(())
}

fn template_app_ready(runtime: &Path, name: &str) -> bool {
    let app = runtime.join(format!("{}.app", name.trim_end_matches(".tar.xz")));
    app.join("Contents/Frameworks/libinotify.0.dylib").is_file()
        && app.join("Contents/SharedSupport/renderer").is_dir()
}

async fn install_sikarugir_template(app: &AppHandle, client: &reqwest::Client, root: &Path) -> Result<(), String> {
    let runtime = root.join("runtime");
    std::fs::create_dir_all(&runtime).map_err(|e| e.to_string())?;
    progress(app, "Looking for the newest Sikarugir Template", None);
    let (name, asset) = latest_sikarugir_template(client).await?;
    if template_app_ready(&runtime, &name) {
        return Ok(());
    }

    let archive = root.join(format!(".{name}.tar.xz"));
    fetch(app, client, &asset.browser_download_url, &archive, &format!("Downloading Sikarugir Template {name}"))
        .await?;
    progress(app, "Checking the Sikarugir Template download", None);
    let expected = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or("Sikarugir's Template release did not include a SHA-256 digest.")?
        .to_ascii_lowercase();
    let file = archive.clone();
    let actual = tauri::async_runtime::spawn_blocking(move || sha256_of(&file))
        .await
        .map_err(|e| e.to_string())??;
    if actual != expected {
        let _ = std::fs::remove_file(&archive);
        return Err("The Sikarugir Template download was damaged. Try again.".into());
    }

    let staging = runtime.join(format!(".{name}.partial"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let archive_for_validation = archive.clone();
    let validation = tauri::async_runtime::spawn_blocking(move || validate_template_tar(&archive_for_validation))
        .await
        .map_err(|e| format!("Template archive inspection stopped: {e}"))?;
    if let Err(error) = validation {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_file(&archive);
        return Err(error);
    }
    progress(app, &format!("Unpacking Sikarugir Template {name}"), None);
    let unpacked = untar(&archive, &staging).await;
    let _ = std::fs::remove_file(&archive);
    if let Err(error) = unpacked {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    let staged_app = staging.join(format!("{}.app", name.trim_end_matches(".tar.xz")));
    if !template_app_ready(&staging, &name) || !staged_app.is_dir() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("Sikarugir's Template archive unpacked without its native runtime and renderer files.".into());
    }
    let installed = runtime.join(format!("{}.app", name.trim_end_matches(".tar.xz")));
    let _ = std::fs::remove_dir_all(&installed);
    std::fs::rename(&staged_app, &installed).map_err(|e| format!("Could not install Sikarugir Template: {e}"))?;
    let _ = std::fs::remove_dir_all(&staging);
    if !template_app_ready(&runtime, &name) {
        return Err("Sikarugir's Template installed without its native runtime.".into());
    }
    crate::logging::info("compat", &format!("installed {name}"));
    Ok(())
}

async fn install_sikarugir_engine(app: &AppHandle, client: &reqwest::Client, dir: &Path) -> Result<CompatTool, String> {
    tauri::async_runtime::spawn_blocking(install_rosetta)
        .await
        .map_err(|e| format!("Rosetta 2 installation stopped: {e}"))??;
    progress(app, "Looking for the newest Sikarugir Wine engine", None);
    let (name, asset) = latest_sikarugir_engine(client).await?;
    let engines = dir.join("sikarugir");
    let root = engines.join(&name);
    let wine = root.join("wswine.bundle/bin/wine");
    if is_sikarugir_engine(&wine) {
        install_sikarugir_template(app, client, &root).await?;
        progress(app, "Sikarugir Wine is ready", Some(1.0));
        return Ok(CompatTool { name, path: wine.to_string_lossy().into_owned(), kind: "wine".into(), managed: true });
    }

    std::fs::create_dir_all(&engines).map_err(|e| e.to_string())?;
    let archive = engines.join(format!(".{}", asset.name));
    fetch(app, client, &asset.browser_download_url, &archive, &format!("Downloading Sikarugir Wine {name}")).await?;
    progress(app, "Checking the Sikarugir Wine download", None);
    let expected = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .ok_or("Sikarugir's engine release did not include a SHA-256 digest.")?
        .to_ascii_lowercase();
    let file = archive.clone();
    let actual = tauri::async_runtime::spawn_blocking(move || sha256_of(&file))
        .await
        .map_err(|e| e.to_string())??;
    if actual != expected {
        let _ = std::fs::remove_file(&archive);
        return Err("The Sikarugir Wine download was damaged. Try again.".into());
    }

    let staging = engines.join(format!(".{name}.partial"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    progress(app, &format!("Unpacking Sikarugir Wine {name}"), None);
    let archive_for_validation = archive.clone();
    let validation = tauri::async_runtime::spawn_blocking(move || validate_tar(&archive_for_validation))
        .await
        .map_err(|e| format!("Archive inspection stopped: {e}"))?;
    if let Err(error) = validation {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_file(&archive);
        return Err(error);
    }
    let unpacked = untar(&archive, &staging).await;
    let _ = std::fs::remove_file(&archive);
    if let Err(error) = unpacked {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    let staged_wine = staging.join("wswine.bundle/bin/wine");
    if !staged_wine.is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("Sikarugir's engine archive did not contain wswine.bundle/bin/wine.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&staged_wine).map_err(|e| e.to_string())?;
        if metadata.permissions().mode() & 0o111 == 0 {
            let _ = std::fs::remove_dir_all(&staging);
            return Err("Sikarugir's engine archive contained a non-executable wine engine.".into());
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    std::fs::rename(staging.join("wswine.bundle"), root.join("wswine.bundle")).map_err(|e| format!("Could not install Sikarugir Wine: {e}"))?;
    let _ = std::fs::remove_dir_all(&staging);
    if !is_sikarugir_engine(&wine) {
        return Err("Sikarugir Wine unpacked without an executable wine engine.".into());
    }
    // The engine archive intentionally omits native macOS libraries and the
    // renderer payload. Keep those official Template files beside the engine,
    // not in /opt/homebrew or another host-specific location.
    install_sikarugir_template(app, client, &root).await?;
    crate::logging::info("compat", &format!("installed Sikarugir Wine {name}"));
    progress(app, "Sikarugir Wine is ready", Some(1.0));
    Ok(CompatTool { name, path: wine.to_string_lossy().into_owned(), kind: "wine".into(), managed: true })
}

/// Get the newest managed compatibility backend into the app's `compat`
/// folder: Sikarugir Wine on macOS, or Proton-GE plus umu-launcher on Linux.
/// Returns the backend, ready to pick.
#[tauri::command]
pub async fn compat_install(app: AppHandle) -> Result<CompatTool, String> {
    if cfg!(windows) {
        return Err("Windows runs these games itself.".into());
    }
    if cfg!(target_os = "macos") {
        let _install = INSTALL.lock().await;
        let dir = managed_dir(&app).ok_or("No app data folder.")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(20))
            .timeout(std::time::Duration::from_secs(900))
            .build()
            .map_err(|e| e.to_string())?;
        return install_sikarugir_engine(&app, &client, &dir).await;
    }
    let _install = INSTALL.lock().await;
    let dir = managed_dir(&app).ok_or("No app data folder.")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(900))
        .build()
        .map_err(|e| e.to_string())?;

    progress(&app, "Looking for the newest Proton-GE", None);
    let rel = latest(&client, "GloriousEggroll/proton-ge-custom").await?;
    let tar = proton_asset(&rel, std::env::consts::ARCH)?;
    let folder = tar.name.trim_end_matches(".tar.gz");
    let mut proton = dir.join(folder).join("proton");
    // Some releases use a version-only folder inside an architecture-labelled archive.
    let legacy = dir.join(&rel.tag_name).join("proton");
    if !proton.is_file() && legacy.is_file() && proton_matches_arch(&legacy, std::env::consts::ARCH) {
        proton = legacy;
    }
    if !proton.is_file() || !proton_matches_arch(&proton, std::env::consts::ARCH) {
        let archive = dir.join(format!(".{}", tar.name));
        fetch(&app, &client, &tar.browser_download_url, &archive, &format!("Downloading {}", rel.tag_name)).await?;
        let checksum = format!("{folder}.sha512sum");
        if let Some(sum) = rel.assets.iter().find(|a| a.name == checksum) {
            progress(&app, "Checking the download", None);
            let want = client
                .get(&sum.browser_download_url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| e.to_string())?
                .text()
                .await
                .map_err(|e| e.to_string())?;
            let want = want.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
            let file = archive.clone();
            let got = tauri::async_runtime::spawn_blocking(move || sha512_of(&file))
                .await
                .map_err(|e| e.to_string())??;
            if want != got {
                let _ = std::fs::remove_file(&archive);
                return Err("The Proton download was damaged. Try again.".into());
            }
        }
        progress(&app, &format!("Unpacking {}", rel.tag_name), None);
        let unpacked = untar(&archive, &dir).await;
        let _ = std::fs::remove_file(&archive);
        unpacked?;
        if !proton.is_file() {
            proton = dir.join(&rel.tag_name).join("proton");
        }
        if !proton.is_file() {
            return Err(format!("{} unpacked without its proton script.", rel.tag_name));
        }
        if !proton_matches_arch(&proton, std::env::consts::ARCH) {
            return Err("The downloaded Proton does not match this computer's architecture.".into());
        }
        crate::logging::info("compat", &format!("installed {}", rel.tag_name));
    }

    install_umu(&app, &client, &dir).await?;

    progress(&app, "Ready", Some(1.0));
    Ok(CompatTool { name: folder.to_string(), path: proton.to_string_lossy().into_owned(), kind: "proton".into(), managed: true })
}

pub async fn ensure_runtime(app: &AppHandle) -> Result<PathBuf, String> {
    let _install = INSTALL.lock().await;
    let dir = managed_dir(app).ok_or("No app data folder.")?;
    if let Some(path) = umu_run(Some(&dir)) {
        return Ok(path);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    install_umu(app, &client, &dir).await?;
    umu_run(Some(&dir)).ok_or("The game runtime launcher could not be installed.".into())
}

async fn install_umu(app: &AppHandle, client: &reqwest::Client, dir: &Path) -> Result<(), String> {
    if !dir.join("umu").join("umu-run").is_file() {
        progress(app, "Looking for the Steam runtime launcher (umu)", None);
        let umu = latest(client, "Open-Wine-Components/umu-launcher").await?;
        let tar = umu.assets.iter().find(|a| a.name.ends_with("zipapp.tar")).ok_or("That umu release has no download.")?;
        let archive = dir.join(format!(".{}", tar.name));
        fetch(app, client, &tar.browser_download_url, &archive, &format!("Downloading umu {}", umu.tag_name)).await?;
        let unpacked = untar(&archive, dir).await;
        let _ = std::fs::remove_file(&archive);
        unpacked?;
        if !dir.join("umu").join("umu-run").is_file() {
            return Err("umu unpacked without umu-run.".into());
        }
        crate::logging::info("compat", &format!("installed umu {}", umu.tag_name));
    }

    Ok(())
}

/// Remove a Proton (or umu) the client downloaded. Nothing outside its own
/// `compat` folder can be removed this way.
#[tauri::command]
pub async fn compat_remove(app: AppHandle, path: String) -> Result<(), String> {
    let dir = managed_dir(&app).ok_or("No app data folder.")?;
    let root = std::fs::canonicalize(&dir).map_err(|_| "Nothing to remove.".to_string())?;
    let tool = std::fs::canonicalize(&path).map_err(|_| "That tool is already gone.".to_string())?;
    let folder = if is_sikarugir_engine(&tool) {
        let folder = tool
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or("Not one of Kryoto's tools.")?
            .to_path_buf();
        let valid_parent = folder.parent().and_then(Path::parent) == Some(root.as_path())
            && folder.parent().and_then(Path::file_name).is_some_and(|name| name == "sikarugir");
        if !valid_parent {
            return Err("Only tools Kryoto downloaded can be removed here.".into());
        }
        folder
    } else {
        let folder = tool.parent().ok_or("Not one of Kryoto's tools.")?.to_path_buf();
        if folder.parent() != Some(root.as_path()) {
            return Err("Only tools Kryoto downloaded can be removed here.".into());
        }
        folder
    };
    if !folder.starts_with(&root) {
        return Err("Only tools Kryoto downloaded can be removed here.".into());
    }
    crate::logging::info("compat", &format!("removing {}", folder.display()));
    tauri::async_runtime::spawn_blocking(move || std::fs::remove_dir_all(folder))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Could not remove it: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sikarugir_catalog_uses_the_first_nonempty_engine_name() {
        assert_eq!(first_engine_name("\n# available engines\n WS12WineSikarugir11.0_1 \nWS12WineSikarugir10.0_8\n").unwrap(), "WS12WineSikarugir11.0_1");
        assert!(first_engine_name("../not-an-engine").is_err());
    }

    #[test]
    fn template_versions_are_compared_numerically() {
        assert_eq!(template_version("Template-1.0.24.tar.xz"), Some(vec![1, 0, 24]));
        assert!(template_version("Template-nope.tar.xz").is_none());
    }

    #[test]
    fn engine_archives_cannot_escape_the_staging_folder() {
        assert!(safe_tar_entry("wswine.bundle/bin/wine"));
        assert!(safe_tar_entry("wswine.bundle/"));
        assert!(!safe_tar_entry("../outside"));
        assert!(!safe_tar_entry("/outside"));
    }

    fn release(names: &[&str]) -> Release {
        Release {
            tag_name: "GE-Proton11-7".into(),
            assets: names.iter().map(|name| Asset { name: (*name).into(), browser_download_url: String::new(), digest: None }).collect(),
        }
    }

    #[test]
    fn download_selection_matches_the_host_even_when_arm_is_first() {
        let rel = release(&["GE-Proton11-7-aarch64.tar.gz", "GE-Proton11-7-x86_64.tar.gz"]);
        assert_eq!(proton_asset(&rel, "x86_64").unwrap().name, "GE-Proton11-7-x86_64.tar.gz");
        assert_eq!(proton_asset(&rel, "aarch64").unwrap().name, "GE-Proton11-7-aarch64.tar.gz");
        assert!(proton_asset(&rel, "x86").is_err());
        assert!(proton_asset(&release(&["GE-Proton11-7-aarch64.tar.gz"]), "x86_64").is_err());
        let legacy = release(&["GE-Proton11-7.tar.gz"]);
        assert!(proton_asset(&legacy, "x86_64").is_ok());
        assert!(proton_asset(&legacy, "aarch64").is_err());
    }

    #[test]
    fn wrong_architecture_is_rejected_by_label_and_elf_header() {
        assert!(!proton_matches_arch(Path::new("/compat/GE-Proton11-7-aarch64/proton"), "x86_64"));
        assert!(!proton_matches_arch(Path::new("/compat/GE-Proton11-7-x86_64/proton"), "aarch64"));
        let root = std::env::temp_dir().join(format!("kryoto-arch-{}", std::process::id()));
        std::fs::create_dir_all(root.join("files/bin")).unwrap();
        let mut header = [0u8; 20];
        header[..4].copy_from_slice(b"\x7fELF");
        header[5] = 1;
        header[18..20].copy_from_slice(&183u16.to_le_bytes());
        std::fs::write(root.join("files/bin/wine"), header).unwrap();
        assert!(!proton_matches_arch(&root.join("proton"), "x86_64"));
        assert!(proton_matches_arch(&root.join("proton"), "aarch64"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_checks_both_flatpak_steam_layouts() {
        let home = std::env::temp_dir().join(format!("kryoto-steam-roots-{}", std::process::id()));
        for root in steam_roots(&home).into_iter().filter(|p| p.to_string_lossy().contains("com.valvesoftware.Steam")) {
            let proton = root.join("compatibilitytools.d/GE-Proton9-27/proton");
            std::fs::create_dir_all(proton.parent().unwrap()).unwrap();
            std::fs::write(&proton, "#!/bin/sh\n").unwrap();
        }
        assert_eq!(protons_in(&proton_dirs(&home)).len(), 2);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn invalid_saved_tool_recovers_to_a_detected_tool_or_explains_the_failure() {
        let tools = vec![CompatTool { name: "working".into(), path: "/working/proton".into(), kind: "proton".into(), managed: true }];
        assert_eq!(resolve_tool(Some("/missing/proton"), &tools).unwrap(), Some(PathBuf::from("/working/proton")));
        assert!(resolve_tool(Some("/missing/proton"), &[]).unwrap_err().contains("Get Proton"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn invalid_saved_wrapper_explains_the_mac_backend() {
        let error = resolve_tool(Some("/missing/Sikarugir"), &[]).unwrap_err();
        assert!(error.contains("Sikarugir"));
        assert!(resolve_tool(None, &[]).unwrap().is_none());
    }

    #[test]
    fn newer_protons_come_first() {
        let root = std::env::temp_dir().join("kryoto-compat-test");
        for name in ["GE-Proton9-27", "GE-Proton10-3", "Proton 8.0", "not-a-proton"] {
            let d = root.join(name);
            std::fs::create_dir_all(&d).unwrap();
            if name != "not-a-proton" {
                std::fs::write(d.join("proton"), "#!/bin/sh\n").unwrap();
            }
        }
        let found: Vec<String> = protons_in(std::slice::from_ref(&root)).into_iter().map(|t| t.name).collect();
        assert_eq!(found, ["Proton 8.0", "GE-Proton10-3", "GE-Proton9-27"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tools_are_known_by_their_file() {
        assert_eq!(kind_of(Path::new("/opt/GE-Proton10-3/proton")), "proton");
        assert_eq!(kind_of(Path::new("/usr/bin/umu-run")), "umu");
        #[cfg(target_os = "macos")]
        assert_eq!(kind_of(Path::new("/Users/test/Applications/Sikarugir/Game.app/Contents/MacOS/Sikarugir")), "sikarugir");
        #[cfg(target_os = "macos")]
        assert_eq!(kind_of(Path::new("/Users/test/Applications/Sikarugir/Game.app/Contents/MacOS/launcher")), "sikarugir");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(kind_of(Path::new("/Users/test/Applications/Sikarugir/Game.app/Contents/MacOS/Sikarugir")), "wine");
        assert_eq!(kind_of(Path::new("/usr/bin/wine")), "wine");
    }

    #[test]
    fn validates_a_managed_sikarugir_engine_bundle() {
        let root = std::env::temp_dir().join(format!("kryoto-sikarugir-engine-{}", std::process::id()));
        let wine = root.join("sikarugir/WS12WineSikarugir11.0_1/wswine.bundle/bin/wine");
        std::fs::create_dir_all(wine.parent().unwrap()).unwrap();
        std::fs::write(&wine, "engine").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&wine).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&wine, permissions).unwrap();
        }
        assert!(is_sikarugir_engine(&wine));
        assert!(!is_sikarugir_engine(&root.join("sikarugir/WS12WineSikarugir11.0_1/wswine.bundle/bin/wineserver")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_managed_sikarugir_engines() {
        let root = std::env::temp_dir().join(format!("kryoto-managed-sikarugir-{}", std::process::id()));
        let wine = root.join("sikarugir/WS12WineSikarugir11.0_1/wswine.bundle/bin/wine");
        std::fs::create_dir_all(wine.parent().unwrap()).unwrap();
        std::fs::write(&wine, "engine").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&wine).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&wine, permissions).unwrap();
        }
        let tools = sikarugir_engines_in(&root);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "WS12WineSikarugir11.0_1");
        assert_eq!(tools[0].kind, "wine");
        assert!(tools[0].managed);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovers_sikarugir_wrappers_and_their_prefixes() {
        let root = std::env::temp_dir().join(format!("kryoto-sikarugir-{}", std::process::id()));
        let app = root.join("Example Game.app");
        let launcher = app.join("Contents/MacOS/Sikarugir");
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::write(&launcher, "wrapper").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&launcher).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&launcher, permissions).unwrap();
        }

        let tools = sikarugir_in(std::slice::from_ref(&root));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "Example Game");
        assert_eq!(tools[0].kind, "sikarugir");
        assert_eq!(sikarugir_wrapper(Path::new(&tools[0].path)), Some(app.clone()));
        assert_eq!(sikarugir_prefix(Path::new(&tools[0].path)), Some(app.join("Contents/SharedSupport/prefix")));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn wrapper_validation_requires_the_real_launcher_location() {
        let root = std::env::temp_dir().join(format!("kryoto-sikarugir-validation-{}", std::process::id()));
        let valid = root.join("Valid.app");
        let launcher = valid.join("Contents/MacOS/Sikarugir");
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        std::fs::write(&launcher, "wrapper").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&launcher).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&launcher, permissions).unwrap();
        }
        assert!(is_sikarugir_wrapper(&valid));
        assert_eq!(sikarugir_wrapper(&launcher), Some(valid.clone()));

        let legacy = root.join("Legacy.app");
        let legacy_launcher = legacy.join("Contents/MacOS/launcher");
        std::fs::create_dir_all(legacy_launcher.parent().unwrap()).unwrap();
        std::fs::write(&legacy_launcher, "wrapper").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&legacy_launcher).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&legacy_launcher, permissions).unwrap();
        }
        assert!(is_sikarugir_wrapper(&legacy));
        assert_eq!(sikarugir_wrapper(&legacy_launcher), Some(legacy));

        let missing = root.join("Missing.app");
        std::fs::create_dir_all(missing.join("Contents/MacOS")).unwrap();
        assert!(!is_sikarugir_wrapper(&missing));
        assert!(sikarugir_wrapper(&missing.join("Contents/MacOS/Sikarugir")).is_none());

        let _ = std::fs::remove_dir_all(root);
    }
}
