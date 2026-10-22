//! Which folder names a game's own files mention.
//!
//! A game that saves to `Documents\My Games\SomeStudio\X9` has those names
//! in it: in its exe, its code (`Assembly-CSharp.dll`, IL2CPP's
//! `global-metadata.dat`), or its settings files, as text or as UTF-16. A
//! folder some other program wrote while it ran (a compiler cache, a code
//! checkout) is never named in the game. So a folder name found in the game's
//! own files ties that folder to the game, without administrator rights and
//! without trusting when it was written.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};

/// The game's files that carry its code and settings: its executables and
/// libraries (not other vendors' runtimes), IL2CPP metadata, Godot's pack,
/// and small text files.
fn worth_reading(path: &Path, size: u64) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "exe" | "dll" => size <= MAX_FILE && !RUNTIMES.iter().any(|r| name.starts_with(r)),
        "pck" => true,
        "dat" => name == "global-metadata.dat",
        "ini" | "cfg" | "json" | "xml" | "txt" | "rpy" | "config" | "toml" | "yaml" | "yml" | "info" => size <= 2 << 20,
        _ => false,
    }
}

/// Libraries every game of an engine or a vendor ships: they name their own
/// folders, never the game's.
const RUNTIMES: [&str; 46] = [
    "d3d", "dxgi", "dxcompiler", "dxil", "steam_api", "steamclient", "vcruntime", "msvcp", "msvcr", "ucrtbase", "api-ms-", "concrt",
    "vccorlib", "xinput", "xaudio", "openal", "fmod", "bink", "nvngx", "nvapi", "amd_", "amdxc", "ffx_", "libcef", "chrome_elf",
    "galaxy", "eossdk", "gse", "onlinefix", "unityplayer", "mono-2.0", "monoposix", "physx", "apex", "nvtt", "discord", "ffmpeg",
    "avcodec", "avformat", "avutil", "sdl2", "libgl", "vulkan", "glew", "openvr", "steam_emu",
];

/// One file is read up to this much; the game is read up to [`MAX_TOTAL`].
const MAX_FILE: u64 = 400 << 20;
const MAX_TOTAL: u64 = 2 << 30;
const CHUNK: usize = 8 << 20;

/// The game's files to read, biggest-signal first: executables near the top
/// of the folder before libraries deep inside it.
pub fn game_files(game_dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut out: Vec<(PathBuf, u64, usize)> = walkdir::WalkDir::new(game_dir)
        .max_depth(6)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && e.depth() > 0 && crate::scan::shipped(&format!("{}/x", e.file_name().to_string_lossy())) && !e.file_name().eq_ignore_ascii_case("binaries")))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let size = e.metadata().ok()?.len();
            worth_reading(e.path(), size).then(|| (e.path().to_path_buf(), size, e.depth()))
        })
        .collect();
    let rank = |p: &Path| match p.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("exe") => 0,
        Some("dat") | Some("pck") => 1,
        Some("dll") => 2,
        _ => 3,
    };
    out.sort_by_key(|(p, _, depth)| (rank(p), *depth));
    out.into_iter().map(|(p, s, _)| (p, s)).collect()
}

/// Which of `names` (compared without case) appear as a whole word, as text
/// or UTF-16, in the game's own files. Stops when all are found, or after
/// `budget`.
pub fn find(files: &[(PathBuf, u64)], names: &[String], budget: Duration) -> HashSet<String> {
    let names: Vec<String> = names.iter().map(|n| n.to_lowercase()).filter(|n| !n.is_empty()).collect();
    let mut found = HashSet::new();
    if names.is_empty() {
        return found;
    }
    // Each name twice: as bytes, and as UTF-16LE (Windows APIs, .NET strings).
    let mut patterns: Vec<Vec<u8>> = Vec::with_capacity(names.len() * 2);
    for n in &names {
        patterns.push(n.as_bytes().to_vec());
        patterns.push(n.encode_utf16().flat_map(|u| u.to_le_bytes()).collect());
    }
    let Ok(ac) = AhoCorasickBuilder::new().ascii_case_insensitive(true).match_kind(MatchKind::Standard).build(&patterns) else { return found };
    let longest = patterns.iter().map(Vec::len).max().unwrap_or(0);
    let started = Instant::now();
    let mut total = 0u64;
    'files: for (path, _) in files {
        let Ok(file) = std::fs::File::open(path) else { continue };
        let mut file = file.take(MAX_FILE);
        let mut buf = vec![0u8; CHUNK + longest + 2];
        let mut carry = 0usize;
        loop {
            if started.elapsed() > budget || total > MAX_TOTAL || found.len() == names.len() {
                break 'files;
            }
            let Ok(n) = file.read(&mut buf[carry..carry + CHUNK]) else { break };
            if n == 0 {
                break;
            }
            total += n as u64;
            let end = carry + n;
            scan(&ac, &buf[..end], &names, &mut found);
            // Keep the tail, so a name across two reads is still seen.
            let keep = (longest + 2).min(end);
            buf.copy_within(end - keep..end, 0);
            carry = keep;
        }
    }
    found
}

fn scan(ac: &AhoCorasick, hay: &[u8], names: &[String], found: &mut HashSet<String>) {
    for m in ac.find_overlapping_iter(hay) {
        let name = &names[m.pattern().as_usize() / 2];
        if found.contains(name) {
            continue;
        }
        let wide = m.pattern().as_usize() % 2 == 1;
        if whole_word(hay, m.start(), m.end(), wide) {
            found.insert(name.clone());
        }
    }
}

/// Not part of a longer word: "Hole" inside "WholeNumber" is not a mention.
fn whole_word(hay: &[u8], start: usize, end: usize, wide: bool) -> bool {
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    if wide {
        let before = start >= 2 && hay[start - 1] == 0 && word(hay[start - 2]);
        let after = end + 1 < hay.len() && hay[end + 1] == 0 && word(hay[end]);
        !before && !after
    } else {
        let before = start >= 1 && word(hay[start - 1]);
        let after = end < hay.len() && word(hay[end]);
        !before && !after
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_the_games_files_are_found_as_text_and_utf16() {
        let dir = std::env::temp_dir().join(format!("kryo-mentions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("X9_Data/Managed")).unwrap();
        let mut exe = b"MZ\x90\0junk".to_vec();
        exe.extend("\\My Games\\SomeStudio\\".encode_utf16().flat_map(|u| u.to_le_bytes()));
        exe.extend(b"\0\0WholeNumber\0");
        std::fs::write(dir.join("X9.exe"), &exe).unwrap();
        std::fs::write(dir.join("X9_Data/Managed/Assembly-CSharp.dll"), b"MZ...persist/x9_profiles/slot...").unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"MZ ccache ctw-native").unwrap();
        let files = game_files(&dir);
        assert!(files.iter().all(|(p, _)| !p.ends_with("steam_api64.dll")), "another vendor's runtime is not the game");
        let names: Vec<String> = ["SomeStudio", "x9_profiles", "ccache", "ctw-native", "Hole"].iter().map(|s| s.to_string()).collect();
        let found = find(&files, &names, Duration::from_secs(5));
        assert!(found.contains("somestudio"), "{found:?}");
        assert!(found.contains("x9_profiles"));
        assert!(!found.contains("ccache") && !found.contains("ctw-native"));
        assert!(!found.contains("hole"), "only whole words count");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_across_two_reads_is_still_found() {
        let dir = std::env::temp_dir().join(format!("kryo-mentions-edge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut exe = vec![b' '; CHUNK - 3];
        exe.extend(b"FarAwayStudio ");
        std::fs::write(dir.join("Game.exe"), &exe).unwrap();
        let found = find(&game_files(&dir), &["FarAwayStudio".to_string()], Duration::from_secs(10));
        assert!(found.contains("farawaystudio"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
