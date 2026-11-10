//! Steam artwork for the player's own games, asked of Steam directly.
//!
//! A game kryo.to does not list stays on the PC: its name and AppID must not
//! reach Kryoto's servers, so the artwork search and lookup for such games
//! (Add a game, a game page with no art, the Artwork editor, games brought
//! over from Hydra) go straight to Steam's public store API from here,
//! instead of through kryo.to's `/api/steam/art` proxy.

use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtMatch {
    pub appid: String,
    pub title: String,
    pub cover: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SteamArt {
    pub capsule: Option<String>,
    pub hero: Option<String>,
    pub header: Option<String>,
    pub logo: Option<String>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("KryotoDesktop/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())
}

/// Steam's own image hosts only: anything else in an answer is dropped.
fn steam_cdn(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    let host = u.host_str().unwrap_or_default();
    u.scheme() == "https" && (host.ends_with(".steamstatic.com") || host == "steamcdn-a.akamaihd.net" || host.ends_with(".steampowered.com"))
}

/// Up to six Steam games for a name, as kryo.to's search answered before.
#[tauri::command]
pub async fn steam_art_search(q: String) -> Result<Vec<ArtMatch>, String> {
    let q = q.trim();
    if q.chars().count() < 2 || q.chars().count() > 120 {
        return Ok(Vec::new());
    }
    let term: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
    let res = client()?
        .get(format!("https://store.steampowered.com/api/storesearch/?term={term}&l=english&cc=US"))
        .send()
        .await
        .map_err(|_| "Steam artwork search is unavailable. You can still choose your own pictures.".to_string())?;
    let json: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(search_results(&json))
}

fn search_results(json: &serde_json::Value) -> Vec<ArtMatch> {
    json["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let appid = match &item["id"] {
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::String(s) => s.clone(),
                        _ => return None,
                    };
                    let ok = !appid.is_empty() && appid.len() <= 10 && appid.bytes().all(|b| b.is_ascii_digit()) && !appid.starts_with('0');
                    let title = item["name"].as_str()?.to_string();
                    let cover = item["tiny_image"].as_str().filter(|u| steam_cdn(u)).map(String::from);
                    ok.then_some(ArtMatch { appid, title, cover })
                })
                .take(6)
                .collect()
        })
        .unwrap_or_default()
}

/// The art Steam really has for an app (its hashed asset paths), from
/// `IStoreBrowseService/GetItems`, which needs no key.
#[tauri::command]
pub async fn steam_art(appid: String) -> Result<SteamArt, String> {
    let id: String = appid.trim().chars().take_while(char::is_ascii_digit).collect();
    if id.is_empty() || id.len() > 10 {
        return Err("Not a Steam app.".into());
    }
    let input = serde_json::json!({
        "ids": [{ "appid": id.parse::<u64>().map_err(|e| e.to_string())? }],
        "context": { "language": "english", "country_code": "US", "steam_realm": 1 },
        "data_request": { "include_assets": true },
    });
    let input: String = url::form_urlencoded::byte_serialize(input.to_string().as_bytes()).collect();
    let res = client()?
        .get(format!("https://api.steampowered.com/IStoreBrowseService/GetItems/v1?input_json={input}"))
        .send()
        .await
        .map_err(|_| "Artwork is unavailable for this game. You can choose your own pictures.".to_string())?;
    let json: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    let art = art_from_assets(&json["response"]["store_items"][0]["assets"]);
    if art == SteamArt::default() {
        return Err("Artwork is unavailable for this game. You can choose your own pictures.".into());
    }
    Ok(art)
}

/// The art for many apps at once (a whole Steam library coming over), fifty to
/// an ask. An app Steam has nothing for, or an ask that fails, is left out.
pub(crate) async fn many(appids: &[String]) -> HashMap<String, SteamArt> {
    let mut out = HashMap::new();
    let Ok(client) = client() else { return out };
    let ids: Vec<u64> = appids.iter().filter_map(|a| a.parse().ok()).collect();
    for part in ids.chunks(50) {
        let items: Vec<_> = part.iter().map(|id| serde_json::json!({ "appid": id })).collect();
        let input = serde_json::json!({
            "ids": items,
            "context": { "language": "english", "country_code": "US", "steam_realm": 1 },
            "data_request": { "include_assets": true },
        });
        let input: String = url::form_urlencoded::byte_serialize(input.to_string().as_bytes()).collect();
        let Ok(res) = client.get(format!("https://api.steampowered.com/IStoreBrowseService/GetItems/v1?input_json={input}")).send().await else { continue };
        let Ok(json) = res.json::<serde_json::Value>().await else { continue };
        out.extend(art_of_items(&json));
    }
    out
}

fn art_of_items(json: &serde_json::Value) -> HashMap<String, SteamArt> {
    let items = json["response"]["store_items"].as_array().map(Vec::as_slice).unwrap_or_default();
    items
        .iter()
        .filter_map(|item| {
            let art = art_from_assets(&item["assets"]);
            let id = item["appid"].as_u64()?.to_string();
            (art != SteamArt::default()).then_some((id, art))
        })
        .collect()
}

/// The URLs in a GetItems `assets` object (kryo.to's `artFromAssets`).
fn art_from_assets(assets: &serde_json::Value) -> SteamArt {
    const BASE: &str = "https://shared.akamai.steamstatic.com/store_item_assets/";
    let Some(format) = assets["asset_url_format"].as_str().filter(|f| f.contains("${FILENAME}")) else { return SteamArt::default() };
    let url = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| assets[*k].as_str().filter(|f| !f.is_empty()))
            .map(|file| format!("{BASE}{}", format.replace("${FILENAME}", file)))
    };
    SteamArt {
        capsule: url(&["library_capsule_2x", "library_capsule"]),
        hero: url(&["library_hero_2x", "library_hero"]),
        header: url(&["header", "header_2x"]),
        logo: url(&["library_logo_2x", "library_logo"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_keeps_steam_games_and_steam_pictures_only() {
        let json = serde_json::json!({"items": [
            {"id": 1145360, "name": "Hades", "tiny_image": "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/1145360/capsule_231x87.jpg"},
            {"id": "x1", "name": "Bad id"},
            {"id": 42, "name": "Elsewhere", "tiny_image": "https://evil.example/a.jpg"},
        ]});
        assert_eq!(
            search_results(&json),
            vec![
                ArtMatch { appid: "1145360".into(), title: "Hades".into(), cover: Some("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/1145360/capsule_231x87.jpg".into()) },
                ArtMatch { appid: "42".into(), title: "Elsewhere".into(), cover: None },
            ]
        );
    }

    #[test]
    fn many_apps_come_back_by_appid() {
        let json = serde_json::json!({"response": {"store_items": [
            {"appid": 1145360, "assets": {"asset_url_format": "steam/apps/1145360/${FILENAME}", "library_capsule": "c.jpg"}},
            {"appid": 42, "assets": {}},
            {"assets": {"asset_url_format": "x/${FILENAME}", "header": "h.jpg"}},
        ]}});
        let art = art_of_items(&json);
        assert_eq!(art.len(), 1);
        assert_eq!(art["1145360"].capsule.as_deref(), Some("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/1145360/c.jpg"));
    }

    #[test]
    fn assets_become_hashed_urls() {
        let assets = serde_json::json!({"asset_url_format": "steam/apps/1145360/${FILENAME}?t=1", "library_hero": "abc/library_hero.jpg", "header": "header.jpg"});
        let art = art_from_assets(&assets);
        assert_eq!(art.hero.as_deref(), Some("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/1145360/abc/library_hero.jpg?t=1"));
        assert_eq!(art.capsule, None);
        assert_eq!(art_from_assets(&serde_json::json!({})), SteamArt::default());
    }
}
