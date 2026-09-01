//! Shared signals.
//!
//! Holds state and pure operations on it — no IPC, no side effects. Anything
//! that has to talk to the backend belongs in `controller`, so that a panel
//! reading state cannot accidentally trigger a fetch.

use leptos::prelude::*;

use rusty_ai::{Message, Preset, ProviderConfig, ToolDef};
use rusty_core::{FeatureImpact, FeatureRow, FeatureSelection, WorkspaceReport};
use rusty_edit::{Document, Entry, Line};
use rusty_embed::{
    Board, Chip, CommandPlan, EmbeddedProject, Explanation, Firmware, LogLine, MemoryReport, Probe,
    Problem, SerialPort, Severity, ToolchainReport, Transport, WizardChoice, WizardOption,
};
use rusty_lsp::{EditRange, FileDiagnostic};
use rusty_term::Screen as TermScreen;

use std::collections::HashMap;

use crate::ipc::IpcError;

/// Whose clock the trace timestamps are on.
///
/// Mixing the two silently is how a waveform lies, so the panel shows which
/// one it got. Firmware means `[rusty:gpio@µs]` stamps from the systimer;
/// Host means the firmware sent none and arrival time stood in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceClock {
    Firmware,
    Host,
}

/// Named numeric channels over time — the analog half of the trace.
///
/// Kept per channel rather than as rows of samples because that is how it is
/// drawn and how it arrives: firmware prints whichever channels it has this
/// loop, and a channel that starts appearing halfway through a flight is
/// normal, not a schema change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plot {
    /// Channel name → its samples, `(µs, value)`, oldest first.
    pub channels: Vec<(String, Vec<(u64, f32)>)>,
    pub clock: Option<TraceClock>,
    /// True once the cap started dropping the oldest samples.
    pub truncated: bool,
}

impl Plot {
    /// The samples for a channel, creating it on first sight.
    pub fn channel(&mut self, name: &str) -> &mut Vec<(u64, f32)> {
        if let Some(index) = self.channels.iter().position(|(known, _)| known == name) {
            return &mut self.channels[index].1;
        }
        self.channels.push((name.to_string(), Vec::new()));
        &mut self.channels.last_mut().expect("just pushed").1
    }
}

/// A captured pin trace: time-ordered `(µs, pin, level)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimTrace {
    pub events: Vec<(u64, u8, bool)>,
    pub clock: Option<TraceClock>,
    /// True when the cap was hit and the oldest events were dropped.
    pub truncated: bool,
}

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
    /// The tab's undo/redo stacks, so history survives switching away.
    pub history: EditHistory,
    /// And its collapsed regions. Carried for the same reason the caret is:
    /// coming back to a tab should be coming back to what you were looking
    /// at, and a file that unfolds itself every time you glance at another
    /// one is a fold feature nobody uses twice.
    pub folds: rusty_edit::Folded,
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

/// A completion request's results, anchored where they were asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionPopup {
    pub path: String,
    pub line: u32,
    /// Where the word being completed starts — what typed text filters
    /// against, and what an accepted item replaces when the server sent no
    /// edit range of its own.
    pub word_start: u32,
    pub items: Vec<rusty_lsp::CompletionItem>,
}

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

/// One tool call the assistant made while answering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolRun {
    pub id: String,
    pub name: String,
    /// `None` while it is still running.
    pub ok: Option<bool>,
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

/// What the bottom dock is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DockTab {
    /// Everything wrong with the project and the machine, from every source.
    Problems,
    /// What flashing and monitoring printed, with defmt levels coloured. The
    /// device talking to you.
    Output,
    /// A real shell behind a pseudo-terminal. You talking to the machine.
    Terminal,
    /// Pin waveforms captured from the running simulation.
    Waves,
    /// Named numeric channels over time — what a control loop is doing, and
    /// the tunables it exposes.
    Plot,
    /// Serial ports and probes currently attached.
    Devices,
    /// Where the target is stopped: the call stack and what the variables
    /// hold there.
    Debug,
    /// The chip's peripherals, as the target holds them right now.
    Registers,
    /// A control loop's attitude and its outputs, side by side.
    Flight,
}

impl DockTab {
    pub const ALL: [DockTab; 9] = [
        DockTab::Problems,
        DockTab::Output,
        DockTab::Terminal,
        DockTab::Waves,
        DockTab::Plot,
        DockTab::Debug,
        DockTab::Registers,
        DockTab::Flight,
        DockTab::Devices,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DockTab::Problems => "Problems",
            DockTab::Output => "Output",
            DockTab::Terminal => "Terminal",
            DockTab::Waves => "Waves",
            DockTab::Plot => "Plot",
            DockTab::Devices => "Devices",
            DockTab::Debug => "Debug",
            DockTab::Registers => "Registers",
            DockTab::Flight => "Flight",
        }
    }
}

/// A draggable boundary between two regions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Divider {
    /// Between the file tree and the editor. Horizontal drag.
    Tree,
    /// Between the panel and the dock. Vertical drag.
    Dock,
    /// Between the debugger's call stack and its variables. Horizontal drag —
    /// which side needs the room depends entirely on what you are looking at,
    /// so neither split can be the right one for everybody.
    DebugStack,
}

impl Divider {
    /// Bounds, in pixels. The lower one keeps a region usable rather than
    /// letting it be dragged to nothing — collapsing is what the toggle is for,
    /// and a two-pixel sidebar is not a smaller sidebar, it is a mistake.
    pub fn bounds(self) -> (f64, f64) {
        match self {
            Divider::Tree => (160.0, 440.0),
            Divider::Dock => (80.0, 600.0),
            Divider::DebugStack => (140.0, 900.0),
        }
    }

    fn storage_key(self) -> &'static str {
        match self {
            Divider::Tree => "rusty.layout.tree",
            Divider::Dock => "rusty.layout.dock",
            Divider::DebugStack => "rusty.layout.debug",
        }
    }
}

/// The `detach` query parameter, percent-decoded — the file this window
/// exists to edit, when it is that kind of window.
fn detached_path() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let raw = search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("detach="))?;
    // Decode %XX; the backend encodes everything outside [A-Za-z0-9._-/].
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok().filter(|p| !p.is_empty())
}

fn stored_size(divider: Divider, fallback: f64) -> f64 {
    let (min, max) = divider.bounds();
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(divider.storage_key()).ok().flatten())
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(fallback)
}

/// Editor font scale from last time. 1.0 is the 12.5px base; the wheel
/// clamps to the same range, so a hand-edited absurd value cannot produce
/// a 300px caret.
fn stored_zoom() -> f64 {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("rusty.editor.zoom").ok().flatten())
        .and_then(|v| v.parse::<f64>().ok())
        .map(|z| z.clamp(0.6, 2.4))
        .unwrap_or(1.0)
}

/// The interface scale from last time, clamped to the slider's own range.
fn stored_ui_zoom() -> f64 {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("rusty.ui.zoom").ok().flatten())
        .and_then(|v| v.parse::<f64>().ok())
        .map(|z| z.clamp(0.7, 1.6))
        .unwrap_or(1.0)
}

pub fn remember_ui_zoom(zoom: f64) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item("rusty.ui.zoom", &format!("{zoom:.2}"));
    }
}

pub fn remember_zoom(zoom: f64) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item("rusty.editor.zoom", &format!("{zoom:.2}"));
    }
}

pub fn remember_size(divider: Divider, value: f64) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item(divider.storage_key(), &value.to_string());
    }
}

const PROVIDER_KEY: &str = "rusty.assistant.provider";

/// Whatever this window still holds from before the profile became a file.
///
/// Read once and deleted, so an upgrade does not cost somebody their model
/// choice and the key does not linger to be misread later. It never held a
/// secret — the key itself lives in the OS credential store and is fetched by
/// the backend at the moment of the request, so it never enters this window.
pub fn carried_provider() -> Option<ProviderConfig> {
    let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten())?;
    let raw = storage.get_item(PROVIDER_KEY).ok().flatten()?;
    let _ = storage.remove_item(PROVIDER_KEY);
    serde_json::from_str(&raw).ok()
}

/// How many lines of device output to keep.
///
/// A monitor left running overnight would otherwise grow without bound and take
/// the window down with it. Ten thousand is far more than anyone scrolls back
/// through, and the oldest are the least interesting.
const LOG_CAPACITY: usize = 10_000;

/// Everything the window knows, grouped by what it is about.
///
/// It was 112 signals in one flat struct, which is a struct nobody can read
/// and a boundary nothing enforces: any component could reach the debugger's
/// breakpoints from inside the wizard. The groups below are the concerns the
/// rest of the frontend is already organised by, so a field now says where it
/// belongs — and the `find_`, `search_`, `ai_`, `sim_` prefixes are gone,
/// because they were the group's name written into every field for want of a
/// group to put them in.
///
/// Still `Copy`, and still one context: a group is a handful of `RwSignal`s,
/// which are themselves `Copy` handles into the reactive graph. Grouping costs
/// nothing at run time and buys a name at every call site.
#[derive(Clone, Copy)]
pub struct AppState {
    pub project: Project,
    pub device: Device,
    pub wizard: Wizard,
    pub ai: Assistant,
    pub editor: Editor,
    pub find: Find,
    pub search: Search,
    pub lsp: Lsp,
    pub sim: Sim,
    pub debug: Debug,
    pub term: Terminal,
    pub layout: Layout,
    pub dock: Dock,
    pub app: Workbench,
}

/// What is open: the detection, the Cargo analysis, and the panels that
/// read them.
#[derive(Clone, Copy)]
pub struct Project {
    /// The open project, once a folder has been chosen.
    pub detected: RwSignal<Option<EmbeddedProject>>,
    /// Cargo analysis. Absent when `cargo metadata` failed — which is normal
    /// for a misconfigured embedded project, and exactly when its diagnosis
    /// matters most, so the app opens anyway.
    pub workspace: RwSignal<Option<WorkspaceReport>>,
    pub toolchain: RwSignal<Option<ToolchainReport>>,
    pub chips: RwSignal<Vec<Chip>>,
    /// The part's pins and what the source names, for the editor's pin map.
    pub pins: RwSignal<Option<rusty_embed::PinReport>>,
    pub boards: RwSignal<Vec<Board>>,
    /// Binaries this project has built, newest first.
    ///
    /// Shared rather than owned by the memory panel: flashing and monitoring
    /// need the same list, and two panels each holding their own copy is how
    /// they end up disagreeing about which build is current.
    pub firmware: RwSignal<Vec<Firmware>>,
    /// Path of the build being worked with, if one has been chosen.
    pub selected_firmware: RwSignal<Option<String>>,
    pub memory: RwSignal<Option<MemoryReport>>,
    /// The feature selection being simulated, once a member has been picked.
    ///
    /// Held rather than derived from the switches because it is exactly what
    /// goes over the wire — a second representation would need converting on
    /// every toggle, and the two would disagree the first time a flag was added.
    pub feature_selection: RwSignal<Option<FeatureSelection>>,
    pub feature_rows: RwSignal<Vec<FeatureRow>>,
    pub feature_impact: RwSignal<Option<FeatureImpact>>,
    /// Board and chip files that would not parse, for the Catalogue screen.
    pub catalog_problems: RwSignal<Vec<rusty_embed::CatalogProblem>>,
    /// Direct dependencies against crates.io, when the Crates panel asked.
    pub crate_rows: RwSignal<Option<Vec<rusty_core::CrateRow>>>,
}

/// What is plugged in, and the command that would talk to it.
#[derive(Clone, Copy)]
pub struct Device {
    /// Devices currently attached. Shared by the Flash panel, the Monitor panel
    /// and the dock's Devices tab — three places that must never disagree about
    /// what is plugged in.
    pub ports: RwSignal<Vec<SerialPort>>,
    pub probes: RwSignal<Vec<Probe>>,
    /// How to reach the board, once a device has been chosen.
    pub transport: RwSignal<Option<Transport>>,
    /// The command that would run, shown before it does.
    pub plan: RwSignal<Option<CommandPlan>>,
}

/// Starting a new project.
#[derive(Clone, Copy)]
pub struct Wizard {
    /// The new-project wizard: what the generator offers, what has been chosen,
    /// what that choice commits the user to, and the command it produces.
    ///
    /// The explanation is the reason this panel exists, so it is state rather
    /// than something computed at the end — it updates while the choice is
    /// still being made, which is the only time it can change a decision.
    pub options: RwSignal<Vec<WizardOption>>,
    pub choice: RwSignal<Option<WizardChoice>>,
    pub explanations: RwSignal<Vec<Explanation>>,
    pub plan: RwSignal<Option<CommandPlan>>,
}

/// The conversation and the provider behind it. The key itself is never
/// here — it lives in the OS credential store and never enters the WebView.
#[derive(Clone, Copy)]
pub struct Assistant {
    /// The assistant.
    ///
    /// The transcript lives here, not in the backend: the backend takes a
    /// history and returns the updated one, so closing the panel cannot strand a
    /// conversation and nothing has to be cleaned up when it is reopened.
    pub config: RwSignal<Option<ProviderConfig>>,
    pub presets: RwSignal<Vec<Preset>>,
    pub tools: RwSignal<Vec<ToolDef>>,
    pub conversation: RwSignal<Vec<Message>>,
    /// Prose from the answer in flight, before it becomes a `Message`.
    pub pending: RwSignal<String>,
    /// Tools the current answer has called, in order, with whether each
    /// finished cleanly. Shown live: a model that goes quiet for ten seconds
    /// while resolving a dependency graph looks broken unless it says so.
    pub activity: RwSignal<Vec<ToolRun>>,
    pub streaming: RwSignal<bool>,
    /// Tokens the last answer cost, when the provider reported them. Surfaced
    /// because with bring-your-own keys every token is the user's money.
    pub usage: RwSignal<Option<(u32, u32)>>,
    /// Whether the assistant profile has a key in the OS credential store.
    /// The key itself never comes back here — only whether one exists.
    pub key_stored: RwSignal<bool>,
    /// The assistant drawer on the right, toggled from the title bar.
    pub open: RwSignal<bool>,
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
    /// Bumped on every keystroke; a re-highlight result is dropped unless the
    /// generation it was requested at is still current.
    pub pulse_gen: RwSignal<u64>,
    /// What the server said about the position under the mouse: path, the
    /// token's range, and the prose. The range is what keeps the card up while
    /// the pointer moves within the same token.
    pub hover: RwSignal<Option<(String, EditRange, String)>>,
    /// The completion popup, when one is up.
    pub completion: RwSignal<Option<CompletionPopup>>,
    /// The signature card: which file and line it hangs over, and what it says.
    pub signature: RwSignal<Option<(String, u32, rusty_lsp::SignatureInfo)>>,
    /// Quick fixes offered at the caret, when the user asked (Ctrl+.).
    pub actions: RwSignal<Option<(String, u32, Vec<rusty_lsp::CodeActionFix>)>>,
    /// Semantic colouring for the active document, as rust-analyzer sees it.
    /// Overlaid on the lexical highlight at render; empty while the index
    /// warms up, and the base colours simply show through.
    pub semantic: RwSignal<Option<(String, Vec<rusty_lsp::SemanticSpan>)>>,
    /// Every open editor, in strip order. The active one is [`Self::document`];
    /// the rest are parked in [`Self::parked`].
    pub tabs: RwSignal<Vec<String>>,
    /// Open editors that are not on screen, holding their unsaved drafts.
    pub parked: RwSignal<Vec<ParkedEditor>>,
    /// The active editor's undo/redo stacks.
    pub history: RwSignal<EditHistory>,
    /// Where the caret has been. Shared by Vim's jump keys and the menu.
    pub nav: RwSignal<NavHistory>,
    /// A rename waiting for its new name: where the symbol is, and what it
    /// is called now. `None` when no rename is being typed.
    pub rename: RwSignal<Option<(String, u32, u32, String)>>,
    /// Somewhere the editor should go — the result of goto-definition. Kept in
    /// state because the target file may still be opening when it is decided.
    pub reveal: RwSignal<Option<rusty_lsp::Location>>,
    /// Directories the user has opened. Collapsed by default, because a tree
    /// that unfolds everything is a list.
    pub expanded: RwSignal<Vec<String>>,
    /// Editor font scale (Ctrl+wheel). Multiplies FONT_SIZE and every pixel
    /// the editor derives from it.
    pub zoom: RwSignal<f64>,
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
}

/// A running simulation: the board, the plot, the trace, the tunables.
#[derive(Clone, Copy)]
pub struct Sim {
    /// What the firmware last printed to the `[rusty:disp]` channel.
    pub display: RwSignal<String>,
    /// The waveform capture for the current simulation run.
    pub trace: RwSignal<SimTrace>,
    /// Named numeric channels the firmware is printing, for the Plot panel.
    pub plot: RwSignal<Plot>,
    /// Which channels are drawn. Empty means all of them — a firmware with
    /// forty channels needs a filter, one with three does not.
    pub plot_shown: RwSignal<Vec<String>>,
    /// Tunables the firmware announced, newest value per name.
    pub params: RwSignal<Vec<rusty_embed::protocol::Param>>,
    /// The port rusty is holding open in both directions, if any. Distinct
    /// from a session merely running: a spawned `espflash monitor` is a
    /// session and cannot be written to, and a tunable that silently went
    /// nowhere would read as firmware ignoring it.
    pub link_port: RwSignal<Option<String>>,
    /// Pin levels for the board view, from whichever source [`sim_pin_source`]
    /// names.
    pub gpio: RwSignal<std::collections::HashMap<u8, bool>>,
    /// Duty cycles for the board view, from `[rusty:pwm]` — the analogue
    /// half of [`Self::gpio`], and what a motor turns on.
    ///
    /// **Absent is not zero.** A pin with no entry has never been reported,
    /// and a motor on it says so rather than showing a commanded stop; a pin
    /// mapped to `0.0` was told to stop. The two look the same on a dial and
    /// mean opposite things when a motor will not start.
    pub pwm: RwSignal<std::collections::HashMap<u8, f32>>,
    /// Sensors the firmware has declared it wants fed, newest wins by name.
    ///
    /// Declared rather than guessed, for the reason the tunables are: a panel
    /// that invented `gyro` and a range for it would one day inject 2000°/s
    /// into a loop written for 250. Empty means the firmware has asked for
    /// nothing, and the panel offers nothing.
    pub sensors: RwSignal<Vec<rusty_embed::SensorDef>>,
    /// The last sample the panel *sent* for each sensor — what its sliders
    /// sit at. Not what the firmware did with it, which only the firmware can
    /// say and only by printing something.
    pub sensor_values: RwSignal<std::collections::HashMap<String, Vec<f32>>>,
    /// Raw ADC counts the panel is holding on each pin.
    pub analog: RwSignal<std::collections::HashMap<u8, u16>>,
    /// The simulated aircraft, when the physical loop is closed.
    ///
    /// Injecting a rate proves the controller *responds*; it cannot show
    /// whether the loop settles, because the rate never changed in answer to
    /// the motors. This is the integrator that closes it: motor duties in,
    /// body rates out, fed back as the sample the firmware reads.
    pub plant: RwSignal<rusty_embed::Plant>,
    /// Whether that feedback is running. Off by default — a panel that
    /// started injecting on its own would make a firmware that reads its own
    /// IMU see two sources disagreeing.
    pub plant_closed: RwSignal<bool>,
    /// Guards the plant's timer against a stale one from a previous run, the
    /// way the editor's pulse does.
    pub plant_gen: RwSignal<u64>,
    /// Where those levels came from, as announced by the run that started.
    ///
    /// `Firmware` until a run says otherwise, because that is what every
    /// emulator did until rusty shipped one that keeps pin state — and a
    /// caption claiming register-level truth over a stock QEMU would send a
    /// user with a dark LED to check their wiring instead of their `println!`.
    pub pin_source: RwSignal<rusty_embed::PinSource>,
    /// The simulation plan for the open project, when the panel asked.
    pub plan: RwSignal<Option<rusty_embed::SimPlan>>,
    /// Tools whose one-click install failed — those cards reveal the manual
    /// instructions, which stay hidden while the button still deserves trust.
    pub install_failed: RwSignal<Vec<String>>,
}

/// The debug session, its breakpoints, and the chip's registers.
#[derive(Clone, Copy)]
pub struct Debug {
    /// The live debug session's state, or `None` when nothing is being
    /// debugged. Everything the gutter, the toolbar and the Debug panel
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
    /// What shell the terminal will start, from the backend.
    /// Which terminal session is current. Bumped by every open; a session's
    /// frame and completion callbacks compare before writing, so a replaced
    /// session's late "the shell is gone" cannot blank the one that
    /// replaced it — which looked like the terminal flickering for ever.
    pub epoch: RwSignal<u64>,
    pub info: RwSignal<Option<rusty_embed::ShellInfo>>,
    /// What the shell picker offers: the built-in plus every shell the
    /// backend actually found on this machine.
    pub choices: RwSignal<Vec<rusty_embed::ShellChoice>>,
}

/// Where the dividers sit and what is on screen. Sizes are the one thing
/// here that survives a reload, in localStorage.
#[derive(Clone, Copy)]
pub struct Layout {
    pub tree_width: RwSignal<f64>,
    pub dock_height: RwSignal<f64>,
    /// How much of the Debug tab the call stack gets.
    pub debug_width: RwSignal<f64>,
    /// Which divider is being dragged, if any. Held centrally so the window
    /// listeners are set up once rather than per handle.
    pub dragging: RwSignal<Option<Divider>>,
    /// Where the grab started: pointer coordinate and the size at that moment.
    /// Dragging moves relative to this — absolute window arithmetic has to
    /// know about every bar between the divider and the window edge, and got
    /// it wrong by exactly their sum.
    pub drag_from: RwSignal<(f64, f64)>,
    /// The bottom dock. Open by default: a build or flash that writes into a
    /// hidden drawer is a build whose failure the user finds out about later.
    pub dock_open: RwSignal<bool>,
    pub dock_tab: RwSignal<DockTab>,
    /// The global toolbar's content, registered by whatever the workspace
    /// currently shows. A slot rather than a switch: each panel mounts its
    /// own tools and clears them on unmount, so the row always answers to
    /// the work on screen and a new panel needs no central edit to join.
    pub toolbar: RwSignal<Option<Callback<(), AnyView>>>,
    pub panel: RwSignal<String>,
    /// Whole-interface scale, browser-zoom style. 1.0 is native.
    pub zoom: RwSignal<f64>,
}

/// The output dock: everything any tool has said, and how it is filtered.
#[derive(Clone, Copy)]
pub struct Dock {
    /// Everything spawned tools have printed, oldest first.
    ///
    /// Lives here rather than in the Flash panel so it survives switching
    /// panels — watching a device is something you do *while* reading the
    /// memory report, not instead of it.
    /// Device and tool output, each line tagged with the channel that was
    /// speaking — build, flash, simulate — so the Output panel can show one
    /// conversation at a time, the way VSCode's channel picker does.
    pub lines: RwSignal<Vec<(&'static str, LogLine)>>,
    /// Which channel new lines belong to. Sessions set it on start;
    /// `note_exit` drops it back to "app", where one-off notices live.
    pub source: RwSignal<&'static str>,
    /// The channel the Output panel shows; "all" shows everything.
    pub pick: RwSignal<&'static str>,
    /// Substring filter over shown lines. Space-separated terms must all
    /// match; a `!` prefix excludes instead.
    pub filter: RwSignal<String>,
    /// Whether the log view sticks to the bottom as lines arrive. Turned off
    /// automatically when the user scrolls up, which is the only way to read
    /// something in a stream that is still moving.
    pub follow: RwSignal<bool>,
}

/// The window itself — recents, shortcuts, updates, and whether something
/// is in flight.
#[derive(Clone, Copy)]
pub struct Workbench {
    /// Projects opened before, newest first — from the backend's
    /// workbench.toml, so the list survives restarts and belongs to the data
    /// directory rather than to this window.
    pub recents: RwSignal<Vec<String>>,
    /// Sidebar width and dock height, in pixels, remembered across sessions.
    ///
    /// A fixed-size panel is the first thing anyone tries to drag, and finding
    /// that they cannot is the moment a tool starts feeling rigid.
    /// `Some(path)` when this window was booted with `?detach=<path>` — a
    /// single file's editor, not the shell. Panels that show project-wide
    /// chrome (the tree, the tab strip) check it; so does everything that
    /// would write session state a one-file window has no business writing.
    pub detached: RwSignal<Option<String>>,
    /// Shortcut overrides from workbench.toml: action id → chord.
    pub keybinds: RwSignal<HashMap<String, String>>,
    /// The action id Settings is currently capturing a chord for. While set,
    /// the global shortcut handler stands down.
    pub capturing: RwSignal<Option<String>>,
    /// The last update check's answer. `None` while one is in flight.
    pub update: RwSignal<Option<rusty_embed::UpdateStatus>>,
    /// Whether a flash or monitor session is attached right now.
    ///
    /// One at a time by construction: the backend stops the previous session
    /// when a new one starts, because two readers on one serial port produce an
    /// access-denied that reads like a driver fault.
    pub session_running: RwSignal<bool>,
    /// Non-zero while any controller action is in flight. A counter rather than
    /// a flag so two overlapping loads cannot have the first to finish clear
    /// the indicator while the second is still running.
    pub in_flight: RwSignal<usize>,
    /// The last failure, shown until something succeeds or the user dismisses.
    pub error: RwSignal<Option<IpcError>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            project: Project {
                detected: RwSignal::new(None),
                workspace: RwSignal::new(None),
                toolchain: RwSignal::new(None),
                chips: RwSignal::new(Vec::new()),
                pins: RwSignal::new(None),
                boards: RwSignal::new(Vec::new()),
                firmware: RwSignal::new(Vec::new()),
                selected_firmware: RwSignal::new(None),
                memory: RwSignal::new(None),
                feature_selection: RwSignal::new(None),
                feature_rows: RwSignal::new(Vec::new()),
                feature_impact: RwSignal::new(None),
                catalog_problems: RwSignal::new(Vec::new()),
                crate_rows: RwSignal::new(None),
            },
            device: Device {
                ports: RwSignal::new(Vec::new()),
                probes: RwSignal::new(Vec::new()),
                transport: RwSignal::new(None),
                plan: RwSignal::new(None),
            },
            wizard: Wizard {
                options: RwSignal::new(Vec::new()),
                choice: RwSignal::new(None),
                explanations: RwSignal::new(Vec::new()),
                plan: RwSignal::new(None),
            },
            ai: Assistant {
                // Loaded from workbench.toml by the controller on boot;
                // `carried_provider` hands over anything this window still holds
                // from before it was a file.
                config: RwSignal::new(None),
                presets: RwSignal::new(Vec::new()),
                tools: RwSignal::new(Vec::new()),
                conversation: RwSignal::new(Vec::new()),
                pending: RwSignal::new(String::new()),
                activity: RwSignal::new(Vec::new()),
                streaming: RwSignal::new(false),
                usage: RwSignal::new(None),
                key_stored: RwSignal::new(false),
                open: RwSignal::new(false),
            },
            editor: Editor {
                tree: RwSignal::new(Vec::new()),
                document: RwSignal::new(None),
                draft: RwSignal::new(String::new()),
                highlighted: RwSignal::new(Vec::new()),
                echo_text: RwSignal::new(String::new()),
                pulse_gen: RwSignal::new(0),
                hover: RwSignal::new(None),
                completion: RwSignal::new(None),
                signature: RwSignal::new(None),
                actions: RwSignal::new(None),
                semantic: RwSignal::new(None),
                tabs: RwSignal::new(Vec::new()),
                parked: RwSignal::new(Vec::new()),
                history: RwSignal::new(EditHistory::default()),
                nav: RwSignal::new(NavHistory::default()),
                rename: RwSignal::new(None),
                reveal: RwSignal::new(None),
                expanded: RwSignal::new(Vec::new()),
                folds: RwSignal::new(rusty_edit::Folded::default()),
                stale: RwSignal::new(Vec::new()),
                watch_session: RwSignal::new(0),
                zoom: RwSignal::new(stored_zoom()),
                vim_on: RwSignal::new(false),
                vim: RwSignal::new(crate::vim::Vim::default()),
            },
            find: Find {
                open: RwSignal::new(false),
                replace_open: RwSignal::new(false),
                query: RwSignal::new(String::new()),
                case: RwSignal::new(false),
                replace: RwSignal::new(String::new()),
                index: RwSignal::new(0),
            },
            search: Search {
                query: RwSignal::new(String::new()),
                case: RwSignal::new(false),
                word: RwSignal::new(false),
                regex: RwSignal::new(false),
                include: RwSignal::new(String::new()),
                exclude: RwSignal::new(String::new()),
                results: RwSignal::new(None),
                generation: RwSignal::new(0),
            },
            lsp: Lsp {
                status: RwSignal::new(LspStatus::Off),
                session: RwSignal::new(0),
                diagnostics: RwSignal::new(HashMap::new()),
            },
            sim: Sim {
                display: RwSignal::new(String::new()),
                trace: RwSignal::new(SimTrace::default()),
                plot: RwSignal::new(Plot::default()),
                plot_shown: RwSignal::new(Vec::new()),
                params: RwSignal::new(Vec::new()),
                link_port: RwSignal::new(None),
                gpio: RwSignal::new(std::collections::HashMap::new()),
                pwm: RwSignal::new(std::collections::HashMap::new()),
                sensors: RwSignal::new(Vec::new()),
                sensor_values: RwSignal::new(std::collections::HashMap::new()),
                analog: RwSignal::new(std::collections::HashMap::new()),
                plant: RwSignal::new(rusty_embed::Plant::default()),
                plant_closed: RwSignal::new(false),
                plant_gen: RwSignal::new(0),
                pin_source: RwSignal::new(rusty_embed::PinSource::Firmware),
                plan: RwSignal::new(None),
                install_failed: RwSignal::new(Vec::new()),
            },
            debug: Debug {
                session: RwSignal::new(None),
                epoch: RwSignal::new(0),
                breakpoints: RwSignal::new(Vec::new()),
                registers: RwSignal::new(None),
                peripheral: RwSignal::new(None),
            },
            term: Terminal {
                screen: RwSignal::new(None),
                epoch: RwSignal::new(0),
                info: RwSignal::new(None),
                choices: RwSignal::new(Vec::new()),
            },
            layout: Layout {
                tree_width: RwSignal::new(stored_size(Divider::Tree, 240.0)),
                dock_height: RwSignal::new(stored_size(Divider::Dock, 196.0)),
                debug_width: RwSignal::new(stored_size(Divider::DebugStack, 420.0)),
                dragging: RwSignal::new(None),
                drag_from: RwSignal::new((0.0, 0.0)),
                dock_open: RwSignal::new(true),
                dock_tab: RwSignal::new(DockTab::Problems),
                toolbar: RwSignal::new(None),
                panel: RwSignal::new("files".to_string()),
                zoom: RwSignal::new(stored_ui_zoom()),
            },
            dock: Dock {
                lines: RwSignal::new(Vec::new()),
                source: RwSignal::new("app"),
                pick: RwSignal::new("all"),
                filter: RwSignal::new(String::new()),
                follow: RwSignal::new(true),
            },
            app: Workbench {
                recents: RwSignal::new(Vec::new()),
                detached: RwSignal::new(detached_path()),
                keybinds: RwSignal::new(HashMap::new()),
                capturing: RwSignal::new(None),
                update: RwSignal::new(None),
                session_running: RwSignal::new(false),
                in_flight: RwSignal::new(0),
                error: RwSignal::new(None),
            },
        }
    }

    /// Append device output, trimming the oldest once past capacity. The
    /// line lands in whichever channel is speaking right now.
    pub fn push_log(&self, line: LogLine) {
        let source = self.dock.source.get_untracked();
        self.dock.lines.update(|lines| {
            if lines.len() >= LOG_CAPACITY {
                // Drain a batch rather than one at a time: removing from the
                // front of a Vec is O(n), and doing that per line on a chatty
                // device would spend more time shuffling than rendering.
                lines.drain(..LOG_CAPACITY / 10);
            }
            lines.push((source, line));
        });
    }

    pub fn clear_log(&self) {
        self.dock.lines.update(Vec::clear);
        self.dock.follow.set(true);
    }

    /// Put it in context once, at the root, so panels registered elsewhere can
    /// reach it without being passed down a tree they are not part of.
    pub fn provide(self) {
        provide_context(self);
    }

    pub fn expect() -> Self {
        expect_context::<Self>()
    }

    pub fn is_busy(&self) -> bool {
        self.app.in_flight.get() > 0
    }

    pub fn has_project(&self) -> bool {
        self.project.detected.with(Option::is_some)
    }

    /// Every problem, from both sources, worst first.
    ///
    /// Derived in one place because the Overview panel, the dock, and the
    /// status bar all show it — and three separate derivations would be three
    /// chances for them to disagree about how many problems there are.
    pub fn problems(&self) -> Vec<Problem> {
        let mut all = Vec::new();
        self.project.detected.with(|p| {
            if let Some(p) = p {
                all.extend(p.problems.iter().cloned());
            }
        });
        self.project.toolchain.with(|t| {
            if let Some(t) = t {
                all.extend(t.problems.iter().cloned());
            }
        });
        all.sort_by_key(|p| match p.severity {
            Severity::Blocking => 0,
            Severity::Warning => 1,
            Severity::Info => 2,
        });
        all
    }

    pub fn blocking_count(&self) -> usize {
        self.problems()
            .iter()
            .filter(|p| p.severity == Severity::Blocking)
            .count()
    }

    /// The build being worked with.
    ///
    /// The user's choice when they have made one, otherwise the same default
    /// `rusty_embed::firmware::newest` applies on the backend: a binary built
    /// for the configured target beats a newer one built for something else,
    /// because flashing the wrong chip's image succeeds and then looks like
    /// broken hardware.
    ///
    /// A selection that no longer exists — the usual outcome of a `cargo clean`
    /// — falls back to the default rather than leaving the panel empty.
    pub fn current_firmware(&self) -> Option<Firmware> {
        let selected = self.project.selected_firmware.get();
        self.project.firmware.with(|all| {
            selected
                .and_then(|path| all.iter().find(|f| f.path == path))
                .or_else(|| all.iter().find(|f| f.matches_configured_target))
                .or_else(|| all.first())
                .cloned()
        })
    }

    /// Compiler errors and warnings, across every file the server has spoken
    /// about. The dock badge and the status bar both read this — one derivation
    /// so they cannot disagree.
    pub fn diag_counts(&self) -> (usize, usize) {
        self.lsp.diagnostics.with(|by_file| {
            let mut errors = 0;
            let mut warnings = 0;
            for diagnostic in by_file.values().flatten() {
                match diagnostic.severity {
                    rusty_lsp::DiagSeverity::Error => errors += 1,
                    rusty_lsp::DiagSeverity::Warning => warnings += 1,
                    _ => {}
                }
            }
            (errors, warnings)
        })
    }

    /// Bring a dock tab forward, opening the dock if it was collapsed.
    pub fn show_dock(&self, tab: DockTab) {
        self.layout.dock_tab.set(tab);
        self.layout.dock_open.set(true);
    }
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
