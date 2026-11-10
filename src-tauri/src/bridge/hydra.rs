//! Hydra Launcher on Bridge. Its library is read by `kryoto-import`, which
//! never writes a file of Hydra's. Hydra hands its games over: a game Kryoto
//! has becomes a Kryoto game, and its saves follow as versions.

use super::{env_path, BroughtOver, Incoming, Provider, ReadProblem};
use kryoto_import::hydra::{self as hy, HydraGame};
use kryoto_import::leveldb::ReadError;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

pub(super) struct Hydra;

/// Hydra's `hydra-db` on this PC, when there is one.
fn default_dir() -> Option<PathBuf> {
    // Debug builds only: a test library in Hydra's format instead of the PC's own.
    #[cfg(debug_assertions)]
    if let Some(dir) = env_path("KRYOTO_DEV_HYDRA_DIR").filter(|d| d.join("CURRENT").is_file()) {
        return Some(dir);
    }
    let home = env_path("HOME").or_else(|| env_path("USERPROFILE"));
    hy::candidate_dirs(env_path("APPDATA").as_deref(), home.as_deref(), env_path("XDG_CONFIG_HOME").as_deref())
        .into_iter()
        .find(|d| d.join("CURRENT").is_file())
}

impl From<HydraGame> for Incoming {
    fn from(h: HydraGame) -> Incoming {
        Incoming {
            key: h.key,
            title: h.title,
            steam_appid: h.steam_appid,
            executable: h.executable,
            folder: None,
            playtime_seconds: h.playtime_seconds,
            last_played: h.last_played,
            added_at: h.added_at,
            launch_options: h.launch_options,
            proton_path: h.proton_path,
            cover: h.cover,
            hero: h.hero,
            logo: h.logo,
            header: None,
            custom_cover: h.custom_cover,
            custom_hero: h.custom_hero,
            developer: h.developer,
            year: h.year,
        }
    }
}

impl Provider for Hydra {
    fn id(&self) -> &'static str {
        "hydra"
    }

    fn name(&self) -> &'static str {
        "Hydra"
    }

    /// The folder the player picked (Hydra's data folder or `hydra-db` itself),
    /// else Hydra's own.
    fn locate(&self, picked: Option<&str>) -> Option<PathBuf> {
        let Some(dir) = picked.map(str::trim).filter(|d| !d.is_empty()) else { return default_dir() };
        let p = PathBuf::from(dir);
        if p.join("CURRENT").is_file() {
            return Some(p);
        }
        let inner = p.join("hydra-db");
        inner.join("CURRENT").is_file().then_some(inner)
    }

    /// Whether Hydra is open: its window holds the library's files, and what
    /// it writes while open would not be in what is read.
    fn running(&self) -> bool {
        super::process_running(&["hydra.exe", "hydralauncher", "hydralauncher.exe"], |name, exe| {
            (name == "hydra" && (exe.contains("hydra") || exe.is_empty())) || exe.contains("/programs/hydra/hydra.exe") || exe.contains("gg.hydralauncher.hydra")
        })
    }

    fn read(&self, dir: &Path) -> Result<Vec<Incoming>, ReadProblem> {
        match hy::read_library(dir) {
            Ok(games) => Ok(games.into_iter().map(Incoming::from).collect()),
            Err(ReadError::Locked) => Err(ReadProblem::Open),
            Err(ReadError::Missing) => Err(ReadProblem::Other("No Hydra library in that folder.".into())),
            Err(ReadError::Io(e)) => Err(ReadProblem::Other(e)),
        }
    }

    /// Their saves, as versions at once: Hydra's own backup first (only read),
    /// then what is on this PC now. Achievements need nothing: they live in
    /// the emulators' own files, which the next round reads.
    fn brought_over(&self, app: &AppHandle, dir: &Path, games: Vec<BroughtOver>) -> bool {
        if games.is_empty() || !crate::saves::enabled(app) {
            return false;
        }
        let backups = dir.parent().map(|d| d.join("Backups"));
        let jobs: Vec<(String, String, Option<PathBuf>)> = games
            .into_iter()
            .map(|g| {
                let key = g.key.replace(':', "-");
                let backup = backups.as_ref().filter(|_| !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).map(|b| b.join(&key));
                (g.id, g.title, backup)
            })
            .collect();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut kept = Vec::new();
            for (id, title, backup) in jobs {
                match crate::saves::bring_from_hydra(&app, &id, backup).await {
                    Ok(n) if n > 0 => kept.push(title),
                    Ok(_) => {}
                    Err(e) => crate::logging::warn("hydra", &format!("{title}: saves not kept: {e}")),
                }
            }
            if !kept.is_empty() {
                let body = format!("{} now {} save versions here: {}.", kept.len(), if kept.len() == 1 { "game has" } else { "games have" }, kept.join(", "));
                let _ = app.emit("notify", crate::downloads::Notice::new("Saves brought over", &body, None));
            }
        });
        true
    }
}
