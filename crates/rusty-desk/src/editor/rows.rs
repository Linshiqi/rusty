//! The rows the view draws, made when the view asks for them.
//!
//! A `Model` over the document rather than a list built up front: the
//! ListView asks only for the rows on screen, so opening a 24,000-line file
//! builds forty rows, and an edit tells the view which rows to ask for again.
//! A row is its text as pieces of one token each at the x they start at, and
//! the marks behind it — the selections crossing it, the carets on it other
//! than the primary one (which the view draws on its own), and where the
//! caret rests: the bracket pair outlined, the name's occurrences washed.
//! A name drawn as a link while Ctrl is held is both: its characters are
//! pieces of the link's colour, and the line under them is a mark.

use std::{cell::RefCell, rc::Rc};

use rusty_edit::{Line, Token};
use rusty_lsp::DiagSeverity;
use slint::{Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};

use super::{Document, Metrics, Shared, find, inlays, resting::Resting};
use crate::lsp::Problems;
use crate::{Hint, Mark, Piece, Row};

pub struct Rows {
    doc: Shared,
    metrics: Rc<RefCell<Metrics>>,
    found: Rc<RefCell<find::Marks>>,
    resting: Rc<RefCell<Resting>>,
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
    /// What can fold, by header, for the document and version they are of.
    regions: ByVersion<Vec<rusty_edit::Region>>,
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
    /// Indent guides turned off in Settings.
    pub no_guides: bool,
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
        resting: Rc<RefCell<Resting>>,
        problems: Problems,
        gutter: Rc<RefCell<Gutter>>,
    ) -> Rows {
        Rows {
            doc,
            metrics,
            found,
            resting,
            problems,
            gutter,
            notify: ModelNotify::default(),
            levels: RefCell::default(),
            lenses: RefCell::default(),
            regions: RefCell::default(),
        }
    }

    /// What can fold in `doc`, found once per version, in header order.
    pub fn regions(&self, doc: &Document) -> Rc<Vec<rusty_edit::Region>> {
        let mut cached = self.regions.borrow_mut();
        if let Some((path, version, regions)) = cached.as_ref()
            && *path == doc.path
            && *version == doc.version
        {
            return regions.clone();
        }
        let regions = Rc::new(rusty_edit::fold::regions(&doc.text));
        *cached = Some((doc.path.clone(), doc.version, regions.clone()));
        regions
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
        let line = doc.line_at_row(row);
        self.lenses(&doc).get(&(line as u32)).cloned()
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

    /// Lines `from..to` changed: the rows they are drawn on are asked for
    /// again.
    pub fn changed(&self, from: usize, to: usize) {
        if to <= from {
            return;
        }
        let rows = match self.doc.borrow().as_ref() {
            Some(doc) => {
                let d = doc.borrow();
                d.row_of(from)..d.row_of(to - 1) + 1
            }
            None => return,
        };
        let count = self.row_count();
        for row in rows.start..rows.end.min(count) {
            self.notify.row_changed(row);
        }
    }

    /// Lines added and removed: rows only while nothing is folded, which is
    /// when the two are the same numbers.
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
            .map_or(0, |doc| doc.borrow().rows())
    }

    fn row_data(&self, at: usize) -> Option<Row> {
        let doc = self.doc.borrow().clone()?;
        let doc = doc.borrow();
        if at >= doc.rows() {
            return None;
        }
        let row = doc.line_at_row(at);
        let text = doc.lines.text(&doc.text, row);
        let metrics = self.metrics.borrow();
        // The painter's colours, the server's over them where it has any,
        // placed around the line's hints.
        let painted = doc.painted.get(row).filter(|line| painted_is(line, text));
        let base = painted.cloned().unwrap_or_else(|| Line {
            spans: vec![rusty_edit::Span {
                text: text.to_string(),
                token: Token::Plain,
            }],
        });
        let coloured = rusty_edit::semantic::overlay(base, &semantic_marks(&doc, row));
        let hints = inlays::on(&doc, row);
        let link = self.resting.borrow().link_on(&doc.path, doc.version, row);
        let (pieces, drawn) =
            place_hinted(&metrics, kinds(&coloured.spans, link).into_iter(), hints);
        let end = inlays::end_x(&metrics, text, hints);
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
        // Whether a block opens here, and whether it is collapsed: the
        // chevron beside the number, and the ellipsis after the line.
        let fold = if doc.folds.is_folded(line) {
            2
        } else if self
            .regions(&doc)
            .binary_search_by_key(&line, |r| r.header)
            .is_ok()
        {
            1
        } else {
            0
        };
        let fold_x = end + metrics.ascii();
        Some(Row {
            number: row as i32 + 1,
            fold,
            fold_x,
            breakpoint,
            stopped,
            lens,
            lens_x,
            verdict,
            pieces: ModelRc::new(VecModel::from(pieces)),
            hints: ModelRc::new(VecModel::from(drawn)),
            marks: ModelRc::new(VecModel::from({
                // The guides first: everything else is drawn over them.
                let level = if gutter.no_guides {
                    0
                } else {
                    self.levels(&doc).get(row).copied().unwrap_or(0)
                };
                let tab = rusty_edit::guides::TAB_SIZE as f32 * metrics.ascii();
                let mut all: Vec<Mark> = (0..level)
                    .map(|k| mark(f32::from(k) * tab, 1.0, 8))
                    .collect();
                all.extend(marks(
                    &doc,
                    &metrics,
                    hints,
                    &self.found.borrow(),
                    &self.resting.borrow(),
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

/// What is drawn on `row` besides its text, back to front: find's scope,
/// the name under the caret where else it occurs, find's matches, the
/// selections crossing it, the bracket pair beside the caret, the squiggles
/// of what the servers found, and the carets on it but the primary one. A
/// span that carries on past the line's end takes a space's width there, as
/// every editor shows the line break it holds.
fn marks(
    doc: &Document,
    metrics: &Metrics,
    hints: &[rusty_lsp::InlayHint],
    found: &find::Marks,
    resting: &Resting,
    problems: &[rusty_lsp::FileDiagnostic],
    row: usize,
) -> Vec<Mark> {
    let (start, end) = (doc.lines.start(row), doc.lines.end(row));
    let text = doc.lines.text(&doc.text, row);
    let x_at =
        |at: usize| inlays::caret_x(metrics, text, hints, text[..at - start].chars().count());
    let mut marks = Vec::new();
    let span = |from: usize, to: usize, kind: i32| {
        let (a, b) = (from.max(start), to.min(end));
        if from > end || to < start || a > b {
            return None;
        }
        let x = x_at(a);
        let past = to > end;
        let width = x_at(b) - x + if past { metrics.ascii() } else { 0.0 };
        (kind > 1 || width > 0.0).then(|| mark(x, width, kind))
    };
    // Over the characters themselves, by column, the hints at their edges
    // left out: a washed name and an outlined bracket are about the code.
    let ink = |from: usize, to: usize, kind: i32| {
        let (x, right) = inlays::ink(metrics, text, hints, from, to);
        (right > x).then(|| mark(x, right - x, kind))
    };
    if let Some((from, to)) = found.scope {
        marks.extend(span(from, to, 4));
    }
    for (from, to) in resting.washes_on(&doc.path, doc.version, row) {
        marks.extend(ink(from, to, 10));
    }
    for at in found.on(start, end) {
        let (from, to) = found.matches[at];
        marks.extend(span(
            from,
            to,
            if found.current == Some(at) { 3 } else { 2 },
        ));
    }
    for cursor in &doc.cursors {
        if !cursor.is_caret() && cursor.start() <= end && cursor.end() > start {
            marks.extend(span(cursor.start(), cursor.end(), 0));
        }
    }
    for col in resting.brackets_on(&doc.path, doc.version, row) {
        marks.extend(ink(col, col + 1, 9));
    }
    if let Some((from, to)) = resting.link_on(&doc.path, doc.version, row) {
        marks.extend(ink(from, to, LINK));
    }
    // Worst last, so an error is drawn over the warning it shares a word
    // with; hints are not problems, and are not drawn.
    let mut squiggles: Vec<(rusty_lsp::DiagSeverity, f32, f32)> = problems
        .iter()
        .filter(|d| d.severity != DiagSeverity::Hint)
        .filter_map(|d| {
            let (from, to) = rusty_lsp::squiggle::drawn_on(d, row as u32, text)?;
            let (from, to) = (from as usize, to as usize);
            let count = text.chars().count();
            // One past the line's end is a cell after everything on it,
            // its hints included.
            let end = inlays::end_x(metrics, text, hints);
            let x = if from >= count {
                end
            } else {
                inlays::char_x(metrics, text, hints, from)
            };
            let right = if to > count {
                end + (to - count) as f32 * metrics.ascii()
            } else {
                inlays::caret_x(metrics, text, hints, to)
            };
            Some((d.severity, x, right - x))
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
    let pieces = place(metrics, kinds(&line.spans, None).into_iter());
    Row {
        number: 0,
        fold: 0,
        fold_x: 0.0,
        breakpoint: 0,
        stopped: false,
        lens: 0,
        lens_x: 0.0,
        verdict: 0,
        pieces: ModelRc::new(VecModel::from(pieces)),
        hints: ModelRc::default(),
        marks: ModelRc::default(),
    }
}

/// The server's colours on `line` of `doc`, as marks over its columns.
fn semantic_marks(doc: &Document, line: usize) -> Vec<(u32, u32, Token)> {
    let line = line as u32;
    let start = doc.semantic.partition_point(|s| s.line < line);
    doc.semantic[start..]
        .iter()
        .take_while(|s| s.line == line)
        .filter_map(|s| {
            rusty_edit::semantic::token_of(&s.kind)
                .map(|token| (s.start_col, s.start_col + s.length, token))
        })
        .collect()
}

/// A line of the document drawn on its own — a stuck line — with its
/// number and its colours, and nothing behind them.
pub fn line_row(metrics: &Metrics, doc: &Document, line: usize) -> Row {
    let text = doc.lines.text(&doc.text, line);
    let pieces = match doc.painted.get(line).filter(|l| painted_is(l, text)) {
        Some(painted) => place(metrics, kinds(&painted.spans, None).into_iter()),
        None => place(metrics, std::iter::once((text, kind(Token::Plain)))),
    };
    Row {
        number: line as i32 + 1,
        pieces: ModelRc::new(VecModel::from(pieces)),
        ..painted_row(metrics, &Line::default())
    }
}

/// Whether a painted line is the text on screen — a line typed on since its
/// painting is drawn plain until the repaint catches up.
pub fn painted_is(line: &Line, text: &str) -> bool {
    let mut rest = text;
    for span in &line.spans {
        match rest.strip_prefix(span.text.as_str()) {
            Some(after) => rest = after,
            None => return false,
        }
    }
    rest.is_empty()
}

/// What a name drawn as a link is, as a piece's kind and as a mark's.
const LINK: i32 = 11;

/// A line's spans as they are drawn: each with its token's kind, and the
/// characters `link` covers — columns, from and to — cut out of whatever
/// spans they are in and given the link's.
fn kinds(spans: &[rusty_edit::Span], link: Option<(usize, usize)>) -> Vec<(&str, i32)> {
    let mut out = Vec::with_capacity(spans.len() + 2);
    let mut col = 0;
    for span in spans {
        let (text, token) = (span.text.as_str(), kind(span.token));
        let count = text.chars().count();
        let cut = link.filter(|&(from, to)| from < col + count && to > col);
        if let Some((from, to)) = cut {
            let byte = |c: usize| {
                text.char_indices()
                    .nth(c - col)
                    .map_or(text.len(), |(at, _)| at)
            };
            let (a, b) = (byte(from.max(col)), byte(to.min(col + count)));
            for (piece, k) in [
                (&text[..a], token),
                (&text[a..b], LINK),
                (&text[b..], token),
            ] {
                if !piece.is_empty() {
                    out.push((piece, k));
                }
            }
        } else {
            out.push((text, token));
        }
        col += count;
    }
    out
}

/// Runs of one kind placed along the line. A tab ends a piece: it is drawn
/// as the gap to the next stop, not as a glyph.
fn place<'t>(metrics: &Metrics, spans: impl Iterator<Item = (&'t str, i32)>) -> Vec<Piece> {
    place_hinted(metrics, spans, &[]).0
}

/// [`place`] around a line's hints: a hint ends a piece too, and takes its
/// width before the column it stands at, as `inlays` measures it.
fn place_hinted<'t>(
    metrics: &Metrics,
    spans: impl Iterator<Item = (&'t str, i32)>,
    hints: &[rusty_lsp::InlayHint],
) -> (Vec<Piece>, Vec<Hint>) {
    let mut pieces = Vec::new();
    let mut drawn = Vec::new();
    let mut x = 0.0;
    let mut col = 0usize;
    let mut next = 0;
    for (text, token) in spans {
        let mut start = x;
        let mut run = String::new();
        for ch in text.chars() {
            if hints.get(next).is_some_and(|h| h.col as usize <= col) {
                if !run.is_empty() {
                    pieces.push(piece(std::mem::take(&mut run), start, token));
                }
                hints_at(metrics, hints, &mut next, col, &mut x, &mut drawn);
                start = x;
            }
            col += 1;
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
    hints_at(metrics, hints, &mut next, col, &mut x, &mut drawn);
    (pieces, drawn)
}

/// The hints standing at or before `col` from `next` on, drawn at `x`,
/// which moves past them.
fn hints_at(
    metrics: &Metrics,
    hints: &[rusty_lsp::InlayHint],
    next: &mut usize,
    col: usize,
    x: &mut f32,
    drawn: &mut Vec<Hint>,
) {
    while let Some(hint) = hints.get(*next).filter(|h| h.col as usize <= col) {
        let (left, right) = (
            inlays::pad(metrics, hint.pad_left),
            inlays::pad(metrics, hint.pad_right),
        );
        let width = inlays::width(metrics, hint);
        drawn.push(Hint {
            text: SharedString::from(hint.label.as_str()),
            x: *x + left,
            width: width - left - right,
        });
        *x += width;
        *next += 1;
    }
}

fn piece(text: String, x: f32, kind: i32) -> Piece {
    Piece {
        text: SharedString::from(text),
        x,
        kind,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, token: Token) -> rusty_edit::Span {
        rusty_edit::Span {
            text: text.into(),
            token,
        }
    }

    /// The link takes its characters out of the spans they are in —
    /// whichever, and however many — and leaves the rest as they were;
    /// columns are characters, so a `中` before the name does not move it.
    #[test]
    fn a_link_recolours_exactly_its_characters() {
        let spans = [
            span("let ", Token::Keyword),
            span("中x", Token::Variable),
            span(" = ", Token::Plain),
            span("norm", Token::Function),
            span("()", Token::Punctuation),
        ];
        let plain = kinds(&spans, None);
        assert_eq!(
            plain,
            [("let ", 1), ("中x", 9), (" = ", 0), ("norm", 6), ("()", 8)]
        );
        assert_eq!(
            kinds(&spans, Some((9, 13))),
            [
                ("let ", 1),
                ("中x", 9),
                (" = ", 0),
                ("norm", LINK),
                ("()", 8)
            ]
        );
        assert_eq!(
            kinds(&spans, Some((5, 10))),
            [
                ("let ", 1),
                ("中", 9),
                ("x", LINK),
                (" = ", LINK),
                ("n", LINK),
                ("orm", 6),
                ("()", 8)
            ]
        );
        assert_eq!(kinds(&spans, Some((40, 44))), plain, "past the line");
    }
}
