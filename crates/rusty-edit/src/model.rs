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
    /// Module paths — `esp_hal::interrupt::` — stepped back a shade. Only
    /// semantic tokens produce this: the path prefix is the least important
    /// part of a line, and unstyled it was the brightest.
    Namespace,
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

/// Where a macro expansion is shown: a document with no file behind it,
/// named after its macro. Nothing reads or writes it on disk, and this prefix
/// is how every part of the editor tells it from a file.
pub const EXPANSION_PREFIX: &str = "expansion:/";

/// Whether an open document is a macro expansion rather than a file.
pub fn is_expansion(path: &str) -> bool {
    path.starts_with(EXPANSION_PREFIX)
}

/// The name the expansion of the macro called on `line` (0-based) of `path`
/// is shown under. The call's file and line come first and the macro's name
/// last, so a tab reads `println!` and two expansions of `println!` are told
/// apart by where they were called — and the same call expanded again is
/// the same document, shown again rather than beside itself.
pub fn expansion_path(path: &str, line: u32, name: &str) -> String {
    format!("{EXPANSION_PREFIX}{path}:{}/{name}", line + 1)
}

/// Where an expansion's macro was called, as `file:line`, and the macro's
/// name — what its header and its tab's tooltip say in place of a path.
/// `None` for a path that is a file.
pub fn expansion_parts(path: &str) -> Option<(&str, &str)> {
    path.strip_prefix(EXPANSION_PREFIX)?.rsplit_once('/')
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
    /// Set when the file cannot be shown as text — because it is not text, or
    /// because it is too large to send (`too_large` says which). There are no
    /// lines in that case — a viewer that renders a firmware image as
    /// mojibake helps nobody.
    pub binary: bool,
    /// Set when the file was refused for its size rather than its contents.
    /// A 3 MB generated register map *is* text; calling it binary sent the
    /// reader looking for a corrupt file.
    #[serde(default)]
    pub too_large: bool,
    /// The painting `lines` came from, by the number the backend keeps it
    /// under: what the editor names when it asks for a repaint, so that an
    /// edit costs the lines it changed rather than the file. `None` for a
    /// document nothing is kept for — refused, or a library's source opened
    /// to read.
    #[serde(default)]
    pub paint: Option<u32>,
    /// Not this project's file — a dependency's source, opened to read.
    /// The editor refuses to write it: the registry cache is shared by every
    /// project on the machine, and "I fixed it in the library" there is a
    /// change the next `cargo build` may silently revert or spread.
    #[serde(default)]
    pub read_only: bool,
}

/// An edited buffer repainted, as the editor applies it.
///
/// `lines` are the painted lines of the text that was sent, from line `from`
/// on. Every other line paints as it did in the painting the request named —
/// below these, moved by however many lines the edit added or removed. A
/// request naming a painting the backend no longer keeps is answered whole:
/// `from` is 0 and `lines` is every line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repaint {
    /// The number to name this painting by in the next request.
    pub version: u32,
    pub from: u32,
    pub lines: Vec<Line>,
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
    /// True when there were more hits than the cap lets through — the panel
    /// says "first N", because a silently partial answer reads as a complete
    /// one. Exactly the cap's worth of hits is a whole answer, and leaves this
    /// false.
    pub truncated: bool,
    /// A query that could not run as written — a malformed glob, named so the
    /// user can fix the pattern instead of trusting an empty result.
    #[serde(default)]
    pub error: Option<String>,
}

/// What a replace did, and what it would not touch.
///
/// Every field is here because the alternative is a silent one: a file holding
/// an unsaved draft, one that could not be written. A replace across a
/// project cannot be undone from inside the window, so one that reports only a
/// number is one nobody can check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceOutcome {
    /// Files rewritten, project-relative and `/`-separated.
    pub changed: Vec<String>,
    /// Occurrences replaced across all of them.
    pub replaced: u32,
    /// Files that matched and were left alone, with why.
    pub skipped: Vec<Skipped>,
    /// The pattern did not parse, or the globs did not combine. Nothing was
    /// written when this is set.
    #[serde(default)]
    pub error: Option<String>,
}

/// A file a replace matched and did not write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub path: String,
    /// `unsaved` or `write-failed` — a stable name the frontend translates,
    /// never a sentence. Same reasoning as a diagnostic's `kind`.
    pub reason: String,
}

/// What changed on disk, while the window was looking elsewhere.
///
/// One batch rather than one event per path: a save from another editor is
/// often a write, a rename and a second write, and `cargo build` touches tens
/// of thousands of files. A refresh per event would be a refresh storm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileChanges {
    /// Files whose *contents* changed. Project-relative, `/`-separated, like
    /// every other path on this wire.
    pub changed: Vec<String>,
    /// True when something appeared, vanished or was renamed, so the tree
    /// itself is stale rather than just a file's text.
    ///
    /// Separate because the two answers cost very different amounts: rereading
    /// one open file is nothing, and walking the project is not.
    pub tree: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name an expansion is shown under says where the macro was called
    /// and what it was, and reads back as both.
    #[test]
    fn an_expansion_is_named_by_its_call_and_its_macro() {
        let path = expansion_path("src/nav.rs", 37, "println!");
        assert_eq!(path, "expansion:/src/nav.rs:38/println!");
        assert!(is_expansion(&path));
        assert_eq!(expansion_parts(&path), Some(("src/nav.rs:38", "println!")));
        assert_eq!(expansion_parts("src/nav.rs"), None);
        assert!(!is_expansion("src/nav.rs"));
    }
}
