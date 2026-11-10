//! Online-Fix's generic Steam fix, the "Fix Repair" archive online-fix.me
//! keeps next to each game (`<Game>_Fix_Repair_Steam_Generic.rar`): the few
//! files that switch an installed game to Online-Fix's Steam layer, never the
//! game itself.
//!
//! What one looks like, from a game it was applied to (Gamble With Your
//! Friends, 2026-08): `OnlineFix64.dll` (the layer), `OnlineFix.ini`
//! (`[Main] RealAppId=<the game>`, `FakeAppId=480`), `winmm.dll` (the loader
//! the game picks up from its own folder), `dlllist.txt` (what the loader
//! loads), `SteamOverlay64.dll`, and Steam's own `steam_api64.dll` where the
//! game keeps it (`<Game>_Data/Plugins/x86_64/` for Unity).
//!
//! Shared by Kryoto Forge, which applies it to a release and packs the fix on
//! its own, and Kryoto Desktop, which applies that package to an installed
//! game from Properties > Repair. Both go through the same checks and the same
//! repair journal (`journal.rs`), so a failure or an interruption puts every
//! replaced file back, and Undo works the same as for any other repair.
//!
//! Nothing here runs anything from the package: it is read as a file list,
//! unpacked by 7-Zip into a staging folder, and copied.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::journal::{self, Journal};

/// How the fix is named wherever a person sees it.
pub const SOURCE_LABEL: &str = "Online-Fix (generic)";
/// The release label on kryo.to.
pub const RELEASE_LABEL: &str = "Fix: Online-Fix (generic)";
/// What the package says about itself, at the root of the fix-only archive.
pub const MANIFEST_NAME: &str = "kryoto-fix.json";
/// online-fix.me's archives are locked with this; Kryoto's own are not.
pub const ARCHIVE_PASSWORDS: [&str; 2] = ["online-fix.me", ""];

/// A generic fix is a handful of files, tens of MB. Anything far past this is
/// a game (or a game with the fix in it), which is never what was asked for.
const MAX_FILES: usize = 80;
const MAX_BYTES: u64 = 300 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X64,
    X86,
}

impl Arch {
    pub fn label(self) -> &'static str {
        match self {
            Arch::X64 => "64-bit",
            Arch::X86 => "32-bit",
        }
    }
    fn steam_api(self) -> &'static str {
        match self {
            Arch::X64 => "steam_api64.dll",
            Arch::X86 => "steam_api.dll",
        }
    }
}

/// One line of an archive's listing (`7z l -slt`).
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub path: String,
    pub size: u64,
    pub dir: bool,
    pub encrypted: bool,
    pub link: bool,
}

/// Read `7z l -slt` output. Only what follows the `----------` separator is
/// the listing; the block before it describes the archive itself.
pub fn parse_slt(text: &str) -> Vec<Listed> {
    // 7-Zip on Windows ends its lines with CRLF.
    let text = text.replace("\r\n", "\n");
    let body = text.split_once("\n----------").map(|(_, b)| b).unwrap_or("");
    let mut out = Vec::new();
    for block in body.split("\n\n") {
        let mut path = None;
        let mut size = 0u64;
        let mut dir = false;
        let mut encrypted = false;
        let mut link = false;
        for line in block.lines() {
            let Some((k, v)) = line.split_once(" = ") else { continue };
            match k.trim() {
                "Path" => path = Some(v.trim().to_string()),
                "Size" => size = v.trim().parse().unwrap_or(0),
                "Folder" => dir = v.trim() == "+",
                "Encrypted" => encrypted = v.trim() == "+",
                "Attributes" => {
                    let a = v.trim();
                    dir |= a.starts_with('D');
                    // 7-Zip prints a unix mode after the Windows letters
                    // ("A_ lrwxrwxrwx"); a link's starts with `l`.
                    link |= a.split_whitespace().nth(1).is_some_and(|m| m.starts_with('l'));
                }
                "Symbolic Link" | "Link" => link |= !v.trim().is_empty(),
                _ => {}
            }
        }
        if let Some(path) = path.filter(|p| !p.is_empty()) {
            out.push(Listed { path, size, dir, encrypted, link });
        }
    }
    out
}

/// A Windows shortcut or internet shortcut, by its extension.
pub fn is_shortcut(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".url") || p.ends_with(".lnk")
}

fn norm(path: &str) -> String {
    path.replace('\\', "/").trim_matches('/').to_string()
}

/// No absolute paths, drive letters, `..`, or links: nothing in the package
/// may land outside the folder it is unpacked into.
pub fn check_paths(entries: &[Listed]) -> Result<(), String> {
    for e in entries {
        let p = e.path.replace('\\', "/");
        let bad = p.starts_with('/')
            || p.contains(':')
            || p.split('/').any(|part| part == "..")
            || p.chars().any(|c| c.is_control());
        if bad {
            return Err(format!("The archive has an unsafe path ({}), so it was not unpacked.", e.path));
        }
        if e.link {
            return Err(format!("The archive has a link ({}), which a fix never needs, so it was not unpacked.", e.path));
        }
    }
    Ok(())
}

fn base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// What a listing holds, once it is known to be a generic fix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scope {
    /// The folder inside the archive that maps onto the game's folder ("" for
    /// the top), the one with `OnlineFix.ini` in it.
    pub root: String,
    /// The fix's files, relative to `root`.
    pub files: Vec<String>,
    pub arch: Arch,
    pub bytes: u64,
}

/// Decide whether a listing is the generic fix and nothing else.
pub fn scope(entries: &[Listed]) -> Result<Scope, String> {
    check_paths(entries)?;
    // Internet shortcuts (online-fix.me ships `OnlineFix.url`, a link to its
    // site) are not part of the fix and never go into a game folder.
    let files: Vec<&Listed> = entries.iter().filter(|e| !e.dir && !is_shortcut(&e.path)).collect();
    let bytes: u64 = files.iter().map(|e| e.size).sum();
    if files.len() > MAX_FILES || bytes > MAX_BYTES {
        return Err(format!(
            "This archive has {} files ({} MB). Online-Fix's generic fix is a handful of files, so this looks like a full game, not the fix. Pick the archive from the game's Fix Repair folder.",
            files.len(),
            bytes / (1024 * 1024)
        ));
    }
    let layer: Vec<String> = files
        .iter()
        .map(|e| norm(&e.path))
        .filter(|p| {
            let b = base(p).to_ascii_lowercase();
            b == "onlinefix64.dll" || b == "onlinefix.dll"
        })
        .collect();
    let ini_dirs: Vec<String> = files
        .iter()
        .map(|e| norm(&e.path))
        .filter(|p| base(p).eq_ignore_ascii_case("OnlineFix.ini"))
        .map(|p| p.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default())
        .collect();
    if layer.is_empty() || ini_dirs.is_empty() {
        return Err("This is not Online-Fix's generic fix: it has no OnlineFix64.dll and OnlineFix.ini.".into());
    }
    if ini_dirs.len() > 1 {
        return Err("The archive holds more than one OnlineFix.ini, so it is not clear which game folder it is for.".into());
    }
    let root = ini_dirs[0].clone();
    let prefix = if root.is_empty() { String::new() } else { format!("{root}/") };
    let mut rel = Vec::new();
    for e in &files {
        let p = norm(&e.path);
        match p.strip_prefix(&prefix) {
            Some(r) if !r.is_empty() => rel.push(r.to_string()),
            _ if prefix.is_empty() => rel.push(p),
            _ => {
                return Err(format!(
                    "{p} is outside the fix's folder; the archive carries more than the generic fix."
                ))
            }
        }
    }
    let arch = if layer.iter().any(|p| base(p).eq_ignore_ascii_case("OnlineFix64.dll")) {
        Arch::X64
    } else {
        Arch::X86
    };
    rel.sort();
    Ok(Scope { root, files: rel, arch, bytes })
}

/// `OnlineFix.ini`'s `[Main]`: which game it is for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IniFacts {
    pub real_appid: Option<String>,
    pub fake_appid: Option<String>,
}

pub fn read_ini(text: &str) -> IniFacts {
    let mut facts = IniFacts::default();
    let mut main = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            main = line.eq_ignore_ascii_case("[main]");
            continue;
        }
        if !main || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().to_string();
            if v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty() {
                match k.trim().to_ascii_lowercase().as_str() {
                    "realappid" => facts.real_appid = Some(v),
                    "fakeappid" => facts.fake_appid = Some(v),
                    _ => {}
                }
            }
        }
    }
    facts
}

/// Find `rel` under `root` whatever its case, as a game on Linux keeps it.
/// The path as it really is, or None when some part of it is not there.
pub fn resolve(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut at = root.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        // Read the folder rather than test the path: Windows answers "exists"
        // for any case, and the name the game really uses is what is wanted.
        let names: Vec<String> = std::fs::read_dir(&at)
            .ok()?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let name = names
            .iter()
            .find(|n| n.as_str() == part)
            .or_else(|| names.iter().find(|n| n.eq_ignore_ascii_case(part)))?;
        at = at.join(name);
    }
    Some(at)
}

/// One file of the fix and where it goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// Relative to the fix's folder.
    pub from: String,
    /// Relative to the game's folder, in the game's own case.
    pub to: String,
    /// A file of the game's that it replaces (kept in the journal for undo).
    pub replaces: bool,
    /// The game already has this exact file (same SHA-256).
    #[serde(default)]
    pub same: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub appid: String,
    pub arch: Arch,
    pub placements: Vec<Placement>,
    pub warnings: Vec<String>,
    /// Every file is already in the game, byte for byte: the game has this fix
    /// (a release built from Online-Fix's own). The fix is still worth having
    /// as a repair: it puts these files back when an antivirus or an update
    /// takes them.
    #[serde(default)]
    pub already_applied: bool,
}

fn find_named(root: &Path, name: &str) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .follow_links(false)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| !e.file_name().to_string_lossy().starts_with(".kryoto"))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.into_path())
        .collect()
}

/// Check the unpacked fix against the game it is meant for, and say where
/// every file goes. Refuses, with the reason, when it is for another game,
/// another build layout, or the other architecture.
pub fn plan(fix_root: &Path, scope: &Scope, game_root: &Path, appid: &str) -> Result<Plan, String> {
    let ini = scope
        .files
        .iter()
        .find(|f| base(f).eq_ignore_ascii_case("OnlineFix.ini"))
        .ok_or("The fix has no OnlineFix.ini.")?;
    let facts = read_ini(&std::fs::read_to_string(fix_root.join(ini)).map_err(|e| format!("Reading OnlineFix.ini: {e}"))?);
    match facts.real_appid.as_deref() {
        Some(id) if id == appid => {}
        Some(id) => {
            return Err(format!(
                "This fix is for Steam app {id} (OnlineFix.ini says so), not this game ({appid}). It was not applied."
            ))
        }
        None => {
            return Err("OnlineFix.ini names no RealAppId, so it cannot be confirmed as this game's fix.".into())
        }
    }
    let wanted = scope.arch.steam_api();
    let other = match scope.arch {
        Arch::X64 => Arch::X86.steam_api(),
        Arch::X86 => Arch::X64.steam_api(),
    };
    let game_dlls = find_named(game_root, wanted);
    if game_dlls.is_empty() {
        let has_other = !find_named(game_root, other).is_empty();
        return Err(if has_other {
            format!(
                "This fix is {} but the game is not: it has {other} and no {wanted}.",
                scope.arch.label()
            )
        } else {
            format!("The game has no {wanted}, so there is nothing for a {} Online-Fix to work with.", scope.arch.label())
        });
    }

    let mut placements = Vec::new();
    for from in &scope.files {
        // The folder must already be the game's: a fix for another layout
        // (another game, another build) is refused rather than scattered.
        let (dir, name) = from.rsplit_once('/').map(|(d, n)| (d, n)).unwrap_or(("", from.as_str()));
        let target_dir = if dir.is_empty() {
            game_root.to_path_buf()
        } else {
            resolve(game_root, dir).filter(|p| p.is_dir()).ok_or_else(|| {
                format!("The fix puts files in \"{dir}\", which this game does not have. It is for another game or another version of it.")
            })?
        };
        let target = resolve(&target_dir, name).unwrap_or_else(|| target_dir.join(name));
        if target.exists() && !target.is_file() {
            return Err(format!("{from} would replace a folder in the game."));
        }
        let to = target
            .strip_prefix(game_root)
            .map_err(|_| "A fix file resolves outside the game folder.".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let same = target.is_file()
            && journal::digest(&target).ok() == journal::digest(&fix_root.join(from)).ok();
        placements.push(Placement { from: from.clone(), replaces: target.is_file(), same, to });
    }

    let mut warnings = Vec::new();
    let ships_dll = scope.files.iter().any(|f| base(f).eq_ignore_ascii_case(wanted));
    let emulated = game_dlls.iter().any(|d| d.parent().is_some_and(|p| p.join("steam_settings").is_dir()));
    if !ships_dll && emulated {
        warnings.push(format!(
            "The fix does not carry Steam's own {wanted}, and this release's is an emulator's (a steam_settings folder sits beside it). Online-Fix needs Steam's own dll, so it may not start until that is put back."
        ));
    }
    let already_applied = !placements.is_empty() && placements.iter().all(|p| p.same);
    if already_applied {
        warnings.push("This release already has these exact files (it is built on Online-Fix's own release). As a repair fix it puts them back when an antivirus or an update removes or replaces them.".into());
    }
    Ok(Plan { appid: appid.to_string(), arch: scope.arch, placements, warnings, already_applied })
}

/// Copy the fix into the game under the repair journal: every file it
/// replaces is backed up and verified first, and if any copy fails the game is
/// put back as it was. Undo is `journal::undo`, as for any repair.
pub fn apply(fix_root: &Path, game_root: &Path, plan: &Plan, version: &str) -> Result<Journal, String> {
    let _guard = journal::lock(game_root)?;
    let targets: Vec<PathBuf> = plan.placements.iter().map(|p| game_root.join(&p.to)).collect();
    let mut record = Journal::prepare(game_root, &targets, &[], SOURCE_LABEL, version)?;
    let copy = || -> Result<(), String> {
        for p in &plan.placements {
            let to = journal::safe_path(game_root, &p.to)?;
            if to.exists() {
                journal::make_writable(&to)?;
            }
            std::fs::copy(fix_root.join(&p.from), &to).map_err(|e| format!("Copying {}: {e}", p.from))?;
        }
        Ok(())
    };
    match copy() {
        Ok(()) => {
            record.state = "applied".into();
            record.save(game_root)?;
            Ok(record)
        }
        Err(error) => match record.restore(game_root) {
            Ok(()) => Err(format!("{error} The game's files were put back.")),
            Err(restore) => Err(format!("{error} Putting the files back failed too: {restore}. Keep the .kryoto-repair folder.")),
        },
    }
}

/// The scope of a package already unpacked to `dir` (Kryoto Desktop, after a
/// download): the same rules as for a listing, without the manifest, which
/// describes the fix rather than being part of it.
pub fn scope_of_dir(dir: &Path) -> Result<Scope, String> {
    let mut entries = Vec::new();
    for e in walkdir::WalkDir::new(dir).follow_links(false).max_depth(12) {
        let e = e.map_err(|e| e.to_string())?;
        let rel = e.path().strip_prefix(dir).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
        if rel.is_empty() || base(&rel).eq_ignore_ascii_case(MANIFEST_NAME) {
            continue;
        }
        let t = e.file_type();
        entries.push(Listed {
            size: if t.is_file() { e.metadata().map(|m| m.len()).unwrap_or(0) } else { 0 },
            dir: t.is_dir(),
            encrypted: false,
            link: t.is_symlink(),
            path: rel,
        });
    }
    scope(&entries)
}

/// The manifest at the root of a fix-only package, if it has one.
pub fn read_manifest(dir: &Path) -> Option<Manifest> {
    serde_json::from_slice(&std::fs::read(dir.join(MANIFEST_NAME)).ok()?).ok()
}

/// Undo an applied fix: the same as Properties > Repair > Undo.
pub fn undo(game_root: &Path) -> Result<(), String> {
    journal::undo(game_root)
}

/// What the fix-only package records about itself (`kryoto-fix.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub kind: String,
    pub label: String,
    pub steam_appid: String,
    pub arch: Arch,
    /// Where the package came from: online-fix.me's file URL, or "imported: <file name>".
    pub provenance: String,
    /// SHA-256 of the archive as it was obtained.
    pub package_sha256: String,
    pub files: Vec<String>,
}

impl Manifest {
    pub fn new(plan: &Plan, provenance: &str, package_sha256: &str) -> Self {
        Self {
            kind: "online-fix-generic".into(),
            label: RELEASE_LABEL.into(),
            steam_appid: plan.appid.clone(),
            arch: plan.arch,
            provenance: provenance.into(),
            package_sha256: package_sha256.into(),
            files: plan.placements.iter().map(|p| p.to.clone()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(path: &str, size: u64) -> Listed {
        Listed { path: path.into(), size, dir: false, encrypted: false, link: false }
    }

    fn generic() -> Vec<Listed> {
        vec![
            Listed { path: "Fix Repair".into(), size: 0, dir: true, encrypted: false, link: false },
            listed("Fix Repair/OnlineFix64.dll", 13_820_928),
            listed("Fix Repair/OnlineFix.ini", 607),
            listed("Fix Repair/winmm.dll", 506_368),
            listed("Fix Repair/dlllist.txt", 35),
            listed("Fix Repair/SteamOverlay64.dll", 117_248),
            listed("Fix Repair/Game_Data/Plugins/x86_64/steam_api64.dll", 300_000),
        ]
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryoto-fixpack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: PathBuf, body: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    const INI: &str = "[Main]\nRealAppId=3892270\nFakeAppId=480\n\n[Misc]\nExtraProtection=false\n";

    /// A fix folder and a game folder, the way Forge and Desktop see them.
    fn setup(d: &Path, appid_in_ini: &str) -> (PathBuf, PathBuf, Scope) {
        let fix = d.join("fix");
        write(fix.join("OnlineFix64.dll"), b"layer");
        write(fix.join("OnlineFix.ini"), INI.replace("3892270", appid_in_ini).as_bytes());
        write(fix.join("winmm.dll"), b"loader");
        write(fix.join("dlllist.txt"), b"OnlineFix64.dll\n");
        write(fix.join("Game_Data/Plugins/x86_64/steam_api64.dll"), b"valve");
        let game = d.join("game");
        write(game.join("Game.exe"), b"exe");
        write(game.join("game_data/Plugins/x86_64/steam_api64.dll"), b"gbe");
        write(game.join("game_data/Plugins/x86_64/steam_settings/configs.user.ini"), b"[user]\naccount_name=me\n");
        write(game.join("save.dat"), b"mine");
        let scope = Scope {
            root: String::new(),
            files: vec![
                "Game_Data/Plugins/x86_64/steam_api64.dll".into(),
                "OnlineFix.ini".into(),
                "OnlineFix64.dll".into(),
                "dlllist.txt".into(),
                "winmm.dll".into(),
            ],
            arch: Arch::X64,
            bytes: 0,
        };
        (fix, game, scope)
    }

    #[test]
    fn reads_7z_technical_listing() {
        let text = "7-Zip 24\n\nListing archive: a.rar\n--\nPath = a.rar\nType = Rar5\n\n----------\nPath = Fix Repair\nFolder = +\nSize = 0\nAttributes = D\n\nPath = Fix Repair\\OnlineFix64.dll\nFolder = -\nSize = 13820928\nEncrypted = +\nAttributes = A\n\n";
        let l = parse_slt(text);
        assert_eq!(l.len(), 2);
        assert!(l[0].dir);
        assert_eq!(l[1].path, "Fix Repair\\OnlineFix64.dll");
        assert_eq!(l[1].size, 13_820_928);
        assert!(l[1].encrypted);
        assert_eq!(parse_slt(&text.replace('\n', "\r\n")), l, "Windows line endings read the same");
    }

    #[test]
    fn finds_the_fix_folder_and_its_architecture() {
        let s = scope(&generic()).unwrap();
        assert_eq!(s.root, "Fix Repair");
        assert_eq!(s.arch, Arch::X64);
        assert!(s.files.contains(&"Game_Data/Plugins/x86_64/steam_api64.dll".to_string()));
        assert!(s.files.contains(&"OnlineFix.ini".to_string()));
    }

    #[test]
    fn refuses_unsafe_paths_and_links() {
        for bad in ["../evil.dll", "C:/Windows/winmm.dll", "/etc/passwd", "Fix Repair/../../x.dll"] {
            let mut l = generic();
            l.push(listed(bad, 1));
            assert!(scope(&l).unwrap_err().contains("unsafe path"), "{bad}");
        }
        let mut l = generic();
        l.push(Listed { path: "Fix Repair/link.dll".into(), size: 1, dir: false, encrypted: false, link: true });
        assert!(scope(&l).unwrap_err().contains("link"));
    }

    #[test]
    fn refuses_a_full_game_and_a_non_fix() {
        let mut game: Vec<Listed> = (0..500).map(|i| listed(&format!("Game/Data/{i}.pak"), 50_000_000)).collect();
        game.extend(generic());
        assert!(scope(&game).unwrap_err().contains("looks like a full game"));
        assert!(scope(&[listed("Game.exe", 10)]).unwrap_err().contains("not Online-Fix's generic fix"));
        let mut extra = generic();
        extra.push(listed("Readme/elsewhere.txt", 10));
        assert!(scope(&extra).unwrap_err().contains("outside the fix's folder"));
    }

    #[test]
    fn plans_onto_the_right_game_whatever_the_case() {
        let d = scratch("plan");
        let (fix, game, s) = setup(&d, "3892270");
        let p = plan(&fix, &s, &game, "3892270").unwrap();
        let dll = p.placements.iter().find(|x| x.from.ends_with("steam_api64.dll")).unwrap();
        assert_eq!(dll.to, "game_data/Plugins/x86_64/steam_api64.dll");
        assert!(dll.replaces);
        assert!(!p.placements.iter().find(|x| x.from == "OnlineFix64.dll").unwrap().replaces);
        assert!(p.warnings.is_empty(), "it ships Steam's own dll");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn refuses_another_games_fix_and_the_wrong_architecture() {
        let d = scratch("wrong");
        let (fix, game, s) = setup(&d, "1500");
        assert!(plan(&fix, &s, &game, "3892270").unwrap_err().contains("for Steam app 1500"));
        let (fix, game, mut s) = setup(&d, "3892270");
        s.arch = Arch::X86;
        assert!(plan(&fix, &s, &game, "3892270").unwrap_err().contains("is 32-bit but the game is not"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn refuses_a_layout_the_game_does_not_have() {
        let d = scratch("layout");
        let (fix, game, mut s) = setup(&d, "3892270");
        write(fix.join("Other_Data/Plugins/x86_64/steam_api64.dll"), b"valve");
        s.files.push("Other_Data/Plugins/x86_64/steam_api64.dll".into());
        assert!(plan(&fix, &s, &game, "3892270").unwrap_err().contains("this game does not have"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn applies_with_backups_and_undoes_cleanly() {
        let d = scratch("apply");
        let (fix, game, s) = setup(&d, "3892270");
        let p = plan(&fix, &s, &game, "3892270").unwrap();
        let record = apply(&fix, &game, &p, "test").unwrap();
        assert_eq!(record.state, "applied");
        assert_eq!(std::fs::read(game.join("game_data/Plugins/x86_64/steam_api64.dll")).unwrap(), b"valve");
        assert_eq!(std::fs::read(game.join("OnlineFix64.dll")).unwrap(), b"layer");
        assert_eq!(std::fs::read(game.join("save.dat")).unwrap(), b"mine", "saves untouched");
        journal::undo(&game).unwrap();
        assert_eq!(std::fs::read(game.join("game_data/Plugins/x86_64/steam_api64.dll")).unwrap(), b"gbe");
        assert!(!game.join("OnlineFix64.dll").exists());
        assert!(game.join("game_data/Plugins/x86_64/steam_settings/configs.user.ini").is_file(), "user config untouched");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_failed_copy_puts_everything_back() {
        let d = scratch("fail");
        let (fix, game, s) = setup(&d, "3892270");
        let p = plan(&fix, &s, &game, "3892270").unwrap();
        // The source of the last placement vanishes mid-way.
        std::fs::remove_file(fix.join("winmm.dll")).unwrap();
        let err = apply(&fix, &game, &p, "test").unwrap_err();
        assert!(err.contains("put back"), "{err}");
        assert_eq!(std::fs::read(game.join("game_data/Plugins/x86_64/steam_api64.dll")).unwrap(), b"gbe");
        assert!(!game.join("OnlineFix64.dll").exists());
        assert_eq!(journal::load(&game).unwrap().unwrap().state, "undone");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn warns_when_the_fix_leaves_an_emulator_dll_in_place() {
        let d = scratch("warn");
        let (fix, game, mut s) = setup(&d, "3892270");
        s.files.retain(|f| !f.ends_with("steam_api64.dll"));
        let p = plan(&fix, &s, &game, "3892270").unwrap();
        assert_eq!(p.warnings.len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn reads_an_unpacked_package_without_its_manifest() {
        let d = scratch("dir");
        let (fix, _game, _) = setup(&d, "3892270");
        std::fs::write(fix.join(MANIFEST_NAME), b"{}").unwrap();
        let s = scope_of_dir(&fix).unwrap();
        assert_eq!(s.root, "");
        assert!(!s.files.iter().any(|f| f == MANIFEST_NAME));
        assert_eq!(s.files.len(), 5);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn leaves_shortcuts_out() {
        let mut l = generic();
        l.push(listed("Fix Repair/OnlineFix.url", 50));
        let s = scope(&l).unwrap();
        assert!(!s.files.iter().any(|f| f.ends_with(".url")));
    }

    #[test]
    fn knows_a_game_that_already_has_the_fix() {
        let d = scratch("already");
        let (fix, game, s) = setup(&d, "3892270");
        for f in &s.files {
            let to = game.join(f.replace("Game_Data", "game_data"));
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(fix.join(f), to).unwrap();
        }
        let p = plan(&fix, &s, &game, "3892270").unwrap();
        assert!(p.already_applied);
        assert!(p.placements.iter().all(|x| x.same));
        let p2 = {
            std::fs::write(game.join("winmm.dll"), b"older loader").unwrap();
            plan(&fix, &s, &game, "3892270").unwrap()
        };
        assert!(!p2.already_applied);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn reads_the_ini() {
        assert_eq!(
            read_ini(INI),
            IniFacts { real_appid: Some("3892270".into()), fake_appid: Some("480".into()) }
        );
        assert_eq!(read_ini("[Other]\nRealAppId=1\n").real_appid, None);
    }
}
