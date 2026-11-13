//! Discord's own web pages never open inside Kryoto (KB-0312).
//!
//! A player asked why Kryoto Desktop wanted their Discord password. Nothing of
//! Kryoto's does: Rich Presence talks to the Discord app on this PC
//! (`discord_rpc`), which needs no sign-in. But the Store is kryo.to, and
//! kryo.to's "Continue with Discord" (sign-in, the invite page) and "Link
//! Discord" (Settings > Connections, the donor role) go to Discord's sign-in,
//! as do plain links to discord.com and discord.gg (profiles, suggestion
//! threads, the server invite) when they open in the same tab. The Store has
//! no Discord session, so Discord asked for an email and a password inside a
//! window that is all Kryoto, which is what a fake sign-in looks like too.
//!
//! So the Store never loads them: they open in the browser, where the address
//! bar is Discord's and the player is usually signed in to Discord already.
//! kryo.to's sign-in with Discord opens there from its start (`/api/auth/discord`),
//! so the whole round trip, its state cookie included, is the browser's.

use url::Url;

/// What to do with a navigation of the Store.
#[derive(Debug, PartialEq, Eq)]
pub enum Outside {
    /// Not Discord's: the Store's usual rules apply.
    No,
    /// Open this in the browser instead, and say so.
    Browser(Url),
}

/// Discord's web hosts: the site, the web app, invites, its old domain.
/// Pictures (cdn.discordapp.com, media.discordapp.net) are left alone: they
/// carry no sign-in, and pages show them inline.
pub fn is_discord_page(url: &Url) -> bool {
    if url.scheme() != "https" && url.scheme() != "http" {
        return false;
    }
    let Some(host) = url.host_str() else { return false };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if matches!(host.as_str(), "cdn.discordapp.com" | "media.discordapp.net" | "images-ext-1.discordapp.net" | "images-ext-2.discordapp.net") {
        return false;
    }
    ["discord.com", "discordapp.com", "discord.gg", "discord.new", "discord.gift"]
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// kryo.to's start of Discord's sign-in (not its callback).
fn is_kryoto_discord_start(url: &Url) -> bool {
    url.path().trim_end_matches('/') == "/api/auth/discord"
}

/// Pages where "Continue with Discord" signs in, rather than links an account
/// that is signed in already.
fn is_sign_in_page(url: &Url) -> bool {
    let p = url.path();
    ["/login", "/register", "/signup", "/invite", "/link"].iter().any(|s| p == *s || p.starts_with(&format!("{s}/")))
}

/// Decide a navigation of the Store. `ours` says whether `url` is kryo.to
/// (or the catalog address set in Settings); `from` is the page it leaves.
pub fn route(url: &Url, ours: bool, from: Option<&Url>) -> Outside {
    if ours && is_kryoto_discord_start(url) {
        // Linking from a signed-in page: the browser may be signed out, or
        // someone else, and Discord on its own there signs in (or makes an
        // account) rather than links. The page the button is on opens
        // instead, so the player links there, as whoever the browser is.
        if let Some(page) = from.filter(|p| p.origin() == url.origin() && !is_sign_in_page(p) && !is_kryoto_discord_start(p)) {
            return Outside::Browser(page.clone());
        }
        return Outside::Browser(url.clone());
    }
    if is_discord_page(url) {
        return Outside::Browser(url.clone());
    }
    Outside::No
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        s.parse().unwrap()
    }

    #[test]
    fn discord_sign_in_never_loads_in_the_store() {
        // The redirect kryo.to's start sends, and a typed-in Discord page.
        for page in [
            "https://discord.com/api/oauth2/authorize?client_id=1&response_type=code",
            "https://discord.com/oauth2/authorize?client_id=1",
            "https://discord.com/login",
            "https://ptb.discord.com/login",
            "https://discord.gg/Q2S8hRzrep",
            "https://discord.com/users/123",
            "https://discord.com/channels/1/2",
            "https://DISCORDAPP.com/login",
        ] {
            assert_eq!(route(&u(page), false, None), Outside::Browser(u(page)), "{page}");
        }
    }

    #[test]
    fn pictures_and_lookalikes_are_not_discord() {
        for page in [
            "https://cdn.discordapp.com/avatars/1/a.png",
            "https://media.discordapp.net/attachments/1/2/a.png",
            "https://notdiscord.com/login",
            "https://discord.com.evil.example/login",
            "https://kryo.to/discord",
        ] {
            assert_eq!(route(&u(page), page.starts_with("https://kryo.to"), None), Outside::No, "{page}");
        }
    }

    #[test]
    fn signing_in_with_discord_opens_kryotos_start_in_the_browser() {
        let start = u("https://kryo.to/api/auth/discord?next=%2Flibrary");
        assert_eq!(route(&start, true, Some(&u("https://kryo.to/login?next=/library"))), Outside::Browser(start.clone()));
        let invite = u("https://kryo.to/api/auth/discord?invite=abc");
        assert_eq!(route(&invite, true, Some(&u("https://kryo.to/invite/abc"))), Outside::Browser(invite.clone()));
        // Its callback is kryo.to's own page and stays.
        assert_eq!(route(&u("https://kryo.to/api/auth/discord/callback?code=1"), true, None), Outside::No);
        // Not kryo.to's start unless it is kryo.to.
        assert_eq!(route(&u("https://example.com/api/auth/discord"), false, None), Outside::No);
    }

    #[test]
    fn linking_opens_the_page_it_was_pressed_on() {
        let start = u("https://kryo.to/api/auth/discord?next=%2Fsettings%23connections");
        let settings = u("https://kryo.to/settings#connections");
        assert_eq!(route(&start, true, Some(&settings)), Outside::Browser(settings));
        // From somewhere that is not kryo.to, or not known: the start itself.
        assert_eq!(route(&start, true, Some(&u("https://example.com/x"))), Outside::Browser(start.clone()));
        assert_eq!(route(&start, true, None), Outside::Browser(start));
    }
}
