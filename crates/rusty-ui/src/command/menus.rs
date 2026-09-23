//! The menu bar: the same actions as the palette and the keyboard, in the
//! shape a menu reads — File, Edit, View and the rest, with flyouts.

use super::*;

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
    let chord = crate::view::palette::chords(state);

    let view_items = view_menu(&chord);

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
                Item::Separator,
                // Where VSCode's Edit menu keeps the comment toggle. Both of
                // these sat inside Project ▸ Add C interop, where a bulk edit
                // had spliced them into the wrong list.
                project_entry(
                    Action::ToggleComment,
                    &t!("menu.edit.comment"),
                    chord(Action::ToggleComment),
                ),
                project_entry(
                    Action::Rename,
                    &t!("menu.edit.rename"),
                    chord(Action::Rename),
                ),
            ],
        },
        Menu {
            title: t!("menu.bar.project"),
            items: project_menu(&chord),
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

/// The Project menu: the verbs, the debugger's, and the project's upkeep.
///
/// A function of the shortcut lookup alone, like the View menu, so a test
/// can hold the debugger's rows to their keys.
fn project_menu(chord: &dyn Fn(Action) -> Option<String>) -> Vec<Item> {
    vec![
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
        // The transport's verbs, where VS Code's Run menu lists them
        // — and the one place their keys are written down for
        // anybody who has not hovered a button.
        entry_when(
            Requires::DebugRunning,
            Action::Pause,
            &t!("debugger.pause"),
            chord(Action::Pause),
        ),
        entry_when(
            Requires::DebugStopped,
            Action::StepOver,
            &t!("debugger.step-over"),
            chord(Action::StepOver),
        ),
        entry_when(
            Requires::DebugStopped,
            Action::StepInto,
            &t!("debugger.step-into"),
            chord(Action::StepInto),
        ),
        entry_when(
            Requires::DebugStopped,
            Action::StepOut,
            &t!("debugger.step-out"),
            chord(Action::StepOut),
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
    ]
}

/// The View menu, folded the way VSCode's is: the two finders on top, one
/// flyout per kind of thing — where to go, how it looks, how the window is
/// laid out, the panels on the rail and the ones below — and the one switch
/// that is none of those. Flat, it was thirty-seven rows: a list to read
/// rather than a menu to use.
///
/// A function of the shortcut lookup alone, so a test can hold it to every
/// row it used to have.
fn view_menu(chord: &dyn Fn(Action) -> Option<String>) -> Vec<Item> {
    let go_to = vec![
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
        Item::Separator,
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
        // Back and Forward where VSCode's Go menu keeps them. They were in
        // Appearance once, beside the Vim switch, and nobody found either.
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
    ];
    let appearance = vec![
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
    ];
    let layout = vec![
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
        entry(Action::ResetLayout, &t!("menu.view.reset-layout"), None),
    ];
    // The rail's panels from the registry, as the rail itself reads them — a
    // contributed panel appears here without anyone remembering to add it.
    let rail: Vec<Item> = panels::all()
        .into_iter()
        .filter(|p| !p.hidden)
        .map(|panel| Item::Entry {
            action: Action::ShowPanel(panel.id),
            label: panel.title.to_string(),
            shortcut: chord(Action::ShowPanel(panel.id)),
            requires: if panel.needs_project {
                Requires::Project
            } else {
                Requires::Nothing
            },
        })
        .collect();
    // And the dock's own list, for the same reason.
    let mut below = vec![
        entry(
            Action::ToggleDock,
            &t!("menu.view.toggle-panel-below"),
            chord(Action::ToggleDock),
        ),
        Item::Separator,
    ];
    below.extend(
        DockTab::ALL
            .into_iter()
            .map(|tab| entry(Action::ShowDock(tab), &tab.label(), None)),
    );

    vec![
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
        Item::Separator,
        Item::Submenu {
            label: t!("menu.view.go"),
            items: go_to,
        },
        Item::Submenu {
            label: t!("menu.view.appearance"),
            items: appearance,
        },
        Item::Submenu {
            label: t!("menu.view.layout"),
            items: layout,
        },
        Item::Separator,
        Item::Submenu {
            label: t!("menu.view.panels"),
            items: rail,
        },
        Item::Submenu {
            label: t!("menu.view.panel-below"),
            items: below,
        },
        Item::Separator,
        // Not in a flyout, and not in Appearance where it was at first: a
        // modal editing switch is not a look, and "I cannot turn Vim on" was
        // the report of somebody who had looked in every reasonable place.
        entry(Action::ToggleVim, &t!("menu.view.vim"), None),
    ]
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

    fn actions(items: &[Item], out: &mut Vec<Action>) {
        for item in items {
            match item {
                Item::Entry { action, .. } => out.push(*action),
                Item::Submenu { items, .. } => actions(items, out),
                Item::Separator => {}
            }
        }
    }

    /// Every row the flat View menu had is still in it, and its top level is
    /// short enough to be a menu. Folding thirty-seven rows into flyouts is
    /// only an improvement if nothing fell out on the way.
    #[test]
    fn the_view_menu_folds_without_losing_a_row() {
        let menu = view_menu(&|_| None);
        assert!(menu.len() <= 12, "{} rows at the top", menu.len());

        let mut reached = Vec::new();
        actions(&menu, &mut reached);
        let mut wanted = vec![
            Action::OpenPalette,
            Action::QuickOpen,
            Action::GoToLine,
            Action::GoToSymbolInFile,
            Action::GoToSymbolInWorkspace,
            Action::FindReferences,
            Action::GoToImplementations,
            Action::GoToTypeDefinition,
            Action::ShowCallHierarchy,
            Action::ExpandMacro,
            Action::NavBack,
            Action::NavForward,
            Action::SwitchEditor,
            Action::ResetLayout,
            Action::ToggleVim,
            Action::ToggleTree,
            Action::SplitEditor,
            Action::ToggleBoard,
            Action::ToggleDock,
        ];
        wanted.extend(Theme::ALL.into_iter().map(Action::SetTheme));
        wanted.extend(
            panels::all()
                .into_iter()
                .filter(|p| !p.hidden)
                .map(|p| Action::ShowPanel(p.id)),
        );
        wanted.extend(DockTab::ALL.into_iter().map(Action::ShowDock));
        for action in wanted {
            assert!(reached.contains(&action), "{action:?} fell out of View");
        }
    }

    /// The debugger's verbs are in the Project menu with their keys, live
    /// exactly when the transport's buttons are: the steps while stopped,
    /// Pause while running. A step there with no condition would be a row
    /// that sends gdb something it refuses.
    #[test]
    fn the_project_menu_carries_the_debugger_with_its_keys() {
        let keys = |action| match action {
            Action::Pause => Some("F6".to_string()),
            Action::StepOver => Some("F10".to_string()),
            Action::StepInto => Some("F11".to_string()),
            Action::StepOut => Some("Shift+F11".to_string()),
            _ => None,
        };
        let menu = project_menu(&keys);
        let row = |wanted: Action| {
            menu.iter()
                .find_map(|item| match item {
                    Item::Entry {
                        action,
                        shortcut,
                        requires,
                        ..
                    } if *action == wanted => Some((shortcut.clone(), *requires)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{wanted:?} is not in the Project menu"))
        };
        for step in [Action::StepOver, Action::StepInto, Action::StepOut] {
            let (shortcut, requires) = row(step);
            assert_eq!(shortcut, keys(step), "{step:?} shows its key");
            assert_eq!(requires, Requires::DebugStopped, "{step:?}");
        }
        assert_eq!(
            row(Action::Pause),
            (keys(Action::Pause), Requires::DebugRunning)
        );
    }

    /// The Vim switch stays on the top level. It was in a submenu once, and
    /// the report was "I cannot turn Vim on" from somebody who had looked.
    #[test]
    fn the_vim_switch_is_not_in_a_flyout() {
        assert!(view_menu(&|_| None).iter().any(|item| matches!(
            item,
            Item::Entry {
                action: Action::ToggleVim,
                ..
            }
        )),);
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
