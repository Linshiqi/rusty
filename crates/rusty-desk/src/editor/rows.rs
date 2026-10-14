//! The rows the view draws, made when the view asks for them.
//!
//! A `Model` over the document rather than a list built up front: the
//! ListView asks only for the rows on screen, so opening a 24,000-line file
//! builds forty rows, and an edit tells the view which rows to ask for again.
//! A row is its text as pieces of one token each at the x they start at, and
//! the marks behind it — the selections crossing it and the carets on it
//! other than the primary one, which the view draws on its own.

use std::{cell::RefCell, rc::Rc};

use rusty_edit::{Line, Token};
use rusty_lsp::DiagSeverity;
use slint::{Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};

use super::{Document, Metrics, Shared, find};
use crate::lsp::Problems;
use crate::{Mark, Piece, Row};

pub struct Rows {
    doc: Shared,
    metrics: Rc<RefCell<Metrics>>,
    found: Rc<RefCell<find::Marks>>,
    problems: Problems,
    notify: ModelNotify,
}

impl Rows {
    pub fn new(
        doc: Shared,
        metrics: Rc<RefCell<Metrics>>,
        found: Rc<RefCell<find::Marks>>,
        problems: Problems,
    ) -> Rows {
        Rows {
            doc,
            metrics,
            found,
            problems,
            notify: ModelNotify::default(),
        }
    }

    /// Another document is on screen, or too much changed to name: every
    /// row is new.
    pub fn reset(&self) {
        self.notify.reset();
    }

    pub fn changed(&self, from: usize, to: usize) {
        let count = self.row_count();
        for row in from..to.min(count) {
            self.notify.row_changed(row);
        }
    }

    pub fn added(&self, at: usize, count: usize) {
        self.notify.row_added(at, count);
    }

    pub fn removed(&self, at: usize, count: usize) {
        self.notify.row_removed(at, count);
    }
}

impl Model for Rows {
    type Data = Row;

    fn row_count(&self) -> usize {
        self.doc
            .borrow()
            .as_ref()
            .map_or(0, |doc| doc.borrow().lines.count())
    }

    fn row_data(&self, row: usize) -> Option<Row> {
        let doc = self.doc.borrow().clone()?;
        let doc = doc.borrow();
        if row >= doc.lines.count() {
            return None;
        }
        let text = doc.lines.text(&doc.text, row);
        let metrics = self.metrics.borrow();
        let painted = doc.painted.get(row).filter(|line| painted_is(line, text));
        let pieces = match painted {
            Some(line) => place(
                &metrics,
                line.spans.iter().map(|s| (s.text.as_str(), s.token)),
            ),
            None => place(&metrics, std::iter::once((text, Token::Plain))),
        };
        Some(Row {
            number: row as i32 + 1,
            pieces: ModelRc::new(VecModel::from(pieces)),
            marks: ModelRc::new(VecModel::from(marks(
                &doc,
                &metrics,
                &self.found.borrow(),
                self.problems
                    .borrow()
                    .get(&doc.path)
                    .map_or(&[][..], Vec::as_slice),
                row,
            ))),
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

/// What is drawn on `row` besides its text, back to front: find's scope
/// and its matches, the selections crossing it, the squiggles of what the
/// servers found, and the carets on it but the primary one. A span that
/// carries on past the line's end takes a space's width there, as every
/// editor shows the line break it holds.
fn marks(
    doc: &Document,
    metrics: &Metrics,
    found: &find::Marks,
    problems: &[rusty_lsp::FileDiagnostic],
    row: usize,
) -> Vec<Mark> {
    let (start, end) = (doc.lines.start(row), doc.lines.end(row));
    let text = doc.lines.text(&doc.text, row);
    let x_at = |at: usize| metrics.x_of(text, text[..at - start].chars().count());
    let mut marks = Vec::new();
    let mut span = |from: usize, to: usize, kind: i32| {
        let (a, b) = (from.max(start), to.min(end));
        if from > end || to < start || a > b {
            return;
        }
        let x = x_at(a);
        let past = to > end;
        let width = x_at(b) - x + if past { metrics.ascii() } else { 0.0 };
        if kind > 1 || width > 0.0 {
            marks.push(mark(x, width, kind));
        }
    };
    if let Some((from, to)) = found.scope {
        span(from, to, 4);
    }
    for at in found.on(start, end) {
        let (from, to) = found.matches[at];
        span(from, to, if found.current == Some(at) { 3 } else { 2 });
    }
    for cursor in &doc.cursors {
        if !cursor.is_caret() && cursor.start() <= end && cursor.end() > start {
            span(cursor.start(), cursor.end(), 0);
        }
    }
    // Worst last, so an error is drawn over the warning it shares a word
    // with; hints are not problems, and are not drawn.
    let mut squiggles: Vec<(rusty_lsp::DiagSeverity, f32, f32)> = problems
        .iter()
        .filter(|d| d.severity != DiagSeverity::Hint)
        .filter_map(|d| {
            let (from, to) = rusty_lsp::squiggle::drawn_on(d, row as u32, text)?;
            let count = text.chars().count();
            let x = metrics.x_of(text, from as usize);
            // One past the line's end is a cell after everything on it.
            let past = (to as usize).saturating_sub(count) as f32 * metrics.ascii();
            let width = metrics.x_of(text, to as usize) + past - x;
            Some((d.severity, x, width))
        })
        .collect();
    squiggles.sort_by_key(|(severity, ..)| std::cmp::Reverse(*severity));
    for (severity, x, width) in squiggles {
        let kind = match severity {
            DiagSeverity::Error => 5,
            DiagSeverity::Warning => 6,
            _ => 7,
        };
        marks.push(Mark {
            path: SharedString::from(wave(width)),
            ..mark(x, width, kind)
        });
    }
    for (index, cursor) in doc.cursors.iter().enumerate() {
        if index > 0 && cursor.head >= start && cursor.head <= end {
            marks.push(mark(x_at(cursor.head), 0.0, 1));
        }
    }
    marks
}

fn mark(x: f32, width: f32, kind: i32) -> Mark {
    Mark {
        x,
        width,
        kind,
        path: SharedString::new(),
    }
}

/// A squiggle's line: up and down every two pixels across `width`, in a
/// box three pixels tall.
fn wave(width: f32) -> String {
    use std::fmt::Write;
    let mut path = String::from("M 0 2.5");
    let (mut x, mut up) = (0.0, true);
    while x < width {
        x = (x + 2.0f32).min(width);
        let _ = write!(path, " L {x:.1} {}", if up { 0.5 } else { 2.5 });
        up = !up;
    }
    path
}

/// A painted line drawn on its own, as the hover card draws a signature:
/// its pieces placed as the editor places code, and nothing behind them.
pub fn painted_row(metrics: &Metrics, line: &Line) -> Row {
    let pieces = place(
        metrics,
        line.spans.iter().map(|s| (s.text.as_str(), s.token)),
    );
    Row {
        number: 0,
        pieces: ModelRc::new(VecModel::from(pieces)),
        marks: ModelRc::default(),
    }
}

/// Whether a painted line is the text on screen — a line typed on since its
/// painting is drawn plain until the repaint catches up.
fn painted_is(line: &Line, text: &str) -> bool {
    let mut rest = text;
    for span in &line.spans {
        match rest.strip_prefix(span.text.as_str()) {
            Some(after) => rest = after,
            None => return false,
        }
    }
    rest.is_empty()
}

/// Runs of one token placed along the line. A tab ends a piece: it is drawn
/// as the gap to the next stop, not as a glyph.
fn place<'t>(metrics: &Metrics, spans: impl Iterator<Item = (&'t str, Token)>) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut x = 0.0;
    for (text, token) in spans {
        let mut start = x;
        let mut run = String::new();
        for ch in text.chars() {
            let width = metrics.advance(ch, x);
            if ch == '\t' {
                if !run.is_empty() {
                    pieces.push(piece(std::mem::take(&mut run), start, token));
                }
                x += width;
                start = x;
                continue;
            }
            run.push(ch);
            x += width;
        }
        if !run.trim().is_empty() {
            pieces.push(piece(run, start, token));
        }
    }
    pieces
}

fn piece(text: String, x: f32, token: Token) -> Piece {
    Piece {
        text: SharedString::from(text),
        x,
        kind: kind(token),
    }
}

/// A token's kind as `Theme.token` numbers it.
fn kind(token: Token) -> i32 {
    match token {
        Token::Plain => 0,
        Token::Keyword => 1,
        Token::Str => 2,
        Token::Number => 3,
        Token::Comment => 4,
        Token::Type => 5,
        Token::Function => 6,
        Token::Macro => 7,
        Token::Punctuation => 8,
        Token::Variable => 9,
        Token::Namespace => 10,
    }
}
