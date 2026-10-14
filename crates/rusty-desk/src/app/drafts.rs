//! Drafts kept between runs — VS Code's hot exit, on the window's side:
//! when a draft is backed up, what a closing window or a project switch
//! keeps, and putting the kept drafts back. `editor/backup.rs` is the file
//! format and the thread that writes it.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use rusty_edit::text_file::{self, TextFile};
use rusty_i18n::t;
use slint::TimerMode;

use super::{App, Tab};
use crate::{
    editor::{Document, backup::Backup},
    work,
};

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

    /// Put back the drafts kept for the project at `root`, each over the
    /// disk's text so that it is unsaved, marked so, and one undo from it.
    pub(super) fn bring_back_drafts(self: &Rc<Self>, root: PathBuf) {
        let asked = root.clone();
        self.keeper.read(&asked.clone(), move |kept| {
            let found: Vec<(Backup, String)> = kept
                .into_iter()
                .map(|draft| {
                    // A file gone or no longer text has nothing under the
                    // draft: saving it writes the draft whole.
                    let disk = match text_file::read_text(&asked, &draft.path) {
                        Ok(TextFile::Text { text, .. }) => text,
                        _ => String::new(),
                    };
                    (draft, disk)
                })
                .collect();
            work::deliver(found, move |app, found| app.drafts_back(&root, found));
        });
    }

    pub(super) fn drafts_back(self: &Rc<Self>, root: &Path, found: Vec<(Backup, String)>) {
        // Another project opened meanwhile: these stay kept for next time.
        if found.is_empty() || self.root().as_deref() != Some(root) {
            return;
        }
        let mut first = None;
        for (draft, disk) in found {
            let path = draft.path.clone();
            if !self.editor.holds(&path) {
                let document = Document::new(path.clone(), disk, draft.charset, draft.ending);
                self.editor.add(document);
            }
            self.editor.restore(&path, draft.text);
            let mut state = self.state.borrow_mut();
            let tab = Tab::File(path);
            if !state.tabs.contains(&tab) {
                state.tabs.push(tab.clone());
            }
            first.get_or_insert(tab);
        }
        // In front only when nothing else is: a file opened meanwhile stays
        // where it is, caret and scroll included.
        let on_a_page = !matches!(self.active_tab(), Some(Tab::File(_)));
        if on_a_page {
            let mut state = self.state.borrow_mut();
            state.active = first.and_then(|tab| state.tabs.iter().position(|t| *t == tab));
        }
        if on_a_page {
            self.front_tab();
        } else {
            self.sync_tabs();
        }
    }

    pub(super) fn backup_failed(self: &Rc<Self>, reason: String) {
        if !self.backup_warned.replace(true) {
            self.note(2, t!("desk.backup-failed", reason = reason));
        }
    }
}
