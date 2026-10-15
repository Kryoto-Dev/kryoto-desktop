//! Wry attaches Linux download handlers to the shared WebContext. The shell's
//! default decide-destination handler runs first and returns true, so the
//! Store's on_download callback never runs and WebKit saves to Downloads.
//! Take over at download-started instead, before any destination is chosen,
//! and check the owning WebView so the shell and mirror views are untouched.

use gtk::glib::ObjectExt;
use tauri::Manager;
use webkit2gtk::{DownloadExt, URIRequestExt, WebContextExt, WebViewExt};

pub fn install(view: &tauri::Webview) -> Result<(), String> {
    let app = view.app_handle().clone();
    view.with_webview(move |native| {
        let source = native.inner();
        let Some(context) = source.context() else {
            return;
        };
        let source = source.downgrade();
        context.connect_download_started(move |_, download| {
            let Some(source) = source.upgrade() else {
                return;
            };
            if download.web_view().as_ref() != Some(&source) {
                return;
            }
            let Some(mut url) = download
                .request()
                .and_then(|r| r.uri())
                .and_then(|u| u.parse::<url::Url>().ok())
            else {
                return;
            };
            let settings = crate::settings::load(&app);
            if !crate::downloads::is_ours(&url, &settings) {
                return;
            }
            // Cancellation here precedes decide-destination, including Wry's
            // default handler for the shell on this same WebContext.
            download.cancel();
            let page = source.uri().and_then(|u| u.parse::<url::Url>().ok());
            let (slug, title) = crate::download_game(&app, &url, page, &settings);
            url.set_fragment(None);
            crate::logging::info("download", "Store download handed to the installer");
            crate::downloads::enqueue(&app, url.to_string(), slug, title);
        });
    })
    .map_err(|e| e.to_string())
}
