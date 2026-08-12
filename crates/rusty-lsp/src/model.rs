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

/// Somewhere in the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    /// Relative to the project root, `/`-separated — the same identity the
    /// file tree uses.
    pub path: String,
    pub line: u32,
    pub col: u32,
}
