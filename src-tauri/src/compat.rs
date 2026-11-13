//! On Linux, the Wine and Proton builds already on this computer, so a game
//! can be played without anyone typing a path.
//!
//! Looked for where Steam, Proton-GE and umu put them, newest Proton first:
//! Proton runs these games best, umu-run is Proton outside Steam, and plain
//! Wine is the last resort. Windows has none of this and gets an empty list.
//!
//! Like Steam, the client can also get them itself: `compat_install` puts the
//! newest Proton-GE and umu-launcher in the app's own `compat` folder, so a
//! Linux player never has to install Wine or Proton by hand. Those come first,
//! and a Proton then runs through umu, inside the Steam Linux Runtime.

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
    /// `proton`, `umu` or `wine`.
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

/// umu-run: the client's own, else one on PATH. Never on Windows.
pub fn umu_run(managed: Option<&Path>) -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    managed
        .map(|m| m.join("umu").join("umu-run"))
        .filter(|p| p.is_file())
        .or_else(|| on_path("umu-run"))
}

/// Every `<dir>/*/proton` under the folders Steam and Proton-GE use.
#[cfg_attr(not(unix), allow(dead_code))]
fn protons_in(dirs: &[PathBuf]) -> Vec<CompatTool> {
    let mut out = Vec::new();
    // `~/.steam/steam`, `~/.steam/root` and `~/.local/share/Steam` are usually
    // links to one folder, so the same Proton was listed two or three times
    // (KB-0315): one entry per real file, under the first path that found it.
    let mut seen: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(dir) else { continue };
        for entry in read.flatten() {
            let script = entry.path().join("proton");
            if script.is_file() && proton_matches_arch(&script, std::env::consts::ARCH) {
                let real = std::fs::canonicalize(&script).unwrap_or_else(|_| script.clone());
                if seen.contains(&real) {
                    continue;
                }
                seen.push(real);
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = script.to_string_lossy().into_owned();
                out.push(CompatTool { name, path, kind: "proton".into(), managed: false });
            }
        }
    }
    // Newest first: "GE-Proton10-3" before "GE-Proton9-27", "Proton 9.0" before "Proton 8.0".
    out.sort_by_key(|t| std::cmp::Reverse(natural_key(&t.name)));
    out
}

/// A sort key that compares the numbers in a name as numbers.
#[cfg_attr(not(unix), allow(dead_code))]
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

#[cfg_attr(not(unix), allow(dead_code))]
fn proton_dirs(home: &Path) -> Vec<PathBuf> {
    let steams = steam_roots(home);
    let mut dirs: Vec<PathBuf> = steams.iter().flat_map(|s| [s.join("compatibilitytools.d"), s.join("steamapps/common")]).collect();
    dirs.push(PathBuf::from("/usr/share/steam/compatibilitytools.d"));
    dirs
}

/// Steam's own Windows client files, which Proton puts in a prefix so a game
/// can reach the real Steam (`legacycompat/` in the Steam folder).
const STEAM_CLIENT_FILES: [&str; 6] =
    ["steamclient.dll", "steamclient64.dll", "GameOverlayRenderer.dll", "GameOverlayRenderer64.dll", "SteamService.exe", "Steam.dll"];

/// The Steam folder that has the client files Proton copies, if Steam is installed.
pub fn steam_client_root(home: &Path) -> Option<PathBuf> {
    steam_roots(home).into_iter().find(|r| r.join("legacycompat").join("steamclient64.dll").is_file())
}

/// Linux: a game whose online layer talks to the real Steam (Online-Fix,
/// Kryoto Online) under Proton. Proton sets the prefix up with Steam's client
/// files from `STEAM_COMPAT_CLIENT_INSTALL_PATH`, which Steam sets for its own
/// games and a start from Kryoto did not; umu even sets it empty. Without
/// them Online-Fix stops with its error 998 and Kryoto Online cannot reach
/// Steam. This says where Steam is, for a Proton started directly, and puts
/// the files in the prefix where Proton would, for one through umu. A
/// launch options line that sets the variable wins. Returns what it did, for
/// the game's log; None when there is nothing to do or no Steam.
pub fn steam_client_for(env: &mut Vec<(String, String)>, home: &Path) -> Option<String> {
    let overrides = env.iter().find(|(k, _)| k == "WINEDLLOVERRIDES").map(|(_, v)| v.to_ascii_lowercase()).unwrap_or_default();
    if !(overrides.contains("onlinefix64") || overrides.contains("kryotoo")) {
        return None;
    }
    let Some(steam) = steam_client_root(home) else {
        return Some("Steam is not installed, so Proton has no Steam client to set up for this game's online layer.".into());
    };
    if !env.iter().any(|(k, _)| k == "STEAM_COMPAT_CLIENT_INSTALL_PATH") {
        env.push(("STEAM_COMPAT_CLIENT_INSTALL_PATH".into(), steam.to_string_lossy().into_owned()));
    }
    let prefix = env.iter().find(|(k, _)| k == "WINEPREFIX").map(|(_, v)| PathBuf::from(v));
    let mut copied = 0;
    for drive in prefix.iter().flat_map(|p| [p.join("drive_c"), p.join("pfx").join("drive_c")]).filter(|d| d.is_dir()) {
        let dst = drive.join("Program Files (x86)").join("Steam");
        if std::fs::create_dir_all(&dst).is_err() {
            continue;
        }
        for f in STEAM_CLIENT_FILES {
            let from = steam.join("legacycompat").join(f);
            let to = dst.join(f);
            let len = |p: &Path| std::fs::metadata(p).map(|m| m.len()).ok();
            if from.is_file() && len(&from) != len(&to) && std::fs::copy(&from, &to).is_ok() {
                copied += 1;
            }
        }
    }
    Some(format!("Steam's client files for Proton come from {}{}.", steam.display(), if copied > 0 { format!(" ({copied} copied into the prefix)") } else { String::new() }))
}

pub fn steam_roots(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".steam/steam"),
        home.join(".steam/root"),
        home.join(".local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        home.join("snap/steam/common/.local/share/Steam"),
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
        let resolved = if path.components().count() == 1 { on_path(saved) } else { path.is_file().then_some(path.clone()) };
        if let Some(tool) = resolved {
            if kind_of(&tool) != "proton" || proton_matches_arch(&tool, std::env::consts::ARCH) {
                return Ok(Some(tool));
            }
        }
        // A valid replacement avoids leaving existing installations broken.
        if let Some(tool) = tools.first() {
            return Ok(Some(PathBuf::from(&tool.path)));
        }
        return Err(format!("The compatibility tool {saved} is missing or does not match this computer ({}). Get Proton in Settings > Compatibility, or pick an installed Wine tool.", std::env::consts::ARCH));
    }
    Ok(tools.first().map(|t| PathBuf::from(&t.path)))
}

#[cfg(unix)]
pub fn detect_in(managed: Option<&Path>) -> Vec<CompatTool> {
    let mut out = Vec::new();
    if let Some(m) = managed {
        out.extend(protons_in(&[m.to_path_buf()]).into_iter().map(|t| CompatTool { managed: true, ..t }));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let real = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
        for t in protons_in(&proton_dirs(&home)) {
            if !out.iter().any(|o| o.path == t.path || real(&o.path) == real(&t.path)) {
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
        umu: umu_run(managed.as_deref()).map(|p| p.to_string_lossy().into_owned()),
        mangohud: on_path("mangohud").is_some(),
        gamemode: on_path("gamemoderun").is_some(),
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Clone, Serialize, Deserialize)]
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

/// How long a looked-up release is used before GitHub is asked again.
const RELEASE_FRESH_SECS: u64 = 6 * 60 * 60;

#[derive(Serialize, Deserialize)]
struct CachedRelease {
    at: u64,
    release: Release,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn release_cache(dir: &Path, repo: &str) -> PathBuf {
    dir.join(format!(".release-{}.json", repo.replace('/', "--")))
}

/// The newest release of `repo` and its downloads.
///
/// GitHub's API allows 60 unauthenticated requests an hour per address, which
/// players behind one shared address (CGNAT, a VPN) run out of (KB-0252). So
/// an answer is kept for six hours in the `compat` folder, a failed request is
/// tried once more, and when the API still says no, the same facts come from
/// github.com's own pages (the `releases/latest` redirect names the tag, the
/// release's asset list names the files), which that limit does not cover. A
/// kept answer of any age beats failing.
async fn latest(client: &reqwest::Client, repo: &str, dir: &Path) -> Result<Release, String> {
    let cache = release_cache(dir, repo);
    let kept = std::fs::read(&cache).ok().and_then(|b| serde_json::from_slice::<CachedRelease>(&b).ok());
    if let Some(k) = &kept {
        if now_secs().saturating_sub(k.at) < RELEASE_FRESH_SECS && !k.release.assets.is_empty() {
            return Ok(k.release.clone());
        }
    }
    let found = match from_api(client, repo).await {
        Ok(r) => Ok(r),
        Err(api) => {
            crate::logging::warn("compat", &format!("{repo}: GitHub API: {}; reading the release page instead", api.message));
            from_release_page(client, repo).await.map_err(|page| (api, page))
        }
    };
    match found {
        Ok(release) => {
            if let Ok(bytes) = serde_json::to_vec(&CachedRelease { at: now_secs(), release: release.clone() }) {
                let _ = std::fs::write(&cache, bytes);
            }
            Ok(release)
        }
        Err((api, page)) => {
            if let Some(k) = kept.filter(|k| !k.release.assets.is_empty()) {
                crate::logging::warn("compat", &format!("{repo}: using the release looked up earlier ({}): {page}", k.release.tag_name));
                return Ok(k.release);
            }
            Err(if api.limited {
                "GitHub is not answering this network right now: it allows 60 checks an hour without an account, shared by everyone on the same connection or VPN. Try again in an hour, or on another network.".into()
            } else {
                format!("Could not reach GitHub to get the newest version: {}", api.message)
            })
        }
    }
}

struct ApiError {
    /// GitHub's rate limit (403 or 429), as opposed to the network.
    limited: bool,
    message: String,
}

async fn from_api(client: &reqwest::Client, repo: &str) -> Result<Release, ApiError> {
    let mut last = ApiError { limited: false, message: String::new() };
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        let res = client
            .get(format!("https://api.github.com/repos/{repo}/releases/latest"))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => {
                return r.json().await.map_err(|e| ApiError { limited: false, message: format!("GitHub answered oddly: {e}") });
            }
            Ok(r) => {
                let status = r.status();
                let limited = status.as_u16() == 403 || status.as_u16() == 429;
                last = ApiError { limited, message: format!("HTTP {status}") };
                // A limit does not lift in two seconds; a server hiccup might.
                if status.is_client_error() {
                    break;
                }
            }
            Err(e) => last = ApiError { limited: false, message: e.to_string() },
        }
    }
    Err(last)
}

/// The tag `github.com/<repo>/releases/latest` sends you on to.
fn tag_from_latest_url(repo: &str, url: &str) -> Option<String> {
    let prefix = format!("https://github.com/{repo}/releases/tag/");
    let tag = url.strip_prefix(&prefix)?.split(['?', '#']).next()?;
    (!tag.is_empty() && !tag.contains('/')).then(|| tag.to_string())
}

/// The downloads a release page lists, as `/<repo>/releases/download/<tag>/<file>` links.
fn assets_from_page(repo: &str, tag: &str, html: &str) -> Vec<Asset> {
    let prefix = format!("/{repo}/releases/download/{tag}/");
    let mut out: Vec<Asset> = Vec::new();
    for part in html.split("href=\"").skip(1) {
        let Some(href) = part.split('"').next() else { continue };
        let href = href.replace("&amp;", "&");
        let Some(name) = href.strip_prefix(&prefix) else { continue };
        if name.is_empty() || name.contains('/') || out.iter().any(|a| a.name == name) {
            continue;
        }
        out.push(Asset { name: name.to_string(), browser_download_url: format!("https://github.com{href}") });
    }
    out
}

async fn from_release_page(client: &reqwest::Client, repo: &str) -> Result<Release, String> {
    let res = client
        .get(format!("https://github.com/{repo}/releases/latest"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let tag = tag_from_latest_url(repo, res.url().as_str()).ok_or_else(|| format!("no release tag at {}", res.url()))?;
    let html = client
        .get(format!("https://github.com/{repo}/releases/expanded_assets/{tag}"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let assets = assets_from_page(repo, &tag, &html);
    if assets.is_empty() {
        return Err(format!("{tag} lists no downloads"));
    }
    Ok(Release { tag_name: tag, assets })
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
    let rel = latest(&client, "GloriousEggroll/proton-ge-custom", &dir).await?;
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
        let umu = latest(client, "Open-Wine-Components/umu-launcher", dir).await?;
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
    fn release_facts_come_from_github_pages_when_the_api_is_limited() {
        let repo = "Open-Wine-Components/umu-launcher";
        assert_eq!(tag_from_latest_url(repo, "https://github.com/Open-Wine-Components/umu-launcher/releases/tag/1.4.4").as_deref(), Some("1.4.4"));
        assert_eq!(tag_from_latest_url(repo, "https://github.com/Open-Wine-Components/umu-launcher/releases"), None);
        assert_eq!(tag_from_latest_url(repo, "https://github.com/someone/else/releases/tag/1.0"), None);
        let html = r#"<a href="/Open-Wine-Components/umu-launcher/releases/download/1.4.4/umu-launcher-1.4.4-zipapp.tar" rel="nofollow">
            <a href="/Open-Wine-Components/umu-launcher/releases/download/1.4.4/umu-launcher-1.4.4-zipapp.tar">
            <a href="/Open-Wine-Components/umu-launcher/archive/refs/tags/1.4.4.zip">
            <a href="/evil/repo/releases/download/1.4.4/umu-launcher-1.4.4-zipapp.tar">"#;
        let assets = assets_from_page(repo, "1.4.4", html);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].name, "umu-launcher-1.4.4-zipapp.tar");
        assert_eq!(assets[0].browser_download_url, "https://github.com/Open-Wine-Components/umu-launcher/releases/download/1.4.4/umu-launcher-1.4.4-zipapp.tar");
    }

    /// Against the real github.com: `cargo test --lib compat::tests::release_pages_live -- --ignored`.
    #[test]
    #[ignore]
    fn release_pages_live() {
        let client = reqwest::Client::builder().user_agent("KryotoDesktop/test").build().unwrap();
        tauri::async_runtime::block_on(async {
            let proton = from_release_page(&client, "GloriousEggroll/proton-ge-custom").await.unwrap();
            proton_asset(&proton, "x86_64").unwrap();
            assert!(proton.assets.iter().any(|a| a.name.ends_with(".sha512sum")));
            let umu = from_release_page(&client, "Open-Wine-Components/umu-launcher").await.unwrap();
            assert!(umu.assets.iter().any(|a| a.name.ends_with("zipapp.tar")));
        });
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

    #[cfg(unix)]
    #[test]
    fn a_steam_folder_reached_through_its_links_lists_each_proton_once() {
        // KB-0315: ~/.steam/steam and ~/.steam/root link to ~/.local/share/Steam.
        let home = std::env::temp_dir().join(format!("kryoto-steam-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let real = home.join(".local/share/Steam");
        let proton = real.join("compatibilitytools.d/GE-Proton10-3/proton");
        std::fs::create_dir_all(proton.parent().unwrap()).unwrap();
        std::fs::write(&proton, "#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(home.join(".steam")).unwrap();
        std::os::unix::fs::symlink(&real, home.join(".steam/steam")).unwrap();
        std::os::unix::fs::symlink(&real, home.join(".steam/root")).unwrap();
        let found = protons_in(&proton_dirs(&home));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].name, "GE-Proton10-3");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn invalid_saved_tool_recovers_to_a_detected_tool_or_explains_the_failure() {
        let tools = vec![CompatTool { name: "working".into(), path: "/working/proton".into(), kind: "proton".into(), managed: true }];
        assert_eq!(resolve_tool(Some("/missing/proton"), &tools).unwrap(), Some(PathBuf::from("/working/proton")));
        assert!(resolve_tool(Some("/missing/proton"), &[]).unwrap_err().contains("Get Proton"));
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
        assert_eq!(kind_of(Path::new("/usr/bin/wine")), "wine");
    }

    #[test]
    fn online_layers_get_steams_client_files_under_proton() {
        let home = std::env::temp_dir().join(format!("kryo-steamclient-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let prefix = home.join("prefix");
        std::fs::create_dir_all(prefix.join("pfx/drive_c")).unwrap();
        let env = |overrides: &str| vec![("WINEDLLOVERRIDES".to_string(), overrides.to_string()), ("WINEPREFIX".to_string(), prefix.to_string_lossy().into_owned())];
        // A plain game is left alone.
        let mut plain = env("steam_api64=n,b");
        assert!(steam_client_for(&mut plain, &home).is_none());
        assert_eq!(plain.len(), 2);
        // No Steam: said, nothing set.
        let mut of = env("OnlineFix64=n;SteamOverlay64=n;winmm=n,b;dnet=n;steam_api64=n");
        assert!(steam_client_for(&mut of, &home).unwrap().contains("not installed"));
        assert!(!of.iter().any(|(k, _)| k == "STEAM_COMPAT_CLIENT_INSTALL_PATH"));
        // Steam installed: where it is, and its files in the prefix.
        let steam = home.join(".local/share/Steam");
        std::fs::create_dir_all(steam.join("legacycompat")).unwrap();
        std::fs::write(steam.join("legacycompat/steamclient64.dll"), b"64").unwrap();
        std::fs::write(steam.join("legacycompat/steamclient.dll"), b"32").unwrap();
        let note = steam_client_for(&mut of, &home).unwrap();
        assert!(note.contains("2 copied"), "{note}");
        assert_eq!(of.iter().find(|(k, _)| k == "STEAM_COMPAT_CLIENT_INSTALL_PATH").map(|(_, v)| PathBuf::from(v)), Some(steam.clone()));
        assert_eq!(std::fs::read(prefix.join("pfx/drive_c/Program Files (x86)/Steam/steamclient64.dll")).unwrap(), b"64");
        // The launch options line's own value wins; files already there are not copied again.
        let mut own = env("kryotoO=n,b");
        own.push(("STEAM_COMPAT_CLIENT_INSTALL_PATH".into(), "/mine".into()));
        assert!(!steam_client_for(&mut own, &home).unwrap().contains("copied"));
        assert_eq!(own.iter().filter(|(k, _)| k == "STEAM_COMPAT_CLIENT_INSTALL_PATH").count(), 1);
        let _ = std::fs::remove_dir_all(&home);
    }
}
