//! A library written by classic-level, the LevelDB binding Hydra uses
//! (`fixtures/make-hydra-db.mjs`): a Snappy-compressed table from the first
//! session and, in the log, an update and a removal from the second.

use kryoto_import::hydra::read_library;
use std::path::Path;

#[test]
fn reads_a_real_classic_level_library() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hydra-db");
    let before: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| (e.file_name(), e.metadata().unwrap().len())).collect();
    let games = read_library(&dir).unwrap();
    let titles: Vec<&str> = games.iter().map(|g| g.title.as_str()).collect();
    // Half-Life is marked removed; Half-Life 2 was deleted in the log.
    // Newest played first.
    assert_eq!(titles, ["Stardew Valley", "Hades"]);
    let hades = &games[1];
    // The log's update wins over the table's first write.
    assert_eq!(hades.playtime_seconds, 7260);
    assert_eq!(hades.launch_options.as_deref(), Some("-dx11"));
    assert_eq!(hades.last_played, Some(1_787_316_130));
    // From the shop sublevels, the compressed block among them.
    assert_eq!(hades.cover.as_deref(), Some("https://cdn.example/hades-cover.jpg"));
    assert_eq!(hades.hero.as_deref(), Some("https://cdn.example/hades-hero.jpg"));
    assert_eq!(hades.developer.as_deref(), Some("Supergiant Games"));
    assert_eq!(hades.year, Some(2020));
    assert_eq!(hades.executable.as_deref(), Some(r"D:\Games\Hades\x64\Hades.exe"));
    let sdv = &games[0];
    assert_eq!((sdv.shop.as_str(), sdv.steam_appid.as_deref(), sdv.playtime_seconds), ("custom", None, 3600));
    assert!(sdv.custom_cover.as_deref().is_some_and(|c| c.ends_with(r"custom-games\sdv.png")));
    // Read-only: not one file was added, removed or changed in size.
    let after: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| (e.file_name(), e.metadata().unwrap().len())).collect();
    assert_eq!(before, after);
}
