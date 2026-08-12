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
    ToggleDock,
    ShowDock(DockTab),
    OpenPalette,
    OpenSettings,
    SetTheme(Theme),
    ResetLayout,
    CloseWindow,
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
    pub shortcut: Option<&'static str>,
}

/// Everything available right now.
///
/// Panels come from the registry rather than a second hard-coded list, so a
/// contributed panel is reachable from the palette without anyone remembering
/// to add it — the same reason the shell renders from the registry.
pub fn all(state: AppState) -> Vec<Command> {
    let mut out = Vec::new();

    for (index, panel) in panels::all().into_iter().enumerate() {
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
            shortcut: match index {
                0..=8 => Some(SHORTCUT_DIGITS[index]),
                _ => None,
            },
        });
    }

    let action = |action, title: &str, shortcut| Command {
        action,
        title: title.to_string(),
        group: "Project",
        shortcut,
    };

    out.push(action(Action::OpenProject, "Open project…", Some("Ctrl O")));
    out.push(action(Action::RefreshProject, "Re-check project", Some("Ctrl R")));
    out.push(action(Action::RefreshToolchain, "Re-scan toolchain", None));
    out.push(action(Action::ReloadCatalog, "Reload chips and boards", None));

    let view = |action, title: &str, shortcut| Command {
        action,
        title: title.to_string(),
        group: "View",
        shortcut,
    };

    out.push(view(Action::ToggleDock, "Toggle the panel below", Some("Ctrl `")));
    out.push(view(Action::ShowDock(DockTab::Problems), "Show problems", None));
    out.push(view(Action::ShowDock(DockTab::Output), "Show output", None));
    out.push(view(Action::ShowDock(DockTab::Terminal), "Show terminal", None));
    out.push(view(Action::ShowDock(DockTab::Devices), "Show devices", None));
    out.push(view(Action::ResetLayout, "Reset panel sizes", None));

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
        shortcut: Some("Ctrl ,"),
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

/// Labels for the first nine panels' number shortcuts.
const SHORTCUT_DIGITS: [&str; 9] = [
    "Ctrl 1", "Ctrl 2", "Ctrl 3", "Ctrl 4", "Ctrl 5", "Ctrl 6", "Ctrl 7", "Ctrl 8", "Ctrl 9",
];

/// One row in a menu.
pub enum Item {
    Entry {
        action: Action,
        label: String,
        shortcut: Option<&'static str>,
        /// Greyed out until a project is open, the way File > Save is greyed
        /// out with no document. Present but unavailable says "this exists and
        /// here is when"; hidden says nothing at all.
        needs_project: bool,
    },
    Separator,
}

pub struct Menu {
    pub title: &'static str,
    pub items: Vec<Item>,
}

fn entry(action: Action, label: &str, shortcut: Option<&'static str>) -> Item {
    Item::Entry {
        action,
        label: label.to_string(),
        shortcut,
        needs_project: false,
    }
}

fn project_entry(action: Action, label: &str, shortcut: Option<&'static str>) -> Item {
    Item::Entry {
        action,
        label: label.to_string(),
        shortcut,
        needs_project: true,
    }
}

/// The menu bar.
///
/// Same [`Action`]s as the palette and the keyboard, so a menu item cannot come
/// to mean something its shortcut does not. The panel entries are read from the
/// registry for the same reason they are in the sidebar — a contributed panel
/// appears in View without anyone remembering to add it.
pub fn menus(state: AppState) -> Vec<Menu> {
    let mut view_items = vec![
        entry(Action::OpenPalette, "Command palette…", Some("Ctrl K")),
        Item::Separator,
    ];
    for (index, panel) in panels::all().into_iter().enumerate() {
        view_items.push(Item::Entry {
            action: Action::ShowPanel(panel.id),
            label: panel.title.to_string(),
            shortcut: SHORTCUT_DIGITS.get(index).copied(),
            needs_project: panel.needs_project,
        });
    }
    view_items.extend([
        Item::Separator,
        entry(Action::ToggleDock, "Panel below", Some("Ctrl `")),
        entry(Action::ShowDock(DockTab::Problems), "Problems", None),
        entry(Action::ShowDock(DockTab::Output), "Output", None),
        entry(Action::ShowDock(DockTab::Terminal), "Terminal", None),
        entry(Action::ShowDock(DockTab::Devices), "Devices", None),
        Item::Separator,
        entry(Action::SetTheme(Theme::System), "Theme: System", None),
        entry(Action::SetTheme(Theme::Light), "Theme: Light", None),
        entry(Action::SetTheme(Theme::Dark), "Theme: Dark", None),
        entry(Action::ResetLayout, "Reset panel sizes", None),
    ]);

    vec![
        Menu {
            title: "File",
            items: {
                let mut items = vec![
                    entry(Action::ShowPanel("wizard"), "New project…", None),
                    entry(Action::OpenProject, "Open project…", Some("Ctrl O")),
                ];
                let recents = state.recents.get_untracked();
                if !recents.is_empty() {
                    items.push(Item::Separator);
                    for (index, path) in recents.iter().take(6).enumerate() {
                        items.push(entry(Action::OpenRecent(index), &recent_label(path), None));
                    }
                }
                items.push(Item::Separator);
                items.extend([
                    project_entry(Action::RefreshProject, "Re-check project", Some("Ctrl R")),
                    entry(Action::RefreshToolchain, "Re-scan toolchain", None),
                    entry(Action::ReloadCatalog, "Reload chips and boards", None),
                    Item::Separator,
                    entry(Action::OpenSettings, "Settings…", Some("Ctrl ,")),
                    Item::Separator,
                    entry(Action::CloseWindow, "Exit", None),
                ]);
                items
            },
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
                project_entry(Action::ShowPanel("flash"), "Flash…", None),
                project_entry(Action::ShowPanel("monitor"), "Monitor…", None),
                Item::Separator,
                project_entry(Action::ShowPanel("memory"), "Memory report", None),
            ],
        },
        Menu {
            title: "Help",
            items: vec![
                entry(Action::OpenSettings, "Keyboard shortcuts", None),
                entry(Action::ShowPanel("assistant"), "Ask the assistant", None),
            ],
        },
    ]
}

/// Carry out an action.
pub fn run(action: Action, state: AppState, chrome: Chrome) {
    match action {
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
            if let Some(path) = state.recents.with_untracked(|list| list.get(index).cloned()) {
                controller::open_recent(state, path, true);
            }
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
        Action::SetTheme(theme) => theme::set(theme),
        Action::ResetLayout => {
            state.sidebar_width.set(188.0);
            state.dock_height.set(196.0);
            remember_size(Divider::Sidebar, 188.0);
            remember_size(Divider::Dock, 196.0);
        }
    }
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
