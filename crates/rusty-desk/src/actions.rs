//! Everything the window can be asked to do, by name.
//!
//! A button, a menu row, a key and the palette all name an action by its id
//! and land in [`App::act`](crate::app::App::act), so the four cannot drift
//! apart. The palette lists this table; a key finds its action here — not
//! in the table's own `chord`, which is only the default, but in the
//! [`Keymap`], which is the table with what somebody rebound over it.

use std::collections::BTreeMap;

use crate::keys::Stroke;

/// One action: its id, the catalogue key of its name, and its key when it
/// has one — VS Code's wherever VS Code has one.
pub struct Action {
    pub id: &'static str,
    pub title: &'static str,
    /// The key it has until somebody rebinds it: one stroke, or two with a
    /// space between (`Ctrl+K Ctrl+S`).
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
    action(
        "project.open-new-window",
        "menu.file.open-project-new-window",
        None,
    ),
    action("project.new", "menu.file.new-project", None),
    action("project.clone", "menu.file.clone", None),
    action("window.new", "menu.file.new-window", Some("Ctrl+Shift+N")),
    // In the folder the tree has selected, named in the tree, as VS Code's
    // explorer buttons make them.
    action("tree.new-file", "context.tree-new-file", None),
    action("tree.new-folder", "context.tree-new-folder", None),
    action("settings.open", "desk.settings", Some("Ctrl+,")),
    action("keys.open", "shortcuts.title", Some("Ctrl+K Ctrl+S")),
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
    // The finder opened with `@` and with `#`, as VS Code's Go to Symbol in
    // Editor and in Workspace open its own.
    action(
        "finder.symbols",
        "menu.view.symbols-file",
        Some("Ctrl+Shift+O"),
    ),
    action(
        "finder.workspace-symbols",
        "menu.view.symbols-workspace",
        Some("Ctrl+T"),
    ),
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
    // No keys of their own, as in VS Code — whose Shift+Alt+H for the call
    // hierarchy is an Alt-letter chord, which this window leaves to AltGr.
    action("editor.type-definition", "menu.view.type-definition", None),
    action("editor.call-hierarchy", "menu.view.call-hierarchy", None),
    action("editor.expand-macro", "menu.view.expand-macro", None),
    action(
        "editor.quick-fix",
        "context.editor-quick-fix",
        Some("Ctrl+."),
    ),
    action("editor.rename", "menu.edit.rename", Some("F2")),
    action("editor.fold", "desk.fold-region", Some("Ctrl+Shift+[")),
    action("editor.unfold", "desk.unfold-region", Some("Ctrl+Shift+]")),
    action(
        "editor.fold-all",
        "context.editor-fold-all",
        Some("Ctrl+K Ctrl+0"),
    ),
    action("editor.vim", "menu.view.vim", None),
    action("editor.split", "desk.split", Some("Ctrl+\\")),
    // No key of its own, as in VS Code.
    action("editor.detach", "desk.detach", None),
    action("update.check", "menu.help.check-updates", None),
    action(
        "editor.unfold-all",
        "context.editor-unfold-all",
        Some("Ctrl+K Ctrl+J"),
    ),
    action("nav.back", "menu.view.back", Some("Alt+ArrowLeft")),
    action("nav.forward", "menu.view.forward", Some("Alt+ArrowRight")),
    action("problem.next", "desk.problem-next", Some("F8")),
    action(
        "problem.previous",
        "desk.problem-previous",
        Some("Shift+F8"),
    ),
    action("tab.close", "desk.close-tab", Some("Ctrl+W")),
    // A tab's right-click menu (`app/tab_menu.rs`); from the palette, for
    // the tab in front.
    action("tab.close-others", "strip.close-others", None),
    action("tab.close-right", "strip.close-right", None),
    action("tab.close-saved", "strip.close-saved", None),
    action("tab.close-all", "strip.close-all", Some("Ctrl+K Ctrl+W")),
    // The tree's words for the same verbs (`app/tree.rs`), and its way of
    // doing them.
    action("tab.copy-path", "context.tree-copy-path", None),
    action(
        "tab.copy-relative-path",
        "context.tree-copy-relative-path",
        None,
    ),
    action("tab.reveal", crate::reveal::label_key(), None),
    action("dock.left", "desk.dock-left", Some("Ctrl+B")),
    action("dock.bottom", "desk.dock-bottom", Some("Ctrl+J")),
    action("dock.right", "desk.dock-right", Some("Ctrl+Alt+B")),
    action("view.zoom-in", "desk.zoom-in", Some("Ctrl+=")),
    action("view.zoom-out", "desk.zoom-out", Some("Ctrl+-")),
    action("view.zoom-reset", "desk.zoom-reset", Some("Ctrl+0")),
    // Ctrl and the wheel size the text under the pointer; these put each
    // back, keyless as VS Code's own resets are.
    action("editor.zoom-reset", "desk.editor-zoom-reset", None),
    action("page.zoom-reset", "desk.page-zoom-reset", None),
    action("terminal.zoom-reset", "desk.terminal-zoom-reset", None),
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
    // A tab's file opened in the group to its right; the palette has Split
    // editor for the tab in front.
    Action {
        id: "tab.split-right",
        title: "strip.split-right",
        chord: None,
        listed: false,
    },
    // An update's steps, reached from Settings, a note and the status bar.
    Action {
        id: "update.download",
        title: "update.download",
        chord: None,
        listed: false,
    },
    Action {
        id: "update.restart",
        title: "update.restart",
        chord: None,
        listed: false,
    },
    Action {
        id: "update.skip",
        title: "update.skip",
        chord: None,
        listed: false,
    },
    Action {
        id: "update.releases",
        title: "update.releases",
        chord: None,
        listed: false,
    },
];

pub fn by_id(id: &str) -> Option<&'static Action> {
    ALL.iter().find(|a| a.id == id)
}

/// The most strokes a chord takes: VS Code's `Ctrl+K Ctrl+S`.
pub const STROKES: usize = 2;

/// A chord written any way at all — the old window's `Shift+Alt` order, a
/// hand-edited file's `ctrl+k` — as this window spells it, or `None` for
/// one that names no key or has too many strokes.
pub fn normal(chord: &str) -> Option<String> {
    let strokes: Vec<String> = chord
        .split_whitespace()
        .map(|stroke| Stroke::parse(stroke).map(|s| s.to_string()))
        .collect::<Option<_>>()?;
    (1..=STROKES)
        .contains(&strokes.len())
        .then(|| strokes.join(" "))
}

/// Why a chord cannot be a shortcut: every stroke of one has to be a key
/// nothing types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A character, Enter or an arrow with no Ctrl: it is typing, and a
    /// field would take it before any shortcut could.
    Types,
    /// Alt with a character: on a layout with AltGr — which is Ctrl+Alt to
    /// the system — that is how a character is typed.
    AltGr,
}

/// Whether `chord` (spelled as [`normal`] spells it) can be a shortcut. A
/// function key is one on its own, as F5 runs and F2 renames; anything
/// else needs Ctrl, or Alt with a named key — Alt+ArrowLeft is Back — and
/// never Alt with a character.
pub fn refusal(chord: &str) -> Option<Refusal> {
    chord.split_whitespace().find_map(|text| {
        let stroke = Stroke::parse(text)?;
        if stroke.alt && stroke.is_character() {
            Some(Refusal::AltGr)
        } else if stroke.ctrl || stroke.alt || stroke.is_function() {
            None
        } else {
            Some(Refusal::Types)
        }
    })
}

/// The keys in force: the table's, with what somebody rebound over them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    /// Action id → chord, for the actions whose key is not the table's.
    rebound: BTreeMap<&'static str, String>,
}

impl Keymap {
    /// The keys `workbench.toml`'s `keybinds` asks for. What names no
    /// action this window lists, a chord that names no key, and one no
    /// shortcut may be are left out — read, never trusted, and the file
    /// keeps them for whichever rusty wrote them.
    pub fn read(saved: &BTreeMap<String, String>) -> Keymap {
        let mut map = Keymap::default();
        for (id, chord) in saved {
            if let (Some(action), Some(chord)) = (by_id(id).filter(|a| a.listed), normal(chord))
                && refusal(&chord).is_none()
            {
                map.set(action.id, chord);
            }
        }
        map
    }

    /// The key `id` has now.
    pub fn chord(&self, id: &str) -> Option<&str> {
        match self.rebound.get(id) {
            Some(chord) => Some(chord),
            None => by_id(id).and_then(|a| a.chord),
        }
    }

    /// Whether `id`'s key is somebody's rather than the table's.
    pub fn is_rebound(&self, id: &str) -> bool {
        self.rebound.contains_key(id)
    }

    /// Give `id` this chord. The table's own chord is no rebinding at all,
    /// so a key set back to its default reads as never changed.
    pub fn set(&mut self, id: &'static str, chord: String) {
        if by_id(id).and_then(|a| a.chord) == Some(chord.as_str()) {
            self.rebound.remove(id);
        } else {
            self.rebound.insert(id, chord);
        }
    }

    /// `id` back to the table's key.
    pub fn reset(&mut self, id: &str) {
        self.rebound.remove(id);
    }

    /// Every key back to the table's.
    pub fn reset_all(&mut self) {
        self.rebound.clear();
    }

    /// Whether anything is rebound.
    pub fn any_rebound(&self) -> bool {
        !self.rebound.is_empty()
    }

    /// Every listed action with the key it has now.
    fn in_force(&self) -> impl Iterator<Item = (&'static Action, &str)> {
        ALL.iter()
            .filter(|a| a.listed)
            .filter_map(|a| Some((a, self.chord(a.id)?)))
    }

    /// The action `chord` runs. Two actions on one key — which the page
    /// says on both rows — run the one somebody bound, then the table's
    /// first. F1 is the palette as well, as in VS Code, while nothing else
    /// has it.
    pub fn bound_to(&self, chord: &str) -> Option<&'static Action> {
        let on: Vec<&'static Action> = self
            .in_force()
            .filter(|(_, c)| *c == chord)
            .map(|(a, _)| a)
            .collect();
        on.iter()
            .find(|a| self.is_rebound(a.id))
            .or_else(|| on.first())
            .copied()
            .or_else(|| (chord == "F1").then(|| by_id("palette.open")).flatten())
    }

    /// Whether `stroke` begins a two-stroke chord, so the key after it is
    /// the rest of the chord rather than a key of its own. As in VS Code, a
    /// stroke that begins a chord waits for the rest even when it is a
    /// chord of its own as well — which the page says on both rows.
    pub fn starts(&self, stroke: &str) -> bool {
        self.in_force().any(|(_, chord)| {
            chord
                .split_once(' ')
                .is_some_and(|(first, _)| first == stroke)
        })
    }

    /// The other actions `id`'s key collides with: the same chord, or one
    /// whose first stroke is the other's whole chord — which can then never
    /// be reached.
    pub fn sharing(&self, id: &str) -> Vec<&'static Action> {
        let Some(mine) = self.chord(id) else {
            return Vec::new();
        };
        let first = |chord: &str| chord.split(' ').next().unwrap_or(chord).to_string();
        self.in_force()
            .filter(|(a, _)| a.id != id)
            .filter(|(_, theirs)| {
                *theirs == mine
                    || (mine.contains(' ') && !theirs.contains(' ') && first(mine) == *theirs)
                    || (theirs.contains(' ') && !mine.contains(' ') && first(theirs) == mine)
            })
            .map(|(a, _)| a)
            .collect()
    }
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
        let keys = Keymap::default();
        for action in ALL {
            assert!(
                keys.sharing(action.id).is_empty(),
                "{} collides with {:?} out of the box",
                action.id,
                keys.sharing(action.id)
                    .iter()
                    .map(|a| a.id)
                    .collect::<Vec<_>>()
            );
        }
    }

    /// The table spells its keys as a press makes them, or a default would
    /// be a key no press ever matches.
    #[test]
    fn every_default_is_spelled_as_a_press_spells_it() {
        for action in ALL {
            if let Some(chord) = action.chord {
                assert_eq!(normal(chord).as_deref(), Some(chord), "{}", action.id);
            }
        }
    }

    #[test]
    fn a_shortcut_is_a_key_nothing_types() {
        assert_eq!(refusal("Ctrl+S"), None);
        assert_eq!(refusal("F5"), None);
        assert_eq!(refusal("Shift+F11"), None);
        assert_eq!(refusal("Alt+ArrowLeft"), None);
        assert_eq!(refusal("Ctrl+K Ctrl+S"), None);
        assert_eq!(refusal("A"), Some(Refusal::Types));
        assert_eq!(refusal("Shift+A"), Some(Refusal::Types));
        assert_eq!(refusal("Enter"), Some(Refusal::Types));
        assert_eq!(
            refusal("Ctrl+K V"),
            Some(Refusal::Types),
            "the second stroke too"
        );
        assert_eq!(refusal("Alt+A"), Some(Refusal::AltGr));
        assert_eq!(
            refusal("Ctrl+Alt+Q"),
            Some(Refusal::AltGr),
            "AltGr is Ctrl+Alt"
        );
        assert_eq!(normal("Ctrl+K Ctrl+S Ctrl+X"), None, "two strokes at most");
    }

    /// What the file says is read, never trusted: an id this window has no
    /// action for and a key that is no shortcut are left out, and the old
    /// window's spelling reads as this one's.
    #[test]
    fn the_file_is_read_and_not_trusted() {
        let saved: BTreeMap<String, String> = [
            ("file.save", "shift+ctrl+alt+up"),
            ("quick.symbol", "Ctrl+Shift+O"),
            ("run.build", "B"),
            ("run.flash", "Ctrl+Banana"),
            ("run.rescan", "Ctrl+R"),
        ]
        .into_iter()
        .map(|(id, chord)| (id.to_string(), chord.to_string()))
        .collect();
        let keys = Keymap::read(&saved);
        assert_eq!(keys.chord("file.save"), Some("Ctrl+Alt+Shift+ArrowUp"));
        assert_eq!(keys.chord("run.build"), Some("Ctrl+Shift+B"), "B types");
        assert_eq!(keys.chord("run.flash"), Some("Ctrl+U"), "no such key");
        assert!(!keys.is_rebound("run.rescan"), "a button's, not listed");
        assert!(
            keys.bound_to("Ctrl+S").is_none(),
            "the old key does nothing"
        );
        assert_eq!(
            keys.bound_to("Ctrl+Alt+Shift+ArrowUp").map(|a| a.id),
            Some("file.save")
        );
    }

    #[test]
    fn a_key_set_back_to_its_default_is_no_rebinding() {
        let mut keys = Keymap::default();
        keys.set("file.save", "Ctrl+Alt+S".into());
        assert!(keys.is_rebound("file.save"));
        keys.set("file.save", "Ctrl+S".into());
        assert!(!keys.is_rebound("file.save"));
        assert!(!keys.any_rebound());
    }

    /// Two actions on one key run the one somebody chose; each says the
    /// other on its row.
    #[test]
    fn a_key_given_twice_runs_the_rebound_one_and_says_so() {
        let mut keys = Keymap::default();
        keys.set("run.build", "Ctrl+B".into());
        assert_eq!(keys.bound_to("Ctrl+B").map(|a| a.id), Some("run.build"));
        let ids = |list: Vec<&Action>| list.iter().map(|a| a.id).collect::<Vec<_>>();
        assert_eq!(ids(keys.sharing("run.build")), ["dock.left"]);
        assert_eq!(ids(keys.sharing("dock.left")), ["run.build"]);
        keys.reset("run.build");
        assert_eq!(keys.bound_to("Ctrl+B").map(|a| a.id), Some("dock.left"));
    }

    /// Ctrl+K begins the chords that have it first; bound alone as well,
    /// it could never run, and both rows say so.
    #[test]
    fn a_stroke_that_begins_a_chord_waits_for_the_rest() {
        let mut keys = Keymap::default();
        assert!(keys.starts("Ctrl+K"));
        assert!(!keys.starts("Ctrl+S"));
        assert_eq!(
            keys.bound_to("Ctrl+K Ctrl+S").map(|a| a.id),
            Some("keys.open")
        );
        keys.set("run.build", "Ctrl+K".into());
        let sharing: Vec<_> = keys.sharing("run.build").iter().map(|a| a.id).collect();
        assert!(sharing.contains(&"keys.open"), "{sharing:?}");
        assert!(
            keys.sharing("keys.open")
                .iter()
                .any(|a| a.id == "run.build")
        );
    }

    #[test]
    fn f1_is_the_palette_while_nothing_else_has_it() {
        let mut keys = Keymap::default();
        assert_eq!(keys.bound_to("F1").map(|a| a.id), Some("palette.open"));
        keys.set("run.build", "F1".into());
        assert_eq!(keys.bound_to("F1").map(|a| a.id), Some("run.build"));
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
