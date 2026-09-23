//! The language server, the project search, the debugger and the terminal.

use super::*;

/// Whether the language server behind the editor is up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LspStatus {
    /// No project, or the server was never asked for.
    Off,
    Starting,
    Ready,
    /// Could not start — usually not installed. The editor still works; the
    /// squiggles and completion do not.
    Missing,
}

/// Project-wide search. Separate from [`Find`] because they are different
/// questions asked of different things, and sharing a query string made one
/// of them clobber the other.
#[derive(Clone, Copy)]
pub struct Search {
    /// Project search. Kept here rather than in the panel so the results
    /// survive switching away and back.
    pub query: RwSignal<String>,
    pub case: RwSignal<bool>,
    pub word: RwSignal<bool>,
    pub regex: RwSignal<bool>,
    /// `*.rs, src/**` — gitignore-style globs, as the boxes in the panel.
    pub include: RwSignal<String>,
    pub exclude: RwSignal<String>,
    pub results: RwSignal<Option<rusty_edit::SearchResults>>,
    /// Which search is current; a stale reply is dropped, and the debounce
    /// timer checks it before firing.
    pub generation: RwSignal<u64>,
    /// The replacement text, and whether its box is showing. Folded away by
    /// default, as VSCode folds it: a replace field always on screen is an
    /// invitation to a project-wide rewrite nobody asked for.
    pub replacement: RwSignal<String>,
    pub replacing: RwSignal<bool>,
    /// What the last replace did. Held rather than shown and forgotten —
    /// it names the files it would not touch, and that is the half somebody
    /// has to act on.
    pub outcome: RwSignal<Option<rusty_edit::ReplaceOutcome>>,
}

/// The language server: whether it is up, which session is live, and what
/// it has said about each file.
#[derive(Clone, Copy)]
pub struct Lsp {
    pub status: RwSignal<LspStatus>,
    /// Which start_lsp call owns the event channel; stale channels' events are
    /// dropped rather than fighting the new server over the status signal.
    pub session: RwSignal<u64>,
    /// What the compiler and rust-analyzer think is wrong, by file.
    pub diagnostics: RwSignal<HashMap<String, Vec<FileDiagnostic>>>,
    /// What the server is busy with, while it is — `Indexing 26% …` — so an
    /// empty completion reads as "not yet" rather than "none".
    pub progress: RwSignal<Option<String>>,
    /// What the server says about itself, once it has said anything worse
    /// than `ok`: the level and the reason. A rust-analyzer that failed to
    /// load the workspace still parses every file, so the squiggles arrive
    /// and nothing else ever does — the one broken state that used to look
    /// exactly like a working one.
    pub health: RwSignal<Option<(rusty_lsp::HealthLevel, Option<String>)>>,
}

/// The debug session, its breakpoints, and the chip's registers.
#[derive(Clone, Copy)]
pub struct Debug {
    /// The live debug session's state, or `None` when nothing is being
    /// debugged. Everything the gutter, the floating transport and the Debug panel
    /// draw comes from this one value.
    pub session: RwSignal<Option<rusty_dbg::DebugState>>,
    /// Which session's frames are current — the same generation guard the
    /// terminal needed, for the same reason.
    pub epoch: RwSignal<u64>,
    /// Breakpoints the user has set, as `(file, zero-based line)`.
    ///
    /// Editor state, not session state: every debugger lets you place
    /// breakpoints before starting, and holding them inside `DebugState`
    /// meant a click did nothing until a session existed — which is
    /// backwards, since placing them is how you decide where to stop.
    /// A starting session sends this list; gdb's answers come back in
    /// `debug` and decorate these.
    pub breakpoints: RwSignal<Vec<(String, u32)>>,
    /// The chip's peripherals, once an SVD has been read. `None` means not
    /// asked yet; `Some(None)` means asked and this machine has no file.
    pub registers: RwSignal<Option<Option<rusty_embed::RegisterMap>>>,
    /// Which peripheral the register view is showing.
    pub peripheral: RwSignal<Option<String>>,
}

/// The shell, and which shell it is.
#[derive(Clone, Copy)]
pub struct Terminal {
    /// The terminal's latest frame, when a shell is open.
    ///
    /// Whole screens rather than an append-only log: a pty is a screen, and
    /// programs that redraw — every progress bar, every prompt redraw after a
    /// backspace — overwrite what is there rather than adding to it.
    pub screen: RwSignal<Option<TermScreen>>,
    /// Which terminal session is current. Bumped by every open; a session's
    /// frame and completion callbacks compare before writing, so a replaced
    /// session's late "the shell is gone" cannot blank the one that
    /// replaced it — which looked like the terminal flickering for ever.
    pub epoch: RwSignal<u64>,
    /// What shell the terminal will start, from the backend.
    pub info: RwSignal<Option<rusty_term::ShellInfo>>,
    /// What the shell picker offers: the built-in plus every shell the
    /// backend actually found on this machine.
    pub choices: RwSignal<Vec<rusty_term::ShellChoice>>,
}
