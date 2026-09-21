//! The playground, as both sides of the wire name it.
//!
//! rusty keeps one project per chip for trying things (`crate::playground`,
//! backend-side, writes them); the window offers them by these names and
//! opens each on this file, so the list is spelled once.

/// The chips there is a playground for — the two rusty's emulator models
/// whole — in the order they are offered: the C3 first, because it builds
/// with stable Rust and needs nothing else installed.
pub const PLAYGROUND_CHIPS: [&str; 2] = ["esp32c3", "esp32"];

/// The file a playground opens on, relative to its root.
pub const PLAYGROUND_MAIN: &str = "src/main.rs";
