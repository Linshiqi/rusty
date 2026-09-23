//! An editor group's state: what is open and parked, the popups over the
//! text, the undo histories, where the caret has been, and the find bar.

use super::*;

/// An open editor that is not on screen — everything needed to come back
/// exactly as left.
///
/// The draft is the load-bearing field: parking is what makes switching tabs
/// safe with unsaved edits in both. The highlight is carried so the return
/// is instant rather than a white flash and a re-request.
#[derive(Clone, Debug, PartialEq)]
pub struct ParkedEditor {
    pub document: rusty_edit::Document,
    pub draft: String,
    pub highlighted: Vec<rusty_edit::Line>,
    /// Where the caret was, as (line, scalar column), when it could be read.
    pub caret: Option<(u32, u32)>,
    /// And its collapsed regions. Carried for the same reason the caret is:
    /// coming back to a tab should be coming back to what you were looking
    /// at, and a file that unfolds itself every time you glance at another
    /// one is a fold feature nobody uses twice.
    pub folds: rusty_edit::Folded,
    /// Which backend painting its lines are, and which it shows plain.
    pub paint: PaintState,
    /// Where the working area was scrolled to, as (top, left) pixels of
    /// whichever scroller was showing the tab — the code surface's, the
    /// Markdown page's or the picture's. The caret says where the user was
    /// typing; this says what they were looking at, which after a long read
    /// of a chapter is somewhere else entirely.
    pub viewport: (i32, i32),
}

/// Places a command asked the language server for — a symbol's references,
/// its implementations — which the finder lists under `title` in place of
/// files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceList {
    pub title: String,
    pub places: Vec<rusty_lsp::Place>,
}

/// Inlay hints rust-analyzer gave for a file: which file, the lines they were
/// asked for over, and the hints, in line order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HintSet {
    pub path: String,
    pub lines: (u32, u32),
    pub hints: Vec<rusty_lsp::InlayHint>,
}

/// What the dock's Calls tab shows (`view/dock/calls.rs`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum CallsView {
    /// Nothing asked for yet.
    #[default]
    Idle,
    /// The server is being asked what function the caret is in; the ask's
    /// number, so an older answer is not shown for a newer ask.
    Asking(u64),
    /// The caret was on nothing a hierarchy starts from — the word it was on.
    NoFunction(String),
    Tree(crate::calls::CallTree),
}

/// Symbols the finder asked for, and the ask they answer: `@` and the file's
/// path, or `#` and the words typed. An answer to an older ask is never
/// shown under a newer one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolAnswer {
    pub ask: String,
    pub symbols: Vec<rusty_lsp::Symbol>,
}

/// How an editor's painted lines stand against the backend's painting of
/// the file (`crate::paint` has the why).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintState {
    /// The number of the backend painting the lines are, outside `stale`.
    /// `None` when they are no painting the backend keeps, and the next
    /// repaint is the whole file.
    pub version: Option<u32>,
    /// The lines on screen as plain text since then.
    pub stale: crate::paint::Lines,
}

/// A repaint on its way. One at a time per group: each answer is the base
/// the next ask is measured against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaintAsk {
    /// Which ask this is. Anything that puts a whole painting on screen drops
    /// the ask, and an answer finding another ask, or none, is dropped too.
    pub serial: u64,
    pub path: String,
    /// The text that was sent, which the answer's line numbers are about.
    pub sent: String,
    /// Lines edited while it was out: what is still plain once it lands.
    pub since: crate::paint::Lines,
    /// Edits arrived while it was out, so ask again when it lands.
    pub again: bool,
}

/// A viewport to put back once a tab's view is on screen.
///
/// Set by the controller when a parked tab is fronted, and once when a fresh
/// document replaces another, consumed by the view that owns the scroller.
/// It exists because the scroller is one DOM element for every document that
/// passes through it: switching tabs left the new document at the old one's
/// offset, and every switch cost a scroll back to where you were.
#[derive(Clone, Debug, PartialEq)]
pub struct ParkedViewport {
    pub path: String,
    pub top: i32,
    pub left: i32,
    /// The caret to place first, when the tab had one — placed without
    /// scrolling, so the viewport below is what decides where the eye lands.
    pub caret: Option<(u32, u32)>,
}

/// The editor's own undo history.
///
/// It has to be ours: the editor writes the textarea's value programmatically
/// on every echo and format, and each such write wipes the browser's native
/// undo stack — Ctrl+Z was dead air until this existed. Whole-text snapshots,
/// coalesced per typing burst; the caret after a restore is recomputed from
/// where the two texts diverge, so nothing else needs remembering.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditHistory {
    pub undo: Vec<String>,
    pub redo: Vec<String>,
    /// When the last snapshot was pushed (ms), for burst coalescing.
    pub last_push: f64,
}

impl Editor {
    /// Work on the undo history of `path`, made empty the first time it is
    /// asked for.
    pub fn with_history<T>(
        &self,
        path: &str,
        work: impl FnOnce(&mut EditHistory) -> T,
    ) -> Option<T> {
        self.histories
            .try_update_value(|all| work(all.entry(path.to_string()).or_default()))
    }

    /// Let a file's history go: the file is open nowhere now, or is a
    /// different file than the one the history was of.
    pub fn forget_history(&self, path: &str) {
        self.histories.update_value(|all| {
            all.remove(path);
        });
    }
}

impl EditHistory {
    /// Undo steps kept, at most.
    pub const STEPS: usize = 200;
    /// And bytes of them. Every step is the whole text, so two hundred steps
    /// of a two-megabyte file would be four hundred megabytes of the window's
    /// memory — held again by every tab parked with them.
    pub const BYTES: usize = 48 * 1024 * 1024;

    /// Drop the oldest undo steps past either limit, keeping the newest one
    /// however large it is: a file that is its own whole budget still undoes
    /// the last thing done to it.
    pub fn trim(&mut self) {
        let mut bytes: usize = self.undo.iter().map(String::len).sum();
        let mut drop = 0;
        while self.undo.len() - drop > 1
            && (self.undo.len() - drop > Self::STEPS || bytes > Self::BYTES)
        {
            bytes -= self.undo[drop].len();
            drop += 1;
        }
        self.undo.drain(..drop);
    }
}

/// The tooltip under the pointer: what the server said, and what it offers
/// to do about it.
///
/// The fixes travel *with* the prose rather than in a signal of their own,
/// because a card describing one position while its buttons rewrite another
/// is a click nobody meant to make. They are asked for only where the token
/// carries a diagnostic — an `impl Trait for T {}` missing its members is the
/// case this exists for, and asking on every hover would put a `codeAction`
/// round trip, resolves and all, behind every idle mouse.
#[derive(Clone, Debug, PartialEq)]
pub struct HoverCard {
    pub path: String,
    /// The token's own span. What "the pointer is still on it" is measured
    /// against, so the card does not close under a reader.
    pub range: EditRange,
    pub text: String,
    /// Where the fixes were asked for — the position they splice against.
    pub line: u32,
    pub col: u32,
    pub fixes: rusty_lsp::CodeActions,
}

/// A completion answer, anchored where it was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionPopup {
    pub path: String,
    pub line: u32,
    /// Where the word being completed starts — what typed text filters
    /// against, and what an accepted item replaces when the server sent no
    /// edit range of its own.
    pub word_start: u32,
    pub items: Vec<rusty_lsp::CompletionItem>,
    /// Ask again as the word grows, rather than only narrowing `items`.
    pub incomplete: bool,
    /// The server's number for this answer, for resolving an accepted item.
    pub reply: u64,
    /// The ask this answers — see [`CompletionAsk`]. An answer to an older
    /// ask never replaces one to a newer.
    pub asked: u64,
}

/// What the completion popup is waiting on. Every ask is numbered, and so
/// is every dismissal; an answer is shown only while the word it was asked
/// about is still the word being typed — same file, same line, same start.
/// An answer used to be shown wherever it landed: type `foo` and Enter
/// quickly, and the popup for `foo` opened on the line below and took the
/// next Enter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompletionAsk {
    pub count: u64,
    /// Path, line and word start of the word being completed, while one is.
    pub anchor: Option<(String, u32, u32)>,
}

/// One place the caret has been, for going back to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavPoint {
    pub path: String,
    pub line: u32,
    pub col: u32,
}

/// Where the editor has been, and where in that it currently is.
///
/// Browser semantics rather than Vim's own jumplist: one list of positions
/// with a cursor into it, so Back and Forward are the same list read in two
/// directions. Vim's `Ctrl+O`/`Ctrl+I` are one caller; the menu is another,
/// and both must agree — two histories would disagree on the first jump.
///
/// In memory, not in a file: it describes this window's reading session, and
/// losing it costs a shrug. That is exactly the test the storage rule asks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavHistory {
    /// Oldest first. Always contains the current position at [`Self::at`]
    /// once anything has been recorded.
    pub entries: Vec<NavPoint>,
    pub at: usize,
}

impl NavHistory {
    /// How far back it is worth being able to go. Beyond this the oldest
    /// entries are dropped, because a reading session is not a log.
    const CAP: usize = 100;

    /// Record a jump from one position to another.
    ///
    /// Truncates whatever was ahead, the way a browser does: once you go back
    /// and then somewhere new, the branch you left is gone. Keeping it would
    /// make Forward land somewhere the reader never chose.
    pub fn jump(&mut self, from: NavPoint, to: NavPoint) {
        if from == to {
            return;
        }
        self.entries.truncate(self.at + 1);
        if self.entries.last() != Some(&from) {
            self.entries.push(from);
        }
        self.entries.push(to);
        if self.entries.len() > Self::CAP {
            let over = self.entries.len() - Self::CAP;
            self.entries.drain(..over);
        }
        self.at = self.entries.len() - 1;
    }

    pub fn back(&mut self) -> Option<NavPoint> {
        if self.at == 0 {
            return None;
        }
        self.at -= 1;
        self.entries.get(self.at).cloned()
    }

    pub fn forward(&mut self) -> Option<NavPoint> {
        if self.at + 1 >= self.entries.len() {
            return None;
        }
        self.at += 1;
        self.entries.get(self.at).cloned()
    }

    pub fn can_go_back(&self) -> bool {
        self.at > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.at + 1 < self.entries.len()
    }
}

/// A group's open files in the order they were last on screen, most recent
/// first — Ctrl+Tab's order, VS Code's "most recently used editor in group".
///
/// A ranking, reconciled with the strip each time it is read ([`Self::order`])
/// rather than kept in step with it: a tab that closed, moved to the other
/// group or was renamed drops out on the next read, and a tab nobody has
/// fronted yet — a strip restored from last session — follows the ones that
/// were, in strip order. Keeping it in step would be one more list of every
/// place a tab can change, and a list like that drifts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentEditors {
    paths: Vec<String>,
}

impl RecentEditors {
    /// Far more than a strip holds; past it the oldest go, as `NavHistory`'s do.
    const CAP: usize = 64;

    /// `path` has just been put on screen.
    pub fn touch(&mut self, path: &str) {
        self.paths.retain(|recent| recent != path);
        self.paths.insert(0, path.to_string());
        self.paths.truncate(Self::CAP);
    }

    /// Every tab on the strip, the one on screen first, then the rest by how
    /// recently they were, then any never fronted in strip order.
    pub fn order(&self, tabs: &[String], active: Option<&str>) -> Vec<String> {
        let mut out: Vec<String> = Vec::with_capacity(tabs.len());
        let first = active.into_iter().map(str::to_string);
        for path in first
            .chain(self.paths.iter().cloned())
            .chain(tabs.iter().cloned())
        {
            if tabs.contains(&path) && !out.contains(&path) {
                out.push(path);
            }
        }
        out
    }
}

/// Ctrl+Tab's list, while Ctrl is held: whose files, in recent order, and the
/// one letting go will open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switcher {
    pub group: Group,
    pub paths: Vec<String>,
    pub at: usize,
    /// Drawn yet. A tap — Ctrl+Tab and straight off — switches before the
    /// list would appear, so flipping between two files never flashes it.
    pub shown: bool,
}

impl Switcher {
    /// The list for `paths` (the file on screen first), on the file before
    /// it — so a tap goes back, and a second tap comes back again — or, going
    /// backwards, on the least recent. Nothing when there is nothing to
    /// switch to.
    pub fn open(group: Group, paths: Vec<String>, back: bool) -> Option<Self> {
        if paths.len() < 2 {
            return None;
        }
        let at = if back { paths.len() - 1 } else { 1 };
        Some(Switcher {
            group,
            paths,
            at,
            shown: false,
        })
    }

    /// One more press: the next row, or the one before, wrapping at both ends.
    pub fn step(&mut self, back: bool) {
        let count = self.paths.len();
        if count == 0 {
            return;
        }
        self.at = if back {
            (self.at + count - 1) % count
        } else {
            (self.at + 1) % count
        };
    }

    pub fn picked(&self) -> Option<&str> {
        self.paths.get(self.at).map(String::as_str)
    }
}

/// What `modal::vim_key` last set on the textarea: the file, the selection
/// in the UTF-16 units the DOM reports, and the cursor (a scalar index) that
/// selection was drawn for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimCaret {
    pub path: Option<String>,
    pub start: u32,
    pub end: u32,
    pub cursor: usize,
}

/// The document in front of you and everything that follows the caret.
///
/// `draft` is the truth while typing; `document` is what the backend last
/// sent. They differ by exactly the unsaved edits.
#[derive(Clone, Copy)]
pub struct Editor {
    /// The project's files, and the one being looked at.
    ///
    /// The draft is held apart from the document so an unsaved edit survives
    /// re-highlighting: the highlighted lines come from the backend and are
    /// replaced wholesale, and folding the text into them would lose whatever
    /// had been typed since.
    pub tree: RwSignal<Vec<Entry>>,
    /// The `.rs` files in the tree that no `mod` declaration reaches, so the
    /// tree can dim them as VS Code dims a file outside the project. Shared
    /// by both groups: it is a fact about the project, not about an editor.
    /// `None` where the reading refused to claim anything at all, which is
    /// not the same answer as claiming nothing is unlinked — see
    /// [`AppState::is_unlinked`] and `rusty_edit::modules`.
    pub unlinked: RwSignal<Option<Vec<String>>>,
    pub document: RwSignal<Option<Document>>,
    pub draft: RwSignal<String>,
    /// The lines being painted, live. Seeded from the opened document, patched
    /// plainly on each keystroke so typed text appears instantly, replaced by a
    /// re-highlight when typing pauses.
    pub highlighted: RwSignal<Vec<Line>>,
    /// The text `highlighted` currently depicts — the reference for the
    /// keystroke patch. Not the same as `draft` for the milliseconds between
    /// an input event and the patch.
    pub echo_text: RwSignal<String>,
    /// Which backend painting `highlighted` is, and which lines it shows
    /// plain. Not reactive: nothing draws it.
    pub paint: StoredValue<PaintState>,
    /// The repaint on its way, if one is.
    pub painting: StoredValue<Option<PaintAsk>>,
    /// Bumped on every keystroke, so the pulse fires once typing pauses.
    pub pulse_gen: RwSignal<u64>,
    /// What the server said about the position under the mouse. The range is
    /// what keeps the card up while the pointer moves within the same token.
    pub hover: RwSignal<Option<HoverCard>>,
    /// The completion popup, when one is up.
    pub completion: RwSignal<Option<CompletionPopup>>,
    /// What the popup is waiting on. Not reactive: nothing draws it.
    pub completion_ask: StoredValue<CompletionAsk>,
    /// The signature card: which file and line it hangs over, and what it says.
    pub signature: RwSignal<Option<(String, u32, rusty_lsp::SignatureInfo)>>,
    /// Quick fixes offered at the caret, when the user asked (Ctrl+.): the
    /// file, the line the popup hangs under, and the answer itself — whose
    /// number is what an accepted fix is applied against.
    pub actions: RwSignal<Option<(String, u32, rusty_lsp::CodeActions)>>,
    /// Semantic colouring for the active document, as rust-analyzer sees it.
    /// Overlaid on the lexical highlight at render; empty while the index
    /// warms up, and the base colours simply show through.
    pub semantic: RwSignal<Option<(String, Vec<rusty_lsp::SemanticSpan>)>>,
    /// The other places the name at the caret occurs in this file, as the
    /// server found them once the caret rested: what the editor washes.
    pub occurrences: RwSignal<Option<(String, Vec<rusty_lsp::EditRange>)>>,
    /// The lines `semantic` covers, when it is not the whole file: a long
    /// file is asked about the lines around the ones on screen.
    pub semantic_lines: StoredValue<Option<(u32, u32)>>,
    /// The inlay hints for the document on screen, and the lines they were
    /// asked for over (`view/panels/files/hints.rs`).
    pub hints: RwSignal<Option<HintSet>>,
    /// Every cursor but the textarea's own, when there are several
    /// (`crate::cursors`, `view/panels/files/multi.rs`). Empty is one cursor.
    pub cursors: RwSignal<Vec<crate::cursors::Cursor>>,
    /// The document lines the view is drawing, first and one past the last —
    /// what a long file's semantic colours are asked for around.
    pub drawn_lines: StoredValue<(u32, u32)>,
    /// Every open editor, in strip order. The active one is [`Self::document`];
    /// the rest are parked in [`Self::parked`].
    pub tabs: RwSignal<Vec<String>>,
    /// Open editors that are not on screen, holding their unsaved drafts.
    pub parked: RwSignal<Vec<ParkedEditor>>,
    /// Undo and redo, one pair of stacks per open file, shared by both
    /// groups: a file open on both sides is one document, and an undo in
    /// either view undoes the last edit made in either. Kept per file and not
    /// per tab, so switching away and back loses nothing and there is nothing
    /// to park. Not reactive: nothing draws it.
    pub histories: StoredValue<HashMap<String, EditHistory>>,
    /// The textarea is behind the draft. An edit made in the other view of
    /// this file changes the draft here, the folds and the echo — everything
    /// on screen — and leaves the textarea, whose text is transparent, to be
    /// written when this view is next used (`controller::catch_up`).
    /// Rewriting a textarea's whole value lays the whole file out again:
    /// measured at 120 ms for 24,000 lines, twice over, on every keystroke
    /// typed on the other side, where the view being typed in lays out only
    /// what changed.
    pub lagging: RwSignal<bool>,
    /// Where the caret has been. Shared by Vim's jump keys and the menu.
    pub nav: RwSignal<NavHistory>,
    /// This group's files by how recently each was on screen: Ctrl+Tab's
    /// order. Not reactive — nothing draws it; the switcher reads it once,
    /// when it opens.
    pub recent: StoredValue<RecentEditors>,
    /// A rename waiting for its new name: where the symbol is, and what it
    /// is called now. `None` when no rename is being typed.
    pub rename: RwSignal<Option<(String, u32, u32, String)>>,
    /// Somewhere the editor should go — the result of goto-definition. Kept in
    /// state because the target file may still be opening when it is decided.
    pub reveal: RwSignal<Option<rusty_lsp::Location>>,
    /// Where a tab was scrolled to when it was parked, waiting for its view to
    /// mount and put the scroller back. A `reveal` for the same path beats
    /// it: a jump into a parked file lands on the target, not where the tab
    /// was left.
    pub viewport: RwSignal<Option<ParkedViewport>>,
    /// Directories the user has opened. Collapsed by default, because a tree
    /// that unfolds everything is a list.
    pub expanded: RwSignal<Vec<String>>,
    /// The tree's Cut or Copy, waiting for its Paste.
    pub clipboard: RwSignal<Option<TreeClip>>,
    /// The last line Ctrl+C or Ctrl+X put on the clipboard with nothing
    /// selected. A paste of exactly this text goes in as a whole line
    /// (`clip.rs`); shared by both groups, so a line copied on one side
    /// pastes as a line on the other.
    pub copied_line: RwSignal<Option<String>>,
    /// Editor font scale (Ctrl+wheel). Multiplies FONT_SIZE and every pixel
    /// the editor derives from it.
    pub zoom: RwSignal<f64>,
    /// The Markdown page's scale (Ctrl+wheel over a page), separate from the
    /// editor's: prose and a listing are read at different sizes, and one
    /// knob for both meant a chapter made comfortable left the code beside
    /// it oversized. Applied as CSS `zoom` on the page, so figures, formulas
    /// and code blocks grow with the text and the column re-wraps.
    pub page_zoom: RwSignal<f64>,
    /// Files the user asked to see as source rather than as what they draw:
    /// a Markdown file's page, an SVG's picture.
    ///
    /// The default for `.md` is the rendered view, because a workbench opens a
    /// README to read it far more often than to edit it — so this holds the
    /// exceptions rather than the rule. Session state, per path, like folds
    /// and for the same reason: which way you were reading a file yesterday
    /// is not worth restoring onto one somebody has since rewritten.
    pub source_view: RwSignal<Vec<String>>,
    /// Pictures the page view and the image view have asked for, by
    /// project-relative path: a `data:` URL once the bytes arrived, or why
    /// they did not. Shared by both groups, like the tree — the same figure
    /// in two panes is one file. Session state; the watcher drops an entry
    /// when its file changes on disk, so the next look re-reads it.
    pub images: RwSignal<HashMap<String, ImageLoad>>,
    /// Fenced code blocks the Markdown page has had highlighted, by
    /// [`snippet_key`] of their language and text: the runs once they
    /// arrived, an empty list while they are on their way. Shared by both
    /// groups like `images`, and keyed by content rather than by page, so
    /// the same block in two chapters — or in a page and an answer — is one
    /// request and can never be stale.
    pub snippets: RwSignal<HashMap<u64, Vec<Line>>>,
    /// Which regions of the active document are collapsed.
    ///
    /// Session state, per tab, deliberately not persisted: a fold is where
    /// you were looking a minute ago, and restoring yesterday's folds on a
    /// file somebody else has since edited would collapse the wrong lines.
    pub folds: RwSignal<rusty_edit::Folded>,
    /// Open files whose copy on disk changed while this window held an
    /// unsaved draft. Marked rather than reloaded: replacing a draft with the
    /// disk's text is an editor eating work, and a modal prompt per file
    /// would be unusable after a `git checkout` touching a dozen of them.
    pub stale: RwSignal<Vec<String>>,
    /// Bumped each time the project's file watcher is started, so batches
    /// from the previous project's watcher can be told apart from the live
    /// one and dropped.
    pub watch_session: RwSignal<u64>,
    /// Modal editing: whether it is on, and where it currently is.
    ///
    /// The switch belongs in `workbench.toml` rather than here — a second
    /// window has to boot into the same mode — and this signal mirrors it.
    /// The *mode* is session state: losing NORMAL on a reload costs a press
    /// of Escape.
    pub vim_on: RwSignal<bool>,
    pub vim: RwSignal<crate::vim::Vim>,
    /// The selection Vim last put on the textarea and the cursor it stands
    /// for — per group, like `vim`. See `modal::remembered_cursor` for why
    /// the textarea alone cannot say where the cursor is.
    pub vim_caret: StoredValue<Option<VimCaret>>,
    /// Write the file a beat after typing stops. Mirrors `workbench.toml`
    /// like [`Self::vim_on`], and for the same reason.
    pub auto_save: RwSignal<bool>,
    /// What the editor draws around the code — inlay hints, the minimap,
    /// sticky scroll, indent guides. Mirrors `workbench.toml`, and one for
    /// both groups.
    pub view: RwSignal<rusty_embed::EditorView>,
    /// A rust-analyzer the user named, in place of the one rusty finds.
    /// Empty is "whichever rusty finds"; it exists for the upstream bug in
    /// `rusty_lsp::convert::explain_health`, and mirrors `workbench.toml`.
    pub rust_analyzer: RwSignal<String>,
    /// Bumped on every edit; an auto-save fires only if its own number is
    /// still the latest, so a burst of typing is one write rather than one
    /// per keystroke.
    pub save_gen: RwSignal<u64>,
}

impl Editor {
    /// A group with nothing open.
    pub(super) fn fresh() -> Self {
        Editor {
            tree: RwSignal::new(Vec::new()),
            document: RwSignal::new(None),
            draft: RwSignal::new(String::new()),
            highlighted: RwSignal::new(Vec::new()),
            echo_text: RwSignal::new(String::new()),
            paint: StoredValue::new(PaintState::default()),
            painting: StoredValue::new(None),
            pulse_gen: RwSignal::new(0),
            hover: RwSignal::new(None),
            completion: RwSignal::new(None),
            completion_ask: StoredValue::new(CompletionAsk::default()),
            signature: RwSignal::new(None),
            actions: RwSignal::new(None),
            semantic: RwSignal::new(None),
            semantic_lines: StoredValue::new(None),
            hints: RwSignal::new(None),
            cursors: RwSignal::new(Vec::new()),
            occurrences: RwSignal::new(None),
            drawn_lines: StoredValue::new((0, 0)),
            tabs: RwSignal::new(Vec::new()),
            parked: RwSignal::new(Vec::new()),
            histories: StoredValue::new(HashMap::new()),
            lagging: RwSignal::new(false),
            nav: RwSignal::new(NavHistory::default()),
            recent: StoredValue::new(RecentEditors::default()),
            rename: RwSignal::new(None),
            reveal: RwSignal::new(None),
            viewport: RwSignal::new(None),
            expanded: RwSignal::new(Vec::new()),
            clipboard: RwSignal::new(None),
            copied_line: RwSignal::new(None),
            source_view: RwSignal::new(Vec::new()),
            images: RwSignal::new(HashMap::new()),
            snippets: RwSignal::new(HashMap::new()),
            folds: RwSignal::new(rusty_edit::Folded::default()),
            stale: RwSignal::new(Vec::new()),
            unlinked: RwSignal::new(None),
            watch_session: RwSignal::new(0),
            zoom: RwSignal::new(stored_zoom()),
            page_zoom: RwSignal::new(stored_page_zoom()),
            vim_on: RwSignal::new(false),
            vim: RwSignal::new(crate::vim::Vim::default()),
            vim_caret: StoredValue::new(None),
            auto_save: RwSignal::new(false),
            view: RwSignal::new(rusty_embed::EditorView::default()),
            rust_analyzer: RwSignal::new(String::new()),
            save_gen: RwSignal::new(0),
        }
    }

    /// A second group beside this one.
    ///
    /// The file tree, the expanded folders, the text zoom, the Vim switch,
    /// the per-file source-view choice and the stale-file list are the
    /// *project's*, not a group's, so the two groups share those handles —
    /// separate copies would be a zoom that only took on one side and a tab
    /// stale in one strip and not the other. Everything about what is open
    /// and how it is being edited is this group's own.
    pub(super) fn beside(&self) -> Self {
        Editor {
            tree: self.tree,
            expanded: self.expanded,
            clipboard: self.clipboard,
            copied_line: self.copied_line,
            zoom: self.zoom,
            page_zoom: self.page_zoom,
            vim_on: self.vim_on,
            auto_save: self.auto_save,
            view: self.view,
            rust_analyzer: self.rust_analyzer,
            source_view: self.source_view,
            images: self.images,
            snippets: self.snippets,
            histories: self.histories,
            stale: self.stale,
            unlinked: self.unlinked,
            watch_session: self.watch_session,
            ..Self::fresh()
        }
    }
}

/// One picture's way to the screen: asked for, arrived as a `data:` URL, or
/// refused with the reason — a file that is not there, or one too large to
/// load as a picture. `Loading` is in the map so a page re-rendered on every
/// keystroke asks for each figure once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageLoad {
    Loading,
    Ready(String),
    Failed(String),
}

/// The key a fenced code block is cached under: its language and its text,
/// hashed — a block is the same block wherever it appears, and a block that
/// changed by one character is another one.
pub fn snippet_key(lang: &str, text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lang.hash(&mut hasher);
    text.hash(&mut hasher);
    hasher.finish()
}

/// Which editor group a state value addresses. Two at most: VS Code's
/// everyday split, and the point past which a comparison becomes a tiling
/// window manager.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Group {
    #[default]
    First,
    Second,
}

impl Group {
    pub fn other(self) -> Group {
        match self {
            Group::First => Group::Second,
            Group::Second => Group::First,
        }
    }

    /// 0 for the first group, 1 for the second: the index into `Groups`, and
    /// the tag a group's scroller carries so the controller can find it.
    pub fn index(self) -> usize {
        match self {
            Group::First => 0,
            Group::Second => 1,
        }
    }
}

/// Both groups' handles, in `AppState` so any state value can reach either.
#[derive(Clone, Copy)]
pub struct Groups {
    pub editors: [Editor; 2],
    pub finds: [Find; 2],
}

/// Find and replace *within* the open document — the bar, not the panel.
#[derive(Clone, Copy)]
pub struct Find {
    /// The in-file find bar. Survives tab switches, as every editor's does;
    /// resets with the project.
    pub open: RwSignal<bool>,
    pub replace_open: RwSignal<bool>,
    pub query: RwSignal<String>,
    pub case: RwSignal<bool>,
    pub replace: RwSignal<String>,
    /// Which match is current, clamped to the match count at use.
    pub index: RwSignal<usize>,
}

impl Find {
    /// A closed bar with nothing typed. One per editor group: a find bar open
    /// on one side must not open on the other.
    pub(super) fn fresh() -> Self {
        Find {
            open: RwSignal::new(false),
            replace_open: RwSignal::new(false),
            query: RwSignal::new(String::new()),
            case: RwSignal::new(false),
            replace: RwSignal::new(String::new()),
            index: RwSignal::new(0),
        }
    }
}

/// What the tree's Cut or Copy took, until Paste uses it.
///
/// A Cut entry is drawn dimmed until it lands, as VS Code draws it; a Copy
/// stays available for another paste. In `Editor` rather than in the tree
/// component, so switching panels between the cut and the paste does not
/// lose it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeClip {
    pub path: String,
    pub is_dir: bool,
    pub cut: bool,
}

#[cfg(test)]
mod nav_tests {
    use super::*;

    fn at(path: &str, line: u32) -> NavPoint {
        NavPoint {
            path: path.into(),
            line,
            col: 0,
        }
    }

    #[test]
    fn back_returns_where_the_jump_started() {
        // The case the whole thing exists for: follow a definition three
        // deep, then walk out the way you came.
        let mut nav = NavHistory::default();
        nav.jump(at("main.rs", 10), at("hal.rs", 200));
        nav.jump(at("hal.rs", 200), at("gpio.rs", 40));

        assert_eq!(nav.back(), Some(at("hal.rs", 200)));
        assert_eq!(nav.back(), Some(at("main.rs", 10)));
        assert_eq!(nav.back(), None, "and stops at the beginning");
    }

    #[test]
    fn forward_only_retraces_what_back_undid() {
        let mut nav = NavHistory::default();
        nav.jump(at("main.rs", 10), at("hal.rs", 200));
        nav.back();
        assert_eq!(nav.forward(), Some(at("hal.rs", 200)));
        assert_eq!(nav.forward(), None);
    }

    #[test]
    fn a_new_jump_after_going_back_drops_the_branch() {
        // Browser semantics. Keeping the abandoned branch would make Forward
        // land somewhere the reader never chose to go.
        let mut nav = NavHistory::default();
        nav.jump(at("main.rs", 10), at("hal.rs", 200));
        nav.back();
        nav.jump(at("main.rs", 10), at("spi.rs", 5));

        assert!(!nav.can_go_forward(), "the old forward branch is gone");
        assert_eq!(nav.back(), Some(at("main.rs", 10)));
    }

    #[test]
    fn jumping_to_where_you_already_are_records_nothing() {
        // Clicking a problem on the line the caret is already on is not a
        // jump, and recording it would make Back a no-op that looks broken.
        let mut nav = NavHistory::default();
        nav.jump(at("main.rs", 10), at("main.rs", 10));
        assert!(nav.entries.is_empty());
        assert!(!nav.can_go_back());
    }

    #[test]
    fn the_same_origin_twice_is_recorded_once() {
        // Two jumps out of one place should need one Back to get home, not
        // two presses that appear to do nothing the first time.
        let mut nav = NavHistory::default();
        nav.jump(at("main.rs", 10), at("hal.rs", 200));
        nav.back();
        nav.jump(at("main.rs", 10), at("hal.rs", 300));
        assert_eq!(nav.back(), Some(at("main.rs", 10)));
        assert_eq!(nav.back(), None);
    }

    #[test]
    fn the_list_is_capped_and_keeps_the_recent_end() {
        let mut nav = NavHistory::default();
        for line in 0..200 {
            nav.jump(at("main.rs", line), at("main.rs", line + 1));
        }
        assert!(nav.entries.len() <= NavHistory::CAP);
        assert_eq!(
            nav.entries.last(),
            Some(&at("main.rs", 200)),
            "the newest position survives the cap",
        );
        assert_eq!(nav.at, nav.entries.len() - 1, "and stays pointed at it");
    }
}

#[cfg(test)]
mod switcher_tests {
    use super::*;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|path| path.to_string()).collect()
    }

    /// Visited `b`, then `c`, then back to `a`: the one on screen, then the
    /// last one before it, then the one before that — and a tab never
    /// fronted since the strip was restored comes last, in strip order.
    #[test]
    fn the_order_is_the_one_on_screen_then_most_recent_then_the_strip() {
        let tabs = paths(&["a.rs", "b.rs", "c.rs", "restored.rs"]);
        let mut recent = RecentEditors::default();
        for path in ["b.rs", "c.rs", "a.rs"] {
            recent.touch(path);
        }
        assert_eq!(
            recent.order(&tabs, Some("a.rs")),
            paths(&["a.rs", "c.rs", "b.rs", "restored.rs"])
        );
    }

    /// Nothing is kept in step with the strip: a closed tab simply is not
    /// listed, and a tab that went to the other group is not this group's.
    #[test]
    fn a_tab_no_longer_on_the_strip_drops_out() {
        let mut recent = RecentEditors::default();
        for path in ["closed.rs", "b.rs", "a.rs"] {
            recent.touch(path);
        }
        assert_eq!(
            recent.order(&paths(&["a.rs", "b.rs"]), Some("a.rs")),
            paths(&["a.rs", "b.rs"])
        );
    }

    /// The case a tap exists for: Ctrl+Tab lands on the file before this
    /// one, and from there on this one — two files flipped at the speed of
    /// the key.
    #[test]
    fn a_tap_opens_the_file_before_and_the_next_tap_comes_back() {
        let tabs = paths(&["a.rs", "b.rs", "c.rs"]);
        let mut recent = RecentEditors::default();
        for path in ["c.rs", "a.rs", "b.rs"] {
            recent.touch(path);
        }
        let first = Switcher::open(Group::First, recent.order(&tabs, Some("b.rs")), false)
            .expect("three files to switch between");
        assert_eq!(first.picked(), Some("a.rs"));

        recent.touch("a.rs");
        let second = Switcher::open(Group::First, recent.order(&tabs, Some("a.rs")), false)
            .expect("still three");
        assert_eq!(second.picked(), Some("b.rs"));
    }

    /// Held, each Tab walks down and each Shift+Tab up, round at both ends;
    /// Ctrl+Shift+Tab alone starts from the least recent.
    #[test]
    fn a_held_list_steps_both_ways_and_wraps() {
        let list = paths(&["a.rs", "b.rs", "c.rs"]);
        let mut switcher = Switcher::open(Group::First, list.clone(), false).unwrap();
        switcher.step(false);
        assert_eq!(switcher.picked(), Some("c.rs"));
        switcher.step(false);
        assert_eq!(switcher.picked(), Some("a.rs"), "past the end is the top");
        switcher.step(true);
        assert_eq!(switcher.picked(), Some("c.rs"), "and back up");

        let backwards = Switcher::open(Group::Second, list, true).unwrap();
        assert_eq!(backwards.picked(), Some("c.rs"));
    }

    #[test]
    fn one_file_has_nothing_to_switch_to() {
        assert!(Switcher::open(Group::First, paths(&["a.rs"]), false).is_none());
        assert!(Switcher::open(Group::First, Vec::new(), true).is_none());
    }

    #[test]
    fn the_ranking_keeps_its_newest_and_forgets_its_oldest() {
        let mut recent = RecentEditors::default();
        for index in 0..100 {
            recent.touch(&format!("{index}.rs"));
        }
        recent.touch("7.rs");
        let tabs: Vec<String> = (0..100).map(|index| format!("{index}.rs")).collect();
        let order = recent.order(&tabs, None);
        assert_eq!(order[0], "7.rs", "a file touched again moves to the front");
        assert_eq!(order[1], "99.rs");
        assert_eq!(
            order.len(),
            100,
            "a forgotten tab still lists, in strip order"
        );
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    fn with(sizes: &[usize]) -> EditHistory {
        EditHistory {
            undo: sizes.iter().map(|&size| "x".repeat(size)).collect(),
            ..EditHistory::default()
        }
    }

    #[test]
    fn the_oldest_steps_go_past_the_step_limit() {
        let mut history = with(&vec![1; EditHistory::STEPS + 3]);
        history.trim();
        assert_eq!(history.undo.len(), EditHistory::STEPS);
    }

    /// A long file keeps as many steps as fit the budget, newest first, and
    /// always the newest one, however large it is.
    #[test]
    fn the_oldest_steps_go_past_the_byte_budget_but_never_the_last() {
        let third = EditHistory::BYTES / 3;
        let mut history = with(&[third, third, third, third + 1]);
        history.trim();
        assert_eq!(history.undo.len(), 2);
        assert_eq!(history.undo[1].len(), third + 1, "the newest stays");

        let mut history = with(&[EditHistory::BYTES * 2]);
        history.trim();
        assert_eq!(history.undo.len(), 1);
    }
}
