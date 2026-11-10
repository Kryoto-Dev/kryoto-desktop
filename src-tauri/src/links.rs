//! `kryoto://` links: what lets kryo.to say "Open in Kryoto Desktop" and
//! "Continue Hades II in Kryoto Desktop", the way a store says "Play in Steam".
//!
//!   kryoto://game/<slug>   the game's page in the Store
//!   kryoto://play/<slug>   start it when it is installed, else its page
//!   kryoto://library/<id>  start this library game, a shortcut in another
//!                          launcher (Bridge); never goes to kryo.to
//!   kryoto://chat/<user>   open (or start) a chat with that kryo.to username
//!   kryoto://settings/<section>[/<setting>]   Settings, scrolled to one setting
//!                          (`#<setting>` works too; src/lib/settings-links.ts)
//!
//! The scheme is registered for the current user at start (no admin rights).
//! Windows starts a new copy of the app for every link; that copy hands the
//! link to the one already running over the single-instance port (see
//! `system::claim_instance`) and exits. A link that started the app is kept
//! until the shell is ready to ask for it (`take_pending_link`).

use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Runtime};

pub const SCHEME: &str = "kryoto";
pub const EVENT: &str = "deep-link";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    /// `game`, `play`, `chat` or `settings`.
    pub action: String,
    /// A game slug, for `chat` a username, for `settings` `section` or `section/setting`.
    pub slug: String,
}

/// A link, or None for anything else - including a slug that is not one.
pub fn parse(raw: &str) -> Option<Link> {
    let rest = raw.trim().strip_prefix(&format!("{SCHEME}://"))?;
    let mut parts = rest.trim_end_matches('/').splitn(2, '/');
    let action = parts.next()?.to_ascii_lowercase();
    let raw_target = parts.next()?;
    if action == "settings" {
        // Which sections exist is the shell's to know; here only the shape.
        let path = raw_target.split('?').next()?.replace('#', "/").to_ascii_lowercase();
        let mut bits = path.split('/').filter(|b| !b.is_empty());
        let section = bits.next()?;
        let setting = bits.next();
        let ok = |s: &str| (1..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if bits.next().is_some() || !ok(section) || setting.is_some_and(|s| !ok(s)) {
            return None;
        }
        let slug = setting.map_or_else(|| section.to_string(), |s| format!("{section}/{s}"));
        return Some(Link { action, slug });
    }
    let target = raw_target.split(['?', '#']).next()?;
    if action == "chat" {
        // kryo.to's username rule: 3-32 of letters, digits, _ . - (and never
        // only dots, so it can never read as a path).
        let ok = (3..=32).contains(&target.len())
            && target.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
            && !target.chars().all(|c| c == '.');
        return ok.then(|| Link { action, slug: target.to_string() });
    }
    let slug = target.to_ascii_lowercase();
    let slug_ok = !slug.is_empty()
        && slug.len() <= 160
        && slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !slug_ok || !matches!(action.as_str(), "game" | "play" | "library") {
        return None;
    }
    Some(Link { action, slug })
}

/// The first `kryoto://` argument this process was started with.
pub fn from_args() -> Option<String> {
    std::env::args().skip(1).find(|a| a.starts_with(&format!("{SCHEME}://")))
}

static PENDING: Mutex<Option<Link>> = Mutex::new(None);

/// Hold a link that arrived before the shell could take it.
pub fn set_pending(raw: &str) {
    if let Some(link) = parse(raw) {
        *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(link);
    }
}

#[tauri::command]
pub fn take_pending_link() -> Option<Link> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Hold a link for the shell, bring the window up, and tell the shell to take
/// it. Held rather than sent: before sign-in there is no shell listening, and
/// an event nobody hears would lose the link. Whoever takes it clears it, so
/// it is opened once.
pub fn deliver<R: Runtime>(app: &AppHandle<R>, raw: &str) {
    let Some(link) = parse(raw) else { return };
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(link);
    crate::system::show_main(app);
    let _ = app.emit(EVENT, ());
}

/// Make `kryoto://` open this app. Best effort: a failure only means the
/// site's links do nothing on this machine, and it is logged.
///
/// A development build leaves the installed app's registration alone unless
/// `KRYOTO_REGISTER_SCHEME=1`, so running one never steals the links.
pub fn register() {
    if cfg!(debug_assertions) && std::env::var("KRYOTO_REGISTER_SCHEME").as_deref() != Ok("1") {
        return;
    }
    // Kryoto Desktop Preview never takes the normal app's links (src/channel.rs).
    if crate::channel::PREVIEW {
        return;
    }
    if let Err(e) = register_os() {
        crate::logging::error("links", &format!("could not register kryoto:// links: {e}"));
    }
}

#[cfg(windows)]
fn register_os() -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let set = |key: &str, name: Option<&str>, value: &str| -> Result<(), String> {
        let key = wide(key);
        let name = name.map(wide);
        let value = wide(value);
        // SAFETY: NUL-terminated UTF-16 strings; the size is the value's bytes.
        // A null name sets the key's default value.
        let rc = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        if rc == 0 { Ok(()) } else { Err(format!("Windows refused (error {rc})")) }
    };
    let base = format!(r"Software\Classes\{SCHEME}");
    set(&base, None, "URL:Kryoto Desktop")?;
    set(&base, Some("URL Protocol"), "")?;
    set(&format!(r"{base}\DefaultIcon"), None, &format!("\"{}\",0", exe.display()))?;
    set(&format!(r"{base}\shell\open\command"), None, &format!("\"{}\" \"%1\"", exe.display()))
}

/// This program, as something outside it can start it. An AppImage runs from
/// a temporary mount; $APPIMAGE is the file itself.
#[cfg(not(windows))]
pub fn this_app() -> Result<std::path::PathBuf, String> {
    match std::env::var_os("APPIMAGE") {
        Some(p) => Ok(std::path::PathBuf::from(p)),
        None => std::env::current_exe().map_err(|e| e.to_string()),
    }
}

#[cfg(not(windows))]
fn register_os() -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "No home folder.".to_string())?;
    let dir = std::path::Path::new(&home).join(".local/share/applications");
    let exe = this_app()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = "kryoto-desktop-links.desktop";
    std::fs::write(
        dir.join(name),
        format!(
            "[Desktop Entry]\nType=Application\nName=Kryoto Desktop\nExec=\"{}\" %u\nNoDisplay=true\nMimeType=x-scheme-handler/{SCHEME};\n",
            exe.display()
        ),
    )
    .map_err(|e| e.to_string())?;
    // Best effort: without xdg-mime the file is still found by most desktops.
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", name, &format!("x-scheme-handler/{SCHEME}")])
        .status();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_game_and_play() {
        assert_eq!(parse("kryoto://game/hades-ii"), Some(Link { action: "game".into(), slug: "hades-ii".into() }));
        assert_eq!(parse("kryoto://play/hades-ii/"), Some(Link { action: "play".into(), slug: "hades-ii".into() }));
        assert_eq!(parse("kryoto://PLAY/Hades-II?x=1").map(|l| l.slug), Some("hades-ii".into()));
    }

    #[test]
    fn parses_chat_with_a_username() {
        assert_eq!(parse("kryoto://chat/Mira_99"), Some(Link { action: "chat".into(), slug: "Mira_99".into() }));
        assert_eq!(parse("kryoto://chat/a.b-c/").map(|l| l.slug), Some("a.b-c".into()));
        for bad in ["kryoto://chat/ab", "kryoto://chat/...", "kryoto://chat/a b c", "kryoto://chat/<script>", &format!("kryoto://chat/{}", "a".repeat(33))] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn parses_settings_with_or_without_a_setting() {
        assert_eq!(parse("kryoto://settings/general"), Some(Link { action: "settings".into(), slug: "general".into() }));
        assert_eq!(parse("kryoto://settings/General/desktop-pet/").map(|l| l.slug), Some("general/desktop-pet".into()));
        assert_eq!(parse("kryoto://settings/general#desktop-pet").map(|l| l.slug), Some("general/desktop-pet".into()));
        for bad in ["kryoto://settings/", "kryoto://settings/a/b/c", "kryoto://settings/gen eral", "kryoto://settings/../etc", "kryoto://settings/general/<b>"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn refuses_anything_else() {
        for bad in [
            "kryoto://game/",
            "kryoto://delete/hades-ii",
            "kryoto://game/../../etc",
            "kryoto://game/a b",
            "https://kryo.to/game/hades-ii",
            "kryoto://game/hades%20ii",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
