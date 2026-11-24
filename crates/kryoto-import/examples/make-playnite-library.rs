//! `cargo run --example make-playnite-library -- <folder>`: a small Playnite
//! library in `<folder>\library` for trying Bridge without Playnite (point
//! `KRYOTO_DEV_PLAYNITE_DIR` at it), with one game of each kind: a game Kryoto
//! can bring in, one with a unicode name and a relative path, a Steam game
//! Playnite's add-on starts, a link, one with no play action, and one whose
//! folder is gone. The "games" are EMPTY files: bring them in, never press Play.
use kryoto_import::bson::Bson;
use kryoto_import::playnite::testlib::{action, guid};
use std::path::Path;

fn main() {
    let root = std::env::args().nth(1).expect("the folder to make it in");
    let root = Path::new(&root);
    let games = root.join("Games");
    for (dir, exe) in [("Lantern Rite Test", "LanternRite.exe"), ("Okami Test", "bin\\okami.exe")] {
        let file = games.join(dir).join(exe);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"").unwrap();
    }
    let lib = root.join("library");
    let dir = |d: &str| Bson::Str(games.join(d).to_string_lossy().into_owned());
    let s = |v: &str| Bson::Str(v.into());
    kryoto_import::playnite::write_library(
        &lib,
        &[
            vec![
                ("_id", Bson::Binary(4, guid(1))),
                ("Name", s("Lantern Rite Test")),
                ("InstallDirectory", dir("Lantern Rite Test")),
                ("IsInstalled", Bson::Bool(true)),
                ("Playtime", Bson::Int64(5_400)),
                ("LastActivity", Bson::DateTime(1_763_000_000_000)),
                ("GameActions", Bson::Array(vec![action("File", "{InstallDir}\\LanternRite.exe", Some("-windowed"), true)])),
            ],
            vec![
                ("_id", Bson::Binary(4, guid(2))),
                ("Name", s("Ōkami Test 大神")),
                ("InstallDirectory", dir("Okami Test")),
                ("IsInstalled", Bson::Bool(true)),
                ("Playtime", Bson::Int64(600)),
                ("GameActions", Bson::Array(vec![action("File", "bin\\okami.exe", None, true)])),
            ],
            vec![
                ("_id", Bson::Binary(4, guid(3))),
                ("Name", s("Hades")),
                ("PluginId", Bson::Binary(4, vec![0xc9, 0xdf, 0x91, 0xcb, 0x77, 0xb9, 0xbf, 0x43, 0x8e, 0x70, 0x55, 0xf4, 0x6e, 0x41, 0x0f, 0xab])),
                ("GameId", s("1145360")),
                ("IsInstalled", Bson::Bool(true)),
            ],
            vec![("_id", Bson::Binary(4, guid(4))), ("Name", s("Web Game")), ("GameActions", Bson::Array(vec![action("URL", "https://example.com/play", None, true)]))],
            vec![("_id", Bson::Binary(4, guid(5))), ("Name", s("No Play Action")), ("IsInstalled", Bson::Bool(true))],
            vec![
                ("_id", Bson::Binary(4, guid(6))),
                ("Name", s("Gone Game")),
                ("InstallDirectory", dir("Gone Game")),
                ("IsInstalled", Bson::Bool(true)),
                ("GameActions", Bson::Array(vec![action("File", "{InstallDir}\\gone.exe", None, true)])),
            ],
        ],
    )
    .unwrap();
    println!("{}", lib.display());
}
