//! The same emulator engine as the portable app and Kryoto Forge.
use crate::{
    library::{self, LibraryGame},
    online::LocalOnline,
};
use kryoto_repair::{
    crack::EmuIdentity, detection::GameIdentity, journal, release, repair, settings::Emulator,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Mutex};
use tauri::{AppHandle, Emitter, Manager, Runtime};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalRepair {
    pub source: String,
    pub version: String,
    pub previous_online: Option<LocalOnline>,
    pub previous_source: Option<String>,
    pub previous_overrides: bool,
    /// Put in by the achievements switch on a game published DRM-free, not
    /// chosen in Repair: an update takes it out for the new files and puts it
    /// back after (downloads.rs `install`).
    pub achievements: bool,
}
#[derive(Clone, Default)]
struct Session {
    game: Option<GameIdentity>,
    source: String,
    logs: Vec<String>,
}
#[derive(Default)]
pub struct Sessions(Mutex<HashMap<String, Session>>);
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    game_id: String,
    text: String,
    percent: Option<u8>,
}

fn game<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<LibraryGame, String> {
    library::load(app)?
        .into_iter()
        .find(|g| g.id == id)
        .ok_or("That game is no longer in the library.".into())
}
fn root<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Result<PathBuf, String> {
    let folders = crate::settings::all_folders(&crate::settings::load(app));
    crate::storage::game_folder(&folders, &game.install_dir)
        .unwrap_or_else(|| PathBuf::from(&game.install_dir))
        .canonicalize()
        .map_err(|_| "The installed game folder cannot be found.".into())
}
fn log<R: Runtime>(app: &AppHandle<R>, id: &str, text: &str, percent: Option<u8>) {
    if let Some(sessions) = app.try_state::<Sessions>() {
        if let Ok(mut all) = sessions.0.lock() {
            let session = all.entry(id.into()).or_default();
            repair::add_log(&mut session.logs, text, percent);
        }
    }
}
fn progress<R: Runtime>(app: &AppHandle<R>, id: &str, text: &str, percent: Option<u8>) {
    log(app, id, text, percent);
    let _ = app.emit(
        "repair-progress",
        Progress {
            game_id: id.into(),
            text: text.into(),
            percent,
        },
    );
}
pub fn can_launch(app: &AppHandle, game: &LibraryGame) -> Result<(), String> {
    let root = root(app, game)?;
    journal::check_idle(&root)?;
    // "Keep my files": what is in the folder is what the player wants to run.
    if !game.keep_files && journal::load(&root)?.is_some_and(|r| r.state == "pending") {
        return Err(
            "A repair was interrupted. Undo it under Properties > Repair before starting the game."
                .into(),
        );
    }
    Ok(())
}

#[tauri::command]
pub fn repair_sources() -> Vec<release::Source> {
    release::sources()
}
#[tauri::command]
pub fn repair_state(app: AppHandle, game_id: String) -> Result<serde_json::Value, String> {
    let game = game(&app, &game_id)?;
    let record = journal::load(&root(&app, &game)?)?;
    Ok(
        serde_json::json!({ "undo_available": record.as_ref().is_some_and(|j| j.state != "undone"), "interrupted": record.as_ref().is_some_and(|j| j.state == "pending"), "report": record.as_ref().filter(|j| j.state == "applied").and_then(|j| j.report.as_ref()) }),
    )
}

#[tauri::command]
pub async fn repair_apply(
    app: AppHandle,
    game_id: String,
    source: Emulator,
) -> Result<repair::RepairReport, String> {
    apply(&app, &game_id, source, false).await
}

/// Repair's one way of putting an emulator into an installed game: the Repair
/// panel, Kryoto Online, and the achievements switch (`achievements`).
pub(crate) async fn apply<R: Runtime>(
    app: &AppHandle<R>,
    game_id: &str,
    source: Emulator,
    achievements: bool,
) -> Result<repair::RepairReport, String> {
    let (app, game_id) = (app.clone(), game_id.to_string());
    let before = game(&app, &game_id)?;
    if app
        .state::<library::Running>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .contains_key(&game_id)
    {
        return Err("Close the game before changing its emulator.".into());
    }
    let root = root(&app, &before)?;
    let slug = before
        .slug
        .as_deref()
        .ok_or("Link this game to its Kryoto page first, under Properties > kryo.to.")?;
    app.state::<Sessions>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .insert(
            game_id.clone(),
            Session {
                source: source.label().into(),
                ..Default::default()
            },
        );
    progress(&app, &game_id, "Checking the installed game", None);
    let probe = match repair::probe(&root, Some(slug)).await {
        Ok(probe) => probe,
        Err(error) => {
            progress(&app, &game_id, &error, None);
            return Err(error);
        }
    };
    if let Some(session) = app
        .state::<Sessions>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .get_mut(&game_id)
    {
        session.game = Some(probe.game);
        session
            .logs
            .extend(probe.evidence.into_iter().map(|s| format!("Detected: {s}")));
    }
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("repair-tools");
    let settings = crate::settings::load(&app);
    let mut identity = EmuIdentity::default();
    if !settings.player_name.trim().is_empty() {
        identity.account_name = settings.player_name.clone();
    }
    let report = match repair::apply(
        &root,
        &cache,
        Some(slug),
        source,
        &identity,
        |text, percent| progress(&app, &game_id, text, percent),
    )
    .await
    {
        Ok(report) => report,
        Err(error) => {
            progress(&app, &game_id, &error, None);
            return Err(error);
        }
    };
    log(
        &app,
        &game_id,
        &format!(
            "{} {} · SHA256 {} · {} DLLs · {} interface lists",
            report.source,
            report.version,
            report.archive_sha256,
            report.dlls_replaced,
            report.interfaces_generated
        ),
        None,
    );
    let saved = library::update(&app, |games| {
        let current = games
            .iter_mut()
            .find(|g| g.id == game_id)
            .ok_or("The library game disappeared during repair.")?;
        current.repair = Some(LocalRepair {
            source: report.source.clone(),
            version: report.version.clone(),
            previous_online: before.online.clone(),
            previous_source: before.source.clone(),
            previous_overrides: before.apply_overrides,
            achievements,
        });
        current.source = Some(format!("Steam + {}", report.source));
        current.online = if source == Emulator::Online {
            Some(LocalOnline {
                version: report.version.clone(),
                ..Default::default()
            })
        } else {
            None
        };
        current.apply_overrides = true;
        Ok(())
    });
    if let Err(error) = saved {
        journal::undo(&root)
            .map_err(|undo| format!("Library save failed: {error}. Undo failed: {undo}"))?;
        return Err(format!(
            "Library save failed: {error}. The previous game files were restored."
        ));
    }
    let _ = app.emit("library-changed", ());
    Ok(report)
}

/// One emulator a game can be switched to, and what it is good for.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorChoice {
    pub id: Emulator,
    pub label: &'static str,
    pub online: bool,
    /// One line, in a player's words.
    pub purpose: &'static str,
}

/// A game's emulator: what it uses now, what it came with, and what it can be
/// switched to. Properties > Emulator draws exactly this (EmulatorRow.tsx), so
/// anything else that learns about a game's Steam files (a file check against
/// the build's recorded list, the files an emulator added) belongs in here too.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Emulators {
    /// What the game's Steam files are now: "gbe_fork / Goldberg", "Kryoto
    /// Online", "Steam's own files"; None when they cannot be told.
    pub current: Option<String>,
    /// The emulator Kryoto put in (Repair, Kryoto Online, the achievements
    /// switch), when it did: Undo puts the release's own files back.
    pub switched: Option<String>,
    /// The release's own label from before Kryoto changed it.
    pub original: Option<String>,
    /// What it can be switched to, one press each, the one in use left out.
    pub choices: Vec<EmulatorChoice>,
    /// Kryoto has a copy of the files it replaced.
    pub undo: bool,
    /// A switch stopped halfway: Undo first.
    pub interrupted: bool,
    /// Why nothing is offered, when nothing is.
    pub blocked: Option<String>,
    /// The emulator the release came with, from its file list
    /// (release_files.rs): "gbe_fork (experimental)".
    pub release: Option<String>,
    /// The uploader left the emulator out of this release.
    pub release_off: bool,
    /// Steam files the uploader kept as the game shipped them.
    pub kept_own: Vec<String>,
}

fn purpose(e: Emulator) -> &'static str {
    match e {
        Emulator::GbeFork => "Plays offline, with achievements. Right for almost every game.",
        Emulator::Online => "Plays online with friends through Kryoto Online. Steam has to be running.",
        Emulator::Rune => "Plays offline, with achievements. Try it when gbe_fork does not start the game.",
        Emulator::RuneSteak | Emulator::RuneSteamclient => "Another RUNE setup, for a game the first one does not start.",
        Emulator::Custom => "",
    }
}

/// The switchable emulators for a game, given what was put in (`switched`, a
/// label) and whether it can be switched at all (`blocked`).
fn choices_for(switched: Option<&str>, blocked: bool) -> Vec<EmulatorChoice> {
    if blocked {
        return Vec::new();
    }
    release::sources()
        .into_iter()
        .filter(|s| Some(s.label) != switched)
        .map(|s| EmulatorChoice { id: s.id, label: s.label, online: s.online, purpose: purpose(s.id) })
        .collect()
}

/// The one place that says which emulator a game uses and which it can use.
/// Off the UI thread: it reads the game's Steam files, often on a hard disk.
#[tauri::command(async)]
pub fn game_emulators(app: AppHandle, game_id: String) -> Result<Emulators, String> {
    let game = game(&app, &game_id)?;
    let record = root(&app, &game).ok().map(|r| journal::load(&r)).transpose()?.flatten();
    let (tracking, name) = crate::achievements::tracking_of(&game);
    use crate::achievements::Tracking;
    let current = match tracking {
        Tracking::Fixable => Some("Steam's own files (no emulator)".to_string()),
        Tracking::Absent => None,
        _ => name.filter(|n| n != "Native achievements"),
    };
    let blocked = if game.launcher.is_some() {
        Some("This game belongs to another launcher, which keeps its files as they are.".to_string())
    } else if game.slug.is_none() {
        Some("Link this game to its kryo.to page first, in the kryo.to tab.".to_string())
    } else if tracking == Tracking::Absent {
        Some("This game has no Steam file, so there is no emulator to change.".to_string())
    } else {
        None
    };
    let switched = game.repair.as_ref().map(|r| r.source.clone());
    // What the release's own list says, when one is kept: no question to kryo.to here.
    let list = crate::release_files::cached_list(&app, &game);
    let swapped = root(&app, &game).is_ok_and(|r| crate::release_files::has_swap(&r));
    Ok(Emulators {
        choices: choices_for(switched.as_deref(), blocked.is_some()),
        current,
        original: game.repair.as_ref().and_then(|r| r.previous_source.clone()),
        switched,
        undo: record.as_ref().is_some_and(|j| j.state != "undone") || game.online.is_some() || swapped,
        interrupted: record.as_ref().is_some_and(|j| j.state == "pending"),
        blocked,
        release: list.as_ref().and_then(crate::release_files::FileList::emulator_words),
        release_off: list.as_ref().is_some_and(|l| l.emulator_off),
        kept_own: list.map(|l| l.excluded).unwrap_or_default(),
    })
}

/// Switch a game to another emulator in one go: what Kryoto put in before
/// comes out first (Undo, from its copies), then the new one goes in through
/// the same `apply` as Repair, which copies every file it replaces.
///
/// When the release has a file list (release_files.rs), the release's own
/// emulator comes out first too, by role: the originals back from the backups
/// it ships (checked by hash), what the emulator added set aside, fixes and
/// the game left alone. Back to the release's own files then puts exactly
/// that back. Without a list, as before.
#[tauri::command]
pub async fn repair_switch(app: AppHandle, game_id: String, source: Emulator) -> Result<repair::RepairReport, String> {
    let before = game(&app, &game_id)?;
    if library::is_running(&app, &game_id) {
        return Err("Close the game before changing its emulator.".into());
    }
    let dir = root(&app, &before)?;
    let applied = journal::load(&dir)?.is_some_and(|j| j.state != "undone");
    if applied || crate::release_files::has_swap(&dir) {
        progress(&app, &game_id, "Putting the game's own files back first", None);
        repair_undo(app.clone(), game_id.clone())?;
    } else if before.online.is_some() {
        // Kryoto Online from before it went through Repair.
        progress(&app, &game_id, "Taking Kryoto Online out first", None);
        crate::online::online_undo(app.clone(), game_id.clone())?;
    }
    let cleaned = match crate::release_files::ensure_list(&app, &before, true).await {
        Some(list) => {
            progress(&app, &game_id, "Taking out the emulator the release came with", None);
            let at = dir.clone();
            tauri::async_runtime::spawn_blocking(move || crate::release_files::clean_for_switch(&at, &list)).await.map_err(|e| e.to_string())??
        }
        None => None,
    };
    if let Some((back, aside)) = cleaned {
        log(&app, &game_id, &format!("Release's own emulator out: {back} original file(s) back, {aside} file(s) set aside"), None);
    }
    match apply(&app, &game_id, source, false).await {
        Ok(report) => Ok(report),
        Err(error) if cleaned.is_some() => match crate::release_files::unclean(&dir) {
            Ok(_) => Err(error),
            Err(undo) => Err(format!("{error} Putting the release's own emulator back failed: {undo}")),
        },
        Err(error) => Err(error),
    }
}

#[tauri::command(async)]
pub fn repair_undo(app: AppHandle, game_id: String) -> Result<(), String> {
    let before = game(&app, &game_id)?;
    if app
        .state::<library::Running>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .contains_key(&game_id)
    {
        return Err("Close the game before Undo.".into());
    }
    let dir = root(&app, &before)?;
    // A switch also set the release's own emulator aside (release_files.rs):
    // Repair's files come out first, then those go back where they were.
    let swapped = crate::release_files::has_swap(&dir);
    match journal::undo(&dir) {
        Ok(()) => {}
        Err(_) if swapped && journal::load(&dir)?.is_none() => {}
        Err(error) => return Err(error),
    }
    if swapped {
        crate::release_files::unclean(&dir)?;
    }
    if let Some(previous) = &before.repair {
        library::update(&app, |games| {
            let game = games
                .iter_mut()
                .find(|g| g.id == game_id)
                .ok_or("The library game disappeared.")?;
            game.online = previous.previous_online.clone();
            game.source = previous.previous_source.clone();
            game.apply_overrides = previous.previous_overrides;
            game.repair = None;
            Ok(())
        })?;
    }
    let _ = app.emit("library-changed", ());
    Ok(())
}

#[tauri::command]
pub fn repair_support(app: AppHandle, game_id: String) -> Result<String, String> {
    let game = game(&app, &game_id)?;
    let all = app.state::<Sessions>();
    let sessions = all.0.lock().map_err(|e| e.to_string())?;
    let saved = journal::load(&root(&app, &game)?)?.and_then(|j| j.report);
    let fallback = Session {
        game: saved.as_ref().map(|r| r.game.clone()),
        source: saved.as_ref().map(|r| r.source.clone()).unwrap_or_else(|| game.source.clone().unwrap_or_default()),
        logs: saved.as_ref().map(|r| {
            let mut lines = vec![format!("{} {} · SHA256 {} · {} DLLs · {} interface lists", r.source, r.version, r.archive_sha256, r.dlls_replaced, r.interfaces_generated)];
            lines.extend(r.warnings.clone()); lines
        }).unwrap_or_else(|| vec!["The repair session log is unavailable. Describe what happens when the game starts.".into()]),
    };
    let session = sessions.get(&game_id).unwrap_or(&fallback);
    let text = repair::support_text(session.game.as_ref(), &session.source, &session.logs);
    kryoto_repair::clipboard::copy(&text)?;
    Ok(text)
}
#[tauri::command]
pub fn repair_support_url(app: AppHandle, game_id: String) -> Result<String, String> {
    let game = game(&app, &game_id)?;
    let mut url = url::Url::parse("https://kryo.to/support").map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("new", "1")
        .append_pair("category", "download")
        .append_pair(
            "subject",
            &format!("{} does not work after auto-fixer", game.title),
        );
    if let Some(slug) = &game.slug {
        url.query_pairs_mut().append_pair("game", slug);
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_emulator_in_use_is_not_offered_again_and_each_says_what_it_is_for() {
        let all = choices_for(None, false);
        assert_eq!(all.len(), release::sources().len());
        assert!(all.iter().all(|c| !c.purpose.is_empty()));
        let after = choices_for(Some("RUNE"), false);
        assert!(after.iter().all(|c| c.label != "RUNE"));
        assert!(after.iter().any(|c| c.id == Emulator::GbeFork));
        assert!(after.iter().any(|c| c.online));
    }

    #[test]
    fn a_game_that_cannot_be_switched_offers_nothing() {
        assert!(choices_for(None, true).is_empty());
    }
}
