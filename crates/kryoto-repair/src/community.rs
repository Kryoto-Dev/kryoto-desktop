//! Community updates: what Kryoto Desktop checks, and records, before it
//! offers kryo.to a newer build of a game the player has.
//!
//! kryo.to holds the offer to the same rules (kryo.to lib/community-update-rules.ts)
//! and adds the checks only it can make; this side exists so a player is told
//! "no" before uploading 40 GB, and so the server has something to check
//! against:
//!
//!   * Only a release made on a source Kryoto can vouch for is offered at all
//!     ([`eligible_source`]). The list is the server's ELIGIBLE_SOURCES; change
//!     both together.
//!   * [`stamp_tree`] notes each file's size and time when Kryoto's release is
//!     installed (no hashing: a 50 GB game on a hard disk would take minutes).
//!   * [`build_manifest`] records the SHA-256, size and CRC-32 of every file in
//!     the game, and where each came from: unchanged since the install, or
//!     written by the updater. A file that changed but the updater did not write
//!     stops the offer here, before anything is sent. gbe_fork's per-player
//!     files, which every Play rewrites with the player's name and language,
//!     are the exception: they go with those reset ([`is_player_file`]).
//!   * [`check_offer`] holds the emulator files to Kryoto's own build (the
//!     gbe_fork Desktop itself installs), byte for byte.
//!   * [`pack_zip`] writes exactly the manifest's files into a zip, the format
//!     the server reads back to compare the archive's own index with the list.

use crate::Cancellation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::Path,
};

/// The `label` an accepted community update's release carries.
pub const COMMUNITY_RELEASE_LABEL: &str = "Community update";

/// One allowlisted source: the label kryo.to shows and what it is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EligibleSource {
    pub id: &'static str,
    pub label: &'static str,
    pub steamless: bool,
}

/// The sources a community update may come from (kryo.to ELIGIBLE_SOURCES).
/// The owner's call, gbe_fork only: RUNE's Steamclient profile patches the
/// game's own dll, and OFME's files are Online-Fix's, not built by Kryoto.
pub const ELIGIBLE_SOURCES: [EligibleSource; 2] = [
    EligibleSource { id: "gbe_fork", label: "Steam + gbe_fork", steamless: false },
    EligibleSource { id: "gbe_fork_steamless", label: "Steam + gbe_fork + Steamless", steamless: true },
];

/// The allowlisted source a release label names. Every part of the label has
/// to be accounted for: gbe_fork plus anything else is not gbe_fork.
pub fn eligible_source(label: &str) -> Option<EligibleSource> {
    let mut parts: Vec<String> = label
        .split('+')
        .map(|p| p.chars().filter(|c| c.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect::<String>())
        .filter(|p| !p.is_empty() && p != "steam")
        .map(|p| if p == "gbe" || p == "gbeforkemu" { "gbefork".to_string() } else { p })
        .collect();
    parts.sort();
    parts.dedup();
    match parts.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["gbefork"] => Some(ELIGIBLE_SOURCES[0]),
        ["gbefork", "steamless"] => Some(ELIGIBLE_SOURCES[1]),
        _ => None,
    }
}

/// Why a release's source is not taken (kryo.to `ineligibleReason`).
pub fn ineligible_reason(label: &str) -> &'static str {
    let l = label.to_ascii_lowercase().replace(['-', ' ', '_'], "");
    if l.contains("onlinefix") || l.contains("ofme") {
        "Online-Fix releases are updated by Kryoto staff from Online-Fix's own release."
    } else {
        "Only games whose Kryoto release uses gbe_fork can be updated this way."
    }
}

/// Where a file came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Unchanged from the Kryoto release the player installed.
    Release,
    /// Written by the updater from the game's own update.
    Update,
    /// Kryoto's emulator file, put back by the updater.
    Emulator,
    /// A game executable the updater unpacked with Steamless.
    Steamless,
}

/// One file of the offered build, as the server's `manifest` takes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// `/`-separated, relative to the game folder; the same path in the zip.
    pub path: String,
    pub sha256: String,
    pub size: u64,
    /// 8 hex digits, as the zip's own index carries it.
    pub crc32: String,
    pub role: Role,
}

/// A file as it was when Kryoto's release was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub size: u64,
    /// Last write, in milliseconds since 1970.
    pub modified_ms: i64,
}

pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let ms = match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    };
    Some(Stamp { size: meta.len(), modified_ms: ms })
}

/// Every file's stamp, for the release just installed.
pub fn stamp_tree(root: &Path) -> BTreeMap<String, Stamp> {
    game_files(root).into_iter().filter_map(|rel| Some((rel.clone(), stamp(&root.join(&rel))?))).collect()
}

/// Kryoto's own bookkeeping at the top of a game folder (the crack record,
/// the originals, this module's records): never part of a release, so never
/// part of an offer. The `.dll.kryoto` backups beside a dll do ship.
fn is_ours(rel: &str) -> bool {
    rel.split('/').next().unwrap_or_default().starts_with(".kryoto")
}

fn is_steam_api(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
    name == "steam_api.dll" || name == "steam_api64.dll"
}

/// A path that stays inside the game folder.
pub fn safe_path(rel: &str) -> bool {
    !rel.is_empty()
        && rel.len() <= 400
        && !rel.contains('\\')
        && !rel.contains('\0')
        && !rel.starts_with('/')
        && !(rel.len() > 1 && rel.as_bytes()[1] == b':')
        && rel.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}

/// SHA-256 (hex), CRC-32 and size of one file, read once.
pub fn fingerprint(path: &Path) -> std::io::Result<(String, u32, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut sha = Sha256::new();
    let mut crc = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
        crc.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hex(&sha.finalize()), crc.finalize(), size))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// gbe_fork's per-player files, which Kryoto Desktop rewrites before every
/// Play with the player's in-game name and language (Desktop's player_name.rs).
const PLAYER_FILES: [&str; 3] = ["configs.user.ini", "force_account_name.txt", "force_language.txt"];

/// A file that holds the player's own name and settings: `steam_settings/`
/// followed by one of [`PLAYER_FILES`], at any depth.
pub fn is_player_file(rel: &str) -> bool {
    let mut parts = rel.rsplit('/');
    let name = parts.next().unwrap_or_default().to_ascii_lowercase();
    parts.next().is_some_and(|dir| dir.eq_ignore_ascii_case("steam_settings")) && PLAYER_FILES.contains(&name.as_str())
}

/// A per-player file with Desktop's edits taken back out: the name and
/// language Kryoto's own build writes (crack.rs [`EmuIdentity`]), every other
/// line as it is. A file that is not UTF-8 was never edited by Desktop, so it
/// comes back as it is.
///
/// [`EmuIdentity`]: crate::crack::EmuIdentity
pub fn neutral_player_file(rel: &str, bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else { return bytes.to_vec() };
    let own = crate::crack::EmuIdentity::default();
    let name = rel.rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
    match name.as_str() {
        "force_account_name.txt" => return own.account_name.into_bytes(),
        "force_language.txt" => return own.language.into_bytes(),
        _ => {}
    }
    let mut out = String::with_capacity(text.len());
    let mut section = String::new();
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let ending = &line[body.len()..];
        let trimmed = body.trim().trim_start_matches('\u{feff}');
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed[1..trimmed.len() - 1].trim().to_ascii_lowercase();
        } else if section == "user::general" && !trimmed.starts_with(';') && !trimmed.starts_with('#') {
            if let Some((key, _)) = body.split_once('=') {
                let value = match key.trim().to_ascii_lowercase().as_str() {
                    "account_name" => Some(own.account_name.as_str()),
                    "language" => Some(own.language.as_str()),
                    _ => None,
                };
                if let Some(value) = value {
                    let spaced = body.contains(" = ");
                    out.push_str(key.trim_end());
                    out.push_str(if spaced { " = " } else { "=" });
                    out.push_str(value);
                    out.push_str(ending);
                    continue;
                }
            }
        }
        out.push_str(line);
    }
    out.into_bytes()
}

/// The bytes an offer carries for a file: the folder's own, except that a
/// per-player file goes as [`neutral_player_file`] makes it.
fn offered_bytes(root: &Path, rel: &str) -> std::io::Result<Vec<u8>> {
    let bytes = std::fs::read(root.join(rel))?;
    Ok(if is_player_file(rel) { neutral_player_file(rel, &bytes) } else { bytes })
}

/// Every file under `root` that an offer would carry, `/`-separated.
fn game_files(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| Some(e.path().strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/")))
        .filter(|rel| !is_ours(rel))
        .collect();
    out.sort();
    out
}

/// The updater's record of the game as it is now.
///
/// `before` is each file's stamp as Kryoto's release installed it; `written`
/// is every file the updater wrote, with what it was. A file in neither that
/// differs from `before` (or is new) was changed by something else, and the
/// offer stops: the list of those paths comes back instead.
pub fn build_manifest(
    root: &Path,
    before: &BTreeMap<String, Stamp>,
    written: &BTreeMap<String, Role>,
    cancel: &dyn Cancellation,
) -> Result<Vec<ManifestEntry>, Vec<String>> {
    let mut entries = Vec::new();
    let mut foreign = Vec::new();
    for rel in game_files(root) {
        if cancel.requested() {
            return Err(vec!["cancelled".into()]);
        }
        let path = root.join(&rel);
        // Desktop rewrites the per-player files on every Play, so they never
        // count as changed: one the release had goes with the player's name
        // and language reset, one it did not have stays out (Desktop makes
        // it again on the next player's first Play).
        if is_player_file(&rel) {
            if !before.contains_key(&rel) && !written.contains_key(&rel) {
                continue;
            }
            let Ok(bytes) = offered_bytes(root, &rel) else {
                foreign.push(rel);
                continue;
            };
            let role = written.get(&rel).copied().unwrap_or(Role::Release);
            let crc = crc32fast::hash(&bytes);
            entries.push(ManifestEntry { sha256: hex(&Sha256::digest(&bytes)), size: bytes.len() as u64, crc32: format!("{crc:08x}"), role, path: rel });
            continue;
        }
        let role = match written.get(&rel) {
            Some(role) => *role,
            None if before.get(&rel).is_some() && before.get(&rel).copied() == stamp(&path) => Role::Release,
            None => {
                foreign.push(rel);
                continue;
            }
        };
        // A Steam dll is always the emulator, changed or not: it is the one
        // file kryo.to holds to Kryoto's own fingerprints, whatever its history.
        let role = if role == Role::Release && is_steam_api(&rel) { Role::Emulator } else { role };
        let Ok((sha256, crc, size)) = fingerprint(&path) else {
            foreign.push(rel);
            continue;
        };
        entries.push(ManifestEntry { path: rel, sha256, size, crc32: format!("{crc:08x}"), role });
    }
    if foreign.is_empty() {
        Ok(entries)
    } else {
        Err(foreign)
    }
}

/// What the server's `diff` says, worked out from the install's stamps and the manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub added: usize,
    pub modified: usize,
    pub removed: usize,
    pub added_executables: Vec<String>,
    pub stray_executables: Vec<String>,
    pub kryoto_files_intact: bool,
    pub size_delta: i64,
    pub hashes: Vec<String>,
}

const PROGRAM: [&str; 13] = ["exe", "dll", "so", "dylib", "bat", "cmd", "ps1", "sh", "vbs", "js", "jar", "msi", "scr"];

fn is_program(rel: &str) -> bool {
    rel.rsplit_once('.').is_some_and(|(_, ext)| PROGRAM.contains(&ext.to_ascii_lowercase().as_str()))
}

pub fn diff(before: &BTreeMap<String, Stamp>, manifest: &[ManifestEntry]) -> Diff {
    let now: BTreeSet<&str> = manifest.iter().map(|e| e.path.as_str()).collect();
    let mut d = Diff { kryoto_files_intact: true, ..Default::default() };
    for e in manifest.iter().filter(|e| e.role != Role::Release) {
        match before.get(&e.path) {
            Some(_) => d.modified += 1,
            None => {
                d.added += 1;
                if is_program(&e.path) {
                    d.added_executables.push(e.path.clone());
                }
            }
        }
        d.hashes.push(e.sha256.clone());
    }
    d.removed = before.keys().filter(|p| !now.contains(p.as_str())).count();
    let before_bytes: u64 = before.values().map(|s| s.size).sum();
    let after_bytes: u64 = manifest.iter().map(|e| e.size).sum();
    d.size_delta = after_bytes as i64 - before_bytes as i64;
    d
}

/// What stops an offer, in the words the player reads. Empty: it may be sent.
///
/// `kryoto_emulator` is the SHA-256 of every file of Kryoto's own emulator
/// build when Desktop has one to compare with; with `None` kryo.to alone
/// checks them (it always does).
pub fn check_offer(source_label: &str, manifest: &[ManifestEntry], kryoto_emulator: Option<&BTreeSet<String>>) -> Vec<String> {
    let mut out = Vec::new();
    let Some(source) = eligible_source(source_label) else {
        out.push(ineligible_reason(source_label).into());
        return out;
    };
    if manifest.is_empty() {
        out.push("There are no files to offer.".into());
    }
    let unsafe_paths: Vec<&str> = manifest.iter().filter(|e| !safe_path(&e.path)).map(|e| e.path.as_str()).collect();
    if !unsafe_paths.is_empty() {
        out.push(format!("Some paths leave the game folder ({}).", unsafe_paths.join(", ")));
    }
    let steam_api: Vec<&ManifestEntry> = manifest.iter().filter(|e| is_steam_api(&e.path)).collect();
    if steam_api.is_empty() {
        out.push("The emulator Kryoto set up is missing from the game.".into());
    }
    let not_emu: Vec<&str> = steam_api.iter().filter(|e| e.role != Role::Emulator).map(|e| e.path.as_str()).collect();
    if !not_emu.is_empty() {
        out.push(format!("A Steam file is not Kryoto's emulator ({}).", not_emu.join(", ")));
    }
    let wrong_emu: Vec<&str> = manifest
        .iter()
        .filter(|e| e.role == Role::Emulator && kryoto_emulator.is_some_and(|k| !k.contains(&e.sha256)))
        .map(|e| e.path.as_str())
        .collect();
    if !wrong_emu.is_empty() {
        out.push(format!("The emulator files do not match Kryoto's own ({}).", wrong_emu.join(", ")));
    }
    if !source.steamless && manifest.iter().any(|e| e.role == Role::Steamless) {
        out.push("It has Steamless-unpacked programs, but the release was not made with Steamless.".into());
    }
    if manifest.iter().any(|e| e.role == Role::Steamless && !e.path.to_ascii_lowercase().ends_with(".exe")) {
        out.push("Only a game's program can be unpacked with Steamless.".into());
    }
    if !manifest.is_empty() && manifest.iter().all(|e| matches!(e.role, Role::Release | Role::Emulator)) {
        out.push("Nothing in it differs from the release you have.".into());
    }
    out
}

/// Write the manifest's files, and nothing else, into a zip at `out`.
///
/// Deflated, zip64 where a file needs it. Each file is hashed again as it is
/// written, so a file that changed after the manifest was made fails the pack
/// instead of shipping bytes the list does not describe; a failed pack
/// leaves no archive behind, not even a well-formed part of one.
pub fn pack_zip(root: &Path, manifest: &[ManifestEntry], out: &Path, cancel: &dyn Cancellation) -> Result<(), String> {
    let result = write_zip(root, manifest, out, cancel);
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result
}

fn write_zip(root: &Path, manifest: &[ManifestEntry], out: &Path, cancel: &dyn Cancellation) -> Result<(), String> {
    let file = std::fs::File::create(out).map_err(|e| format!("Creating the archive failed: {e}"))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let mut buf = vec![0u8; 1 << 20];
    for entry in manifest {
        if cancel.requested() {
            return Err("cancelled".into());
        }
        if !safe_path(&entry.path) {
            return Err(format!("{} leaves the game folder.", entry.path));
        }
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(entry.size >= u32::MAX as u64);
        zip.start_file(entry.path.as_str(), options).map_err(|e| e.to_string())?;
        let mut sha = Sha256::new();
        if is_player_file(&entry.path) {
            // Never the folder's own copy: it holds the player's name.
            let bytes = offered_bytes(root, &entry.path).map_err(|e| format!("{}: {e}", entry.path))?;
            sha.update(&bytes);
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
            if hex(&sha.finalize()) != entry.sha256 {
                return Err(format!("{} changed after it was recorded.", entry.path));
            }
            continue;
        }
        let mut src = std::fs::File::open(root.join(&entry.path)).map_err(|e| format!("{}: {e}", entry.path))?;
        loop {
            let n = src.read(&mut buf).map_err(|e| format!("{}: {e}", entry.path))?;
            if n == 0 {
                break;
            }
            sha.update(&buf[..n]);
            zip.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        }
        if hex(&sha.finalize()) != entry.sha256 {
            return Err(format!("{} changed after it was recorded.", entry.path));
        }
    }
    zip.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cancel;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kry-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    #[test]
    fn sources_match_the_server_allowlist() {
        assert_eq!(eligible_source("Steam + gbe_fork").map(|s| s.id), Some("gbe_fork"));
        assert_eq!(eligible_source("Steam + GBE-Fork").map(|s| s.id), Some("gbe_fork"));
        assert_eq!(eligible_source("Steam + gbe_fork + Steamless").map(|s| s.id), Some("gbe_fork_steamless"));
        for s in ["Steam + RUNE", "Steam + RUNE steamclient", "OFME", "Online-Fix", "Steam + gbe_fork + Kryoto Online", "Goldberg", "CODEX", "Steam", ""] {
            assert_eq!(eligible_source(s), None, "{s}");
        }
    }

    #[test]
    fn manifest_records_release_and_updater_files_and_refuses_anything_else() {
        let root = dir("community-manifest");
        put(&root, "Game.exe", b"new game");
        put(&root, "data/a.pak", b"same as release");
        put(&root, "steam_api64.dll", b"gbe");
        put(&root, ".kryoto-crack.json", b"{}");
        put(&root, "steam_api64.dll.kryoto", b"original");
        let mut before = stamp_tree(&root);
        assert!(!before.contains_key(".kryoto-crack.json") && before.contains_key("steam_api64.dll.kryoto"));
        before.insert("Game.exe".into(), Stamp { size: 8, modified_ms: 1 });
        let written = BTreeMap::from([("Game.exe".to_string(), Role::Update), ("steam_api64.dll".to_string(), Role::Emulator)]);
        let m = build_manifest(&root, &before, &written, &Cancel::new()).unwrap();
        assert_eq!(m.iter().map(|e| (e.path.as_str(), e.role)).collect::<Vec<_>>(), [
            ("Game.exe", Role::Update),
            ("data/a.pak", Role::Release),
            ("steam_api64.dll", Role::Emulator),
            ("steam_api64.dll.kryoto", Role::Release),
        ]);
        assert_eq!(m[0].crc32, format!("{:08x}", crc32fast::hash(b"new game")));

        let d = diff(&before, &m);
        assert_eq!((d.added, d.modified, d.removed), (0, 2, 0));
        assert_eq!(d.hashes.len(), 2);

        // A file nobody wrote that is not the release's: hand-made, and the offer stops.
        put(&root, "data/b.pak", b"dropped in by hand");
        put(&root, "data/a.pak", b"edited by hand, longer");
        let err = build_manifest(&root, &before, &written, &Cancel::new()).unwrap_err();
        assert_eq!(err, ["data/a.pak", "data/b.pak"]);
    }

    #[test]
    fn a_play_does_not_stop_the_offer_and_no_players_name_or_language_is_sent() {
        let root = dir("community-player");
        let release_ini = "[user::general]\r\naccount_name=kryoto\r\nlanguage=english\r\nip_country=US\r\n";
        put(&root, "Game.exe", b"new game");
        put(&root, "steam_api64.dll", b"gbe");
        put(&root, "bin/steam_settings/configs.user.ini", release_ini.as_bytes());
        put(&root, "bin/steam_settings/force_account_name.txt", b"kryoto");
        let mut before = stamp_tree(&root);
        before.insert("Game.exe".into(), Stamp { size: 8, modified_ms: 1 });
        let written = BTreeMap::from([("Game.exe".to_string(), Role::Update)]);

        // What Play writes (Desktop's player_name.rs): the name and language,
        // and a language file the release did not have.
        std::thread::sleep(std::time::Duration::from_millis(20));
        put(&root, "bin/steam_settings/configs.user.ini", b"[user::general]\r\naccount_name=Somebody Real\r\nlanguage=german\r\nip_country=US\r\n");
        put(&root, "bin/steam_settings/force_account_name.txt", b"Somebody Real");
        put(&root, "bin/steam_settings/force_language.txt", b"german");

        let m = build_manifest(&root, &before, &written, &Cancel::new()).unwrap();
        assert_eq!(m.iter().map(|e| (e.path.as_str(), e.role)).collect::<Vec<_>>(), [
            ("Game.exe", Role::Update),
            ("bin/steam_settings/configs.user.ini", Role::Release),
            ("bin/steam_settings/force_account_name.txt", Role::Release),
            ("steam_api64.dll", Role::Emulator),
        ]);
        // The release's own bytes go, not the player's, and they count as unchanged.
        assert_eq!(m[1].sha256, sha(release_ini.as_bytes()));
        assert_eq!(m[2].sha256, sha(b"kryoto"));
        let d = diff(&before, &m);
        // Game.exe and the emulator, as without a Play.
        assert_eq!((d.added, d.modified, d.hashes.len()), (0, 2, 2));

        let out = root.join(".kryoto-offer.zip");
        pack_zip(&root, &m, &out, &Cancel::new()).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let mut ini = String::new();
        archive.by_name("bin/steam_settings/configs.user.ini").unwrap().read_to_string(&mut ini).unwrap();
        assert_eq!(ini, release_ini);
        for i in 0..archive.len() {
            let mut body = Vec::new();
            archive.by_index(i).unwrap().read_to_end(&mut body).unwrap();
            let body = String::from_utf8_lossy(&body);
            assert!(!body.contains("Somebody") && !body.contains("german"), "{body}");
        }

        // Anything else changed by hand still stops the offer.
        put(&root, "bin/steam_settings/configs.main.ini", b"[main::general]\nnew_app_ticket=1\n");
        assert_eq!(build_manifest(&root, &before, &written, &Cancel::new()).unwrap_err(), ["bin/steam_settings/configs.main.ini"]);
    }

    #[test]
    fn only_the_name_and_language_are_reset() {
        let ini = b"; made by Kryoto\n[user::general]\naccount_name = Somebody\nlanguage=german\nip_country=DE\n[user::saves]\nlocal_save_path=./saves\n";
        assert_eq!(
            String::from_utf8(neutral_player_file("steam_settings/configs.user.ini", ini)).unwrap(),
            "; made by Kryoto\n[user::general]\naccount_name = kryoto\nlanguage=english\nip_country=DE\n[user::saves]\nlocal_save_path=./saves\n"
        );
        assert!(is_player_file("steam_settings/configs.user.ini") && is_player_file("x/Steam_Settings/FORCE_LANGUAGE.txt"));
        assert!(!is_player_file("configs.user.ini") && !is_player_file("steam_settings/configs.main.ini"));
        let odd = [0xff, 0xfe, b'a'];
        assert_eq!(neutral_player_file("steam_settings/configs.user.ini", &odd), odd);
    }

    #[test]
    fn check_offer_holds_the_emulator_to_kryotos_own_build() {
        let entry = |path: &str, sha256: &str, role| ManifestEntry { path: path.into(), sha256: sha256.into(), size: 1, crc32: "00000001".into(), role };
        let ours = BTreeSet::from([sha(b"gbe")]);
        let good = vec![entry("Game.exe", &sha(b"x"), Role::Update), entry("steam_api64.dll", &sha(b"gbe"), Role::Emulator)];
        assert!(check_offer("Steam + gbe_fork", &good, Some(&ours)).is_empty());

        let other_emu = vec![good[0].clone(), entry("steam_api64.dll", &sha(b"other"), Role::Emulator)];
        assert!(check_offer("Steam + gbe_fork", &other_emu, Some(&ours))[0].contains("do not match Kryoto's own"));
        let steams_own = vec![good[0].clone(), entry("steam_api64.dll", &sha(b"valve"), Role::Update)];
        assert!(check_offer("Steam + gbe_fork", &steams_own, Some(&ours))[0].contains("not Kryoto's emulator"));
        assert!(check_offer("Steam + gbe_fork", &good[..1], Some(&ours))[0].contains("missing"));
        assert!(check_offer("Steam + RUNE", &good, Some(&ours))[0].contains("gbe_fork"));
        assert!(check_offer("OFME", &good, Some(&ours))[0].contains("Online-Fix releases are updated by Kryoto staff"));
        let unpacked = [good.clone(), vec![entry("GameU.exe", &sha(b"u"), Role::Steamless)]].concat();
        assert!(check_offer("Steam + gbe_fork", &unpacked, Some(&ours))[0].contains("Steamless"));
        assert!(check_offer("Steam + gbe_fork + Steamless", &unpacked, Some(&ours)).is_empty());
        let escapes = [good.clone(), vec![entry("../x.dll", &sha(b"u"), Role::Update)]].concat();
        assert!(check_offer("Steam + gbe_fork", &escapes, Some(&ours))[0].contains("leave the game folder"));
    }

    #[test]
    fn zip_holds_exactly_the_manifest_and_its_index_matches_it() {
        let root = dir("community-zip");
        put(&root, "Game.exe", b"new game");
        put(&root, "data/a.pak", &vec![7u8; 300_000]);
        put(&root, "notes.txt", b"not in the manifest");
        let written = BTreeMap::from([("Game.exe".to_string(), Role::Update), ("data/a.pak".to_string(), Role::Update)]);
        let mut m = build_manifest(&root, &BTreeMap::new(), &BTreeMap::from([("notes.txt".to_string(), Role::Update)]).into_iter().chain(written).collect(), &Cancel::new()).unwrap();
        m.retain(|e| e.path != "notes.txt");
        let out = root.join(".kryoto-offer.zip");
        pack_zip(&root, &m, &out, &Cancel::new()).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let listed: Vec<(String, u64, String)> = (0..archive.len())
            .map(|i| {
                let f = archive.by_index(i).unwrap();
                (f.name().to_string(), f.size(), format!("{:08x}", f.crc32()))
            })
            .collect();
        assert_eq!(listed, m.iter().map(|e| (e.path.clone(), e.size, e.crc32.clone())).collect::<Vec<_>>());

        // A file that changed after it was recorded does not get packed, and no part of the archive is left.
        put(&root, "Game.exe", b"swapped after the manifest");
        let bad = root.join(".kryoto-offer-bad.zip");
        assert!(pack_zip(&root, &m, &bad, &Cancel::new()).unwrap_err().contains("changed after it was recorded"));
        assert!(!bad.exists());
    }
}
