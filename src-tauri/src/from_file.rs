//! Installing a game from an archive the player already has: one they
//! downloaded from kryo.to in a browser, or from one of a game's mirrors, so
//! it is not downloaded a second time.
//!
//! Nothing here installs anything itself. The file is looked at (`archive_look`:
//! is it a whole 7z archive, which game and build its name and size point to),
//! then queued as a download row whose archive is already on disk
//! (`downloads::enqueue_file`). From there it is a finished download: read for
//! its SHA-256 with the row's progress, matched against every file kryo.to lists
//! for the game (`match_sha`, the same hashes a download is checked against),
//! the same room check, unpacker and fallbacks, library entry and install
//! record. The file is the player's: never moved, deleted or unpacked in place.
//!
//! What leaves this PC: the game's name from the file name (a search on
//! kryo.to, as Add a game does) and the game's public listing. The hash is
//! compared here; nothing is uploaded.

use crate::downloads;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;
use tauri::AppHandle;

/// What the dialog shows about a file before it is installed.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Look {
    pub path: String,
    pub file_name: String,
    pub size: u64,
    /// The kryo.to game it is for: picked by the player, else found by the
    /// file's name. None: the player picks it ("Which game is this?").
    pub slug: Option<String>,
    pub title: Option<String>,
    pub cover: Option<String>,
    pub nsfw: bool,
    /// The build its size points to.
    pub version: Option<String>,
    /// It is one of the game's add-ons (a DLC, a language pack): its name.
    pub addon: Option<String>,
    /// `Some(true)`: kryo.to lists a file of exactly this size for the game;
    /// `Some(false)`: it lists sizes and none is this one, so the file will
    /// not match; None: kryo.to does not say, or could not be reached.
    pub size_match: Option<bool>,
    /// The game (for an add-on, the game it goes into) is in the library.
    pub installed: bool,
    /// kryo.to answered about the game.
    pub reachable: bool,
}

/// A whole, single-file 7z archive at `path`, checked before anything is
/// written: its name and size. `Err` says, in the player's words, what it is
/// instead. The archive's own checks are its start header's and end header's
/// CRCs, and the length its start header declares against the file's.
pub fn check_file(path: &Path) -> Result<(String, u64), String> {
    let meta = std::fs::metadata(path).map_err(|_| "That file is not there any more.".to_string())?;
    if meta.is_dir() {
        return Err("That is a folder. Pick the game's .7z file instead.".into());
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    if is_split_part(&lower) {
        return Err(format!("{name} is one part of a split archive. Kryoto installs whole .7z files, the way kryo.to and its mirrors serve them."));
    }
    let ext = Path::new(&lower).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    if ext == "exe" || ext == "appimage" || ext == "msi" {
        return Err(format!("{name} is a program, not a game's archive. Pick the .7z file the game downloads as."));
    }
    if ext != "7z" {
        let what = if ext.is_empty() { "a file with no type".to_string() } else { format!("a .{ext} file") };
        return Err(format!("{name} is {what}. Kryoto installs .7z archives, the files kryo.to and its mirrors serve."));
    }
    let size = meta.len();
    let mut head = [0u8; 32];
    let read = {
        use std::io::Read;
        let mut f = std::fs::File::open(path).map_err(|e| format!("Could not open {name}: {}", crate::logging::why(&e)))?;
        let mut n = 0;
        while n < head.len() {
            match f.read(&mut head[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) => return Err(format!("Could not read {name}: {}", crate::logging::why(&e))),
            }
        }
        n
    };
    let head = &head[..read];
    let Some(declared) = downloads::seven_z_length(head) else {
        return Err(format!(
            "{name} is not a 7z archive: it is {}. A download that was cut off or saved as a web page looks like this. Download it again.",
            downloads::looks_like(head)
        ));
    };
    if declared > size {
        return Err(format!(
            "{name} is cut short: {} of {}. A copy that stopped part way cannot be installed. Download it again.",
            crate::storage::human_size(size),
            crate::storage::human_size(declared)
        ));
    }
    match sevenz_rust::Archive::open(path) {
        Ok(_) => Ok((name, size)),
        Err(sevenz_rust::Error::PasswordRequired | sevenz_rust::Error::MaybeBadPassword(_)) => {
            Err(format!("{name} wants a password, which Kryoto does not have. kryo.to's files never do: this is not one of them."))
        }
        // The built-in reader is older than some 7-Zip features: not damage.
        // The unpacker (7-Zip itself) reads them.
        Err(sevenz_rust::Error::UnsupportedCompressionMethod(_) | sevenz_rust::Error::ExternalUnsupported | sevenz_rust::Error::Unsupported(_) | sevenz_rust::Error::MaxMemLimited { .. }) => Ok((name, size)),
        Err(e) => Err(format!("{name} is damaged: its own check failed ({}). Download it again.", crate::logging::why(&e))),
    }
}

/// `Game.7z.001`, `Game.part1.rar`, `Game.z01`: one piece of several.
fn is_split_part(lower: &str) -> bool {
    let last = lower.rsplit('.').next().unwrap_or("");
    (last.len() == 3 && last.chars().all(|c| c.is_ascii_digit()))
        || (last.len() == 3 && last.starts_with('z') && last[1..].chars().all(|c| c.is_ascii_digit()))
        || lower.rsplit_once(".part").is_some_and(|(_, rest)| rest.split('.').next().is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())))
}

/// The names a file might be listed under on kryo.to, best first: a DLC's
/// game (`<game> - DLC <name> - kryoto.7z`), the title its name carries, and
/// that title without what a host or a language pack adds after ` - `, or a
/// version.
pub fn guess_names(file_name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.trim().trim_matches(|c: char| c == '-' || c == '_' || c.is_whitespace()).to_string();
        if s.chars().filter(|c| c.is_alphanumeric()).count() >= 2 && !out.iter().any(|o| o.eq_ignore_ascii_case(&s)) {
            out.push(s);
        }
    };
    if let Some((game, _)) = downloads::dlc_parts(file_name) {
        push(&game);
    }
    let Some(title) = downloads::title_in_file_name(file_name) else { return out };
    // Hosts write spaces as dots or underscores.
    let spaced = if title.contains(' ') { title.clone() } else { title.replace(['_', '.'], " ") };
    push(&spaced);
    if let Some((first, _)) = spaced.split_once(" - ") {
        push(first);
    }
    // Up to a version: "Hollow Knight v1.5.78", "Hollow Knight v1 5 78".
    let words: Vec<&str> = spaced.split_whitespace().collect();
    let versionish = |w: &str| {
        let bare = w.strip_prefix(['v', 'V']);
        bare.is_some_and(|b| !b.is_empty() && b.chars().next().is_some_and(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit() || c == '.'))
            || (w.contains('.') && w.chars().next().is_some_and(|c| c.is_ascii_digit()) && w.chars().all(|c| c.is_ascii_digit() || c == '.'))
    };
    if let Some(at) = words.iter().skip(1).position(|w| versionish(w)) {
        push(&words[..at + 1].join(" "));
    }
    out
}

/// The build a file's name says it is (`Game v1.2.3 - kryoto.7z`), or None.
/// What an unverified install is recorded as: never kryo.to's current build.
pub fn version_in_name(file_name: &str) -> Option<String> {
    let stem = file_name.trim();
    let stem = match stem.rsplit_once('.') {
        Some((s, ext)) if (1..=4).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) => s,
        _ => stem,
    };
    let words: Vec<&str> = stem.split(|c: char| c.is_whitespace() || "()[]_".contains(c)).filter(|w| !w.is_empty()).collect();
    for (i, w) in words.iter().enumerate() {
        let bare = w.strip_prefix(['v', 'V']).unwrap_or(w);
        let dotted = bare.contains('.') && bare.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric())) && bare.chars().next().is_some_and(|c| c.is_ascii_digit());
        let tagged = bare.len() < w.len() && !bare.is_empty() && bare.chars().all(|c| c.is_ascii_digit());
        if dotted || tagged {
            return Some(bare.to_string());
        }
        if w.eq_ignore_ascii_case("build") {
            if let Some(n) = words.get(i + 1).filter(|n| n.chars().all(|c| c.is_ascii_digit())) {
                return Some(n.to_string());
            }
        }
    }
    None
}

/// The release or add-on a SHA-256 is, in a `/api/games/<slug>/downloads`
/// answer: every full release (archived ones too, by the hashes kept for
/// them), else the main one's own list, else an add-on.
#[derive(Debug, Default, PartialEq)]
pub struct ShaMatch {
    /// The release's version.
    pub version: Option<String>,
    /// An add-on's name.
    pub addon: Option<String>,
    /// The add-on is a repair fix (`kind: "repair"`).
    pub repair: bool,
}

impl ShaMatch {
    pub fn what(&self) -> String {
        match (&self.addon, &self.version) {
            (Some(a), _) => format!("add-on {a}"),
            (None, Some(v)) => format!("build {v}"),
            (None, None) => "current build".into(),
        }
    }
}

pub fn match_sha(json: &serde_json::Value, sha: &str) -> Option<ShaMatch> {
    if sha.len() != 64 {
        return None;
    }
    let has = |v: &serde_json::Value| v["hashes"].as_array().into_iter().flatten().any(|h| h["sha256"].as_str().is_some_and(|s| s.eq_ignore_ascii_case(sha)));
    let text = |v: &serde_json::Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    for r in json["releases"].as_array().into_iter().flatten() {
        if has(r) {
            return Some(ShaMatch { version: text(&r["version"]), ..Default::default() });
        }
    }
    if has(json) {
        return Some(ShaMatch { version: text(&json["version"]), ..Default::default() });
    }
    for a in json["addons"].as_array().into_iter().flatten() {
        if has(a) {
            return Some(ShaMatch {
                version: None,
                addon: Some(text(&a["label"]).unwrap_or_else(|| "Add-on".into())),
                repair: a["kind"].as_str() == Some("repair"),
            });
        }
    }
    None
}

/// What a file of `size` bytes called `file_name` most likely is, before it
/// is read: a release or add-on listing a file of exactly that size (each
/// listed hash carries its file's size), else an add-on by its name.
#[derive(Debug, Default, PartialEq)]
pub struct Guess {
    pub version: Option<String>,
    pub addon: Option<String>,
    pub size_match: Option<bool>,
}

pub fn guess_release(json: &serde_json::Value, file_name: &str, size: u64) -> Guess {
    let sizes = |v: &serde_json::Value| -> Vec<u64> { v["hashes"].as_array().into_iter().flatten().filter_map(|h| h["bytes"].as_u64()).collect() };
    let text = |v: &serde_json::Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let releases: Vec<&serde_json::Value> = json["releases"].as_array().into_iter().flatten().collect();
    let addons: Vec<&serde_json::Value> = json["addons"].as_array().into_iter().flatten().collect();
    // The main release first, so two builds of one size read as the current.
    let mut ordered = releases.clone();
    ordered.sort_by_key(|r| !r["primary"].as_bool().unwrap_or(false));
    if let Some(r) = ordered.iter().find(|r| sizes(r).contains(&size)) {
        return Guess { version: text(&r["version"]), addon: None, size_match: Some(true) };
    }
    if sizes(json).contains(&size) {
        return Guess { version: text(&json["version"]), addon: None, size_match: Some(true) };
    }
    let label = |a: &serde_json::Value| text(&a["label"]).unwrap_or_else(|| "Add-on".into());
    if let Some(a) = addons.iter().find(|a| sizes(a).contains(&size)) {
        return Guess { version: None, addon: Some(label(a)), size_match: Some(true) };
    }
    let listed = releases.iter().chain(addons.iter()).any(|v| !sizes(v).is_empty()) || !sizes(json).is_empty();
    // By name: kryo.to's own name for an add-on's file, or a DLC's naming.
    let named = |a: &&&serde_json::Value| {
        let same = |n: &serde_json::Value| n.as_str().is_some_and(|n| downloads::safe_name(n).eq_ignore_ascii_case(&downloads::safe_name(file_name)));
        a["hashes"].as_array().into_iter().flatten().any(|h| same(&h["name"])) || a["links"].as_array().into_iter().flatten().any(|l| same(&l["name"]))
    };
    let addon = addons.iter().find(named).map(|a| label(a)).or_else(|| downloads::dlc_label(file_name));
    Guess { version: None, addon, size_match: listed.then_some(false) }
}

/// 7-Zip's own test of every file in the archive (`7z t`), with progress over
/// the archive's size. `Ok(true)` when it passed, `Ok(false)` when no 7-Zip on
/// this PC could run it (the headers were still checked, `check_file`), `Err`
/// with what 7-Zip found, or `STOPPED`.
pub fn test_archive(archive: &Path, progress: &dyn Fn(u64, Option<u64>), stop: &AtomicU8) -> Result<bool, String> {
    use std::io::Read;
    let size = std::fs::metadata(archive).map(|m| m.len()).unwrap_or(0);
    let mut unsupported = Vec::new();
    for tool in downloads::seven_zip_tools(true) {
        let mut cmd = std::process::Command::new(&tool);
        // -bso0: no listing. -bsp1: progress on stdout. -bse2: errors on stderr.
        cmd.arg("t").arg("-bso0").arg("-bsp1").arg("-bse2").arg("-p-").arg(archive);
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        crate::launch::clean_appimage_env(&mut cmd);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(_) => continue,
        };
        downloads::full_speed_child(&child);
        let Some(mut stderr) = child.stderr.take() else { continue };
        let errors = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        progress(0, Some(size));
        if let Some(mut out) = child.stdout.take() {
            let mut buf = [0u8; 4096];
            let mut line = Vec::new();
            let mut last = Instant::now();
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if stop.load(Ordering::SeqCst) != downloads::RUN {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(downloads::STOPPED.into());
                }
                for &b in &buf[..n] {
                    if matches!(b, b'\x08' | b'\r' | b'\n') {
                        if let Some(pct) = downloads::seven_zip_percent(&String::from_utf8_lossy(&line)) {
                            if last.elapsed().as_millis() > 200 {
                                progress(size * pct / 100, Some(size));
                                last = Instant::now();
                            }
                        }
                        line.clear();
                    } else {
                        line.push(b);
                    }
                }
            }
        }
        let status = child.wait();
        let errors = errors.join().unwrap_or_default();
        match status {
            Ok(s) if s.success() => {
                progress(size, Some(size));
                return Ok(true);
            }
            Ok(_) => {
                let lower = errors.to_ascii_lowercase();
                // An older 7-Zip than the archive needs (7zr without the ARM64
                // filter): another tool may read it; it is not damage.
                if lower.contains("unsupported") {
                    unsupported.push(tool.display().to_string());
                    continue;
                }
                if let Some(damaged) = downloads::damaged_files(&errors) {
                    let (first, rest) = damaged.split_first().map(|(f, r)| (f.clone(), r.len())).unwrap_or_default();
                    return Err(if rest == 0 { format!("7-Zip found damage in {first}") } else { format!("7-Zip found damage in {first} and {rest} more files") });
                }
                let said = errors.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("it failed its test").trim_start_matches("ERROR:").trim().to_string();
                return Err(format!("7-Zip says: {said}"));
            }
            Err(_) => continue,
        }
    }
    if !unsupported.is_empty() {
        crate::logging::warn("install", &format!("no 7-Zip here could test {} ({})", archive.display(), unsupported.join(", ")));
    }
    Ok(false)
}

fn valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.len() <= 160 && slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn client() -> Result<reqwest::Client, String> {
    crate::net::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

/// The file at `path`, checked, and what it is for: `slug` when the player
/// picked the game, else the game its name points to. Signed in or not:
/// everything it asks kryo.to for is public.
#[tauri::command]
pub async fn archive_look(app: AppHandle, path: String, slug: Option<String>) -> Result<Look, String> {
    let file = PathBuf::from(path.trim());
    if !file.is_absolute() {
        return Err("That file is not there any more.".into());
    }
    let checked = file.clone();
    let (file_name, size) = tauri::async_runtime::spawn_blocking(move || check_file(&checked)).await.map_err(|e| e.to_string())??;
    let mut look = Look { path: file.to_string_lossy().into_owned(), file_name: file_name.clone(), size, ..Default::default() };
    let client = client()?;
    let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(&app));
    let slug = match slug.map(|s| s.trim().to_ascii_lowercase()).filter(|s| valid_slug(s)) {
        Some(s) => Some(s),
        None => {
            let mut found = None;
            for name in guess_names(&file_name).into_iter().take(3) {
                if let Some(s) = downloads::find_slug(&client, &endpoint, &name).await {
                    found = Some(s);
                    break;
                }
            }
            found
        }
    };
    let Some(slug) = slug else { return Ok(look) };
    let (meta, listing) = tokio::join!(downloads::fetch_meta(&client, &endpoint, &slug), downloads::game_listing(&app, &client, &slug));
    if let Some(meta) = meta {
        look.title = Some(meta.title).filter(|t| !t.trim().is_empty());
        look.cover = meta.cover.or(meta.header);
        look.nsfw = meta.nsfw;
    }
    look.reachable = look.title.is_some() || listing.is_some();
    if let Some(json) = &listing {
        let guess = guess_release(json, &file_name, size);
        look.title = look.title.or_else(|| json["title"].as_str().map(String::from));
        look.version = guess.version;
        look.addon = guess.addon;
        look.size_match = guess.size_match;
    } else {
        look.addon = downloads::dlc_label(&file_name);
    }
    look.installed = crate::library::load(&app).is_ok_and(|games| games.iter().any(|g| g.slug.as_deref() == Some(slug.as_str()) && g.launcher.is_none()));
    look.slug = Some(slug);
    Ok(look)
}

/// Install the file at `path` as the game `slug`: queued in Downloads as a
/// finished download whose archive is this file. `anyway`: the player already
/// agreed to install it if it matches nothing on kryo.to (the dialog said so).
/// The row's id.
#[tauri::command]
pub async fn archive_install(app: AppHandle, path: String, slug: String, title: Option<String>, anyway: bool) -> Result<String, String> {
    let slug = slug.trim().to_ascii_lowercase();
    if !valid_slug(&slug) {
        return Err("Pick the game this file is for first.".into());
    }
    let file = PathBuf::from(path.trim());
    if !file.is_absolute() {
        return Err("That file is not there any more.".into());
    }
    let checked = file.clone();
    let (_, size) = tauri::async_runtime::spawn_blocking(move || check_file(&checked)).await.map_err(|e| e.to_string())??;
    let id = downloads::enqueue_file(&app, &file, size, slug.clone(), title, anyway)?;
    crate::logging::info("download", &format!("{id}: installing {} for {slug} from the player's own file", file.display()));
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryoto-from-file-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A tiny real 7z archive of plain data files (never a program).
    fn archive(dir: &Path, name: &str) -> PathBuf {
        let src = dir.join(format!("{name}-src"));
        std::fs::create_dir_all(src.join("Game/Data")).unwrap();
        let data: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        std::fs::write(src.join("Game/Data/level.bin"), &data).unwrap();
        std::fs::write(src.join("Game/readme.txt"), b"hello").unwrap();
        let out = dir.join(name);
        sevenz_rust::compress_to_path(&src, &out).unwrap();
        out
    }

    /// What an install from the player's file records, in the library and in
    /// the game folder's own record (`.kryoto-game.json`, which the uninstall
    /// and a later reinstall read): Kryoto's copy when its hash matched, "not
    /// verified" when it matched nothing, and the file is never the app's to delete.
    #[test]
    fn the_install_record_says_whether_it_is_kryotos_copy() {
        let dir = scratch("record");
        let game_dir = dir.join("Captain Hardcore");
        std::fs::create_dir_all(&game_dir).unwrap();
        let sha = "a".repeat(64);
        let mut row = downloads::Download {
            from_file: true,
            slug: Some("captain-hardcore".into()),
            archive_path: dir.join("Captain Hardcore - kryoto.7z").to_string_lossy().into_owned(),
            sha256: Some(sha.clone()),
            verified: true,
            ..Default::default()
        };
        row.meta.version = Some("1.1".into());
        assert_eq!(downloads::own_archive(&row), None, "the player's file is theirs");
        let matched = downloads::installed_entry(&row, "Captain Hardcore", &game_dir, "Game.exe".into());
        assert_eq!((matched.build_sha.as_deref(), matched.unverified, matched.version.as_deref()), (Some(sha.as_str()), false, Some("1.1")));
        assert_eq!(matched.pinned_version, None, "an older build matched by hash is still offered updates");

        row.verified = false;
        row.sha256 = None;
        row.not_verified = true;
        row.meta.version = version_in_name("Captain Hardcore v1.0 - kryoto.7z");
        let mut unmatched = downloads::installed_entry(&row, "Captain Hardcore", &game_dir, "Game.exe".into());
        assert_eq!((unmatched.build_sha.as_deref(), unmatched.unverified, unmatched.version.as_deref()), (None, true, Some("1.0")));
        unmatched.id = "captain-hardcore".into();
        crate::library::write_marker(&unmatched);
        let record = crate::library::read_marker(&game_dir).expect("the folder's record is written");
        assert!(record.unverified && record.build_sha.is_none(), "{record:?}");

        // A download is never marked, and its archive stays the app's to tidy.
        let download = downloads::Download { archive_path: "D:/x.7z".into(), sha256: Some(sha), verified: true, ..Default::default() };
        assert!(!downloads::installed_entry(&download, "X", &game_dir, String::new()).unverified);
        assert_eq!(downloads::own_archive(&download).as_deref(), Some("D:/x.7z"));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn listing(main: &str, old: &str, addon: &str) -> serde_json::Value {
        json!({
            "slug": "captain-hardcore", "title": "Captain Hardcore", "version": "1.2",
            "hashes": [{ "name": "Captain Hardcore - kryoto.7z", "sha256": main, "bytes": 1000 }],
            "releases": [
                { "id": "20", "version": "1.2", "primary": true, "hashes": [{ "name": "Captain Hardcore - kryoto.7z", "sha256": main, "bytes": 1000 }] },
                { "id": "19", "version": "1.1", "primary": false, "archived": true, "hashes": [{ "name": "Captain Hardcore - kryoto.7z", "sha256": old, "bytes": 900 }] }
            ],
            "addons": [
                { "id": "31", "label": "DLC: Firebird", "kind": "addon", "links": [], "hashes": [{ "name": "Captain Hardcore - DLC Firebird - kryoto.7z", "sha256": addon, "bytes": 77 }] },
                { "id": "32", "label": "Online fix", "kind": "repair", "links": [], "hashes": [{ "sha256": "f".repeat(64), "bytes": 55 }] }
            ]
        })
    }

    #[test]
    fn a_file_is_matched_by_its_hash_to_a_build_or_an_add_on() {
        let (main, old, addon) = ("a".repeat(64), "b".repeat(64), "c".repeat(64));
        let json = listing(&main, &old, &addon);
        assert_eq!(match_sha(&json, &main), Some(ShaMatch { version: Some("1.2".into()), ..Default::default() }), "the current build");
        assert_eq!(match_sha(&json, &old.to_ascii_uppercase()).unwrap().version.as_deref(), Some("1.1"), "an archived older build, in any case");
        assert_eq!(match_sha(&json, &addon), Some(ShaMatch { version: None, addon: Some("DLC: Firebird".into()), repair: false }));
        assert!(match_sha(&json, &"f".repeat(64)).unwrap().repair, "a repair fix stays one");
        assert_eq!(match_sha(&json, &"d".repeat(64)), None, "a re-pack or a changed copy matches nothing");
        assert_eq!(match_sha(&json, "abc"), None);
        // An old site that lists only the main release's hashes.
        assert_eq!(match_sha(&json!({ "version": "3", "hashes": [{ "sha256": main }] }), &main).unwrap().version.as_deref(), Some("3"));
    }

    #[test]
    fn the_size_points_to_a_build_before_the_file_is_read() {
        let json = listing(&"a".repeat(64), &"b".repeat(64), &"c".repeat(64));
        let g = guess_release(&json, "Captain Hardcore - kryoto.7z", 1000);
        assert_eq!((g.version.as_deref(), g.size_match), (Some("1.2"), Some(true)));
        assert_eq!(guess_release(&json, "whatever.7z", 900).version.as_deref(), Some("1.1"), "an older build by size, renamed by a host");
        let dlc = guess_release(&json, "Captain Hardcore - DLC Firebird - kryoto.7z", 77);
        assert_eq!((dlc.addon.as_deref(), dlc.size_match), (Some("DLC: Firebird"), Some(true)));
        // No size fits: said up front, and an add-on is still known by its name.
        let none = guess_release(&json, "Captain Hardcore - kryoto.7z", 5);
        assert_eq!((none.version, none.addon, none.size_match), (None, None, Some(false)));
        assert_eq!(guess_release(&json, "Captain Hardcore - DLC Firebird - kryoto.7z", 5).addon.as_deref(), Some("DLC: Firebird"));
        // A listing without sizes cannot say either way.
        let bare = json!({ "releases": [{ "version": "1", "primary": true, "hashes": [{ "sha256": "a".repeat(64) }] }] });
        assert_eq!(guess_release(&bare, "x.7z", 5).size_match, None);
    }

    #[test]
    fn names_and_versions_come_from_the_file_name() {
        assert_eq!(guess_names("Captain Hardcore - kryoto.7z"), vec!["Captain Hardcore"]);
        assert_eq!(guess_names("Forts - DLC Firebird - kryoto.7z")[0], "Forts");
        assert!(guess_names("Hollow.Knight.v1.5.78.7z").contains(&"Hollow Knight".to_string()));
        assert!(guess_names("Stardew Valley - Language Polish - kryoto.7z").contains(&"Stardew Valley".to_string()));
        assert!(guess_names("download.7z").is_empty());
        assert_eq!(version_in_name("Hollow Knight v1.5.78 - kryoto.7z").as_deref(), Some("1.5.78"));
        assert_eq!(version_in_name("Game (Build 12345).7z").as_deref(), Some("12345"));
        assert_eq!(version_in_name("Game v2.7z").as_deref(), Some("2"));
        assert_eq!(version_in_name("Captain Hardcore - kryoto.7z"), None);
        assert_eq!(version_in_name("Portal 2 - kryoto.7z"), None, "a number in the title is not a build");
    }

    #[test]
    fn a_whole_archive_passes_and_anything_else_is_refused_before_anything_is_written() {
        let dir = scratch("check");
        let good = archive(&dir, "Captain Hardcore - kryoto.7z");
        let (name, size) = check_file(&good).unwrap();
        assert_eq!((name.as_str(), size), ("Captain Hardcore - kryoto.7z", std::fs::metadata(&good).unwrap().len()));

        let page = dir.join("Game - kryoto.7z");
        std::fs::write(&page, b"<!doctype html><html><title>Just a moment...</title></html>").unwrap();
        let e = check_file(&page).unwrap_err();
        assert!(e.contains("not a 7z archive") && e.contains("a web page"), "{e}");

        let bytes = std::fs::read(&good).unwrap();
        let cut = dir.join("cut.7z");
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        assert!(check_file(&cut).unwrap_err().contains("cut short"));

        // A byte flipped in the end header: its own CRC refuses it.
        let mut bad = bytes.clone();
        let last = bad.len() - 3;
        bad[last] ^= 0xff;
        let damaged = dir.join("damaged.7z");
        std::fs::write(&damaged, &bad).unwrap();
        assert!(check_file(&damaged).unwrap_err().contains("is damaged"));

        let zip = dir.join("game.zip");
        std::fs::write(&zip, b"PK\x03\x04rest").unwrap();
        assert!(check_file(&zip).unwrap_err().contains("a .zip file"));
        let part = dir.join("Game.7z.001");
        std::fs::write(&part, &bytes).unwrap();
        assert!(check_file(&part).unwrap_err().contains("split archive"));
        // Plain text under a program's name: refused by its type, never run.
        let program = dir.join("Game.AppImage");
        std::fs::write(&program, b"not a program").unwrap();
        assert!(check_file(&program).unwrap_err().contains("is a program"));
        assert!(check_file(&dir).unwrap_err().contains("folder"));
        assert!(check_file(&dir.join("gone.7z")).unwrap_err().contains("not there"));
        // Nothing was written beside them: the archive's source folder and seven files.
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 8, "{names:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 7-Zip's own test, where a 7-Zip is installed: a whole archive passes, a
    /// byte flipped in its packed data does not. Skipped without one.
    #[test]
    fn seven_zip_tests_the_data_itself() {
        let dir = scratch("test");
        let good = archive(&dir, "Game - kryoto.7z");
        let stop = AtomicU8::new(downloads::RUN);
        let seen = std::cell::Cell::new(0u64);
        match test_archive(&good, &|done, _| seen.set(done), &stop) {
            Ok(false) => {
                eprintln!("no 7-Zip here; skipped");
                return;
            }
            Ok(true) => assert_eq!(seen.get(), std::fs::metadata(&good).unwrap().len()),
            Err(e) => panic!("a whole archive failed: {e}"),
        }
        let mut bytes = std::fs::read(&good).unwrap();
        let mid = bytes.len() / 3;
        bytes[mid] ^= 0xff;
        let broken = dir.join("broken.7z");
        std::fs::write(&broken, bytes).unwrap();
        assert!(check_file(&broken).is_ok(), "the headers are whole");
        assert!(test_archive(&broken, &|_, _| {}, &stop).is_err(), "the data is not");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn split_parts_are_told_apart() {
        assert!(is_split_part("game.7z.001"));
        assert!(is_split_part("game.part1.rar"));
        assert!(is_split_part("game.z01"));
        assert!(!is_split_part("game.7z"));
        assert!(!is_split_part("portal 2 - kryoto.7z"));
    }
}
