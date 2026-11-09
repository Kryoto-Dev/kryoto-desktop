//! The built-in browser's ad blocker: the Store view refuses requests to ad
//! servers, on any site typed into its address bar.
//!
//! It is the web view's own request filter, never a script in the page: a
//! page script (an extension's, a plugin's) lands in Cloudflare's frame too and
//! every download's check fails (see `no_plugin_touches_the_store_pages`). So
//! it blocks by host only, from `resources/adblock/hosts.txt`, which
//! `scripts/adblock-hosts.mjs` makes from the lists bundled with uBlock
//! Origin. Hiding page elements and the tricks some sites need (in-video ads)
//! take a page script, and are left out on purpose.
//!
//! kryo.to's own pages are never filtered: their ads are Kryoto's, and open in
//! the system browser (`store_mount`'s pop-up handling). Mirror windows on
//! Windows keep the full uBlock in a profile of their own (resolvers/ublock.rs).
//!
//! Windows: WebView2's `WebResourceRequested`, answered 403. Linux: a WebKit
//! content filter, compiled once and kept in the app's cache.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
#[cfg(target_os = "linux")]
use tauri::Manager;
use tauri::Runtime;

const LIST: &str = include_str!("../resources/adblock/hosts.txt");

/// Never blocked, whatever a list says: the checks pages put in front of a
/// download, and the site's own address.
const NEVER: &[&str] = &["challenges.cloudflare.com", "hcaptcha.com", "recaptcha.net", "localhost"];

static ON: AtomicBool = AtomicBool::new(true);
/// The catalog's host when it is not kryo.to (Settings, Developer).
static CATALOG: RwLock<String> = RwLock::new(String::new());

struct Hosts {
    block: HashSet<&'static str>,
    third_party: HashSet<&'static str>,
    allow: HashSet<&'static str>,
}

fn hosts() -> &'static Hosts {
    static HOSTS: OnceLock<Hosts> = OnceLock::new();
    HOSTS.get_or_init(|| {
        let mut h = Hosts { block: HashSet::new(), third_party: HashSet::new(), allow: HashSet::new() };
        for line in LIST.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if let Some(host) = line.strip_prefix("@@") {
                h.allow.insert(host);
            } else if let Some(host) = line.strip_suffix(" 3p") {
                h.third_party.insert(host);
            } else {
                h.block.insert(line);
            }
        }
        h
    })
}

/// The host and each domain above it: `a.b.c.com`, `b.c.com`, `c.com`, `com`.
#[cfg_attr(not(windows), allow(dead_code))]
fn suffixes(host: &str) -> impl Iterator<Item = &str> {
    std::iter::once(host).chain(host.match_indices('.').map(move |(at, _)| &host[at + 1..]))
}

/// The site a host belongs to, near enough: its last two labels, or three
/// under a country's own second level (`bbc.co.uk`).
#[cfg_attr(not(windows), allow(dead_code))]
fn site(host: &str) -> &str {
    let labels: Vec<(usize, &str)> = host.match_indices('.').collect();
    let from = |n: usize| if labels.len() >= n { labels[labels.len() - n].0 + 1 } else { 0 };
    let two = &host[from(2)..];
    let (second, tld) = two.split_once('.').unwrap_or((two, ""));
    if tld.len() == 2 && second.len() <= 3 && labels.len() >= 2 {
        &host[from(3)..]
    } else {
        two
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn ours(host: &str) -> bool {
    host == "kryo.to"
        || host.ends_with(".kryo.to")
        || host == "127.0.0.1"
        || host == "[::1]"
        || CATALOG.read().is_ok_and(|c| !c.is_empty() && *c == host)
}

/// Whether a list blocks requests to `host` from a page on `page`.
#[cfg_attr(not(windows), allow(dead_code))]
fn listed(host: &str, page: &str) -> bool {
    let h = hosts();
    if suffixes(host).any(|s| h.allow.contains(s) || NEVER.contains(&s)) {
        return false;
    }
    if suffixes(host).any(|s| h.block.contains(s)) {
        return true;
    }
    site(host) != site(page) && suffixes(host).any(|s| h.third_party.contains(s))
}

/// Whether the Store refuses a request for `request` from the page at `page`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn refuses(request: &str, page: &str) -> bool {
    if !ON.load(Ordering::Relaxed) || request == page {
        return false;
    }
    let Ok(request) = url::Url::parse(request) else { return false };
    if !matches!(request.scheme(), "http" | "https" | "ws" | "wss") {
        return false;
    }
    let Some(host) = request.host_str() else { return false };
    let page_host = url::Url::parse(page).ok().and_then(|p| p.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
    // kryo.to is never filtered; nor is a site that is itself on a list, which
    // the player went to on purpose (an advertiser's own dashboard).
    if ours(&page_host) || ours(host) || (!page_host.is_empty() && listed(&page_host, &page_host)) {
        return false;
    }
    listed(&host.to_ascii_lowercase(), &page_host)
}

/// Settings, General, "Block ads on other sites".
pub fn set_enabled<R: Runtime>(app: &tauri::AppHandle<R>, on: bool) {
    ON.store(on, Ordering::Relaxed);
    #[cfg(target_os = "linux")]
    if let Some(view) = app.get_webview(crate::STORE) {
        let dir = filter_dir(app);
        let _ = view.with_webview(move |w| linux::set(&w.inner(), on, dir));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}

#[cfg(target_os = "linux")]
fn filter_dir<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<std::path::PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("adblock"))
}

/// A mirror's download window on Linux (resolvers/browser.rs) gets the same
/// filter: the full uBlock there is a WebView2 extension, Windows only.
#[cfg(target_os = "linux")]
pub fn install_mirror<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    let app = window.app_handle();
    let (on, dir) = (crate::settings::load(app).block_ads, filter_dir(app));
    let _ = window.with_webview(move |w| linux::set(&w.inner(), on, dir));
}

/// Put the filter on the Store view, once, as it is made.
pub fn install<R: Runtime>(view: &tauri::Webview<R>, settings: &crate::settings::Settings) {
    ON.store(settings.block_ads, Ordering::Relaxed);
    let catalog = url::Url::parse(&crate::settings::catalog_endpoint(settings)).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    if let Ok(mut c) = CATALOG.write() {
        *c = catalog;
    }
    #[cfg(windows)]
    windows::install(view);
    #[cfg(target_os = "linux")]
    {
        let dir = filter_dir(view.app_handle());
        let on = settings.block_ads;
        let _ = view.with_webview(move |w| linux::set(&w.inner(), on, dir));
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = view;
}

#[cfg(windows)]
mod windows {
    use std::sync::{Arc, Mutex};
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::{NavigationStartingEventHandler, WebResourceRequestedEventHandler};
    use windows::core::{w, Interface, PWSTR};

    pub fn install<R: tauri::Runtime>(view: &tauri::Webview<R>) {
        let result = view.with_webview(|w| unsafe {
            let outcome = (|| -> windows::core::Result<()> {
                let core = w.controller().CoreWebView2()?;
                let environment = core.cast::<ICoreWebView2_2>()?.Environment()?;
                // The page the requests are for: the top frame's address, from
                // the moment it starts loading (redirects included).
                let page = Arc::new(Mutex::new(String::new()));
                let on_nav = page.clone();
                let nav = NavigationStartingEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    if let Ok(mut p) = on_nav.lock() {
                        *p = webview2_com::take_pwstr(uri);
                    }
                    Ok(())
                }));
                let mut token = Default::default();
                core.add_NavigationStarting(&nav, &mut token)?;
                let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut uri = PWSTR::null();
                    args.Request()?.Uri(&mut uri)?;
                    let uri = webview2_com::take_pwstr(uri);
                    let page = page.lock().map(|p| p.clone()).unwrap_or_default();
                    if super::refuses(&uri, &page) {
                        let response = environment.CreateWebResourceResponse(None, 403, w!("Blocked"), w!(""))?;
                        args.SetResponse(&response)?;
                    }
                    Ok(())
                }));
                core.add_WebResourceRequested(&handler, &mut token)?;
                // Every request, from frames and workers too where the
                // runtime can say so; older runtimes hear the page's own.
                match core.cast::<ICoreWebView2_22>() {
                    Ok(core) => core.AddWebResourceRequestedFilterWithRequestSourceKinds(
                        w!("*"),
                        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                        COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
                    ),
                    Err(_) => core.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL),
                }
            })();
            if let Err(error) = outcome {
                crate::logging::warn("adblock", &format!("the Store's request filter is off: {error}"));
            }
        });
        if let Err(error) = result {
            crate::logging::warn("adblock", &format!("the Store's request filter is off: {error}"));
        }
    }
}

/// WebKit's content blocker rules for the list: one rule a host, since its
/// patterns have no alternation. The top-level page is never blocked (the
/// rules leave out `document`), so a page the player opens always opens.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn webkit_rules() -> String {
    let h = hosts();
    // kryo.to's pages, by the address of the page in the tab (its top URL),
    // which is what "on kryo.to" means here (adblock::tests::webkit_filter_live).
    let page = |host: &str| format!("^[a-z]+://([^/]*\\.)?{}[:/]", host.replace('.', "\\."));
    let mut unless: Vec<String> = ["kryo.to", "localhost", "127.0.0.1"].into_iter().map(page).collect();
    if let Ok(c) = CATALOG.read() {
        if !c.is_empty() {
            unless.push(page(&c));
        }
    }
    let types = ["image", "style-sheet", "script", "font", "raw", "svg-document", "media"];
    let filter = |host: &str| format!("^[a-z][a-z0-9.+-]*://([^/]*\\.)?{}[:/]", host.replace('.', "\\."));
    let mut rules = Vec::with_capacity(h.block.len() + h.third_party.len() + h.allow.len() + NEVER.len());
    let sorted = |set: &HashSet<&'static str>| {
        let mut v: Vec<&str> = set.iter().copied().collect();
        v.sort_unstable();
        v
    };
    for host in sorted(&h.block) {
        rules.push(serde_json::json!({ "trigger": { "url-filter": filter(host), "resource-type": types, "unless-top-url": unless }, "action": { "type": "block" } }));
    }
    for host in sorted(&h.third_party) {
        rules.push(serde_json::json!({ "trigger": { "url-filter": filter(host), "resource-type": types, "load-type": ["third-party"], "unless-top-url": unless }, "action": { "type": "block" } }));
    }
    for host in sorted(&h.allow).into_iter().chain(NEVER.iter().copied()) {
        rules.push(serde_json::json!({ "trigger": { "url-filter": filter(host) }, "action": { "type": "ignore-previous-rules" } }));
    }
    serde_json::Value::Array(rules).to_string()
}

#[cfg(target_os = "linux")]
mod linux {
    use gtk::gio;
    use gtk::glib::{self, translate::*};
    use std::cell::RefCell;
    use std::path::PathBuf;
    use webkit2gtk::{ffi, UserContentManagerExt, WebViewExt};

    thread_local! {
        /// The compiled filter, once there is one (GTK's thread only).
        static FILTER: RefCell<Option<*mut ffi::WebKitUserContentFilter>> = const { RefCell::new(None) };
    }

    struct Job {
        manager: webkit2gtk::UserContentManager,
        store: *mut ffi::WebKitUserContentFilterStore,
        id: String,
        rules: Option<glib::Bytes>,
    }

    /// The filter's name in the store: a new list compiles anew.
    fn identifier(rules: &str) -> String {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(rules.as_bytes());
        format!("kryoto-ads-{}", hex::encode(&digest[..8]))
    }

    #[cfg(test)]
    pub fn ready() -> bool {
        FILTER.with(|f| f.borrow().is_some())
    }

    pub fn set(view: &webkit2gtk::WebView, on: bool, dir: Option<PathBuf>) {
        let Some(manager) = view.user_content_manager() else { return };
        manager.remove_all_filters();
        if !on {
            return;
        }
        if let Some(filter) = FILTER.with(|f| *f.borrow()) {
            unsafe { ffi::webkit_user_content_manager_add_filter(manager.to_glib_none().0, filter) };
            return;
        }
        let Some(dir) = dir else { return };
        let _ = std::fs::create_dir_all(&dir);
        let rules = super::webkit_rules();
        let id = identifier(&rules);
        unsafe {
            let store = ffi::webkit_user_content_filter_store_new(dir.to_string_lossy().as_ref().to_glib_none().0);
            let job = Box::new(Job { manager, store, id, rules: Some(glib::Bytes::from_owned(rules.into_bytes())) });
            let id = job.id.clone();
            ffi::webkit_user_content_filter_store_load(store, id.to_glib_none().0, std::ptr::null_mut(), Some(loaded), Box::into_raw(job) as glib::ffi::gpointer);
        }
    }

    fn take_error(error: *mut glib::ffi::GError) -> String {
        if error.is_null() {
            return "unknown error".into();
        }
        let e: glib::Error = unsafe { from_glib_full(error) };
        e.to_string()
    }

    fn apply(job: &Job, filter: *mut ffi::WebKitUserContentFilter) {
        unsafe {
            ffi::webkit_user_content_manager_add_filter(job.manager.to_glib_none().0, filter);
            glib::gobject_ffi::g_object_unref(job.store as *mut _);
        }
        FILTER.with(|f| *f.borrow_mut() = Some(filter));
    }

    unsafe extern "C" fn loaded(source: *mut glib::gobject_ffi::GObject, result: *mut gio::ffi::GAsyncResult, data: glib::ffi::gpointer) {
        let mut job = Box::from_raw(data as *mut Job);
        let mut error = std::ptr::null_mut();
        let filter = ffi::webkit_user_content_filter_store_load_finish(source as *mut _, result, &mut error);
        if !filter.is_null() {
            return apply(&job, filter);
        }
        let _ = take_error(error);
        // Not compiled yet (the first start, or a new list): compile it. Some
        // seconds, off the page's way.
        let Some(rules) = job.rules.take() else { return };
        let id = job.id.clone();
        let store = job.store;
        crate::logging::info("adblock", "compiling the Store's content filter");
        ffi::webkit_user_content_filter_store_save(store, id.to_glib_none().0, rules.to_glib_none().0, std::ptr::null_mut(), Some(saved), Box::into_raw(job) as glib::ffi::gpointer);
    }

    unsafe extern "C" fn saved(source: *mut glib::gobject_ffi::GObject, result: *mut gio::ffi::GAsyncResult, data: glib::ffi::gpointer) {
        let job = Box::from_raw(data as *mut Job);
        let mut error = std::ptr::null_mut();
        let filter = ffi::webkit_user_content_filter_store_save_finish(source as *mut _, result, &mut error);
        if filter.is_null() {
            crate::logging::warn("adblock", &format!("the Store's content filter did not compile: {}", take_error(error)));
            glib::gobject_ffi::g_object_unref(job.store as *mut _);
            return;
        }
        crate::logging::info("adblock", "the Store's content filter is on");
        apply(&job, filter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ad_servers_are_refused_on_other_sites() {
        assert!(refuses("https://securepubads.g.doubleclick.net/tag/js/gpt.js", "https://www.youtube.com/watch?v=x"));
        assert!(refuses("https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js", "https://example.com/"));
        // The page's own files, and the check in front of downloads.
        assert!(!refuses("https://www.youtube.com/s/player/base.js", "https://www.youtube.com/watch?v=x"));
        assert!(!refuses("https://challenges.cloudflare.com/turnstile/v0/api.js", "https://gofile.io/d/abc"));
    }

    #[test]
    fn kryoto_and_its_ads_are_never_filtered() {
        // kryo.to's own ad tag, on kryo.to.
        assert!(!refuses("https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js", "https://kryo.to/game/x"));
        assert!(!refuses("https://d2f9q8zryik75d.cloudfront.net/?rzqfd=1", "https://kryo.to/game/x"));
        assert!(!refuses("https://kryo.to/api/x", "https://example.com/"));
        // A page the player opens is never blocked, even an ad server's.
        assert!(!refuses("https://doubleclick.net/", "https://doubleclick.net/"));
    }

    #[test]
    fn third_party_rules_spare_the_site_itself() {
        let h = hosts();
        let Some(host) = h.third_party.iter().find(|h| !h.contains("kryo")) else { return };
        assert!(listed(host, "example.com"));
        assert!(!listed(host, host));
        assert_eq!(site("a.b.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site("www.youtube.com"), "youtube.com");
    }

    #[test]
    fn the_list_is_bundled() {
        assert!(hosts().block.len() > 10_000);
        let rules: serde_json::Value = serde_json::from_str(&webkit_rules()).unwrap();
        assert!(rules.as_array().unwrap().len() > 10_000);
    }

    /// WebKitGTK itself: the rules compile, ads are refused on another site and
    /// kryo.to is left alone. Needs a display and the network:
    ///   cargo test --lib adblock::tests::webkit_filter_live -- --ignored --nocapture
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn webkit_filter_live() {
        use gtk::glib;
        use webkit2gtk::WebViewExt;
        gtk::init().expect("a display");
        let ctx = glib::MainContext::default();
        let pump = |until: &dyn Fn() -> bool, secs: u64| {
            let started = std::time::Instant::now();
            while !until() && started.elapsed().as_secs() < secs {
                while ctx.iteration(false) {}
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            started.elapsed()
        };
        let manager = webkit2gtk::UserContentManager::new();
        let view = webkit2gtk::WebView::with_user_content_manager(&manager);
        let dir = std::env::temp_dir().join("kryoto-adblock-live");
        linux::set(&view, true, Some(dir));
        let took = pump(&linux::ready, 300);
        assert!(linux::ready(), "the content filter did not compile");
        println!("content filter ready in {:.1} s ({} rules)", took.as_secs_f64(), hosts().block.len() + hosts().third_party.len());
        let page = r#"<script>const r = { top: location.href }; const done = (k, v) => { r[k] = v; document.title = JSON.stringify(r) }</script>
            <script src="https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js" onload="done('ad', 'loaded')" onerror="done('ad', 'refused')"></script>
            <script src="https://cdnjs.cloudflare.com/ajax/libs/jquery/3.7.1/jquery.min.js" onload="done('plain', 'loaded')" onerror="done('plain', 'refused')"></script>"#;
        for (base, ad) in [("https://example.org/", "refused"), ("https://kryo.to/", "loaded")] {
            view.load_html(page, Some(base));
            // This page's own title: the last one's stays until it is replaced.
            let both = || view.title().is_some_and(|t| t.contains(&format!("\"top\":\"{base}\"")) && t.contains("\"ad\"") && t.contains("\"plain\""));
            pump(&both, 30);
            let title = view.title().map(|t| t.to_string()).unwrap_or_default();
            println!("{base}: {title}");
            assert!(title.contains(&format!("\"ad\":\"{ad}\"")), "{base}: {title}");
            assert!(title.contains("\"plain\":\"loaded\""), "{base}: {title}");
        }
    }
}
