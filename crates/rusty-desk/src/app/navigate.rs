//! Moving around the code by what it means: the definition of a name, its
//! references and implementations, back and forward through the jumps;
//! and changing it by what it means: a fix the server offers, a rename
//! across every file that uses it.
//!
//! The picker is the list a choice is made from and the field a new name
//! is typed in — one overlay with one set of keys, as for files and
//! commands.

use std::rc::Rc;

use rusty_edit::text_file::{self, TextFile};
use rusty_i18n::t;
use rusty_lsp::{CodeActions, Location, Place};

use super::{App, Tab};
use crate::{
    editor::Document,
    picker::{Mode, Pick},
    work,
};

/// Where the caret was or is: a file, a line and a column, from zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Spot {
    path: String,
    line: usize,
    col: usize,
}

/// The jumps made, to walk back through, and those walked back from.
#[derive(Default)]
pub(super) struct Jumps {
    back: Vec<Spot>,
    forward: Vec<Spot>,
}

/// How many jumps are remembered.
const JUMPS: usize = 50;

impl App {
    fn here(&self) -> Option<Spot> {
        let path = self.editor.current_path()?;
        let (line, col, ..) = self.editor.status()?;
        Some(Spot {
            path,
            line: line - 1,
            col: col - 1,
        })
    }

    /// Go to a place, remembering where the caret was for Alt+Left.
    pub(super) fn jump(self: &Rc<Self>, location: &Location) {
        if let Some(here) = self.here() {
            let mut state = self.state.borrow_mut();
            state.jumps.back.push(here);
            if state.jumps.back.len() > JUMPS {
                state.jumps.back.remove(0);
            }
            state.jumps.forward.clear();
        }
        self.arrive(location);
    }

    fn arrive(self: &Rc<Self>, location: &Location) {
        let at = Some((location.line as usize, location.col as usize));
        if location.external {
            self.open_outside(location.path.clone(), at);
        } else {
            self.open_file_at(location.path.clone(), at);
        }
    }

    /// Alt+Left and Alt+Right: back to where a jump left from, and on again.
    pub(super) fn walk(self: &Rc<Self>, back: bool) {
        let target = {
            let mut state = self.state.borrow_mut();
            let jumps = &mut state.jumps;
            if back {
                jumps.back.pop()
            } else {
                jumps.forward.pop()
            }
        };
        let Some(target) = target else {
            let key = if back {
                "misc.vim-no-back"
            } else {
                "misc.vim-no-forward"
            };
            self.note_transient(0, rusty_i18n::lookup(key));
            return;
        };
        if let Some(here) = self.here() {
            let mut state = self.state.borrow_mut();
            if back {
                state.jumps.forward.push(here);
            } else {
                state.jumps.back.push(here);
            }
        }
        let external = target.path.contains(':') || target.path.starts_with('/');
        self.arrive(&Location {
            path: target.path,
            line: target.line as u32,
            col: target.col as u32,
            external,
        });
    }

    /// A place a tool printed: a file of the project by its relative path,
    /// or one outside it by its absolute one, opened to be read.
    pub(super) fn open_place(self: &Rc<Self>, path: String, at: (usize, usize)) {
        if !std::path::Path::new(&path).is_absolute() {
            self.open_file_at(path, Some(at));
        } else if let Some(inside) = self.root().and_then(|root| within(&root, &path)) {
            self.open_file_at(inside, Some(at));
        } else {
            self.open_outside(path, Some(at));
        }
    }

    /// A file outside the project — the definition of something in a
    /// dependency, or in a toolchain's header — opened to be read. The
    /// server named it, so it is a file the server has read; nothing in
    /// it is ever written.
    fn open_outside(self: &Rc<Self>, path: String, at: Option<(usize, usize)>) {
        let go = move |app: &Rc<App>| {
            if let Some((line, col)) = at {
                app.editor.go_to(line + 1, Some(col + 1));
            }
        };
        if self.editor.holds(&path) {
            self.open_tab(Tab::File(path));
            go(self);
            return;
        }
        work::spawn(
            move || {
                let read = std::fs::read(&path)
                    .map(|bytes| text_file::decode(&bytes))
                    .map_err(|error| error.to_string());
                (path, read)
            },
            move |app, (path, read)| match read {
                Ok(TextFile::Text {
                    text,
                    charset,
                    ending,
                }) => {
                    app.editor
                        .show_read_only(Document::new(path.clone(), text, charset, ending));
                    app.open_tab(Tab::File(path));
                    go(app);
                }
                Ok(_) => app.note(
                    1,
                    t!("desk.open-failed", path = path, reason = t!("desk.binary")),
                ),
                Err(reason) => app.note(2, t!("desk.open-failed", path = path, reason = reason)),
            },
        );
    }

    /// F12, and Ctrl+click: the definition of the name at the caret.
    pub(super) fn go_to_definition(self: &Rc<Self>) {
        let Some((path, line, col, draft)) = self.editor.caret_question() else {
            return;
        };
        self.servers.ask(
            &path,
            Some(draft),
            move |client, path| client.definition(path, line, col),
            |app, found| match found {
                Some(location) => app.jump(&location),
                None => app.note_transient(0, t!("desk.nothing-found")),
            },
        );
    }

    /// Shift+F12 and Ctrl+F12: the references to the name at the caret, or
    /// what implements it, in the picker. One implementation is a jump.
    pub(super) fn find_places(self: &Rc<Self>, implementations: bool) {
        let Some((path, line, col, draft)) = self.editor.caret_question() else {
            return;
        };
        let name = self.editor.caret_word().unwrap_or_default();
        self.servers.ask(
            &path,
            Some(draft),
            move |client, path| {
                if implementations {
                    client.implementations(path, line, col)
                } else {
                    client.references(path, line, col)
                }
            },
            move |app, places| app.show_places(implementations, &name, places),
        );
    }

    fn show_places(self: &Rc<Self>, implementations: bool, name: &str, mut places: Vec<Place>) {
        if places.is_empty() {
            self.note_transient(0, t!("desk.nothing-found"));
            return;
        }
        if implementations && places.len() == 1 {
            self.jump(&places[0].location);
            return;
        }
        places.sort_by(|a, b| {
            (&a.location.path, a.location.line).cmp(&(&b.location.path, b.location.line))
        });
        let rows: Vec<(String, String, Pick)> = places
            .into_iter()
            .map(|place| {
                let location = place.location;
                let short = location
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&location.path)
                    .to_string();
                (
                    place.text.trim().to_string(),
                    format!("{short}:{}", location.line + 1),
                    Pick::Place(location),
                )
            })
            .collect();
        let heading = if implementations {
            t!("places.implementations", name = name)
        } else {
            t!("places.references", name = name)
        };
        self.open_list(Mode::Places, heading, rows);
    }

    /// Ctrl+.: what the server offers to do at the caret, in the picker.
    pub(super) fn quick_fix(self: &Rc<Self>) {
        let Some((path, line, col, draft)) = self.editor.caret_question() else {
            return;
        };
        let Some(version) = self.editor.version_now() else {
            return;
        };
        self.servers.ask(
            &path,
            Some(draft),
            move |client, path| client.code_actions(path, line, col),
            move |app, actions| app.show_fixes(version, actions),
        );
    }

    fn show_fixes(self: &Rc<Self>, version: u64, actions: CodeActions) {
        if actions.fixes.is_empty() {
            self.note_transient(0, rusty_i18n::lookup("misc.no-quick-fixes"));
            return;
        }
        let rows: Vec<(String, String, Pick)> = actions
            .fixes
            .iter()
            .enumerate()
            .map(|(index, fix)| {
                let detail = if fix.elsewhere.is_empty() {
                    String::new()
                } else {
                    t!(
                        "misc.edited-elsewhere",
                        count = fix.elsewhere.len().to_string()
                    )
                };
                (fix.title.clone(), detail, Pick::Fix(index))
            })
            .collect();
        self.state.borrow_mut().fixes = Some((version, actions));
        self.open_list(
            Mode::Fixes,
            rusty_i18n::lookup("context.editor-quick-fix"),
            rows,
        );
    }

    /// The fix picked from the list.
    pub(super) fn apply_fix(self: &Rc<Self>, index: usize) {
        let Some((version, actions)) = self.state.borrow_mut().fixes.take() else {
            return;
        };
        self.apply_fix_from(version, actions, index);
    }

    /// A fix the server offered — in Ctrl+.'s list or on the hover card —
    /// applied: its edits to this file, then, when it changes others, the
    /// server writes those and the open ones are read again. A file it
    /// changes with an unsaved draft refuses it whole: the draft's next save
    /// would put its stale bytes back over the fix.
    pub(super) fn apply_fix_from(
        self: &Rc<Self>,
        version: u64,
        actions: CodeActions,
        index: usize,
    ) {
        let Some(fix) = actions.fixes.get(index).cloned() else {
            return;
        };
        if let Some(draft) = fix.elsewhere.iter().find(|path| self.editor.is_dirty(path)) {
            self.note(1, t!("misc.save-first", path = draft.clone()));
            return;
        }
        let Some(path) = self.editor.current_path() else {
            return;
        };
        self.editor.apply_server_edits(version, fix.edits.clone());
        if fix.elsewhere.is_empty() {
            return;
        }
        let reply = actions.reply;
        self.servers.ask_result(
            &path,
            move |client, path| client.apply_action_elsewhere(path, reply, index as u32),
            |app, changed| match changed {
                Ok(changed) => app.reread(changed),
                Err(reason) => app.note(2, reason),
            },
        );
    }

    /// F2: a new name for the symbol at the caret, typed in the picker.
    pub(super) fn rename_prompt(self: &Rc<Self>) {
        let Some((path, line, col, _)) = self.editor.caret_question() else {
            return;
        };
        let Some(name) = self.editor.caret_word() else {
            return;
        };
        self.state.borrow_mut().renaming = Some((path, line, col, name.clone()));
        self.open_picker_with(Mode::Rename, &name);
    }

    /// The picker's row while a new name is typed.
    pub(super) fn rename_rows(&self, query: &str) -> Vec<(String, String, Pick)> {
        let name = query.trim();
        let unchanged = self
            .state
            .borrow()
            .renaming
            .as_ref()
            .is_some_and(|(.., old)| old == name);
        if name.is_empty() || unchanged {
            return Vec::new();
        }
        vec![(
            t!("desk.rename-to", name = name),
            rusty_i18n::lookup("misc.rename-scope"),
            Pick::Rename(name.to_string()),
        )]
    }

    /// Rename across the project. Everything unsaved is saved first: the
    /// server writes the other files on disk, and reads them there, so a
    /// draft would both be missed by it and, at its next save, write its
    /// stale bytes back over the rename.
    pub(super) fn rename_to(self: &Rc<Self>, name: String) {
        let Some((path, line, col, _)) = self.state.borrow_mut().renaming.take() else {
            return;
        };
        let Some(root) = self.root() else { return };
        let unsaved = self.editor.unsaved();
        work::spawn(
            move || {
                let mut written = Vec::new();
                for (path, text, charset, ending) in unsaved {
                    if let Err(error) = text_file::save_text(&root, &path, &text, charset, ending) {
                        return Err((path, error.to_string()));
                    }
                    written.push((path, text));
                }
                Ok(written)
            },
            move |app, saved| match saved {
                Ok(written) => {
                    for (path, text) in written {
                        app.editor.saved(&path, &text);
                    }
                    app.sync_tabs();
                    app.servers.ask_result(
                        &path,
                        move |client, path| client.rename(path, line, col, &name),
                        |app, changed| match changed {
                            Ok(changed) if changed.is_empty() => {
                                app.note_transient(0, rusty_i18n::lookup("misc.rename-nothing"));
                            }
                            Ok(changed) => {
                                let count = changed.len().to_string();
                                app.note_transient(0, t!("misc.renamed-in", count = count));
                                app.reread(changed);
                            }
                            Err(reason) => app.note(2, reason),
                        },
                    );
                }
                Err((path, reason)) => {
                    app.note(2, t!("desk.save-failed", path = path, reason = reason))
                }
            },
        );
    }

    /// Files something wrote on disk, read again into the documents open
    /// on them. The client names what it wrote by absolute path; a tab
    /// holds its file relative to the project.
    fn reread(self: &Rc<Self>, paths: Vec<String>) {
        let Some(root) = self.root() else { return };
        let open: Vec<String> = paths
            .iter()
            .map(|path| within(&root, path).unwrap_or_else(|| path.clone()))
            .filter(|p| self.editor.holds(p))
            .collect();
        work::spawn(
            move || {
                open.into_iter()
                    .filter_map(|path| match text_file::read_text(&root, &path) {
                        Ok(TextFile::Text { text, .. }) => Some((path, text)),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            },
            |app, read| {
                for (path, text) in read {
                    app.editor.reload(&path, text);
                }
                app.sync_tabs();
            },
        );
    }
}

/// `path` as the project at `root` names it — relative, `/` between —
/// when it is inside it. A drive letter's case is not a difference.
pub(super) fn within(root: &std::path::Path, path: &str) -> Option<String> {
    let plain = |text: &str| {
        let text = text.replace('\\', "/");
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(drive), Some(':')) => format!("{}{}", drive.to_ascii_lowercase(), &text[1..]),
            _ => text,
        }
    };
    let root = plain(&root.to_string_lossy());
    let path = plain(path);
    let rest = path.strip_prefix(root.trim_end_matches('/'))?;
    rest.strip_prefix('/').map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_file_is_named_as_its_tab_names_it() {
        let root = std::path::Path::new(r"E:\work\proj");
        assert_eq!(
            within(root, r"e:\work\proj\src\main.rs").as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(
            within(root, "E:/work/proj/core/lib.rs").as_deref(),
            Some("core/lib.rs")
        );
        assert_eq!(within(root, r"E:\work\project2\x.rs"), None);
        assert_eq!(within(root, "/elsewhere/x.rs"), None);
    }
}
