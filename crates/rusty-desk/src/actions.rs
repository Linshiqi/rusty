//! Everything the window can be asked to do, by name.
//!
//! A button, a menu row, a key and the palette all name an action by its id
//! and land in [`App::act`](crate::app::App::act), so the four cannot drift
//! apart. The palette lists this table; a key finds its action here.

/// One action: its id, the catalogue key of its name, and its key when it
/// has one — VS Code's wherever VS Code has one.
pub struct Action {
    pub id: &'static str,
    pub title: &'static str,
    pub chord: Option<&'static str>,
    /// Whether the palette lists it. A few exist only for a button.
    pub listed: bool,
}

const fn action(id: &'static str, title: &'static str, chord: Option<&'static str>) -> Action {
    Action {
        id,
        title,
        chord,
        listed: true,
    }
}

pub const ALL: &[Action] = &[
    action("palette.open", "menu.view.palette", Some("Ctrl+Shift+P")),
    action("finder.open", "menu.view.quick-open", Some("Ctrl+P")),
    action("project.open", "menu.file.open-project", Some("Ctrl+O")),
    action("project.new", "menu.file.new-project", None),
    action("project.clone", "menu.file.clone", None),
    action("settings.open", "desk.settings", Some("Ctrl+,")),
    action("welcome.open", "desk.welcome", None),
    action("environment.open", "panel.toolchain", None),
    action("memory.open", "panel.memory", None),
    action("crates.open", "panel.crates", None),
    action("math.open", "panel.math", None),
    action("git.history", "git.history", None),
    action("git.fetch", "git.fetch", None),
    action("git.pull", "git.pull", None),
    action("git.push", "git.push", None),
    action("file.save", "desk.save", Some("Ctrl+S")),
    action("find.open", "desk.find-open", Some("Ctrl+F")),
    action("find.replace", "desk.find-replace-open", Some("Ctrl+H")),
    action("find.next", "desk.find-next-action", Some("F3")),
    action(
        "find.previous",
        "desk.find-previous-action",
        Some("Shift+F3"),
    ),
    action("editor.go-to-line", "menu.view.go-to-line", Some("Ctrl+G")),
    action("editor.show-hover", "desk.show-hover", None),
    action(
        "editor.definition",
        "context.editor-definition",
        Some("F12"),
    ),
    action(
        "editor.references",
        "menu.view.references",
        Some("Shift+F12"),
    ),
    action(
        "editor.implementations",
        "menu.view.implementations",
        Some("Ctrl+F12"),
    ),
    action(
        "editor.quick-fix",
        "context.editor-quick-fix",
        Some("Ctrl+."),
    ),
    action("editor.rename", "menu.edit.rename", Some("F2")),
    action("nav.back", "menu.view.back", Some("Alt+ArrowLeft")),
    action("nav.forward", "menu.view.forward", Some("Alt+ArrowRight")),
    action("problem.next", "desk.problem-next", Some("F8")),
    action(
        "problem.previous",
        "desk.problem-previous",
        Some("Shift+F8"),
    ),
    action("tab.close", "desk.close-tab", Some("Ctrl+W")),
    action("dock.left", "desk.dock-left", Some("Ctrl+B")),
    action("dock.bottom", "desk.dock-bottom", Some("Ctrl+J")),
    action("dock.right", "desk.dock-right", Some("Ctrl+Alt+B")),
    action("view.zoom-in", "desk.zoom-in", Some("Ctrl+=")),
    action("view.zoom-out", "desk.zoom-out", Some("Ctrl+-")),
    action("view.zoom-reset", "desk.zoom-reset", Some("Ctrl+0")),
    action("theme.system", "menu.view.theme-system", None),
    action("theme.dark", "menu.view.theme-dark", None),
    action("theme.light", "menu.view.theme-light", None),
    action("run.run", "desk.run", Some("Ctrl+F5")),
    action("run.stop", "toolbar.stop", Some("Shift+F5")),
    action("run.build", "toolbar.build", Some("Ctrl+Shift+B")),
    action("run.test", "menu.project.test", None),
    action("run.flash", "toolbar.flash", Some("Ctrl+U")),
    action("run.monitor", "menu.device.monitor", Some("Ctrl+Shift+M")),
    action("run.debug", "toolbar.debug", Some("F5")),
    action("debug.pause", "debugger.pause", Some("F6")),
    action("debug.over", "debugger.step-over", Some("F10")),
    action("debug.into", "debugger.step-into", Some("F11")),
    action("debug.out", "debugger.step-out", Some("Shift+F11")),
    action("app.exit", "desk.exit", None),
    // Reached from a button, not looked for by name.
    Action {
        id: "run.rescan",
        title: "device.rescan",
        chord: None,
        listed: false,
    },
    Action {
        id: "run.choose-target",
        title: "device.choose",
        chord: None,
        listed: false,
    },
    Action {
        id: "project.menu",
        title: "menu.file.open-recent",
        chord: None,
        listed: false,
    },
    Action {
        id: "app.menu",
        title: "desk.app-menu",
        chord: None,
        listed: false,
    },
    Action {
        id: "problems.show",
        title: "dock.tab.problems",
        chord: None,
        listed: false,
    },
    Action {
        id: "notes.history",
        title: "desk.app-menu",
        chord: None,
        listed: false,
    },
    Action {
        id: "git.branches",
        title: "panel.git",
        chord: None,
        listed: false,
    },
];

/// The action a chord is bound to. F1 is the palette as well, as in VS Code.
pub fn bound_to(chord: &str) -> Option<&'static Action> {
    if chord == "F1" {
        return ALL.iter().find(|a| a.id == "palette.open");
    }
    ALL.iter().find(|a| a.chord == Some(chord))
}

pub fn by_id(id: &str) -> Option<&'static Action> {
    ALL.iter().find(|a| a.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_chords_are_each_one_action() {
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                assert_ne!(a.id, b.id, "two actions called {}", a.id);
                if a.chord.is_some() {
                    assert_ne!(a.chord, b.chord, "{} and {} share a key", a.id, b.id);
                }
            }
        }
    }

    /// Every name is a key the catalogue has, or the palette shows a key.
    #[test]
    fn every_title_is_in_the_catalogue() {
        for action in ALL {
            assert!(
                rusty_i18n::translate(action.title).is_some(),
                "{} is named by {}, which the catalogue lacks",
                action.id,
                action.title
            );
        }
    }
}
