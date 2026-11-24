//! `cargo run --example make-lutris-library -- <folder>`: a small Lutris home
//! in `<folder>` (`.local/share/lutris` and `.config/lutris`), for trying
//! Bridge without Lutris: point `KRYOTO_DEV_LUTRIS_DIR` at
//! `<folder>/.local/share/lutris`. One game of each kind: a Wine game with a
//! prefix, a native one with a unicode name, a Steam one, one not installed,
//! and one whose folder is gone. The "games" are EMPTY files: bring them in,
//! never press Play.
use kryoto_import::lutris::testlib::{add, make_db};
use std::path::Path;

fn main() {
    let root = std::env::args().nth(1).expect("the folder to make it in");
    let root = Path::new(&root);
    let data = root.join(".local/share/lutris");
    let config = root.join(".config/lutris");
    std::fs::create_dir_all(&config).unwrap();
    let games = root.join("Games");
    let witcher = games.join("witcher");
    let celeste = games.join("celeste");
    for f in [witcher.join("drive_c/Game/witcher.exe"), celeste.join("Celeste")] {
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"").unwrap();
    }
    let conn = make_db(&data).unwrap();
    let wine = format!("game:\n  exe: drive_c/Game/witcher.exe\n  args: -skipintro\n  prefix: {}\nwine:\n  version: system\nsystem:\n  env:\n    DXVK_HUD: fps\n", witcher.display());
    add(&conn, &config, "the-witcher-test", "The Witcher Test", "wine", &witcher.to_string_lossy(), true, 12.5, 1_763_000_000, Some(&wine)).unwrap();
    let native = format!("game:\n  exe: {}\n", celeste.join("Celeste").display());
    add(&conn, &config, "celeste-test", "Céleste Test 日本", "linux", &celeste.to_string_lossy(), true, 1.0, 0, Some(&native)).unwrap();
    add(&conn, &config, "dota-2", "Dota 2", "steam", "", true, 3.0, 0, Some("game:\n  appid: 570\n")).unwrap();
    add(&conn, &config, "uninstalled", "Not Installed", "linux", "", false, 0.0, 0, None).unwrap();
    let gone = format!("game:\n  exe: {}\n", games.join("gone/gone").display());
    add(&conn, &config, "gone", "Gone Game", "linux", &games.join("gone").to_string_lossy(), true, 0.0, 0, Some(&gone)).unwrap();
    println!("{}", data.display());
}
