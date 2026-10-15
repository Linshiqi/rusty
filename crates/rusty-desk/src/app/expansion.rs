//! A macro's expansion, as rust-analyzer's "Expand macro recursively" shows
//! it: a document with no file behind it, read-only, painted as Rust, beside
//! the code.
//!
//! Its path is `rusty_edit::expansion_path` — the call's file and line, then
//! the macro — so two expansions of one macro at two calls are two tabs, and
//! the same call expanded again fronts the tab it has. Nothing that touches
//! the disk or a server takes it: `Servers::for_path` refuses it, so no
//! server hears of it; the strip in `workbench.toml` leaves it out; no
//! draft of it is kept; and saving, renaming and a breakpoint pass it by.

use std::rc::Rc;

use rusty_edit::text_file::{Charset, LineEnding};
use rusty_i18n::t;
use rusty_lsp::{MacroExpansion, ServerKind};

use super::{App, Tab};
use crate::{editor::Document, language};

impl App {
    /// The macro call at the caret, expanded all the way down, beside the
    /// code; nothing to expand says so.
    pub(super) fn expand_macro(self: &Rc<Self>) {
        let Some((path, line, col, draft)) = self.keyed().caret_question() else {
            return;
        };
        // rust-analyzer's own request, and a macro call is Rust's.
        let rust = language::of(&path).and_then(|l| l.server) == Some(ServerKind::RustAnalyzer);
        if !rust || !self.servers.serves(&path) {
            self.note_transient(0, rusty_i18n::lookup("misc.no-macro"));
            return;
        }
        let file = path.clone();
        self.servers.ask_always(
            &path,
            Some(draft),
            move |client, path| client.expand_macro(path, line, col),
            move |app, found| match found {
                Ok(Some(expansion)) => app.show_expansion(&file, line, expansion),
                Ok(None) => app.note_transient(0, rusty_i18n::lookup("misc.no-macro")),
                Err(reason) => app.note_transient(
                    1,
                    t!(
                        "desk.server-could-not",
                        server = ServerKind::RustAnalyzer.label(),
                        reason = reason
                    ),
                ),
            },
        );
    }

    /// What the macro called on `line` of `file` expands to, as a document
    /// beside the code, under a line saying what it is.
    fn show_expansion(self: &Rc<Self>, file: &str, line: u32, expansion: MacroExpansion) {
        let path = rusty_edit::expansion_path(file, line, &expansion.name);
        let heading = t!(
            "files.expansion-of",
            name = expansion.name.clone(),
            place = format!("{file}:{}", line + 1)
        );
        let text = format!("// {heading}\n\n{}\n", expansion.expansion.trim_end());
        // The same call expanded again, to something else since: the tab
        // it has, holding what it expands to now.
        let changed = self
            .editor
            .draft_of(&path)
            .is_some_and(|(held, ..)| held != text);
        if changed {
            self.editor.close(&path);
        }
        if !self.editor.holds(&path) {
            let mut document = Document::new(path.clone(), text, Charset::Utf8, LineEnding::Lf);
            document.read_only = true;
            self.editor.add(document);
        }
        self.open_beside(Tab::File(path));
    }

    /// Put `tab` beside the code — VS Code's `ViewColumn.Beside`, kept to two
    /// groups: in the group on the right, with the keys left in the code
    /// they were in, so the expansion is read beside the call. With the keys
    /// on the right already, it comes to the front there.
    fn open_beside(self: &Rc<Self>, tab: Tab) {
        let keys_on_right = {
            let state = self.state.borrow();
            state.split && state.focus == 1
        };
        if keys_on_right {
            self.open_tab(tab);
            return;
        }
        let shown_already =
            matches!(&tab, Tab::File(path) if self.shade_file().as_ref() == Some(path));
        {
            let mut state = self.state.borrow_mut();
            let at = match state.other.iter().position(|t| *t == tab) {
                Some(at) => at,
                None => {
                    state.other.push(tab);
                    state.other.len() - 1
                }
            };
            state.other_active = Some(at);
            if !state.split {
                (state.split, state.focus) = (true, 0);
            }
        }
        let scroll = if shown_already {
            self.editor.shade_scroll()
        } else {
            (0.0, 0.0)
        };
        self.editor.show_shade(self.shade_file().as_deref(), scroll);
        self.sync_tabs();
        self.strip_changed();
        if let Some(window) = self.window() {
            window.invoke_focus_code();
        }
    }
}
