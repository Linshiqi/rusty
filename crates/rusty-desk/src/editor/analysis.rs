//! What the server says about the text beside the text: its colours over
//! the painter's (semantic tokens) and its inlay hints.
//!
//! Asked a moment after the typing stops, when a document comes on screen,
//! and when the server says its analysis changed — rust-analyzer answers
//! with nothing while it loads, and says so with a refresh once it has. A
//! long file is asked about around the rows on screen, and again when the
//! view leaves what the answer covered.
//!
//! **An answer is drawn only over the text it is about.** The question
//! carries the text on screen (`Servers::ask` gives the server the draft
//! first), and the answer is carried from that text to the one on screen
//! by `rusty_lsp::follow`, as every edit carries what is already drawn. A
//! hints answer names the text the server had (`InlayHints::about`); one
//! about another text is asked for again, once, rather than drawn on the
//! wrong columns.

use std::{
    cell::{Cell, RefCell},
    time::Duration,
};

use rusty_lsp::{InlayHints, SemanticSpan};
use slint::{ComponentHandle, Timer, TimerMode};

use super::{Surface, inlays};
use crate::Code;

/// Above this many lines a file is asked about around the view only:
/// whole, a 24,000-line file's colours were megabytes and most of a second.
const WHOLE_UP_TO: usize = 3000;
/// How far either side of the view a long file is asked about.
const AROUND: usize = 400;
/// How long after the last keystroke the server is asked.
const AFTER: Duration = Duration::from_millis(300);

/// A hint as the probe reads it: its line, its column, its label and where
/// the character at its column starts.
pub type FirstHint = (usize, usize, String, f32);

#[derive(Default)]
pub(super) struct Analysis {
    timer: Timer,
    /// The lines the last answer for a long file covered, and of which file.
    covered: RefCell<Option<(String, usize, usize)>>,
    /// A hints answer about another text was asked for again already.
    retried: Cell<bool>,
}

impl Surface {
    /// Ask again a moment from now; every call restarts the wait.
    pub fn analyse_soon(&self) {
        let this = self.this.clone();
        self.analysis
            .timer
            .start(TimerMode::SingleShot, AFTER, move || {
                if let Some(this) = this.upgrade() {
                    this.analyse();
                }
            });
    }

    /// Ask the server for the colours and the hints of the document on
    /// screen, the text on screen given to it first.
    fn analyse(&self) {
        let Some(doc) = self.doc() else { return };
        let (path, text, version, count) = {
            let d = doc.borrow();
            (d.path.clone(), d.text.clone(), d.version, d.lines.count())
        };
        let range = (count > WHOLE_UP_TO).then(|| {
            let first = self.first_line_on_screen();
            (
                first.saturating_sub(AROUND),
                (first + AROUND * 2).min(count),
            )
        });
        *self.analysis.covered.borrow_mut() = range.map(|(from, to)| (path.clone(), from, to));
        let lines = range.map(|(from, to)| (from as u32, to as u32));
        let (from, to) = lines.unwrap_or((0, count as u32));
        let hints = self.overview.hints.get();
        let draft = self.servers.draft(&text);
        let asked = path.clone();
        self.servers.ask(
            &path,
            Some(draft),
            move |client, path| {
                let colours = client.semantic_tokens(path, lines).ok();
                let hints = hints
                    .then(|| client.inlay_hints(path, from, to).ok())
                    .flatten();
                Ok((colours, hints))
            },
            move |app, (colours, hints)| {
                app.editor()
                    .analysed(&asked, version, &text, colours, hints)
            },
        );
    }

    /// The server's answer about `asked`, the text of `path` at `version`:
    /// carried to the text there now, and drawn.
    fn analysed(
        &self,
        path: &str,
        version: u64,
        asked: &str,
        colours: Option<Vec<SemanticSpan>>,
        hints: Option<InlayHints>,
    ) {
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return;
        };
        let mut again = false;
        {
            let mut d = doc.borrow_mut();
            let moved = d.version != version;
            if let Some(mut colours) = colours {
                if moved {
                    rusty_lsp::follow::semantic(&mut colours, asked, &d.text);
                }
                colours.sort_by_key(|s| (s.line, s.start_col));
                d.semantic = colours;
            }
            match hints {
                Some(answer) if answer.about != rusty_lsp::text_mark(asked) => {
                    // About another text: the hints already drawn have
                    // followed every edit, and stay while it is asked again.
                    again = !self.analysis.retried.replace(true);
                }
                Some(answer) => {
                    self.analysis.retried.set(false);
                    let mut hints = answer.hints;
                    if moved {
                        rusty_lsp::follow::hints(&mut hints, asked, &d.text);
                    }
                    hints.sort_by_key(|h| (h.line, h.col));
                    if let Some(window) = self.window.upgrade() {
                        let labels: String = hints.iter().map(|h| h.label.as_str()).collect();
                        self.metrics.borrow_mut().learn(&window, &labels);
                    }
                    d.hints = hints;
                }
                None => {}
            }
            if !self.overview.hints.get() {
                d.hints.clear();
            }
        }
        if again {
            self.analyse_soon();
        }
        if self
            .doc()
            .is_some_and(|current| std::rc::Rc::ptr_eq(&current, &doc))
        {
            self.hints_changed();
        }
    }

    /// The hints or the colours of the document on screen changed: every
    /// row is drawn again, the view is as wide as its widest line with its
    /// hints, and the caret stands where its column now is.
    pub(super) fn hints_changed(&self) {
        let Some(doc) = self.doc() else { return };
        let widest = {
            let d = doc.borrow();
            let metrics = self.metrics.borrow();
            let mut lines: Vec<usize> = d.hints.iter().map(|h| h.line as usize).collect();
            lines.dedup();
            lines
                .into_iter()
                .filter(|&line| line < d.lines.count())
                .map(|line| inlays::end_on(&metrics, &d, line))
                .fold(self.widest.get(), f32::max)
        };
        if widest > self.widest.get() {
            self.widest.set(widest);
            if let Some(window) = self.window.upgrade() {
                window.global::<Code>().set_content_width(widest + 200.0);
            }
        }
        self.rows.reset();
        self.show_caret();
        self.overview
            .paintings
            .set(self.overview.paintings.get() + 1);
        self.view_changed();
    }

    /// A long file's view left what its answer covered: ask about where it
    /// is now.
    pub(super) fn analysis_follows_view(&self) {
        let Some(doc) = self.doc() else { return };
        let covered = self.analysis.covered.borrow().clone();
        let Some((path, from, to)) = covered else {
            return;
        };
        let first = self.first_line_on_screen();
        let outside = doc.borrow().path == path && (first < from || first + 60 > to);
        if outside {
            *self.analysis.covered.borrow_mut() = None;
            self.analyse_soon();
        }
    }

    /// For the probe: how many colours and hints the document on screen
    /// holds, and its first hint that is not a parameter's — its line, its
    /// column, its label and where the character at its column starts.
    pub fn told(&self) -> Option<(usize, usize, Option<FirstHint>)> {
        let doc = self.doc()?;
        let d = doc.borrow();
        let metrics = self.metrics.borrow();
        let first = d.hints.iter().find(|h| !h.parameter).map(|h| {
            let (line, col) = (h.line as usize, h.col as usize);
            let start = inlays::char_on(&metrics, &d, line, col);
            (line, col, h.label.clone(), start)
        });
        Some((d.semantic.len(), d.hints.len(), first))
    }

    /// The line drawn on the view's first row.
    fn first_line_on_screen(&self) -> usize {
        let (Some(window), Some(doc)) = (self.window.upgrade(), self.doc()) else {
            return 0;
        };
        let code = window.global::<Code>();
        let row = ((-code.get_scroll_y()) / code.get_row_height().max(1.0)).max(0.0) as usize;
        let d = doc.borrow();
        d.line_at_row(row.min(d.rows().saturating_sub(1)))
    }
}
