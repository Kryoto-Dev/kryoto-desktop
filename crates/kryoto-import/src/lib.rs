//! Bridge: bringing games over from other launchers, reading their libraries without
//! changing them, and working out which Kryoto game each one is.

pub mod hydra;
pub mod leveldb;
pub mod matcher;
pub mod steam;
pub mod vdf;
mod snappy;
