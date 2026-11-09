//! `cargo run --example hydra-dump -- <hydra-db folder>`: what Kryoto reads
//! from a Hydra library, for checking the reader against a real one.
fn main() {
    let dir = std::env::args().nth(1).expect("the hydra-db folder");
    let all = kryoto_import::leveldb::read_all(std::path::Path::new(&dir)).expect("read");
    let mut prefixes = std::collections::BTreeMap::<String, usize>::new();
    for k in all.keys() {
        let k = String::from_utf8_lossy(k);
        let p = k.split('!').nth(1).unwrap_or("").to_string();
        *prefixes.entry(p).or_default() += 1;
    }
    println!("{} keys: {prefixes:?}", all.len());
    for g in kryoto_import::hydra::read_library(std::path::Path::new(&dir)).expect("games") {
        println!(
            "{:<28} {:<40} appid={:<9} {:>6}s exe={} last={:?} cover={} dev={:?} year={:?}",
            g.key, g.title, g.steam_appid.unwrap_or_default(), g.playtime_seconds,
            g.executable.as_deref().map(|e| std::path::Path::new(e).exists()).map(|b| if b { "on-disk" } else { "missing" }).unwrap_or("none"),
            g.last_played, g.cover.is_some(), g.developer, g.year
        );
    }
}
