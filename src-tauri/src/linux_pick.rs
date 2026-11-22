//! The file and folder picker on Linux (KB-0042).
//!
//! rfd's GTK dialogs run a GTK loop of their own, on a second thread it
//! starts beside the one Tauri's windows live on (rfd `GtkGlobalThread`).
//! GTK is not made for two: on a Steam Deck, picking a folder the player had
//! just made in the dialog hung, and so did the app. Here the picker is made
//! on Tauri's own GTK thread and answered from a signal, so nothing waits on
//! it: the desktop's own chooser through the XDG desktop portal when there is
//! one (what works in Game Mode, in sandboxes and on KDE), else GTK's own.
//! The page also takes a typed path the whole time (Settings > Storage).

use gtk::gio;
use gtk::glib;
use gtk::glib::ToVariant;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::oneshot::Sender;

/// What to ask for.
pub struct Ask {
    pub title: String,
    pub directory: bool,
    /// The folder it opens in.
    pub folder: Option<String>,
    pub extensions: Vec<String>,
    pub filter_name: String,
}

/// The one answer: a path, or None for Cancel, however it ends.
type Answer = Rc<RefCell<Option<Sender<Option<String>>>>>;

fn answer(tx: &Answer, path: Option<String>) {
    if let Some(tx) = tx.borrow_mut().take() {
        let _ = tx.send(path);
    }
}

const PORTAL: &str = "org.freedesktop.portal.Desktop";

/// Open the picker. On Tauri's GTK thread (`run_on_main_thread`); returns at
/// once, and `tx` gets the answer.
pub fn open(parent: Option<gtk::Window>, ask: Ask, tx: Sender<Option<String>>) {
    let tx: Answer = Rc::new(RefCell::new(Some(tx)));
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return gtk_chooser(parent.as_ref(), &ask, tx);
    };
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let token = format!("kryoto{}_{nanos}", std::process::id());
    let sender = conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    // Listened for before asking, so a quick answer is not missed.
    let expected = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let sub = listen(&conn, &expected, tx.clone());

    let options = glib::VariantDict::new(None);
    options.insert_value("handle_token", &token.to_variant());
    options.insert_value("modal", &true.to_variant());
    options.insert_value("directory", &ask.directory.to_variant());
    if let Some(folder) = &ask.folder {
        let mut bytes = folder.as_bytes().to_vec();
        bytes.push(0);
        options.insert_value("current_folder", &glib::Variant::array_from_fixed_array(&bytes));
    }
    if !ask.directory && !ask.extensions.is_empty() {
        let patterns: Vec<(u32, String)> = ask.extensions.iter().map(|e| (0u32, format!("*.{e}"))).collect();
        options.insert_value("filters", &vec![(ask.filter_name.clone(), patterns)].to_variant());
    }
    let params = glib::Variant::tuple_from_iter([String::new().to_variant(), ask.title.to_variant(), options.end()]);
    let conn2 = conn.clone();
    conn.call(
        Some(PORTAL),
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.FileChooser",
        "OpenFile",
        Some(&params),
        glib::VariantTy::new("(o)").ok(),
        gio::DBusCallFlags::NONE,
        10_000,
        gio::Cancellable::NONE,
        move |res| match res {
            Ok(reply) => {
                // An older portal names its own request: listen there instead.
                let handle = reply.child_value(0).str().map(str::to_string).unwrap_or_default();
                if !handle.is_empty() && handle != expected {
                    if let Some(id) = sub.borrow_mut().take() {
                        conn2.signal_unsubscribe(id);
                    }
                    let again = listen(&conn2, &handle, tx);
                    drop(again);
                }
            }
            Err(e) => {
                crate::logging::info("picker", &format!("no file chooser portal ({e}); GTK's own instead"));
                if let Some(id) = sub.borrow_mut().take() {
                    conn2.signal_unsubscribe(id);
                }
                gtk_chooser(parent.as_ref(), &ask, tx);
            }
        },
    );
}

/// Wait for the portal's answer on the request `path`; unsubscribes itself.
fn listen(conn: &gio::DBusConnection, path: &str, tx: Answer) -> Rc<RefCell<Option<gio::SignalSubscriptionId>>> {
    let slot: Rc<RefCell<Option<gio::SignalSubscriptionId>>> = Rc::new(RefCell::new(None));
    let mine = slot.clone();
    let id = conn.signal_subscribe(Some(PORTAL), Some("org.freedesktop.portal.Request"), Some("Response"), Some(path), None, gio::DBusSignalFlags::NONE, move |c, _, _, _, _, params| {
        if let Some(id) = mine.borrow_mut().take() {
            c.signal_unsubscribe(id);
        }
        // 0: picked; 1: cancelled; 2: closed another way.
        let code = params.child_value(0).get::<u32>().unwrap_or(2);
        let results = glib::VariantDict::new(Some(&params.child_value(1)));
        let uri = results.lookup_value("uris", None).and_then(|v| v.get::<Vec<String>>()).and_then(|u| u.into_iter().next());
        let path = if code == 0 { uri.and_then(|u| gio::File::for_uri(&u).path()).map(|p| p.display().to_string()) } else { None };
        answer(&tx, path);
    });
    *slot.borrow_mut() = Some(id);
    slot
}

thread_local! {
    /// Pickers that are up: held until they answer.
    static OPEN: RefCell<Vec<gtk::FileChooserNative>> = const { RefCell::new(Vec::new()) };
}

/// GTK's own chooser, shown without blocking the loop and answered from its signal.
fn gtk_chooser(parent: Option<&gtk::Window>, ask: &Ask, tx: Answer) {
    let action = if ask.directory { gtk::FileChooserAction::SelectFolder } else { gtk::FileChooserAction::Open };
    let chooser = gtk::FileChooserNative::new(Some(&ask.title), parent, action, Some("Choose"), Some("Cancel"));
    chooser.set_modal(true);
    chooser.set_create_folders(true);
    if let Some(folder) = &ask.folder {
        let _ = chooser.set_current_folder(folder);
    }
    if !ask.directory && !ask.extensions.is_empty() {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&ask.filter_name));
        for e in &ask.extensions {
            filter.add_pattern(&format!("*.{e}"));
            filter.add_pattern(&format!("*.{}", e.to_uppercase()));
        }
        chooser.add_filter(&filter);
    }
    chooser.connect_response(move |c, response| {
        let path = if response == gtk::ResponseType::Accept { c.filename().map(|p| p.display().to_string()) } else { None };
        answer(&tx, path);
        OPEN.with(|open| open.borrow_mut().retain(|x| x != c));
    });
    chooser.show();
    OPEN.with(|open| open.borrow_mut().push(chooser));
}
