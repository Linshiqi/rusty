//! Shared signals.
//!
//! Holds state and pure operations on it — no IPC, no side effects. Anything
//! that has to talk to the backend belongs in `controller`, so that a panel
//! reading state cannot accidentally trigger a fetch.

use leptos::prelude::*;

use rusty_ai::{Message, Preset, ProviderConfig, ToolDef};
use rusty_edit::{Document, Entry, Line};
use rusty_lsp::{EditRange, FileDiagnostic};
use rusty_term::Screen as TermScreen;
use rusty_core::{FeatureImpact, FeatureRow, FeatureSelection, WorkspaceReport};
use rusty_embed::{
    Board, Chip, CommandPlan, EmbeddedProject, Explanation, Firmware, LogLine, MemoryReport, Probe,
    Problem, SerialPort, Severity, ToolchainReport, Transport, WizardChoice, WizardOption,
};

use std::collections::HashMap;

use crate::ipc::IpcError;

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
    /// Serial ports and probes currently attached.
    Devices,
}

impl DockTab {
    pub const ALL: [DockTab; 4] =
        [DockTab::Problems, DockTab::Output, DockTab::Terminal, DockTab::Devices];

    pub fn label(self) -> &'static str {
        match self {
            DockTab::Problems => "Problems",
            DockTab::Output => "Output",
            DockTab::Terminal => "Terminal",
            DockTab::Devices => "Devices",
        }
    }
}

/// A draggable boundary between two regions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Divider {
    /// Between the sidebar and the panel. Horizontal drag.
    Sidebar,
    /// Between the panel and the dock. Vertical drag.
    Dock,
}

impl Divider {
    /// Bounds, in pixels. The lower one keeps a region usable rather than
    /// letting it be dragged to nothing — collapsing is what the toggle is for,
    /// and a two-pixel sidebar is not a smaller sidebar, it is a mistake.
    pub fn bounds(self) -> (f64, f64) {
        match self {
            Divider::Sidebar => (150.0, 380.0),
            Divider::Dock => (80.0, 600.0),
        }
    }

    fn storage_key(self) -> &'static str {
        match self {
            Divider::Sidebar => "rusty.layout.sidebar",
            Divider::Dock => "rusty.layout.dock",
        }
    }
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

pub fn remember_size(divider: Divider, value: f64) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item(divider.storage_key(), &value.to_string());
    }
}

const PROVIDER_KEY: &str = "rusty.assistant.provider";

/// The provider profile from last time.
///
/// Safe to keep in the browser's storage because it holds no secret — the key
/// itself lives in the OS credential store and is fetched by the backend at the
/// moment of the request, so it never enters this window at all.
fn stored_provider() -> Option<ProviderConfig> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(PROVIDER_KEY).ok().flatten())
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

pub fn remember_provider(config: &ProviderConfig) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
        && let Ok(raw) = serde_json::to_string(config)
    {
        let _ = storage.set_item(PROVIDER_KEY, &raw);
    }
}

/// How many lines of device output to keep.
///
/// A monitor left running overnight would otherwise grow without bound and take
/// the window down with it. Ten thousand is far more than anyone scrolls back
/// through, and the oldest are the least interesting.
const LOG_CAPACITY: usize = 10_000;

/// Everything the panels read. `Copy`, because Leptos signals are handles.
#[derive(Clone, Copy)]
pub struct AppState {
    /// The open project, once a folder has been chosen.
    pub project: RwSignal<Option<EmbeddedProject>>,
    /// Cargo analysis. Absent when `cargo metadata` failed — which is normal
    /// for a misconfigured embedded project, and exactly when its diagnosis
    /// matters most, so the app opens anyway.
    pub workspace: RwSignal<Option<WorkspaceReport>>,
    pub toolchain: RwSignal<Option<ToolchainReport>>,
    pub chips: RwSignal<Vec<Chip>>,
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

    /// Devices currently attached. Shared by the Flash panel, the Monitor panel
    /// and the dock's Devices tab — three places that must never disagree about
    /// what is plugged in.
    pub ports: RwSignal<Vec<SerialPort>>,
    pub probes: RwSignal<Vec<Probe>>,
    /// How to reach the board, once a device has been chosen.
    pub transport: RwSignal<Option<Transport>>,
    /// The command that would run, shown before it does.
    pub plan: RwSignal<Option<CommandPlan>>,
    /// The new-project wizard: what the generator offers, what has been chosen,
    /// what that choice commits the user to, and the command it produces.
    ///
    /// The explanation is the reason this panel exists, so it is state rather
    /// than something computed at the end — it updates while the choice is
    /// still being made, which is the only time it can change a decision.
    pub wizard_options: RwSignal<Vec<WizardOption>>,
    pub wizard_choice: RwSignal<Option<WizardChoice>>,
    pub wizard_explanations: RwSignal<Vec<Explanation>>,
    pub wizard_plan: RwSignal<Option<CommandPlan>>,

    /// The assistant.
    ///
    /// The transcript lives here, not in the backend: the backend takes a
    /// history and returns the updated one, so closing the panel cannot strand a
    /// conversation and nothing has to be cleaned up when it is reopened.
    pub ai_config: RwSignal<Option<ProviderConfig>>,
    pub ai_presets: RwSignal<Vec<Preset>>,
    pub ai_tools: RwSignal<Vec<ToolDef>>,
    pub conversation: RwSignal<Vec<Message>>,
    /// Prose from the answer in flight, before it becomes a `Message`.
    pub ai_pending: RwSignal<String>,
    /// Tools the current answer has called, in order, with whether each
    /// finished cleanly. Shown live: a model that goes quiet for ten seconds
    /// while resolving a dependency graph looks broken unless it says so.
    pub ai_activity: RwSignal<Vec<ToolRun>>,
    pub ai_streaming: RwSignal<bool>,
    /// Tokens the last answer cost, when the provider reported them. Surfaced
    /// because with bring-your-own keys every token is the user's money.
    pub ai_usage: RwSignal<Option<(u32, u32)>>,

    /// The project's files, and the one being looked at.
    ///
    /// The draft is held apart from the document so an unsaved edit survives
    /// re-highlighting: the highlighted lines come from the backend and are
    /// replaced wholesale, and folding the text into them would lose whatever
    /// had been typed since.
    pub file_tree: RwSignal<Vec<Entry>>,
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

    /// What the compiler and rust-analyzer think is wrong, by file.
    pub diagnostics: RwSignal<HashMap<String, Vec<FileDiagnostic>>>,
    /// What the server said about the position under the mouse: path, the
    /// token's range, and the prose. The range is what keeps the card up while
    /// the pointer moves within the same token.
    pub hover: RwSignal<Option<(String, EditRange, String)>>,
    /// The completion popup, when one is up.
    pub completion: RwSignal<Option<CompletionPopup>>,
    /// The signature card: which file and line it hangs over, and what it says.
    pub signature: RwSignal<Option<(String, u32, rusty_lsp::SignatureInfo)>>,
    /// The in-file find bar. Survives tab switches, as every editor's does;
    /// resets with the project.
    pub find_open: RwSignal<bool>,
    pub find_replace_open: RwSignal<bool>,
    pub find_query: RwSignal<String>,
    pub find_case: RwSignal<bool>,
    pub find_replace: RwSignal<String>,
    /// Which match is current, clamped to the match count at use.
    pub find_index: RwSignal<usize>,
    /// Direct dependencies against crates.io, when the Crates panel asked.
    pub crate_rows: RwSignal<Option<Vec<rusty_core::CrateRow>>>,
    /// The assistant drawer on the right, toggled from the title bar.
    pub assistant_open: RwSignal<bool>,
    /// Pin levels the running firmware has reported, for the board view.
    pub sim_gpio: RwSignal<std::collections::HashMap<u8, bool>>,
    /// The simulation plan for the open project, when the panel asked.
    pub sim_plan: RwSignal<Option<rusty_embed::SimPlan>>,
    /// Tools whose one-click install failed — those cards reveal the manual
    /// instructions, which stay hidden while the button still deserves trust.
    pub sim_install_failed: RwSignal<Vec<String>>,
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
    /// Project search. Kept here rather than in the panel so the results
    /// survive switching away and back.
    pub search_query: RwSignal<String>,
    pub search_case: RwSignal<bool>,
    pub search_word: RwSignal<bool>,
    pub search_regex: RwSignal<bool>,
    /// `*.rs, src/**` — gitignore-style globs, as the boxes in the panel.
    pub search_include: RwSignal<String>,
    pub search_exclude: RwSignal<String>,
    /// Show dot-entries in the file tree. Off by default; `.git` never shows.
    pub show_hidden: RwSignal<bool>,
    pub search_results: RwSignal<Option<rusty_edit::SearchResults>>,
    /// Which search is current; a stale reply is dropped, and the debounce
    /// timer checks it before firing.
    pub search_gen: RwSignal<u64>,
    /// Somewhere the editor should go — the result of goto-definition. Kept in
    /// state because the target file may still be opening when it is decided.
    pub reveal: RwSignal<Option<rusty_lsp::Location>>,
    pub lsp_status: RwSignal<LspStatus>,
    /// Which start_lsp call owns the event channel; stale channels' events are
    /// dropped rather than fighting the new server over the status signal.
    pub lsp_session: RwSignal<u64>,
    /// Directories the user has opened. Collapsed by default, because a tree
    /// that unfolds everything is a list.
    pub expanded: RwSignal<Vec<String>>,

    /// The terminal's latest frame, when a shell is open.
    ///
    /// Whole screens rather than an append-only log: a pty is a screen, and
    /// programs that redraw — every progress bar, every prompt redraw after a
    /// backspace — overwrite what is there rather than adding to it.
    pub terminal: RwSignal<Option<TermScreen>>,

    /// Whether a flash or monitor session is attached right now.
    ///
    /// One at a time by construction: the backend stops the previous session
    /// when a new one starts, because two readers on one serial port produce an
    /// access-denied that reads like a driver fault.
    pub session_running: RwSignal<bool>,

    pub active_panel: RwSignal<String>,
    /// Projects opened before, newest first — from the backend's
    /// workbench.toml, so the list survives restarts and belongs to the data
    /// directory rather than to this window.
    pub recents: RwSignal<Vec<String>>,

    /// Sidebar width and dock height, in pixels, remembered across sessions.
    ///
    /// A fixed-size panel is the first thing anyone tries to drag, and finding
    /// that they cannot is the moment a tool starts feeling rigid.
    pub sidebar_width: RwSignal<f64>,
    pub dock_height: RwSignal<f64>,
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
    /// Everything spawned tools have printed, oldest first.
    ///
    /// Lives here rather than in the Flash panel so it survives switching
    /// panels — watching a device is something you do *while* reading the
    /// memory report, not instead of it.
    pub log: RwSignal<Vec<LogLine>>,
    /// Whether the log view sticks to the bottom as lines arrive. Turned off
    /// automatically when the user scrolls up, which is the only way to read
    /// something in a stream that is still moving.
    pub log_follow: RwSignal<bool>,
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
            project: RwSignal::new(None),
            workspace: RwSignal::new(None),
            toolchain: RwSignal::new(None),
            chips: RwSignal::new(Vec::new()),
            boards: RwSignal::new(Vec::new()),
            firmware: RwSignal::new(Vec::new()),
            selected_firmware: RwSignal::new(None),
            memory: RwSignal::new(None),
            feature_selection: RwSignal::new(None),
            feature_rows: RwSignal::new(Vec::new()),
            feature_impact: RwSignal::new(None),
            ports: RwSignal::new(Vec::new()),
            probes: RwSignal::new(Vec::new()),
            transport: RwSignal::new(None),
            plan: RwSignal::new(None),
            wizard_options: RwSignal::new(Vec::new()),
            wizard_choice: RwSignal::new(None),
            wizard_explanations: RwSignal::new(Vec::new()),
            wizard_plan: RwSignal::new(None),
            ai_config: RwSignal::new(stored_provider()),
            ai_presets: RwSignal::new(Vec::new()),
            ai_tools: RwSignal::new(Vec::new()),
            conversation: RwSignal::new(Vec::new()),
            ai_pending: RwSignal::new(String::new()),
            ai_activity: RwSignal::new(Vec::new()),
            ai_streaming: RwSignal::new(false),
            ai_usage: RwSignal::new(None),
            file_tree: RwSignal::new(Vec::new()),
            document: RwSignal::new(None),
            draft: RwSignal::new(String::new()),
            highlighted: RwSignal::new(Vec::new()),
            echo_text: RwSignal::new(String::new()),
            pulse_gen: RwSignal::new(0),
            diagnostics: RwSignal::new(HashMap::new()),
            hover: RwSignal::new(None),
            completion: RwSignal::new(None),
            signature: RwSignal::new(None),
            semantic: RwSignal::new(None),
            actions: RwSignal::new(None),
            crate_rows: RwSignal::new(None),
            assistant_open: RwSignal::new(false),
            sim_gpio: RwSignal::new(std::collections::HashMap::new()),
            sim_plan: RwSignal::new(None),
            sim_install_failed: RwSignal::new(Vec::new()),
            find_open: RwSignal::new(false),
            find_replace_open: RwSignal::new(false),
            find_query: RwSignal::new(String::new()),
            find_case: RwSignal::new(false),
            find_replace: RwSignal::new(String::new()),
            find_index: RwSignal::new(0),
            tabs: RwSignal::new(Vec::new()),
            parked: RwSignal::new(Vec::new()),
            history: RwSignal::new(EditHistory::default()),
            search_query: RwSignal::new(String::new()),
            search_case: RwSignal::new(false),
            search_word: RwSignal::new(false),
            search_regex: RwSignal::new(false),
            search_include: RwSignal::new(String::new()),
            search_exclude: RwSignal::new(String::new()),
            show_hidden: RwSignal::new(false),
            search_results: RwSignal::new(None),
            search_gen: RwSignal::new(0),
            reveal: RwSignal::new(None),
            lsp_status: RwSignal::new(LspStatus::Off),
            lsp_session: RwSignal::new(0),
            expanded: RwSignal::new(Vec::new()),
            terminal: RwSignal::new(None),
            session_running: RwSignal::new(false),
            active_panel: RwSignal::new("overview".to_string()),
            recents: RwSignal::new(Vec::new()),
            sidebar_width: RwSignal::new(stored_size(Divider::Sidebar, 188.0)),
            dock_height: RwSignal::new(stored_size(Divider::Dock, 196.0)),
            dragging: RwSignal::new(None),
            drag_from: RwSignal::new((0.0, 0.0)),
            dock_open: RwSignal::new(true),
            dock_tab: RwSignal::new(DockTab::Problems),
            log: RwSignal::new(Vec::new()),
            log_follow: RwSignal::new(true),
            in_flight: RwSignal::new(0),
            error: RwSignal::new(None),
        }
    }

    /// Append device output, trimming the oldest once past capacity.
    pub fn push_log(&self, line: LogLine) {
        self.log.update(|lines| {
            if lines.len() >= LOG_CAPACITY {
                // Drain a batch rather than one at a time: removing from the
                // front of a Vec is O(n), and doing that per line on a chatty
                // device would spend more time shuffling than rendering.
                lines.drain(..LOG_CAPACITY / 10);
            }
            lines.push(line);
        });
    }

    pub fn clear_log(&self) {
        self.log.update(Vec::clear);
        self.log_follow.set(true);
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
        self.in_flight.get() > 0
    }

    pub fn has_project(&self) -> bool {
        self.project.with(Option::is_some)
    }

    /// Every problem, from both sources, worst first.
    ///
    /// Derived in one place because the Overview panel, the dock, and the
    /// status bar all show it — and three separate derivations would be three
    /// chances for them to disagree about how many problems there are.
    pub fn problems(&self) -> Vec<Problem> {
        let mut all = Vec::new();
        self.project.with(|p| {
            if let Some(p) = p {
                all.extend(p.problems.iter().cloned());
            }
        });
        self.toolchain.with(|t| {
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
        let selected = self.selected_firmware.get();
        self.firmware.with(|all| {
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
        self.diagnostics.with(|by_file| {
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
        self.dock_tab.set(tab);
        self.dock_open.set(true);
    }
}
