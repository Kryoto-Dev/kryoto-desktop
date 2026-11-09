//! kryo.to collections in Kryoto Desktop: the player's own and the ones they
//! follow, in the Library's sidebar, installed with one action, and watched.
//!
//! The Store's page reads them with the player's session (`collections` in
//! the page report, kryo.to `/api/account/collections/sync`) and they are kept
//! here, so the sidebar has them before the Store has loaded.
//!
//! Installing one queues every game not on this PC (downloads.rs
//! `enqueue_collection`) and remembers the collection as installed (`tracked`,
//! with the games it held). A later report with a game not in `known` is an
//! update: the shell asks once (src/library/CollectionInstall.tsx), and either
//! answer makes those games known, so the same change is never asked twice.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview};

use crate::library;

/// What the sidebar keeps of an account's collections.
const MAX_COLLECTIONS: usize = 60;
const MAX_GAMES: usize = 300;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Game {
    pub slug: String,
    pub title: String,
    pub cover: Option<String>,
    pub nsfw: bool,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Collection {
    pub id: String,
    pub name: String,
    /// Its page on kryo.to.
    pub href: String,
    /// `owner`, `collaborator`, `following`, or `visitor` for one offered from
    /// its page that the account has no tie to yet.
    pub role: String,
    pub games: Vec<Game>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Tracked {
    /// The games already told about: installed, or answered Later.
    known: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    collections: Vec<Collection>,
    tracked: BTreeMap<String, Tracked>,
}

/// New games in an installed collection.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    id: String,
    name: String,
    href: String,
    added: Vec<Game>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    collections: Vec<Collection>,
    /// The collections installed here, by id.
    installed: Vec<String>,
    updates: Vec<Update>,
}

static STORE: Mutex<Option<Stored>> = Mutex::new(None);

fn file<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("collections.json"))
}

/// Read, change and save the kept state under one lock.
fn with<R: Runtime, T>(app: &AppHandle<R>, change: impl FnOnce(&mut Stored) -> T) -> Result<T, String> {
    let mut slot = STORE.lock().map_err(|_| "collections lock poisoned")?;
    let path = file(app)?;
    let stored = slot.get_or_insert_with(|| std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
    let out = change(stored);
    let bytes = serde_json::to_vec(stored).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(out)
}

fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

fn plain_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.len() <= 160 && slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn plain_text(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

/// What the page sent, reduced to what is safe to keep and draw: only a
/// page on kryo.to itself, and only plain games. Pure, for the tests.
fn clean(mut c: Collection) -> Option<Collection> {
    if !plain_id(&c.id) || !c.href.starts_with("/user/") || c.href.len() > 300 {
        return None;
    }
    c.name = plain_text(&c.name, 120);
    if c.name.is_empty() {
        return None;
    }
    if !matches!(c.role.as_str(), "owner" | "collaborator" | "following" | "visitor") {
        c.role = "visitor".into();
    }
    c.games.retain(|g| plain_slug(&g.slug));
    c.games.truncate(MAX_GAMES);
    for g in &mut c.games {
        g.title = plain_text(&g.title, 200);
        if g.title.is_empty() {
            g.title = g.slug.clone();
        }
        g.cover = g.cover.take().filter(|u| u.len() <= 2000 && (u.starts_with("https://") || u.starts_with('/')));
    }
    Some(c)
}

/// What changed in the installed collections. A game already on this PC is
/// no news. Pure, for the tests.
fn updates(list: &[Collection], tracked: &BTreeMap<String, Tracked>, on_pc: &HashSet<String>) -> Vec<Update> {
    list.iter()
        .filter_map(|c| {
            let t = tracked.get(&c.id)?;
            let added: Vec<Game> = c.games.iter().filter(|g| !t.known.contains(&g.slug) && !on_pc.contains(&g.slug)).cloned().collect();
            (!added.is_empty()).then(|| Update { id: c.id.clone(), name: c.name.clone(), href: c.href.clone(), added })
        })
        .collect()
}

fn on_this_pc<R: Runtime>(app: &AppHandle<R>) -> HashSet<String> {
    library::load(app).unwrap_or_default().into_iter().filter_map(|g| g.slug).collect()
}

fn state<R: Runtime>(app: &AppHandle<R>) -> Result<State, String> {
    let on_pc = on_this_pc(app);
    with(app, |s| State {
        collections: s.collections.clone(),
        installed: s.tracked.keys().cloned().collect(),
        updates: updates(&s.collections, &s.tracked, &on_pc),
    })
}

fn tell<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(now) = state(app) {
        let _ = app.emit_to("main", "collections-state", now);
    }
}

/// `{ collections }` from the Store's page. One offered from its page stays
/// while it is being installed, though no list holds it.
pub fn report<R: Runtime>(app: &AppHandle<R>, value: serde_json::Value) {
    let Some(list) = value.get("collections").cloned().and_then(|v| serde_json::from_value::<Vec<Collection>>(v).ok()) else { return };
    let list: Vec<Collection> = list.into_iter().filter_map(clean).take(MAX_COLLECTIONS).collect();
    let same = with(app, |s| {
        let mut next = list;
        let offered: Vec<Collection> = s.collections.iter().filter(|c| c.role == "visitor" && !next.iter().any(|n| n.id == c.id)).rev().take(5).cloned().collect();
        next.extend(offered);
        let same = s.collections == next;
        s.collections = next;
        same
    });
    if same == Ok(false) {
        tell(app);
    }
}

/// Signed out: the account's lists go. What is installed stays remembered,
/// so signing back in does not ask about the same games again.
pub fn signed_out<R: Runtime>(app: &AppHandle<R>) {
    if with(app, |s| std::mem::take(&mut s.collections).len()).is_ok_and(|n| n > 0) {
        tell(app);
    }
}

#[tauri::command(async)]
pub fn collections_state(app: AppHandle) -> Result<State, String> {
    state(&app)
}

/// Queue `slugs` of a collection, into `folder` (a library folder; none is
/// the default) and remember the collection as installed. A collection the
/// account had no tie to is followed, so its new games are heard about.
#[tauri::command(async)]
pub fn collection_install(app: AppHandle, id: String, slugs: Vec<String>, folder: Option<String>) -> Result<usize, String> {
    let settings = crate::settings::load(&app);
    let library_dir = match folder {
        Some(f) => {
            let known = crate::settings::all_folders(&settings).into_iter().find(|k| crate::settings::same_path(k, &f)).ok_or("That is not one of your library folders.")?;
            (!crate::settings::same_path(&known, &settings.library_dir)).then_some(known)
        }
        None => None,
    };
    let on_pc = on_this_pc(&app);
    let (games, visitor) = with(&app, |s| {
        let c = s.collections.iter().find(|c| c.id == id)?;
        let games: Vec<Game> = c.games.iter().filter(|g| slugs.contains(&g.slug) && !on_pc.contains(&g.slug)).cloned().collect();
        let known = c.games.iter().map(|g| g.slug.clone()).collect();
        let visitor = c.role == "visitor";
        s.tracked.insert(id.clone(), Tracked { known });
        Some((games, visitor))
    })?
    .ok_or("That collection is no longer on your list.")?;
    let queued = crate::downloads::enqueue_collection(&app, &games, library_dir)?;
    crate::logging::info("collections", &format!("collection {id}: {queued} game(s) queued"));
    if visitor {
        if let Some(view) = app.get_webview(crate::STORE) {
            let _ = view.eval(format!("fetch('/api/account/collections/{id}/follow',{{method:'POST',credentials:'include'}}).finally(()=>window.__kryoDesktopRefresh&&window.__kryoDesktopRefresh())"));
        }
    }
    tell(&app);
    Ok(queued)
}

/// Later: those games are known now. The collection still shows what is
/// missing, and nothing asks about them again.
#[tauri::command(async)]
pub fn collection_later(app: AppHandle, id: String) -> Result<(), String> {
    with(&app, |s| {
        let current: Vec<String> = s.collections.iter().find(|c| c.id == id).map(|c| c.games.iter().map(|g| g.slug.clone()).collect()).unwrap_or_default();
        if let Some(t) = s.tracked.get_mut(&id) {
            for slug in current {
                if !t.known.contains(&slug) {
                    t.known.push(slug);
                }
            }
        }
    })?;
    tell(&app);
    Ok(())
}

/// Stop watching a collection: no more questions about it. Its games stay.
#[tauri::command(async)]
pub fn collection_forget(app: AppHandle, id: String) -> Result<(), String> {
    with(&app, |s| s.tracked.remove(&id))?;
    tell(&app);
    Ok(())
}

/// Keep what a page or a command says a collection holds, listed or not.
fn remember<R: Runtime>(app: &AppHandle<R>, c: Collection) -> Result<(), String> {
    with(app, |s| match s.collections.iter_mut().find(|k| k.id == c.id) {
        Some(known) => known.games = c.games,
        None => s.collections.push(c),
    })
}

/// Install from a collection's page in the Store: the page says what it
/// holds (it may be one the account does not follow), and the shell asks.
#[tauri::command]
pub fn store_install_collection(app: AppHandle, webview: Webview, collection: Collection) -> Result<(), String> {
    crate::from_kryoto_store(&app, &webview)?;
    let c = clean(collection).ok_or("Not a kryo.to collection.")?;
    let id = c.id.clone();
    remember(&app, c)?;
    tell(&app);
    let _ = app.emit_to("main", "collection-install", id);
    crate::system::show_main(&app);
    Ok(())
}

/// Sent from a phone or kryo.to: everything missing, into the default folder
/// (nobody may be at this PC). What it holds came with the command.
pub fn install_all(app: &AppHandle, collection: Collection) -> Result<usize, String> {
    let c = clean(collection).ok_or("Not a kryo.to collection.")?;
    let id = c.id.clone();
    let on_pc = on_this_pc(app);
    let slugs: Vec<String> = c.games.iter().filter(|g| !on_pc.contains(&g.slug)).map(|g| g.slug.clone()).collect();
    remember(app, c)?;
    let queued = collection_install(app.clone(), id, slugs, None)?;
    let body = format!("{queued} game{} added to Downloads.", if queued == 1 { "" } else { "s" });
    let _ = app.emit("notify", crate::downloads::Notice::new("Collection sent to this PC", &body, None));
    Ok(queued)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(slug: &str) -> Game {
        Game { slug: slug.into(), title: slug.into(), ..Default::default() }
    }

    fn collection(id: &str, slugs: &[&str]) -> Collection {
        Collection { id: id.into(), name: "List".into(), href: "/user/mira/c/list".into(), role: "following".into(), games: slugs.iter().map(|s| game(s)).collect() }
    }

    #[test]
    fn only_a_plain_collection_is_kept() {
        assert!(clean(collection("12", &["celeste"])).is_some());
        assert!(clean(collection("../1", &[])).is_none());
        assert!(clean(Collection { href: "https://evil.example/x".into(), ..collection("12", &[]) }).is_none());
        assert!(clean(Collection { name: "   ".into(), ..collection("12", &[]) }).is_none());
        // Bad slugs go, a role nobody said becomes the plainest, odd art goes.
        let mut c = collection("12", &["celeste", "../auth"]);
        c.role = "admin".into();
        c.games[0].cover = Some("javascript:alert(1)".into());
        let c = clean(c).unwrap();
        assert_eq!(c.role, "visitor");
        assert_eq!(c.games.len(), 1);
        assert_eq!(c.games[0].cover, None);
    }

    #[test]
    fn a_new_game_in_an_installed_collection_is_an_update_once() {
        let list = vec![collection("1", &["a", "b", "c"]), collection("2", &["x"])];
        let mut tracked = BTreeMap::new();
        tracked.insert("1".to_string(), Tracked { known: vec!["a".into(), "b".into()] });
        let none = HashSet::new();
        // Only the installed one, and only what is new in it.
        let found = updates(&list, &tracked, &none);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].added, vec![game("c")]);
        // Already on this PC: no news.
        assert!(updates(&list, &tracked, &HashSet::from(["c".to_string()])).is_empty());
        // Answered (Later or Install now): known, so never asked again.
        tracked.get_mut("1").unwrap().known.push("c".into());
        assert!(updates(&list, &tracked, &none).is_empty());
        // A game taken out and nothing added says nothing.
        let fewer = vec![collection("1", &["a"])];
        assert!(updates(&fewer, &tracked, &none).is_empty());
    }
}
