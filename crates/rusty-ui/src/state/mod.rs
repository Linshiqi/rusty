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

use rusty_i18n::t;

use crate::ipc::IpcError;

mod assistant;
mod editor;
mod git;
mod layout;
mod project;
mod services;
mod sim;
mod storage;
mod window;
mod workbench;

pub use assistant::*;
pub use editor::*;
pub use git::*;
pub use layout::*;
pub use project::*;
pub use services::*;
pub use sim::*;
pub use storage::*;
use window::{detached_path, query_param};
pub use workbench::*;

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
    /// The editor group this value addresses, and its find bar. The shell
    /// provides the state for the first group; the second group's subtree
    /// provides [`AppState::group`] of itself, so `AppState::expect()` inside
    /// it — and every controller called from there — works on that group
    /// without knowing there are two. See [`Group`].
    pub editor: Editor,
    pub setup: Setup,
    pub find: Find,
    pub group: Group,
    /// Both groups' handles, so any state value can reach the other group.
    pub groups: Groups,
    pub search: Search,
    pub lsp: Lsp,
    pub sim: Sim,
    pub debug: Debug,
    pub git: Git,
    pub term: Terminal,
    pub layout: Layout,
    pub dock: Dock,
    pub app: Workbench,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let first = Editor::fresh();
        let editors = [first, first.beside()];
        let finds = [Find::fresh(), Find::fresh()];
        Self {
            group: Group::First,
            groups: Groups { editors, finds },
            editor: editors[0],
            find: finds[0],
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
                disk: RwSignal::new(None),
                disk_busy: RwSignal::new(false),
                disk_auto_sweep: RwSignal::new(false),
                disk_idle_days: RwSignal::new(7),
            },
            device: Device {
                ports: RwSignal::new(Vec::new()),
                probes: RwSignal::new(Vec::new()),
                transport: RwSignal::new(None),
                plan: RwSignal::new(None),
                picker: RwSignal::new(false),
                pending: RwSignal::new(None),
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
                thinking: RwSignal::new(String::new()),
                cut_short: RwSignal::new(false),
                activity: RwSignal::new(Vec::new()),
                streaming: RwSignal::new(false),
                usage: RwSignal::new(None),
                key_stored: RwSignal::new(false),
                open: RwSignal::new(false),
            },
            setup: Setup {
                open: RwSignal::new(false),
                steps: RwSignal::new(Vec::new()),
                running: RwSignal::new(None),
                busy: RwSignal::new(None),
                installed: RwSignal::new(Vec::new()),
                failed: RwSignal::new(Vec::new()),
                checked: RwSignal::new(false),
                data_dir: RwSignal::new(None),
            },
            git: Git {
                history: RwSignal::new(None),
                branches: RwSignal::new(Vec::new()),
                rev: RwSignal::new(None),
                selected: RwSignal::new(None),
                detail: RwSignal::new(None),
                file: RwSignal::new(None),
                unavailable: RwSignal::new(None),
                not_a_repo: RwSignal::new(false),
                loaded: RwSignal::new(false),
                mode: RwSignal::new(GitMode::History),
                status: RwSignal::new(None),
                stashes: RwSignal::new(Vec::new()),
                diff: RwSignal::new(None),
                diff_for: RwSignal::new(None),
                message: RwSignal::new(String::new()),
                stash_note: RwSignal::new(String::new()),
                tags: RwSignal::new(Vec::new()),
                remotes: RwSignal::new(Vec::new()),
                prompt: RwSignal::new(None),
                query: RwSignal::new(String::new()),
                ref_filter: RwSignal::new(String::new()),
                folded: RwSignal::new(Vec::new()),
                reveal: RwSignal::new(None),
                detail_loading: RwSignal::new(false),
                diff_whole: RwSignal::new(false),
                stamp: StoredValue::new(None),
                root: StoredValue::new(None),
                cache: StoredValue::new(Vec::new()),
                gate: StoredValue::new(ReadGate::default()),
                split: RwSignal::new(stored_split()),
                limit: RwSignal::new(rusty_git::LIMIT),
                amend: RwSignal::new(false),
                menu: RwSignal::new(None),
                clone: RwSignal::new(None),
                images: RwSignal::new(None),
                detail_hidden: RwSignal::new(false),
                window_target: RwSignal::new(query_param("gitdiff")),
                identity: RwSignal::new(None),
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
                replacement: RwSignal::new(String::new()),
                replacing: RwSignal::new(false),
                outcome: RwSignal::new(None),
            },
            lsp: Lsp {
                status: RwSignal::new(LspStatus::Off),
                session: RwSignal::new(0),
                diagnostics: RwSignal::new(HashMap::new()),
                progress: RwSignal::new(None),
                health: RwSignal::new(None),
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
                readings: RwSignal::new(std::collections::HashMap::new()),
                adc: RwSignal::new(std::collections::HashMap::new()),
                i2c: RwSignal::new(Vec::new()),
                screens: RwSignal::new(HashMap::new()),
                rmt: RwSignal::new(HashMap::new()),
                spi: RwSignal::new(Vec::new()),
                plant: RwSignal::new(rusty_embed::Plant::default()),
                plant_closed: RwSignal::new(false),
                plant_gen: RwSignal::new(0),
                paused: RwSignal::new(false),
                pin_source: RwSignal::new(rusty_embed::PinSource::Firmware),
                plan: RwSignal::new(None),
                install_failed: RwSignal::new(Vec::new()),
                unsaved_sheet: StoredValue::new(None),
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
                tree_width: RwSignal::new(stored_size(Divider::Tree, Divider::Tree.default_size())),
                git_detail_height: RwSignal::new(stored_size(
                    Divider::GitDetail,
                    Divider::GitDetail.default_size(),
                )),
                git_message_height: RwSignal::new(stored_size(
                    Divider::GitMessage,
                    Divider::GitMessage.default_size(),
                )),
                git_files_width: RwSignal::new(stored_size(
                    Divider::GitFiles,
                    Divider::GitFiles.default_size(),
                )),
                git_changes_width: RwSignal::new(stored_size(
                    Divider::GitChanges,
                    Divider::GitChanges.default_size(),
                )),
                git_split: RwSignal::new(stored_size(
                    Divider::GitSplit,
                    Divider::GitSplit.default_size(),
                )),
                dock_height: RwSignal::new(stored_size(
                    Divider::Dock,
                    Divider::Dock.default_size(),
                )),
                debug_width: RwSignal::new(stored_size(
                    Divider::DebugStack,
                    Divider::DebugStack.default_size(),
                )),
                dragging: RwSignal::new(None),
                drag_from: RwSignal::new((0.0, 0.0, 1.0)),
                dock_open: RwSignal::new(true),
                dock_tab: RwSignal::new(DockTab::Problems),
                dock_tabs: RwSignal::new(DockTab::PINNED.to_vec()),
                panel: RwSignal::new("files".to_string()),
                zoom: RwSignal::new(stored_ui_zoom()),
                split: RwSignal::new(false),
                focus: RwSignal::new(Group::First),
                editor_split: RwSignal::new(stored_size(
                    Divider::EditorSplit,
                    Divider::EditorSplit.default_size(),
                )),
                assistant_width: RwSignal::new(stored_size(
                    Divider::Assistant,
                    Divider::Assistant.default_size(),
                )),
                git_sidebar_width: RwSignal::new(stored_size(
                    Divider::GitSidebar,
                    Divider::GitSidebar.default_size(),
                )),
                board_beside: RwSignal::new(false),
                board_width: RwSignal::new(stored_size(
                    Divider::Board,
                    Divider::Board.default_size(),
                )),
                tree_hidden: RwSignal::new(stored_tree_hidden()),
                quick_open: RwSignal::new(false),
                quick_seed: RwSignal::new(String::new()),
                quick_places: RwSignal::new(None),
                quick_symbols: RwSignal::new(None),
                switcher: RwSignal::new(None),
                calls: RwSignal::new(CallsView::Idle),
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
                update_open: RwSignal::new(false),
                update_stage: RwSignal::new(UpdateStage::Idle),
                update_progress: RwSignal::new(None),
                session_running: RwSignal::new(false),
                in_flight: RwSignal::new(0),
                error: RwSignal::new(None),
                activity: RwSignal::new(None),
                outcome: RwSignal::new(None),
                after_stop: RwSignal::new(None),
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

    /// Whether a project is open, tracked — for views and effects, which want
    /// to re-run when one is opened or closed.
    pub fn has_project(&self) -> bool {
        self.project.detected.with(Option::is_some)
    }

    /// The same question from a controller or an event handler, where there
    /// is no reactive owner to subscribe. Asking the tracked form there is
    /// harmless at run time, but Leptos warns about it on every boot — four
    /// times, from four controllers — and a warning that is always there is
    /// one that hides the day it means something.
    pub fn has_project_now(&self) -> bool {
        self.project.detected.with_untracked(Option::is_some)
    }

    /// The chip whose playground is open, tracked — `None` for any other
    /// project. The window lays a playground out code beside board, and
    /// offers what only a playground has: its example back, the other
    /// chip's, and keeping it as a project of its own.
    pub fn playground(&self) -> Option<String> {
        self.project
            .detected
            .with(|p| p.as_ref().and_then(|p| p.playground.clone()))
    }

    /// The same, from a controller.
    pub fn playground_now(&self) -> Option<String> {
        self.project
            .detected
            .with_untracked(|p| p.as_ref().and_then(|p| p.playground.clone()))
    }

    /// The path of the document on screen, tracked — for views.
    pub fn active_path(&self) -> Option<String> {
        self.editor
            .document
            .with(|d| d.as_ref().map(|d| d.path.clone()))
    }

    /// The same, untracked — for controllers and handlers. The expression
    /// behind both was written out twenty-three times before this.
    pub fn active_path_now(&self) -> Option<String> {
        self.editor
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.path.clone()))
    }

    /// Is this file in no crate's module tree — the state where
    /// rust-analyzer parses it and answers nothing else?
    ///
    /// Two sources and one answer, so the tree and the tab strip cannot dim
    /// different files: rust-analyzer's own `unlinked-file`, which exists
    /// only for a file the client has opened, and rusty's reading of the
    /// `mod` declarations (`rusty_edit::modules`), which is what a file
    /// nobody has opened yet is judged by.
    pub fn is_unlinked(&self, path: &str) -> bool {
        // The scan wins wherever it has an opinion, because it has just read
        // the files. rust-analyzer's diagnostic is right when it arrives and
        // then *stays* until the server re-analyses that file — so a `mod`
        // line added to the parent module left the child dimmed with nothing
        // on either side able to clear it. Its verdict answers only where the
        // scan refused to claim anything at all.
        let claimed = self
            .editor
            .unlinked
            .with(|claim| claim.as_ref().map(|list| list.iter().any(|p| p == path)));
        claimed.unwrap_or_else(|| {
            self.lsp.diagnostics.with(|by_file| {
                by_file.get(path).is_some_and(|items| {
                    items
                        .iter()
                        .any(|d| d.code.as_deref() == Some("unlinked-file"))
                })
            })
        })
    }

    /// Whether this file has edits the disk has not seen.
    ///
    /// The active editor keeps its draft in `editor.draft`; every other open
    /// one keeps its own inside `parked`. A read-only document can never be
    /// dirty — its draft is the disk's text by construction, and treating it
    /// as unsaved would put a dot on every dependency you glanced at.
    pub fn is_dirty(&self, path: &str) -> bool {
        // Both groups, whichever this value addresses: a file is open in one
        // of them at most, and a draft anywhere is what protects the disk.
        self.groups.editors.iter().any(|editor| {
            let active = editor.document.with(|doc| {
                doc.as_ref().is_some_and(|doc| {
                    doc.path == path
                        && !doc.read_only
                        && editor.draft.with(|draft| draft != &doc.text)
                })
            });
            active
                || editor.parked.with(|parked| {
                    parked
                        .iter()
                        .find(|editor| editor.document.path == path)
                        .is_some_and(|editor| {
                            !editor.document.read_only && editor.draft != editor.document.text
                        })
                })
        })
    }

    /// Every open editor with unsaved changes, project-relative.
    ///
    /// What a project-wide replace must not write over: the draft is still on
    /// screen and still looks authoritative, so replacing underneath it means
    /// the next Ctrl+S quietly puts the old text back.
    pub fn dirty_paths(&self) -> Vec<String> {
        self.groups
            .editors
            .iter()
            .flat_map(|editor| editor.tabs.get_untracked())
            .filter(|path| self.is_dirty(path))
            .collect()
    }

    /// This state addressing `which` group: `editor` and `find` are that
    /// group's, everything else is shared. `AppState` is a bundle of `Copy`
    /// handles, so this costs nothing — and it is what lets every controller
    /// written for one editor serve two. The second group's subtree provides
    /// this as its context; a controller reached from outside either group
    /// asks [`AppState::focused`] which one the user meant.
    pub fn group(self, which: Group) -> AppState {
        AppState {
            editor: self.groups.editors[which.index()],
            find: self.groups.finds[which.index()],
            group: which,
            ..self
        }
    }

    /// The group beside this one.
    pub fn other(self) -> AppState {
        self.group(self.group.other())
    }

    /// The group the user last worked in — where a file opened from the
    /// tree, the finder or a search hit lands. Untracked: a controller's
    /// question, asked at the moment of the click.
    pub fn focused(self) -> AppState {
        self.group(self.layout.focus.get_untracked())
    }

    /// Every group on screen, the first first. Untracked, for the watcher and
    /// the language server, which have to tell both about the disk.
    pub fn open_groups(self) -> Vec<AppState> {
        if self.layout.split.get_untracked() {
            vec![self.group(Group::First), self.group(Group::Second)]
        } else {
            vec![self.group(Group::First)]
        }
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
        self.project
            .firmware
            .with(|all| pick_firmware(all, selected.as_deref()))
    }

    /// [`Self::current_firmware`] for a controller, which has no reactive
    /// owner to subscribe (see `has_project_now`).
    pub fn current_firmware_untracked(&self) -> Option<Firmware> {
        let selected = self.project.selected_firmware.get_untracked();
        self.project
            .firmware
            .with_untracked(|all| pick_firmware(all, selected.as_deref()))
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

    /// Bring a dock tab forward, opening the dock if it was collapsed — and
    /// putting it on the strip if it was not, which is how most tabs arrive:
    /// a debug run brings Debug, a serial link brings Plot, a flash brings
    /// Output.
    pub fn show_dock(&self, tab: DockTab) {
        self.reveal_tab(tab);
        self.layout.dock_tab.set(tab);
        self.layout.dock_open.set(true);
    }

    /// Put a tab on the strip without bringing it forward or opening the
    /// dock. For the firmware's side of things: its first telemetry sample
    /// means there is a plot, and the strip should say so — but the user is
    /// reading Output, and a panel that switched under them would be the
    /// banner that reflowed the workspace, again. Untracked and free when the
    /// tab is already there, because the protocol reader calls this per line.
    pub fn reveal_tab(&self, tab: DockTab) {
        let tabs = self.layout.dock_tabs;
        if tabs.with_untracked(|strip| strip.contains(&tab)) {
            return;
        }
        tabs.update(|strip| *strip = DockTab::strip_with(strip, tab));
    }

    /// Take a tab off the strip. A pinned one stays; hiding the one in front
    /// fronts its left-hand neighbour, as closing an editor tab does.
    pub fn hide_tab(&self, tab: DockTab) {
        let fronted = self.layout.dock_tab.get_untracked();
        let next = self
            .layout
            .dock_tabs
            .with_untracked(|strip| DockTab::strip_without(strip, tab, fronted));
        if let Some((strip, front)) = next {
            self.layout.dock_tabs.set(strip);
            if front != fronted {
                self.layout.dock_tab.set(front);
            }
        }
    }

    /// Back to the three the strip starts with — Reset layout's share of the
    /// dock, alongside the dividers.
    pub fn reset_strip(&self) {
        self.layout.dock_tabs.set(DockTab::PINNED.to_vec());
        if !self.layout.dock_tab.get_untracked().pinned() {
            self.layout.dock_tab.set(DockTab::Problems);
        }
    }
}

#[cfg(test)]
mod dock_strip_tests {
    use super::DockTab;

    const PINNED: [DockTab; 3] = DockTab::PINNED;

    /// A tab revealed mid-session sits where the full strip would put it,
    /// however many were revealed before it and in whatever order; revealing
    /// it again is not a second copy.
    #[test]
    fn a_revealed_tab_takes_its_place_in_the_order_and_only_once() {
        let strip = DockTab::strip_with(&PINNED, DockTab::Flight);
        let strip = DockTab::strip_with(&strip, DockTab::Waves);
        assert_eq!(
            strip,
            [
                DockTab::Problems,
                DockTab::Output,
                DockTab::Terminal,
                DockTab::Waves,
                DockTab::Flight,
            ]
        );
        assert_eq!(DockTab::strip_with(&strip, DockTab::Waves), strip);
    }

    /// Hiding the tab in front moves the front to its left-hand neighbour —
    /// the pinned three guarantee there is one — and hiding any other tab
    /// leaves the front alone.
    #[test]
    fn hiding_the_fronted_tab_fronts_its_left_neighbour() {
        let strip =
            DockTab::strip_with(&DockTab::strip_with(&PINNED, DockTab::Waves), DockTab::Plot);

        let (rest, front) = DockTab::strip_without(&strip, DockTab::Plot, DockTab::Plot).unwrap();
        assert_eq!(front, DockTab::Waves);
        assert!(!rest.contains(&DockTab::Plot));

        let (rest, front) =
            DockTab::strip_without(&strip, DockTab::Waves, DockTab::Terminal).unwrap();
        assert_eq!(front, DockTab::Terminal);
        assert_eq!(rest, DockTab::strip_with(&PINNED, DockTab::Plot));

        let (_, front) = DockTab::strip_without(&strip, DockTab::Waves, DockTab::Waves).unwrap();
        assert_eq!(
            front,
            DockTab::Terminal,
            "the last pinned tab is the neighbour"
        );
    }

    /// The pinned three cannot go, and a tab that is not on the strip is
    /// nothing to do rather than a change.
    #[test]
    fn the_pinned_three_stay_and_an_absent_tab_is_a_no_op() {
        for tab in PINNED {
            assert!(
                DockTab::strip_without(&PINNED, tab, tab).is_none(),
                "{tab:?} must not be hideable"
            );
        }
        assert!(DockTab::strip_without(&PINNED, DockTab::Flight, DockTab::Problems).is_none());
        // And every pinned tab is one the full order knows, so a strip built
        // through `strip_with` always carries all three.
        for tab in PINNED {
            assert!(DockTab::ALL.contains(&tab));
        }
    }
}
