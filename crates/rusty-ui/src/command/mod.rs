//! Everything the user can ask for, in one list.
//!
//! The palette and the keyboard shortcuts run the *same* actions. Written
//! twice they drift: a shortcut keeps working after the menu item it mirrors
//! has changed, and only one of them gets fixed.
//!
//! An enum rather than boxed closures, so the set is enumerable — the palette
//! lists it, and a test could assert every action is reachable.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::{AppState, Divider, DockTab, remember_size},
    theme::{self, Theme},
    view::panels,
};

/// Where users go. Named once, in `rusty_embed::model` — see [`rusty_embed::REPO`]
/// for why it is not the repository the source is in.
use rusty_embed::{REPO_ISSUES as ISSUES, REPO_RELEASES as RELEASES};

mod menus;
mod rows;

pub use menus::*;
pub use rows::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    ShowPanel(&'static str),
    OpenProject,
    /// The clone dialog: a URL, a folder, and a project at the end of it.
    CloneRepository,
    RefreshProject,
    RefreshToolchain,
    ReloadCatalog,
    ScanDevices,
    /// The project's verbs — the title bar's buttons, the menus' rows and
    /// the keys, one action each, so none of the three can come to mean
    /// something the others do not.
    Build,
    Test,
    /// In the simulator.
    Run,
    Debug,
    Stop,
    /// The running simulation stopped and started again with the code on
    /// screen — Run does the same while one runs.
    Restart,
    /// The debugger's own verbs, the transport's buttons on VS Code's keys:
    /// F6, F10, F11 and Shift+F11. Continue is Debug's F5, which resumes a
    /// session that has stopped.
    Pause,
    StepOver,
    StepInto,
    StepOut,
    /// A chip's playground, opened: code beside the board, no project to
    /// make first. The chip as `rusty_embed::PLAYGROUND_CHIPS` spells it.
    OpenPlayground(&'static str),
    /// The open playground's example back, and what is in it kept as a
    /// project of its own.
    ResetPlayground,
    KeepPlayground,
    /// The simulated board beside the editor, or not.
    ToggleBoard,
    /// Build, write the image to the chosen device, and stay attached.
    Flash,
    /// Build and write the image, and stop there.
    FlashOnly,
    /// Attach to the chosen device without writing anything.
    Monitor,
    /// Open the title bar's device picker.
    PickDevice,
    /// Open the nth entry of the recents list. An index rather than the path
    /// so the action stays `Copy`; resolved against the list at run time.
    OpenRecent(usize),
    /// Editor edits, routed to the focused editor as the keystroke would be —
    /// the menu and Ctrl+Z must be the same muscle.
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    ToggleDock,
    ShowDock(DockTab),
    OpenPalette,
    /// The file finder: type part of a name, Enter opens it.
    QuickOpen,
    /// The finder over the symbols of the file in front (`@`), or of the
    /// whole workspace (`#`).
    GoToSymbolInFile,
    GoToSymbolInWorkspace,
    /// The finder with `:` typed: a line of the file in front, by number.
    GoToLine,
    /// Where the symbol at the caret is used, implemented or typed, through
    /// the language server: a jump to one place, the finder for several.
    FindReferences,
    GoToImplementations,
    GoToTypeDefinition,
    /// Who calls the function at the caret, and what it calls, in the
    /// dock's Calls tab.
    ShowCallHierarchy,
    /// The macro call at the caret expanded all the way down, opened beside
    /// the code.
    ExpandMacro,
    /// Fold the Files panel's tree away, or bring it back.
    ToggleTree,
    /// The left group's file opened on the right as well: a second view of
    /// the same document.
    SplitEditor,
    OpenSettings,
    /// The environment check, on purpose rather than because it interrupted.
    CheckEnvironment,
    /// Ask the release feed for a newer rusty, and show what it answered.
    CheckUpdates,
    SetTheme(Theme),
    ResetLayout,
    /// Scaffold C interop, in whichever direction.
    ScaffoldC(&'static str),
    CloseWindow,
    /// Modal editing on or off.
    ToggleVim,
    /// Back and forward through the positions the caret has visited.
    NavBack,
    NavForward,
    /// The focused group's files, most recently used first — Ctrl+Tab, and
    /// Ctrl+Shift+Tab to start from the least recent. Held, the keys walk
    /// the list and letting go opens the pick (`view/switcher.rs`); run from
    /// the palette or a menu, it is a tap.
    SwitchEditor,
    SwitchEditorBack,
    /// Comment or uncomment the selected lines. Not a Vim feature — this
    /// editor had none at all, in any mode.
    ToggleComment,
    /// Rename the symbol under the caret, through the language server.
    Rename,
    /// Open a page in the desktop browser. `&'static str` so the action stays
    /// `Copy` and can sit in the palette beside every other one.
    OpenUrl(&'static str),
}

/// The overlays an action might open.
///
/// Passed explicitly rather than read from context: the keyboard handler is a
/// window listener, which runs outside the reactive owner that holds context,
/// and `expect_context` there fails at runtime rather than at compile time.
#[derive(Clone, Copy)]
pub struct Chrome {
    pub settings_open: RwSignal<bool>,
    pub palette_open: RwSignal<bool>,
}

/// One row in the palette.
#[derive(Clone)]
pub struct Command {
    pub action: Action,
    pub title: String,
    /// The heading it appears under, and part of what a search matches.
    /// Translated like the title: a heading is read, not typed.
    pub group: String,
    /// Shown right-aligned. `None` for anything without a binding.
    pub shortcut: Option<String>,
}

/// A chip as people call it — `ESP32-C3` for `esp32c3` — from the
/// catalogue, or the id in capitals before the catalogue has arrived.
pub fn chip_name(state: AppState, chip: &str) -> String {
    state
        .project
        .chips
        .with_untracked(|chips| chips.iter().find(|c| c.id == chip).map(|c| c.name.clone()))
        .unwrap_or_else(|| chip.to_uppercase())
}

/// A recents entry as a menu label: the folder, then where it is — two
/// projects both named `firmware` are told apart by the rest of the path.
pub fn recent_label(path: &str) -> String {
    let name = path
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path);
    format!("{name} — {path}")
}

/// When a menu row is available.
///
/// An enum rather than a closure, for the reason [`Action`] is one: the set
/// is enumerable and a row cannot capture state it should not see. It also
/// keeps the *rendering* reactive — the menu is built once, and the answer
/// is re-derived whenever what it depends on changes.
///
/// Greying out rather than hiding, which is this project's menu convention:
/// present but unavailable says "this exists, and here is when"; hidden says
/// nothing at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Requires {
    Nothing,
    /// A project open. File > Save with no document, in other words.
    Project,
    /// Somewhere to go. Back at the start of a session is the case that
    /// otherwise looks like a broken button.
    NavBack,
    NavForward,
    /// A playground open: its example and keeping it mean nothing elsewhere.
    Playground,
    /// A debug session stopped, for the steps; one running, for Pause —
    /// exactly when the transport's own buttons are live.
    DebugStopped,
    DebugRunning,
}

impl Requires {
    pub fn met(self, state: AppState) -> bool {
        match self {
            Requires::Nothing => true,
            Requires::Project => state.has_project(),
            Requires::Playground => state.playground().is_some(),
            Requires::DebugStopped => state
                .debug
                .session
                .with(|s| s.as_ref().is_some_and(rusty_dbg::DebugState::stopped)),
            Requires::DebugRunning => state
                .debug
                .session
                .with(|s| s.as_ref().is_some_and(|s| s.running)),
            Requires::NavBack => state.editor.nav.with(|nav| nav.can_go_back()),
            Requires::NavForward => state.editor.nav.with(|nav| nav.can_go_forward()),
        }
    }
}

/// Carry out an action.
pub fn run(action: Action, state: AppState, chrome: Chrome) {
    match action {
        Action::CheckEnvironment => {
            // Re-probe first: the report may be from before somebody
            // installed something in a terminal, and a page that says a tool
            // is missing when it is not is worse than no page. The
            // Environment page rather than the first-run sheet: it says
            // everything the sheet does and what is installed besides.
            controller::refresh_toolchain(state);
            state.layout.panel.set("toolchain".to_string());
        }
        Action::Build => {
            if state.has_project_now() {
                controller::build_project(state);
            }
        }
        Action::Test => {
            if state.has_project_now() {
                controller::test_project(state);
            }
        }
        Action::Run | Action::Debug => {
            if state.has_project_now() {
                controller::simulate(state, action == Action::Debug);
            }
        }
        Action::Stop => {
            // A test under the debugger is a debug session and not a tool
            // on the dock's session, so `session_running` never says so —
            // and Shift+F5 and the menu's Stop did nothing to it, while the
            // transport's Stop, which never asked, stopped it.
            if state.app.session_running.get_untracked()
                || state.debug.session.with_untracked(Option::is_some)
            {
                controller::stop_anything(state);
            }
        }
        Action::Restart => {
            if state.has_project_now() {
                controller::restart_simulation(state);
            }
        }
        Action::Pause => controller::debug_verb(state, "pause"),
        Action::StepOver => controller::debug_verb(state, "over"),
        Action::StepInto => controller::debug_verb(state, "into"),
        Action::StepOut => controller::debug_verb(state, "out"),
        Action::OpenPlayground(chip) => controller::open_playground(state, chip),
        Action::ResetPlayground => controller::reset_playground(state),
        Action::KeepPlayground => controller::keep_playground(state),
        Action::ToggleBoard => {
            if state.has_project_now() {
                controller::toggle_board(state);
            }
        }
        Action::Flash | Action::FlashOnly | Action::Monitor => {
            if state.has_project_now() {
                let verb = match action {
                    Action::Flash => crate::state::DeviceAction::Flash,
                    Action::FlashOnly => crate::state::DeviceAction::FlashOnly,
                    _ => crate::state::DeviceAction::Monitor,
                };
                controller::device_action(state, verb);
            }
        }
        Action::PickDevice => {
            if state.has_project_now() {
                controller::scan_devices(state);
                state.device.picker.set(true);
            }
        }
        Action::CheckUpdates => controller::check_update(state, true),
        Action::ShowPanel("assistant") => state.ai.open.set(true),
        Action::ShowPanel(id) => {
            // Silently ignoring a blocked panel would leave the palette looking
            // broken; the sidebar already explains the requirement.
            let allowed = panels::all()
                .into_iter()
                .find(|p| p.id == id)
                .is_some_and(|p| !p.needs_project || state.has_project_now());
            if allowed {
                state.layout.panel.set(id.to_string());
            }
        }
        Action::OpenProject => controller::choose_project(state),
        Action::CloneRepository => controller::open_clone_dialog(state),
        Action::OpenRecent(index) => {
            if let Some(path) = state
                .app
                .recents
                .with_untracked(|list| list.get(index).cloned())
            {
                controller::open_recent(state, path, true);
            }
        }
        Action::Undo => editor_key(state, "z", false),
        Action::Redo => editor_key(state, "y", false),
        Action::Cut => editor_exec(state, "cut", None),
        Action::Copy => editor_exec(state, "copy", None),
        Action::Paste => {
            // Through the async clipboard, then execCommand('insertText') so
            // the insertion fires a real input event — history, echo and the
            // language server all hear about it exactly as if typed.
            use wasm_bindgen_futures::JsFuture;
            leptos::task::spawn_local(async move {
                let Some(window) = web_sys::window() else {
                    return;
                };
                let promise = window.navigator().clipboard().read_text();
                if let Ok(value) = JsFuture::from(promise).await
                    && let Some(text) = value.as_string()
                    && !text.is_empty()
                {
                    editor_exec(state, "insertText", Some(&text));
                }
            });
        }
        Action::RefreshProject => controller::refresh_project(state),
        Action::RefreshToolchain => controller::refresh_toolchain(state),
        Action::ReloadCatalog => controller::load_catalog(state),
        Action::ScanDevices => controller::scan_devices(state),
        Action::ToggleDock => state.layout.dock_open.update(|open| *open = !*open),
        Action::ShowDock(tab) => state.show_dock(tab),
        Action::OpenPalette => chrome.palette_open.set(true),
        Action::QuickOpen
        | Action::GoToLine
        | Action::GoToSymbolInFile
        | Action::GoToSymbolInWorkspace => {
            if state.has_project_now() {
                let seed = match action {
                    Action::GoToLine => ":",
                    Action::GoToSymbolInFile => "@",
                    Action::GoToSymbolInWorkspace => "#",
                    _ => "",
                };
                state.layout.quick_places.set(None);
                state.layout.quick_seed.set(seed.to_string());
                state.layout.quick_open.set(true);
            }
        }
        Action::FindReferences => {
            controller::find_places(state.focused(), controller::PlaceQuery::References)
        }
        Action::GoToImplementations => {
            controller::find_places(state.focused(), controller::PlaceQuery::Implementations)
        }
        Action::GoToTypeDefinition => {
            controller::find_places(state.focused(), controller::PlaceQuery::TypeDefinition)
        }
        Action::ShowCallHierarchy => controller::show_call_hierarchy(state.focused()),
        Action::ExpandMacro => controller::expand_macro(state.focused()),
        Action::ToggleTree => controller::toggle_tree(state),
        Action::SplitEditor => controller::split_active(state),
        Action::OpenSettings => chrome.settings_open.set(true),
        Action::CloseWindow => controller::window_action(crate::ipc::cmd::window::CLOSE),
        Action::OpenUrl(url) => controller::open_url(state, url.to_string()),
        Action::ToggleComment => editor_key(state, "/", false),
        Action::Rename => editor_chord(state, "F2", false, false),
        // The group the user is in: a jump list belongs to an editor, and
        // there are two.
        Action::NavBack => controller::nav_back(state.focused()),
        Action::NavForward => controller::nav_forward(state.focused()),
        // With no key held there is nothing to let go of, so open and commit
        // together: the tap. The held form never comes through here — the
        // switcher's own listener takes the keys first (`view/switcher.rs`).
        Action::SwitchEditor | Action::SwitchEditorBack => {
            controller::switch_editor(state, action == Action::SwitchEditorBack);
            controller::commit_switch(state);
        }
        Action::ToggleVim => {
            let on = !state.editor.vim_on.get_untracked();
            controller::set_vim(state, on);
        }
        Action::SetTheme(theme) => theme::set(theme),
        Action::ScaffoldC(direction) => controller::scaffold_c_interop(state, direction),
        Action::ResetLayout => {
            // Every divider, from the one place its default is spelled. Two
            // of the three were reset here with their defaults copied in by
            // hand; the third was added later and this arm never heard.
            for divider in Divider::ALL {
                let size = divider.default_size();
                state.layout.size_signal(divider).set(size);
                remember_size(divider, size);
            }
            // And the dock's strip back to its three: the tabs are layout
            // as much as the dividers are, and "reset" that left a Flight
            // tab from last week's run in place would not read as one.
            state.reset_strip();
        }
    }
}

/// Send a Ctrl+key keydown to the editor's textarea, as the keyboard would.
///
/// The undo stack, its coalescing and its caret rules live in the editor's
/// own keydown path; synthesising the event means the menu cannot drift from
/// the shortcut.
fn editor_key(state: AppState, key: &str, shift: bool) {
    editor_chord(state, key, true, shift);
}

/// The same, for keys that are not Ctrl chords — F2 is a bare key, and
/// sending it as Ctrl+F2 would reach a handler that is not listening.
///
/// To the focused group's textarea (`controller::editor_area`): a menu item
/// acts on the file being worked in, not on whichever group is leftmost.
fn editor_chord(state: AppState, key: &str, ctrl: bool, shift: bool) {
    let Some(element) = controller::editor_area(state.focused().group) else {
        return;
    };
    let _ = element.focus();
    let options = web_sys::KeyboardEventInit::new();
    options.set_key(key);
    options.set_ctrl_key(ctrl);
    options.set_shift_key(shift);
    options.set_bubbles(true);
    options.set_cancelable(true);
    if let Ok(event) =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &options)
    {
        let _ = element.dispatch_event(&event);
    }
}

/// Run a document editing command against the focused editor.
fn editor_exec(state: AppState, command: &str, argument: Option<&str>) {
    use wasm_bindgen::JsCast;
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Some(element) = controller::editor_area(state.focused().group) {
        let _ = element.focus();
    }
    let Ok(html) = document.dyn_into::<web_sys::HtmlDocument>() else {
        return;
    };
    let _ = match argument {
        Some(value) => html.exec_command_with_show_ui_and_value(command, false, value),
        None => html.exec_command(command),
    };
}

/// Whether `needle` appears in `haystack` in order, ignoring case.
///
/// Subsequence rather than substring, which is what a palette is expected to
/// do: "flsh" should find "Flash", and "gtov" should find "Go to Overview".
pub fn matches(needle: &str, haystack: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut chars = needle.chars().flat_map(char::to_lowercase).peekable();
    for candidate in haystack.chars().flat_map(char::to_lowercase) {
        match chars.peek() {
            Some(wanted) if *wanted == candidate => {
                chars.next();
            }
            Some(_) => {}
            None => return true,
        }
    }
    chars.peek().is_none()
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn subsequence_matching_is_what_a_palette_does() {
        assert!(matches("flsh", "Flash"));
        assert!(matches("gtov", "Go to Overview"));
        assert!(matches("", "anything"));
        assert!(matches("MEM", "Memory"), "case is ignored");

        assert!(!matches("xyz", "Flash"));
        // Order matters — otherwise every query matches everything.
        assert!(!matches("hsalf", "Flash"));
    }
}
