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
    /// What the server is busy with — `Indexing 12/45 esp-hal`, `Fetching`,
    /// `Building build-artifacts` — or `None` once nothing is. `Ready` says
    /// the protocol is up; this says why a request may still come back
    /// empty, which is the difference between "no completion" and "not yet".
    #[serde(rename_all = "camelCase")]
    Progress {
        text: Option<String>,
    },
    /// What rust-analyzer says about *itself*. A server that failed to load
    /// the workspace — `cargo metadata` refused, the toolchain it pins is
    /// missing, a manifest does not parse — still lexes and parses every
    /// file, so syntax errors keep arriving while completion, hover and
    /// go-to-definition answer nothing at all, for ever. That state used to
    /// look exactly like a working one. It arrives through
    /// `experimental/serverStatus`, which the handshake asks for, and
    /// through `window/showMessage`.
    #[serde(rename_all = "camelCase")]
    Health {
        level: HealthLevel,
        /// What went wrong, in the server's own words.
        message: Option<String>,
    },
    Exited {},
}

/// How rust-analyzer describes its own state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HealthLevel {
    /// Loaded, and answering.
    Ok,
    /// Answering, with something degraded — one linked project of several
    /// failed to load.
    Warning,
    /// Not loaded: it can parse a file, and nothing more.
    Error,
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
    /// Where the item stood in the server's reply — the handle, with
    /// [`CompletionList::reply`], for `completionItem/resolve`, which is how
    /// an item that is not yet in scope brings its `use` line along when
    /// accepted.
    pub index: u32,
    /// The server's short note beside the label: for an item that would be
    /// imported, ` (use esp_hal::gpio::Output)` — the one thing the row has
    /// to say before the user commits to it — and `(…)` for a function.
    pub label_detail: Option<String>,
    /// What the typed word is matched against, when that is not the label:
    /// rust-analyzer's postfix `.if` filters as `if`.
    #[serde(default)]
    pub filter: Option<String>,
    /// `insert` is a snippet — `$0` and `${1:x}` placeholders, which the
    /// editor expands rather than inserts.
    #[serde(default)]
    pub snippet: bool,
    /// The type or signature the row shows at its right edge.
    #[serde(default)]
    pub description: Option<String>,
}

/// One `textDocument/completion` answer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionList {
    pub items: Vec<CompletionItem>,
    /// The server left out what a longer word might bring — rust-analyzer
    /// always says so, because its imports are searched by the word typed.
    /// Such a list is asked for again as the word grows, never only
    /// narrowed.
    pub incomplete: bool,
    /// Which answer this is. An accepted item is resolved against the
    /// server's own copy of the answer it came from, and the popup asks on
    /// every keystroke, so the newest answer is often not that one.
    pub reply: u64,
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

/// A place the server named, with its line as it reads now — a row in a list
/// of references or implementations, which nobody chooses from by line
/// number alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub location: Location,
    /// Where the name ends on that line, in scalars: what the row marks.
    pub end_col: u32,
    /// The line, or its first thousand characters. Empty when the file could
    /// not be read.
    pub text: String,
}

/// A function in a call hierarchy: its name, where it is, and the server's
/// own description of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallItem {
    pub name: String,
    /// What it is, in words, as a [`Symbol`]'s kind is.
    pub kind: String,
    /// What the server says beside the name — for rust-analyzer, the
    /// signature.
    pub detail: Option<String>,
    /// Its name, with the line it is on.
    pub place: Place,
    /// The item exactly as the server sent it. Nobody reads it but the
    /// server: the next level of the hierarchy is asked for by handing it
    /// back, and the server is free to put whatever it needs in there.
    pub item: String,
}

/// One end of a call: the function at the other end, and every place the
/// call is made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    /// The caller, for the calls into a function; the callee, for the calls
    /// out of one.
    pub item: CallItem,
    /// Where the calls are, with their lines: in the caller's file either
    /// way — `item`'s for a call in, the asked function's for a call out.
    pub sites: Vec<Place>,
}

/// What a macro call turns into, fully expanded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroExpansion {
    /// The macro, by the name it was called with.
    pub name: String,
    pub expansion: String,
}

/// Something with a name in the code: a function, a struct, an impl block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Symbol {
    pub name: String,
    /// What it is, in words: `function`, `struct`, `trait`, `impl`.
    pub kind: String,
    /// What it sits in — `impl Point`, `mod regs` — which tells apart two
    /// `new`s in one file.
    pub container: Option<String>,
    /// How deep in its file's outline; 0 for a workspace search's answers.
    pub depth: u32,
    pub location: Location,
}

/// One text replacement inside a code action, scalar-addressed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionEdit {
    pub range: EditRange,
    pub new_text: String,
}

/// A quick fix or refactoring the server offers at a position, with its
/// edits for this file already resolved — the frontend applies text, it
/// never negotiates.
///
/// An action that also edits other files names them in `elsewhere`; those
/// edits stay with the client, which writes them the way a rename is written
/// once the fix is accepted (`apply_action_elsewhere`). They used to make
/// the client drop the whole action, and the fix for a file no `mod` line
/// declares — which edits *only* the parent module — was the fix nobody
/// could reach. An action that creates, renames or deletes a file is still
/// dropped whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeActionFix {
    pub title: String,
    /// `quickfix`, `refactor.rewrite`… when the server said.
    pub kind: Option<String>,
    pub edits: Vec<ActionEdit>,
    /// Project-relative paths of the other files the action changes.
    #[serde(default)]
    pub elsewhere: Vec<String>,
}

/// What the server offers to do at one position, numbered.
///
/// Numbered because two things ask now — Ctrl+. at the caret, and a hover
/// over a squiggle — and an accepted fix is applied from the server's own
/// copy of the answer by index. One slot per file was enough while the caret
/// was the only asker; with two, a hover in flight would renumber the fixes
/// an open popup is showing, and the click would write another position's
/// edits into somebody's other file. The answer says which answer it is,
/// exactly as a completion list does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodeActions {
    pub fixes: Vec<CodeActionFix>,
    pub reply: u64,
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
