//! Hydra Launcher's library (github.com/hydralauncher/hydra), read-only.
//!
//! Hydra is an Electron app. Its user data folder is Electron's `userData`
//! for the package name `hydralauncher`, and the library is the LevelDB
//! store `hydra-db` in it (`src/main/constants.ts`, `levelDatabasePath`),
//! opened through `classic-level` with JSON values. Sublevels prefix their
//! keys `!<name>!` (abstract-level), so a game is `!games!<shop>:<objectId>`
//! (`src/main/level/sublevels/keys.ts`) holding a `Game`
//! (`src/types/level.types.ts`), and its store art is under
//! `!gameShopAssets!<shop>:<objectId>`. `shop` is `steam` (objectId is the
//! Steam AppID), `epic`, `custom` (a game the player added, objectId a UUID)
//! or `launchbox` (an emulator ROM).

use crate::leveldb::{self, ReadError};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One game from Hydra's library, as Kryoto uses it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HydraGame {
    /// `<shop>:<objectId>`, Hydra's own key: what a remembered link is kept by.
    pub key: String,
    pub shop: String,
    pub object_id: String,
    pub title: String,
    /// The Steam AppID: the objectId of a Steam game, or the Steam game an Epic
    /// or custom one was linked to in Hydra's catalogue.
    pub steam_appid: Option<String>,
    /// Absolute path of the exe Hydra starts, when it has one.
    pub executable: Option<String>,
    /// Hydra's own counted time (not Steam's or Epic's imported hours), in seconds.
    pub playtime_seconds: u64,
    /// Unix seconds.
    pub last_played: Option<u64>,
    pub added_at: Option<u64>,
    pub launch_options: Option<String>,
    /// Linux: the Proton Hydra runs it with.
    pub proton_path: Option<String>,
    pub cover: Option<String>,
    pub hero: Option<String>,
    pub logo: Option<String>,
    pub icon: Option<String>,
    /// Pictures the player picked in Hydra: web addresses, or files on this PC
    /// (Hydra writes those as `local:<path>`; given here as the plain path).
    pub custom_cover: Option<String>,
    pub custom_hero: Option<String>,
    pub developer: Option<String>,
    pub year: Option<u32>,
    pub favorite: bool,
}

/// Where Hydra keeps `hydra-db` on this PC, most likely first. `home`,
/// `appdata` (Windows' Roaming) and `xdg_config` are passed in so this stays
/// testable; the caller reads them from the environment.
pub fn candidate_dirs(appdata: Option<&Path>, home: Option<&Path>, xdg_config: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        if let Some(a) = appdata {
            out.push(a.join("hydralauncher").join("hydra-db"));
        }
    } else if cfg!(target_os = "macos") {
        if let Some(h) = home {
            out.push(h.join("Library/Application Support/hydralauncher/hydra-db"));
        }
    } else {
        if let Some(x) = xdg_config {
            out.push(x.join("hydralauncher").join("hydra-db"));
        }
        if let Some(h) = home {
            out.push(h.join(".config/hydralauncher/hydra-db"));
            // The Flatpak (Flathub's gg.hydralauncher.hydra) keeps its own config.
            out.push(h.join(".var/app/gg.hydralauncher.hydra/config/hydralauncher/hydra-db"));
        }
    }
    out.dedup();
    out
}

/// What the library holds: the games still in it (removed ones are kept by
/// Hydra with `isDeleted`, and skipped here), newest-played first.
pub fn read_library(db: &Path) -> Result<Vec<HydraGame>, ReadError> {
    let all = leveldb::read_all(db)?;
    Ok(games_from(&all))
}

pub(crate) fn games_from(all: &BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<HydraGame> {
    let json = |k: &[u8]| all.get(k).and_then(|v| serde_json::from_slice::<Value>(v).ok());
    let prefix = b"!games!";
    let mut games = Vec::new();
    for (k, v) in all.range(prefix.to_vec()..) {
        if !k.starts_with(prefix) {
            break;
        }
        let Ok(key) = std::str::from_utf8(&k[prefix.len()..]) else { continue };
        let Ok(game) = serde_json::from_slice::<Value>(v) else { continue };
        if game["isDeleted"].as_bool() == Some(true) {
            continue;
        }
        let assets = json(format!("!gameShopAssets!{key}").as_bytes()).unwrap_or(Value::Null);
        let details = shop_details(all, key);
        if let Some(g) = to_game(key, &game, &assets, details.as_ref()) {
            games.push(g);
        }
    }
    games.sort_by(|a, b| b.last_played.cmp(&a.last_played).then_with(|| a.title.cmp(&b.title)));
    games
}

/// Steam's details Hydra cached for the game (any language), for the
/// developer and release year.
fn shop_details(all: &BTreeMap<Vec<u8>, Vec<u8>>, key: &str) -> Option<Value> {
    let prefix = format!("!gameShopCache!{key}:").into_bytes();
    all.range(prefix.clone()..).take_while(|(k, _)| k.starts_with(&prefix)).find_map(|(_, v)| serde_json::from_slice(v).ok())
}

fn to_game(key: &str, g: &Value, assets: &Value, details: Option<&Value>) -> Option<HydraGame> {
    let (shop, object_id) = key.split_once(':')?;
    let text = |v: &Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let shop = text(&g["shop"]).unwrap_or_else(|| shop.to_string());
    let object_id = text(&g["objectId"]).unwrap_or_else(|| object_id.to_string());
    let title = text(&g["title"]).or_else(|| text(&assets["title"]))?;
    let digits = |s: &str| (!s.is_empty() && s.len() <= 10 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.to_string());
    let steam_appid = if shop == "steam" {
        digits(&object_id)
    } else {
        (g["catalogueGame"]["shop"].as_str() == Some("steam")).then(|| g["catalogueGame"]["objectId"].as_str().and_then(digits)).flatten()
    };
    let web = |v: &Value| text(v).filter(|s| s.starts_with("https://") || s.starts_with("http://"));
    let picked = |v: &Value| {
        text(v).and_then(|s| match s.strip_prefix("local:") {
            Some(path) => Some(decode_local(path)),
            None => (s.starts_with("https://") || s.starts_with("http://")).then_some(s),
        })
    };
    let ms = |v: &Value| v.as_f64().filter(|n| n.is_finite() && *n > 0.0).map(|n| (n / 1000.0) as u64).unwrap_or(0);
    let developer = details.and_then(|d| d["developers"].as_array()?.iter().find_map(text));
    let year = details.and_then(|d| d["release_date"]["date"].as_str()).and_then(year_in);
    Some(HydraGame {
        key: format!("{shop}:{object_id}"),
        steam_appid,
        executable: text(&g["executablePath"]),
        playtime_seconds: ms(&g["playTimeInMilliseconds"]),
        last_played: g["lastTimePlayed"].as_str().and_then(iso_to_unix),
        added_at: g["addedToLibraryAt"].as_str().and_then(iso_to_unix),
        launch_options: text(&g["launchOptions"]),
        proton_path: text(&g["protonPath"]),
        cover: web(&assets["coverImageUrl"]).or_else(|| web(&assets["libraryImageUrl"])),
        hero: web(&g["libraryHeroImageUrl"]).or_else(|| web(&assets["libraryHeroImageUrl"])),
        logo: web(&g["logoImageUrl"]).or_else(|| web(&assets["logoImageUrl"])),
        icon: web(&g["iconUrl"]).or_else(|| web(&assets["iconUrl"])),
        custom_cover: picked(&g["customCoverImageUrl"]).or_else(|| picked(&g["customIconUrl"])),
        custom_hero: picked(&g["customHeroImageUrl"]),
        developer,
        year,
        favorite: g["favorite"].as_bool().unwrap_or(false),
        shop,
        object_id,
        title,
    })
}

/// Hydra's `local:` paths are written as is for its own custom-game art, and
/// percent-encoded for screenshots (`src/shared/local-screenshots.ts`).
fn decode_local(path: &str) -> String {
    if !path.contains('%') {
        return path.to_string();
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| path.to_string())
}

/// The four-digit year in a store date ("21 Aug, 2020", "2020-08-21").
fn year_in(date: &str) -> Option<u32> {
    date.split(|c: char| !c.is_ascii_digit()).filter(|p| p.len() == 4).filter_map(|p| p.parse().ok()).find(|y| (1970..=2100).contains(y))
}

/// `2026-08-21T12:42:10.123Z` (what JSON makes of a JavaScript Date) as Unix seconds.
pub fn iso_to_unix(s: &str) -> Option<u64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if s.get(4..5)? != "-" || s.get(7..8)? != "-" || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let (h, mi, sec) = if s.len() >= 19 { (num(11, 13)?, num(14, 16)?, num(17, 19)?) } else { (0, 0, 0) };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let t = days * 86_400 + h * 3600 + mi * 60 + sec;
    u64::try_from(t).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(entries: &[(&str, Value)]) -> BTreeMap<Vec<u8>, Vec<u8>> {
        entries.iter().map(|(k, v)| (k.as_bytes().to_vec(), serde_json::to_vec(v).unwrap())).collect()
    }

    #[test]
    fn reads_games_as_hydra_writes_them() {
        let all = db(&[
            (
                "!games!steam:1145360",
                serde_json::json!({
                    "title": "Hades", "objectId": "1145360", "shop": "steam", "remoteId": null, "isDeleted": false,
                    "playTimeInMilliseconds": 7_260_500.0, "steamPlayTimeInMilliseconds": 99_000_000,
                    "lastTimePlayed": "2026-08-21T12:42:10.123Z", "executablePath": "D:\\Games\\Hades\\x64\\Hades.exe",
                    "launchOptions": "-dx11", "libraryHeroImageUrl": "https://cdn.example/hero.jpg", "iconUrl": null
                }),
            ),
            ("!gameShopAssets!steam:1145360", serde_json::json!({"coverImageUrl": "https://cdn.example/cover.jpg", "logoImageUrl": ""})),
            ("!gameShopCache!steam:1145360:english", serde_json::json!({"developers": ["Supergiant Games"], "release_date": {"date": "17 Sep, 2020"}})),
            (
                "!games!custom:3b1f",
                serde_json::json!({"title": "My Mod", "shop": "custom", "objectId": "3b1f", "isDeleted": false, "playTimeInMilliseconds": 0,
                  "lastTimePlayed": null, "executablePath": "C:\\Mods\\mod.exe", "customIconUrl": "local:C:\\Users\\x\\AppData\\Roaming\\hydralauncher\\Assets\\custom-games\\a.png"}),
            ),
            ("!games!steam:70", serde_json::json!({"title": "Half-Life", "isDeleted": true, "playTimeInMilliseconds": 1})),
            ("!user!", serde_json::json!({"displayName": "someone"})),
        ]);
        let games = games_from(&all);
        assert_eq!(games.len(), 2, "the removed game is skipped");
        let h = &games[0];
        assert_eq!((h.key.as_str(), h.steam_appid.as_deref(), h.playtime_seconds), ("steam:1145360", Some("1145360"), 7260));
        assert_eq!(h.last_played, Some(1_787_316_130));
        assert_eq!(h.developer.as_deref(), Some("Supergiant Games"));
        assert_eq!(h.year, Some(2020));
        assert_eq!(h.cover.as_deref(), Some("https://cdn.example/cover.jpg"));
        assert_eq!(h.logo, None);
        assert_eq!(h.launch_options.as_deref(), Some("-dx11"));
        let c = &games[1];
        assert_eq!((c.shop.as_str(), c.steam_appid.as_deref()), ("custom", None));
        assert!(c.custom_cover.as_deref().unwrap().ends_with("a.png") && !c.custom_cover.as_deref().unwrap().starts_with("local:"));
    }

    #[test]
    fn a_custom_game_linked_to_steam_keeps_the_appid() {
        let all = db(&[(
            "!games!custom:9",
            serde_json::json!({"title": "Celeste", "shop": "custom", "objectId": "9", "catalogueGame": {"shop": "steam", "objectId": "504230"}}),
        )]);
        assert_eq!(games_from(&all)[0].steam_appid.as_deref(), Some("504230"));
    }

    #[test]
    fn dates() {
        assert_eq!(iso_to_unix("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(iso_to_unix("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(iso_to_unix("garbage"), None);
        assert_eq!(year_in("17 Sep, 2020"), Some(2020));
        assert_eq!(year_in("Coming soon"), None);
        assert_eq!(decode_local("C:/a%20b.png"), "C:/a b.png");
    }

    #[test]
    fn where_hydra_lives() {
        let dirs = candidate_dirs(Some(Path::new("R")), Some(Path::new("H")), Some(Path::new("X")));
        if cfg!(windows) {
            assert_eq!(dirs, vec![PathBuf::from("R").join("hydralauncher").join("hydra-db")]);
        } else if !cfg!(target_os = "macos") {
            assert_eq!(dirs[0], PathBuf::from("X/hydralauncher/hydra-db"));
            assert!(dirs.iter().any(|d| d.to_string_lossy().contains("gg.hydralauncher.hydra")));
        }
    }
}
