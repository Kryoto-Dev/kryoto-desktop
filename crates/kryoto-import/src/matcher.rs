//! Which Kryoto game another launcher's game is, most reliable signal first:
//! the Steam AppID, then the exe's file and folder name, then the title once
//! it is normalized (case, punctuation, edition words, roman numerals,
//! trademark signs), with the year and developer breaking ties. When none of
//! those is sure, the player is asked rather than a guess made.
//!
//! Everything here runs on the PC against an index of Kryoto's catalog that
//! the client downloads whole: nothing about the player's games is sent
//! anywhere to be matched.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// One published game in Kryoto's match index (`/api/desktop/match-index`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CatalogGame {
    pub slug: String,
    pub title: String,
    /// As kryo.to stores it: a beta branch listed as its own game carries a
    /// suffix (`2027330x`).
    pub appid: Option<String>,
    pub developer: Option<String>,
    pub year: Option<u32>,
    /// File names (lower case) of the exes its releases start.
    pub exes: Vec<String>,
    /// An adult game: its art is blurred unless the player shows adult art.
    pub nsfw: bool,
}

/// What is known about the game being matched.
#[derive(Debug, Clone, Default)]
pub struct Candidate<'a> {
    pub title: &'a str,
    pub steam_appid: Option<&'a str>,
    /// Absolute path of its exe.
    pub executable: Option<&'a str>,
    pub developer: Option<&'a str>,
    pub year: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum How {
    SteamAppid,
    Executable,
    Folder,
    Title,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Verdict {
    /// Sure enough to link without asking.
    Linked { slug: String, how: How },
    /// Possibly one of these (best first): the player picks, or keeps it as
    /// their own game.
    Ask { options: Vec<String> },
    /// Not on Kryoto (yet).
    Own,
}

pub struct Index {
    games: Vec<CatalogGame>,
    by_appid: HashMap<String, Vec<usize>>,
    by_title: HashMap<String, Vec<usize>>,
    by_exe: HashMap<String, Vec<usize>>,
    tokens: Vec<Vec<String>>,
}

impl Index {
    pub fn new(games: Vec<CatalogGame>) -> Self {
        let mut by_appid: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_title: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_exe: HashMap<String, Vec<usize>> = HashMap::new();
        let mut tokens = Vec::with_capacity(games.len());
        for (i, g) in games.iter().enumerate() {
            if let Some(a) = g.appid.as_deref().and_then(appid_digits) {
                by_appid.entry(a).or_default().push(i);
            }
            let key = squash(&normalize(&g.title));
            if !key.is_empty() {
                by_title.entry(key).or_default().push(i);
            }
            for exe in &g.exes {
                let name = exe_name(exe);
                if !name.is_empty() && !generic_exe(&name) {
                    let list = by_exe.entry(name).or_default();
                    if !list.contains(&i) {
                        list.push(i);
                    }
                }
            }
            tokens.push(words(&g.title));
        }
        Index { games, by_appid, by_title, by_exe, tokens }
    }

    pub fn get(&self, slug: &str) -> Option<&CatalogGame> {
        self.games.iter().find(|g| g.slug == slug)
    }

    pub fn len(&self) -> usize {
        self.games.len()
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    /// The games with this Steam AppID (a game and its beta-branch listings).
    pub fn by_appid(&self, appid: &str) -> Vec<&CatalogGame> {
        appid_digits(appid).and_then(|a| self.by_appid.get(&a)).map(|l| l.iter().map(|&i| &self.games[i]).collect()).unwrap_or_default()
    }

    pub fn judge(&self, c: &Candidate) -> Verdict {
        let slugs = |list: &[usize]| list.iter().map(|&i| self.games[i].slug.clone()).collect::<Vec<_>>();
        let own_key = squash(&normalize(c.title));

        // 1. The Steam AppID.
        if let Some(appid) = c.steam_appid.and_then(appid_digits) {
            if let Some(list) = self.by_appid.get(&appid) {
                if list.len() == 1 {
                    return linked(&self.games[list[0]], How::SteamAppid);
                }
                // A game and its beta listings share the number: the one
                // stored without a suffix, else the one with the same title.
                let exact: Vec<usize> = list.iter().copied().filter(|&i| self.games[i].appid.as_deref().map(str::trim) == Some(appid.as_str())).collect();
                let titled: Vec<usize> = list.iter().copied().filter(|&i| squash(&normalize(&self.games[i].title)) == own_key).collect();
                if titled.len() == 1 {
                    return linked(&self.games[titled[0]], How::SteamAppid);
                }
                if exact.len() == 1 {
                    return linked(&self.games[exact[0]], How::SteamAppid);
                }
                return Verdict::Ask { options: slugs(list) };
            }
        }

        // 2. The exe's file name, when it is not one every engine uses.
        let exe = c.executable.map(exe_name).filter(|n| !n.is_empty() && !generic_exe(n));
        let folder = c.executable.and_then(game_folder).map(|f| squash(&normalize(&f))).filter(|f| !f.is_empty());
        if let Some(list) = exe.as_ref().and_then(|e| self.by_exe.get(e)) {
            let agreeing: Vec<usize> = list
                .iter()
                .copied()
                .filter(|&i| {
                    let t = squash(&normalize(&self.games[i].title));
                    t == own_key || folder.as_deref() == Some(t.as_str()) || similarity(&words(c.title), &self.tokens[i]) >= 0.5
                })
                .collect();
            if agreeing.len() == 1 {
                return linked(&self.games[agreeing[0]], How::Executable);
            }
            // The same exe under another name (a renamed copy, a nightly
            // build) is likely the game, but not sure: ask.
            return Verdict::Ask { options: slugs(if agreeing.is_empty() { list } else { &agreeing }) };
        }

        // 3. The title, then 4. the folder the game is in.
        for (key, how) in [(Some(own_key.clone()), How::Title), (folder.clone(), How::Folder)] {
            let Some(list) = key.as_ref().filter(|k| !k.is_empty()).and_then(|k| self.by_title.get(k)) else { continue };
            match self.tie_break(list, c) {
                Ok(i) => return linked(&self.games[i], how),
                Err(options) => return Verdict::Ask { options: slugs(&options) },
            }
        }

        // 5. Close but not the same: ask.
        let mine = words(c.title);
        let mut close: Vec<(f32, usize)> = self
            .tokens
            .iter()
            .enumerate()
            .map(|(i, t)| (similarity(&mine, t), i))
            .filter(|(s, i)| *s >= 0.6 && same_numbers(&mine, &self.tokens[*i]))
            .collect();
        close.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        if !close.is_empty() {
            return Verdict::Ask { options: close.iter().take(3).map(|(_, i)| self.games[*i].slug.clone()).collect() };
        }
        Verdict::Own
    }

    /// One game among several with the same normalized title: the one whose
    /// year matches, else whose developer does. A single game is still
    /// refused when both its year and developer disagree (a remake that took
    /// the original's name).
    fn tie_break(&self, list: &[usize], c: &Candidate) -> Result<usize, Vec<usize>> {
        let year_ok = |i: usize| match (self.games[i].year.filter(|y| *y > 0), c.year) {
            (Some(a), Some(b)) => Some(a.abs_diff(b) <= 1),
            _ => None,
        };
        let dev_ok = |i: usize| match (self.games[i].developer.as_deref().filter(|d| !d.is_empty()), c.developer) {
            (Some(a), Some(b)) => Some(squash(&normalize(a)) == squash(&normalize(b))),
            _ => None,
        };
        if list.len() == 1 {
            let i = list[0];
            return if year_ok(i) == Some(false) && dev_ok(i) == Some(false) { Err(vec![i]) } else { Ok(i) };
        }
        for test in [&year_ok as &dyn Fn(usize) -> Option<bool>, &dev_ok] {
            let hits: Vec<usize> = list.iter().copied().filter(|&i| test(i) == Some(true)).collect();
            if hits.len() == 1 {
                return Ok(hits[0]);
            }
        }
        Err(list.to_vec())
    }
}

fn linked(g: &CatalogGame, how: How) -> Verdict {
    Verdict::Linked { slug: g.slug.clone(), how }
}

/// kryo.to's AppID without a branch suffix, when it is one.
pub fn appid_digits(raw: &str) -> Option<String> {
    let d: String = raw.trim().chars().take_while(char::is_ascii_digit).collect();
    (!d.is_empty() && d.len() <= 10 && d != "0").then_some(d)
}

/// The exe's file name, lower case (`D:\Games\Hades\Hades.exe` -> `hades.exe`).
pub fn exe_name(path: &str) -> String {
    path.replace('\\', "/").rsplit('/').next().unwrap_or_default().trim().to_lowercase()
}

/// Names too common to say which game it is.
fn generic_exe(name: &str) -> bool {
    let stem = name.strip_suffix(".exe").unwrap_or(name);
    const GENERIC: &[&str] = &[
        "game", "launcher", "start", "play", "setup", "main", "app", "run", "client", "bin", "win64", "x64", "player",
        "unitycrashhandler64", "unitycrashhandler32", "crashreporter", "crashhandler", "unins000", "nw", "electron",
        "dosbox", "scummvm", "python", "java", "javaw", "renpy", "game-win64-shipping", "start_protected_game",
    ];
    stem.len() < 3 || GENERIC.contains(&stem)
}

/// The folder a game is installed in, from its exe: skipping the
/// subfolders engines put exes in (`bin`, `x64`, `Binaries/Win64`).
pub fn game_folder(exe: &str) -> Option<String> {
    const INNER: &[&str] = &["bin", "bin64", "bin32", "x64", "x86", "win64", "win32", "binaries", "game", "retail", "shipping", "_retail_", "release", "build"];
    let path = exe.replace('\\', "/");
    let mut parent = Path::new(&path).parent()?;
    loop {
        let name = parent.file_name()?.to_string_lossy().to_string();
        if INNER.contains(&name.to_ascii_lowercase().as_str()) {
            parent = parent.parent()?;
            continue;
        }
        return Some(name);
    }
}

/// A title reduced to what tells games apart: lower case, accents and
/// trademark signs off, punctuation to spaces, edition words dropped and
/// roman numerals as digits. Tokens joined by single spaces.
pub fn normalize(title: &str) -> String {
    let mut s = String::with_capacity(title.len());
    for ch in title.chars().flat_map(char::to_lowercase) {
        match ch {
            '™' | '®' | '©' | '℠' => {}
            '\'' | '’' | '‘' | '`' | '´' => {}
            '&' => s.push_str(" and "),
            c if c.is_alphanumeric() => s.push(fold(c)),
            _ => s.push(' '),
        }
    }
    let mut tokens: Vec<&str> = s.split_whitespace().collect();
    let original = tokens.clone();
    // Several-word phrases first.
    for phrase in [&["game", "of", "the", "year"][..], &["directors", "cut"], &["early", "access"], &["collectors", "edition"]] {
        while let Some(at) = tokens.windows(phrase.len()).position(|w| w == phrase) {
            tokens.drain(at..at + phrase.len());
        }
    }
    const EDITION: &[&str] = &["deluxe", "goty", "edition", "definitive", "ultimate", "complete", "gold", "premium", "standard", "digital", "collectors", "bundle"];
    tokens.retain(|t| !EDITION.contains(t));
    if tokens.first() == Some(&"the") && tokens.len() > 1 {
        tokens.remove(0);
    }
    if tokens.is_empty() {
        tokens = original;
    }
    tokens.iter().map(|t| roman(t).map(String::from).unwrap_or_else(|| t.to_string())).collect::<Vec<_>>().join(" ")
}

/// Without spaces, so "GoreBox 27" and "GoreBox27" are one game.
pub fn squash(normalized: &str) -> String {
    normalized.chars().filter(|c| !c.is_whitespace()).collect()
}

/// II to XX as numbers. Not I (the word) or X (Mega Man X is not Mega Man 10).
fn roman(t: &str) -> Option<&'static str> {
    Some(match t {
        "ii" => "2",
        "iii" => "3",
        "iv" => "4",
        "v" => "5",
        "vi" => "6",
        "vii" => "7",
        "viii" => "8",
        "ix" => "9",
        "xi" => "11",
        "xii" => "12",
        "xiii" => "13",
        "xiv" => "14",
        "xv" => "15",
        "xvi" => "16",
        "xvii" => "17",
        "xviii" => "18",
        "xix" => "19",
        "xx" => "20",
        _ => return None,
    })
}

fn fold(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
        'ç' | 'č' | 'ć' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' | 'ń' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
        'ý' | 'ÿ' => 'y',
        'š' | 'ś' => 's',
        'ž' | 'ź' | 'ż' => 'z',
        'ł' => 'l',
        'ß' => 's',
        c => c,
    }
}

fn words(title: &str) -> Vec<String> {
    const STOP: &[&str] = &["the", "of", "a", "an", "and", "to", "in", "on"];
    normalize(title).split(' ').filter(|w| !w.is_empty() && !STOP.contains(w)).map(String::from).collect()
}

/// Dice's coefficient over the two titles' words.
fn similarity(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a: HashSet<&String> = a.iter().collect();
    let b: HashSet<&String> = b.iter().collect();
    2.0 * a.intersection(&b).count() as f32 / (a.len() + b.len()) as f32
}

/// A sequel is another game: the numbers in both titles must agree.
fn same_numbers(a: &[String], b: &[String]) -> bool {
    let nums = |w: &[String]| w.iter().filter(|t| t.chars().all(|c| c.is_ascii_digit())).cloned().collect::<HashSet<_>>();
    nums(a) == nums(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(slug: &str, title: &str, appid: Option<&str>, dev: &str, year: u32, exes: &[&str]) -> CatalogGame {
        CatalogGame {
            slug: slug.into(),
            title: title.into(),
            appid: appid.map(String::from),
            developer: (!dev.is_empty()).then(|| dev.into()),
            year: (year > 0).then_some(year),
            exes: exes.iter().map(|e| e.to_string()).collect(),
            nsfw: false,
        }
    }

    fn index() -> Index {
        Index::new(vec![
            game("the-witcher-3-wild-hunt", "The Witcher 3: Wild Hunt", Some("292030"), "CD PROJEKT RED", 2015, &["witcher3.exe"]),
            game("hades", "Hades", Some("1145360"), "Supergiant Games", 2020, &["Hades.exe"]),
            game("hades-ii", "Hades II", Some("1145350"), "Supergiant Games", 2025, &["Hades2.exe"]),
            game("gta-v", "Grand Theft Auto 5", None, "Rockstar North", 2015, &["GTA5.exe", "PlayGTAV.exe"]),
            game("gorebox27-early-access", "GoreBox27 (early access)", Some("2027330x"), "", 0, &[]),
            game("gorebox", "GoreBox", Some("2027330"), "F2Games", 2023, &[]),
            game("stardew-valley", "Stardew Valley", Some("413150"), "ConcernedApe", 2016, &["Stardew Valley.exe", "Game.exe"]),
            game("re2-1998", "Resident Evil 2", None, "Capcom", 1998, &[]),
            game("re2-2019", "Resident Evil 2", None, "Capcom R&D Division 1", 2019, &[]),
            game("mega-man-10", "Mega Man 10", None, "Capcom", 2010, &[]),
            game("rainbow-six-siege", "Rainbow Six Siege", None, "Ubisoft Montreal", 2015, &[]),
            game("civ-6", "Sid Meier's Civilization 6", None, "Firaxis", 2016, &[]),
            game("celeste", "Celeste", Some("504230"), "Maddy Makes Games", 2018, &[]),
            game("nier-automata", "NieR: Automata", None, "PlatinumGames", 2017, &[]),
            game("pokemon-like", "Pokémon Uranium", None, "", 2016, &[]),
        ])
    }

    fn judge(c: Candidate) -> Verdict {
        index().judge(&c)
    }

    fn linked(slug: &str, how: How) -> Verdict {
        Verdict::Linked { slug: slug.into(), how }
    }

    #[test]
    fn normalizes_real_store_titles() {
        let same = [
            ("The Witcher® 3: Wild Hunt – Game of the Year Edition", "The Witcher 3: Wild Hunt"),
            ("Grand Theft Auto V", "Grand Theft Auto 5"),
            ("Assassin’s Creed® II", "Assassin's Creed 2"),
            ("Sid Meier’s Civilization® VI", "Sid Meier's Civilization 6"),
            ("FINAL FANTASY VII REMAKE INTERGRADE", "Final Fantasy 7 Remake Intergrade"),
            ("DOOM Eternal", "Doom Eternal"),
            ("Pokémon Uranium", "Pokemon Uranium"),
            ("Death Stranding Director's Cut", "DEATH STRANDING"),
            ("Baldur's Gate: Enhanced Edition", "Baldurs Gate Enhanced"),
            ("Dark Souls™ III - Deluxe Edition", "DARK SOULS III"),
        ];
        for (a, b) in same {
            assert_eq!(squash(&normalize(a)), squash(&normalize(b)), "{a} / {b}");
        }
        // Squashed, the spaces and hyphens fall away too.
        assert_eq!(squash(&normalize("Half-Life 2")), squash(&normalize("Half Life 2")));
        assert_eq!(squash(&normalize("NieR:Automata™")), squash(&normalize("NieR: Automata")));
        assert_eq!(squash(&normalize("GoreBox27 (early access)")), squash(&normalize("GoreBox 27")));
        let different = [("Hades", "Hades II"), ("Mega Man X", "Mega Man 10"), ("Portal", "Portal 2"), ("Dark Souls Remastered", "Dark Souls")];
        for (a, b) in different {
            assert_ne!(squash(&normalize(a)), squash(&normalize(b)), "{a} / {b}");
        }
        // A title made only of edition words is kept, not emptied.
        assert_eq!(normalize("Ultimate Edition"), "ultimate edition");
        assert_eq!(normalize("I Am Bread"), "i am bread");
    }

    #[test]
    fn the_steam_appid_wins_over_everything() {
        // A Hydra title that matches nothing, but the AppID is Hades'.
        assert_eq!(judge(Candidate { title: "Hades Supergiant", steam_appid: Some("1145360"), ..Default::default() }), linked("hades", How::SteamAppid));
        // A game and its beta listing share the number: the exact one, or the same title.
        assert_eq!(judge(Candidate { title: "GoreBox", steam_appid: Some("2027330"), ..Default::default() }), linked("gorebox", How::SteamAppid));
        assert_eq!(
            judge(Candidate { title: "GoreBox 27 Early Access", steam_appid: Some("2027330"), ..Default::default() }),
            linked("gorebox27-early-access", How::SteamAppid)
        );
    }

    #[test]
    fn the_exe_or_its_folder_when_there_is_no_appid() {
        assert_eq!(
            judge(Candidate { title: "GTA V", executable: Some(r"E:\Rockstar\Grand Theft Auto V\PlayGTAV.exe"), ..Default::default() }),
            linked("gta-v", How::Executable)
        );
        // Game.exe says nothing; the folder does.
        assert_eq!(
            judge(Candidate { title: "SDV (my copy)", executable: Some(r"D:\Games\Stardew Valley\bin\Game.exe"), ..Default::default() }),
            linked("stardew-valley", How::Folder)
        );
        // The same exe under a name that disagrees: asked, never linked.
        assert_eq!(
            judge(Candidate { title: "Totally Different", executable: Some(r"C:\x\Hades.exe"), ..Default::default() }),
            Verdict::Ask { options: vec!["hades".into()] }
        );
        // A renamed build that still says which game it is: linked by its exe.
        assert_eq!(
            judge(Candidate { title: "Hades nightly", executable: Some(r"C:\Builds\h2\Hades.exe"), ..Default::default() }),
            linked("hades", How::Executable)
        );
    }

    #[test]
    fn titles_with_year_and_developer_as_tie_breakers() {
        assert_eq!(judge(Candidate { title: "The Witcher® 3: Wild Hunt – Game of the Year Edition", ..Default::default() }), linked("the-witcher-3-wild-hunt", How::Title));
        assert_eq!(judge(Candidate { title: "Sid Meier’s Civilization® VI", ..Default::default() }), linked("civ-6", How::Title));
        assert_eq!(judge(Candidate { title: "Resident Evil 2", year: Some(2019), ..Default::default() }), linked("re2-2019", How::Title));
        assert_eq!(judge(Candidate { title: "Resident Evil 2", developer: Some("Capcom"), ..Default::default() }), linked("re2-1998", How::Title));
        // Nothing to break the tie with: ask.
        assert_eq!(judge(Candidate { title: "Resident Evil 2", ..Default::default() }), Verdict::Ask { options: vec!["re2-1998".into(), "re2-2019".into()] });
        // One game of that name, but another year and developer: ask.
        assert_eq!(
            judge(Candidate { title: "Celeste", year: Some(2005), developer: Some("Someone Else"), ..Default::default() }),
            Verdict::Ask { options: vec!["celeste".into()] }
        );
    }

    #[test]
    fn close_titles_are_asked_and_sequels_are_not_confused() {
        let v = judge(Candidate { title: "Tom Clancy's Rainbow Six® Siege", ..Default::default() });
        assert_eq!(v, Verdict::Ask { options: vec!["rainbow-six-siege".into()] });
        // Hades II is never offered for Hades III, nor Mega Man 10 for Mega Man X.
        assert_eq!(judge(Candidate { title: "Hades III", ..Default::default() }), Verdict::Own);
        assert_eq!(judge(Candidate { title: "Mega Man X", ..Default::default() }), Verdict::Own);
        assert_eq!(judge(Candidate { title: "A Game Kryoto Has Never Heard Of", ..Default::default() }), Verdict::Own);
    }

    #[test]
    fn folders_skip_engine_subfolders() {
        assert_eq!(game_folder(r"D:\Games\Hades\x64\Hades.exe").as_deref(), Some("Hades"));
        assert_eq!(game_folder("/home/a/Games/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe").as_deref(), Some("Cyberpunk 2077"));
        assert_eq!(exe_name(r"D:\Games\Hades\x64\Hades.exe"), "hades.exe");
        assert_eq!(appid_digits("165 bonus").as_deref(), Some("165"));
        assert_eq!(appid_digits("x"), None);
    }
}
