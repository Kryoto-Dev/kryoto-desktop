//! What Discord is told, from the player's settings and a library game. Pure,
//! so every rule about what other people see is tested here.
//!
//! Limits are Discord's: two buttons with labels up to 32 characters and
//! addresses up to 512, text lines of 2 to 128 characters, a picture address
//! up to 256. A payload over any of them is refused whole.

use crate::library::LibraryGame;
use crate::settings::{DiscordPresence, PresenceButton};
use serde_json::{json, Map, Value};

const SITE: &str = "https://kryo.to";
/// The K// mark. A plain address: nothing has to be uploaded to Discord.
pub const LOGO: &str = "https://kryo.to/icon-512x512.png";
/// How long each button of a `rotate` slot shows. Discord takes five updates in
/// twenty seconds; this is one in five minutes.
pub const TURN_SECONDS: u64 = 300;
/// What an adult game shows, unless the player says otherwise.
const NEUTRAL: &str = "Playing a game";
const BROWSING: &str = "Browsing Kryoto";

/// Who is signed in. `profile` is None for a guest or an anonymous account.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Who {
    pub profile: Option<String>,
}

impl Who {
    pub fn new(username: Option<&str>, anonymous: bool) -> Who {
        Who { profile: username.filter(|_| !anonymous).map(String::from) }
    }
}

fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// A line Discord will take, or None. `{game}` alone can be one letter long,
/// and Discord wants two.
fn line(text: &str) -> Option<String> {
    let text = cut(text.trim(), 128);
    match text.chars().count() {
        0 => None,
        1 => Some(format!("{text}\u{2800}")),
        _ => Some(text),
    }
}

/// "12 hours", "1 hour", "45 minutes"; nothing under a minute.
pub fn hours(seconds: u64) -> String {
    let plural = |n: u64, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    match seconds {
        0..=59 => String::new(),
        60..=3599 => plural(seconds / 60, "minute"),
        _ => plural(seconds / 3600, "hour"),
    }
}

/// A template with its placeholders filled in. A line that leans on something
/// empty goes whole: "{version}" on a game with no version, say.
fn fill(template: &str, values: &[(&str, String)]) -> Option<String> {
    let mut out = template.to_string();
    for (key, value) in values {
        let token = format!("{{{key}}}");
        if out.contains(&token) {
            if value.trim().is_empty() {
                return None;
            }
            out = out.replace(&token, value);
        }
    }
    line(&out)
}

/// An adult game whose name and art stay off Discord.
fn neutral(s: &DiscordPresence, g: &LibraryGame) -> bool {
    g.nsfw && !s.adult_names && g.discord_show != Some(true)
}

/// Why Discord does not show a game as itself, or None when it does. Every
/// place that says so asks here.
pub fn hidden_because(s: &DiscordPresence, g: &LibraryGame) -> Option<&'static str> {
    if !s.on {
        Some("off")
    } else if g.discord_show == Some(false) {
        Some("game")
    } else if g.hidden && g.discord_show != Some(true) {
        Some("hidden")
    } else if neutral(s, g) {
        Some("adult")
    } else {
        None
    }
}

/// What to keep for a game the player just flipped: nothing, when that is
/// what Settings would do anyway.
pub fn choice(s: &DiscordPresence, g: &LibraryGame, show: bool) -> Option<bool> {
    let usual = !g.hidden && (!g.nsfw || s.adult_names);
    (show != usual).then_some(show)
}

/// A picture Discord can fetch itself: never a file on this PC.
fn picture(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let url = if raw.starts_with("//") {
        format!("https:{raw}")
    } else if raw.starts_with('/') {
        format!("{SITE}{raw}")
    } else {
        raw.to_string()
    };
    (url.starts_with("https://") && url.len() <= 256).then_some(url)
}

fn cover(g: &LibraryGame) -> Option<String> {
    [&g.custom_cover, &g.cover, &g.header, &g.custom_hero, &g.hero].into_iter().flatten().find_map(|u| picture(u))
}

/// A button's address and label, or None when it has nothing to point at.
fn button(b: &PresenceButton, game: Option<&LibraryGame>, who: &Who) -> Option<Value> {
    let (fallback, url) = match b.kind.as_str() {
        "desktop" => ("Get Kryoto Desktop", format!("{SITE}/desktop")),
        "game" => ("Open in Kryoto", format!("{SITE}/game/{}", game?.slug.as_deref()?)),
        "donate" => ("Donate to Kryoto", format!("{SITE}/donate")),
        "profile" => ("My Kryoto profile", format!("{SITE}/user/{}", url::form_urlencoded::byte_serialize(who.profile.as_deref()?.as_bytes()).collect::<String>())),
        "custom" => {
            let url = b.url.trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 512 {
                return None;
            }
            ("", url.to_string())
        }
        _ => return None,
    };
    let label = cut(b.label.trim(), 32);
    let label = if label.is_empty() { fallback.to_string() } else { label };
    (!label.is_empty()).then(|| json!({ "label": label, "url": url }))
}

/// Which turn a game started at `started` is on. From the start time, so the
/// same moment always gives the same button and nothing has to be remembered.
pub fn turn(started: u64, now: u64, seconds: u64) -> u64 {
    now.saturating_sub(started) / seconds.max(1)
}

/// The game's page and Donate, taking turns. A game with no page to link, or
/// one that must not be named, just shows Donate.
fn rotating(game: Option<&LibraryGame>, who: &Who, turn: u64) -> Option<Value> {
    let of = |kind: &str| button(&PresenceButton { kind: kind.into(), ..Default::default() }, game, who);
    let options: Vec<Value> = [of("game"), of("donate")].into_iter().flatten().collect();
    options.get((turn % options.len().max(1) as u64) as usize).cloned()
}

fn buttons(s: &DiscordPresence, game: Option<&LibraryGame>, who: &Who, turn: u64) -> Vec<Value> {
    [&s.button1, &s.button2].into_iter().filter_map(|b| if b.kind == "rotate" { rotating(game, who, turn) } else { button(b, game, who) }).collect()
}

fn finish(details: Option<String>, state: Option<String>, assets: Map<String, Value>, since: Option<u64>, buttons: Vec<Value>) -> Value {
    let mut a = Map::new();
    a.insert("type".into(), json!(0));
    // "Playing <this line>" in the member list, instead of the application's name.
    a.insert("status_display_type".into(), json!(2));
    if let Some(d) = details {
        a.insert("details".into(), json!(d));
    }
    if let Some(s) = state {
        a.insert("state".into(), json!(s));
    }
    if !assets.is_empty() {
        a.insert("assets".into(), Value::Object(assets));
    }
    if let Some(start) = since {
        a.insert("timestamps".into(), json!({ "start": start }));
    }
    if !buttons.is_empty() {
        a.insert("buttons".into(), Value::Array(buttons));
    }
    Value::Object(a)
}

fn logo_assets() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("large_image".into(), json!(LOGO));
    m.insert("large_text".into(), json!("Kryoto"));
    m
}

/// A game being played.
pub fn for_game(s: &DiscordPresence, g: &LibraryGame, started: u64, turn: u64, who: &Who) -> Value {
    let since = s.elapsed.then_some(started);
    // Neither its name, its art, nor a page that names it, whatever the
    // templates say. The player's own link stays.
    if neutral(s, g) {
        let links = buttons(s, None, who, turn);
        return finish(line(NEUTRAL), None, logo_assets(), since, links);
    }
    // A beta branch is another game to the people watching too (GoreBox27).
    let title = crate::library::title_with_branch(&g.title, g.steam_branch.as_deref());
    let values = [
        ("game", title.clone()),
        ("playtime", if s.playtime { hours(g.playtime_seconds) } else { String::new() }),
        ("version", g.version.clone().unwrap_or_default()),
        ("developer", g.developer.clone().unwrap_or_default()),
    ];
    let details = fill(&s.line1, &values).or_else(|| line(&title));
    let state = fill(&s.line2, &values);
    let mut assets = Map::new();
    match s.picture.as_str() {
        "none" => {}
        "kryoto" => assets = logo_assets(),
        _ => match cover(g) {
            Some(url) => {
                assets.insert("large_image".into(), json!(url));
                if let Some(text) = line(&title) {
                    assets.insert("large_text".into(), json!(text));
                }
                assets.insert("small_image".into(), json!(LOGO));
                assets.insert("small_text".into(), json!("Kryoto"));
            }
            None => assets = logo_assets(),
        },
    }
    finish(details, state, assets, since, buttons(s, Some(g), who, turn))
}

/// Kryoto open with no game, for a player who asked for it.
pub fn browsing(s: &DiscordPresence, since: u64, who: &Who) -> Value {
    finish(line(BROWSING), None, logo_assets(), s.elapsed.then_some(since), buttons(s, None, who, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> LibraryGame {
        LibraryGame {
            title: "Hades".into(),
            slug: Some("hades".into()),
            cover: Some("https://cdn.example/hades.jpg".into()),
            version: Some("1.38".into()),
            developer: Some("Supergiant Games".into()),
            playtime_seconds: 45_000,
            ..Default::default()
        }
    }

    fn who() -> Who {
        Who::new(Some("mira"), false)
    }

    #[test]
    fn the_defaults_show_the_game_cover_time_and_both_buttons() {
        let a = for_game(&DiscordPresence::default(), &game(), 1_700_000_000, 0, &who());
        assert_eq!(a["details"], "Hades");
        assert_eq!(a["state"], "12 hours played");
        assert_eq!(a["assets"]["large_image"], "https://cdn.example/hades.jpg");
        assert_eq!(a["assets"]["small_image"], LOGO);
        assert_eq!(a["timestamps"]["start"], 1_700_000_000u64);
        assert_eq!(a["buttons"], json!([{ "label": "Get Kryoto Desktop", "url": "https://kryo.to/desktop" }, { "label": "Open in Kryoto", "url": "https://kryo.to/game/hades" }]));
    }

    #[test]
    fn the_second_button_takes_turns_and_the_clock_stays_put() {
        let s = DiscordPresence::default();
        let at = |t| for_game(&s, &game(), 1_700_000_000, t, &who());
        let second = |t| at(t)["buttons"][1]["label"].as_str().unwrap().to_string();
        assert_eq!((second(0), second(1), second(2)), ("Open in Kryoto".to_string(), "Donate to Kryoto".to_string(), "Open in Kryoto".to_string()));
        assert_eq!(at(0)["buttons"][0], at(1)["buttons"][0], "the first button never moves");
        assert_eq!(at(0)["timestamps"], at(7)["timestamps"], "a turn does not restart the clock");
        // Turns come from the time played, and only change every few minutes.
        assert_eq!((turn(100, 100, TURN_SECONDS), turn(100, 399, TURN_SECONDS), turn(100, 400, TURN_SECONDS)), (0, 0, 1));
        assert_eq!(turn(500, 100, TURN_SECONDS), 0, "a clock set back stays on the first turn");
        // Nothing to turn to: no page, or a game that must not be named.
        let own = LibraryGame { slug: None, ..game() };
        let adult = LibraryGame { nsfw: true, ..game() };
        for g in [own, adult] {
            for t in 0..3 {
                assert_eq!(for_game(&s, &g, 1, t, &who())["buttons"][1]["label"], "Donate to Kryoto");
            }
        }
    }

    #[test]
    fn a_player_who_picks_their_own_buttons_gets_exactly_those() {
        let custom = PresenceButton { kind: "custom".into(), label: "My stream".into(), url: "https://example.com/live".into() };
        let donate = PresenceButton { kind: "donate".into(), ..Default::default() };
        let s = DiscordPresence { button1: custom, button2: donate, ..Default::default() };
        assert_eq!(for_game(&s, &game(), 1, 0, &who())["buttons"], for_game(&s, &game(), 1, 1, &who())["buttons"]);
    }

    #[test]
    fn templates_fill_in_and_a_line_with_nothing_to_say_goes() {
        let s = DiscordPresence { line1: "{game} by {developer}".into(), line2: "Build {version}, {playtime}".into(), ..Default::default() };
        let a = for_game(&s, &game(), 1, 0, &who());
        assert_eq!((a["details"].as_str(), a["state"].as_str()), (Some("Hades by Supergiant Games"), Some("Build 1.38, 12 hours")));
        let bare = LibraryGame { version: None, playtime_seconds: 30, ..game() };
        assert!(for_game(&s, &bare, 1, 0, &who()).get("state").is_none());
        // A line 1 with nothing left still names the game.
        let s = DiscordPresence { line1: "{version}".into(), ..Default::default() };
        assert_eq!(for_game(&s, &bare, 1, 0, &who())["details"], "Hades");
        // Off means off, whatever the template asks for.
        let s = DiscordPresence { playtime: false, ..Default::default() };
        assert!(for_game(&s, &game(), 1, 0, &who()).get("state").is_none());
        let s = DiscordPresence { elapsed: false, ..Default::default() };
        assert!(for_game(&s, &game(), 1, 0, &who()).get("timestamps").is_none());
    }

    #[test]
    fn an_adult_game_shows_no_name_art_or_page_whatever_the_templates_say() {
        let adult = LibraryGame { title: "Secret Title".into(), slug: Some("secret-title".into()), nsfw: true, ..game() };
        let s = DiscordPresence { line1: "{game} {developer}".into(), line2: "{game}".into(), ..Default::default() };
        let a = for_game(&s, &adult, 1, 0, &who());
        let sent = a.to_string();
        assert!(!sent.contains("Secret") && !sent.contains("secret-title") && !sent.contains("cdn.example") && !sent.contains("Supergiant"), "{sent}");
        assert_eq!(a["details"], NEUTRAL);
        assert_eq!(a["buttons"], json!([{ "label": "Get Kryoto Desktop", "url": "https://kryo.to/desktop" }, { "label": "Donate to Kryoto", "url": "https://kryo.to/donate" }]));
        // The player can choose to show them all, or just this one, and then
        // it works like any other game.
        let s = DiscordPresence { adult_names: true, ..Default::default() };
        let a = for_game(&s, &adult, 1, 0, &who());
        assert_eq!((a["details"].as_str(), a["assets"]["large_image"].as_str()), (Some("Secret Title"), Some("https://cdn.example/hades.jpg")));
        assert_eq!(a["buttons"][1]["url"], "https://kryo.to/game/secret-title");
        let this_one = LibraryGame { discord_show: Some(true), ..adult.clone() };
        assert_eq!(for_game(&DiscordPresence::default(), &this_one, 1, 0, &who())["details"], "Secret Title");
    }

    #[test]
    fn a_game_kryoto_does_not_list_has_no_page_to_link_and_a_file_is_no_picture() {
        let own = LibraryGame { slug: None, cover: Some("kryoto-art://abc".into()), custom_cover: Some(r"C:\art\a.png".into()), ..game() };
        let a = for_game(&DiscordPresence::default(), &own, 1, 0, &who());
        assert_eq!(a["buttons"], json!([{ "label": "Get Kryoto Desktop", "url": "https://kryo.to/desktop" }, { "label": "Donate to Kryoto", "url": "https://kryo.to/donate" }]));
        assert_eq!(a["assets"]["large_image"], LOGO);
        // kryo.to's own paths, and an address too long for Discord.
        assert_eq!(picture("/api/steam/image.jpg?x=1").as_deref(), Some("https://kryo.to/api/steam/image.jpg?x=1"));
        assert_eq!(picture(&format!("https://a.example/{}", "x".repeat(300))), None);
        assert_eq!(picture("http://a.example/x.png"), None);
    }

    #[test]
    fn buttons_can_change_go_and_never_show_an_anonymous_profile() {
        let profile = PresenceButton { kind: "profile".into(), ..Default::default() };
        let s = DiscordPresence { button1: profile.clone(), button2: PresenceButton { kind: "none".into(), ..Default::default() }, ..Default::default() };
        assert_eq!(for_game(&s, &game(), 1, 0, &who())["buttons"], json!([{ "label": "My Kryoto profile", "url": "https://kryo.to/user/mira" }]));
        assert!(for_game(&s, &game(), 1, 0, &Who::new(Some("mira"), true)).get("buttons").is_none());
        assert!(for_game(&s, &game(), 1, 0, &Who::new(None, false)).get("buttons").is_none());

        let custom = |label: &str, url: &str| PresenceButton { kind: "custom".into(), label: label.into(), url: url.into() };
        let s = DiscordPresence { button1: custom("My stream with a very very long label indeed", "https://twitch.tv/me"), button2: custom("Bad", "javascript:alert(1)"), ..Default::default() };
        let b = &for_game(&s, &game(), 1, 0, &who())["buttons"];
        assert_eq!(b.as_array().unwrap().len(), 1);
        assert_eq!(b[0]["label"].as_str().unwrap().chars().count(), 32);
        // A custom link needs a label of its own.
        let s = DiscordPresence { button1: custom("", "https://twitch.tv/me"), button2: PresenceButton::default(), ..Default::default() };
        assert!(for_game(&s, &game(), 1, 0, &who()).get("buttons").is_none());
    }

    #[test]
    fn one_answer_for_why_a_game_is_hidden_and_a_flip_keeps_only_what_differs() {
        let s = DiscordPresence::default();
        let off = DiscordPresence { on: false, ..Default::default() };
        let adult = LibraryGame { nsfw: true, ..game() };
        assert_eq!(hidden_because(&s, &game()), None);
        assert_eq!(hidden_because(&off, &game()), Some("off"));
        assert_eq!(hidden_because(&s, &LibraryGame { discord_show: Some(false), ..game() }), Some("game"));
        assert_eq!(hidden_because(&s, &adult), Some("adult"));
        assert_eq!(hidden_because(&s, &LibraryGame { discord_show: Some(true), ..adult.clone() }), None);
        assert_eq!(hidden_because(&DiscordPresence { adult_names: true, ..Default::default() }, &adult), None);
        // Hiding a game is kept; showing it again is back to the usual.
        assert_eq!((choice(&s, &game(), false), choice(&s, &game(), true)), (Some(false), None));
        // A hidden game: nothing on Discord until the player says, and then just that.
        let secret = LibraryGame { hidden: true, ..game() };
        assert_eq!(hidden_because(&s, &secret), Some("hidden"));
        assert_eq!(hidden_because(&s, &LibraryGame { discord_show: Some(true), ..secret.clone() }), None);
        assert_eq!((choice(&s, &secret, true), choice(&s, &secret, false)), (Some(true), None));

        // An adult game: showing is the exception, hiding is the usual...
        assert_eq!((choice(&s, &adult, true), choice(&s, &adult, false)), (Some(true), None));
        // ...until Settings shows them all.
        let all = DiscordPresence { adult_names: true, ..Default::default() };
        assert_eq!((choice(&all, &adult, false), choice(&all, &adult, true)), (Some(false), None));
    }

    #[test]
    fn lines_stay_inside_discords_limits() {
        assert_eq!(line("Q").unwrap().chars().count(), 2);
        assert_eq!(line(&"x".repeat(300)).unwrap().chars().count(), 128);
        assert_eq!(line("   "), None);
        assert_eq!((hours(59).as_str(), hours(60).as_str(), hours(3600).as_str(), hours(7300).as_str()), ("", "1 minute", "1 hour", "2 hours"));
    }

    #[test]
    fn browsing_names_no_game() {
        let a = browsing(&DiscordPresence::default(), 5, &who());
        assert_eq!(a["details"], BROWSING);
        assert_eq!(a["buttons"], json!([{ "label": "Get Kryoto Desktop", "url": "https://kryo.to/desktop" }, { "label": "Donate to Kryoto", "url": "https://kryo.to/donate" }]));
    }
}
