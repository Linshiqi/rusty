//! The playground, as both sides of the wire name it.
//!
//! rusty keeps one project per chip for trying things (`crate::playground`,
//! backend-side, writes them); the window offers them by these names and
//! opens each on this file, so the list is spelled once.

/// The chips there is a playground for — those rusty's emulators model
/// whole — in the order they are offered: the C3 first, because it builds
/// with stable Rust and needs nothing else installed; the CH32V003J4M6,
/// which rusty emulates itself, last, because it needs nightly.
pub const PLAYGROUND_CHIPS: [&str; 3] = ["esp32c3", "esp32", "ch32v003j4m6"];

/// The file a playground opens on, relative to its root.
pub const PLAYGROUND_MAIN: &str = "src/main.rs";
