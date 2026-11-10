//! The player's own sounds: a file of their choosing played instead of one of
//! Kryoto's (Settings > Sounds). Each is copied into a folder of its own in
//! app data, `custom-sounds`, apart from the library and every cache, so
//! clearing a cache or moving games never takes it. The original file is left
//! where it was. One file per sound, named after it (`notify.mp3`).

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

/// A sound is a few seconds long; this is generous for that.
const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// What WebView2 and WebKitGTK both decode.
const EXTENSIONS: [&str; 5] = ["wav", "mp3", "ogg", "flac", "m4a"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomSound {
    /// The sound it replaces (`SoundName` in src/lib/sound-recipes.ts).
    name: String,
    /// The copy's file name in the folder.
    file: String,
}

fn dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("custom-sounds"))
}

/// Kryoto's sound names are short camelCase words; nothing else names a file here.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn existing(dir: &Path, name: &str) -> Option<PathBuf> {
    EXTENSIONS.iter().map(|e| dir.join(format!("{name}.{e}"))).find(|p| p.is_file())
}

/// Every sound the player has replaced.
#[tauri::command(async)]
pub fn sound_custom_list(app: AppHandle) -> Result<Vec<CustomSound>, String> {
    let dir = dir(&app)?;
    let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let (Some(stem), Some(ext)) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|s| s.to_str())) else { continue };
        if path.is_file() && valid_name(stem) && EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            out.push(CustomSound { name: stem.to_string(), file: entry.file_name().to_string_lossy().into_owned() });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Use the file at `path` for the sound `name`: copied in, replacing any
/// earlier choice for it.
#[tauri::command(async)]
pub fn sound_custom_set(app: AppHandle, name: String, path: String) -> Result<CustomSound, String> {
    if !valid_name(&name) {
        return Err("That is not one of Kryoto's sounds.".into());
    }
    let src = Path::new(&path);
    let ext = src.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !EXTENSIONS.contains(&ext.as_str()) {
        return Err("Pick a WAV, MP3, OGG, FLAC or M4A file.".into());
    }
    let size = std::fs::metadata(src).map_err(|e| format!("Could not read that file: {e}"))?.len();
    if size > MAX_BYTES {
        return Err("That file is over 5 MB. A sound should only be a few seconds long.".into());
    }
    let dir = dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not make the custom sounds folder: {e}"))?;
    let file = format!("{name}.{ext}");
    let dest = dir.join(&file);
    // Copied to a temporary name first, so a failed copy leaves the earlier choice in place.
    let partial = dir.join(format!("{name}.partial"));
    std::fs::copy(src, &partial).map_err(|e| format!("Could not copy that file: {e}"))?;
    if let Some(old) = existing(&dir, &name) {
        let _ = std::fs::remove_file(old);
    }
    std::fs::rename(&partial, &dest).map_err(|e| format!("Could not save that file: {e}"))?;
    Ok(CustomSound { name, file })
}

/// Back to Kryoto's own sound for `name`.
#[tauri::command(async)]
pub fn sound_custom_clear(app: AppHandle, name: String) -> Result<(), String> {
    if !valid_name(&name) {
        return Ok(());
    }
    if let Some(path) = existing(&dir(&app)?, &name) {
        std::fs::remove_file(path).map_err(|e| format!("Could not remove it: {e}"))?;
    }
    Ok(())
}

/// The chosen file's bytes, for the page to decode and play.
#[tauri::command(async)]
pub fn sound_custom_read(app: AppHandle, name: String) -> Result<tauri::ipc::Response, String> {
    if !valid_name(&name) {
        return Err("That is not one of Kryoto's sounds.".into());
    }
    let path = existing(&dir(&app)?, &name).ok_or("No sound of your own for that.")?;
    Ok(tauri::ipc::Response::new(std::fs::read(path).map_err(|e| e.to_string())?))
}

/// The folder, made if it is not there yet, for "Open the folder".
#[tauri::command(async)]
pub fn sound_custom_folder(app: AppHandle) -> Result<String, String> {
    let dir = dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::valid_name;

    #[test]
    fn only_sound_names_name_a_file() {
        assert!(valid_name("notify"));
        assert!(valid_name("bpSelect"));
        assert!(!valid_name(""));
        assert!(!valid_name("../notify"));
        assert!(!valid_name("a.b"));
        assert!(!valid_name(&"x".repeat(33)));
    }
}
