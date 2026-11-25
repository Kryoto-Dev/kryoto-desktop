//! Properties > Emulator & fixes > Online fix: Online-Fix's own fix for a
//! game, got by the player from online-fix.me (suggestion #91).
//!
//! Online-Fix's generic fix lets a game play online, through Steam, with
//! others who use the same fix. It is not Kryoto Online. This is the player's
//! way to it on a game kryo.to does not ship one for; kryo.to's own packed
//! copy, when staff published one, is the "From kryo.to" row (KryoFix.tsx,
//! addons.rs `install_fix`), and both go in the same way.
//!
//! How it is fetched, and why that way: online-fix.me's search, game pages
//! and folder listings are public, so Kryoto finds the game's `Fix Repair/`
//! file itself (`kryoto_repair::fixsite`). The file is only handed to a real
//! browser window that opened its folder (403 to anything else, 401 even with
//! an account's cookies), so Kryoto never downloads it by itself: it opens
//! that folder in a window of its own and the player clicks the file, exactly
//! as on the site, with the site's checks untouched. The window keeps only
//! that one file (resolvers/browser.rs `save_in_window`). A player who got the
//! archive some other way points Kryoto at it ("I have the file").
//!
//! What Kryoto does with it is the same as for Forge's fix-only package:
//! 7-Zip tests the whole archive (password `online-fix.me`) and lists it, the
//! list must be the generic fix and nothing else (`fixpack::scope`: Online-
//! Fix's layer and ini, no unsafe paths or links, not a game), its SHA-256 must
//! match the one Pixeldrain publishes for the same file when the site's
//! Hosters page lists one, what unpacks must be exactly what was listed, and
//! its `OnlineFix.ini` must name this game's Steam app id and fit its folders
//! and architecture (`fixpack::plan`). Only then does anything in the game
//! change: what Kryoto put in before comes out, the release's own emulator
//! by role (repair.rs `clear_for_switch`, as an emulator switch does), and
//! the fix goes in under the repair journal, which keeps a verified copy of
//! every file it replaces. So "Back to the release's own files" (repair.rs
//! `repair_undo`) takes it out in one press, the Files check never reports
//! its files as damaged (release_files.rs `kryoto_changes`), and on Linux
//! the game's source label names Online-Fix so its Wine settings apply
//! (launch.rs `wine_overrides_for`).

use crate::library::{self, LibraryGame};
use kryoto_repair::{fixpack, fixsite, journal};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// What online-fix.me has for a game.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Lookup {
    /// `found`, `none` (no fix for this game), `unreachable`, `signin`.
    pub status: &'static str,
    /// The game's page on online-fix.me.
    pub page: Option<String>,
    /// The fix's file name and size, as the site lists them.
    pub file: Option<String>,
    pub size: Option<String>,
    #[serde(skip)]
    pub file_url: Option<String>,
    /// The game version online-fix.me's own release is, which the fix is made for.
    pub made_for: Option<String>,
    /// That version is newer than the one installed.
    pub newer_needed: bool,
    /// Why it could not be read, for `unreachable`. For `none`, why there is
    /// none when the folder says (only an Xbox app fix, `fixsite::GDK_ONLY`).
    pub detail: Option<String>,
}

/// Everything the Online fix row draws.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineFixState {
    /// Why there is no online fix to offer here at all (another launcher's
    /// game, not linked, no Steam file).
    pub blocked: Option<String>,
    /// The release itself already carries an online fix.
    pub carried: bool,
    /// Kryoto put Online-Fix's fix in (Back to the release's own files takes it out).
    pub applied: bool,
    /// A full 7-Zip is on this PC: Online-Fix's locked RAR needs one.
    pub seven_zip: bool,
    /// The version installed, as kryo.to named it.
    pub installed: Option<String>,
    pub lookup: Option<Lookup>,
}

/// Lookups for this run, so reopening Properties asks online-fix.me once.
static LOOKUPS: Mutex<Option<HashMap<String, (Instant, Lookup)>>> = Mutex::new(None);
const KEEP_FOUND: Duration = Duration::from_secs(6 * 60 * 60);
const KEEP_FAILED: Duration = Duration::from_secs(60);

fn cached(id: &str) -> Option<Lookup> {
    let all = LOOKUPS.lock().ok()?;
    let (at, l) = all.as_ref()?.get(id)?;
    let keep = if l.status == "unreachable" { KEEP_FAILED } else { KEEP_FOUND };
    (at.elapsed() < keep).then(|| l.clone())
}

fn remember(id: &str, l: &Lookup) {
    if let Ok(mut all) = LOOKUPS.lock() {
        all.get_or_insert_with(HashMap::new).insert(id.to_string(), (Instant::now(), l.clone()));
    }
}

/// online-fix.me's words, or an online fix by its files: what a release
/// already carries. Kryoto Online is not an online fix of Online-Fix's.
pub fn carried_by(labels: &[&str], fix_files: &[String], on_disk: bool) -> bool {
    let says = |s: &str| {
        let s = s.to_ascii_lowercase();
        s.contains("online-fix") || s.contains("onlinefix") || s.contains("online fix") || s.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "ofme")
    };
    on_disk
        || labels.iter().any(|l| says(l))
        || fix_files.iter().any(|f| {
            let n = f.rsplit(['/', '\\']).next().unwrap_or(f).to_ascii_lowercase();
            n == "onlinefix64.dll" || n == "onlinefix.dll" || n == "onlinefix.ini"
        })
}

/// Whether online-fix.me's version is newer than the installed one, when
/// both can be put in order.
pub fn newer_needed(made_for: Option<&str>, installed: Option<&str>) -> bool {
    match (made_for, installed) {
        (Some(m), Some(i)) => fixsite::compare_versions(m, i) == Some(std::cmp::Ordering::Greater),
        _ => false,
    }
}

fn state_of(app: &AppHandle, game: &LibraryGame) -> OnlineFixState {
    let blocked = crate::repair::game_emulators(app.clone(), game.id.clone()).ok().and_then(|e| e.blocked);
    let applied = game.repair.as_ref().is_some_and(|r| r.source == fixpack::SOURCE_LABEL);
    let root = PathBuf::from(&game.install_dir);
    let exe = crate::launch::inside(&root, &game.executable).unwrap_or_else(|_| root.clone());
    let fix_files: Vec<String> = crate::release_files::cached_list(app, game)
        .map(|l| l.listed().iter().filter(|f| f.role == crate::release_files::Role::Fix).map(|f| f.path.clone()).collect())
        .unwrap_or_default();
    // What the release came with, before Kryoto changed its label.
    let label = if applied { game.repair.as_ref().and_then(|r| r.previous_source.clone()) } else { game.source.clone() };
    let carried = !applied && carried_by(&[label.as_deref().unwrap_or("")], &fix_files, crate::launch::wine_overrides_from_files(&root, &exe).is_some());
    OnlineFixState {
        blocked,
        carried,
        applied,
        seven_zip: seven_zip().is_some(),
        installed: game.version.clone(),
        lookup: cached(&game.id),
    }
}

/// What is known without asking online-fix.me.
#[tauri::command(async)]
pub fn online_fix_state(app: AppHandle, game_id: String) -> Result<OnlineFixState, String> {
    Ok(state_of(&app, &crate::repair::game(&app, &game_id)?))
}

fn client() -> Result<reqwest::Client, String> {
    crate::net::builder().user_agent(crate::resolvers::UA).timeout(Duration::from_secs(25)).build().map_err(|e| e.to_string())
}

fn unreachable(detail: String) -> Lookup {
    Lookup { status: "unreachable", detail: Some(detail), ..Default::default() }
}

/// One page of the site, or why not. A Cloudflare check or an error page
/// reads as the site being unreachable from here, not as "no fix".
async fn page(client: &reqwest::Client, url: &str, referer: Option<&str>) -> Result<(u16, String), String> {
    let mut req = client.get(url);
    if let Some(r) = referer {
        req = req.header(reqwest::header::REFERER, r);
    }
    let res = req.send().await.map_err(|e| crate::logging::why(&e))?;
    let status = res.status().as_u16();
    let body = fixsite::decode_page(&res.bytes().await.map_err(|e| crate::logging::why(&e))?);
    if status == 403 || status == 429 || status >= 500 || body.contains("cf-chl") || body.contains("Just a moment...") {
        return Err(format!("online-fix.me answered {status}"));
    }
    Ok((status, body))
}

/// The game's title as online-fix.me writes it. The site is Latin only:
/// "Call of Duty®: Black Ops III" finds nothing with its ®, and "Rise of the
/// Rōnin" is found as "Rise of the Ronin", never as "Rise of the Rnin". A
/// letter with no plain spelling splits the word instead of joining its two
/// halves: the search finds "Rise of the R nin" but not "Rise of the Rnin".
fn site_title(title: &str) -> String {
    let plain = crate::downloads::latin_with(title, |c| if c.is_alphabetic() { " " } else { "" });
    plain.chars().filter(|c| !c.is_ascii_control()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Ask online-fix.me what it has for this game.
async fn look_up(title: &str, slug: Option<&str>, installed: Option<&str>) -> Lookup {
    let Ok(client) = client() else { return unreachable("no network client".into()) };
    let title = site_title(title);
    let search = format!("{}/index.php?do=search&subaction=search&story={}", fixsite::SITE, url::form_urlencoded::byte_serialize(title.as_bytes()).collect::<String>());
    let html = match page(&client, &search, None).await {
        Ok((_, h)) => h,
        Err(e) => return unreachable(e),
    };
    let pages = fixsite::parse_search(&html);
    let Some(found) = fixsite::pick_page(&title, slug, &pages) else { return Lookup { status: "none", ..Default::default() } };
    let page_url = found.url.clone();
    let html = match page(&client, &page_url, None).await {
        Ok((_, h)) => h,
        Err(e) => return unreachable(e),
    };
    if fixsite::wants_signin(&html) {
        return Lookup { status: "signin", page: Some(page_url), ..Default::default() };
    }
    let Some(base) = fixsite::parse_uploads_link(&html) else { return Lookup { status: "none", page: Some(page_url), ..Default::default() } };
    // The full release's name says the game version the fix is made for.
    let made_for = match page(&client, &base, Some(&page_url)).await {
        Ok((200, listing)) => fixsite::release_version(&fixsite::parse_listing(&listing, &base).into_iter().map(|f| f.name).collect::<Vec<_>>()),
        _ => None,
    };
    let folder = format!("{base}Fix%20Repair/");
    let fixes = match page(&client, &folder, Some(&page_url)).await {
        Ok((404, _)) => Vec::new(),
        Ok((200, listing)) => fixsite::parse_listing(&listing, &folder),
        Ok((code, _)) => return unreachable(format!("online-fix.me answered {code}")),
        Err(e) => return unreachable(e),
    };
    let Some(fix) = fixsite::generic_fix(&fixes) else {
        let detail = fixsite::only_gdk(&fixes).then(|| fixsite::GDK_ONLY.to_string());
        return Lookup { status: "none", page: Some(page_url), made_for, detail, ..Default::default() };
    };
    Lookup {
        status: "found",
        newer_needed: newer_needed(made_for.as_deref(), installed),
        page: Some(page_url),
        file: Some(fix.name.clone()),
        size: Some(fix.size.clone()),
        file_url: Some(fix.url.clone()),
        made_for,
        detail: None,
    }
}

/// What online-fix.me has for this game, asked now (or from this run's answer).
#[tauri::command]
pub async fn online_fix_lookup(app: AppHandle, game_id: String, fresh: bool) -> Result<OnlineFixState, String> {
    let game = crate::repair::game(&app, &game_id)?;
    let mut state = state_of(&app, &game);
    if state.blocked.is_some() {
        return Ok(state);
    }
    if fresh || state.lookup.is_none() {
        let l = look_up(&game.title, game.slug.as_deref(), game.version.as_deref()).await;
        remember(&game_id, &l);
        state.lookup = Some(l);
    }
    Ok(state)
}

/// Open the game's page on online-fix.me in the player's browser (to sign
/// in there, or to download the fix by hand for "I have the file").
#[tauri::command]
pub fn online_fix_open_page(game_id: String) -> Result<(), String> {
    let page = cached(&game_id).and_then(|l| l.page).unwrap_or_else(|| fixsite::SITE.to_string());
    crate::open_external(page)
}

#[cfg(windows)]
const NEEDS_7ZIP: &str = "Online-Fix locks its fix in a RAR archive with a password, which only 7-Zip opens. Install 7-Zip from 7-zip.org, then try again.";
#[cfg(not(windows))]
const NEEDS_7ZIP: &str = "Online-Fix locks its fix in a RAR archive with a password, which only 7-Zip opens. Install 7-Zip (the `7zip` package, command `7zz`), then try again.";

/// A 7-Zip that reads RAR (not the `7zr.exe` Kryoto keeps, which reads only
/// .7z). Kept once found; looked for again while there is none, so one
/// installed while the app runs is picked up.
fn seven_zip() -> Option<PathBuf> {
    static FOUND: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(p) = FOUND.get() {
        return Some(p.clone());
    }
    let found = crate::downloads::seven_zip_tools(false).into_iter().find(|t| run_7z(t, &["i".into()]).is_some())?;
    Some(FOUND.get_or_init(|| found).clone())
}

/// Run 7-Zip, quietly. None when that 7-Zip is not there.
fn run_7z(tool: &Path, args: &[std::ffi::OsString]) -> Option<(bool, String)> {
    let mut cmd = std::process::Command::new(tool);
    cmd.args(args).stdin(std::process::Stdio::null());
    crate::launch::clean_appimage_env(&mut cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    Some((out.status.success(), format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))))
}

/// 7-Zip's complaint, in a player's words.
pub fn archive_trouble(text: &str) -> String {
    let l = text.to_ascii_lowercase();
    if l.contains("wrong password") || l.contains("can not open encrypted archive") {
        "The archive is locked with a password other than online-fix.me's, so it is not Online-Fix's fix. Get it from the game's page on online-fix.me.".into()
    } else if l.contains("unexpected end") || l.contains("data error") || l.contains("crc failed") || l.contains("headers error") {
        "The archive is incomplete or damaged. Download it again.".into()
    } else if l.contains("open the file as archive") || l.contains("is not archive") {
        "That file is not an archive. Online-Fix's fix is a .rar file.".into()
    } else {
        "7-Zip could not read the archive. Download it again.".into()
    }
}

/// Test, list and unpack the archive into `staging`: whole, opened by
/// online-fix.me's password, the generic fix and nothing else, and exactly
/// what it listed. Returns the fix's folder and what it holds.
fn unpack(seven: &Path, archive: &Path, staging: &Path) -> Result<(PathBuf, fixpack::Scope), String> {
    let mut last = String::new();
    for pw in fixpack::ARCHIVE_PASSWORDS {
        let pass: std::ffi::OsString = format!("-p{pw}").into();
        // `t` reads and checks every file, writing nothing.
        let (ok, text) = run_7z(seven, &["t".into(), pass.clone(), "-y".into(), archive.into()]).ok_or(NEEDS_7ZIP)?;
        if !ok {
            last = text;
            continue;
        }
        let (ok, text) = run_7z(seven, &["l".into(), "-slt".into(), pass.clone(), archive.into()]).ok_or(NEEDS_7ZIP)?;
        if !ok {
            return Err(archive_trouble(&text));
        }
        let scope = fixpack::scope(&fixpack::parse_slt(&text))?;
        let _ = std::fs::remove_dir_all(staging);
        std::fs::create_dir_all(staging).map_err(|e| e.to_string())?;
        // -snl-: never make links, whatever the archive says.
        let (ok, text) = run_7z(seven, &["x".into(), pass, format!("-o{}", staging.display()).into(), "-y".into(), "-snl-".into(), archive.into()]).ok_or(NEEDS_7ZIP)?;
        if !ok {
            return Err(archive_trouble(&text));
        }
        let root = if scope.root.is_empty() { staging.to_path_buf() } else { staging.join(&scope.root) };
        let on_disk = fixpack::scope_of_dir(&root)?;
        if on_disk.files != scope.files {
            return Err("What unpacked is not what the archive listed, so it was not used.".into());
        }
        return Ok((root, scope));
    }
    Err(archive_trouble(&last))
}

/// The hash Pixeldrain publishes for the same file, when online-fix.me's
/// Hosters page lists a copy there. None when there is none (or no answer).
async fn published_sha256(page_url: &str, name: &str) -> Option<String> {
    let client = client().ok()?;
    let (_, html) = page(&client, page_url, None).await.ok()?;
    let hosters = fixsite::parse_hosters_link(&html)?;
    let (_, listing) = page(&client, &hosters, Some(page_url)).await.ok()?;
    let id = fixsite::pixeldrain_copy(&fixsite::parse_hosters(&listing), name)?.to_string();
    let info: serde_json::Value = client.get(format!("https://pixeldrain.com/api/file/{id}/info")).send().await.ok()?.json().await.ok()?;
    info["hash_sha256"].as_str().filter(|h| h.len() == 64).map(str::to_ascii_lowercase)
}

/// The fix put into the game. Plain words on every refusal; nothing in the
/// game changes until the archive and the fix have passed every check.
async fn install(app: &AppHandle, game_id: &str, archive: &Path, published: Option<String>) -> Result<OnlineFixState, String> {
    let say = |text: &str| crate::repair::progress(app, game_id, text, None);
    let game = crate::repair::game(app, game_id)?;
    if library::is_running(app, game_id) {
        return Err("Close the game before putting the online fix in.".into());
    }
    if let Some(why) = state_of(app, &game).blocked {
        return Err(why);
    }
    // By its name, before anything is read: "I have the file" takes any
    // archive the player picks.
    if archive.file_name().is_some_and(|n| fixsite::is_gdk(&n.to_string_lossy())) {
        return Err(fixsite::GDK_REFUSED.into());
    }
    let root = crate::repair::root(app, &game)?;
    let appid = crate::achievements::appid_for(app, &game)
        .await
        .ok_or("Kryoto does not know this game's Steam app id, so it cannot check that the fix is for it. Link it to its kryo.to page in the kryo.to tab first.")?;
    let seven = seven_zip().ok_or(NEEDS_7ZIP)?;
    say("Checking the archive");
    let sha = {
        let a = archive.to_path_buf();
        tauri::async_runtime::spawn_blocking(move || journal::digest(&a)).await.map_err(|e| e.to_string())??
    };
    if let Some(want) = &published {
        if want != &sha {
            return Err("This file is not the one online-fix.me publishes (its SHA-256 is different), so it was not used. Download it again.".into());
        }
    }
    let staging = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("online-fix").join(format!("unpack-{}", crate::downloads::safe_name(game_id)));
    let unpacked = {
        let (s, a, st) = (seven.clone(), archive.to_path_buf(), staging.clone());
        tauri::async_runtime::spawn_blocking(move || unpack(&s, &a, &st)).await.map_err(|e| e.to_string())?
    };
    let result = async {
        let (fix_root, scope) = unpacked?;
        say("Checking that it is this game's fix");
        // Before anything in the game changes: its app id, folders and architecture.
        fixpack::plan(&fix_root, &scope, &root, &appid)?;
        let before = game.clone();
        let cleaned = crate::repair::clear_for_switch(app, game_id, &before).await?;
        let before = crate::repair::game(app, game_id)?;
        let back_out = |error: String| -> String {
            match cleaned.map(|_| crate::release_files::unclean(&root)) {
                Some(Err(undo)) => format!("{error} Putting the release's own emulator back failed: {undo}"),
                _ => error,
            }
        };
        say("Putting the online fix in");
        let plan = fixpack::plan(&fix_root, &scope, &root, &appid).map_err(back_out)?;
        fixpack::apply(&fix_root, &root, &plan, &sha[..12]).map_err(back_out)?;
        let file = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let saved = library::update(app, |games| {
            let g = games.iter_mut().find(|g| g.id == game_id).ok_or("The library game disappeared.")?;
            g.repair = Some(crate::repair::LocalRepair {
                source: fixpack::SOURCE_LABEL.into(),
                version: file.clone(),
                previous_online: before.online.clone(),
                previous_source: before.source.clone(),
                previous_overrides: before.apply_overrides,
                achievements: false,
            });
            // Named, so Linux starts it with Online-Fix's Wine settings.
            g.source = Some(format!("Steam + {}", fixpack::SOURCE_LABEL));
            g.online = None;
            g.apply_overrides = true;
            Ok(())
        });
        if let Err(error) = saved {
            let _ = journal::undo(&root);
            return Err(back_out(format!("Saving the library failed: {error}. The game's files were put back.")));
        }
        crate::logging::info("online-fix", &format!("{}: Online-Fix's fix in ({file}, SHA-256 {}, {} files)", game.title, &sha[..16], plan.placements.len()));
        Ok(())
    }
    .await;
    let _ = std::fs::remove_dir_all(&staging);
    if let Err(e) = &result {
        say(e);
        crate::logging::warn("online-fix", &format!("{}: {e}", game.title));
    }
    result?;
    say("Done. Start the game with Steam running");
    let _ = app.emit("library-changed", ());
    Ok(state_of(app, &crate::repair::game(app, game_id)?))
}

/// Get the online fix: online-fix.me's folder opens in a window, the player
/// clicks the file, and Kryoto checks it and puts it in.
#[tauri::command]
pub async fn online_fix_get(app: AppHandle, game_id: String) -> Result<OnlineFixState, String> {
    crate::required::refuse()?;
    let game = crate::repair::game(&app, &game_id)?;
    let lookup = match cached(&game_id) {
        Some(l) => l,
        None => {
            let l = look_up(&game.title, game.slug.as_deref(), game.version.as_deref()).await;
            remember(&game_id, &l);
            l
        }
    };
    let (Some(file_url), Some(name), Some(page_url)) = (lookup.file_url.clone(), lookup.file.clone(), lookup.page.clone()) else {
        return Err(match lookup.status {
            "unreachable" => "online-fix.me did not answer. It may be down or blocked where you are. Try again later, or download the fix in your browser and use I have the file.".into(),
            "signin" => "online-fix.me wants you to sign in before it shows this game's files. Open its page, sign in there, then try again.".into(),
            _ => lookup.detail.unwrap_or_else(|| "online-fix.me has no online fix for this game.".into()),
        });
    };
    if library::is_running(&app, &game_id) {
        return Err("Close the game before putting the online fix in.".into());
    }
    // Before the window: no point downloading what cannot be opened here.
    if seven_zip().is_none() {
        return Err(NEEDS_7ZIP.into());
    }
    let file: url::Url = file_url.parse().map_err(|_| "online-fix.me listed a link Kryoto cannot read.".to_string())?;
    if !fixsite::acceptable(&file) {
        return Err("online-fix.me listed a file that is not on its own servers, so Kryoto did not open it.".into());
    }
    let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?.join("online-fix").join(crate::downloads::safe_name(&game_id));
    crate::repair::progress(&app, &game_id, &format!("online-fix.me is open in a window: click {name} there"), None);
    let _ = app.emit(
        "notify",
        crate::downloads::Notice::new("Click the online fix in the window", &format!("online-fix.me is open on {}'s fix. Click {name} to download it; Kryoto does the rest.", game.title), Some(game_id.clone())),
    );
    let title = format!("Kryoto: click {name} to download {}'s online fix", game.title);
    let saved = crate::resolvers::save_in_window(&app, &fixsite::folder_of(&file), &file, &dir, &title)
        .await
        .map_err(|e| format!("The online fix was not downloaded: {e}."))?;
    crate::repair::progress(&app, &game_id, "Downloaded. Checking it", None);
    let published = published_sha256(&page_url, &name).await;
    let result = install(&app, &game_id, &saved, published).await;
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// "I have the file": an archive the player downloaded themselves, checked
/// and put in the same way. Their file is left where it is.
#[tauri::command]
pub async fn online_fix_from_file(app: AppHandle, game_id: String, path: String) -> Result<OnlineFixState, String> {
    crate::required::refuse()?;
    let archive = PathBuf::from(&path);
    if !archive.is_file() {
        return Err("That file is not there any more.".into());
    }
    let ext = archive.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if !["rar", "zip", "7z"].contains(&ext.as_str()) {
        return Err("Pick the online fix's archive (a .rar file from online-fix.me).".into());
    }
    let published = match (cached(&game_id).and_then(|l| l.page), archive.file_name()) {
        (Some(page_url), Some(name)) => published_sha256(&page_url, &name.to_string_lossy()).await,
        _ => None,
    };
    install(&app, &game_id, &archive, published).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_that_already_has_an_online_fix_is_told_apart_from_kryoto_online() {
        assert!(carried_by(&["OFME"], &[], false));
        assert!(carried_by(&["Steam + online-fix"], &[], false));
        assert!(carried_by(&["RUNE + optional online-fix"], &[], false));
        assert!(carried_by(&["Steam + gbe_fork"], &["Game/OnlineFix64.dll".into()], false), "by the release's own fix files");
        assert!(carried_by(&[""], &[], true), "by its files on disk");
        assert!(!carried_by(&["Steam + Kryoto Online"], &[], false));
        assert!(!carried_by(&["Steam + gbe_fork"], &["Game/Steamless.exe".into()], false));
        assert!(!carried_by(&["Software"], &[], false), "ofme only as a word");
    }

    #[test]
    fn an_accented_title_is_looked_up_in_plain_letters() {
        assert_eq!(site_title("Rise of the Rōnin"), "Rise of the Ronin");
        assert_eq!(site_title("Brütal Legend"), "Brutal Legend");
        assert_eq!(site_title("Call of Duty®: Black Ops III "), "Call of Duty: Black Ops III");
        assert_eq!(site_title("Ștefan și Țara"), "Stefan si Tara");
        // A decomposed accent is left out; its letter stays.
        assert_eq!(site_title("Rise of the Ro\u{304}nin"), "Rise of the Ronin");
        // A letter with no plain spelling splits the word; other scripts go.
        assert_eq!(site_title("Ti\u{1EBF}ng Vi\u{1EC7}t"), "Ti ng Vi t");
        assert_eq!(site_title("Ōkami 大神"), "Okami");
        // Its page is found by title, though its slug ends in -po-seti, not -online.
        let pages = vec![fixsite::Page { title: "Rise of the Ronin   ".into(), url: "https://online-fix.me/games/rpg/17719-rise-of-the-ronin-po-seti.html".into() }];
        assert!(fixsite::pick_page(&site_title("Rise of the Rōnin"), None, &pages).is_some());
    }

    #[test]
    fn a_fix_for_a_newer_game_version_is_said_only_when_it_is() {
        assert!(newer_needed(Some("1.0.11"), Some("1.0.9")));
        assert!(!newer_needed(Some("1.0.11"), Some("1.0.11")));
        assert!(!newer_needed(Some("1.0.11"), Some("b26004539")), "a build number cannot be compared");
        assert!(!newer_needed(None, Some("1.0")));
    }

    #[test]
    fn seven_zips_complaints_are_said_plainly() {
        assert!(archive_trouble("ERROR: Wrong password : OnlineFix64.dll").contains("not Online-Fix's fix"));
        assert!(archive_trouble("ERROR: Unexpected end of archive").contains("Download it again"));
        assert!(archive_trouble("Can not open the file as archive").contains("not an archive"));
    }

    /// The fix in a synthetic game the way `install` puts it in (fixpack under
    /// the journal): the Files check leaves every file it wrote alone, and
    /// Back to the release's own files (journal undo) puts the game back.
    #[test]
    fn the_files_check_leaves_the_fix_alone_and_back_takes_it_out() {
        let d = std::env::temp_dir().join(format!("kryoto-online-fix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (fix, game) = (d.join("fix"), d.join("game"));
        let write = |p: PathBuf, b: &[u8]| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b).unwrap();
        };
        write(fix.join("OnlineFix64.dll"), b"layer");
        write(fix.join("OnlineFix.ini"), b"[Main]\nRealAppId=4954910\nFakeAppId=480\n");
        write(fix.join("winmm.dll"), b"loader");
        write(fix.join("dlllist.txt"), b"OnlineFix64.dll\n");
        write(fix.join("steam_api64.dll"), b"valve");
        write(game.join("Game.exe"), b"exe");
        write(game.join("steam_api64.dll"), b"gbe");
        write(game.join("save.dat"), b"mine");
        let scope = fixpack::scope_of_dir(&fix).unwrap();
        let root = game.canonicalize().unwrap();
        let plan = fixpack::plan(&fix, &scope, &root, "4954910").unwrap();
        fixpack::apply(&fix, &root, &plan, "test").unwrap();
        assert_eq!(std::fs::read(root.join("steam_api64.dll")).unwrap(), b"valve");

        let (skip, switched) = crate::release_files::kryoto_changes(&LibraryGame::default(), &root);
        for f in ["onlinefix64.dll", "onlinefix.ini", "winmm.dll", "dlllist.txt", "steam_api64.dll"] {
            assert!(skip.contains(f), "{f} is Kryoto's change, never reported as damaged");
        }
        assert!(switched, "the release's emulator files are not expected while it is in");
        assert!(!skip.contains("save.dat"));
        // The launch settings on Linux follow the label Kryoto gives the game.
        assert!(crate::launch::wine_overrides_for(Some(&format!("Steam + {}", fixpack::SOURCE_LABEL))).unwrap().contains("OnlineFix64=n"));

        journal::undo(&root).unwrap();
        assert_eq!(std::fs::read(root.join("steam_api64.dll")).unwrap(), b"gbe");
        assert!(!root.join("OnlineFix64.dll").exists() && !root.join("winmm.dll").exists());
        assert_eq!(std::fs::read(root.join("save.dat")).unwrap(), b"mine");
        let _ = std::fs::remove_dir_all(d);
    }
}
