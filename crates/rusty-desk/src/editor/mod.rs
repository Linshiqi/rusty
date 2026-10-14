//! The code surface's Rust half: the open documents, the one on screen, and
//! everything a key, a click or a paste does to it.
//!
//! Slint draws; this measures, places and decides. A row is handed over as
//! pieces of one token at the x they start at and marks behind them
//! (`rows.rs`), so the view does no text layout, and the caret, a click and
//! a selection are placed by the arithmetic that placed the text
//! (`metrics.rs`). What an edit does is pure arithmetic over the text and
//! the cursors (`cursors.rs`, `pairs.rs`, `indent.rs`, `clip.rs`,
//! `lineops.rs`, under their tests); this module applies it to a
//! [`Document`] and tells the view, the painter and the app what changed.

pub mod backup;
mod clip;
mod cursors;
mod document;
mod find;
mod finding;
mod history;
mod hover;
mod indent;
mod keymap;
mod lineops;
mod lines;
mod metrics;
mod pairs;
mod problems;

pub use problems::is_problem;
mod rows;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    ops::Range,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use rusty_edit::{
    Files,
    text_file::{Charset, LineEnding},
};
use slint::{ComponentHandle, ModelRc};

use crate::{
    AppWindow, Code, Find, language,
    lsp::{Problems, Servers},
    work,
};
use cursors::Cursor;
use document::Change;
pub use document::Document;
use history::Kind;
use keymap::Command;
pub use metrics::Metrics;
use rows::Rows;

type Shared = Rc<RefCell<Option<Rc<RefCell<Document>>>>>;

/// Who is told after an edit, with the edited document's path.
type OnEdit = RefCell<Option<Box<dyn Fn(&str)>>>;
/// Who is told when the caret moves.
type OnCaret = RefCell<Option<Box<dyn Fn()>>>;

/// A press being dragged: what the press itself selected — a point, a word
/// or a line — which every move extends from.
#[derive(Clone, Copy, Debug)]
struct Drag {
    span: (usize, usize),
    unit: Unit,
    /// Which cursor the drag moves: the primary one, or one Alt added.
    cursor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Point,
    Word,
    Line,
}

/// The last press, for counting a double and a triple click.
#[derive(Clone, Copy, Debug)]
struct Press {
    at: Instant,
    line: usize,
    x: f32,
    count: u8,
}

/// How long two presses may be apart, and how far, to be one click count.
const CLICK_GAP: Duration = Duration::from_millis(450);
const CLICK_SLOP: f32 = 4.0;

pub struct Surface {
    files: Arc<Files>,
    open: RefCell<HashMap<String, Rc<RefCell<Document>>>>,
    current: Shared,
    metrics: Rc<RefCell<Metrics>>,
    rows: Rc<Rows>,
    window: slint::Weak<AppWindow>,
    on_edit: OnEdit,
    on_caret: OnCaret,
    /// Paths with a repaint on its way: one at a time each.
    painting: RefCell<HashSet<String>>,
    drag: Cell<Option<Drag>>,
    press: Cell<Option<Press>>,
    /// The last line copied with nothing selected, which pastes as a line.
    copied_line: RefCell<Option<String>>,
    /// The lines drawn with marks, to redraw when the cursors leave them.
    marked: RefCell<Range<usize>>,
    /// The widest line seen, for how far the view scrolls sideways.
    widest: Cell<f32>,
    /// The find bar, and what it found, which the rows draw.
    finding: RefCell<finding::Finding>,
    found: Rc<RefCell<find::Marks>>,
    /// The servers told about every document, and what they found.
    servers: Rc<Servers>,
    problems: Problems,
    /// The lines squiggles are drawn on in the document on screen.
    squiggled: RefCell<Vec<usize>>,
    hovering: hover::Hovering,
}

impl Surface {
    pub fn new(
        window: &AppWindow,
        files: Arc<Files>,
        servers: Rc<Servers>,
        problems: Problems,
    ) -> Rc<Surface> {
        let current: Shared = Rc::default();
        let metrics = Rc::new(RefCell::new(Metrics::measure(window)));
        let found: Rc<RefCell<find::Marks>> = Rc::default();
        let rows = Rc::new(Rows::new(
            current.clone(),
            metrics.clone(),
            found.clone(),
            problems.clone(),
        ));
        window
            .global::<Code>()
            .set_rows(ModelRc::from(rows.clone()));
        let surface = Rc::new(Surface {
            files,
            open: RefCell::default(),
            current,
            metrics,
            rows,
            window: window.as_weak(),
            on_edit: RefCell::default(),
            on_caret: RefCell::default(),
            painting: RefCell::default(),
            drag: Cell::new(None),
            press: Cell::new(None),
            copied_line: RefCell::default(),
            marked: RefCell::new(0..0),
            widest: Cell::new(0.0),
            finding: RefCell::default(),
            found,
            servers,
            problems,
            squiggled: RefCell::default(),
            hovering: hover::Hovering::default(),
        });
        surface.wire(window);
        surface
    }

    pub fn on_edit(&self, tell: impl Fn(&str) + 'static) {
        *self.on_edit.borrow_mut() = Some(Box::new(tell));
    }

    pub fn on_caret(&self, tell: impl Fn() + 'static) {
        *self.on_caret.borrow_mut() = Some(Box::new(tell));
    }

    // ---- documents -------------------------------------------------------

    pub fn holds(&self, path: &str) -> bool {
        self.open.borrow().contains_key(path)
    }

    pub fn is_dirty(&self, path: &str) -> bool {
        self.open
            .borrow()
            .get(path)
            .is_some_and(|doc| doc.borrow().dirty())
    }

    /// The path of the document on screen.
    pub fn current_path(&self) -> Option<String> {
        Some(self.doc()?.borrow().path.clone())
    }

    /// A document as it is to be written: text, charset, line endings.
    pub fn draft_of(&self, path: &str) -> Option<(String, Charset, LineEnding)> {
        let open = self.open.borrow();
        let doc = open.get(path)?.borrow();
        Some((doc.text.clone(), doc.charset, doc.ending))
    }

    /// A document's draft as it would be backed up, when it differs from
    /// the disk's.
    pub fn backup_of(&self, path: &str) -> Option<backup::Backup> {
        let open = self.open.borrow();
        backup_of(&open.get(path)?.borrow())
    }

    /// Every open document with its draft to keep, or `None` where its
    /// text is the disk's and any old backup is to go.
    pub fn backups(&self) -> Vec<(String, Option<backup::Backup>)> {
        self.open
            .borrow()
            .values()
            .map(|doc| {
                let doc = doc.borrow();
                (doc.path.clone(), backup_of(&doc))
            })
            .collect()
    }

    /// Which text of `path` an answer about it was asked about.
    pub fn version_of(&self, path: &str) -> Option<u64> {
        Some(self.open.borrow().get(path)?.borrow().version)
    }

    /// Replace `path`'s text — a format's answer — when it is still the
    /// text at `version`; the caret keeps its line and column. Undone in
    /// one step, as VS Code's format is.
    pub fn replace(&self, path: &str, version: u64, text: String) -> bool {
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return false;
        };
        let change = {
            let mut d = doc.borrow_mut();
            if d.version != version {
                return false;
            }
            let (line, col) = d.lines.position(&d.text, d.primary().head);
            let lines = lines::Lines::of(&text);
            let at = lines.byte(&text, line, col);
            d.edit(text, vec![Cursor::caret(at)], Kind::Other)
        };
        if let Some(change) = change {
            self.edited(&doc, change);
        }
        true
    }

    /// A draft put back over a document just opened: the disk's text is
    /// what it was saved as, and the draft is one undo away from it.
    pub fn restore(&self, path: &str, draft: String) {
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return;
        };
        let change = doc
            .borrow_mut()
            .edit(draft, vec![Cursor::caret(0)], Kind::Other);
        if let Some(change) = change {
            self.edited(&doc, change);
        }
    }

    /// A document changed by something other than a key: on screen it is
    /// redrawn as a key's edit is; off screen it is painted and said.
    fn edited(&self, doc: &Rc<RefCell<Document>>, change: Change) {
        if self.doc().is_some_and(|current| Rc::ptr_eq(&current, doc)) {
            self.after_edit(doc, change);
        } else {
            self.paint(doc);
            let path = doc.borrow().path.clone();
            self.servers.changed(&path, &doc.borrow().text);
            self.tell_edit(&path);
        }
    }

    /// The disk now holds `text` for `path`.
    pub fn saved(&self, path: &str, text: &str) {
        if let Some(doc) = self.open.borrow().get(path) {
            doc.borrow_mut().mark_saved(text);
        }
        self.servers.saved(path);
        self.tell_edit(path);
    }

    /// Open a document and put it on screen; its colours follow from a
    /// painting off the UI thread.
    pub fn show_new(&self, document: Document) {
        let path = document.path.clone();
        self.add(document);
        self.show(&path);
    }

    /// Open a document without putting it on screen.
    pub fn add(&self, document: Document) {
        if let Some(window) = self.window.upgrade() {
            self.metrics.borrow_mut().learn(&window, &document.text);
        }
        let path = document.path.clone();
        self.servers.opened(&path, &document.text);
        let doc = Rc::new(RefCell::new(document));
        self.open.borrow_mut().insert(path, doc.clone());
        self.paint(&doc);
    }

    pub fn show(&self, path: &str) {
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return;
        };
        self.widest.set(self.metrics.borrow().widest(&doc.borrow()));
        *self.current.borrow_mut() = Some(doc);
        *self.marked.borrow_mut() = 0..0;
        self.squiggled.borrow_mut().clear();
        self.finding.borrow_mut().scope = None;
        self.research(None);
        self.rows.reset();
        if let Some(window) = self.window.upgrade() {
            let code = window.global::<Code>();
            code.set_has_document(true);
            code.set_content_width(self.widest.get() + 200.0);
            code.set_scroll_y(0.0);
            code.set_scroll_x(0.0);
            self.show_caret();
            window.invoke_focus_code();
        }
    }

    pub fn show_nothing(&self) {
        *self.current.borrow_mut() = None;
        self.rows.reset();
        if let Some(window) = self.window.upgrade() {
            window.global::<Code>().set_has_document(false);
        }
    }

    pub fn close(&self, path: &str) {
        if self.open.borrow_mut().remove(path).is_some() {
            self.servers.closed(path);
        }
        let showing = self
            .current
            .borrow()
            .as_ref()
            .is_some_and(|doc| doc.borrow().path == path);
        if showing {
            self.show_nothing();
        }
    }

    /// Ctrl+G's line and column, counted from one: the caret there, and the
    /// line in the middle of the view when it was not on screen.
    pub fn go_to(&self, line: usize, col: Option<usize>) {
        let Some(doc) = self.doc() else { return };
        let (at, line) = {
            let d = doc.borrow();
            let line = line.saturating_sub(1).min(d.lines.count() - 1);
            let col = col.map_or(0, |col| col.saturating_sub(1));
            (d.lines.byte(&d.text, line, col), line)
        };
        if let Some(window) = self.window.upgrade() {
            let code = window.global::<Code>();
            let (row, view) = (code.get_row_height(), code.get_view_height());
            let top = line as f32 * row;
            let scrolled = -code.get_scroll_y();
            if top < scrolled || top + row > scrolled + view {
                code.set_scroll_y(-(top - (view - row) / 2.0).max(0.0));
            }
        }
        self.set_cursors(vec![Cursor::caret(at)]);
        if let Some(window) = self.window.upgrade() {
            window.invoke_focus_code();
        }
    }

    /// How many lines the document on screen has.
    pub fn line_count(&self) -> Option<usize> {
        Some(self.doc()?.borrow().lines.count())
    }

    /// The caret on screen as people number it, and what the document is
    /// written as: line and column from one, charset, line endings.
    pub fn status(&self) -> Option<(usize, usize, &'static str, &'static str)> {
        let doc = self.current.borrow().clone()?;
        let doc = doc.borrow();
        let (line, col) = doc.lines.position(&doc.text, doc.primary().head);
        Some((line + 1, col + 1, doc.charset.label(), doc.ending.label()))
    }

    fn doc(&self) -> Option<Rc<RefCell<Document>>> {
        self.current.borrow().clone()
    }

    // ---- telling ---------------------------------------------------------

    fn tell_edit(&self, path: &str) {
        if let Some(tell) = self.on_edit.borrow().as_ref() {
            tell(path);
        }
    }

    fn tell_caret(&self) {
        if let Some(tell) = self.on_caret.borrow().as_ref() {
            tell();
        }
    }

    // ---- the view's callbacks --------------------------------------------

    fn wire(self: &Rc<Self>, window: &AppWindow) {
        self.wire_find(&window.global::<Find>());
        let code = window.global::<Code>();
        self.wire_hover(&code);
        let this = Rc::downgrade(self);
        code.on_typed(move |text| {
            if let Some(this) = this.upgrade() {
                this.hide_hover();
                this.type_text(&text);
            }
        });
        let this = Rc::downgrade(self);
        code.on_key(move |event| {
            let Some(this) = this.upgrade() else {
                return false;
            };
            let m = event.modifiers;
            // Any key puts the card away: it is about the text as it was.
            this.hide_hover();
            let page = this.page();
            match keymap::command(&event.text, m.control || m.meta, m.alt, m.shift, page) {
                Some(command) if this.doc().is_some() => {
                    this.run(command);
                    true
                }
                _ => false,
            }
        });
        let this = Rc::downgrade(self);
        code.on_pointer(move |kind, row, x, y, modifiers| {
            if let Some(this) = this.upgrade() {
                this.pointer(kind, row as usize, x, y, modifiers.shift, modifiers.alt);
            }
        });
    }

    /// How many rows a page is.
    fn page(&self) -> isize {
        let Some(window) = self.window.upgrade() else {
            return 20;
        };
        let code = window.global::<Code>();
        ((code.get_view_height() / code.get_row_height()) as isize - 1).max(1)
    }

    // ---- commands --------------------------------------------------------

    fn run(&self, command: Command) {
        let Some(doc) = self.doc() else { return };
        let (text, cursors) = {
            let d = doc.borrow();
            (d.text.clone(), d.cursors.clone())
        };
        let primary = cursors[0];
        let single = cursors.len() == 1;
        // Each arm answers an edit (the new text, the cursors, what kind),
        // or moves the cursors itself and answers nothing.
        let edit: Option<(String, Vec<Cursor>, Kind)> = match command {
            Command::Move(motion, extend) => {
                self.set_cursors(cursors::moved(&text, &cursors, motion, extend));
                None
            }
            Command::Vertical(lines, extend) => {
                self.vertical(lines, extend);
                None
            }
            Command::Edge { end, extend } => {
                let at = if end { text.len() } else { 0 };
                let anchor = if extend { primary.anchor } else { at };
                self.set_cursors(vec![Cursor { anchor, head: at }]);
                None
            }
            Command::SelectAll => {
                self.set_cursors(vec![Cursor {
                    anchor: 0,
                    head: text.len(),
                }]);
                None
            }
            Command::SingleCursor => {
                if single && self.finding_open() {
                    self.find_close();
                } else {
                    self.set_cursors(vec![primary]);
                }
                None
            }
            Command::Backspace { word } => {
                let pair = (single && primary.is_caret() && !word)
                    .then(|| pairs::on_backspace(&text, primary.head))
                    .flatten();
                Some(match pair {
                    Some(edit) => applied(&text, edit, Kind::Deleting),
                    None => {
                        let (t, c) = cursors::erased(&text, &cursors, false, word);
                        (t, c, Kind::Deleting)
                    }
                })
            }
            Command::Delete { word } => {
                let (t, c) = cursors::erased(&text, &cursors, true, word);
                Some((t, c, Kind::Deleting))
            }
            Command::Enter => Some(if single && primary.is_caret() {
                applied(&text, pairs::on_enter(&text, primary.head), Kind::Other)
            } else {
                let (t, c) = cursors::broken(&text, &cursors);
                (t, c, Kind::Other)
            }),
            Command::Tab => {
                let shifted = (single
                    && indent::shifts_lines(&text, primary.start(), primary.end(), false))
                .then(|| indent::shift(&text, primary.start(), primary.end(), false))
                .flatten();
                Some(match shifted {
                    Some(edit) => applied(&text, edit, Kind::Other),
                    None => {
                        let (t, c) = cursors::typed(&text, &cursors, "    ");
                        (t, c, Kind::Typing)
                    }
                })
            }
            Command::Indent | Command::Outdent => {
                let back = command == Command::Outdent;
                indent::shift(&text, primary.start(), primary.end(), back)
                    .map(|edit| applied(&text, edit, Kind::Other))
            }
            Command::Copy => {
                self.copy(&text, &cursors);
                None
            }
            Command::Cut => Some(if single && primary.is_caret() {
                let (line, edit) = clip::cut_line(&text, primary.head);
                clipboard_set(&line);
                *self.copied_line.borrow_mut() = Some(line);
                applied(&text, edit, Kind::Other)
            } else {
                clipboard_set(&cursors::copied(&text, &cursors));
                let (t, c) = cursors::cut(&text, &cursors);
                (t, c, Kind::Other)
            }),
            Command::Paste => clipboard_get().map(|clip| {
                let remembered = self.copied_line.borrow().clone();
                if single
                    && primary.is_caret()
                    && clip::is_copied_line(&clip, remembered.as_deref())
                {
                    applied(
                        &text,
                        clip::paste_line(&text, primary.head, &clip),
                        Kind::Other,
                    )
                } else {
                    let (t, c) = cursors::pasted(&text, &cursors, &clip);
                    (t, c, Kind::Other)
                }
            }),
            Command::Undo | Command::Redo => {
                let change = if command == Command::Undo {
                    doc.borrow_mut().undo()
                } else {
                    doc.borrow_mut().redo()
                };
                if let Some(change) = change {
                    self.after_edit(&doc, change);
                }
                None
            }
            Command::NextMatch => {
                let last = *cursors.last().unwrap_or(&primary);
                if last.is_caret() {
                    // The first press takes the word the caret is in.
                    if let Some((start, end)) = cursors::word_at(&text, last.head) {
                        let mut next = cursors.clone();
                        if let Some(slot) = next.last_mut() {
                            *slot = Cursor {
                                anchor: start,
                                head: end,
                            };
                        }
                        self.set_cursors(next);
                    }
                } else if let Some(found) = cursors::next_match(&text, &cursors) {
                    let mut next = cursors.clone();
                    next.push(found);
                    self.set_cursors(next);
                    self.reveal(found.head);
                }
                None
            }
            Command::AllMatches => {
                let target = if primary.is_caret() {
                    cursors::word_at(&text, primary.head)
                        .map(|(anchor, head)| Cursor { anchor, head })
                } else {
                    Some(primary)
                };
                if let Some(target) = target {
                    self.set_cursors(cursors::every_match(&text, target));
                }
                None
            }
            Command::AddCursor { up } => {
                if let Some(added) = cursors::beside_vertically(&text, &cursors, up) {
                    let mut next = cursors.clone();
                    next.push(added);
                    self.set_cursors(next);
                    self.reveal(added.head);
                }
                None
            }
            Command::MoveLines { up } => {
                lineops::moved(&text, &cursors, up).map(|(t, c)| (t, c, Kind::Other))
            }
            Command::CopyLines { up } => {
                let (t, c) = lineops::copied(&text, &cursors, up);
                Some((t, c, Kind::Other))
            }
            Command::DeleteLines => {
                let (t, c) = lineops::deleted(&text, &cursors);
                Some((t, c, Kind::Other))
            }
            Command::OpenLine { above } => {
                let (t, c) = lineops::opened(&text, &cursors, above);
                Some((t, c, Kind::Other))
            }
            Command::SelectLine => {
                self.set_cursors(lineops::line_selected(&text, &cursors));
                None
            }
            Command::ToggleComment => {
                let path = doc.borrow().path.clone();
                language::of(&path)
                    .and_then(|language| language.comment)
                    .and_then(|marker| lineops::commented(&text, &cursors, marker))
                    .map(|(t, c)| (t, c, Kind::Other))
            }
        };
        if let Some((text, cursors, kind)) = edit {
            let change = doc.borrow_mut().edit(text, cursors::merged(&cursors), kind);
            match change {
                Some(change) => self.after_edit(&doc, change),
                None => self.cursors_moved(),
            }
        }
    }

    /// Text typed or composed: at every cursor, with the bracket pairs' rules
    /// for one character at one cursor.
    fn type_text(&self, typed: &str) {
        let Some(doc) = self.doc() else { return };
        if let Some(window) = self.window.upgrade() {
            self.metrics.borrow_mut().learn(&window, typed);
        }
        let (text, cursors) = {
            let d = doc.borrow();
            (d.text.clone(), d.cursors.clone())
        };
        let mut chars = typed.chars();
        let one = match (chars.next(), chars.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        };
        let pair = match one {
            Some(ch) if cursors.len() == 1 => {
                pairs::on_type(&text, cursors[0].start(), cursors[0].end(), ch)
            }
            _ => None,
        };
        let (text, cursors) = match pair {
            Some(edit) => {
                let (t, c, _) = applied(&text, edit, Kind::Typing);
                (t, c)
            }
            None => cursors::typed(&text, &cursors, typed),
        };
        let change = doc.borrow_mut().edit(text, cursors, Kind::Typing);
        if let Some(change) = change {
            self.after_edit(&doc, change);
        }
    }

    fn copy(&self, text: &str, cursors: &[Cursor]) {
        let copied = if cursors.len() == 1 && cursors[0].is_caret() {
            let line = clip::copy_line(text, cursors[0].head);
            *self.copied_line.borrow_mut() = Some(line.clone());
            line
        } else {
            cursors::copied(text, cursors)
        };
        clipboard_set(&copied);
    }

    /// Up and Down by `lines`, to the column the run started at — measured
    /// in pixels, so a caret below a `中` lands under it and not a column
    /// of ASCII off.
    fn vertical(&self, lines: isize, extend: bool) {
        let Some(doc) = self.doc() else { return };
        {
            let metrics = self.metrics.borrow();
            let mut d = doc.borrow_mut();
            let last = d.lines.count() as isize - 1;
            let x_of = |d: &Document, at: usize| {
                let (line, col) = d.lines.position(&d.text, at);
                metrics.x_of(d.lines.text(&d.text, line), col)
            };
            let want = d.want_x.unwrap_or_else(|| x_of(&d, d.cursors[0].head));
            let moved: Vec<Cursor> = d
                .cursors
                .iter()
                .enumerate()
                .map(|(index, cursor)| {
                    let x = if index == 0 {
                        want
                    } else {
                        x_of(&d, cursor.head)
                    };
                    let target = d.lines.line_of(cursor.head) as isize + lines;
                    let head = if target < 0 {
                        0
                    } else if target > last {
                        d.text.len()
                    } else {
                        let target = target as usize;
                        let there = d.lines.text(&d.text, target);
                        d.lines.byte(&d.text, target, metrics.column_at(there, x))
                    };
                    let anchor = if extend { cursor.anchor } else { head };
                    Cursor { anchor, head }
                })
                .collect();
            d.set_cursors(cursors::merged(&moved));
            d.want_x = Some(want);
        }
        self.cursors_moved();
    }

    fn set_cursors(&self, cursors: Vec<Cursor>) {
        let Some(doc) = self.doc() else { return };
        doc.borrow_mut().set_cursors(cursors::merged(&cursors));
        self.cursors_moved();
    }

    // ---- the pointer -----------------------------------------------------

    fn pointer(&self, kind: i32, row: usize, x: f32, y: f32, shift: bool, alt: bool) {
        let Some(doc) = self.doc() else { return };
        let row_height = self
            .window
            .upgrade()
            .map_or(19.0, |w| w.global::<Code>().get_row_height());
        let (line, at) = {
            let d = doc.borrow();
            let last = d.lines.count() as isize - 1;
            let line = (row as isize + (y / row_height).floor() as isize).clamp(0, last) as usize;
            let text = d.lines.text(&d.text, line);
            let col = self.metrics.borrow().column_at(text, x.max(0.0));
            (line, d.lines.byte(&d.text, line, col))
        };
        if kind == 0 {
            self.hide_hover();
        }
        match kind {
            0 => self.press(&doc, line, x, at, shift, alt),
            1 => self.drag_to(&doc, at),
            _ => self.drag.set(None),
        }
    }

    fn press(
        &self,
        doc: &Rc<RefCell<Document>>,
        line: usize,
        x: f32,
        at: usize,
        shift: bool,
        alt: bool,
    ) {
        let now = Instant::now();
        let count = match self.press.get() {
            Some(last)
                if now.duration_since(last.at) < CLICK_GAP
                    && last.line == line
                    && (last.x - x).abs() < CLICK_SLOP =>
            {
                last.count % 3 + 1
            }
            _ => 1,
        };
        self.press.set(Some(Press {
            at: now,
            line,
            x,
            count,
        }));
        let unit = match count {
            2 => Unit::Word,
            3 => Unit::Line,
            _ => Unit::Point,
        };
        let (text, mut cursors) = {
            let d = doc.borrow();
            (d.text.clone(), d.cursors.clone())
        };
        let span = unit_span(&text, at, unit);
        let drag = if alt && unit == Unit::Point {
            // Alt adds a cursor, and dragging moves only it.
            cursors.push(Cursor::caret(at));
            Drag {
                span,
                unit,
                cursor: cursors.len() - 1,
            }
        } else if shift {
            // Shift extends the primary selection from where it was begun.
            cursors.truncate(1);
            cursors[0].head = at;
            Drag {
                span: (cursors[0].anchor, cursors[0].anchor),
                unit,
                cursor: 0,
            }
        } else {
            cursors = vec![Cursor {
                anchor: span.0,
                head: span.1,
            }];
            Drag {
                span,
                unit,
                cursor: 0,
            }
        };
        self.drag.set(Some(drag));
        doc.borrow_mut().set_cursors(cursors);
        self.cursors_moved();
    }

    fn drag_to(&self, doc: &Rc<RefCell<Document>>, at: usize) {
        let Some(drag) = self.drag.get() else { return };
        let (text, mut cursors) = {
            let d = doc.borrow();
            (d.text.clone(), d.cursors.clone())
        };
        let here = unit_span(&text, at, drag.unit);
        let cursor = if here.0 < drag.span.0 {
            Cursor {
                anchor: drag.span.1,
                head: here.0,
            }
        } else {
            Cursor {
                anchor: drag.span.0,
                head: here.1.max(drag.span.1),
            }
        };
        if let Some(slot) = cursors.get_mut(drag.cursor) {
            *slot = cursor;
        }
        doc.borrow_mut().set_cursors(cursors);
        self.cursors_moved();
    }

    // ---- after a change --------------------------------------------------

    fn after_edit(&self, doc: &Rc<RefCell<Document>>, change: Change) {
        if change.added > change.removed {
            self.rows
                .added(change.first + change.removed, change.added - change.removed);
        } else if change.removed > change.added {
            self.rows
                .removed(change.first + change.added, change.removed - change.added);
        }
        self.rows.changed(change.first, change.first + change.added);
        let widest = {
            let d = doc.borrow();
            let metrics = self.metrics.borrow();
            (change.first..change.first + change.added)
                .map(|line| metrics.x_of(d.lines.text(&d.text, line), usize::MAX))
                .fold(self.widest.get(), f32::max)
        };
        if widest > self.widest.get() {
            self.widest.set(widest);
            if let Some(window) = self.window.upgrade() {
                window.global::<Code>().set_content_width(widest + 200.0);
            }
        }
        self.paint(doc);
        let path = doc.borrow().path.clone();
        self.servers.changed(&path, &doc.borrow().text);
        self.tell_edit(&path);
        self.research(Some(change));
        self.cursors_moved();
    }

    /// The cursors moved (or the text under them changed): redraw the rows
    /// their marks were and are on, and put the caret on screen.
    fn cursors_moved(&self) {
        let Some(doc) = self.doc() else { return };
        let now = {
            let d = doc.borrow();
            let first = d
                .cursors
                .iter()
                .map(|c| d.lines.line_of(c.start()))
                .min()
                .unwrap_or(0);
            let last = d
                .cursors
                .iter()
                .map(|c| d.lines.line_of(c.end()))
                .max()
                .unwrap_or(0);
            first..last + 1
        };
        let was = self.marked.replace(now.clone());
        let span = was.start.min(now.start)..was.end.max(now.end);
        if span.len() > 400 {
            self.rows.reset();
        } else {
            self.rows.changed(was.start, was.end);
            self.rows.changed(now.start, now.end);
        }
        self.find_follow_selection();
        self.show_caret();
    }

    /// Scroll so `at` is on screen.
    fn reveal(&self, at: usize) {
        let Some(doc) = self.doc() else { return };
        let (line, x) = {
            let d = doc.borrow();
            let (line, col) = d.lines.position(&d.text, at);
            (
                line,
                self.metrics.borrow().x_of(d.lines.text(&d.text, line), col),
            )
        };
        self.scroll_to(line, x);
    }

    fn show_caret(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(doc) = self.doc() else { return };
        let (line, x) = {
            let d = doc.borrow();
            let (line, col) = d.lines.position(&d.text, d.primary().head);
            (
                line,
                self.metrics.borrow().x_of(d.lines.text(&d.text, line), col),
            )
        };
        let code = window.global::<Code>();
        code.set_caret_line(line as i32);
        code.set_caret_x(x);
        self.scroll_to(line, x);
        self.tell_caret();
    }

    fn scroll_to(&self, line: usize, x: f32) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let code = window.global::<Code>();
        let row = code.get_row_height();
        let top = line as f32 * row;
        let (view, scrolled) = (code.get_view_height(), -code.get_scroll_y());
        if top < scrolled {
            code.set_scroll_y(-top);
        } else if view > 0.0 && top + row > scrolled + view {
            code.set_scroll_y(-(top + row - view));
        }
        // Sideways: keep the caret a few characters in from either edge,
        // past the gutter.
        let gutter = 64.0;
        let margin = 4.0 * self.metrics.borrow().ascii();
        let (width, across) = (code.get_view_width(), -code.get_scroll_x());
        if width > 0.0 {
            if x < across + margin {
                code.set_scroll_x(-(x - margin).max(0.0));
            } else if x + gutter + margin > across + width {
                code.set_scroll_x(-(x + gutter + margin - width));
            }
        }
    }

    // ---- painting --------------------------------------------------------

    /// Repaint `doc` off the UI thread: one repaint on its way per document,
    /// and when it lands on a text since edited, another for the new text.
    fn paint(&self, doc: &Rc<RefCell<Document>>) {
        let (path, text, base, stale, version) = {
            let d = doc.borrow();
            let stale = d
                .unpainted
                .clone()
                .map(|lines| (lines.start as u32, lines.end as u32));
            (d.path.clone(), d.text.clone(), d.paint, stale, d.version)
        };
        if !self.painting.borrow_mut().insert(path.clone()) {
            return;
        }
        let files = self.files.clone();
        let asked = path.clone();
        work::spawn(
            move || files.repaint(&asked, &text, base, stale),
            move |app, repaint| app.editor().painted(&path, version, repaint),
        );
    }

    /// A repaint has come back for `path`, painted for the text at
    /// `version`.
    pub fn painted(&self, path: &str, version: u64, repaint: rusty_edit::Repaint) {
        self.painting.borrow_mut().remove(path);
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return;
        };
        let from = repaint.from as usize;
        let count = repaint.lines.len();
        let placed = doc
            .borrow_mut()
            .painted_from(version, repaint.version, from, repaint.lines);
        if !placed {
            self.paint(&doc);
            return;
        }
        let showing = self.doc().is_some_and(|current| Rc::ptr_eq(&current, &doc));
        if showing {
            self.rows.changed(from, from + count);
        }
    }
}

/// What a press selected at `at`: the point, its word, or its line.
fn unit_span(text: &str, at: usize, unit: Unit) -> (usize, usize) {
    match unit {
        Unit::Point => (at, at),
        Unit::Word => cursors::word_at(text, at).unwrap_or((at, at)),
        Unit::Line => {
            let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
            let end = text[at..].find('\n').map_or(text.len(), |i| at + i + 1);
            (start, end)
        }
    }
}

/// A [`pairs::Edit`] applied: the text after it and the one cursor it
/// leaves, which is a selection when the edit says what to select.
fn applied(text: &str, edit: pairs::Edit, kind: Kind) -> (String, Vec<Cursor>, Kind) {
    let mut out = String::with_capacity(text.len() + edit.text.len());
    out.push_str(&text[..edit.range.0]);
    out.push_str(&edit.text);
    out.push_str(&text[edit.range.1..]);
    let cursor = match edit.select {
        Some((anchor, head)) => Cursor { anchor, head },
        None => Cursor::caret(edit.caret),
    };
    (out, vec![cursor], kind)
}

fn backup_of(doc: &Document) -> Option<backup::Backup> {
    doc.dirty().then(|| backup::Backup {
        path: doc.path.clone(),
        text: doc.text.clone(),
        charset: doc.charset,
        ending: doc.ending,
    })
}

fn clipboard_set(text: &str) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text.to_string());
    }
}

fn clipboard_get() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}
