//! The rows the view draws, made when the view asks for them.
//!
//! A `Model` over the document rather than a list built up front: the
//! ListView asks only for the rows on screen, so opening a 24,000-line file
//! builds forty rows, and an edit tells the view which rows to ask for again.

use std::{cell::RefCell, rc::Rc};

use rusty_edit::{Line, Token};
use slint::{Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};

use super::{Metrics, Shared};
use crate::{Piece, Row};

pub struct Rows {
    doc: Shared,
    metrics: Rc<RefCell<Metrics>>,
    notify: ModelNotify,
}

impl Rows {
    pub fn new(doc: Shared, metrics: Rc<RefCell<Metrics>>) -> Rows {
        Rows {
            doc,
            metrics,
            notify: ModelNotify::default(),
        }
    }

    /// Another document is on screen: every row is new.
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
            .map_or(0, |doc| doc.borrow().lines.len())
    }

    fn row_data(&self, row: usize) -> Option<Row> {
        let doc = self.doc.borrow().clone()?;
        let doc = doc.borrow();
        let text = doc.lines.get(row)?;
        let metrics = self.metrics.borrow();
        let painted = doc.painted.get(row).filter(|line| painted_is(line, text));
        let pieces = match painted {
            Some(line) => place(
                &metrics,
                line.spans.iter().map(|s| (s.text.as_str(), s.token)),
            ),
            None => place(&metrics, std::iter::once((text.as_str(), Token::Plain))),
        };
        Some(Row {
            number: row as i32 + 1,
            pieces: ModelRc::new(VecModel::from(pieces)),
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
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
        if !run.is_empty() && !run.trim().is_empty() {
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
