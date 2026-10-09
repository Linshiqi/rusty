//! The playground, as both sides of the wire name it.
//!
//! rusty keeps one project per chip for trying things (`crate::playground`,
//! backend-side, writes them); the window offers them by these names and
//! opens each on this file, so the list is spelled once.

/// The playgrounds, in the order they are offered: one per chip rusty's
/// emulators model whole — the C3 first, because it builds with stable Rust
/// and needs nothing else installed; the CH32V003J4M6, which rusty emulates
/// itself, after the Espressif parts, because it needs nightly — and
/// [`PLAYGROUND_DRAW`], which is no chip at all.
pub const PLAYGROUNDS: [&str; 4] = ["esp32c3", "esp32", "ch32v003j4m6", PLAYGROUND_DRAW];

/// The drawing playground: Rust on this machine drawing vectors into the
/// Draw tab through rusty-draw — no chip, no board, stable Rust.
pub const PLAYGROUND_DRAW: &str = "draw";

/// The file a chip's playground opens on, relative to its root.
pub const PLAYGROUND_MAIN: &str = "src/main.rs";

/// The file `playground` opens on: its firmware's `main`, or for the
/// drawing playground the example whose `main` has the ▶ Run above it.
pub fn playground_main(playground: &str) -> &'static str {
    if playground == PLAYGROUND_DRAW {
        "examples/vectors.rs"
    } else {
        PLAYGROUND_MAIN
    }
}

/// Whether `playground` is code beside a board. The drawing playground is
/// code beside nothing: what it draws appears in the Draw tab below.
pub fn playground_has_board(playground: &str) -> bool {
    playground != PLAYGROUND_DRAW
}
