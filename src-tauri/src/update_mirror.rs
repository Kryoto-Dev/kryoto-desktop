//! Updates when GitHub is out of reach.
//!
//! The updater asks GitHub (`plugins.updater` in tauri.conf.json) with its own
//! client, which has neither of this app's ways past a network that filters:
//! no public-resolver lookup when the network's DNS will not answer (players
//! fixed "can't update" by changing their DNS) and no Encrypted Client Hello.
//! When that check or its download fails, the shell asks here: the same
//! updater, pointed at kryo.to's copy of the release's manifest
//! (`/api/desktop/update/latest.json`, which relays the release's own files),
//! with kryo.to looked up the way every client here looks it up (net.rs
//! `lookup`) and its name hidden where that works (net.rs `ech_for`). The
//! signatures are the release's and are checked against the key built into
//! this app, as for GitHub's copy: nothing about verification changes.
//!
//! The update it finds goes into the same resource table the updater plugin's
//! own `check` uses, so the shell installs it with the plugin's own
//! `downloadAndInstall` (src/lib/updates.ts).

use std::time::Duration;

use serde::Serialize;
use tauri::{Manager, ResourceId, Webview};
use tauri_plugin_updater::UpdaterExt;

/// What the updater plugin's `check` answers, so `new Update(...)` in the shell takes it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    rid: ResourceId,
    current_version: String,
    version: String,
    date: Option<String>,
    body: Option<String>,
    raw_json: serde_json::Value,
}

/// kryo.to's copy of this channel's manifest.
fn mirror_url(endpoint: &str, preview: bool) -> String {
    format!("{}/api/desktop/update/latest.json{}", endpoint.trim_end_matches('/'), if preview { "?channel=preview" } else { "" })
}

/// Check for an update through kryo.to. The shell's own view only.
#[tauri::command]
pub async fn update_check_mirror(webview: Webview) -> Result<Option<Found>, String> {
    if webview.label() != "main" {
        return Err("Only the app itself checks for updates.".into());
    }
    let endpoint = crate::settings::catalog_endpoint(&crate::settings::load(webview.app_handle()));
    let url = mirror_url(&endpoint, crate::channel::PREVIEW);
    let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
    let host = parsed.host_str().unwrap_or_default().to_string();
    let addrs = crate::net::lookup(&host, parsed.port_or_known_default().unwrap_or(443)).await;
    let ech = crate::net::ech_for(&url);
    let updater = webview
        .updater_builder()
        .endpoints(vec![parsed])
        .map_err(|e| e.to_string())?
        .configure_client(move |builder| {
            let builder = if addrs.is_empty() { builder } else { builder.resolve_to_addrs(&host, &addrs) };
            match &ech {
                Some(config) => builder.tls_backend_preconfigured((**config).clone()),
                None => builder,
            }
        })
        .build()
        .map_err(|e| e.to_string())?;
    let update = tokio::time::timeout(Duration::from_secs(25), updater.check())
        .await
        .map_err(|_| "kryo.to did not answer the update check in time.".to_string())?
        .map_err(|e| e.to_string())?;
    let Some(update) = update else { return Ok(None) };
    crate::logging::info("update", &format!("found {} through kryo.to (GitHub could not be reached)", update.version));
    Ok(Some(Found {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        date: update.raw_json["pub_date"].as_str().map(String::from),
        body: update.body.clone(),
        raw_json: update.raw_json.clone(),
        rid: webview.resources_table().add(update),
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn asks_kryotos_copy_of_its_own_channel() {
        assert_eq!(super::mirror_url("https://kryo.to/", false), "https://kryo.to/api/desktop/update/latest.json");
        assert_eq!(super::mirror_url("https://preview.kryo.to", true), "https://preview.kryo.to/api/desktop/update/latest.json?channel=preview");
    }
}
