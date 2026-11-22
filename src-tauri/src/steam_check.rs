//! Why a game that closed at once sent the player to Steam.
//!
//! Steam's "An error occurred while launching this game: No licenses" (or
//! "license not found") is what a player sees when the game's own program
//! asks the real Steam client whether this account owns it. Reports came in
//! for six games in two days (kryoto-bugs KB-0373, KB-0365) and for many
//! before (support tickets since 2026-10-03), and staff answered each with
//! "your antivirus". The production reports and tickets show four different
//! ways it happens, none of them Kryoto's launch path:
//!
//!   * Online-Fix's own files (its loader `winmm.dll`, its layer
//!     `OnlineFix64.dll`) taken by an antivirus: the release ships Steam's own
//!     `steam_api64.dll`, so without them the game asks Steam. Scam With Your
//!     Friends (OFME): 38 "Windows Security blocked a file" install reports
//!     from 17 PCs, and "when I run runme, it says there's no license".
//!   * Steam's DRM wrapper (SteamStub, a `.bind` section) still on the game's
//!     program: the wrapper asks Steam before the game starts. The Men of
//!     Yoshiwara: Kikuya (repair note "SteamStub was found but Steamless is
//!     not configured"), Dead Rising 2.
//!   * Kryoto Online on a game it does not work for: it forwards to the real
//!     Steam, and a game that checks ownership past it is refused. NBA 2K27
//!     ("with online enabled it gives me an error saying no licenses"), Call
//!     of Duty: Black Ops III after the auto-fixer's Kryoto Online.
//!   * Steam's own files with no emulator at all.
//!
//! After a start that ended within seconds, the client looks for exactly
//! these in the game's folder and says which one it is, with what to do.

use std::path::{Path, PathBuf};

/// A program that is part of the game, not a tool beside it.
fn is_game_program(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    name.ends_with(".exe") && !["unins", "setup", "redist", "crash", "vcredist", "dxsetup", "easyanticheat_setup", "eac_setup"].iter().any(|s| name.contains(s))
}

/// The programs the game may really run: the one Play starts, the others in
/// its folder, and an Unreal game's `-Shipping.exe` (its root exe only starts
/// that one). Bounded, so a huge folder on a hard disk is not walked whole.
fn programs(root: &Path, exe: &Path) -> Vec<PathBuf> {
    let mut out = vec![exe.to_path_buf()];
    let mut dirs: Vec<PathBuf> = exe.parent().map(|p| vec![p.to_path_buf()]).unwrap_or_default();
    if !dirs.iter().any(|d| d == root) {
        dirs.push(root.to_path_buf());
    }
    for dir in &dirs {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten().take(400) {
            let p = e.path();
            if p.is_file() && is_game_program(&p) && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    for e in walkdir::WalkDir::new(root).max_depth(5).into_iter().filter_map(|e| e.ok()).take(20_000) {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        if e.file_type().is_file() && name.ends_with("-shipping.exe") && !out.iter().any(|p| p == e.path()) {
            out.push(e.into_path());
        }
    }
    out.truncate(40);
    out
}

fn name_of(path: &Path, root: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// Online-Fix's own files that should be beside its `OnlineFix.ini` and are
/// not: what an antivirus takes first.
fn online_fix_gone(root: &Path, exe: &Path) -> Option<Vec<String>> {
    let dirs: Vec<&Path> = [exe.parent(), Some(root)].into_iter().flatten().collect();
    let has = |dir: &Path, name: &str| kryoto_repair::fixpack::resolve(dir, name).is_some_and(|p| p.is_file());
    let dir = dirs.into_iter().find(|d| has(d, "OnlineFix.ini"))?;
    let mut gone = Vec::new();
    if !has(dir, "OnlineFix64.dll") && !has(dir, "OnlineFix.dll") {
        gone.push("OnlineFix64.dll".to_string());
    }
    // The loader the game picks up from its own folder; dlllist.txt is its list.
    if has(dir, "dlllist.txt") && !has(dir, "winmm.dll") {
        gone.push("winmm.dll".to_string());
    }
    (!gone.is_empty()).then_some(gone)
}

/// Why this game would have asked Steam whether the account owns it, in a
/// player's words, or None when nothing in its folder says so. `exe` is the
/// program Play started; `kryoto_online`: Kryoto Online is what is in it.
pub fn why_steam_asked(root: &Path, exe: &Path, kryoto_online: bool) -> Option<String> {
    if let Some(gone) = online_fix_gone(root, exe) {
        return Some(format!(
            "Steam opened because Online-Fix's own files are gone from the game's folder ({}), usually taken by an antivirus. Without them the game asks Steam whether you own it, and Steam says No licenses. Exclude the game's folder in your antivirus, then get the files back under Properties > Emulator & fixes > Files.",
            gone.join(", ")
        ));
    }
    if kryoto_online {
        return Some("If Steam opened and said No licenses: Kryoto Online does not work with every game, and this one checks with Steam past it. Go back to the release's own files under Properties > Emulator & fixes.".into());
    }
    if let Some(wrapped) = programs(root, exe).into_iter().find(|p| kryoto_repair::detection::has_steamstub(p)) {
        return Some(format!(
            "{} still has Steam's DRM wrapper, so it asks Steam whether you own the game, and Steam says No licenses. Kryoto cannot take the wrapper off on this PC. Try Kryoto Online under Properties > Emulator & fixes, and report it on the game's page so the release can be fixed.",
            name_of(&wrapped, root)
        ));
    }
    if kryoto_repair::crack::untouched_steam_api(root).is_some() {
        return Some("If Steam opened and said No licenses: this copy has Steam's own files and no emulator, so it asks Steam whether you own it. Pick an emulator under Properties > Emulator & fixes.".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kryoto-steam-check-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: PathBuf, body: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// The smallest PE image `has_steamstub` reads: one section, named `name`.
    fn pe(section: &[u8; 8]) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[0] = b'M';
        b[1] = b'Z';
        b[60] = 64;
        b.extend_from_slice(b"PE\0\0");
        b.extend_from_slice(&0x8664u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&[0u8; 12]);
        b.extend_from_slice(&0u16.to_le_bytes()); // no optional header
        b.extend_from_slice(&[0u8; 2]);
        let mut s = [0u8; 40];
        s[..8].copy_from_slice(section);
        b.extend_from_slice(&s);
        b
    }

    #[test]
    fn online_fix_files_an_antivirus_took_are_named() {
        let d = scratch("ofme");
        write(d.join("Scam With Your Friends.exe"), &pe(b".text\0\0\0"));
        write(d.join("OnlineFix.ini"), b"[Main]\nRealAppId=4954910\nFakeAppId=480\n");
        write(d.join("dlllist.txt"), b"OnlineFix64.dll\n");
        write(d.join("steam_api64.dll"), b"valve");
        let exe = d.join("Scam With Your Friends.exe");
        let said = why_steam_asked(&d, &exe, false).unwrap();
        assert!(said.contains("OnlineFix64.dll, winmm.dll") && said.contains("No licenses") && said.contains("antivirus"), "{said}");
        // With the layer and its loader there, nothing to say about it.
        write(d.join("OnlineFix64.dll"), b"layer");
        write(d.join("winmm.dll"), b"loader");
        assert!(why_steam_asked(&d, &exe, false).is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_drm_wrapper_left_on_the_game_is_found_even_behind_an_unreal_launcher() {
        let d = scratch("stub");
        write(d.join("FNAF_SOTM.exe"), &pe(b".text\0\0\0"));
        write(d.join("FNAF_SOTM/Binaries/Win64/FNAF_SOTM-Win64-Shipping.exe"), &pe(b".bind\0\0\0"));
        write(d.join("FNAF_SOTM/Binaries/Win64/steam_api64.dll"), b"gbe");
        write(d.join("FNAF_SOTM/Binaries/Win64/steam_settings/steam_appid.txt"), b"2215390");
        let said = why_steam_asked(&d, &d.join("FNAF_SOTM.exe"), false).unwrap();
        assert!(said.starts_with("FNAF_SOTM/Binaries/Win64/FNAF_SOTM-Win64-Shipping.exe still has Steam's DRM wrapper"), "{said}");
        assert!(said.contains("report it on the game's page"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn kryoto_online_is_named_and_a_working_emulator_says_nothing() {
        let d = scratch("online");
        write(d.join("game.exe"), &pe(b".text\0\0\0"));
        write(d.join("steam_api64.dll"), b"proxy");
        write(d.join("kryoto-online.ini"), b"[Settings]\nAppId=480\n");
        assert!(why_steam_asked(&d, &d.join("game.exe"), true).unwrap().contains("Kryoto Online does not work with every game"));
        // gbe_fork in place, no wrapper: the cause is something else, so nothing is claimed.
        write(d.join("steam_settings/steam_appid.txt"), b"620");
        assert!(why_steam_asked(&d, &d.join("game.exe"), false).is_none());
        let _ = std::fs::remove_dir_all(d);
    }
}
