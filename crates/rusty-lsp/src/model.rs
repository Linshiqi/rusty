//! What the language server tells the frontend, on the wire.
//!
//! Compiled unconditionally and free of IO, like every other `model` here.
//! Positions in these types are **Unicode-scalar columns**, 0-based — the
//! backend does all conversion from the protocol's negotiated encoding, so the
//! frontend can slice a line with `chars()` and be right about CJK and emoji
//! without knowing UTF-16 exists.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagSeverity {
    // Ordered worst-first so `min` picks the one worth painting when ranges
    // overlap.
    Error,
    Warning,
    Info,
    Hint,
}

/// One problem the compiler or the server found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiagnostic {
    pub severity: DiagSeverity,
    pub message: String,
    /// Who said so — `rustc` for check output, `rust-analyzer` for its own.
    pub source: Option<String>,
    /// `E0308` and friends, when there is one.
    pub code: Option<String>,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// What the server session tells the frontend as it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum LspEvent {
    /// The protocol is up. Not "the index is ready" — requests may still be
    /// answered slowly or emptily while rust-analyzer loads the workspace.
    Ready {},
    /// No server could be started, with what to do about it.
    #[serde(rename_all = "camelCase")]
    Unavailable {
        message: String,
        install: Option<String>,
    },
    /// The diagnostics for one file, replacing whatever was known before.
    /// An empty list is meaningful: it is how "the error you fixed is gone"
    /// arrives.
    #[serde(rename_all = "camelCase")]
    Diagnostics {
        path: String,
        items: Vec<FileDiagnostic>,
    },
    Exited {},
}

/// One completion the server offered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    pub label: String,
    /// `function`, `struct`, `field`… — the LSP kind, named rather than
    /// numbered, because a number on the wire invites the frontend to keep its
    /// own copy of the table.
    pub kind: Option<String>,
    pub detail: Option<String>,
    /// What accepting this item inserts.
    pub insert: String,
    /// The range the insertion replaces, when the server said. Without it the
    /// caller replaces the word being typed.
    pub edit: Option<EditRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditRange {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// Somewhere — in the project, or in a dependency's source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    /// Project-relative and `/`-separated when `external` is false — the same
    /// identity the file tree uses. Absolute when `external` is true, because
    /// a dependency's source has no relative spelling.
    pub path: String,
    pub line: u32,
    pub col: u32,
    /// Outside the project: `core`, a registry crate, a git checkout. Shown
    /// read-only — the definition of the thing you clicked is exactly as
    /// interesting when it lives in esp-hal as when it lives in your crate,
    /// and "nothing happens" was how most Ctrl+clicks used to end.
    pub external: bool,
}

/// What the server said about a position, and how much text it covers.
///
/// The range is what makes the tooltip liveable: while the pointer stays on
/// the same token there is nothing to re-request and nothing to dismiss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverInfo {
    pub text: String,
    /// Scalar columns, like everything the frontend touches. Absent when the
    /// server did not say; the caller falls back to the queried cell.
    pub range: Option<EditRange>,
}
