//! What a rendered terminal screen looks like on the wire.
//!
//! Compiled unconditionally and free of IO, like every other `model` here, so
//! the Leptos frontend draws these types directly instead of through a
//! generated binding layer.
//!
//! Runs of same-styled text rather than a cell grid. A 120×40 screen is 4800
//! cells and perhaps 200 runs, and it is redrawn many times a second while a
//! build scrolls — sending cells would spend more time serialising than the
//! emulator spends parsing.

use serde::{Deserialize, Serialize};

/// A colour as the terminal expresses it.
///
/// Indexed colours stay indexed rather than being resolved to RGB here: 0–15
/// are the palette the *theme* defines, so resolving them in the backend would
/// paint a light-theme terminal with dark-theme colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Colour {
    /// The theme's foreground or background, depending on where it is used.
    #[default]
    Default,
    /// A palette entry. 0–7 normal, 8–15 bright, 16–255 the xterm cube.
    Indexed {
        index: u8,
    },
    Rgb {
        r: u8,
        g: u8,
        b: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    pub fg: Colour,
    pub bg: Colour,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Selected text and some prompts use reverse video. Applied by swapping
    /// fg and bg at render time rather than here, so a `Default` colour still
    /// means "whatever the theme says".
    pub inverse: bool,
}

/// A run of characters sharing one style.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
}

/// One frame of the terminal.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    pub rows: Vec<Row>,
    pub cols: u16,
    /// Absent when the program has hidden it, and while scrolled back — a
    /// cursor drawn over history is a cursor in the wrong place.
    pub cursor: Option<Cursor>,
    /// How many lines above the live screen the view is showing.
    pub scrollback: usize,
    /// Set once the child has exited, with its status. The view stops taking
    /// keystrokes and says so rather than swallowing them.
    pub exited: Option<i32>,
}

/// What the frontend sends back when a key is pressed.
///
/// Bytes, not key names. Terminals are a byte protocol — `Ctrl C` is `0x03`
/// and Up is `ESC [ A` — and translating in the frontend keeps the backend
/// from having to model every keyboard layout in the world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub bytes: Vec<u8>,
}
