//! Which games have a change to their folder under way, so a second change
//! is refused instead of running at the same time.
//!
//! The repair lock (`kryoto_repair::journal::lock`) keeps two repairs apart,
//! but Move, Rename folder, Uninstall, community updates, Go back to the
//! previous build and add-ons never took it, so a Repair could run while a
//! Move copied the folder. Move, Rename folder and Uninstall cannot hold that
//! lock for their length either: its file is inside the folder they move,
//! rename or recycle, and Windows will not rename a folder with a file in it
//! open. So every command that writes into a game's folder enters this set
//! first, for its whole length, and says what it is doing; a second one is
//! refused with that. Kept by game id, so it does not matter which of a
//! game's folders each command works on. One Kryoto only: a second copy of
//! the app is kept out by the repair lock, where it is held.

use std::sync::Mutex;

/// Each game with a change under way, and what that change is.
static BUSY: Mutex<Vec<(String, &'static str)>> = Mutex::new(Vec::new());

/// A game held in the set. It leaves the set when this is dropped, so an
/// error on the way out cannot leave the game held for good. Not re-entrant:
/// a command that calls another one passes its own guard on instead.
pub struct Busy {
    id: String,
}

/// Hold `id` for a change, said as what the game is doing ("being moved").
/// Refused while something else holds it.
pub fn enter(id: &str, doing: &'static str) -> Result<Busy, String> {
    let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, other)) = busy.iter().find(|(held, _)| held == id) {
        return Err(format!("This game is {other}. Wait for that to finish, then try again."));
    }
    busy.push((id.to_string(), doing));
    Ok(Busy { id: id.to_string() })
}

impl Busy {
    /// The game this guard holds, for a callee that is handed it.
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.lock().unwrap_or_else(|e| e.into_inner()).retain(|(held, _)| held != &self.id);
    }
}

/// None of these folders has a repair, an online fix or Get the files back
/// under way, by the repair lock. For Move, Rename folder and Uninstall,
/// which cannot hold that lock themselves; a second Kryoto may hold it.
pub fn idle(folders: &[&std::path::Path]) -> Result<(), String> {
    folders.iter().filter(|f| f.is_dir()).try_for_each(|f| kryoto_repair::journal::check_idle(f))
}

/// The repair lock on a game folder, for a change that does not rename or
/// recycle it (an add-on, taking Kryoto Online out): a Repair from a second
/// Kryoto waits, and so does Play. None when the folder is gone, so taking
/// the lock never makes a deleted game's folder again.
pub fn lock(root: &std::path::Path) -> Result<Option<kryoto_repair::journal::Lock>, String> {
    root.is_dir().then(|| kryoto_repair::journal::lock(root)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_change_is_refused_with_what_the_first_is_doing() {
        let moving = enter("busy-test-a", "being moved").unwrap();
        assert_eq!(moving.id(), "busy-test-a");
        assert_eq!(
            enter("busy-test-a", "being uninstalled").err().unwrap(),
            "This game is being moved. Wait for that to finish, then try again."
        );
        // Another game is not held up.
        let other = enter("busy-test-b", "being repaired").unwrap();
        drop(moving);
        let again = enter("busy-test-a", "being uninstalled").unwrap();
        drop((again, other));
    }

    #[test]
    fn a_change_that_fails_leaves_the_game_free() {
        fn fails() -> Result<(), String> {
            let _busy = enter("busy-test-c", "getting an add-on")?;
            Err("Applying the add-on failed.".into())
        }
        assert!(fails().is_err());
        assert!(enter("busy-test-c", "getting an add-on").is_ok());
    }

    #[test]
    fn a_folder_another_change_has_locked_is_not_idle() {
        let dir = std::env::temp_dir().join(format!("kryoto-busy-idle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("gone");
        assert!(idle(&[&dir, &missing]).is_ok());
        let lock = kryoto_repair::journal::lock(&dir).unwrap();
        assert!(idle(&[&dir]).unwrap_err().contains("changing this game's files"));
        drop(lock);
        assert!(idle(&[&dir]).is_ok());
        // An add-on undone on a game whose folder was deleted makes none.
        assert!(super::lock(&missing).unwrap().is_none());
        assert!(!missing.exists());
        let held = super::lock(&dir).unwrap();
        assert!(held.is_some() && idle(&[&dir]).is_err());
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
