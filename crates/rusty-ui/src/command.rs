//! Everything the user can ask for, in one list.
//!
//! The palette and the keyboard shortcuts run the *same* actions. Written
//! twice they drift: a shortcut keeps working after the menu item it mirrors
//! has changed, and only one of them gets fixed.
//!
//! An enum rather than boxed closures, so the set is enumerable — the palette
//! lists it, and a test could assert every action is reachable.

use leptos::prelude::*;

use crate::{
    controller,
    state::{AppState, Divider, DockTab, remember_size},
    theme::{self, Theme},
    view::panels,
};

/// Where users go. Named once, in `rusty_embed::model` — see [`rusty_embed::REPO`]
/// for why it is not the repository the source is in.
use rusty_embed::{REPO_ISSUES as ISSUES, REPO_RELEASES as RELEASES};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    ShowPanel(&'static str),
    OpenProject,
    RefreshProject,
    RefreshToolchain,
    ReloadCatalog,
    ScanDevices,
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
    OpenSettings,
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
    pub group: &'static str,
    /// Shown right-aligned. `None` for anything without a binding.
    pub shortcut: Option<String>,
}

/// Everything available right now.
///
/// Panels come from the registry rather than a second hard-coded list, so a
/// contributed panel is reachable from the palette without anyone remembering
/// to add it — the same reason the shell renders from the registry.
pub fn all(state: AppState) -> Vec<Command> {
    let mut out = Vec::new();
    // What each action's key actually is right now — overrides included, so
    // the palette never advertises a chord that stopped working.
    let bound = crate::view::palette::effective(state);
    let chord = |action: Action| {
        bound
            .iter()
            .find(|(binding, _)| binding.action == action)
            .map(|(_, chord)| chord.clone())
    };

    for panel in panels::all() {
        // Disabled panels stay listed but say why, rather than vanishing —
        // a palette that hides things teaches people it cannot be trusted.
        let blocked = panel.needs_project && !state.has_project();
        out.push(Command {
            action: Action::ShowPanel(panel.id),
            title: if blocked {
                format!("{} — needs a project", panel.title)
            } else {
                panel.title.to_string()
            },
            group: "Go to",
            shortcut: chord(Action::ShowPanel(panel.id)),
        });
    }

    let action = |action, title: &str, shortcut| Command {
        action,
        title: title.to_string(),
        group: "Project",
        shortcut,
    };

    out.push(action(
        Action::OpenProject,
        "Open project…",
        chord(Action::OpenProject),
    ));
    out.push(action(
        Action::RefreshProject,
        "Re-check project",
        chord(Action::RefreshProject),
    ));
    out.push(action(Action::RefreshToolchain, "Re-scan toolchain", None));
    out.push(action(
        Action::ReloadCatalog,
        "Reload chips and boards",
        None,
    ));

    let view = |action, title: &str, shortcut| Command {
        action,
        title: title.to_string(),
        group: "View",
        shortcut,
    };

    out.push(view(
        Action::ToggleDock,
        "Toggle the panel below",
        chord(Action::ToggleDock),
    ));
    out.push(view(
        Action::ShowDock(DockTab::Problems),
        "Show problems",
        None,
    ));
    out.push(view(Action::ShowDock(DockTab::Output), "Show output", None));
    out.push(view(
        Action::ShowDock(DockTab::Terminal),
        "Show terminal",
        None,
    ));
    out.push(view(Action::ShowDock(DockTab::Waves), "Show waves", None));
    out.push(view(
        Action::ShowDock(DockTab::Devices),
        "Show devices",
        None,
    ));
    out.push(view(Action::ResetLayout, "Reset panel sizes", None));
    out.push(view(Action::NavBack, "Back", chord(Action::NavBack)));
    out.push(view(Action::NavForward, "Forward", chord(Action::NavForward)));
    out.push(view(Action::ToggleVim, "Vim keys in the editor", None));

    for theme in Theme::ALL {
        out.push(Command {
            action: Action::SetTheme(theme),
            title: format!("Theme: {}", theme.label()),
            group: "Settings",
            shortcut: None,
        });
    }
    out.push(Command {
        action: Action::OpenSettings,
        title: "Settings".to_string(),
        group: "Settings",
        shortcut: chord(Action::OpenSettings),
    });

    out
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
}

impl Requires {
    pub fn met(self, state: AppState) -> bool {
        match self {
            Requires::Nothing => true,
            Requires::Project => state.has_project(),
            Requires::NavBack => state.nav.with(|nav| nav.can_go_back()),
            Requires::NavForward => state.nav.with(|nav| nav.can_go_forward()),
        }
    }
}

/// One row in a menu.
#[derive(Clone)]
pub enum Item {
    Entry {
        action: Action,
        label: String,
        shortcut: Option<String>,
        /// When this row can be used. See [`Requires`].
        requires: Requires,
    },
    /// A flyout, VSCode's Open Recent shape — the list stays out of the way
    /// until asked for.
    Submenu {
        label: &'static str,
        items: Vec<Item>,
    },
    Separator,
}

pub struct Menu {
    pub title: &'static str,
    pub items: Vec<Item>,
}

fn entry(action: Action, label: &str, shortcut: Option<String>) -> Item {
    entry_when(Requires::Nothing, action, label, shortcut)
}

fn entry_when(
    requires: Requires,
    action: Action,
    label: &str,
    shortcut: Option<String>,
) -> Item {
    Item::Entry {
        action,
        label: label.to_string(),
        shortcut,
        requires,
    }
}

fn project_entry(action: Action, label: &str, shortcut: Option<String>) -> Item {
    entry_when(Requires::Project, action, label, shortcut)
}

/// The menu bar.
///
/// Same [`Action`]s as the palette and the keyboard, so a menu item cannot come
/// to mean something its shortcut does not. The panel entries are read from the
/// registry for the same reason they are in the sidebar — a contributed panel
/// appears in View without anyone remembering to add it.
pub fn menus(state: AppState) -> Vec<Menu> {
    let bound = crate::view::palette::effective(state);
    let chord = |action: Action| {
        bound
            .iter()
            .find(|(binding, _)| binding.action == action)
            .map(|(_, chord)| chord.clone())
    };

    // Shaped like VSCode's View menu: palette on top, appearance folded into
    // a submenu, then the panels the sidebar shows — and only those. The
    // wizard and the assistant have their own doors; listing them here made
    // the menu a pile.
    let mut view_items = vec![
        entry(
            Action::OpenPalette,
            "Command palette…",
            chord(Action::OpenPalette),
        ),
        Item::Separator,
        Item::Submenu {
            label: "Appearance",
            items: vec![
                entry(Action::SetTheme(Theme::System), "Theme: System", None),
                entry(Action::SetTheme(Theme::Light), "Theme: Light", None),
                entry(Action::SetTheme(Theme::Dark), "Theme: Dark", None),
                Item::Separator,
                entry(Action::ResetLayout, "Reset panel sizes", None),
                Item::Separator,
                entry_when(Requires::NavBack, Action::NavBack, "Back", chord(Action::NavBack)),
                entry_when(
                    Requires::NavForward,
                    Action::NavForward,
                    "Forward",
                    chord(Action::NavForward),
                ),
                Item::Separator,
                entry(Action::ToggleVim, "Vim keys in the editor", None),
            ],
        },
        Item::Separator,
    ];
    for panel in panels::all().into_iter().filter(|p| !p.hidden) {
        view_items.push(Item::Entry {
            action: Action::ShowPanel(panel.id),
            label: panel.title.to_string(),
            shortcut: chord(Action::ShowPanel(panel.id)),
            requires: if panel.needs_project {
                Requires::Project
            } else {
                Requires::Nothing
            },
        });
    }
    view_items.extend([
        Item::Separator,
        entry(Action::ToggleDock, "Panel below", chord(Action::ToggleDock)),
        entry(Action::ShowDock(DockTab::Problems), "Problems", None),
        entry(Action::ShowDock(DockTab::Output), "Output", None),
        entry(Action::ShowDock(DockTab::Terminal), "Terminal", None),
        entry(Action::ShowDock(DockTab::Waves), "Waves", None),
        entry(Action::ShowDock(DockTab::Devices), "Devices", None),
    ]);

    vec![
        Menu {
            title: "File",
            items: {
                let mut items = vec![
                    entry(Action::ShowPanel("wizard"), "New project…", None),
                    entry(
                        Action::OpenProject,
                        "Open project…",
                        chord(Action::OpenProject),
                    ),
                ];
                let recents = state.recents.get_untracked();
                if !recents.is_empty() {
                    let recent_items = recents
                        .iter()
                        .take(8)
                        .enumerate()
                        .map(|(index, path)| {
                            entry(Action::OpenRecent(index), &recent_label(path), None)
                        })
                        .collect();
                    items.push(Item::Submenu {
                        label: "Open Recent",
                        items: recent_items,
                    });
                }
                items.extend([
                    Item::Separator,
                    entry(
                        Action::OpenSettings,
                        "Settings…",
                        chord(Action::OpenSettings),
                    ),
                    Item::Separator,
                    entry(Action::CloseWindow, "Exit", None),
                ]);
                items
            },
        },
        Menu {
            title: "Edit",
            items: vec![
                project_entry(Action::Undo, "Undo", Some("Ctrl+Z".to_string())),
                project_entry(Action::Redo, "Redo", Some("Ctrl+Y".to_string())),
                Item::Separator,
                project_entry(Action::Cut, "Cut", Some("Ctrl+X".to_string())),
                project_entry(Action::Copy, "Copy", Some("Ctrl+C".to_string())),
                project_entry(Action::Paste, "Paste", Some("Ctrl+V".to_string())),
                Item::Separator,
                project_entry(
                    Action::ShowPanel("search"),
                    "Search in project",
                    chord(Action::ShowPanel("search")),
                ),
            ],
        },
        Menu {
            title: "Project",
            items: vec![
                project_entry(
                    Action::RefreshProject,
                    "Re-check project",
                    chord(Action::RefreshProject),
                ),
                entry(Action::RefreshToolchain, "Re-scan toolchain", None),
                entry(Action::ReloadCatalog, "Reload chips and boards", None),
                Item::Separator,
                // The two directions people actually need, named as
                // directions rather than as tool names: nobody thinks
                // "I need cc", they think "I have this C driver".
                Item::Submenu {
                    label: "Add C interop",
                    items: vec![
                        project_entry(Action::ToggleComment, "Comment or uncomment", chord(Action::ToggleComment)),
                project_entry(Action::Rename, "Rename symbol…", chord(Action::Rename)),
                Item::Separator,
                project_entry(Action::ScaffoldC("rust-calls-c"), "Rust calls C…", None),
                        project_entry(Action::ScaffoldC("c-calls-rust"), "C calls Rust…", None),
                    ],
                },
            ],
        },
        Menu {
            title: "View",
            items: view_items,
        },
        Menu {
            title: "Device",
            items: vec![
                entry(Action::ScanDevices, "Re-scan ports and probes", None),
                Item::Separator,
                project_entry(
                    Action::ShowDock(DockTab::Devices),
                    "Flash and monitor…",
                    None,
                ),
                Item::Separator,
                project_entry(Action::ShowPanel("memory"), "Memory report", None),
            ],
        },
        Menu {
            title: "Help",
            items: vec![
                entry(Action::OpenSettings, "Keyboard shortcuts", None),
                entry(Action::ShowPanel("assistant"), "Ask the assistant", None),
                Item::Separator,
                // Somewhere to send a bug. Without this the only route back
                // from a user is the one they invent, and most people invent
                // none — a workbench nobody can report a fault in gets
                // reported as "it did not work" or not at all.
                entry(Action::OpenUrl(ISSUES), "Report a problem…", None),
                entry(Action::OpenUrl(RELEASES), "Downloads and releases", None),
            ],
        },
    ]
}

/// Carry out an action.
pub fn run(action: Action, state: AppState, chrome: Chrome) {
    match action {
        Action::ShowPanel("assistant") => state.assistant_open.set(true),
        Action::ShowPanel(id) => {
            // Silently ignoring a blocked panel would leave the palette looking
            // broken; the sidebar already explains the requirement.
            let allowed = panels::all()
                .into_iter()
                .find(|p| p.id == id)
                .is_some_and(|p| !p.needs_project || state.has_project());
            if allowed {
                state.active_panel.set(id.to_string());
            }
        }
        Action::OpenProject => controller::choose_project(state),
        Action::OpenRecent(index) => {
            if let Some(path) = state
                .recents
                .with_untracked(|list| list.get(index).cloned())
            {
                controller::open_recent(state, path, true);
            }
        }
        Action::Undo => editor_key("z", false),
        Action::Redo => editor_key("y", false),
        Action::Cut => editor_exec("cut", None),
        Action::Copy => editor_exec("copy", None),
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
                    editor_exec("insertText", Some(&text));
                }
            });
        }
        Action::RefreshProject => controller::refresh_project(state),
        Action::RefreshToolchain => controller::refresh_toolchain(state),
        Action::ReloadCatalog => controller::load_catalog(state),
        Action::ScanDevices => controller::scan_devices(state),
        Action::ToggleDock => state.dock_open.update(|open| *open = !*open),
        Action::ShowDock(tab) => state.show_dock(tab),
        Action::OpenPalette => chrome.palette_open.set(true),
        Action::OpenSettings => chrome.settings_open.set(true),
        Action::CloseWindow => controller::window_action(crate::ipc::cmd::window::CLOSE),
        Action::OpenUrl(url) => controller::open_url(state, url.to_string()),
        Action::ToggleComment => editor_key("/", false),
        Action::Rename => editor_chord("F2", false, false),
        Action::NavBack => controller::nav_back(state),
        Action::NavForward => controller::nav_forward(state),
        Action::ToggleVim => {
            let on = !state.vim_on.get_untracked();
            controller::set_vim(state, on);
        }
        Action::SetTheme(theme) => theme::set(theme),
        Action::ScaffoldC(direction) => controller::scaffold_c_interop(state, direction),
        Action::ResetLayout => {
            state.tree_width.set(240.0);
            state.dock_height.set(196.0);
            remember_size(Divider::Tree, 240.0);
            remember_size(Divider::Dock, 196.0);
        }
    }
}

/// Send a Ctrl+key keydown to the editor's textarea, as the keyboard would.
///
/// The undo stack, its coalescing and its caret rules live in the editor's
/// own keydown path; synthesising the event means the menu cannot drift from
/// the shortcut.
fn editor_key(key: &str, shift: bool) {
    editor_chord(key, true, shift);
}

/// The same, for keys that are not Ctrl chords — F2 is a bare key, and
/// sending it as Ctrl+F2 would reach a handler that is not listening.
fn editor_chord(key: &str, ctrl: bool, shift: bool) {
    use wasm_bindgen::JsCast;
    let Some(element) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("editor-area"))
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    else {
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
fn editor_exec(command: &str, argument: Option<&str>) {
    use wasm_bindgen::JsCast;
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Some(element) = document
        .get_element_by_id("editor-area")
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
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

#[cfg(test)]
mod menu_tests {
    use super::*;

    /// Rows that ask for a project keep asking. The check that matters is
    /// that the *constructors* carry the condition through — `project_entry`
    /// existed before `Requires` did, and a refactor that quietly turned its
    /// rows unconditional would ungrey File > Save with no document open.
    #[test]
    fn a_project_row_still_requires_a_project() {
        let Item::Entry { requires, .. } = project_entry(Action::Undo, "Undo", None) else {
            panic!("not an entry");
        };
        assert_eq!(requires, Requires::Project);

        let Item::Entry { requires, .. } = entry(Action::Undo, "Undo", None) else {
            panic!("not an entry");
        };
        assert_eq!(requires, Requires::Nothing, "a plain row asks nothing");
    }

    /// The View menu offers Back and Forward, and each says when it applies
    /// rather than sitting lit over an empty history.
    #[test]
    fn back_and_forward_are_conditional_rows() {
        let rows = [
            entry_when(Requires::NavBack, Action::NavBack, "Back", None),
            entry_when(Requires::NavForward, Action::NavForward, "Forward", None),
        ];
        for row in rows {
            let Item::Entry { requires, .. } = row else {
                panic!("not an entry");
            };
            assert_ne!(
                requires,
                Requires::Nothing,
                "a navigation row with no condition is lit over an empty history",
            );
        }
    }
}
