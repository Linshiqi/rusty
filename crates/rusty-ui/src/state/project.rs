//! The open project, the device the verbs go to, the new-project wizard and
//! the first-run setup sheet.

use super::*;

/// The build a device verb or the Memory panel means: the one picked in the
/// Memory panel, else the one built for the configured target, else any.
/// One rule for the tracked and untracked readers, so they cannot differ.
pub(super) fn pick_firmware(all: &[Firmware], selected: Option<&str>) -> Option<Firmware> {
    selected
        .and_then(|path| all.iter().find(|f| f.path == path))
        .or_else(|| all.iter().find(|f| f.matches_configured_target))
        .or_else(|| all.first())
        .cloned()
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
    /// Where the builds went on disk, when the Disk section asked.
    pub disk: RwSignal<Option<rusty_core::DiskReport>>,
    /// A scan or a removal in flight — one at a time, since a removal
    /// re-derives what it removes and a scan read at the same moment would
    /// describe a directory being changed under it.
    pub disk_busy: RwSignal<bool>,
    /// Sweep stale artifacts after every successful cargo command. Mirrors
    /// `workbench.toml`, which the backend reads at the end of each build.
    pub disk_auto_sweep: RwSignal<bool>,
    /// Days an incremental cache may go untouched before the scan calls it
    /// idle. Session state: a threshold is something to try, not to keep.
    pub disk_idle_days: RwSignal<u32>,
}

/// What is plugged in, and the command that would talk to it.
#[derive(Clone, Copy)]
pub struct Device {
    /// Devices currently attached, as the last scan found them. The title
    /// bar's picker lists these and Flash chooses among them — one list, so
    /// the two cannot disagree about what is plugged in.
    pub ports: RwSignal<Vec<SerialPort>>,
    pub probes: RwSignal<Vec<Probe>>,
    /// How to reach the board, once a device has been chosen.
    pub transport: RwSignal<Option<Transport>>,
    /// The command Flash would run, shown in the picker before it does.
    pub plan: RwSignal<Option<CommandPlan>>,
    /// The title bar's device picker is open.
    pub picker: RwSignal<bool>,
    /// What Flash or Monitor was about to do when it found no device to do
    /// it to — done the moment one is picked, rather than asking for the
    /// click a second time.
    pub pending: RwSignal<Option<DeviceAction>>,
}

/// What to do once the running session has actually exited: a flash asked
/// for while a monitor held the port, or a simulation restarted with the
/// code on screen. Done from the old session's own exit, which is the only
/// moment the port is certainly free and the old session's end cannot
/// clear the new one's running flag.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AfterStop {
    Device(DeviceAction),
    Simulate { debug: bool },
}

/// The device verbs, as a picker waiting for a device holds them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceAction {
    /// Build, write the image, and stay attached — the inner loop.
    Flash,
    /// Build and write the image, and stop there.
    FlashOnly,
    /// Attach to what is already on the board.
    Monitor,
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

/// The first-run environment check, the install queue it drives, and the
/// progress of any install — which the Environment page draws too.
///
/// The sheet answers "can you build anything at all" without being asked;
/// the Environment page answers "what is on this machine" whenever somebody
/// asks. They read the same report — `rusty_embed::setup::plan` is the one
/// derivation — and the same `busy`, `installed` and `failed`, so an install
/// begun on one is drawn on both; only the sheet interrupts.
#[derive(Clone, Copy)]
pub struct Setup {
    /// The screen is up.
    pub open: RwSignal<bool>,
    /// What is missing, newest report first.
    pub steps: RwSignal<Vec<rusty_embed::setup::SetupStep>>,
    /// Which step the queue is on, when it is running one.
    pub running: RwSignal<Option<usize>>,
    /// The tool being installed right now, by the queue or by one row's own
    /// Install — so the sheet and the Environment page draw the same
    /// spinner beside the same name.
    pub busy: RwSignal<Option<String>>,
    /// Tools that finished in this run, so a tick can appear beside them
    /// without waiting for the whole queue and a re-probe.
    pub installed: RwSignal<Vec<String>>,
    /// And the ones that did not, with nothing hidden: a queue that reports
    /// success for a step that failed is worse than one that stops.
    pub failed: RwSignal<Vec<String>>,
    /// True once the check has run at least once this session, so opening a
    /// second project does not reopen a screen the user has dismissed.
    pub checked: RwSignal<bool>,
    /// Where downloaded tools land, as an actual path. Shown rather than
    /// described: "the data directory" is not an answer to "where is this
    /// gigabyte going".
    pub data_dir: RwSignal<Option<String>>,
}
