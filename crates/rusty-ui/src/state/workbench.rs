//! The workbench's own state: what is running, the log, and the update.

use super::*;

/// An update's way from found to installed.
///
/// `Ready` outlives the sheet on purpose: dismissed with a verified download
/// held, Settings ▸ Updates still offers the restart, and the launch check's
/// sheet is not shown twice for the same thing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum UpdateStage {
    #[default]
    Idle,
    /// A check the user asked for is in flight.
    Checking,
    Downloading,
    /// Downloaded and its signature verified; a restart installs it.
    Ready,
    /// The restart is under way. Nothing more to click.
    Applying,
}

/// The window itself — recents, shortcuts, updates, and whether something
/// is in flight.
#[derive(Clone, Copy)]
pub struct Workbench {
    /// Projects opened before, newest first — from the backend's
    /// workbench.toml, so the list survives restarts and belongs to the data
    /// directory rather than to this window.
    pub recents: RwSignal<Vec<String>>,
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
    /// The update sheet is up — after a check that found something, or one
    /// the user asked for, which shows its answer whatever it is.
    pub update_open: RwSignal<bool>,
    /// Where an update is between found and installed.
    pub update_stage: RwSignal<UpdateStage>,
    /// Bytes so far and the total, while one downloads.
    pub update_progress: RwSignal<Option<rusty_embed::UpdateProgress>>,
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
    /// What the running session is doing, for the status bar
    /// (`crate::activity`). Set when a session starts streaming, cleared
    /// when it exits — or at once when the user stops it, since a run
    /// somebody stopped has no verdict to give.
    pub activity: RwSignal<Option<crate::activity::Activity>>,
    /// How the last session ended, until the next one starts.
    pub outcome: RwSignal<Option<crate::activity::Outcome>>,
    /// What follows the running session's exit, when something is waiting
    /// on it (`AfterStop`).
    pub after_stop: RwSignal<Option<AfterStop>>,
}
