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
    gutter: Rc<RefCell<Gutter>>,
    notify: ModelNotify,
    /// Each line's indent guides, for the document and version they are of:
    /// counted for the whole text at once, since a blank line takes its
    /// neighbours', and kept until the text changes.
    levels: ByVersion<Vec<u8>>,
    /// The lenses of the document and version they are of: the line each is
    /// drawn on, and what it runs.
    lenses: ByVersion<Lenses>,
}

/// Something worked out from one version of one document, kept until the
/// text changes.
type ByVersion<T> = RefCell<Option<(String, u64, Rc<T>)>>;

/// What a file can run, by the line each lens is drawn on.
pub type Lenses = std::collections::BTreeMap<u32, rusty_edit::Runnable>;

/// What the margin shows besides the numbers: each file's breakpoints —
/// placed, or asked for where the debugger found no code — and the line the
/// debugger stopped at. Lines from zero; files as the project names them.
#[derive(Default)]
pub struct Gutter {
    pub breakpoints: std::collections::BTreeMap<String, std::collections::BTreeMap<u32, bool>>,
    pub stopped: Option<(String, u32)>,
    /// Whether an example's `main` gets ▶ Run: not where `cargo run` would
    /// flash a board — a root that is its own firmware, or inside the
    /// excluded firmware crate (`firmware`).
    pub runs_examples: bool,
    pub firmware: Option<String>,
    /// The last run's verdicts, by the harness's name for each test; a
    /// lens finds its own by its file's module path.
    pub verdicts: std::collections::HashMap<String, rusty_edit::testrun::Verdict>,
}

impl Gutter {
    /// Every breakpoint, as the debugger is told them.
    pub fn all(&self) -> Vec<(String, u32)> {
        self.breakpoints
            .iter()
            .flat_map(|(path, lines)| lines.keys().map(move |line| (path.clone(), *line)))
            .collect()
    }
}

impl Rows {
    pub fn new(
        doc: Shared,
        metrics: Rc<RefCell<Metrics>>,
        found: Rc<RefCell<find::Marks>>,
        problems: Problems,
        gutter: Rc<RefCell<Gutter>>,
    ) -> Rows {
        Rows {
            doc,
            metrics,
            found,
            problems,
            gutter,
            notify: ModelNotify::default(),
            levels: RefCell::default(),
            lenses: RefCell::default(),
        }
    }

    /// The lenses of `doc`, found once per version: a test, a module of
    /// them, or an example's `main` where running it runs it here.
    pub fn lenses(&self, doc: &Document) -> Rc<Lenses> {
        let mut cached = self.lenses.borrow_mut();
        if let Some((path, version, lenses)) = cached.as_ref()
            && *path == doc.path
            && *version == doc.version
        {
            return lenses.clone();
        }
        let mut found = Lenses::new();
        if doc.path.ends_with(".rs") {
            let lines: Vec<&str> = doc.text.split('\n').collect();
            for runnable in rusty_edit::tests_in::runnables(&doc.text) {
                if let Some(line) = rusty_edit::lens::lens_line(&lines, runnable.line) {
                    found.insert(line, runnable);
                }
            }
            let gutter = self.gutter.borrow();
            let inside_firmware = gutter
                .firmware
                .as_ref()
                .is_some_and(|dir| doc.path.starts_with(&format!("{dir}/")));
            if gutter.runs_examples
                && !inside_firmware
                && let Some(main) = rusty_edit::tests_in::example_main(&doc.path, &doc.text)
            {
                found.insert(main.line, main);
            }
        }
        let found = Rc::new(found);
        *cached = Some((doc.path.clone(), doc.version, found.clone()));
        found
    }

    /// What the lens on `row` of the document on screen runs.
    pub fn runnable_at(&self, row: usize) -> Option<rusty_edit::Runnable> {
        let doc = self.doc.borrow().clone()?;
        let doc = doc.borrow();
        self.lenses(&doc).get(&(row as u32)).cloned()
    }

    /// The indent guides of `doc`, counted once per version.
    fn levels(&self, doc: &Document) -> Rc<Vec<u8>> {
        let mut cached = self.levels.borrow_mut();
        if let Some((path, version, levels)) = cached.as_ref()
            && *path == doc.path
            && *version == doc.version
        {
            return levels.clone();
        }
        let levels = Rc::new(rusty_edit::guides::indent_levels(&doc.text));
        *cached = Some((doc.path.clone(), doc.version, levels.clone()));
        levels
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
        let gutter = self.gutter.borrow();
        let line = row as u32;
        let breakpoint = match gutter
            .breakpoints
            .get(&doc.path)
            .and_then(|lines| lines.get(&line))
        {
            Some(true) => 1,
            Some(false) => 2,
            None => 0,
        };
        let stopped = gutter
            .stopped
            .as_ref()
            .is_some_and(|(path, at)| *path == doc.path && *at == line);
        // A lens after everything on its line.
        let lenses = self.lenses(&doc);
        let (lens, lens_x, verdict) = match lenses.get(&line) {
            Some(runnable) => {
                let end = metrics.x_of(text, text.chars().count());
                let kind = match runnable.kind {
                    rusty_edit::RunnableKind::Test => 1,
                    rusty_edit::RunnableKind::Example => 2,
                    rusty_edit::RunnableKind::Module => 3,
                };
                let verdict = match rusty_edit::testrun::verdict_of_lens(
                    &gutter.verdicts,
                    &doc.path,
                    &runnable.filter,
                    runnable.kind == rusty_edit::RunnableKind::Module,
                ) {
                    Some(rusty_edit::testrun::Verdict::Passed) => 1,
                    Some(rusty_edit::testrun::Verdict::Failed) => 2,
                    _ => 0,
                };
                (kind, end + metrics.ascii() * 3.0, verdict)
            }
            None => (0, 0.0, 0),
        };
        Some(Row {
            number: row as i32 + 1,
            breakpoint,
            stopped,
            lens,
            lens_x,
            verdict,
            pieces: ModelRc::new(VecModel::from(pieces)),
            marks: ModelRc::new(VecModel::from({
                // The guides first: everything else is drawn over them.
                let level = self.levels(&doc).get(row).copied().unwrap_or(0);
                let tab = rusty_edit::guides::TAB_SIZE as f32 * metrics.ascii();
                let mut all: Vec<Mark> = (0..level)
                    .map(|k| mark(f32::from(k) * tab, 1.0, 8))
                    .collect();
                all.extend(marks(
                    &doc,
                    &metrics,
                    &self.found.borrow(),
                    self.problems
                        .borrow()
                        .get(&doc.path)
                        .map_or(&[][..], Vec::as_slice),
                    row,
                ));
                all
            })),
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
        breakpoint: 0,
        stopped: false,
        lens: 0,
        lens_x: 0.0,
        verdict: 0,
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
pub(crate) fn kind(token: Token) -> i32 {
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
