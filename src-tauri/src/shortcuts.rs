//! A game on the desktop: the right-click menu's Create desktop shortcut.
//!
//! The shortcut starts Kryoto with `kryoto://library/<id>` rather than the
//! game's own exe, so everything Play does still happens: play time, cloud
//! saves, the in-game name, Proton.
//!
//! - Windows: a `.url` file on the desktop, with the game's own icon. `.url`
//!   text is read in the system's code page, so a path with anything but ASCII
//!   in it is left out and the link's icon, Kryoto's, is used instead.
//! - Linux: a `.desktop` entry on the desktop and one in the applications menu,
//!   starting this app directly (`xdg-open` fails on some desktops, see
//!   linux_open.rs), with the game's cover as its icon.
//!
//! Only files that carry the game's link are ever replaced or removed.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Runtime};

use crate::library::LibraryGame;

fn link_to(game: &LibraryGame) -> String {
    format!("{}://library/{}", crate::links::SCHEME, game.id)
}

/// A title as a file name on every system this runs on.
fn file_stem(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c })
        .take(80)
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_matches(['.', ' ']);
    if s.is_empty() { "Game".to_string() } else { s.to_string() }
}

fn is_ours(path: &Path, link: &str) -> bool {
    std::fs::read(path).is_ok_and(|bytes| String::from_utf8_lossy(&bytes).lines().any(|l| l.contains(link)))
}

/// Where to write: the title, or the title and the id when that name is
/// already something else of the player's.
fn free_path(dir: &Path, stem: &str, id: &str, extension: &str, link: &str) -> PathBuf {
    let plain = dir.join(format!("{stem}.{extension}"));
    if !plain.exists() || is_ours(&plain, link) {
        return plain;
    }
    dir.join(format!("{stem} ({id}).{extension}"))
}

fn desktop_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("KRYOTO_DEV_DESKTOP_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let found = app.path().desktop_dir().ok();
    // Linux names it in user-dirs.dirs, which not every desktop has.
    #[cfg(not(windows))]
    let found = found.or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join("Desktop")));
    found.filter(|d| d.is_dir())
}

/// The text of a Windows Internet Shortcut.
#[cfg_attr(not(windows), allow(dead_code))]
fn url_file(link: &str, icon: Option<&Path>) -> String {
    let mut text = format!("[InternetShortcut]\r\nURL={link}\r\n");
    if let Some(icon) = icon.and_then(Path::to_str).filter(|p| p.is_ascii()) {
        text.push_str(&format!("IconIndex=0\r\nIconFile={icon}\r\n"));
    }
    text
}

/// The text of a Linux desktop entry. `Exec` is quoted as the specification
/// asks, and a `%` in a path doubled so it is not read as a field code.
#[cfg_attr(windows, allow(dead_code))]
fn desktop_entry(title: &str, app: &Path, link: &str, icon: Option<&Path>) -> String {
    let name: String = title.chars().filter(|c| !c.is_control()).collect();
    let mut exec = String::new();
    for c in app.to_string_lossy().chars() {
        match c {
            '"' | '`' | '$' => exec.extend(['\\', c]),
            '\\' => exec.push_str("\\\\\\\\"),
            '%' => exec.push_str("%%"),
            c => exec.push(c),
        }
    }
    let icon = icon.map_or_else(|| "kryoto-desktop".to_string(), |p| p.to_string_lossy().into_owned());
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nComment=Start in Kryoto Desktop\nExec=\"{exec}\" {link}\nIcon={icon}\nTerminal=false\nCategories=Game;\n"
    )
}

/// The exe the shortcut shows the icon of, when Play starts one.
#[cfg_attr(not(windows), allow(dead_code))]
fn game_exe(game: &LibraryGame) -> Option<PathBuf> {
    let exe = Path::new(&game.install_dir).join(&game.executable);
    let is_exe = exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    (is_exe && exe.is_file()).then_some(exe)
}

/// The applications menu's folder, for this user.
#[cfg(not(windows))]
fn menu_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share/applications"))
}

#[cfg(not(windows))]
fn menu_entry(game: &LibraryGame) -> Option<PathBuf> {
    Some(menu_dir()?.join(format!("kryoto-game-{}.desktop", game.id)))
}

/// The game's cover, as a file a desktop entry can name.
#[cfg(not(windows))]
async fn icon_file<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Option<PathBuf> {
    let bytes = crate::art::bytes_of(app, game.cover.as_deref()?).await?;
    let extension = if bytes.starts_with(b"\x89PNG") { "png" } else { "jpg" };
    let dir = app.path().app_data_dir().ok()?.join("shortcut-icons");
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join(format!("{}.{extension}", game.id));
    std::fs::write(&file, bytes).ok()?;
    Some(file)
}

#[cfg(windows)]
async fn create<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Result<&'static str, String> {
    let dir = desktop_dir(app).ok_or("Could not find your desktop folder.")?;
    let link = link_to(game);
    let icon = game_exe(game);
    let file = free_path(&dir, &file_stem(&game.title), &game.id, "url", &link);
    std::fs::write(&file, url_file(&link, icon.as_deref())).map_err(|e| format!("Could not write the shortcut: {e}"))?;
    Ok("desktop")
}

#[cfg(not(windows))]
async fn create<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) -> Result<&'static str, String> {
    use std::os::unix::fs::PermissionsExt;
    let link = link_to(game);
    let exe = crate::links::this_app()?;
    let icon = icon_file(app, game).await;
    let text = desktop_entry(&game.title, &exe, &link, icon.as_deref());
    let menu = menu_entry(game).ok_or("No home folder.")?;
    if let Some(dir) = menu.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not write the shortcut: {e}"))?;
    }
    std::fs::write(&menu, &text).map_err(|e| format!("Could not write the shortcut: {e}"))?;
    let Some(dir) = desktop_dir(app) else { return Ok("menu") };
    let file = free_path(&dir, &file_stem(&game.title), &game.id, "desktop", &link);
    std::fs::write(&file, &text).map_err(|e| format!("Could not write the shortcut: {e}"))?;
    // A desktop entry on the desktop only opens when it is executable, and on
    // GNOME when it is also marked trusted.
    let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755));
    #[cfg(not(target_os = "macos"))]
    {
        let mut gio = std::process::Command::new("gio");
        crate::linux_open::clean(&mut gio);
        let _ = gio.arg("set").arg(&file).args(["metadata::trusted", "true"]).output();
    }
    Ok("desktop")
}

/// Make the shortcut. Answers where it went: `desktop`, or `menu` on a Linux
/// desktop with no desktop folder.
#[tauri::command]
pub async fn game_shortcut(app: AppHandle, id: String) -> Result<String, String> {
    let game = crate::library::load(&app)?.into_iter().find(|g| g.id == id).ok_or("That game is no longer in the library.")?;
    if crate::links::parse(&link_to(&game)).is_none_or(|l| l.slug != game.id) {
        return Err("This game cannot have a shortcut.".into());
    }
    let place = create(&app, &game).await?;
    crate::logging::info("shortcuts", &format!("{}: shortcut made ({place})", game.title));
    Ok(place.to_string())
}

/// The game left the library: its shortcuts would only say so.
pub fn remove<R: Runtime>(app: &AppHandle<R>, game: &LibraryGame) {
    let link = link_to(game);
    let stem = file_stem(&game.title);
    let mut files = Vec::new();
    if let Some(dir) = desktop_dir(app) {
        for extension in ["url", "desktop"] {
            files.push(dir.join(format!("{stem}.{extension}")));
            files.push(dir.join(format!("{stem} ({}).{extension}", game.id)));
        }
    }
    #[cfg(not(windows))]
    {
        files.extend(menu_entry(game));
        if let Ok(dir) = app.path().app_data_dir() {
            for extension in ["png", "jpg"] {
                let _ = std::fs::remove_file(dir.join("shortcut-icons").join(format!("{}.{extension}", game.id)));
            }
        }
    }
    for file in files {
        if is_ours(&file, &link) {
            let _ = std::fs::remove_file(file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_becomes_a_file_name() {
        assert_eq!(file_stem("Hades II"), "Hades II");
        assert_eq!(file_stem("S.T.A.L.K.E.R.: Shadow of Chernobyl?"), "S.T.A.L.K.E.R. Shadow of Chernobyl");
        assert_eq!(file_stem("A/B\\C\n..."), "A B C");
        assert_eq!(file_stem("???"), "Game");
        assert_eq!(file_stem(&"x".repeat(200)).len(), 80);
    }

    #[test]
    fn a_url_file_leaves_out_what_it_cannot_carry() {
        let plain = url_file("kryoto://library/hades", Some(Path::new(r"C:\Games\Hades\Hades.exe")));
        assert_eq!(plain, "[InternetShortcut]\r\nURL=kryoto://library/hades\r\nIconIndex=0\r\nIconFile=C:\\Games\\Hades\\Hades.exe\r\n");
        let unicode = url_file("kryoto://library/hades", Some(Path::new(r"C:\Spiele\Müller\Hades.exe")));
        assert_eq!(unicode, "[InternetShortcut]\r\nURL=kryoto://library/hades\r\n");
    }

    #[test]
    fn a_desktop_entry_quotes_its_program() {
        let entry = desktop_entry("Hades\nII", Path::new("/home/a b/100%/$Kryoto.AppImage"), "kryoto://library/hades", None);
        assert!(entry.contains("Name=HadesII\n"));
        assert!(entry.contains("Exec=\"/home/a b/100%%/\\$Kryoto.AppImage\" kryoto://library/hades\n"));
        assert!(entry.contains("Icon=kryoto-desktop\n"));
    }

    #[test]
    fn someone_elses_file_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("kryoto-shortcuts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = "kryoto://library/hades";
        assert_eq!(free_path(&dir, "Hades", "hades", "url", link), dir.join("Hades.url"));
        std::fs::write(dir.join("Hades.url"), "[InternetShortcut]\r\nURL=https://example.com\r\n").unwrap();
        assert_eq!(free_path(&dir, "Hades", "hades", "url", link), dir.join("Hades (hades).url"));
        std::fs::write(dir.join("Hades.url"), url_file(link, None)).unwrap();
        assert_eq!(free_path(&dir, "Hades", "hades", "url", link), dir.join("Hades.url"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
