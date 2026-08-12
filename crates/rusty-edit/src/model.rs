//! What the file tree and a highlighted document look like on the wire.
//!
//! Compiled unconditionally and free of IO, like every other `model` here, so
//! the Leptos frontend draws these types directly.

use serde::{Deserialize, Serialize};

/// One entry in the project tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    /// Path relative to the project root, with `/` separators on every
    /// platform. Relative because it is an identity the frontend echoes back,
    /// and an absolute Windows path in a URL-ish position invites someone to
    /// concatenate it with something.
    pub path: String,
    pub is_dir: bool,
    /// Empty for files, and for directories nobody has expanded.
    pub children: Vec<Entry>,
}

/// What a run of characters means, rather than what colour it is.
///
/// Semantic, not RGB. syntect's own themes are fixed palettes, and shipping one
/// would paint a light-theme window with dark-theme colours — so the kind
/// travels and the stylesheet decides, exactly as the terminal's indexed colours
/// do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Token {
    #[default]
    Plain,
    Keyword,
    /// String and character literals.
    Str,
    Number,
    Comment,
    /// Types, traits, enum variants — anything that names a type.
    Type,
    Function,
    /// `println!`, `#[derive(...)]` and friends. Rust's macros and attributes
    /// carry enough weight to be worth telling apart from function calls.
    Macro,
    /// Brackets, commas, operators.
    Punctuation,
    /// `let`, `self`, lifetimes — bindings rather than keywords.
    Variable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub text: String,
    pub token: Token,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub spans: Vec<Span>,
}

/// A file, ready to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    /// Relative path, as in [`Entry::path`].
    pub path: String,
    /// Highlighted lines, for display.
    pub lines: Vec<Line>,
    /// The same content unstyled, for the editor to put in a text box.
    ///
    /// Both are sent because the editor overlays a plain text area on the
    /// highlighted view; deriving one from the other in the frontend would mean
    /// re-joining spans and getting trailing whitespace subtly wrong.
    pub text: String,
    /// Which grammar was used, if one matched.
    pub language: Option<String>,
    /// Set when the file is not text. There are no lines in that case — a
    /// viewer that renders a firmware image as mojibake helps nobody.
    pub binary: bool,
    /// Set when the file was too large to highlight in full.
    pub truncated: bool,
    /// Not this project's file — a dependency's source, opened to read.
    /// The editor refuses to write it: the registry cache is shared by every
    /// project on the machine, and "I fixed it in the library" there is a
    /// change the next `cargo build` may silently revert or spread.
    #[serde(default)]
    pub read_only: bool,
}

/// What rustfmt made of the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Formatted {
    pub text: String,
    /// False when the input was already formatted — the caller skips the
    /// caret-preserving rewrite entirely rather than diffing to find out.
    pub changed: bool,
}

/// One place the query appears.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// Project-relative, `/`-separated — the file tree's identity for it.
    pub path: String,
    /// 0-based, like every line number on this wire.
    pub line: u32,
    /// Unicode-scalar column of the match, for the editor to reveal.
    pub col: u32,
    /// The matched line, windowed when it is enormous (minified JS, lock
    /// files) so one line cannot flood the panel.
    pub text: String,
    /// Byte range of the match inside `text`, for highlighting. Bytes, not
    /// scalars: the frontend slices, it does not count.
    pub span_start: u32,
    pub span_end: u32,
}

/// Everything a search found, and whether it stopped early.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// Distinct files in `hits`.
    pub files: u32,
    /// True when the cap was reached — the panel says "first N", because a
    /// silently partial answer reads as a complete one.
    pub truncated: bool,
}
