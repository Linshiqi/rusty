//! The shell's layout: the dock's tabs, every divider, and which panels are
//! showing.

use super::*;

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
    /// The signal lab: what the sheet's signals play, and what a filter in
    /// the firmware made of them — in time, as a spectrum, as a measured
    /// response, and against a design.
    Signals,
    /// Where the target is stopped: the call stack and what the variables
    /// hold there.
    Debug,
    /// The chip's peripherals, as the target holds them right now.
    Registers,
    /// A control loop's attitude and its outputs, side by side.
    Flight,
    /// Who calls a function and what it calls, a level at a time.
    Calls,
}

impl DockTab {
    /// Every tab there is, in the order the strip draws them. The View menu
    /// and the palette list these; the strip itself carries a subset
    /// ([`Layout::dock_tabs`]).
    pub const ALL: [DockTab; 10] = [
        DockTab::Problems,
        DockTab::Output,
        DockTab::Terminal,
        DockTab::Calls,
        DockTab::Waves,
        DockTab::Plot,
        DockTab::Signals,
        DockTab::Debug,
        DockTab::Registers,
        DockTab::Flight,
    ];

    /// The three every IDE's panel opens with, and the only ones that cannot
    /// be hidden: what is wrong, what the tools said, and a shell. The other
    /// six are on the strip only while something has put them there.
    pub const PINNED: [DockTab; 3] = [DockTab::Problems, DockTab::Output, DockTab::Terminal];

    pub fn pinned(self) -> bool {
        Self::PINNED.contains(&self)
    }

    /// `strip` with `tab` on it, in [`Self::ALL`]'s order — a tab that
    /// appears mid-session lands where it always sits, not at the end. A
    /// tab already there changes nothing.
    pub fn strip_with(strip: &[DockTab], tab: DockTab) -> Vec<DockTab> {
        Self::ALL
            .into_iter()
            .filter(|t| *t == tab || strip.contains(t))
            .collect()
    }

    /// `strip` without `tab`, and which tab is in front afterwards: `fronted`
    /// unless it was the one that went, then its left-hand neighbour, as
    /// closing an editor tab does. `None` when there is nothing to do — a
    /// pinned tab, or one that was not on the strip.
    pub fn strip_without(
        strip: &[DockTab],
        tab: DockTab,
        fronted: DockTab,
    ) -> Option<(Vec<DockTab>, DockTab)> {
        if tab.pinned() {
            return None;
        }
        let at = strip.iter().position(|t| *t == tab)?;
        let rest = strip.iter().copied().filter(|t| *t != tab).collect();
        let front = if fronted == tab {
            strip[..at].last().copied().unwrap_or(DockTab::Problems)
        } else {
            fronted
        };
        Some((rest, front))
    }

    pub fn label(self) -> String {
        match self {
            DockTab::Problems => t!("dock.tab.problems"),
            DockTab::Output => t!("dock.tab.output"),
            DockTab::Terminal => t!("dock.tab.terminal"),
            DockTab::Waves => t!("dock.tab.waves"),
            DockTab::Plot => t!("dock.tab.plot"),
            DockTab::Signals => t!("dock.tab.signals"),
            DockTab::Debug => t!("dock.tab.debug"),
            DockTab::Registers => t!("dock.tab.registers"),
            DockTab::Flight => t!("dock.tab.flight"),
            DockTab::Calls => t!("dock.tab.calls"),
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
    /// Between the Git log and the commit opened below it. Vertical drag,
    /// anchored to the bottom like the dock: up grows the commit.
    GitDetail,
    /// The cap on the opened commit's message. Vertical drag anchored to the
    /// top, so dragging down shows more of an essay-length message; a short
    /// one never fills the cap, which is why it is a cap and not a height.
    GitMessage,
    /// Between the opened commit's files and the chosen file's patch.
    GitFiles,
    /// Between the Changes view's file column and its diff.
    GitChanges,
    /// The line between old and new in a side-by-side diff. Sized in
    /// permille of the text width rather than in pixels, because half is
    /// the right default at every pane width and pixels cannot say "half";
    /// the handle converts its travel with the width it measures on grab.
    GitSplit,
    /// Where two editor groups meet, in permille of the editor area for the
    /// same reason as `GitSplit`: half is the right default at every width.
    EditorSplit,
    /// The assistant drawer's width. Anchored to the right edge of the
    /// window, so dragging left grows it — the dock's rule turned on its
    /// side. It was a fixed 400px for a release, which is too narrow for a
    /// chapter's formulas and too wide for a one-line answer, depending on
    /// the afternoon.
    Assistant,
    /// The Git panel's branch and tag list against everything else.
    GitSidebar,
    /// The simulated board beside the editor: anchored to the right like
    /// the assistant drawer, so dragging left grows it.
    Board,
}

impl Divider {
    pub const ALL: [Divider; 12] = [
        Divider::Tree,
        Divider::Dock,
        Divider::DebugStack,
        Divider::GitDetail,
        Divider::GitMessage,
        Divider::GitFiles,
        Divider::GitChanges,
        Divider::GitSplit,
        Divider::EditorSplit,
        Divider::Assistant,
        Divider::GitSidebar,
        Divider::Board,
    ];

    /// Whether the line is vertical — a column split, dragged left and right.
    pub fn vertical(self) -> bool {
        matches!(
            self,
            Divider::Tree
                | Divider::DebugStack
                | Divider::GitFiles
                | Divider::GitChanges
                | Divider::GitSplit
                | Divider::EditorSplit
                | Divider::Assistant
                | Divider::GitSidebar
                | Divider::Board
        )
    }

    /// How far a drag that started at `from_pointer` has moved the divider
    /// now that the pointer is at (`x`, `y`) — positive in the direction that
    /// grows the region the divider sizes. Spelled here, once, because the
    /// drag listener and the handle must agree on which axis a divider lives
    /// on, and a divider added to one and not the other drags sideways.
    pub fn travel(self, from_pointer: f64, x: f64, y: f64) -> f64 {
        match self {
            Divider::Tree
            | Divider::DebugStack
            | Divider::GitFiles
            | Divider::GitChanges
            | Divider::GitSplit
            | Divider::EditorSplit
            | Divider::GitSidebar => x - from_pointer,
            // Anchored to the bottom, so dragging up grows it.
            Divider::Dock | Divider::GitDetail => from_pointer - y,
            // Anchored to the right, so dragging left grows it.
            Divider::Assistant | Divider::Board => from_pointer - x,
            // Anchored to the top, so dragging down grows it.
            Divider::GitMessage => y - from_pointer,
        }
    }

    /// Where a divider sits before anyone has dragged it: in pixels, or in
    /// permille for the two that are a share of their width (`GitSplit`,
    /// `EditorSplit`).
    ///
    /// Spelled once: the boot default and View ▸ Reset layout both read it,
    /// so a divider added here is reset there without a second copy of the
    /// number to forget.
    pub fn default_size(self) -> f64 {
        match self {
            Divider::Tree => 240.0,
            Divider::Dock => 196.0,
            Divider::DebugStack => 420.0,
            Divider::GitDetail => 340.0,
            Divider::GitMessage => 140.0,
            Divider::GitFiles => 380.0,
            Divider::GitChanges => 380.0,
            Divider::Assistant => 400.0,
            Divider::GitSidebar => 240.0,
            Divider::Board => 560.0,
            // Permille: half and half.
            Divider::GitSplit | Divider::EditorSplit => 500.0,
        }
    }

    /// Bounds, in the divider's own units — pixels, or permille for the two
    /// splits. The lower one keeps a region usable rather than letting it be
    /// dragged to nothing — collapsing is what the toggle is for, and a
    /// two-pixel sidebar is not a smaller sidebar, it is a mistake.
    pub fn bounds(self) -> (f64, f64) {
        match self {
            Divider::Tree => (160.0, 440.0),
            Divider::Dock => (80.0, 600.0),
            Divider::DebugStack => (140.0, 900.0),
            Divider::GitDetail => (120.0, 1400.0),
            Divider::GitMessage => (40.0, 1000.0),
            Divider::GitFiles => (160.0, 1200.0),
            Divider::GitChanges => (220.0, 1200.0),
            Divider::GitSidebar => (160.0, 520.0),
            // Narrower than 300 and a formula wraps mid-fraction; wider than
            // 900 and there is no editor left beside it on a laptop.
            Divider::Assistant => (300.0, 900.0),
            // The sheet's corner controls and one devkit need about 340; past
            // 1400 there is no editor left on any screen worth having.
            Divider::Board => (340.0, 1400.0),
            // Neither side narrower than a seventh of the text.
            Divider::GitSplit => (150.0, 850.0),
            // Neither group narrower than a fifth of the area: a group that
            // thin shows a gutter and no code.
            Divider::EditorSplit => (200.0, 800.0),
        }
    }

    pub(super) fn storage_key(self) -> &'static str {
        match self {
            Divider::Tree => "rusty.layout.tree",
            Divider::Dock => "rusty.layout.dock",
            Divider::DebugStack => "rusty.layout.debug",
            Divider::GitDetail => "rusty.layout.git-detail",
            Divider::GitMessage => "rusty.layout.git-message",
            Divider::GitFiles => "rusty.layout.git-files",
            Divider::GitChanges => "rusty.layout.git-changes",
            Divider::GitSplit => "rusty.layout.git-split",
            Divider::EditorSplit => "rusty.layout.editor-split",
            Divider::Assistant => "rusty.layout.assistant",
            Divider::GitSidebar => "rusty.layout.git-sidebar",
            Divider::Board => "rusty.layout.board",
        }
    }
}

/// Where the dividers sit and what is on screen. Sizes are the one thing
/// here that survives a reload, in localStorage.
#[derive(Clone, Copy)]
pub struct Layout {
    /// Sidebar width and dock height, in pixels, remembered across sessions.
    ///
    /// A fixed-size panel is the first thing anyone tries to drag, and finding
    /// that they cannot is the moment a tool starts feeling rigid.
    pub tree_width: RwSignal<f64>,
    pub dock_height: RwSignal<f64>,
    /// How much of the Debug tab the call stack gets.
    pub debug_width: RwSignal<f64>,
    /// The Git panel's four splits: the opened commit's height, the cap on
    /// its message, its file column, and the Changes view's file column.
    pub git_detail_height: RwSignal<f64>,
    pub git_message_height: RwSignal<f64>,
    pub git_files_width: RwSignal<f64>,
    pub git_changes_width: RwSignal<f64>,
    /// Where old meets new in a side-by-side diff, in permille of the text.
    pub git_split: RwSignal<f64>,
    /// Which divider is being dragged, if any. Held centrally so the window
    /// listeners are set up once rather than per handle.
    pub dragging: RwSignal<Option<Divider>>,
    /// Where the grab started: pointer coordinate, the size at that moment,
    /// and how many of the size's units one pixel of travel is worth — 1.0
    /// for every divider sized in pixels. Dragging moves relative to this —
    /// absolute window arithmetic has to know about every bar between the
    /// divider and the window edge, and got it wrong by exactly their sum.
    pub drag_from: RwSignal<(f64, f64, f64)>,
    /// The bottom dock. Open by default: a build or flash that writes into a
    /// hidden drawer is a build whose failure the user finds out about later.
    pub dock_open: RwSignal<bool>,
    pub dock_tab: RwSignal<DockTab>,
    /// Which tabs the strip carries, in [`DockTab::ALL`]'s order. The pinned
    /// three from the first paint; the rest appear when something puts them
    /// there — a debug run, a serial link, the firmware's first telemetry
    /// sample — and go when the user hides them. Nine tabs on a window with
    /// no project open was nine names for things that were not happening.
    ///
    /// Session state, not persisted: nothing is running at boot, so the strip
    /// starts with what is true at boot, and the View menu lists them all for
    /// anyone who wants one before it has anything to show.
    pub dock_tabs: RwSignal<Vec<DockTab>>,
    pub panel: RwSignal<String>,
    /// Whole-interface scale, browser-zoom style. 1.0 is native.
    pub zoom: RwSignal<f64>,
    /// Two editor groups side by side, and which one the user last worked
    /// in — where a file opened from the tree, the finder or a search hit
    /// lands.
    pub split: RwSignal<bool>,
    pub focus: RwSignal<Group>,
    /// Where the two groups meet, in permille of the editor area's width.
    pub editor_split: RwSignal<f64>,
    /// The assistant drawer's width in pixels (`Divider::Assistant`).
    pub assistant_width: RwSignal<f64>,
    /// The Git panel's branch list, in pixels (`Divider::GitSidebar`).
    pub git_sidebar_width: RwSignal<f64>,
    /// The simulated board beside the editor, Wokwi's shape: code on the
    /// left, the board running it on the right, the Output below both.
    /// Session state — the playground turns it on, and anybody may.
    pub board_beside: RwSignal<bool>,
    /// Its width in pixels (`Divider::Board`).
    pub board_width: RwSignal<f64>,
    /// The file tree folded away: a second click on the Files switcher, or
    /// Ctrl+B. Remembered across sessions, like the pin map's fold.
    pub tree_hidden: RwSignal<bool>,
    /// The file finder (Ctrl+P) is up.
    pub quick_open: RwSignal<bool>,
    /// What the finder opens with already typed: `@` lists the file's
    /// symbols and `#` the workspace's, VS Code's prefixes — so Ctrl+Shift+O
    /// and Ctrl+T are the finder with one character in it.
    pub quick_seed: RwSignal<String>,
    /// Places the finder lists instead of files, when a command asked for
    /// some: a symbol's references, its implementations.
    pub quick_places: RwSignal<Option<PlaceList>>,
    /// The symbols the finder last asked the server for, and which ask.
    pub quick_symbols: RwSignal<Option<SymbolAnswer>>,
    /// Ctrl+Tab's list, while Ctrl is held.
    pub switcher: RwSignal<Option<Switcher>>,
    /// The call hierarchy in the dock's Calls tab.
    pub calls: RwSignal<CallsView>,
}

impl Layout {
    /// The signal a divider drives. One mapping, so the drag handles, the
    /// boot restore and Reset layout cannot disagree about which region a
    /// divider resizes.
    pub fn size_signal(&self, divider: Divider) -> RwSignal<f64> {
        match divider {
            Divider::Tree => self.tree_width,
            Divider::Dock => self.dock_height,
            Divider::DebugStack => self.debug_width,
            Divider::GitDetail => self.git_detail_height,
            Divider::GitMessage => self.git_message_height,
            Divider::GitFiles => self.git_files_width,
            Divider::GitChanges => self.git_changes_width,
            Divider::GitSplit => self.git_split,
            Divider::EditorSplit => self.editor_split,
            Divider::Assistant => self.assistant_width,
            Divider::GitSidebar => self.git_sidebar_width,
            Divider::Board => self.board_width,
        }
    }
}

/// The output dock: everything any tool has said, and how it is filtered.
#[derive(Clone, Copy)]
pub struct Dock {
    /// Everything spawned tools have printed, oldest first, each line tagged
    /// with the channel that was speaking — build, flash, simulate — so the
    /// Output panel can show one conversation at a time, the way VSCode's
    /// channel picker does.
    ///
    /// Lives here rather than in any panel so it survives switching panels —
    /// watching a device is something you do *while* reading the memory
    /// report, not instead of it.
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
