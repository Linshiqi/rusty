//! Where the caret rests: the bracket beside it and its partner outlined,
//! and the other places the name under it occurs washed — VS Code's
//! `editor.matchBrackets` and `editor.occurrencesHighlight`.
//!
//! The brackets are worked out here, at once, on every move and every edit,
//! from the painted lines (`brackets.rs`), and again when a repaint lands.
//! The occurrences are the server's (`textDocument/documentHighlight`),
//! asked [`ASK_AFTER`] after the caret stops: later than the change an edit
//! sends and later than the colours, so the server has the text the
//! position is in. An edit takes the washes away at once, since they are
//! about the text before it; a move off them takes them away too, and a
//! move within them keeps them, as VS Code's do. Every ask is numbered, and
//! an answer is drawn only while it is the last ask and the text is the
//! text it was asked about.

use std::{cell::RefCell, time::Duration};

use rusty_lsp::EditRange;
use slint::{Timer, TimerMode};

use super::{Surface, asks::Asks, brackets, cursors, document::Change, rows};

/// How long after the caret stops the server is asked where else its name
/// occurs: after the 300 ms the colours and hints wait.
const ASK_AFTER: Duration = Duration::from_millis(400);

/// Above this many rows to draw again, every row is.
const REDRAW_ALL: usize = 200;

/// An occurrence washed: its line, and the characters it covers.
pub type Wash = (usize, usize, usize);

/// What is drawn where the caret rests, each with the document and the
/// version of its text it is about — the rows draw it only over that text.
#[derive(Default)]
pub struct Resting {
    pub brackets: Option<(String, u64, [brackets::At; 2])>,
    pub washes: Option<(String, u64, Vec<Wash>)>,
}

impl Resting {
    /// The columns of the brackets outlined on `line` of `path` at `version`.
    pub fn brackets_on(&self, path: &str, version: u64, line: usize) -> Vec<usize> {
        match &self.brackets {
            Some((p, v, pair)) if p == path && *v == version => pair
                .iter()
                .filter(|(l, _)| *l == line)
                .map(|(_, col)| *col)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The characters washed on `line` of `path` at `version`.
    pub fn washes_on(&self, path: &str, version: u64, line: usize) -> Vec<(usize, usize)> {
        match &self.washes {
            Some((p, v, washes)) if p == path && *v == version => {
                let first = washes.partition_point(|w| w.0 < line);
                washes[first..]
                    .iter()
                    .take_while(|w| w.0 == line)
                    .map(|w| (w.1, w.2))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// Everything drawn taken away after `change`: the lines it was drawn
    /// on outside the change, where they are now — the rows that still show
    /// it, since the change drew its own again.
    pub fn take_after(&mut self, change: Change) -> Vec<usize> {
        let mut lines: Vec<usize> = Vec::new();
        if let Some((.., pair)) = self.brackets.take() {
            lines.extend(pair.iter().map(|(line, _)| *line));
        }
        if let Some((.., washes)) = self.washes.take() {
            lines.extend(washes.iter().map(|w| w.0));
        }
        lines
            .into_iter()
            .filter_map(|line| moved(line, change))
            .collect()
    }
}

/// Where line `line` is after `change`: below it, moved by what it added
/// or took away; inside it, nowhere the change did not draw already.
fn moved(line: usize, change: Change) -> Option<usize> {
    if line >= change.first + change.removed {
        Some(line + change.added - change.removed)
    } else if line < change.first {
        Some(line)
    } else {
        None
    }
}

/// Whether a caret at `at` (line, column) is still on what is washed:
/// inside an occurrence, or at either end of one.
pub fn keeps(washes: &[Wash], at: (usize, usize)) -> bool {
    washes
        .iter()
        .any(|&(line, from, to)| line == at.0 && from <= at.1 && at.1 <= to)
}

/// The asking for occurrences: when, which ask is the last, and where the
/// caret last rested.
#[derive(Default)]
pub(super) struct Occurring {
    timer: Timer,
    asks: Asks,
    rested: RefCell<Option<(String, u64, usize, usize)>>,
}

impl Surface {
    /// The caret moved, or the text under it changed: the brackets beside
    /// it now, and the washes kept while the caret is on one, or taken away
    /// and asked for again once it stops.
    pub(super) fn caret_rested(&self) {
        let Some(doc) = self.doc() else { return };
        let spot = {
            let d = doc.borrow();
            let at = self.vim_cursor(&d).unwrap_or(d.primary().head);
            let (line, col) = d.lines.position(&d.text, at);
            (d.path.clone(), d.version, line, col)
        };
        if self.occurring.rested.borrow().as_ref() == Some(&spot) {
            return;
        }
        *self.occurring.rested.borrow_mut() = Some(spot.clone());
        self.match_brackets();
        let (path, version, line, col) = spot;
        let kept = self
            .resting
            .borrow()
            .washes
            .as_ref()
            .is_some_and(|(p, v, washes)| {
                *p == path && *v == version && keeps(washes, (line, col))
            });
        if kept {
            return;
        }
        self.washes_away();
        let this = self.this.clone();
        self.occurring
            .timer
            .start(TimerMode::SingleShot, ASK_AFTER, move || {
                if let Some(this) = this.upgrade() {
                    this.ask_occurrences();
                }
            });
    }

    /// The bracket beside the caret and its partner, worked out again: on
    /// a move, an edit, or a repaint that says what is a string now.
    pub(super) fn match_brackets(&self) {
        let Some(doc) = self.doc() else { return };
        let found = {
            let d = doc.borrow();
            let at = self.vim_cursor(&d).unwrap_or(d.primary().head);
            let caret = d.lines.position(&d.text, at);
            brackets::matching(caret, d.lines.count(), |n| {
                let text = d.lines.text(&d.text, n);
                let painted = d.painted.get(n).filter(|p| rows::painted_is(p, text));
                brackets::on_line(text, painted)
            })
            .map(|pair| (d.path.clone(), d.version, pair))
        };
        let before = std::mem::replace(&mut self.resting.borrow_mut().brackets, found.clone());
        if before == found {
            return;
        }
        let lines = before
            .iter()
            .chain(found.iter())
            .flat_map(|(.., pair)| pair.iter().map(|(line, _)| *line))
            .collect();
        self.redraw_lines(lines);
    }

    /// The washes gone, and any answer on its way forgotten.
    pub(super) fn washes_away(&self) {
        self.occurring.asks.forget();
        let gone = self.resting.borrow_mut().washes.take();
        if let Some((.., washes)) = gone {
            self.redraw_lines(washes.iter().map(|w| w.0).collect());
        }
    }

    /// An edit: what was drawn where the caret rested is about the text
    /// before it, and goes from the rows that still show it.
    pub(super) fn rest_edited(&self, change: Change) {
        self.occurring.asks.forget();
        let stale = self.resting.borrow_mut().take_after(change);
        self.redraw_lines(stale);
    }

    /// Nothing on screen: nothing rests, and nothing is asked.
    pub(super) fn rest_nowhere(&self) {
        self.occurring.timer.stop();
        self.occurring.asks.forget();
        *self.occurring.rested.borrow_mut() = None;
        *self.resting.borrow_mut() = Resting::default();
    }

    /// The server settled, or says its analysis changed: a caret that
    /// rested while it was loading was answered with nothing, and is asked
    /// about again where nothing is washed.
    pub fn occurrences_again(&self) {
        if self.doc().is_none() {
            return;
        }
        let washed = self
            .resting
            .borrow()
            .washes
            .as_ref()
            .is_some_and(|(.., washes)| !washes.is_empty());
        if washed {
            return;
        }
        let this = self.this.clone();
        self.occurring
            .timer
            .start(TimerMode::SingleShot, ASK_AFTER, move || {
                if let Some(this) = this.upgrade() {
                    this.ask_occurrences();
                }
            });
    }

    /// Ask the server where else the name under the caret occurs: one
    /// caret, on or beside a name, or a selection that is the name.
    fn ask_occurrences(&self) {
        let Some(doc) = self.doc() else { return };
        let question = {
            let d = doc.borrow();
            let primary = d.primary();
            let vim = self.vim_cursor(&d);
            let at = vim.unwrap_or(primary.head);
            let word = cursors::word_at(&d.text, at);
            let on_name = match word {
                Some(word) => {
                    vim.is_some() || primary.is_caret() || (primary.start(), primary.end()) == word
                }
                None => false,
            };
            (d.cursors.len() == 1 && on_name).then(|| {
                let (line, col) = d.lines.position(&d.text, at);
                (
                    d.path.clone(),
                    d.version,
                    line as u32,
                    col as u32,
                    self.servers.draft(&d.text),
                )
            })
        };
        let Some((path, version, line, col, draft)) = question else {
            return;
        };
        let asked = self.occurring.asks.ask();
        let about = path.clone();
        self.servers.ask(
            &path,
            Some(draft),
            move |client, path| client.document_highlights(path, line, col),
            move |app, ranges| app.editor().occurred(asked, &about, version, ranges),
        );
    }

    /// The server's answer: drawn while it is the last ask and the text is
    /// the one it was asked about. A name running over lines is not one a
    /// caret rests on, and is not washed.
    pub fn occurred(&self, asked: u64, path: &str, version: u64, ranges: Vec<EditRange>) {
        if !self.occurring.asks.answers(asked) {
            return;
        }
        let Some(doc) = self.doc() else { return };
        let washes: Vec<Wash> = {
            let d = doc.borrow();
            if d.path != path || d.version != version {
                return;
            }
            let mut washes: Vec<Wash> = ranges
                .iter()
                .filter(|r| r.start_line == r.end_line && r.end_col > r.start_col)
                .filter(|r| (r.start_line as usize) < d.lines.count())
                .map(|r| {
                    (
                        r.start_line as usize,
                        r.start_col as usize,
                        r.end_col as usize,
                    )
                })
                .collect();
            washes.sort_unstable();
            washes.dedup();
            washes
        };
        let mut lines: Vec<usize> = washes.iter().map(|w| w.0).collect();
        let before = self
            .resting
            .borrow_mut()
            .washes
            .replace((path.to_string(), version, washes));
        if let Some((.., before)) = before {
            lines.extend(before.iter().map(|w| w.0));
        }
        self.redraw_lines(lines);
    }

    /// For the probe: how many brackets are outlined and how many
    /// occurrences washed in the document on screen.
    pub fn resting_marks(&self) -> (usize, usize) {
        let resting = self.resting.borrow();
        (
            resting.brackets.as_ref().map_or(0, |_| 2),
            resting.washes.as_ref().map_or(0, |(.., w)| w.len()),
        )
    }

    /// The rows `lines` are drawn on, drawn again.
    fn redraw_lines(&self, mut lines: Vec<usize>) {
        lines.sort_unstable();
        lines.dedup();
        if lines.len() > REDRAW_ALL {
            self.rows.reset();
            return;
        }
        let count = self.doc().map_or(0, |doc| doc.borrow().lines.count());
        for line in lines.into_iter().filter(|&line| line < count) {
            self.rows.changed(line, line + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A move within an occurrence, or to either end of it, keeps the
    /// washes; anywhere else they go.
    #[test]
    fn the_washes_stay_while_the_caret_is_on_one() {
        let washes = [(2, 4, 9), (7, 0, 5)];
        assert!(keeps(&washes, (2, 4)));
        assert!(keeps(&washes, (2, 9)), "just after the name");
        assert!(keeps(&washes, (7, 3)));
        assert!(!keeps(&washes, (2, 10)));
        assert!(!keeps(&washes, (3, 5)), "another line");
        assert!(!keeps(&[], (0, 0)));
    }

    /// Below an edit the marks moved with their lines; on the lines it
    /// changed, the edit drew its rows again itself.
    #[test]
    fn an_edit_takes_the_marks_away_from_where_they_now_are() {
        let mut resting = Resting {
            brackets: Some(("a.rs".into(), 1, [(1, 4), (9, 0)])),
            washes: Some(("a.rs".into(), 1, vec![(0, 2, 5), (5, 1, 4), (12, 0, 3)])),
        };
        let change = Change {
            first: 4,
            removed: 2,
            added: 3,
        };
        let mut stale = resting.take_after(change);
        stale.sort_unstable();
        assert_eq!(stale, [0, 1, 10, 13]);
        assert!(resting.brackets.is_none() && resting.washes.is_none());
    }

    /// Marks are drawn only over the text they are about.
    #[test]
    fn marks_belong_to_one_text() {
        let resting = Resting {
            brackets: Some(("a.rs".into(), 3, [(1, 4), (1, 9)])),
            washes: Some(("a.rs".into(), 3, vec![(1, 0, 2), (2, 5, 8), (2, 9, 12)])),
        };
        assert_eq!(resting.brackets_on("a.rs", 3, 1), [4, 9]);
        assert!(
            resting.brackets_on("a.rs", 4, 1).is_empty(),
            "another version"
        );
        assert!(resting.brackets_on("b.rs", 3, 1).is_empty(), "another file");
        assert_eq!(resting.washes_on("a.rs", 3, 2), [(5, 8), (9, 12)]);
        assert!(resting.washes_on("a.rs", 3, 3).is_empty());
    }
}
