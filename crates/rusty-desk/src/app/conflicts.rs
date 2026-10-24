//! A file's merge conflicts, from the editor's side: the bar over the code
//! that counts them and walks them, and the verbs that take every block of
//! the file one way at once. One block at a time is the lens on its first
//! marker (`editor/rows.rs`); the file as a whole — a side's version
//! checked out — is the Git panel's (`git.rs`).

use std::rc::Rc;

use rusty_edit::conflict::Choice;
use rusty_i18n::t;
use slint::ComponentHandle;

use super::{App, Tab};
use crate::Shell;

impl App {
    /// The bar over the code, as the file in front stands: its blocks of
    /// markers, the one the caret is in, and whether git still has it
    /// unmerged — what, with no blocks left, makes the bar say so and
    /// offer to mark it resolved.
    pub(super) fn sync_conflict_bar(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let path = match self.active_tab() {
            Some(Tab::File(path)) => Some(path),
            _ => None,
        };
        let (count, at) = if path.is_some() {
            self.editor.conflicts_here()
        } else {
            (0, 0)
        };
        let unmerged = path.as_ref().is_some_and(|path| {
            self.state.borrow().git.status.as_ref().is_some_and(|s| {
                s.entries
                    .iter()
                    .any(|e| &e.path == path && e.is_conflicted())
            })
        });
        let label = if count == 0 {
            t!("desk.conflicts-none-left")
        } else if at == 0 {
            t!("desk.conflicts-count", count = count)
        } else {
            t!("desk.conflict-of", at = at, count = count)
        };
        if shell.get_conflict_count() != count as i32 {
            shell.set_conflict_count(count as i32);
        }
        if shell.get_conflict_label().as_str() != label {
            shell.set_conflict_label(label.into());
        }
        if shell.get_conflict_unmerged() != unmerged {
            shell.set_conflict_unmerged(unmerged);
        }
    }

    /// The caret to the next conflict, or the one before — said when the
    /// file has none, rather than a key that seems to do nothing.
    pub(super) fn conflict_step(self: &Rc<Self>, forward: bool) {
        if !self.keyed().go_to_conflict(forward) {
            self.note_transient(0, t!("desk.conflicts-none"));
        }
    }

    /// Every block of the file in front taken one way, as one edit.
    pub(super) fn conflicts_take(self: &Rc<Self>, choice: Choice) {
        if self.keyed().resolve_all_conflicts(choice) == 0 {
            self.note_transient(0, t!("desk.conflicts-none"));
        }
    }
}
