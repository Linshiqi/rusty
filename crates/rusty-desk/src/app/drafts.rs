//! Drafts kept between runs — VS Code's hot exit, on the window's side:
//! when a draft is backed up, what a closing window or a project switch
//! keeps, and putting the kept drafts back. `editor/backup.rs` is the file
//! format and the thread that writes it.

use std::{path::PathBuf, rc::Rc, time::Duration};

use rusty_i18n::t;
use slint::TimerMode;

use super::{App, strip::Back};
use crate::work;

/// How long after the last keystroke a draft is backed up.
pub(super) const BACKUP_AFTER: Duration = Duration::from_secs(1);
/// How long a closing window waits for its drafts to reach the disk.
pub(super) const KEEP_BUDGET: Duration = Duration::from_secs(5);

impl App {
    /// A draft was edited: back it up once typing has stopped for a moment.
    pub(super) fn back_up_soon(self: &Rc<Self>, path: &str) {
        let weak = Rc::downgrade(self);
        let owned = path.to_string();
        self.backup_timers
            .borrow_mut()
            .entry(path.to_string())
            .or_default()
            .start(TimerMode::SingleShot, BACKUP_AFTER, move || {
                if let Some(app) = weak.upgrade() {
                    app.back_up(&owned);
                }
            });
    }

    /// Keep `path`'s draft, or forget it where the text is the disk's.
    pub(super) fn back_up(&self, path: &str) {
        let Some(root) = self.root() else { return };
        match self.editor.backup_of(path) {
            Some(draft) => self.keeper.write(&root, draft),
            None => self.keeper.remove(&root, path),
        }
    }

    /// Keep every open draft and forget every backup of a file now saved:
    /// the project is going, or the window is.
    pub(super) fn keep_drafts(&self) {
        // The strip too, with where each file was left: written by the same
        // keeper, so the wait at the window's close waits for it.
        self.strip_changed();
        let Some(root) = self.root() else { return };
        for (path, draft) in self.editor.backups() {
            match draft {
                Some(draft) => self.keeper.write(&root, draft),
                None => self.keeper.remove(&root, &path),
            }
        }
    }

    pub(super) fn keep_drafts_and_wait(&self) {
        self.keep_drafts();
        if !self.keeper.flush(KEEP_BUDGET) {
            eprintln!("rusty: the drafts were still being written when the window closed");
        }
    }

    /// Put back the project's strip and the drafts kept for it, each draft
    /// over the disk's text so that it is unsaved, marked so, and one undo
    /// from it. Read after every backup asked before (`Keeper::read`).
    pub(super) fn bring_back(self: &Rc<Self>, root: PathBuf) {
        self.strip_opening();
        let asked = root.clone();
        self.keeper.read(&asked.clone(), move |kept| {
            let back = Back::read(&asked, kept);
            work::deliver(back, move |app, back| app.came_back(&root, back));
        });
    }

    pub(super) fn backup_failed(self: &Rc<Self>, reason: String) {
        if !self.backup_warned.replace(true) {
            self.note(2, t!("desk.backup-failed", reason = reason));
        }
    }
}
