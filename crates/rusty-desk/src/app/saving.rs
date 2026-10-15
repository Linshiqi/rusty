//! Saving: a file written in what it was read as, Rust through rustfmt and
//! C and C++ through clang-format first, each within a budget.

use std::{path::Path, rc::Rc, sync::mpsc, time::Duration};

use rusty_edit::text_file;
use rusty_i18n::t;

use super::{App, Tab};
use crate::{
    language::{self, Formatter},
    work,
};

/// How long a save waits for its formatter before saving what it has — VS
/// Code's budget for its own format-on-save.
pub(super) const FORMAT_BUDGET: Duration = Duration::from_secs(1);

/// What happens once a file is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum After {
    Stay,
    Close,
    /// Close the window of its own the file is in (`torn.rs`).
    CloseWindow(usize),
}

/// What the formatter made of a file being saved.
pub(super) enum Format {
    Changed(String),
    Unchanged,
    /// It did not answer within [`FORMAT_BUDGET`]; the save goes ahead.
    Late,
    Failed(String),
}

impl App {
    pub(super) fn save(self: &Rc<Self>) {
        if let Some(path) = self.editor.current_path() {
            self.save_file(path, After::Stay);
        }
    }

    /// Save a file: through its language's formatter first, within a
    /// budget, and the answer used only while the draft is still the text
    /// it was sent.
    pub(super) fn save_file(self: &Rc<Self>, path: String, after: After) {
        // A macro's expansion has no file to be written to.
        if rusty_edit::is_expansion(&path) {
            return;
        }
        let (Some(root), Some((text, ..)), Some(version)) = (
            self.root(),
            self.editor.draft_of(&path),
            self.editor.version_of(&path),
        ) else {
            return;
        };
        let Some(formatter) = language::of(&path).and_then(|language| language.formatter) else {
            self.write_file(path, after);
            return;
        };
        work::spawn(
            move || {
                let format = format_within(formatter, &root, &path, text);
                (path, format)
            },
            move |app, (path, format)| {
                match format {
                    Format::Changed(text) => {
                        app.editor.replace(&path, version, text);
                    }
                    // Once: a missing clang-format is the same sentence at
                    // every save.
                    Format::Failed(reason) => {
                        app.note_once(1, t!("desk.format-failed", reason = reason));
                    }
                    Format::Unchanged | Format::Late => {}
                }
                app.write_file(path, after);
            },
        );
    }

    /// Write every unsaved draft as it stands — no formatter, nothing asked
    /// — and then `then`: what is copied, built or run has to be what is on
    /// screen. A write that fails says so, and `then` does not run.
    pub(super) fn write_all_then(self: &Rc<Self>, then: impl FnOnce(&Rc<App>) + Send + 'static) {
        let Some(root) = self.root() else {
            then(self);
            return;
        };
        let drafts: Vec<_> = self
            .editor
            .backups()
            .into_iter()
            .filter(|(_, draft)| draft.is_some())
            .filter_map(|(path, _)| {
                let (text, charset, ending) = self.editor.draft_of(&path)?;
                Some((path, text, charset, ending))
            })
            .collect();
        if drafts.is_empty() {
            then(self);
            return;
        }
        work::spawn(
            move || {
                drafts
                    .into_iter()
                    .map(|(path, text, charset, ending)| {
                        let written = text_file::save_text(&root, &path, &text, charset, ending)
                            .map_err(|error| error.to_string());
                        (path, text, written)
                    })
                    .collect::<Vec<_>>()
            },
            move |app, written| {
                let mut failed = false;
                for (path, text, result) in written {
                    match result {
                        Ok(()) => {
                            app.editor.saved(&path, &text);
                            app.back_up(&path);
                        }
                        Err(reason) => {
                            failed = true;
                            app.note(2, t!("desk.save-failed", path = path, reason = reason));
                        }
                    }
                }
                app.sync_tabs();
                if !failed {
                    then(app);
                }
            },
        );
    }

    /// Write a file's draft as it stands now, in what it was read as.
    pub(super) fn write_file(self: &Rc<Self>, path: String, after: After) {
        if rusty_edit::is_expansion(&path) {
            return;
        }
        let (Some(root), Some((text, charset, ending))) =
            (self.root(), self.editor.draft_of(&path))
        else {
            return;
        };
        work::spawn(
            move || {
                text_file::save_text(&root, &path, &text, charset, ending)
                    .map(|()| (path.clone(), text))
                    .map_err(|error| (path, error.to_string()))
            },
            move |app, saved| match saved {
                Ok((path, text)) => {
                    app.editor.saved(&path, &text);
                    // Gone if nothing was typed while it was written; kept
                    // anew if something was.
                    app.back_up(&path);
                    app.sync_tabs();
                    match after {
                        After::Close => app.close_tab(&Tab::File(path).id()),
                        After::CloseWindow(id) => app.torn_close(id),
                        After::Stay => {}
                    }
                }
                Err((path, reason)) => {
                    app.note(2, t!("desk.save-failed", path = path, reason = reason))
                }
            },
        );
    }
}

/// The formatter's answer for a file being saved, or [`Format::Late`] once
/// [`FORMAT_BUDGET`] has passed — the format runs on, and its answer is
/// dropped when it comes.
fn format_within(formatter: Formatter, root: &Path, path: &str, text: String) -> Format {
    let (answer, answered) = mpsc::channel();
    let (root, path) = (root.to_path_buf(), path.to_string());
    std::thread::spawn(move || {
        let formatted = match formatter {
            Formatter::Rustfmt => rusty_edit::format_rust(&root, &path, &text),
            Formatter::ClangFormat => rusty_edit::format_c(&root, &path, &text),
        };
        let _ = answer.send(formatted.map(|f| {
            if f.changed {
                Format::Changed(f.text)
            } else {
                Format::Unchanged
            }
        }));
    });
    match answered.recv_timeout(FORMAT_BUDGET) {
        Ok(Ok(format)) => format,
        Ok(Err(error)) => Format::Failed(error.to_string()),
        Err(_) => Format::Late,
    }
}
