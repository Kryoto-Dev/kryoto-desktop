//! online-fix.me, read: which of its game pages is a game, where that page
//! keeps the generic fix (`Fix Repair/`), which game version its release is,
//! and which download links are the site's own. Pure functions over the HTML
//! the site serves, shared by Kryoto Desktop (Properties > Emulator & fixes >
//! Online fix) and Kryoto Forge (its own copies in
//! `kryoto-pipeline/src/onlinefix.rs` predate this file and read the same).
//!
//! How the site works (2026-10-30, see `fixpack.rs` for the archive itself):
//!   * Search and game pages are public. A game page links "Download fix from
//!     server" to `https://uploads.online-fix.me:2053/uploads/<Game>/`, a plain
//!     directory listing anyone can read, with a `Fix Repair/` folder holding
//!     `<Game>_Fix_Repair_Steam_Generic.rar`, and the full release beside it
//!     (`<Game>.v<version>-OFME.rar`), whose name says which game version the
//!     fix is made for.
//!   * The file itself is handed only to a real browser window that opened its
//!     folder (a plain request gets 403, or 401 with an account's cookies). So
//!     nothing here downloads it: a person clicks it in a window the app opens.
//!   * The game page's "Online-Fix Hosters" page lists each file host's copies
//!     as JSON (`data-links`); Pixeldrain's API publishes a file's SHA-256,
//!     which is the only published hash there is for a fix.
//!   * Archives are locked with the password `online-fix.me`.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub const SITE: &str = "https://online-fix.me";

/// A game page on online-fix.me.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Page {
    pub title: String,
    pub url: String,
}

/// A file in one of a game's folders on the uploads host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteFile {
    pub name: String,
    pub url: String,
    /// As the listing writes it ("11M").
    pub size: String,
}

/// The site serves windows-1251; titles are Latin, but the page is decoded
/// byte by byte so nothing breaks on the Cyrillic around them.
pub fn decode_page(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if b < 0x80 { b as char } else { ' ' }).collect()
}

fn html_text(s: &str) -> String {
    s.replace("&#039;", "'").replace("&#39;", "'").replace("&quot;", "\"").replace("&amp;", "&").trim().to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Game pages in a search result page, in order, once each.
pub fn parse_search(html: &str) -> Vec<Page> {
    let re = regex::Regex::new(
        r#"<a[^>]+href="(https://online-fix\.me/games/[a-z0-9_-]+/\d+-[a-z0-9_-]+\.html)"[^>]*>\s*(?:<h2[^>]*>)?\s*([^<\s][^<]{1,199})<"#,
    )
    .expect("static regex");
    let mut seen = std::collections::HashSet::new();
    re.captures_iter(html)
        .map(|c| Page { title: html_text(c[2].trim()), url: c[1].to_string() })
        .filter(|p| !p.title.is_empty() && seen.insert(p.url.clone()))
        .collect()
}

fn plain(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// The page for this game among search results: its title, or the page's
/// own slug (`18079-gamble-with-your-friends-online.html`), the same as the
/// game's once punctuation is gone. Anything looser is a different game; a
/// wrong pick is still refused later, by the fix's own app id.
pub fn pick_page<'a>(title: &str, slug: Option<&str>, pages: &'a [Page]) -> Option<&'a Page> {
    let want = plain(title);
    let want_slug = slug.map(plain).unwrap_or_default();
    if want.is_empty() {
        return None;
    }
    let page_slug = |p: &Page| {
        let file = p.url.rsplit('/').next().unwrap_or("").trim_end_matches(".html");
        let name = file.split_once('-').map(|(_, n)| n).unwrap_or(file);
        plain(name.strip_suffix("-online").unwrap_or(name))
    };
    pages
        .iter()
        .find(|p| plain(&p.title) == want)
        .or_else(|| pages.iter().find(|p| { let s = page_slug(p); s == want || (!want_slug.is_empty() && s == want_slug) }))
}

/// The game's folder on the uploads host, from its page.
pub fn parse_uploads_link(html: &str) -> Option<String> {
    regex::Regex::new(r#"href="(https://uploads\.online-fix\.me:2053/uploads/[^"]+/)""#)
        .expect("static regex")
        .captures(html)
        .map(|c| c[1].to_string())
}

/// A game page that shows a sign-in form where its download links would be.
pub fn wants_signin(html: &str) -> bool {
    let l = html.to_ascii_lowercase();
    parse_uploads_link(html).is_none() && (l.contains("name=\"login_name\"") || l.contains("name=\"login_password\""))
}

/// The archives in a directory listing (nginx's autoindex).
pub fn parse_listing(html: &str, base: &str) -> Vec<RemoteFile> {
    let re = regex::Regex::new(r#"<a href="([^"/]+\.(?:rar|zip|7z))">[^<]*</a>\s+\S+\s+\S+\s+(\S+)"#).expect("static regex");
    re.captures_iter(html)
        .map(|c| RemoteFile { name: percent_decode(&c[1]), url: format!("{base}{}", &c[1]), size: c[2].to_string() })
        .collect()
}

/// The generic Steam fix among a `Fix Repair/` folder's files, else its only one.
pub fn generic_fix(files: &[RemoteFile]) -> Option<&RemoteFile> {
    files
        .iter()
        .find(|f| {
            let n = f.name.to_ascii_lowercase();
            n.contains("generic") && n.contains("steam")
        })
        .or_else(|| files.iter().find(|f| f.name.to_ascii_lowercase().contains("generic")))
        .or(if files.len() == 1 { files.first() } else { None })
}

/// The game version online-fix.me's release is, from its archive names in
/// the game's folder (`Gamble.With.Your.Friends.v1.0.11-OFME.rar`, or
/// `.part1.rar`): the newest, when several are there.
pub fn release_version(names: &[String]) -> Option<String> {
    let re = regex::Regex::new(r"(?i)\.v(\d+(?:\.\d+)*[a-z]?)(?:[-_.]|$)").expect("static regex");
    names
        .iter()
        .filter(|n| !n.to_ascii_lowercase().contains("fix_repair"))
        .filter_map(|n| re.captures(n).map(|c| c[1].to_string()))
        .max_by(|a, b| compare_versions(a, b).unwrap_or(Ordering::Equal))
}

fn numbers(v: &str) -> Option<Vec<u64>> {
    let re = regex::Regex::new(r"\d+(?:\.\d+)+|^\d+$").expect("static regex");
    let m = re.find(v.trim().trim_start_matches(['v', 'V']))?;
    m.as_str().split('.').map(|p| p.parse().ok()).collect()
}

/// Two game versions in order, when both are dotted numbers ("1.0.11" and
/// "1.0.9"; "v" before them is fine). None when either is a build number or
/// anything else that cannot be put in order.
pub fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (numbers(a)?, numbers(b)?);
    let len = a.len().max(b.len());
    let pad = |v: &[u64]| (0..len).map(|i| v.get(i).copied().unwrap_or(0)).collect::<Vec<_>>();
    Some(pad(&a).cmp(&pad(&b)))
}

/// online-fix.me or one of its own hosts (uploads, hosters), over HTTPS.
pub fn is_online_fix(url: &url::Url) -> bool {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    url.scheme() == "https" && (host == "online-fix.me" || host.ends_with(".online-fix.me"))
}

/// An archive from online-fix.me's own hosts: nothing else is ever taken.
pub fn acceptable(url: &url::Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    is_online_fix(url) && [".rar", ".zip", ".7z"].iter().any(|e| path.ends_with(e))
}

/// The same file, whatever query or fragment a page adds.
pub fn same_file(a: &url::Url, b: &url::Url) -> bool {
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default() && a.path() == b.path()
}

/// The folder a file is in.
pub fn folder_of(url: &url::Url) -> url::Url {
    let mut f = url.clone();
    f.set_path(&url.path()[..url.path().rfind('/').unwrap_or(0) + 1]);
    f.set_query(None);
    f.set_fragment(None);
    f
}

/// A name for a downloaded file, safe on every file system.
pub fn file_name(url: &url::Url) -> String {
    let raw = url.path_segments().and_then(|mut s| s.next_back()).map(percent_decode).unwrap_or_default();
    let n: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    if n.trim_matches(['.', '_']).is_empty() {
        "online-fix.rar".into()
    } else {
        n
    }
}

/// The game's "Online-Fix Hosters" page, from its game page.
pub fn parse_hosters_link(html: &str) -> Option<String> {
    regex::Regex::new(r#"href="(https://hosters\.online-fix\.me:2053/[^"]+)""#).expect("static regex").captures(html).map(|c| c[1].to_string())
}

/// A file on one host, as Online-Fix Hosters lists it.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct HostedFile {
    pub direct_link: String,
    pub file_name: String,
    #[serde(default)]
    pub is_dangerous: bool,
}

/// Each host on an Online-Fix Hosters page with its files, in page order.
pub fn parse_hosters(html: &str) -> Vec<(String, Vec<HostedFile>)> {
    let re = regex::Regex::new(r#"(?s)data-links='([^']*)'.*?>\s*([^<]+?)\s*</div>"#).expect("static regex");
    re.captures_iter(html)
        .filter_map(|c| Some((html_text(&c[2]), serde_json::from_str::<Vec<HostedFile>>(&html_text(&c[1])).ok()?)))
        .collect()
}

/// `https://pixeldrain.com/u/<id>` -> `<id>`.
pub fn pixeldrain_id(link: &str) -> Option<&str> {
    let id = link.strip_prefix("https://pixeldrain.com/u/")?;
    (!id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric())).then_some(id)
}

/// The Pixeldrain copy of `name` on a Hosters page, when it lists one.
pub fn pixeldrain_copy<'a>(hosts: &'a [(String, Vec<HostedFile>)], name: &str) -> Option<&'a str> {
    hosts
        .iter()
        .filter(|(host, _)| host.eq_ignore_ascii_case("pixeldrain"))
        .flat_map(|(_, files)| files.iter())
        .find(|f| !f.is_dangerous && f.file_name.eq_ignore_ascii_case(name))
        .and_then(|f| pixeldrain_id(&f.direct_link))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_this_games_page_and_no_other() {
        let html = r#"<a href="https://online-fix.me/games/adventures/18079-gamble-with-your-friends-online.html"><h2 class="title">Gamble With Your Friends</h2></a>
<a href="https://online-fix.me/games/adventures/18080-gamble-with-your-friends-2-online.html"><h2>Gamble With Your Friends 2</h2></a>
<a href="https://online-fix.me/games/survival/16179-raft-po-seti.html"><h2>Raft  </h2></a>"#;
        let pages = parse_search(html);
        assert_eq!(pages.len(), 3);
        assert_eq!(pick_page("Gamble With Your Friends", None, &pages).unwrap().url, pages[0].url);
        assert_eq!(pick_page("Gamble With Your Friends 2", None, &pages).unwrap().url, pages[1].url);
        assert_eq!(pick_page("Raft", None, &pages).unwrap().url, pages[2].url);
        assert!(pick_page("Gamble", None, &pages).is_none(), "a part of a title is another game");
        // By the page's slug when the title on the page is written differently.
        let odd = vec![Page { title: "Scam with ur Friends".into(), url: "https://online-fix.me/games/x/1-scam-with-your-friends-online.html".into() }];
        assert!(pick_page("Scam With Your Friends", Some("scam-with-your-friends"), &odd).is_some());
    }

    #[test]
    fn reads_the_folders_and_the_version_the_fix_is_made_for() {
        let game = r#"<a target="_blank" href="https://uploads.online-fix.me:2053/uploads/Gamble%20With%20Your%20Friends/" class="btn">Download fix from server</a>"#;
        let base = parse_uploads_link(game).unwrap();
        assert!(!wants_signin(game));
        assert!(wants_signin(r#"<form><input name="login_name"><input name="login_password"></form>"#));
        let listing = r#"<pre><a href="../">../</a>
<a href="Fix%20Repair/">Fix Repair/</a>                                         04-May-2026 09:14       -
<a href="Gamble.With.Your.Friends.v1.0.9-OFME.rar">Gamble.With.Your.Friends.v1.0.9-OFME.rar</a> 01-May-2026 09:14     790M
<a href="Gamble.With.Your.Friends.v1.0.11-OFME.part1.rar">Gamble.With.Your.Friends.v1.0.11-OFME.part1.rar</a> 04-May-2026 09:14     800M
</pre>"#;
        let names: Vec<String> = parse_listing(listing, &base).into_iter().map(|f| f.name).collect();
        assert_eq!(release_version(&names).as_deref(), Some("1.0.11"), "the newest, not the first");
        let fixes = parse_listing(
            r#"<a href="GambleWithYourFriends_Fix_Repair_Steam_Generic.rar">GambleWithYourFriends_Fix_Repair_Steam_Generic.rar</a> 04-May-2026 09:14     11M"#,
            &format!("{base}Fix%20Repair/"),
        );
        let fix = generic_fix(&fixes).unwrap();
        assert_eq!(fix.size, "11M");
        assert!(fix.url.ends_with("/Fix%20Repair/GambleWithYourFriends_Fix_Repair_Steam_Generic.rar"));
    }

    #[test]
    fn puts_versions_in_order_only_when_it_can() {
        assert_eq!(compare_versions("1.0.11", "1.0.9"), Some(Ordering::Greater));
        assert_eq!(compare_versions("v1.2", "1.2.0"), Some(Ordering::Equal));
        assert_eq!(compare_versions("1.0.11", "6000.3.15.43652 (b25306743)"), Some(Ordering::Less));
        assert_eq!(compare_versions("1.0.11", "b26004539"), None, "a build number has no order against a version");
    }

    #[test]
    fn takes_only_archives_from_online_fix() {
        let ok: url::Url = "https://uploads.online-fix.me:2053/uploads/G/Fix%20Repair/X_Fix_Repair_Steam_Generic.rar".parse().unwrap();
        assert!(acceptable(&ok));
        for bad in ["https://ads.example.com/setup.exe", "https://online-fix.me.evil.example/fix.rar", "https://uploads.online-fix.me:2053/uploads/x/Setup.exe", "http://uploads.online-fix.me/uploads/x/fix.rar"] {
            assert!(!acceptable(&bad.parse().unwrap()), "{bad}");
        }
        let mut again = ok.clone();
        again.set_query(Some("x=1"));
        assert!(same_file(&again, &ok));
        assert_eq!(folder_of(&ok).path(), "/uploads/G/Fix%20Repair/");
        assert_eq!(file_name(&ok), "X_Fix_Repair_Steam_Generic.rar");
        assert_eq!(file_name(&"https://online-fix.me/a/..".parse().unwrap()), "online-fix.rar");
    }

    #[test]
    fn finds_a_published_hash_only_on_pixeldrain() {
        let html = r#"<div class="option selected" data-links='[{"direct_link": "https://pixeldrain.com/u/4jbkTxQv", "file_name": "Game_Fix_Repair_Steam_Generic.rar", "id": 1, "is_dangerous": false}]' data-id="1">Pixeldrain</div>
<div class="option" data-links='[{"direct_link": "https://rootz.so/d/x", "file_name": "Game_Fix_Repair_Steam_Generic.rar", "id": 3, "is_dangerous": true}]'>Rootz</div>"#;
        let hosts = parse_hosters(html);
        assert_eq!(pixeldrain_copy(&hosts, "game_fix_repair_steam_generic.rar"), Some("4jbkTxQv"));
        assert_eq!(pixeldrain_copy(&hosts, "Other.rar"), None);
        assert_eq!(pixeldrain_id("https://pixeldrain.com/u/../x"), None);
    }
}
