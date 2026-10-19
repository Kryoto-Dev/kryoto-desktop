//! Achievements in Kryoto Desktop: the list a game lacks, given before it
//! starts; unlocks noticed while it runs; and the list with your progress for
//! the game page. The work is in `kryoto_repair::achievements`, shared with
//! Forge - this is the app's side of it.
//!
//! An installed gbe_fork build from before October 2026 has no achievement
//! list, so the emulator threw every unlock away. `before_launch` writes one
//! into it the first time it is played, which is what gives the whole existing
//! catalogue achievements without a rebuild.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use kryoto_repair::achievements::{self as ach, Achievement, Roots, Unlock};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::library::{LibraryGame, Running};

const POLL: Duration = Duration::from_secs(2);
/// How long a cached list is trusted before Steam is asked again.
const SCHEMA_FRESH: Duration = Duration::from_secs(7 * 24 * 3600);

fn client() -> reqwest::Client {
    reqwest::Client::builder().user_agent("Kryoto Desktop").build().unwrap_or_default()
}

/// The Steam AppID of an installed game: what the emulator was told, or what
/// Forge recorded on the release.
pub fn appid_of(install_dir: &Path) -> Option<String> {
    let digits = |s: String| {
        let s = s.trim().trim_start_matches('\u{feff}').to_string();
        (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) && s != "480").then_some(s)
    };
    for entry in walkdir::WalkDir::new(install_dir).max_depth(6).into_iter().filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name == "steam_appid.txt" {
            if let Some(id) = std::fs::read_to_string(entry.path()).ok().and_then(digits) {
                return Some(id);
            }
        } else if name == ".kryoto-release.json" {
            let v: Option<serde_json::Value> = std::fs::read(entry.path()).ok().and_then(|b| serde_json::from_slice(&b).ok());
            let id = v.and_then(|v| v["steam_appid"].as_u64().map(|n| n.to_string()).or_else(|| v["steam_appid"].as_str().map(str::to_string)));
            if let Some(id) = id.and_then(digits) {
                return Some(id);
            }
        }
    }
    None
}

/// Where this game's emulator saves: the PC's own folders, or the Wine/Proton
/// prefix it runs in.
pub fn roots_for(env: &[(String, String)]) -> Roots {
    let prefix = env.iter().find_map(|(k, v)| match k.as_str() {
        "WINEPREFIX" => Some(PathBuf::from(v)),
        "STEAM_COMPAT_DATA_PATH" => Some(PathBuf::from(v).join("pfx")),
        _ => None,
    });
    let Some(prefix) = prefix else { return Roots::from_env() };
    let users = prefix.join("drive_c").join("users");
    // Proton's user is steamuser; plain Wine uses the login name.
    let user = std::fs::read_dir(&users)
        .ok()
        .and_then(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.join("AppData").join("Roaming").is_dir() && !p.ends_with("Public"))
        });
    Roots {
        app_data: user.map(|u| u.join("AppData").join("Roaming")),
        public_documents: Some(users.join("Public").join("Documents")),
    }
}

/// Before the game starts: give its gbe_fork folders the list they lack.
/// Bounded, because it holds up Play: the list and the stats setting are
/// written here (one quick question to Steam), and the icons are fetched in
/// the background while the game runs. A slow Steam costs a few seconds once,
/// and the next start finds the file already there.
pub fn before_launch(game: &LibraryGame) {
    let dir = PathBuf::from(&game.install_dir);
    let Some(appid) = appid_of(&dir) else { return };
    let result = tauri::async_runtime::block_on(async {
        tokio::time::timeout(Duration::from_secs(8), ach::ensure_gbe_schema(&client(), &dir, &appid)).await
    });
    match result {
        Ok(Ok(fix)) => {
            if fix.folders > 0 {
                crate::logging::info("achievements", &format!("{}: achievement list added ({} folder(s))", game.title, fix.folders));
            }
            if !fix.icons.is_empty() {
                let title = game.title.clone();
                tauri::async_runtime::spawn(async move {
                    let n = ach::fetch_icons(&client(), fix.icons).await.len();
                    crate::logging::info("achievements", &format!("{title}: {n} achievement icons fetched"));
                });
            }
        }
        Ok(Err(e)) => crate::logging::warn("achievements", &format!("{}: {e}", game.title)),
        Err(_) => crate::logging::warn("achievements", &format!("{}: Steam was too slow; trying again next start", game.title)),
    }
}

fn cache_file<R: Runtime>(app: &AppHandle<R>, appid: &str) -> Option<PathBuf> {
    Some(app.path().app_data_dir().ok()?.join("achievements").join(format!("{appid}.json")))
}

/// The game's list, from the app's cache while it is fresh, else Steam (and
/// the stale cache if Steam does not answer).
async fn schema<R: Runtime>(app: &AppHandle<R>, appid: &str) -> Vec<Achievement> {
    let file = cache_file(app, appid);
    let cached: Option<(Vec<Achievement>, bool)> = file.as_ref().and_then(|f| {
        let fresh = std::fs::metadata(f)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < SCHEMA_FRESH);
        let list = serde_json::from_slice(&std::fs::read(f).ok()?).ok()?;
        Some((list, fresh))
    });
    if let Some((list, true)) = &cached {
        return list.clone();
    }
    match ach::fetch_schema(&client(), appid, "english").await {
        Ok(list) => {
            if let Some(f) = &file {
                let _ = std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")));
                let _ = std::fs::write(f, serde_json::to_vec(&list).unwrap_or_default());
            }
            list
        }
        Err(_) => cached.map(|(l, _)| l).unwrap_or_default(),
    }
}

/// One achievement on the game page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameAchievement {
    #[serde(flatten)]
    pub achievement: Achievement,
    pub unlocked: bool,
    /// Unix seconds.
    pub unlocked_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameAchievements {
    pub appid: Option<String>,
    pub total: usize,
    pub unlocked: usize,
    /// Unlocked first, newest first; then the rest, most common first.
    pub list: Vec<GameAchievement>,
    pub tracking: Tracking,
    pub emulator: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tracking { Supported, Unsupported, Unknown }

fn named_tracking(source: &str) -> (Tracking, Option<String>) {
    let name: String = source.chars().filter(|c| c.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect();
    let emulator = if name.contains("kryotoonline") || name.ends_with("kryotoo") {
        return (Tracking::Unsupported, Some("Kryoto Online".into()));
    } else if name.contains("gbefork") || name.contains("goldberg") {
        "gbe_fork / Goldberg"
    } else if name.contains("rune") {
        "RUNE"
    } else if name.contains("codex") {
        "CODEX"
    } else if name.contains("onlinefix") || name == "ofme" {
        "OnlineFix"
    } else {
        return (Tracking::Unknown, None);
    };
    (Tracking::Supported, Some(emulator.into()))
}

fn metadata_tracking(game: &LibraryGame) -> (Tracking, Option<String>) {
    if let Some(repair) = &game.repair {
        let known = named_tracking(&repair.source);
        if known.0 != Tracking::Unknown { return known; }
    }
    if game.online.is_some() {
        return named_tracking("Kryoto Online");
    }
    if game.addons.iter().any(|a| {
        named_tracking(&a.label).0 == Tracking::Unsupported || a.files.iter().any(|f| {
            f.replace('\\', "/").rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case("kryoto-online.ini"))
        })
    }) {
        return named_tracking("Kryoto Online");
    }
    named_tracking(game.source.as_deref().unwrap_or_default())
}

pub(crate) fn tracking_of(game: &LibraryGame) -> (Tracking, Option<String>) {
    if ach::local::load(Path::new(&game.install_dir)).is_some() {
        return (Tracking::Supported, Some("Native achievements".into()));
    }
    // Inspect the active Steam DLLs, excluding Repair's backups and SDK copies.
    // A downloaded Online add-on can replace them without changing the release label.
    let mut supported = None;
    for dll in kryoto_repair::crack::find_steam_api_dlls(Path::new(&game.install_dir)) {
        if let Some(product) = kryoto_repair::peversion::product_name(&dll) {
            let known = named_tracking(&product);
            if known.0 == Tracking::Unsupported { return known; }
            if known.0 == Tracking::Supported { supported = Some(known); }
        }
    }
    supported.unwrap_or_else(|| metadata_tracking(game))
}

/// The list with what is unlocked. Unlocks the list does not name (a game
/// Steam lists none for) still count.
pub fn merge(list: Vec<Achievement>, unlocks: &[Unlock]) -> GameAchievements {
    let mut out: Vec<GameAchievement> = list
        .into_iter()
        .map(|a| {
            let u = unlocks.iter().find(|u| u.name == a.name);
            GameAchievement { unlocked: u.is_some(), unlocked_at: u.and_then(|u| u.time), achievement: a }
        })
        .collect();
    out.sort_by(|a, b| {
        b.unlocked.cmp(&a.unlocked).then_with(|| b.unlocked_at.cmp(&a.unlocked_at)).then_with(|| {
            b.achievement.percent.partial_cmp(&a.achievement.percent).unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    let unlocked = out.iter().filter(|a| a.unlocked).count();
    GameAchievements { appid: None, total: out.len(), unlocked, list: out, tracking: Tracking::Unknown, emulator: None }
}

#[tauri::command]
pub async fn game_achievements(app: AppHandle, id: String) -> Result<GameAchievements, String> {
    let game = crate::library::load(&app)?.into_iter().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
    if let Some(local) = ach::local::load(Path::new(&game.install_dir)) {
        let mut out = merge(local.list.clone(), &local.unlocks());
        out.tracking = Tracking::Supported;
        out.emulator = Some("Native achievements".into());
        return Ok(out);
    }
    let (tracking, emulator) = tracking_of(&game);
    let appid = appid_of(Path::new(&game.install_dir));
    if tracking == Tracking::Unsupported || appid.is_none() {
        return Ok(GameAchievements { appid, total: 0, unlocked: 0, list: Vec::new(), tracking, emulator });
    }
    let appid = appid.unwrap();
    let list = schema(&app, &appid).await;
    // Where the game saves: its Wine/Proton prefix on Linux, the PC's own
    // folders on Windows - the same place the watcher reads while it runs.
    let roots = crate::library::plan_for(&app, &game, None).map(|(plan, _)| roots_for(&plan.env)).unwrap_or_else(|_| Roots::from_env());
    let unlocks = ach::read_unlocked(&roots, &appid);
    let mut out = merge(list, &unlocks);
    out.appid = Some(appid);
    out.tracking = tracking;
    out.emulator = emulator;
    Ok(out)
}

/// While the game runs: every couple of seconds, look at its save files, and
/// announce each achievement that was not unlocked before. Stops when the game
/// does.
pub fn watch<R: Runtime>(app: AppHandle<R>, game: &LibraryGame, roots: Roots) {
    if let Some(local) = ach::local::load(Path::new(&game.install_dir)) {
        watch_local(app, game, local);
        return;
    }
    let Some(appid) = appid_of(Path::new(&game.install_dir)) else { return };
    let (id, title) = (game.id.clone(), game.title.clone());
    std::thread::spawn(move || {
        let mut known: HashSet<String> = ach::read_unlocked(&roots, &appid).into_iter().map(|u| u.name).collect();
        let stamp = |roots: &Roots| -> Vec<(PathBuf, Option<SystemTime>)> {
            ach::save_files(roots, &appid).into_iter().map(|p| (p.clone(), std::fs::metadata(&p).and_then(|m| m.modified()).ok())).collect()
        };
        let mut seen = stamp(&roots);
        let mut list: Option<Vec<Achievement>> = None;
        loop {
            std::thread::sleep(POLL);
            let running = app.try_state::<Running>().is_some_and(|r| r.0.lock().map(|m| m.contains_key(&id)).unwrap_or(false));
            let now = stamp(&roots);
            if now != seen {
                seen = now;
                for u in ach::read_unlocked(&roots, &appid) {
                    if !known.insert(u.name.clone()) {
                        continue;
                    }
                    let list = list.get_or_insert_with(|| tauri::async_runtime::block_on(schema(&app, &appid)));
                    let a = list.iter().find(|a| a.name == u.name);
                    let name = a.map(|a| a.display_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| u.name.clone());
                    let body = a.map(|a| a.description.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| title.clone());
                    crate::logging::info("achievements", &format!("{title}: unlocked {name}"));
                    let _ = app.emit(
                        "achievement-unlocked",
                        serde_json::json!({ "gameId": id, "name": u.name, "displayName": name, "icon": a.and_then(|a| a.icon.clone()) }),
                    );
                    let _ = app.emit("notify", crate::downloads::Notice::new(&format!("Achievement unlocked: {name}"), &body, Some(id.clone())).sound("achievement"));
                }
            }
            if !running {
                break;
            }
        }
    });
}

fn watch_local<R: Runtime>(app: AppHandle<R>, game: &LibraryGame, local: ach::local::LocalGame) {
    let (id, title) = (game.id.clone(), game.title.clone());
    std::thread::spawn(move || {
        let mut known: HashSet<String> = local.unlocks().into_iter().map(|u| u.name).collect();
        loop {
            std::thread::sleep(POLL);
            let running = app.try_state::<Running>().is_some_and(|r| r.0.lock().map(|m| m.contains_key(&id)).unwrap_or(false));
            for u in local.unlocks() {
                if !known.insert(u.name.clone()) { continue; }
                let Some(a) = local.list.iter().find(|a| a.name == u.name) else { continue };
                let _ = app.emit("achievement-unlocked", serde_json::json!({
                    "gameId": id, "name": u.name, "displayName": a.display_name, "icon": a.icon
                }));
                let _ = app.emit("notify", crate::downloads::Notice::new(
                    &format!("Achievement unlocked: {}", a.display_name),
                    if a.description.is_empty() { &title } else { &a.description }, Some(id.clone())
                ).sound("achievement"));
            }
            if !running { break; }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rune_profiles_and_gbe_are_supported_but_kryoto_online_is_not() {
        for source in ["Steam + gbe_fork", "GBE-Fork", "Goldberg", "Steam + RUNE", "RUNE (Steakclient)", "RUNE (Steamclient)", "CODEX", "Online-Fix", "OFME"] {
            assert_eq!(named_tracking(source).0, Tracking::Supported, "{source}");
        }
        for source in ["Steam + Kryoto Online", "kryoto-online", "KryotoO"] {
            assert_eq!(named_tracking(source), (Tracking::Unsupported, Some("Kryoto Online".into())));
        }
        for source in ["Steam (DRM-free)", "Kryoto Original release", "unknown repack", "custom", ""] {
            assert_eq!(named_tracking(source), (Tracking::Unknown, None));
        }
    }

    #[test]
    fn local_changes_override_the_original_release_label() {
        let mut game = LibraryGame { source: Some("Steam + gbe_fork".into()), ..Default::default() };
        game.online = Some(Default::default());
        assert_eq!(metadata_tracking(&game).0, Tracking::Unsupported);
        game.repair = Some(crate::repair::LocalRepair { source: "gbe_fork".into(), ..Default::default() });
        assert_eq!(metadata_tracking(&game).0, Tracking::Supported);
        game.repair = None;
        game.online = None;
        game.addons.push(crate::addons::InstalledAddon { files: vec!["Binaries\\kryoto-online.ini".into()], ..Default::default() });
        assert_eq!(metadata_tracking(&game).0, Tracking::Unsupported);
    }

    #[test]
    fn active_dll_identity_overrides_stale_labels_and_ignores_repair_backups() {
        fn write_dll(path: &Path, product: &str) {
            let key: Vec<u8> = "ProductName\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
            let value: Vec<u8> = format!("{product}\0").encode_utf16().flat_map(u16::to_le_bytes).collect();
            let value_at = (6 + key.len() + 3) & !3;
            let size = value_at + value.len();
            let mut bytes = vec![0u8; 0x200 + size];
            bytes[..2].copy_from_slice(b"MZ");
            bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
            bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
            bytes[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
            bytes[0x98..0x9d].copy_from_slice(b".rsrc");
            bytes[0xa8..0xac].copy_from_slice(&(size as u32).to_le_bytes());
            bytes[0xac..0xb0].copy_from_slice(&0x200u32.to_le_bytes());
            bytes[0x200..0x202].copy_from_slice(&(size as u16).to_le_bytes());
            bytes[0x202..0x204].copy_from_slice(&(value.len() as u16 / 2).to_le_bytes());
            bytes[0x204..0x206].copy_from_slice(&1u16.to_le_bytes());
            bytes[0x206..0x206 + key.len()].copy_from_slice(&key);
            bytes[0x200 + value_at..].copy_from_slice(&value);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let dir = std::env::temp_dir().join(format!("kry-ach-tracking-{}", std::process::id()));
        let game = LibraryGame { install_dir: dir.to_string_lossy().into(), source: Some("Steam + RUNE".into()), ..Default::default() };
        let dll = dir.join("bin/steam_api64.dll");
        write_dll(&dll, "kryoto-online");
        assert_eq!(tracking_of(&game).0, Tracking::Unsupported);
        write_dll(&dll, "gbe_fork");
        write_dll(&dir.join(".kryoto-repair/backup/steam_api64.dll"), "kryoto-online");
        assert_eq!(tracking_of(&game).0, Tracking::Supported);
        std::fs::write(&dll, b"ProductName kryoto-online").unwrap();
        assert_eq!(tracking_of(&game), named_tracking("RUNE"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn a(name: &str, percent: f32) -> Achievement {
        Achievement {
            name: name.into(),
            display_name: name.into(),
            description: String::new(),
            hidden: false,
            icon: None,
            icon_gray: None,
            percent: Some(percent),
        }
    }

    #[test]
    fn unlocked_come_first_newest_first_then_the_most_common() {
        let out = merge(
            vec![a("RARE", 1.0), a("COMMON", 90.0), a("OLD", 50.0), a("NEW", 5.0)],
            &[Unlock { name: "OLD".into(), time: Some(10) }, Unlock { name: "NEW".into(), time: Some(20) }],
        );
        let order: Vec<&str> = out.list.iter().map(|x| x.achievement.name.as_str()).collect();
        assert_eq!(order, ["NEW", "OLD", "COMMON", "RARE"]);
        assert_eq!((out.unlocked, out.total), (2, 4));
    }

    #[test]
    fn the_appid_comes_from_the_emulators_file_but_never_spacewar() {
        let dir = std::env::temp_dir().join(format!("kry-appid-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("bin/steam_settings")).unwrap();
        std::fs::write(dir.join("bin/steam_settings/steam_appid.txt"), "480").unwrap();
        assert_eq!(appid_of(&dir), None);
        std::fs::write(dir.join("bin/steam_appid.txt"), "\u{feff}620\r\n").unwrap();
        assert_eq!(appid_of(&dir).as_deref(), Some("620"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole path on a real gbe_fork build: the list Desktop gives a game
    /// before Play, the game unlocking through the emulator, and the unlocks
    /// read back the way the game page and the watcher read them.
    ///
    /// Opt-in, Windows: `KRYOTO_ACH_GAME=<folder with gbe_fork's steam_api64.dll,
    /// steam_appid.txt and steam_settings/> KRYOTO_ACH_EXE=<a program in it that
    /// unlocks> KRYOTO_ACH_APPID=620 cargo test achievements_end_to_end -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn achievements_end_to_end() {
        let game = PathBuf::from(std::env::var("KRYOTO_ACH_GAME").expect("KRYOTO_ACH_GAME"));
        let exe = std::env::var("KRYOTO_ACH_EXE").expect("KRYOTO_ACH_EXE");
        let appid = std::env::var("KRYOTO_ACH_APPID").unwrap_or_else(|_| "620".into());
        assert_eq!(appid_of(&game).as_deref(), Some(appid.as_str()));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let fix = rt.block_on(ach::ensure_gbe_schema(&client(), &game, &appid)).unwrap();
        println!("list written into {} steam_settings folder(s), {} icons to fetch", fix.folders, fix.icons.len());
        let roots = Roots::from_env();
        let before = ach::read_unlocked(&roots, &appid);
        let out = std::process::Command::new(&exe).current_dir(&game).output().unwrap();
        println!("{}", String::from_utf8_lossy(&out.stdout));
        let after = ach::read_unlocked(&roots, &appid);
        println!("before: {before:?}\nafter: {after:?}");
        assert!(after.len() > before.len(), "the game's unlocks were not recorded");
    }

    #[test]
    fn a_proton_prefix_is_where_its_saves_are() {
        let r = roots_for(&[("STEAM_COMPAT_DATA_PATH".into(), "/p".into())]);
        assert_eq!(r.public_documents, Some(PathBuf::from("/p/pfx/drive_c/users/Public/Documents")));
    }

    /// Runs a native game's own trophy recorder against a private fixture, then
    /// reads its actual shipped manifest through Desktop's list/progress path.
    #[test]
    #[ignore]
    fn native_achievements_end_to_end() {
        let manifest = std::env::var("KRYOTO_NATIVE_MANIFEST").expect("manifest path");
        let recorder = std::env::var("KRYOTO_NATIVE_RECORDER").expect("recorder fixture executable");
        let dir = std::env::temp_dir().join(format!("kry-native-e2e-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::create_dir(dir.join("userdata")).unwrap();
        std::fs::copy(manifest, dir.join(ach::local::MANIFEST)).unwrap();
        let game = LibraryGame { install_dir: dir.to_string_lossy().into(), ..Default::default() };
        assert_eq!(appid_of(&dir), None);
        assert_eq!(tracking_of(&game).0, Tracking::Supported);
        let local = ach::local::load(&dir).unwrap();
        assert_eq!((local.list.len(), local.unlocks().len()), (40, 0));
        assert!(std::process::Command::new(recorder).arg(dir.join("userdata")).status().unwrap().success());
        let progress = merge(local.list.clone(), &local.unlocks());
        assert_eq!((progress.total, progress.unlocked), (40, 35));
        assert!(progress.list.iter().any(|a| a.achievement.name == "0" && a.unlocked));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
