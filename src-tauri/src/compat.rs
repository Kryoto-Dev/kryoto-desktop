//! On Linux, the Wine and Proton builds already on this computer, so a game
//! can be played without anyone typing a path. On macOS, Windows games use a
//! Sikarugir wrapper: Sikarugir is a Wineskin-style app bundle whose launcher
//! owns the Wine engine, renderers and prefix.
//!
//! Looked for where Steam, Proton-GE and umu put them, newest Proton first:
//! Proton runs these games best, umu-run is Proton outside Steam, and plain
//! Wine is the last resort. Windows has none of this and gets an empty list.
//!
//! Like Steam, the client can also get them itself: `compat_install` puts the
//! newest Proton-GE and umu-launcher in the app's own `compat` folder, so a
//! Linux player never has to install Wine or Proton by hand. Those come first,
//! and a Proton then runs through umu, inside the Steam Linux Runtime. macOS
//! does not use Proton or a host `wine` binary: it discovers wrappers in the
//! standard Sikarugir folder and starts their `Contents/MacOS/Sikarugir` (or
//! legacy `Contents/MacOS/launcher`) launcher.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

// Several Play presses and Settings can need the same missing runtime.
// Serialize writes so they cannot unpack over one another's download.
static INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompatTool {
    /// What to call it: the folder name for a Proton, else the program.
    pub name: String,
    pub path: String,
    /// `proton`, `umu`, `wine` or `sikarugir`.
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

/// The launcher inside a Sikarugir wrapper. Sikarugir's Homebrew setup keeps
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
            // A macOS selection must be the launcher in a real wrapper, not a
            // same-named executable copied somewhere else. Sikarugir's prefix
            // and renderer are relative to that bundle.
            (path.components().count() != 1 && sikarugir_wrapper(&path).is_some()).then_some(path.clone())
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
            !cfg!(target_os = "macos") || sikarugir_wrapper(Path::new(&tool.path)).is_some()
        }) {
            return Ok(Some(PathBuf::from(&tool.path)));
        }
        if cfg!(target_os = "macos") {
            return Err(format!("The Sikarugir wrapper {saved} is missing or invalid. Pick a wrapper from ~/Applications/Sikarugir in Settings > Compatibility."));
        }
        return Err(format!("The compatibility tool {saved} is missing or does not match this computer ({}). Get Proton in Settings > Compatibility, or pick an installed Wine tool.", std::env::consts::ARCH));
    }
    Ok(tools
        .iter()
        .find(|tool| !cfg!(target_os = "macos") || sikarugir_wrapper(Path::new(&tool.path)).is_some())
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
pub fn detect_in(_managed: Option<&Path>) -> Vec<CompatTool> {
    sikarugir_in(&sikarugir_dirs())
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

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
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

/// Unpack with the system's tar, which every Linux has.
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

fn sha512_of(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha512};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha512::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Get the newest Proton-GE and umu-launcher into the app's `compat` folder,
/// the way Steam gets Proton for you. Returns the Proton, ready to pick.
#[tauri::command]
pub async fn compat_install(app: AppHandle) -> Result<CompatTool, String> {
    if cfg!(windows) {
        return Err("Windows runs these games itself.".into());
    }
    if cfg!(target_os = "macos") {
        return Err("Install Sikarugir separately, create a wrapper, and keep it in ~/Applications/Sikarugir. Kryoto will detect it when you refresh this page.".into());
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
    let folder = tool.parent().ok_or("Not one of Kryoto's tools.")?.to_path_buf();
    if folder.parent() != Some(root.as_path()) {
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

    fn release(names: &[&str]) -> Release {
        Release {
            tag_name: "GE-Proton11-7".into(),
            assets: names.iter().map(|name| Asset { name: (*name).into(), browser_download_url: String::new() }).collect(),
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
