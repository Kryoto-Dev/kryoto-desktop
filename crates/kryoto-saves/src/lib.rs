//! Cloud saves: where a game keeps its saves, and moving them between PCs.
//!
//! A game's saves can be anywhere - the Steam emulator's folder, `Documents\My
//! Games`, `AppData`, `LocalLow`, the registry, or the game's own folder - and
//! the same game can keep them somewhere else on another PC. So nothing here
//! stores an absolute path: a save location is a [`rules::Rule`], a folder
//! relative to a placeholder root ([`rules::Root`]) that every PC (and every
//! Wine prefix) resolves for itself.
//!
//! Rules come from exact sources first - the emulator's own config
//! ([`emulator`]), the game engine's convention ([`engine`]), the Ludusavi
//! database ([`known`]) - and from what Kryoto Desktop saw the game write while
//! it ran ([`trace`]). [`scan`] turns rules into files, [`bundle`] packs them
//! into a `.ksave` and unpacks one again, and [`decide`] says what a sync
//! should do. No Tauri, no network: the app wires it up.

pub mod bundle;
pub mod decide;
pub mod emulator;
pub mod engine;
pub mod known;
pub mod mentions;
pub mod rules;
pub mod scan;
pub mod trace;
#[cfg(windows)]
pub mod registry;

pub use rules::{Kind, Root, Rule, SaveRoots, Source};
