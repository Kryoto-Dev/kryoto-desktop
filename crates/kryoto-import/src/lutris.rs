//! Lutris (github.com/lutris/lutris), Linux. Read, and written only when the
//! player sends games to it.
//!
//! Where it keeps things (`lutris/settings.py`): the library is the SQLite
//! file `pga.db` in `$XDG_DATA_HOME/lutris` (`~/.local/share/lutris`); each
//! installed game's settings are a YAML file `games/<configpath>.yml` in the
//! config folder, which is `~/.config/lutris` when that exists and the data
//! folder otherwise (Lutris is moving everything there); covers are
//! `coverart/<slug>.jpg` in the data folder. The Flatpak keeps the same under
//! `~/.var/app/net.lutris.Lutris/{data,config}/lutris`.
//!
//! A `games` row (`lutris/database/schema.py`) has `id`, `name`, `slug`,
//! `runner`, `directory`, `installed`, `lastplayed` (Unix seconds),
//! `playtime` (hours, a real number), `configpath`. A game's YAML has a
//! `game` section (`exe`, `args`, `prefix`, `working_dir`), one named after
//! its runner (`wine: version: ...`) and `system` (`env`).
//!
//! Sending a game writes what Lutris's own "Add locally installed game"
//! writes (`Game.save`, `games_db.add_game`, `config.write_game_config`): a
//! new YAML file and a new row. Lutris has no command that adds an installed
//! game without its installer window (`lutris -i` runs an install script,
//! `--import` unpacks an archive into a new folder), so the files are written
//! here, carefully: never while Lutris has the library open or left a
//! journal behind, the library copied first, one transaction, no other row
//! touched, and everything written undone if any part fails.

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value as Yaml};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One game from Lutris, as Kryoto uses it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LutrisGame {
    pub id: i64,
    pub slug: String,
    pub name: String,
    /// `wine`, `linux`, `steam`, `dosbox`...
    pub runner: String,
    pub directory: Option<String>,
    pub installed: bool,
    pub playtime_seconds: u64,
    /// Unix seconds.
    pub last_played: Option<u64>,
    pub installed_at: Option<u64>,
    /// Absolute path of the program it starts.
    pub exe: Option<String>,
    pub args: Option<String>,
    /// Wine's prefix, for a Windows game.
    pub prefix: Option<String>,
    /// The Wine build Lutris runs it with (`lutris-GE-Proton8-26`, `system`).
    pub wine_version: Option<String>,
    /// Its own environment variables.
    pub env: Vec<(String, String)>,
    /// Its cover, a file on this PC.
    pub cover: Option<String>,
    /// A game Kryoto sent to Lutris: it starts Kryoto, so it is already here.
    pub from_kryoto: bool,
}

/// Lutris's data folders on this PC, most likely first: the usual one, then
/// the Flatpak's.
pub fn candidate_data_dirs(home: Option<&Path>, xdg_data: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(x) = xdg_data {
        out.push(x.join("lutris"));
    }
    if let Some(h) = home {
        out.push(h.join(".local/share/lutris"));
        out.push(h.join(".var/app/net.lutris.Lutris/data/lutris"));
    }
    out.dedup();
    out
}

/// The data folder in what the player picked: the folder with `pga.db`, a
/// Flatpak's app folder, or a home folder.
pub fn data_in(picked: &Path) -> Option<PathBuf> {
    [picked.to_path_buf(), picked.join("data/lutris"), picked.join(".local/share/lutris"), picked.join("lutris")].into_iter().find(|d| d.join("pga.db").is_file())
}

/// Where the games' YAML files are, for a data folder: Lutris's own rule,
/// the config folder beside it when it exists, else the data folder itself.
pub fn config_for(data: &Path, xdg_config: Option<&Path>) -> PathBuf {
    let parent = data.parent();
    let beside = match parent.and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()) {
        // ~/.local/share/lutris: ~/.config/lutris (or $XDG_CONFIG_HOME/lutris).
        Some(n) if n == "share" => xdg_config.map(|x| x.join("lutris")).or_else(|| parent.and_then(Path::parent).and_then(Path::parent).map(|h| h.join(".config/lutris"))),
        // The Flatpak: .../net.lutris.Lutris/data/lutris -> .../config/lutris.
        Some(n) if n == "data" => parent.and_then(Path::parent).map(|a| a.join("config/lutris")),
        _ => None,
    };
    beside.filter(|c| c.is_dir()).unwrap_or_else(|| data.to_path_buf())
}

/// The Wine program for a Lutris Wine version, when Lutris has it.
pub fn wine_binary(data: &Path, version: &str) -> Option<PathBuf> {
    let v = version.trim();
    if v.is_empty() || v == "system" || v.contains('/') || v.contains("..") {
        return None;
    }
    let p = data.join("runners/wine").join(v).join("bin/wine");
    p.is_file().then_some(p)
}

fn open_read(db: &Path) -> Result<Connection, String> {
    if !db.is_file() {
        return Err("No Lutris library in that folder.".into());
    }
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| format!("Lutris's library could not be opened: {e}"))?;
    let _ = conn.busy_timeout(Duration::from_secs(3));
    Ok(conn)
}

fn text(v: ValueRef) -> Option<String> {
    match v {
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()).filter(|s| !s.trim().is_empty()),
        ValueRef::Integer(n) => Some(n.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        _ => None,
    }
}

fn number(v: ValueRef) -> Option<f64> {
    let n: Option<f64> = match v {
        ValueRef::Integer(n) => Some(n as f64),
        ValueRef::Real(f) => Some(f),
        ValueRef::Text(t) => std::str::from_utf8(t).ok()?.trim().parse().ok(),
        _ => None,
    };
    n.filter(|f| f.is_finite())
}

/// Every game in Lutris's library. Only reads: the database is opened read
/// only, and the YAML files are just read.
pub fn read_library(data: &Path, config: &Path, home: Option<&Path>) -> Result<Vec<LutrisGame>, String> {
    let conn = open_read(&data.join("pga.db"))?;
    let mut stmt = conn.prepare("SELECT * FROM games").map_err(|e| format!("Lutris's library could not be read: {e}"))?;
    let cols: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let at = |name: &str| cols.iter().position(|c| c == name);
    let (id, name, slug, runner, directory, installed, lastplayed, playtime, configpath, installed_at) =
        (at("id"), at("name"), at("slug"), at("runner"), at("directory"), at("installed"), at("lastplayed"), at("playtime"), at("configpath"), at("installed_at"));
    let mut rows = stmt.query([]).map_err(|e| format!("Lutris's library could not be read: {e}"))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| format!("Lutris's library could not be read: {e}"))? {
        let get = |i: Option<usize>| i.and_then(|i| row.get_ref(i).ok());
        let s = |i: Option<usize>| get(i).and_then(text);
        let n = |i: Option<usize>| get(i).and_then(number);
        let Some(title) = s(name) else { continue };
        let mut g = LutrisGame {
            id: n(id).map(|f| f as i64).unwrap_or(0),
            slug: s(slug).unwrap_or_default(),
            name: title.trim().to_string(),
            runner: s(runner).unwrap_or_default(),
            directory: s(directory).map(|d| home_path(&d, home)),
            installed: n(installed).is_some_and(|f| f != 0.0),
            playtime_seconds: n(playtime).filter(|h| *h > 0.0).map(|h| (h * 3600.0).round() as u64).unwrap_or(0),
            last_played: n(lastplayed).filter(|t| *t > 0.0).map(|t| t as u64),
            installed_at: n(installed_at).filter(|t| *t > 0.0).map(|t| t as u64),
            ..Default::default()
        };
        if let Some(cp) = s(configpath).filter(|c| safe_name(c)) {
            if let Ok(raw) = std::fs::read_to_string(config.join("games").join(format!("{cp}.yml"))) {
                settings(&mut g, &raw, home);
            }
        }
        g.cover = ["jpg", "png"].iter().map(|e| data.join("coverart").join(format!("{}.{e}", g.slug))).find(|p| !g.slug.is_empty() && safe_name(&g.slug) && p.is_file()).map(|p| p.to_string_lossy().into_owned());
        out.push(g);
    }
    Ok(out)
}

/// A file name Lutris made, never a path.
fn safe_name(s: &str) -> bool {
    !s.is_empty() && !s.contains('/') && !s.contains('\\') && s != "." && s != ".."
}

fn home_path(p: &str, home: Option<&Path>) -> String {
    match (p.strip_prefix("~/"), home) {
        (Some(rest), Some(h)) => h.join(rest).to_string_lossy().into_owned(),
        _ => p.to_string(),
    }
}

fn scalar(v: &Yaml) -> Option<String> {
    let s = match v {
        Yaml::String(s) => Some(s.clone()),
        Yaml::Number(n) => Some(n.to_string()),
        Yaml::Bool(b) => Some(b.to_string()),
        _ => None,
    };
    s.filter(|s| !s.trim().is_empty())
}

/// What a game's YAML adds: its program, arguments, prefix, Wine and
/// environment. A file that does not parse adds nothing.
fn settings(g: &mut LutrisGame, raw: &str, home: Option<&Path>) {
    let Ok(doc) = serde_yaml::from_str::<Yaml>(raw) else { return };
    let section = |name: &str| doc.get(name).filter(|v| v.is_mapping());
    if let Some(game) = section("game") {
        let field = |k: &str| game.get(k).and_then(scalar);
        g.args = field("args");
        g.prefix = field("prefix").map(|p| home_path(&p, home));
        let working = field("working_dir").map(|w| home_path(&w, home));
        g.exe = field("exe").map(|e| home_path(&e, home)).map(|e| {
            // Lutris's paths are Linux paths, whatever reads them.
            if e.starts_with('/') || Path::new(&e).is_absolute() {
                e
            } else {
                // Lutris joins a relative exe onto the game's folder; the
                // working folder is only where it starts, so it is the base
                // only for a game with no folder.
                match g.directory.as_deref().or(working.as_deref()) {
                    Some(base) => Path::new(base).join(&e).to_string_lossy().into_owned(),
                    None => e,
                }
            }
        });
        g.from_kryoto = g.args.as_deref().is_some_and(|a| a.contains("kryoto://library/"));
    }
    if !g.runner.is_empty() {
        g.wine_version = section(&g.runner).and_then(|r| r.get("version")).and_then(scalar);
    }
    if let Some(Yaml::Mapping(env)) = section("system").and_then(|s| s.get("env")) {
        g.env = env.iter().filter_map(|(k, v)| Some((scalar(k)?, scalar(v).unwrap_or_default()))).filter(|(k, _)| is_env_name(k)).collect();
    }
}

fn is_env_name(k: &str) -> bool {
    let mut c = k.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/* ── Sending games to Lutris ───────────────────────────────────────── */

/// A game going to Lutris.
#[derive(Debug, Clone, Default)]
pub struct NewGame {
    /// Kryoto's id for it: what the ledger keeps it by, and the slug's start.
    pub key: String,
    pub name: String,
    /// The program Lutris starts (Kryoto itself), and its arguments.
    pub exe: String,
    pub args: String,
    /// The game's own folder, for Lutris's "Browse files".
    pub directory: String,
    pub playtime_seconds: u64,
    pub last_played: Option<u64>,
    /// A cover picture's bytes.
    pub cover: Option<Vec<u8>>,
}

/// One game Kryoto added to Lutris: enough to update it or take it back out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Added {
    pub key: String,
    pub id: i64,
    pub slug: String,
    pub configpath: String,
    /// Files written for it (its YAML, its cover).
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Written {
    /// Every game of this send, new or updated, for the ledger.
    pub games: Vec<Added>,
    /// The ones this send put in as new rows: what to take back if the
    /// ledger cannot be written. A game sent before whose row the player
    /// removed is one of them, under the key the ledger already has.
    pub inserted: Vec<Added>,
    pub added: usize,
    pub updated: usize,
    pub pictures: usize,
    /// The copy of `pga.db` from before.
    pub backup: Option<PathBuf>,
}

const NEEDED: &[&str] = &["name", "slug", "runner", "installed", "configpath"];

/// Lutris's library, opened for writing, or the reason not to.
fn open_write(data: &Path) -> Result<Connection, String> {
    let db = data.join("pga.db");
    if !db.is_file() {
        return Err("No Lutris library in that folder.".into());
    }
    // A journal beside it: Lutris stopped in the middle of a change.
    if data.join("pga.db-journal").exists() || data.join("pga.db-wal").exists() {
        return Err("Lutris did not close cleanly last time. Open Lutris once and close it, then try again.".into());
    }
    let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| format!("Lutris's library could not be opened: {e}"))?;
    let _ = conn.busy_timeout(Duration::from_secs(2));
    Ok(conn)
}

fn columns(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn.prepare("PRAGMA table_info(games)").map_err(|e| e.to_string())?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1)).map_err(|e| e.to_string())?.filter_map(Result::ok).collect::<Vec<_>>();
    if NEEDED.iter().any(|n| !names.iter().any(|c| c == n)) {
        return Err("This Lutris library is laid out in a way Kryoto does not know, so nothing was changed.".into());
    }
    Ok(names)
}

fn back_up(data: &Path, backups: &Path, now: u64) -> Result<PathBuf, String> {
    std::fs::create_dir_all(backups).map_err(|e| e.to_string())?;
    let copy = backups.join(format!("lutris-pga-{now}.db"));
    std::fs::copy(data.join("pga.db"), &copy).map_err(|e| format!("Could not copy Lutris's library first, so nothing was changed: {e}"))?;
    Ok(copy)
}

/// A slug no other game in Lutris has, so its cover and settings are its own.
fn free_slug(conn: &Connection, key: &str, mine: &[String]) -> Result<String, String> {
    let base: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let base = base.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let base = if base.is_empty() { "game".to_string() } else { base };
    let taken = |s: &str| -> Result<bool, String> {
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM games WHERE slug = ?1", [s], |r| r.get(0)).map_err(|e| e.to_string())?;
        Ok(n > 0 || mine.iter().any(|m| m == s))
    };
    let mut slug = base.clone();
    let mut n = 1;
    while taken(&slug)? {
        slug = if n == 1 { format!("{base}-kryoto") } else { format!("{base}-kryoto-{n}") };
        n += 1;
    }
    Ok(slug)
}

/// The `game` keys Kryoto writes: what Lutris starts, with what, and where.
/// None for one it leaves out.
fn own_keys(g: &NewGame) -> [(&'static str, Option<String>); 3] {
    [
        ("exe", Some(g.exe.clone())),
        ("args", Some(g.args.clone()).filter(|a| !a.is_empty())),
        ("working_dir", Path::new(&g.exe).parent().map(|d| d.to_string_lossy().into_owned())),
    ]
}

/// A Lutris game's YAML for a program Lutris starts as it is.
pub fn game_yaml(g: &NewGame) -> String {
    let mut game = Mapping::new();
    for (k, v) in own_keys(g) {
        if let Some(v) = v {
            game.insert(k.into(), v.into());
        }
    }
    let mut system = Mapping::new();
    // Lutris's own libraries are for games, not for a WebKit app.
    system.insert("disable_runtime".into(), true.into());
    let mut doc = Mapping::new();
    doc.insert("game".into(), Yaml::Mapping(game));
    doc.insert("linux".into(), Yaml::Mapping(Mapping::new()));
    doc.insert("system".into(), Yaml::Mapping(system));
    serde_yaml::to_string(&Yaml::Mapping(doc)).unwrap_or_default()
}

/// The YAML for a game sent before, from the file Lutris has: only Kryoto's
/// own keys change, so what the player set in Lutris's Configure (Wine,
/// environment, gamescope) stays. None when the file is right already. A file
/// that is missing or does not parse gets the whole YAML: Lutris could not
/// start the game from it either.
fn updated_yaml(old: &str, g: &NewGame) -> Option<String> {
    let Ok(Yaml::Mapping(mut doc)) = serde_yaml::from_str::<Yaml>(old) else { return Some(game_yaml(g)) };
    let before = doc.clone();
    if !doc.get("game").is_some_and(Yaml::is_mapping) {
        doc.insert("game".into(), Yaml::Mapping(Mapping::new()));
    }
    let Some(Yaml::Mapping(game)) = doc.get_mut("game") else { return Some(game_yaml(g)) };
    for (k, v) in own_keys(g) {
        match v {
            Some(v) => {
                game.insert(k.into(), v.into());
            }
            None => {
                game.remove(k);
            }
        }
    }
    (doc != before).then(|| serde_yaml::to_string(&Yaml::Mapping(doc)).unwrap_or_default())
}

/// What this run wrote, so a failure takes it all back.
#[derive(Default)]
struct Undo {
    created: Vec<PathBuf>,
    replaced: Vec<(PathBuf, Vec<u8>)>,
}

impl Undo {
    fn write(&mut self, file: &Path, bytes: &[u8]) -> Result<(), String> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        match std::fs::read(file) {
            Ok(old) => self.replaced.push((file.to_path_buf(), old)),
            Err(_) => self.created.push(file.to_path_buf()),
        }
        let tmp = file.with_extension("kryoto-tmp");
        std::fs::write(&tmp, bytes).map_err(|e| format!("Could not write {}: {e}", file.display()))?;
        std::fs::rename(&tmp, file).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Could not write {}: {e}", file.display())
        })
    }

    fn roll_back(self) {
        for f in self.created {
            let _ = std::fs::remove_file(f);
        }
        for (f, old) in self.replaced {
            let _ = std::fs::write(f, old);
        }
    }
}

/// Add these games to Lutris, or update the ones sent before (`earlier`,
/// from the ledger) whose row is still there. All or nothing.
pub fn send(data: &Path, config: &Path, games: &[NewGame], earlier: &[Added], backups: &Path, now: u64) -> Result<Written, String> {
    let mut conn = open_write(data)?;
    let cols = columns(&conn)?;
    let backup = back_up(data, backups, now)?;
    let mut undo = Undo::default();
    match write_all(&mut conn, &cols, data, config, games, earlier, now, &mut undo) {
        Ok(mut out) => {
            out.backup = Some(backup);
            Ok(out)
        }
        Err(e) => {
            undo.roll_back();
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_all(conn: &mut Connection, cols: &[String], data: &Path, config: &Path, games: &[NewGame], earlier: &[Added], now: u64, undo: &mut Undo) -> Result<Written, String> {
    let busy = |e: rusqlite::Error| {
        if matches!(e.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)) {
            "Lutris is using its library. Close Lutris and try again.".to_string()
        } else {
            format!("Lutris's library could not be changed, so nothing was: {e}")
        }
    };
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(busy)?;
    let has = |c: &str| cols.iter().any(|x| x == c);
    let mut out = Written::default();
    let mut slugs: Vec<String> = Vec::new();
    for g in games {
        // Sent before and still in Lutris: updated in place.
        let before = earlier.iter().find(|a| a.key == g.key).filter(|a| {
            tx.query_row("SELECT COUNT(*) FROM games WHERE id = ?1 AND slug = ?2 AND configpath = ?3", rusqlite::params![a.id, a.slug, a.configpath], |r| r.get::<_, i64>(0)).unwrap_or(0) == 1
        });
        let (slug, configpath) = match before {
            Some(a) => (a.slug.clone(), a.configpath.clone()),
            None => {
                let slug = free_slug(&tx, &g.key, &slugs)?;
                let mut cp = format!("{slug}-{now}");
                let mut n = 2;
                while config.join("games").join(format!("{cp}.yml")).exists() {
                    cp = format!("{slug}-{}", now + n);
                    n += 1;
                }
                (slug, cp)
            }
        };
        slugs.push(slug.clone());
        let yml = config.join("games").join(format!("{configpath}.yml"));
        let text = match before {
            Some(_) => updated_yaml(&std::fs::read_to_string(&yml).unwrap_or_default(), g),
            None => Some(game_yaml(g)),
        };
        if let Some(text) = text {
            undo.write(&yml, text.as_bytes())?;
        }
        let mut files = vec![yml.to_string_lossy().into_owned()];
        // Lutris reads `<slug>.jpg` whatever the picture's format.
        let cover = data.join("coverart").join(format!("{slug}.jpg"));
        // Lutris keys covers by slug alone: once another game has this slug,
        // the picture is that game's too, and Kryoto no longer owns it.
        let shared = before.is_some_and(|a| tx.query_row("SELECT COUNT(*) FROM games WHERE slug = ?1 AND id != ?2", rusqlite::params![a.slug, a.id], |r| r.get::<_, i64>(0)).unwrap_or(1) > 0);
        let ours = !shared && before.is_some_and(|a| a.files.iter().any(|f| Path::new(f) == cover));
        let mut has_cover = ours;
        match &g.cover {
            // A cover Lutris or the player put there is left alone.
            Some(bytes) if ours || !cover.exists() => {
                undo.write(&cover, bytes)?;
                out.pictures += 1;
                has_cover = true;
            }
            _ => {}
        }
        if has_cover {
            files.push(cover.to_string_lossy().into_owned());
        }
        let hours = g.playtime_seconds as f64 / 3600.0;
        let id = match before {
            Some(a) => {
                tx.execute("UPDATE games SET name = ?1 WHERE id = ?2", rusqlite::params![g.name, a.id]).map_err(busy)?;
                if has("directory") {
                    tx.execute("UPDATE games SET directory = ?1 WHERE id = ?2", rusqlite::params![g.directory, a.id]).map_err(busy)?;
                }
                out.updated += 1;
                a.id
            }
            None => {
                let mut names: Vec<&str> = Vec::new();
                let mut values: Vec<rusqlite::types::Value> = Vec::new();
                let mut put = |c: &'static str, v: rusqlite::types::Value| {
                    if has(c) {
                        names.push(c);
                        values.push(v);
                    }
                };
                use rusqlite::types::Value as V;
                put("name", V::Text(g.name.clone()));
                put("sortname", V::Text(String::new()));
                put("slug", V::Text(slug.clone()));
                put("runner", V::Text("linux".into()));
                put("platform", V::Text("Linux".into()));
                put("directory", V::Text(g.directory.clone()));
                put("installed", V::Integer(1));
                put("installed_at", V::Integer(now as i64));
                put("lastplayed", V::Integer(g.last_played.unwrap_or(0) as i64));
                put("configpath", V::Text(configpath.clone()));
                put("playtime", V::Real(hours));
                put("has_custom_coverart_big", V::Integer(has_cover as i64));
                let marks = vec!["?"; names.len()].join(", ");
                let sql = format!("INSERT INTO games ({}) VALUES ({marks})", names.join(", "));
                tx.execute(&sql, rusqlite::params_from_iter(values)).map_err(busy)?;
                out.added += 1;
                tx.last_insert_rowid()
            }
        };
        let entry = Added { key: g.key.clone(), id, slug, configpath, files };
        if before.is_none() {
            out.inserted.push(entry.clone());
        }
        out.games.push(entry);
    }
    tx.commit().map_err(busy)?;
    Ok(out)
}

/// Take back games Kryoto added: only rows that are still exactly those
/// (same id, slug and settings file), with the files written for them. A
/// game whose row is gone keeps its files, and a cover stays while another
/// game has the same slug: Lutris finds covers by slug, so it is that game's.
pub fn take_back(data: &Path, added: &[Added], backups: &Path, now: u64) -> Result<usize, String> {
    if added.is_empty() {
        return Ok(0);
    }
    let mut conn = open_write(data)?;
    columns(&conn)?;
    back_up(data, backups, now)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| format!("Lutris's library could not be changed: {e}"))?;
    let mut gone: Vec<&Added> = Vec::new();
    for a in added {
        if tx.execute("DELETE FROM games WHERE id = ?1 AND slug = ?2 AND configpath = ?3", rusqlite::params![a.id, a.slug, a.configpath]).map_err(|e| e.to_string())? == 1 {
            gone.push(a);
        }
    }
    let mut files: Vec<&String> = Vec::new();
    for a in &gone {
        let cover = data.join("coverart").join(format!("{}.jpg", a.slug));
        let shared = tx.query_row("SELECT COUNT(*) FROM games WHERE slug = ?1", [&a.slug], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())? > 0;
        files.extend(a.files.iter().filter(|f| !shared || Path::new(f) != cover));
    }
    tx.commit().map_err(|e| format!("Lutris's library could not be changed: {e}"))?;
    for f in files {
        let _ = std::fs::remove_file(f);
    }
    Ok(gone.len())
}

/// Synthetic Lutris folders, for tests and `examples/make-lutris-library.rs`.
#[doc(hidden)]
pub mod testlib {
    use rusqlite::Connection;
    use std::path::Path;

    /// `pga.db` with Lutris's `games` table (`schema.py`) in `data`.
    pub fn make_db(data: &Path) -> rusqlite::Result<Connection> {
        std::fs::create_dir_all(data).ok();
        let conn = Connection::open(data.join("pga.db"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS games (id INTEGER PRIMARY KEY, name TEXT, sortname TEXT, slug TEXT, installer_slug TEXT, parent_slug TEXT, platform TEXT, runner TEXT, executable TEXT, directory TEXT, updated DATETIME, lastplayed INTEGER, installed INTEGER, installed_at INTEGER, year INTEGER, configpath TEXT, has_custom_banner INTEGER, has_custom_icon INTEGER, has_custom_coverart_big INTEGER, playtime REAL, service TEXT, service_id TEXT, discord_id TEXT);
             CREATE TABLE IF NOT EXISTS categories (id INTEGER PRIMARY KEY, name TEXT UNIQUE);",
        )?;
        Ok(conn)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(conn: &Connection, config: &Path, slug: &str, name: &str, runner: &str, directory: &str, installed: bool, hours: f64, lastplayed: i64, yaml: Option<&str>) -> rusqlite::Result<i64> {
        let configpath = yaml.map(|_| format!("{slug}-1700000000"));
        if let (Some(cp), Some(y)) = (&configpath, yaml) {
            std::fs::create_dir_all(config.join("games")).ok();
            std::fs::write(config.join("games").join(format!("{cp}.yml")), y).ok();
        }
        conn.execute(
            "INSERT INTO games (name, sortname, slug, runner, platform, directory, installed, installed_at, lastplayed, configpath, playtime) VALUES (?1, '', ?2, ?3, '', ?4, ?5, 1700000000, ?6, ?7, ?8)",
            rusqlite::params![name, slug, runner, directory, installed as i64, lastplayed, configpath.unwrap_or_default(), hours],
        )?;
        Ok(conn.last_insert_rowid())
    }
}

#[cfg(test)]
mod tests {
    use super::testlib::*;
    use super::*;

    fn temp(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kryoto-lutris-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rows(data: &Path) -> Vec<(i64, String, String)> {
        let c = Connection::open(data.join("pga.db")).unwrap();
        let mut s = c.prepare("SELECT id, slug, name FROM games ORDER BY id").unwrap();
        let out = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect::<Vec<_>>();
        out
    }

    /// A Lutris home with a Wine game, a native one, a Steam one, one not
    /// installed, one with no settings file and one Kryoto sent.
    fn library(root: &Path) -> (PathBuf, PathBuf) {
        let data = root.join(".local/share/lutris");
        let config = root.join(".config/lutris");
        std::fs::create_dir_all(&config).unwrap();
        let conn = make_db(&data).unwrap();
        let games = root.join("Games");
        let wine_yaml = "game:\n  exe: drive_c/Game/witcher.exe\n  args: -skipintro\n  prefix: ~/Games/witcher\nwine:\n  version: lutris-GE-Proton8-26\nsystem:\n  env:\n    DXVK_HUD: fps\n    bad name: x\n";
        add(&conn, &config, "the-witcher", "The Witcher", "wine", &games.join("witcher").to_string_lossy(), true, 12.5, 1_760_000_000, Some(wine_yaml)).unwrap();
        add(&conn, &config, "celeste", "Céleste 日本", "linux", &games.join("celeste").to_string_lossy(), true, 0.0, 0, Some("game:\n  exe: /opt/celeste/Celeste\n")).unwrap();
        add(&conn, &config, "dota-2", "Dota 2", "steam", "", true, 3.0, 0, Some("game:\n  appid: 570\n")).unwrap();
        add(&conn, &config, "gone", "Gone", "linux", "/nowhere", false, 1.0, 0, None).unwrap();
        add(&conn, &config, "no-yaml", "No Settings", "linux", "/opt/x", true, 0.0, 0, None).unwrap();
        add(&conn, &config, "hades", "Hades", "linux", "/g/hades", true, 0.0, 0, Some("game:\n  exe: /usr/bin/kryoto-desktop\n  args: kryoto://library/hades\n")).unwrap();
        std::fs::create_dir_all(data.join("coverart")).unwrap();
        std::fs::write(data.join("coverart/the-witcher.jpg"), b"\xff\xd8\xff").unwrap();
        (data, config)
    }

    #[test]
    fn reads_every_kind_of_game_without_changing_anything() {
        let root = temp("read");
        let (data, config) = library(&root);
        assert_eq!(config_for(&data, None), config);
        let before = std::fs::read(data.join("pga.db")).unwrap();
        let got = read_library(&data, &config, Some(&root)).unwrap();
        assert_eq!(got.len(), 6);
        let w = &got[0];
        assert_eq!((w.runner.as_str(), w.installed, w.playtime_seconds, w.last_played), ("wine", true, 45_000, Some(1_760_000_000)));
        assert_eq!(Path::new(w.exe.as_deref().unwrap()), root.join("Games/witcher").join("drive_c/Game/witcher.exe"));
        assert_eq!((w.args.as_deref(), w.wine_version.as_deref()), (Some("-skipintro"), Some("lutris-GE-Proton8-26")));
        assert_eq!(Path::new(w.prefix.as_deref().unwrap()), root.join("Games/witcher"));
        assert_eq!(w.env, vec![("DXVK_HUD".to_string(), "fps".to_string())]);
        assert!(w.cover.as_deref().unwrap().ends_with("the-witcher.jpg"));
        assert_eq!((got[1].name.as_str(), got[1].exe.as_deref()), ("Céleste 日本", Some("/opt/celeste/Celeste")));
        assert_eq!(got[2].runner, "steam");
        assert!(!got[3].installed);
        assert_eq!(got[4].exe, None);
        assert!(got[5].from_kryoto);
        assert_eq!(std::fs::read(data.join("pga.db")).unwrap(), before);
        assert!(!data.join("pga.db-journal").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_relative_exe_is_in_the_games_folder_not_where_it_starts() {
        let mut g = LutrisGame { directory: Some("/games/foo".into()), ..Default::default() };
        settings(&mut g, "game:\n  exe: drive_c/Foo/bin/foo.exe\n  working_dir: /games/foo/drive_c/Foo\n", None);
        assert_eq!(Path::new(g.exe.as_deref().unwrap()), Path::new("/games/foo/drive_c/Foo/bin/foo.exe"));
        // No folder in Lutris's library: the working folder is all there is.
        let mut g = LutrisGame::default();
        settings(&mut g, "game:\n  exe: bin/foo.exe\n  working_dir: /games/bar\n", None);
        assert_eq!(Path::new(g.exe.as_deref().unwrap()), Path::new("/games/bar/bin/foo.exe"));
    }

    #[test]
    fn finds_the_flatpak_and_a_picked_folder() {
        let root = temp("find");
        let flat = root.join(".var/app/net.lutris.Lutris");
        make_db(&flat.join("data/lutris")).unwrap();
        let dirs = candidate_data_dirs(Some(&root), None);
        assert_eq!(dirs.iter().find(|d| d.join("pga.db").is_file()), Some(&flat.join("data/lutris")));
        assert_eq!(data_in(&flat), Some(flat.join("data/lutris")));
        assert_eq!(data_in(&root.join("nothing")), None);
        // No config folder in the Flatpak yet: the data folder holds the games.
        assert_eq!(config_for(&flat.join("data/lutris"), None), flat.join("data/lutris"));
        std::fs::create_dir_all(flat.join("config/lutris")).unwrap();
        assert_eq!(config_for(&flat.join("data/lutris"), None), flat.join("config/lutris"));
        let _ = std::fs::remove_dir_all(root);
    }

    fn outgoing(key: &str, name: &str) -> NewGame {
        NewGame { key: key.into(), name: name.into(), exe: "/usr/bin/kryoto-desktop".into(), args: format!("kryoto://library/{key}"), directory: format!("/games/{key}"), playtime_seconds: 7_200, last_played: Some(1_760_000_000), cover: Some(b"\xff\xd8\xffcover".to_vec()) }
    }

    #[test]
    fn sends_updates_and_takes_back_only_its_own() {
        let root = temp("send");
        let (data, config) = library(&root);
        let backups = root.join("backups");
        let others = rows(&data);
        // "hades" is taken in Lutris already: this one gets a slug of its own.
        let w = send(&data, &config, &[outgoing("hades", "Hades"), outgoing("lantern", "Lantern Rite")], &[], &backups, 1_800_000_000).unwrap();
        assert_eq!((w.added, w.updated, w.pictures), (2, 0, 2));
        assert_eq!(w.games[0].slug, "hades-kryoto");
        assert!(w.backup.as_ref().unwrap().is_file());
        let after = rows(&data);
        assert_eq!(&after[..others.len()], &others[..], "no other row touched");
        let read = read_library(&data, &config, Some(&root)).unwrap();
        let sent = read.iter().find(|g| g.slug == "lantern").unwrap();
        assert_eq!((sent.runner.as_str(), sent.installed, sent.playtime_seconds), ("linux", true, 7_200));
        assert_eq!((sent.exe.as_deref(), sent.args.as_deref(), sent.from_kryoto), (Some("/usr/bin/kryoto-desktop"), Some("kryoto://library/lantern"), true));
        assert!(sent.cover.is_some());
        // Again: updated in place, never a second row.
        let again = send(&data, &config, &[outgoing("lantern", "Lantern Rite 2")], &w.games, &backups, 1_800_000_100).unwrap();
        assert_eq!((again.added, again.updated), (0, 1));
        assert_eq!(rows(&data).len(), after.len());
        assert!(rows(&data).iter().any(|r| r.2 == "Lantern Rite 2"));
        // Undo: only Kryoto's rows and files.
        let ledger: Vec<Added> = w.games.iter().filter(|a| a.key != "lantern").cloned().chain(again.games.clone()).collect();
        assert_eq!(take_back(&data, &ledger, &backups, 1_800_000_200).unwrap(), 2);
        assert_eq!(rows(&data), others);
        assert!(ledger.iter().flat_map(|a| &a.files).all(|f| !Path::new(f).exists()));
        assert!(data.join("coverart/the-witcher.jpg").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_send_says_which_rows_it_put_in() {
        let root = temp("inserted");
        let (data, config) = library(&root);
        let backups = root.join("backups");
        let w = send(&data, &config, &[outgoing("lantern", "Lantern Rite"), outgoing("moss", "Moss")], &[], &backups, 1_800_000_000).unwrap();
        assert_eq!(w.inserted, w.games);
        // The player removed Lantern in Lutris: sending it again puts in a new row.
        Connection::open(data.join("pga.db")).unwrap().execute("DELETE FROM games WHERE slug = 'lantern'", []).unwrap();
        let before = rows(&data);
        let again = send(&data, &config, &[outgoing("lantern", "Lantern Rite"), outgoing("moss", "Moss")], &w.games, &backups, 1_800_000_100).unwrap();
        assert_eq!((again.added, again.updated), (1, 1));
        assert_eq!(again.inserted.len(), 1);
        assert_eq!(again.inserted[0].key, "lantern");
        assert_ne!(again.inserted[0].configpath, w.games[0].configpath);
        // Taking back just those leaves Lutris as it was before this send.
        assert_eq!(take_back(&data, &again.inserted, &backups, 1_800_000_200).unwrap(), 1);
        assert_eq!(rows(&data), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn undo_leaves_a_cover_another_game_now_uses() {
        let root = temp("shared-cover");
        let (data, config) = library(&root);
        let backups = root.join("backups");
        let w = send(&data, &config, &[outgoing("lantern", "Lantern Rite"), outgoing("moss", "Moss")], &[], &backups, 1_800_000_000).unwrap();
        let conn = Connection::open(data.join("pga.db")).unwrap();
        // Lantern: the player removed Kryoto's entry in Lutris and installed
        // the game through Lutris, same slug, with a cover of their own.
        conn.execute("DELETE FROM games WHERE slug = 'lantern'", []).unwrap();
        add(&conn, &config, "lantern", "Lantern Rite", "wine", "/g/lantern", true, 1.0, 0, Some("game:\n  exe: l.exe\n")).unwrap();
        // Moss: Lutris added its own Moss beside Kryoto's.
        let theirs = add(&conn, &config, "moss", "Moss", "wine", "/g/moss", true, 1.0, 0, None).unwrap();
        std::fs::write(data.join("coverart/lantern.jpg"), b"custom").unwrap();
        // Sending Moss again leaves the cover it now shares alone.
        std::fs::write(data.join("coverart/moss.jpg"), b"theirs").unwrap();
        let moss: Vec<Added> = w.games.iter().filter(|a| a.key == "moss").cloned().collect();
        let again = send(&data, &config, &[outgoing("moss", "Moss")], &moss, &backups, 1_800_000_100).unwrap();
        assert_eq!((again.updated, again.pictures), (1, 0));
        assert_eq!(std::fs::read(data.join("coverart/moss.jpg")).unwrap(), b"theirs");
        assert!(!again.games[0].files.iter().any(|f| f.ends_with("moss.jpg")));
        // Undo with the first ledger: Kryoto's Moss goes, nothing of Lutris's does.
        assert_eq!(take_back(&data, &w.games, &backups, 1_800_000_200).unwrap(), 1);
        assert_eq!(std::fs::read(data.join("coverart/lantern.jpg")).unwrap(), b"custom");
        assert_eq!(std::fs::read(data.join("coverart/moss.jpg")).unwrap(), b"theirs");
        assert!(rows(&data).iter().any(|r| r.0 == theirs));
        assert!(!Path::new(&moss[0].files[0]).exists(), "Kryoto's own settings file goes");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sending_again_keeps_what_the_player_set_in_lutris() {
        let root = temp("keep-yaml");
        let (data, config) = library(&root);
        let backups = root.join("backups");
        let w = send(&data, &config, &[outgoing("lantern", "Lantern Rite")], &[], &backups, 1_800_000_000).unwrap();
        let yml = PathBuf::from(&w.games[0].files[0]);
        // The player's Configure in Lutris: gamescope, an env var, the runtime back on.
        let mine = "# mine\ngame:\n  exe: /usr/bin/kryoto-desktop\n  args: kryoto://library/lantern\n  working_dir: /usr/bin\n  prefix: /p\nlinux:\n  gamescope: true\nsystem:\n  env:\n    MANGOHUD: '1'\n";
        std::fs::write(&yml, mine).unwrap();
        // Nothing of Kryoto's changed: the file is left as it is.
        send(&data, &config, &[outgoing("lantern", "Lantern Rite")], &w.games, &backups, 1_800_000_100).unwrap();
        assert_eq!(std::fs::read_to_string(&yml).unwrap(), mine);
        // Kryoto moved: only its own keys change.
        let moved = NewGame { exe: "/opt/kryoto/kryoto-desktop".into(), args: String::new(), ..outgoing("lantern", "Lantern Rite") };
        send(&data, &config, &[moved], &w.games, &backups, 1_800_000_200).unwrap();
        let doc: Yaml = serde_yaml::from_str(&std::fs::read_to_string(&yml).unwrap()).unwrap();
        assert_eq!((doc["game"]["exe"].as_str(), doc["game"]["working_dir"].as_str(), doc["game"].get("args")), (Some("/opt/kryoto/kryoto-desktop"), Some("/opt/kryoto"), None));
        assert_eq!((doc["game"]["prefix"].as_str(), doc["linux"]["gamescope"].as_bool(), doc["system"]["env"]["MANGOHUD"].as_str()), (Some("/p"), Some(true), Some("1")));
        assert_eq!(doc["system"].get("disable_runtime"), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_failure_halfway_leaves_lutris_as_it_was() {
        let root = temp("fail");
        let (data, config) = library(&root);
        // Lutris's library refuses the second game.
        Connection::open(data.join("pga.db")).unwrap().execute_batch("CREATE TRIGGER boom BEFORE INSERT ON games WHEN NEW.name = 'Boom' BEGIN SELECT RAISE(ABORT, 'boom'); END;").unwrap();
        let before = rows(&data);
        let yamls = std::fs::read_dir(config.join("games")).unwrap().count();
        let err = send(&data, &config, &[outgoing("first", "First"), outgoing("boom", "Boom")], &[], &root.join("backups"), 1_800_000_000).unwrap_err();
        assert!(err.contains("nothing was"), "{err}");
        assert_eq!(rows(&data), before);
        assert_eq!(std::fs::read_dir(config.join("games")).unwrap().count(), yamls);
        assert!(!data.join("coverart/first.jpg").exists());
        // A backup was made before anything changed.
        assert_eq!(std::fs::read_dir(root.join("backups")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_a_library_left_mid_change_or_unknown() {
        let root = temp("refuse");
        let (data, config) = library(&root);
        std::fs::write(data.join("pga.db-journal"), b"").unwrap();
        assert!(send(&data, &config, &[outgoing("a", "A")], &[], &root.join("b"), 1).unwrap_err().contains("did not close cleanly"));
        std::fs::remove_file(data.join("pga.db-journal")).unwrap();
        let odd = root.join("odd");
        std::fs::create_dir_all(&odd).unwrap();
        Connection::open(odd.join("pga.db")).unwrap().execute_batch("CREATE TABLE games (id INTEGER PRIMARY KEY, title TEXT);").unwrap();
        assert!(send(&odd, &odd, &[outgoing("a", "A")], &[], &root.join("b"), 1).unwrap_err().contains("nothing was changed"));
        assert!(read_library(&root.join("none"), &root, None).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn wine_builds_are_found_only_inside_lutris() {
        let root = temp("wine");
        let bin = root.join("runners/wine/lutris-GE-Proton8-26/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("wine"), b"").unwrap();
        assert_eq!(wine_binary(&root, "lutris-GE-Proton8-26"), Some(bin.join("wine")));
        for v in ["system", "", "../../x", "missing"] {
            assert_eq!(wine_binary(&root, v), None, "{v}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_yaml_lutris_reads_back() {
        let y = game_yaml(&outgoing("hades", "Hades"));
        let doc: Yaml = serde_yaml::from_str(&y).unwrap();
        assert_eq!(doc["game"]["exe"].as_str(), Some("/usr/bin/kryoto-desktop"));
        assert_eq!(doc["game"]["args"].as_str(), Some("kryoto://library/hades"));
        assert_eq!(doc["system"]["disable_runtime"].as_bool(), Some(true));
    }
}
