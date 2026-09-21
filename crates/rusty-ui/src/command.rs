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
                t!("panel.needs-project", panel = panel.title.clone())
            } else {
                panel.title.to_string()
            },
            group: t!("palette.group-panels"),
            shortcut: chord(Action::ShowPanel(panel.id)),
        });
    }

    let action = |action, title: &str, shortcut| Command {
        action,
        title: title.to_string(),
        group: t!("palette.group-project"),
        shortcut,
    };

    out.push(action(
        Action::OpenProject,
        &t!("menu.file.open-project"),
        chord(Action::OpenProject),
    ));
    out.push(action(
        Action::CloneRepository,
        &t!("menu.file.clone"),
        chord(Action::CloneRepository),
    ));
    for chip in rusty_embed::PLAYGROUND_CHIPS {
        out.push(action(
            Action::OpenPlayground(chip),
            &t!("palette.playground", chip = chip_name(state, chip)),
            None,
        ));
    }
    // Only in a playground: anywhere else there is no example to put back.
    if state.playground_now().is_some() {
        out.push(action(
            Action::ResetPlayground,
            &t!("menu.file.playground-reset"),
            None,
        ));
        out.push(action(
            Action::KeepPlayground,
            &t!("menu.file.playground-keep"),
            None,
        ));
    }
    out.push(action(
        Action::RefreshProject,
        &t!("menu.project.recheck"),
        chord(Action::RefreshProject),
    ));
    out.push(action(
        Action::RefreshToolchain,
        &t!("menu.project.rescan-toolchain"),
        None,
    ));
    out.push(action(
        Action::ReloadCatalog,
        &t!("menu.project.reload-catalogue"),
        None,
    ));
    for (verb, title) in [
        (Action::Build, t!("menu.project.build")),
        (Action::Test, t!("menu.project.test")),
        (Action::Run, t!("menu.project.run")),
        (Action::Debug, t!("menu.project.debug")),
        (Action::Stop, t!("menu.project.stop")),
        (Action::Restart, t!("menu.project.restart")),
        (Action::Flash, t!("menu.device.flash")),
        (Action::FlashOnly, t!("menu.device.flash-only")),
        (Action::Monitor, t!("menu.device.monitor")),
        (Action::PickDevice, t!("menu.device.pick")),
        (Action::ScanDevices, t!("menu.device.rescan")),
    ] {
        out.push(action(verb, &title, chord(verb)));
    }

    let view = |action, title: &str, shortcut| Command {
        action,
        title: title.to_string(),
        group: t!("palette.group-view"),
        shortcut,
    };

    out.push(view(
        Action::ToggleDock,
        &t!("palette.toggle-dock"),
        chord(Action::ToggleDock),
    ));
    // Every dock tab, from the one list the dock itself renders. Five of
    // the nine were spelled out here once; the other four were reachable
    // from nowhere but a click on the strip, which this module's header
    // says is exactly the drift it exists to prevent.
    for tab in DockTab::ALL {
        out.push(view(
            Action::ShowDock(tab),
            &t!("palette.show-dock", name = tab.label()),
            None,
        ));
    }
    out.push(view(
        Action::ResetLayout,
        &t!("menu.view.reset-layout"),
        None,
    ));
    out.push(view(
        Action::NavBack,
        &t!("menu.view.back"),
        chord(Action::NavBack),
    ));
    out.push(view(
        Action::NavForward,
        &t!("menu.view.forward"),
        chord(Action::NavForward),
    ));
    out.push(view(
        Action::SwitchEditor,
        &t!("menu.view.switch-editor"),
        chord(Action::SwitchEditor),
    ));
    out.push(view(
        Action::SwitchEditorBack,
        &t!("menu.view.switch-editor-back"),
        chord(Action::SwitchEditorBack),
    ));
    out.push(view(Action::ToggleVim, &t!("menu.view.vim"), None));
    out.push(view(
        Action::QuickOpen,
        &t!("menu.view.quick-open"),
        chord(Action::QuickOpen),
    ));
    for (action, title) in [
        (Action::GoToLine, t!("menu.view.go-to-line")),
        (Action::GoToSymbolInFile, t!("menu.view.symbol-in-file")),
        (
            Action::GoToSymbolInWorkspace,
            t!("menu.view.symbol-in-workspace"),
        ),
        (Action::FindReferences, t!("menu.view.references")),
        (Action::GoToImplementations, t!("menu.view.implementations")),
        (Action::GoToTypeDefinition, t!("menu.view.type-definition")),
        (Action::ShowCallHierarchy, t!("menu.view.call-hierarchy")),
        (Action::ExpandMacro, t!("menu.view.expand-macro")),
    ] {
        out.push(view(action, &title, chord(action)));
    }
    out.push(view(
        Action::ToggleTree,
        &t!("menu.view.toggle-tree"),
        chord(Action::ToggleTree),
    ));
    out.push(view(
        Action::SplitEditor,
        &t!("menu.view.split"),
        chord(Action::SplitEditor),
    ));
    out.push(view(
        Action::ToggleBoard,
        &t!("menu.view.board-beside"),
        chord(Action::ToggleBoard),
    ));

    for theme in Theme::ALL {
        out.push(Command {
            action: Action::SetTheme(theme),
            title: t!("palette.theme", name = theme.label()),
            group: t!("palette.group-settings"),
            shortcut: None,
        });
    }
    out.push(Command {
        action: Action::OpenSettings,
        title: t!("palette.settings"),
        group: t!("palette.group-settings"),
        shortcut: chord(Action::OpenSettings),
    });

    out
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
}

impl Requires {
    pub fn met(self, state: AppState) -> bool {
        match self {
            Requires::Nothing => true,
            Requires::Project => state.has_project(),
            Requires::Playground => state.playground().is_some(),
            Requires::NavBack => state.editor.nav.with(|nav| nav.can_go_back()),
            Requires::NavForward => state.editor.nav.with(|nav| nav.can_go_forward()),
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
        label: String,
        items: Vec<Item>,
    },
    Separator,
}

pub struct Menu {
    pub title: String,
    pub items: Vec<Item>,
}

fn entry(action: Action, label: &str, shortcut: Option<String>) -> Item {
    entry_when(Requires::Nothing, action, label, shortcut)
}

fn entry_when(requires: Requires, action: Action, label: &str, shortcut: Option<String>) -> Item {
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
            &t!("menu.view.palette"),
            chord(Action::OpenPalette),
        ),
        project_entry(
            Action::QuickOpen,
            &t!("menu.view.quick-open"),
            chord(Action::QuickOpen),
        ),
        project_entry(
            Action::GoToLine,
            &t!("menu.view.go-to-line"),
            chord(Action::GoToLine),
        ),
        project_entry(
            Action::GoToSymbolInFile,
            &t!("menu.view.symbol-in-file"),
            chord(Action::GoToSymbolInFile),
        ),
        project_entry(
            Action::GoToSymbolInWorkspace,
            &t!("menu.view.symbol-in-workspace"),
            chord(Action::GoToSymbolInWorkspace),
        ),
        project_entry(
            Action::FindReferences,
            &t!("menu.view.references"),
            chord(Action::FindReferences),
        ),
        project_entry(
            Action::GoToImplementations,
            &t!("menu.view.implementations"),
            chord(Action::GoToImplementations),
        ),
        project_entry(
            Action::GoToTypeDefinition,
            &t!("menu.view.type-definition"),
            chord(Action::GoToTypeDefinition),
        ),
        project_entry(
            Action::ShowCallHierarchy,
            &t!("menu.view.call-hierarchy"),
            chord(Action::ShowCallHierarchy),
        ),
        project_entry(
            Action::ExpandMacro,
            &t!("menu.view.expand-macro"),
            chord(Action::ExpandMacro),
        ),
        Item::Separator,
        Item::Submenu {
            label: t!("menu.view.appearance"),
            items: vec![
                entry(
                    Action::SetTheme(Theme::System),
                    &t!("menu.view.theme-system"),
                    None,
                ),
                entry(
                    Action::SetTheme(Theme::Light),
                    &t!("menu.view.theme-light"),
                    None,
                ),
                entry(
                    Action::SetTheme(Theme::Dark),
                    &t!("menu.view.theme-dark"),
                    None,
                ),
                Item::Separator,
                entry(Action::ResetLayout, &t!("menu.view.reset-layout"), None),
            ],
        },
        Item::Separator,
        // Above Appearance, not inside it. Both of these were folded into
        // that submenu at first, where nobody found them — a modal editing
        // switch is not a *look*, and navigation certainly is not. The
        // symptom was exactly what you would expect: "I cannot turn Vim on",
        // from someone looking in every reasonable place.
        entry_when(
            Requires::NavBack,
            Action::NavBack,
            &t!("menu.view.back"),
            chord(Action::NavBack),
        ),
        entry_when(
            Requires::NavForward,
            Action::NavForward,
            &t!("menu.view.forward"),
            chord(Action::NavForward),
        ),
        project_entry(
            Action::SwitchEditor,
            &t!("menu.view.switch-editor"),
            chord(Action::SwitchEditor),
        ),
        Item::Separator,
        entry(Action::ToggleVim, &t!("menu.view.vim"), None),
        Item::Separator,
        project_entry(
            Action::ToggleTree,
            &t!("menu.view.toggle-tree"),
            chord(Action::ToggleTree),
        ),
        project_entry(
            Action::SplitEditor,
            &t!("menu.view.split"),
            chord(Action::SplitEditor),
        ),
        project_entry(
            Action::ToggleBoard,
            &t!("menu.view.board-beside"),
            chord(Action::ToggleBoard),
        ),
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
        entry(
            Action::ToggleDock,
            &t!("menu.view.panel-below"),
            chord(Action::ToggleDock),
        ),
    ]);
    // The dock's own list, so a tab added there appears here without anyone
    // remembering to — the same reason the panels above come from the
    // registry.
    view_items.extend(
        DockTab::ALL
            .into_iter()
            .map(|tab| entry(Action::ShowDock(tab), &tab.label(), None)),
    );

    vec![
        Menu {
            title: t!("menu.bar.file"),
            items: {
                let mut items = vec![
                    entry(
                        Action::ShowPanel("wizard"),
                        &t!("menu.file.new-project"),
                        None,
                    ),
                    entry(
                        Action::OpenProject,
                        &t!("menu.file.open-project"),
                        chord(Action::OpenProject),
                    ),
                    entry(
                        Action::CloneRepository,
                        &t!("menu.file.clone"),
                        chord(Action::CloneRepository),
                    ),
                    // Beside New and Open, because it is the third way to
                    // have something to work on: one already made, per chip.
                    Item::Submenu {
                        label: t!("menu.file.playground"),
                        items: {
                            let mut items: Vec<Item> = rusty_embed::PLAYGROUND_CHIPS
                                .into_iter()
                                .map(|chip| {
                                    entry(
                                        Action::OpenPlayground(chip),
                                        &chip_name(state, chip),
                                        None,
                                    )
                                })
                                .collect();
                            items.extend([
                                Item::Separator,
                                entry_when(
                                    Requires::Playground,
                                    Action::ResetPlayground,
                                    &t!("menu.file.playground-reset"),
                                    None,
                                ),
                                entry_when(
                                    Requires::Playground,
                                    Action::KeepPlayground,
                                    &t!("menu.file.playground-keep"),
                                    None,
                                ),
                            ]);
                            items
                        },
                    },
                ];
                let recents = state.app.recents.get_untracked();
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
                        label: t!("menu.file.open-recent"),
                        items: recent_items,
                    });
                }
                items.extend([
                    Item::Separator,
                    entry(
                        Action::OpenSettings,
                        &t!("menu.file.settings"),
                        chord(Action::OpenSettings),
                    ),
                    Item::Separator,
                    entry(Action::CloseWindow, &t!("menu.file.exit"), None),
                ]);
                items
            },
        },
        Menu {
            title: t!("menu.bar.edit"),
            items: vec![
                project_entry(
                    Action::Undo,
                    &t!("menu.edit.undo"),
                    Some("Ctrl+Z".to_string()),
                ),
                project_entry(
                    Action::Redo,
                    &t!("menu.edit.redo"),
                    Some("Ctrl+Y".to_string()),
                ),
                Item::Separator,
                project_entry(
                    Action::Cut,
                    &t!("menu.edit.cut"),
                    Some("Ctrl+X".to_string()),
                ),
                project_entry(
                    Action::Copy,
                    &t!("menu.edit.copy"),
                    Some("Ctrl+C".to_string()),
                ),
                project_entry(
                    Action::Paste,
                    &t!("menu.edit.paste"),
                    Some("Ctrl+V".to_string()),
                ),
                Item::Separator,
                project_entry(
                    Action::ShowPanel("search"),
                    &t!("menu.edit.search"),
                    chord(Action::ShowPanel("search")),
                ),
            ],
        },
        Menu {
            title: t!("menu.bar.project"),
            items: vec![
                // The verbs first, as a Build menu leads with Build: they are
                // what the menu is opened for.
                project_entry(
                    Action::Build,
                    &t!("menu.project.build"),
                    chord(Action::Build),
                ),
                project_entry(Action::Test, &t!("menu.project.test"), chord(Action::Test)),
                Item::Separator,
                project_entry(Action::Run, &t!("menu.project.run"), chord(Action::Run)),
                project_entry(
                    Action::Debug,
                    &t!("menu.project.debug"),
                    chord(Action::Debug),
                ),
                project_entry(Action::Stop, &t!("menu.project.stop"), chord(Action::Stop)),
                project_entry(
                    Action::Restart,
                    &t!("menu.project.restart"),
                    chord(Action::Restart),
                ),
                Item::Separator,
                project_entry(
                    Action::RefreshProject,
                    &t!("menu.project.recheck"),
                    chord(Action::RefreshProject),
                ),
                entry(
                    Action::RefreshToolchain,
                    &t!("menu.project.rescan-toolchain"),
                    None,
                ),
                entry(
                    Action::ReloadCatalog,
                    &t!("menu.project.reload-catalogue"),
                    None,
                ),
                Item::Separator,
                // The two directions people actually need, named as
                // directions rather than as tool names: nobody thinks
                // "I need cc", they think "I have this C driver".
                Item::Submenu {
                    label: t!("menu.project.c-interop"),
                    items: vec![
                        project_entry(
                            Action::ToggleComment,
                            &t!("menu.project.comment"),
                            chord(Action::ToggleComment),
                        ),
                        project_entry(
                            Action::Rename,
                            &t!("menu.project.rename"),
                            chord(Action::Rename),
                        ),
                        Item::Separator,
                        project_entry(
                            Action::ScaffoldC("rust-calls-c"),
                            &t!("menu.project.rust-calls-c"),
                            None,
                        ),
                        project_entry(
                            Action::ScaffoldC("c-calls-rust"),
                            &t!("menu.project.c-calls-rust"),
                            None,
                        ),
                    ],
                },
            ],
        },
        Menu {
            title: t!("menu.bar.view"),
            items: view_items,
        },
        Menu {
            title: t!("menu.bar.device"),
            items: vec![
                project_entry(
                    Action::Flash,
                    &t!("menu.device.flash"),
                    chord(Action::Flash),
                ),
                project_entry(
                    Action::FlashOnly,
                    &t!("menu.device.flash-only"),
                    chord(Action::FlashOnly),
                ),
                project_entry(
                    Action::Monitor,
                    &t!("menu.device.monitor"),
                    chord(Action::Monitor),
                ),
                Item::Separator,
                project_entry(Action::PickDevice, &t!("menu.device.pick"), None),
                entry(Action::ScanDevices, &t!("menu.device.rescan"), None),
                Item::Separator,
                project_entry(Action::ShowPanel("memory"), &t!("menu.device.memory"), None),
            ],
        },
        Menu {
            title: t!("menu.bar.help"),
            items: vec![
                // First in Help because it is the first question a fresh
                // install raises, and because the automatic check only
                // appears when something is missing — somebody who wants to
                // look anyway needs a way in.
                entry(
                    Action::CheckEnvironment,
                    &t!("menu.help.check-environment"),
                    None,
                ),
                // Beside it, the other "is this machine current" question.
                // The launch check asks on its own; this is for anyone who
                // wants the answer now, and it says it either way.
                entry(Action::CheckUpdates, &t!("menu.help.check-updates"), None),
                Item::Separator,
                entry(Action::OpenSettings, &t!("menu.help.shortcuts"), None),
                entry(
                    Action::ShowPanel("assistant"),
                    &t!("menu.help.assistant"),
                    None,
                ),
                Item::Separator,
                // Somewhere to send a bug. Without this the only route back
                // from a user is the one they invent, and most people invent
                // none — a workbench nobody can report a fault in gets
                // reported as "it did not work" or not at all.
                entry(Action::OpenUrl(ISSUES), &t!("menu.help.report"), None),
                entry(Action::OpenUrl(RELEASES), &t!("menu.help.releases"), None),
            ],
        },
    ]
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
            if state.app.session_running.get_untracked() {
                controller::stop_anything(state);
            }
        }
        Action::Restart => {
            if state.has_project_now() {
                controller::restart_simulation(state);
            }
        }
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
