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

/// One text replacement inside a code action, scalar-addressed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionEdit {
    pub range: EditRange,
    pub new_text: String,
}

/// A quick fix or refactoring the server offers at a position, with its
/// edits already resolved — the frontend applies text, it never negotiates.
///
/// Only single-file actions travel: an action that would touch other files
/// is dropped by the client rather than half-applied, until a multi-file
/// apply path exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionFix {
    pub title: String,
    /// `quickfix`, `refactor.rewrite`… when the server said.
    pub kind: Option<String>,
    pub edits: Vec<ActionEdit>,
}

/// One file's share of a rename.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEdits {
    pub path: String,
    pub edits: Vec<ActionEdit>,
}

/// What renaming a symbol would change, everywhere.
///
/// Unlike a code action this must *not* refuse when other files are
/// involved: a `pub fn` renamed in one file and not its callers is a broken
/// build, and that is the normal case rather than the exception.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameEdits {
    pub files: Vec<FileEdits>,
}

/// One run of semantic colour, as rust-analyzer sees the code.
///
/// The kind is the server's own legend name (`function`, `struct`,
/// `parameter`…) — named rather than numbered, because a number on the wire
/// invites the frontend to keep its own copy of the legend. Columns and
/// length are Unicode scalars, converted at the boundary like every other
/// position here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSpan {
    pub line: u32,
    pub start_col: u32,
    pub length: u32,
    pub kind: String,
}

/// The signature of the call the caret is inside, with the parameter it is on.
///
/// One signature, not a list: Rust has no overloading, so the "active
/// signature" the protocol allows for is the only one worth shipping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureInfo {
    /// The whole signature as text, e.g. `fn set_gain(&mut self, db: i8)`.
    pub label: String,
    /// Byte range of the active parameter inside `label` — bytes, so the
    /// frontend slices directly; the backend has already unwound the
    /// protocol's UTF-16 offsets.
    pub param_start: Option<u32>,
    pub param_end: Option<u32>,
    /// The signature's documentation, when the server sent any.
    pub doc: Option<String>,
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
