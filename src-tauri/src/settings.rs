//! Client settings: where games go, what happens to archives, what opens first.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime};

/// Kryoto Desktop Preview talks to the Kryoto Preview (src/channel.rs).
pub const DEFAULT_CATALOG_ENDPOINT: &str = if crate::channel::PREVIEW { "https://preview.kryo.to" } else { "https://kryo.to" };

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub recovery_error: Option<String>,
    /// Where downloaded games are installed, one folder each.
    pub library_dir: String,
    /// Delete the archive once a game is installed from it.
    pub delete_archives: bool,
    /// Where archives are downloaded to, so a game can install on another
    /// drive. Empty: `_downloads` inside the library folder.
    pub download_dir: String,
    /// `store` or `library`.
    pub start_page: String,
    /// Wine or Proton for games that have not picked their own (not Windows).
    pub default_compat_tool: Option<String>,
    /// Minimize the client while a game runs.
    pub minimize_on_play: bool,
    /// Let a paired phone on this network see the library and start games.
    pub phone_lan: bool,
    /// Show a notification when a download finishes.
    pub notify_downloads: bool,
    /// kryo.to's palette: `monochrome`, `oled`, `amber`, `emerald`, `nord`, `sepia`, `blossom`.
    pub palette: String,
    /// `account` follows kryo.to; also `system`, `light` or `dark`.
    pub theme: String,
    /// kryo.to's corner setting: `sharp`, `soft`, `rounded`, `round`, `pill`.
    pub radius: String,
    /// `teletext` (the house face) or `mono`.
    pub font: String,
    /// How large everything is drawn, in percent (text_size.rs): 90, 100,
    /// 110, 125 or 150. Its own commands change it; a page save keeps it.
    pub text_size: u32,
    /// Show adult games' art unblurred. Off by default, as on kryo.to.
    pub show_adult: bool,
    /// Take palette, corners, typeface and the adult blur from the signed-in
    /// kryo.to account instead of the four settings above.
    pub follow_account: bool,
    /// More library folders besides `library_dir` (which is where new games
    /// go). Settings > Storage adds and removes them.
    pub library_folders: Vec<String>,
    /// With more than one library folder, a new download asks which one to
    /// install into (the default picked). Off: always the default.
    pub ask_install_folder: bool,
    /// Send errors and crashes to kryo.to so they get fixed.
    pub send_reports: bool,
    /// The close button hides the window to the tray instead of quitting.
    pub close_to_tray: bool,
    /// Start (to the tray) when you sign in to Windows.
    pub start_with_system: bool,
    /// Buttons press in under the pointer, as on kryo.to.
    pub press_effect: bool,
    /// Add play time to the account on kryo.to (community statistics).
    pub share_playtime: bool,
    /// Parallel connections per download (1 = one stream).
    pub connections: u32,
    /// Download speed cap in MB/s; 0 is no cap.
    pub speed_limit_mb: u32,
    /// Optional local or staging Kryo.to origin for development.
    pub catalog_endpoint: String,
    /// Linux: MangoHud's overlay on every Windows game (when it is installed).
    pub linux_mangohud: bool,
    /// Linux: run Windows games under Feral GameMode (when it is installed).
    pub linux_gamemode: bool,
    /// Linux: Proton-GE's FSR upscaling at lower fullscreen resolutions.
    pub linux_fsr: bool,
    /// Settings written before 0.3 had 8 connections as the default. The first
    /// load after that moves an untouched 8 to the new default of 16, once.
    /// Missing from an old file reads as false (the field's own default, not
    /// the struct's).
    #[serde(default)]
    pub connections_v2: bool,
    /// The name you have in games: your K// username, the one typed below,
    /// or each build's own (player_name.rs). Games can pick their own.
    pub player_name_mode: crate::player_name::Mode,
    pub player_name: String,
    /// The language games start in, as Steam names it; empty leaves each
    /// build's own (player_name.rs). Games can pick their own.
    pub game_language: String,
    /// Cloud saves (saves/), for accounts kryo.to has them on for.
    pub cloud_saves: bool,
    /// Back a game's saves up when it closes.
    pub cloud_saves_on_exit: bool,
    /// Newer saves from another PC before Play: `auto` puts them here,
    /// `ask` asks first.
    pub cloud_saves_restore: String,
    /// Watch where games write while they run, to find their saves.
    pub cloud_saves_detect: bool,
    /// How: `precise` asks for administrator once per game so Windows says
    /// exactly which files it wrote (saves/precise.rs); `smart` needs none.
    pub cloud_saves_find: String,
    /// Tell kryo.to the save folders found (placeholder paths only), so
    /// other players' games find theirs on first launch.
    pub cloud_saves_share: bool,
    /// How many local backups of replaced saves each game keeps.
    pub cloud_saves_keep_local: u32,
    /// The desktop pet (pet.rs): out on the screen or put away.
    pub pet: bool,
    /// Which mascot it is: telly, kubi, yuki or blip.
    pub pet_mascot: String,
    /// Where it was put, in physical pixels; the screen's corner when unset.
    pub pet_pos: Option<[i32; 2]>,
    /// What Discord shows while a game runs (discord_rpc/).
    pub discord: DiscordPresence,
    /// Patchbay (patchbay/): a server of the player's own for cloud saves,
    /// instead of kryo.to. Empty is kryo.to.
    pub saves_endpoint: String,
    /// Where save versions are kept (saves/): `account` (kryo.to, a supporter
    /// perk), `local` (a folder on this PC, `saves_local_dir`) or `patchbay`
    /// (`saves_endpoint`). Empty, as before there was a choice: the own server
    /// when one is set, else kryo.to.
    pub saves_mode: String,
    /// `local`: the folder save versions go in, the player's pick, apart from
    /// every cache.
    pub saves_local_dir: String,
    /// `local`: versions kept per game, the newest ones.
    pub saves_local_keep: u32,
    /// The card about kryo.to's cloud saves was closed: never shown again.
    pub saves_promo_closed: bool,
    /// Where achievements are kept (achievements_cloud.rs): `account`, `local`
    /// (`achievements.json` in `saves_local_dir`) or `patchbay`; empty as
    /// `saves_mode`.
    pub achievements_mode: String,
    /// Patchbay: the same for achievements.
    pub achievements_endpoint: String,
    /// Patchbay: the debrid service downloads go through first, `""` for none
    /// `real-debrid` or `torbox`. Its token is in the OS secure store, not here.
    pub debrid: String,
    /// Unpack archives in place with HatchZip (hatchzip.rs): the archive is
    /// eaten from its end as the game's files appear, so a game needs room for
    /// itself, not for itself and its archive. Off: 7-Zip as before, except
    /// when the drive is short, where HatchZip steps in anyway.
    pub extract_in_place: bool,
    /// New-feature notices (src/ui/FeatureNotice.tsx) the player has dismissed.
    pub seen_notices: Vec<String>,
    /// VR (vr.rs): the OpenXR runtime VR games get (`XR_RUNTIME_JSON`), by its
    /// id in `vr::runtimes`; empty leaves the system's own choice.
    pub vr_runtime: String,
    /// VR: start SteamVR before a VR game when it is installed and not running.
    pub vr_start_steamvr: bool,
    /// The Store's address bar (adblock.rs): refuse requests to ad servers on
    /// sites other than kryo.to. kryo.to's own pages are never filtered.
    pub block_ads: bool,
    /// Game updates (game_update.rs): get a newer Kryoto build of an
    /// installed game without asking. Off by default: a new build can stop
    /// working with its emulator setup or a fix applied to the old one.
    pub auto_update_games: bool,
    /// Where an update copies the files it replaces first, so it can be
    /// undone. Empty: no copies (and no Undo).
    pub update_backup_dir: String,
    /// The first-start setup (src/setup/FirstStart.tsx) is done or skipped. False
    /// only on a fresh install: a settings file from before it existed reads
    /// as true, so nobody who already uses Kryoto is walked through it.
    #[serde(default = "already_set_up")]
    pub welcomed: bool,
}

fn already_set_up() -> bool {
    true
}

/// One of the two buttons Discord allows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PresenceButton {
    /// `desktop`, `game`, `donate`, `profile`, `custom`, `none`, or `rotate`:
    /// the game's page and Donate, taking turns.
    pub kind: String,
    /// Blank is the usual words for that kind.
    pub label: String,
    /// For `custom`.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscordPresence {
    pub on: bool,
    /// Show Kryoto itself while no game runs.
    pub browsing: bool,
    /// The two lines, with {game}, {playtime}, {version} and {developer}.
    pub line1: String,
    pub line2: String,
    /// `cover`, `kryoto` or `none`.
    pub picture: String,
    /// The running clock.
    pub elapsed: bool,
    /// Total play time, where a line asks for it.
    pub playtime: bool,
    /// Adult games show their name and art like any other.
    pub adult_names: bool,
    pub button1: PresenceButton,
    pub button2: PresenceButton,
}

impl Default for DiscordPresence {
    fn default() -> Self {
        Self {
            on: true,
            browsing: false,
            line1: "{game}".into(),
            line2: "{playtime} played".into(),
            picture: "cover".into(),
            elapsed: true,
            playtime: true,
            adult_names: false,
            button1: PresenceButton { kind: "desktop".into(), ..Default::default() },
            button2: PresenceButton { kind: "rotate".into(), ..Default::default() },
        }
    }
}

impl DiscordPresence {
    fn clean(&mut self) {
        if !matches!(self.picture.as_str(), "cover" | "kryoto" | "none") {
            self.picture = "cover".into();
        }
        for line in [&mut self.line1, &mut self.line2] {
            *line = line.trim().chars().take(200).collect();
        }
        for b in [&mut self.button1, &mut self.button2] {
            if !matches!(b.kind.as_str(), "desktop" | "game" | "donate" | "profile" | "custom" | "none" | "rotate") {
                b.kind = "none".into();
            }
            b.label = b.label.trim().chars().take(32).collect();
            b.url = b.url.trim().chars().take(512).collect();
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recovery_error: None,
            library_dir: String::new(),
            delete_archives: true,
            download_dir: String::new(),
            start_page: "library".into(),
            default_compat_tool: None,
            minimize_on_play: false,
            phone_lan: false,
            notify_downloads: true,
            palette: "monochrome".into(),
            theme: "account".into(),
            radius: "pill".into(),
            font: "teletext".into(),
            text_size: crate::text_size::DEFAULT,
            show_adult: false,
            follow_account: true,
            library_folders: Vec::new(),
            ask_install_folder: true,
            send_reports: true,
            close_to_tray: true,
            start_with_system: false,
            press_effect: true,
            share_playtime: true,
            connections: 16,
            speed_limit_mb: 0,
            catalog_endpoint: String::new(),
            linux_mangohud: false,
            linux_gamemode: false,
            linux_fsr: false,
            connections_v2: true,
            player_name_mode: crate::player_name::Mode::Account,
            player_name: String::new(),
            game_language: String::new(),
            cloud_saves: true,
            cloud_saves_on_exit: true,
            cloud_saves_restore: "auto".into(),
            cloud_saves_detect: true,
            cloud_saves_find: "precise".into(),
            cloud_saves_share: true,
            cloud_saves_keep_local: 5,
            pet: true,
            pet_mascot: "telly".into(),
            pet_pos: None,
            discord: DiscordPresence::default(),
            saves_endpoint: String::new(),
            saves_mode: String::new(),
            saves_local_dir: String::new(),
            saves_local_keep: 10,
            saves_promo_closed: false,
            achievements_mode: String::new(),
            achievements_endpoint: String::new(),
            debrid: String::new(),
            extract_in_place: false,
            seen_notices: Vec::new(),
            vr_runtime: String::new(),
            vr_start_steamvr: true,
            block_ads: true,
            auto_update_games: false,
            update_backup_dir: String::new(),
            welcomed: false,
        }
    }
}

/// A Patchbay endpoint: an http(s) base address, which may have a path (a
/// server behind a reverse proxy at `/kryoto`), but no query, fragment or
/// credentials. Plain HTTP on this PC or the local network only, so a token
/// never crosses the internet in the clear.
pub fn normalize_own_endpoint(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    let url = url::Url::parse(value).map_err(|_| "Use a full address, such as https://saves.example.com.".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an http(s) address without a query or credentials.".into());
    }
    if url.scheme() == "http" {
        let local = match url.host() {
            Some(url::Host::Domain(h)) => h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") || h.ends_with(".lan") || h.ends_with(".home.arpa"),
            Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
            None => false,
        };
        if !local {
            return Err("Use HTTPS, or HTTP on this PC or your local network only.".into());
        }
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub fn normalize_catalog_endpoint(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    let mut url = url::Url::parse(value)
        .map_err(|_| "Use a full endpoint URL, such as http://localhost:3000.".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err("Use an http(s) site origin only, without a path, query or credentials.".into());
    }
    if url.scheme() == "http" {
        let host = url.host_str().unwrap_or_default();
        let loopback = host == "localhost"
            || host.ends_with(".localhost")
            || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
        if !loopback {
            return Err("Use HTTPS, or HTTP on localhost only.".into());
        }
    }
    url.set_path("/");
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub fn catalog_endpoint(settings: &Settings) -> String {
    normalize_catalog_endpoint(&settings.catalog_endpoint)
        .ok()
        .filter(|endpoint| !endpoint.is_empty())
        .unwrap_or_else(|| DEFAULT_CATALOG_ENDPOINT.into())
}

pub fn is_catalog_origin(url: &url::Url, settings: &Settings) -> bool {
    if !settings.catalog_endpoint.trim().is_empty() {
        let Ok(endpoint) = url::Url::parse(&catalog_endpoint(settings)) else { return false };
        return url.origin() == endpoint.origin();
    }
    let host = url.host_str().unwrap_or("");
    // The status page is a page of its own: it knows nobody, and asking it who
    // is signed in would read as everyone having signed out.
    url.scheme() == "https" && host != "status.kryo.to" && (host == "kryo.to" || host.ends_with(".kryo.to"))
}

fn file<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("settings.json"))
}

/// Every library folder, the default (where new games go) first.
pub fn all_folders(s: &Settings) -> Vec<String> {
    let mut out = vec![s.library_dir.clone()];
    for f in &s.library_folders {
        if !out.iter().any(|o| same_folder(o, f)) {
            out.push(f.clone());
        }
    }
    out
}

/// The same place on disk, however it is written: a junction, or `~1` names.
pub fn same_folder(a: &str, b: &str) -> bool {
    same_path(a, b) || matches!((dunce::canonicalize(a), dunce::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

pub fn same_path(a: &str, b: &str) -> bool {
    let norm = |p: &str| {
        let p = p.trim().trim_end_matches(['\\', '/']).replace('\\', "/");
        if cfg!(windows) { p.to_lowercase() } else { p }
    };
    norm(a) == norm(b)
}

/// The settings as last read or written. `load` runs on every Store
/// navigation and page report, which is no reason to go to the disk each time.
static CACHE: std::sync::RwLock<Option<Settings>> = std::sync::RwLock::new(None);

pub fn write<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> Result<(), String> {
    let target = file(app)?;
    save_settings_file(&target, settings)?;
    if let Ok(mut cache) = CACHE.write() {
        *cache = Some(settings.clone());
    }
    Ok(())
}

fn save_settings_file(target: &std::path::Path, settings: &Settings) -> Result<(), String> {
    read_settings_file(target).map_err(|_| "Your existing settings could not be read. Recover them before saving changes.".to_string())?;
    replace_settings_file(target, settings)
}

/// One save at a time. Two at once shared the temporary file, and the second
/// failed to rename it ("cannot find the file"): whatever it was recording, like
/// a notice seen, was lost and asked again at the next start.
static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn replace_settings_file(target: &std::path::Path, settings: &Settings) -> Result<(), String> {
    use std::io::Write;
    let _one_at_a_time = SAVING.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = target.with_extension("json.tmp");
    let mut output = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    output.write_all(serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    std::fs::rename(tmp, target).map_err(|e| e.to_string())
}

pub fn load<R: Runtime>(app: &AppHandle<R>) -> Settings {
    if let Some(s) = CACHE.read().ok().and_then(|c| c.clone()) {
        return s;
    }
    let s = read(app);
    if let Ok(mut cache) = CACHE.write() {
        *cache = Some(s.clone());
    }
    s
}

fn read<R: Runtime>(app: &AppHandle<R>) -> Settings {
    let mut s = file(app).and_then(|path| read_settings_file(&path)).unwrap_or_else(recovery_defaults);
    if s.library_dir.trim().is_empty() {
        let home = app.path().home_dir().unwrap_or_else(|_| PathBuf::from("."));
        s.library_dir = home.join("Kryoto Games").to_string_lossy().into_owned();
    }
    if !s.connections_v2 && s.recovery_error.is_none() {
        if s.connections == 8 {
            s.connections = 16;
        }
        s.connections_v2 = true;
        let _ = write(app, &s);
    }
    s
}

fn read_settings_file(path: &std::path::Path) -> Result<Settings, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("Settings could not be read: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(format!("Settings could not be read: {e}")),
    }
}

fn recovery_defaults(error: String) -> Settings {
    Settings { send_reports: false, share_playtime: false, recovery_error: Some(error), welcomed: true, ..Settings::default() }
}

/// Explicit player action: preserve the damaged file before choosing safe defaults.
#[tauri::command(async)]
pub fn settings_recover(app: AppHandle) -> Result<Settings, String> {
    let current = load(&app);
    if current.recovery_error.is_none() { return Ok(current); }
    let target = file(&app)?;
    let settings = recover_settings_file(&target, current)?;
    if let Ok(mut cache) = CACHE.write() { *cache = Some(settings.clone()); }
    Ok(settings)
}

fn recover_settings_file(target: &std::path::Path, current: Settings) -> Result<Settings, String> {
    if target.exists() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
        let backup = target.with_extension(format!("json.backup-{stamp}"));
        // Keep the original in place until the complete replacement is ready.
        // A failed write or restart must still find the unreadable original,
        // rather than mistake a missing file for a first run with sharing on.
        let mut original = std::fs::File::open(target).map_err(|e| format!("Could not preserve your settings. No defaults were saved: {e}"))?;
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(backup)
            .map_err(|e| format!("Could not preserve your settings. No defaults were saved: {e}"))?;
        std::io::copy(&mut original, &mut output).map_err(|e| format!("Could not preserve your settings. No defaults were saved: {e}"))?;
        output.sync_all().map_err(|e| format!("Could not preserve your settings. No defaults were saved: {e}"))?;
    }
    let mut settings = Settings { recovery_error: None, ..current };
    settings.send_reports = false;
    settings.share_playtime = false;
    replace_settings_file(target, &settings)?;
    Ok(settings)
}

#[tauri::command(async)]
pub fn settings_get(app: AppHandle) -> Settings {
    load(&app)
}

#[tauri::command(async)]
pub fn settings_save(app: AppHandle, mut settings: Settings) -> Result<Settings, String> {
    if settings.library_dir.trim().is_empty() {
        return Err("Pick a folder for the library.".into());
    }
    let before = load(&app);
    settings.download_dir = settings.download_dir.trim().to_string();
    // Only a folder being picked now has to answer: with an external drive
    // unplugged, every other switch in Settings failed on the one it held.
    if !settings.download_dir.is_empty() && settings.download_dir != before.download_dir {
        std::fs::create_dir_all(&settings.download_dir).map_err(|e| format!("Cannot use {} for downloads: {e}", settings.download_dir))?;
    }
    settings.catalog_endpoint = normalize_catalog_endpoint(&settings.catalog_endpoint)?;
    settings.saves_endpoint = normalize_own_endpoint(&settings.saves_endpoint).map_err(|e| format!("Saves server: {e}"))?;
    settings.achievements_endpoint = normalize_own_endpoint(&settings.achievements_endpoint).map_err(|e| format!("Achievements server: {e}"))?;
    if !matches!(settings.debrid.as_str(), "" | "real-debrid" | "torbox") {
        settings.debrid = String::new();
    }
    settings.player_name = crate::player_name::clean(&settings.player_name).unwrap_or_default();
    settings.game_language = crate::player_name::clean_language(&settings.game_language).unwrap_or_default();
    if !matches!(settings.cloud_saves_restore.as_str(), "auto" | "ask") {
        settings.cloud_saves_restore = "auto".into();
    }
    if !matches!(settings.cloud_saves_find.as_str(), "precise" | "smart") {
        settings.cloud_saves_find = "precise".into();
    }
    settings.cloud_saves_keep_local = settings.cloud_saves_keep_local.clamp(1, 20);
    if !matches!(settings.saves_mode.as_str(), "" | "account" | "local" | "patchbay") {
        settings.saves_mode = String::new();
    }
    settings.saves_local_dir = settings.saves_local_dir.trim().to_string();
    if !matches!(settings.achievements_mode.as_str(), "" | "account" | "local" | "patchbay") {
        settings.achievements_mode = String::new();
    }
    if settings.achievements_mode == "patchbay" && settings.achievements_endpoint.is_empty() {
        return Err("Add your achievements server in Settings > Patchbay first.".into());
    }
    if settings.saves_mode == "local" || settings.achievements_mode == "local" {
        if settings.saves_local_dir.is_empty() {
            return Err("Choose a folder for your save versions.".into());
        }
        if settings.saves_local_dir != before.saves_local_dir {
            std::fs::create_dir_all(&settings.saves_local_dir).map_err(|e| format!("Cannot use {} for saves: {e}", settings.saves_local_dir))?;
        }
    }
    if settings.saves_mode == "patchbay" && settings.saves_endpoint.is_empty() {
        return Err("Add your saves server in Settings > Patchbay first.".into());
    }
    settings.saves_local_keep = settings.saves_local_keep.clamp(1, 100);
    settings.discord.clean();
    if !crate::pet::is_mascot(&settings.pet_mascot) {
        settings.pet_mascot = "telly".into();
    }
    settings.seen_notices.retain(|n| !n.is_empty() && n.len() <= 40 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    settings.seen_notices.dedup();
    settings.seen_notices.truncate(64);
    if settings.vr_runtime.len() > 80 {
        settings.vr_runtime = String::new();
    }
    // A choice made in Settings is the player's, never migrated again.
    settings.connections_v2 = true;
    if settings.library_dir != before.library_dir {
        std::fs::create_dir_all(&settings.library_dir).map_err(|e| format!("Cannot use {}: {e}", settings.library_dir))?;
    }
    // The phone switches belong to Settings > Phone's own commands, which also
    // start and stop the server; saving the whole page must not undo them.
    settings.phone_lan = before.phone_lan;
    // Where the pet stands is the pet's to remember (pet.rs), not the page's.
    settings.pet_pos = before.pet_pos;
    // So is the text size (text_size.rs): a key can change it while this page's
    // copy is a moment old.
    settings.text_size = before.text_size;
    write(&app, &settings)?;
    if before.discord != settings.discord {
        crate::discord_rpc::refresh(&app);
    }
    if before.pet != settings.pet || before.pet_mascot != settings.pet_mascot {
        crate::pet::sync(&app);
    }
    if before.block_ads != settings.block_ads {
        crate::adblock::set_enabled(&app, settings.block_ads);
    }
    if before.saves_endpoint != settings.saves_endpoint
        || before.achievements_endpoint != settings.achievements_endpoint
        || before.saves_mode != settings.saves_mode
        || before.saves_local_dir != settings.saves_local_dir
        || before.achievements_mode != settings.achievements_mode
    {
        crate::patchbay::endpoints_changed(&app);
    }
    if before.share_playtime != settings.share_playtime {
        crate::presence::beat(&app);
    }
    if before.catalog_endpoint != settings.catalog_endpoint {
        crate::catalog_endpoint_changed(&app, &settings)?;
    }
    if before.start_with_system != settings.start_with_system {
        if let Err(e) = crate::system::set_autostart(settings.start_with_system) {
            crate::logging::error("settings", &format!("start with Windows: {e}"));
            return Err(format!("Saved, but starting with Windows could not be turned {}: {e}", if settings.start_with_system { "on" } else { "off" }));
        }
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    #[test]
    fn failed_recovery_write_keeps_the_original_and_safe_restart_choices() {
        let dir = std::env::temp_dir().join(format!("kryoto-recovery-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "damaged settings").unwrap();
        // Block the staging write after the backup succeeds.
        std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
        let current = recovery_defaults(read_settings_file(&path).unwrap_err());
        assert!(recover_settings_file(&path, current).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "damaged settings");
        let restarted = recovery_defaults(read_settings_file(&path).unwrap_err());
        assert!(!restarted.send_reports && !restarted.share_playtime);
        assert!(restarted.recovery_error.is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn damaged_existing_settings_preserve_bytes_and_disable_sharing() {
        let path = std::env::temp_dir().join(format!("kryoto-settings-{}-{}.json", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        assert!(read_settings_file(&path).unwrap().recovery_error.is_none());
        std::fs::write(&path, "{\"sendReports\":false,").unwrap();
        let settings = recovery_defaults(read_settings_file(&path).unwrap_err());
        assert!(settings.recovery_error.is_some());
        assert!(!settings.send_reports && !settings.share_playtime);
        assert!(save_settings_file(&path, &Settings::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"sendReports\":false,");
        let recovered = recover_settings_file(&path, settings).unwrap();
        assert!(recovered.recovery_error.is_none());
        assert!(!recovered.send_reports && !recovered.share_playtime);
        let backup = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().find(|entry| entry.file_name().to_string_lossy().starts_with(&format!("{}.backup-", path.file_name().unwrap().to_string_lossy()))).unwrap().path();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{\"sendReports\":false,");
        std::fs::remove_file(backup).unwrap();
        std::fs::remove_file(path).unwrap();
    }
    use super::{catalog_endpoint, is_catalog_origin, normalize_catalog_endpoint, read_settings_file, recovery_defaults, recover_settings_file, save_settings_file, Settings, DEFAULT_CATALOG_ENDPOINT};

    #[test]
    fn the_status_page_is_not_kryo_to_itself() {
        let settings = Settings::default();
        let origin = |u: &str| is_catalog_origin(&url::Url::parse(u).unwrap(), &settings);
        assert!(origin("https://kryo.to/game/x"));
        assert!(origin("https://chat.kryo.to/"));
        assert!(!origin("https://status.kryo.to/"));
    }

    #[test]
    fn an_old_file_is_moved_to_the_new_connection_default_once() {
        let old: Settings = serde_json::from_str(r#"{"connections":8}"#).unwrap();
        assert!(!old.connections_v2);
        let new: Settings = serde_json::from_str(r#"{"connections":8,"connectionsV2":true}"#).unwrap();
        assert!(new.connections_v2);
    }

    #[test]
    fn only_a_fresh_install_is_walked_through_the_setup() {
        // A settings file from before the setup existed: already set up.
        let old: Settings = serde_json::from_str(r#"{"connections":16}"#).unwrap();
        assert!(old.welcomed);
        // No settings file at all: a fresh install.
        assert!(!Settings::default().welcomed);
        // A damaged file being recovered is not a fresh install either.
        assert!(recovery_defaults("broken".into()).welcomed);
    }

    #[test]
    fn custom_endpoint_is_a_safe_origin() {
        assert_eq!(normalize_catalog_endpoint(" http://localhost:3000/ ").unwrap(), "http://localhost:3000");
        assert_eq!(normalize_catalog_endpoint("").unwrap(), "");
        assert!(normalize_catalog_endpoint("http://example.com").is_err());
        assert!(normalize_catalog_endpoint("http://127.0.0.1:3000/path").is_err());
        assert!(normalize_catalog_endpoint("https://user@example.com").is_err());
    }

    #[test]
    fn own_endpoints_keep_tokens_off_the_open_internet() {
        use super::normalize_own_endpoint as n;
        assert_eq!(n(" https://saves.example.com/kryoto/ ").unwrap(), "https://saves.example.com/kryoto");
        assert_eq!(n("http://192.168.1.20:8080").unwrap(), "http://192.168.1.20:8080");
        assert_eq!(n("http://nas.local").unwrap(), "http://nas.local");
        assert_eq!(n("").unwrap(), "");
        assert!(n("http://example.com").is_err());
        assert!(n("http://8.8.8.8").is_err());
        assert!(n("https://user:pw@example.com").is_err());
        assert!(n("https://example.com/?token=1").is_err());
        assert!(n("ftp://example.com").is_err());
    }

    #[test]
    fn custom_origin_replaces_the_production_catalog_trust() {
        let settings = Settings { catalog_endpoint: "http://localhost:3000".into(), ..Default::default() };
        assert_eq!(catalog_endpoint(&settings), "http://localhost:3000");
        assert!(is_catalog_origin(&"http://localhost:3000/game/demo".parse().unwrap(), &settings));
        assert!(!is_catalog_origin(&"https://kryo.to/game/demo".parse().unwrap(), &settings));
        assert_eq!(catalog_endpoint(&Settings::default()), DEFAULT_CATALOG_ENDPOINT);
    }
}
