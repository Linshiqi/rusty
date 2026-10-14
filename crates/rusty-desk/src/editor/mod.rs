//! The code surface's Rust half: the documents, where every character sits,
//! and the rows the view draws.
//!
//! Slint draws; this measures and places. A row is handed over as pieces —
//! a run of one kind of token at the x it starts at — so the view needs no
//! text layout of its own, and the caret, a click and every overlay are
//! placed by the same arithmetic ([`Metrics`]) that placed the text.

mod metrics;
mod rows;

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use rusty_edit::{Files, Line};
use slint::{ComponentHandle, ModelRc, SharedString, platform::Key};

use crate::{AppWindow, Code};
pub use metrics::Metrics;
use rows::Rows;

/// One open file and where the caret is in it.
pub struct Document {
    pub path: String,
    /// The text, split at `\n` — as the painting splits it, so line `n` here
    /// is line `n` there.
    pub lines: Vec<String>,
    pub painted: Vec<Line>,
    pub paint: Option<u32>,
    /// Line and character (not byte) of the caret.
    pub caret: (usize, usize),
    /// The text as it is on disk, to know whether the draft differs.
    pub saved: String,
}

impl Document {
    pub fn from_disk(opened: rusty_edit::Document) -> Document {
        Document {
            path: opened.path,
            lines: opened.text.split('\n').map(str::to_string).collect(),
            painted: opened.lines,
            paint: opened.paint,
            caret: (0, 0),
            saved: opened.text,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn dirty(&self) -> bool {
        self.text() != self.saved
    }
}

type Shared = Rc<RefCell<Option<Rc<RefCell<Document>>>>>;

/// Who is told after an edit, with the edited document's path.
type OnEdit = RefCell<Option<Box<dyn Fn(&str)>>>;

/// The surface: the open documents, the one on screen, and its rows.
pub struct Surface {
    files: Arc<Files>,
    open: RefCell<HashMap<String, Rc<RefCell<Document>>>>,
    current: Shared,
    metrics: Rc<RefCell<Metrics>>,
    rows: Rc<Rows>,
    window: slint::Weak<AppWindow>,
    /// Told after every edit, with the edited document's path.
    on_edit: OnEdit,
    /// Told whenever the caret moves, for the status bar.
    on_caret: RefCell<Option<Box<dyn Fn()>>>,
}

impl Surface {
    pub fn new(window: &AppWindow, files: Arc<Files>) -> Rc<Surface> {
        let current: Shared = Rc::default();
        let metrics = Rc::new(RefCell::new(Metrics::measure(window)));
        let rows = Rc::new(Rows::new(current.clone(), metrics.clone()));
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
        });
        surface.wire(window);
        surface
    }

    pub fn files(&self) -> Arc<Files> {
        self.files.clone()
    }

    pub fn on_edit(&self, tell: impl Fn(&str) + 'static) {
        *self.on_edit.borrow_mut() = Some(Box::new(tell));
    }

    pub fn on_caret(&self, tell: impl Fn() + 'static) {
        *self.on_caret.borrow_mut() = Some(Box::new(tell));
    }

    /// Whether `path` is open here.
    pub fn holds(&self, path: &str) -> bool {
        self.open.borrow().contains_key(path)
    }

    pub fn is_dirty(&self, path: &str) -> bool {
        self.open
            .borrow()
            .get(path)
            .is_some_and(|doc| doc.borrow().dirty())
    }

    /// Put a document read from the disk on screen.
    pub fn show_new(&self, document: Document) {
        let path = document.path.clone();
        if let Some(window) = self.window.upgrade() {
            self.metrics.borrow_mut().learn(&window, &document.saved);
        }
        let doc = Rc::new(RefCell::new(document));
        self.open.borrow_mut().insert(path.clone(), doc);
        self.show(&path);
    }

    /// Put an open document on screen.
    pub fn show(&self, path: &str) {
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return;
        };
        let width = self.metrics.borrow().content_width(&doc.borrow().lines);
        *self.current.borrow_mut() = Some(doc);
        self.rows.reset();
        if let Some(window) = self.window.upgrade() {
            let code = window.global::<Code>();
            code.set_content_width(width + 200.0);
            code.set_scroll_y(0.0);
            self.show_caret();
            window.invoke_focus_code();
        }
    }

    /// Nothing on screen.
    pub fn show_nothing(&self) {
        *self.current.borrow_mut() = None;
        self.rows.reset();
    }

    pub fn close(&self, path: &str) {
        self.open.borrow_mut().remove(path);
        let showing = self
            .current
            .borrow()
            .as_ref()
            .is_some_and(|doc| doc.borrow().path == path);
        if showing {
            self.show_nothing();
        }
    }

    /// Every open document whose draft differs from the disk: path and text.
    pub fn dirty_drafts(&self) -> Vec<(String, String)> {
        let mut drafts: Vec<(String, String)> = self
            .open
            .borrow()
            .values()
            .filter_map(|doc| {
                let doc = doc.borrow();
                let text = doc.text();
                (text != doc.saved).then(|| (doc.path.clone(), text))
            })
            .collect();
        drafts.sort();
        drafts
    }

    /// The document on screen's path and text, for a save.
    pub fn draft(&self) -> Option<(String, String)> {
        let doc = self.current.borrow().clone()?;
        let doc = doc.borrow();
        Some((doc.path.clone(), doc.text()))
    }

    /// The disk now holds `text` for `path`.
    pub fn saved(&self, path: &str, text: String) {
        if let Some(doc) = self.open.borrow().get(path) {
            doc.borrow_mut().saved = text;
        }
        self.tell(path);
    }

    /// The caret on screen, as people number them: line and column from one.
    pub fn caret(&self) -> Option<(usize, usize)> {
        let doc = self.current.borrow().clone()?;
        let (line, col) = doc.borrow().caret;
        Some((line + 1, col + 1))
    }

    fn tell(&self, path: &str) {
        if let Some(tell) = self.on_edit.borrow().as_ref() {
            tell(path);
        }
    }

    fn wire(self: &Rc<Self>, window: &AppWindow) {
        let code = window.global::<Code>();
        let this = Rc::downgrade(self);
        code.on_typed(move |text| {
            if let Some(this) = this.upgrade() {
                this.insert(&text);
            }
        });
        let this = Rc::downgrade(self);
        code.on_key(move |event| {
            this.upgrade().is_some_and(|this| {
                this.key(&event.text, event.modifiers.control || event.modifiers.meta)
            })
        });
        let this = Rc::downgrade(self);
        code.on_pressed(move |line, x| {
            if let Some(this) = this.upgrade() {
                let Some(doc) = this.current.borrow().clone() else {
                    return;
                };
                let col = {
                    let doc = doc.borrow();
                    let text = doc.lines.get(line as usize).map_or("", String::as_str);
                    this.metrics.borrow().column_at(text, x)
                };
                doc.borrow_mut().caret = (line as usize, col);
                this.show_caret();
            }
        });
    }

    /// A key that is not text. Text arrives through `insert`, typed or
    /// composed alike. A chord is left to the window's bindings.
    fn key(&self, text: &SharedString, control: bool) -> bool {
        let Some(key) = text.chars().next() else {
            return false;
        };
        if control || self.current.borrow().is_none() {
            return false;
        }
        let handled = match key {
            k if k == char::from(Key::LeftArrow) => self.move_caret(0, -1),
            k if k == char::from(Key::RightArrow) => self.move_caret(0, 1),
            k if k == char::from(Key::UpArrow) => self.move_caret(-1, 0),
            k if k == char::from(Key::DownArrow) => self.move_caret(1, 0),
            k if k == char::from(Key::Home) => self.with_doc(|doc| doc.caret.1 = 0),
            k if k == char::from(Key::End) => self.with_doc(|doc| {
                let (line, _) = doc.caret;
                doc.caret.1 = doc.lines[line].chars().count();
            }),
            k if k == char::from(Key::Backspace) => {
                self.backspace();
                true
            }
            k if k == char::from(Key::Return) => {
                self.insert("\n");
                true
            }
            k if k == char::from(Key::Tab) => {
                self.insert("    ");
                true
            }
            _ => return false,
        };
        if handled {
            self.show_caret();
        }
        handled
    }

    fn with_doc(&self, change: impl FnOnce(&mut Document)) -> bool {
        let Some(doc) = self.current.borrow().clone() else {
            return false;
        };
        change(&mut doc.borrow_mut());
        true
    }

    fn move_caret(&self, lines: isize, chars: isize) -> bool {
        self.with_doc(|doc| {
            let (line, col) = doc.caret;
            let line = (line as isize + lines).clamp(0, doc.lines.len() as isize - 1) as usize;
            let width = doc.lines[line].chars().count() as isize;
            let col = (col as isize + chars).clamp(0, width) as usize;
            doc.caret = (line, col);
        })
    }

    fn insert(&self, typed: &str) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(doc) = self.current.borrow().clone() else {
            return;
        };
        self.metrics.borrow_mut().learn(&window, typed);
        let (first, added) = {
            let mut doc = doc.borrow_mut();
            let (line, col) = doc.caret;
            let at = byte_at(&doc.lines[line], col);
            let rest = doc.lines[line].split_off(at);
            let mut parts = typed.split('\n');
            doc.lines[line].push_str(parts.next().unwrap_or_default());
            let mut row = line;
            for part in parts {
                row += 1;
                doc.lines.insert(row, part.to_string());
            }
            let col = doc.lines[row].chars().count();
            doc.lines[row].push_str(&rest);
            doc.caret = (row, col);
            (line, row - line)
        };
        self.repaint(&doc, first, added as isize);
        self.show_caret();
    }

    fn backspace(&self) {
        let Some(doc) = self.current.borrow().clone() else {
            return;
        };
        let (first, removed) = {
            let mut doc = doc.borrow_mut();
            let (line, col) = doc.caret;
            if col > 0 {
                let from = byte_at(&doc.lines[line], col - 1);
                let to = byte_at(&doc.lines[line], col);
                doc.lines[line].replace_range(from..to, "");
                doc.caret = (line, col - 1);
                (line, 0)
            } else if line > 0 {
                let tail = doc.lines.remove(line);
                let col = doc.lines[line - 1].chars().count();
                doc.lines[line - 1].push_str(&tail);
                doc.caret = (line - 1, col);
                (line - 1, 1)
            } else {
                return;
            }
        };
        self.repaint(&doc, first, -(removed as isize));
    }

    /// Repaint after an edit at `first` that added (or, negative, removed)
    /// `added` lines below it, and tell the view which rows changed.
    fn repaint(&self, doc: &Rc<RefCell<Document>>, first: usize, added: isize) {
        let (from, count, path) = {
            let mut doc = doc.borrow_mut();
            let text = doc.text();
            let repaint = self.files.repaint(
                &doc.path,
                &text,
                doc.paint,
                Some((first as u32, first as u32 + 1)),
            );
            doc.paint = Some(repaint.version);
            let from = repaint.from as usize;
            let count = repaint.lines.len();
            // Keep the painting's lines in step with the text first: the rows
            // the edit added or removed, then the lines the repaint returned.
            if added > 0 {
                for at in 0..added as usize {
                    doc.painted.insert(first + 1 + at, Line::default());
                }
            } else {
                for _ in 0..(-added) as usize {
                    doc.painted.remove(first + 1);
                }
            }
            for (at, line) in repaint.lines.into_iter().enumerate() {
                if let Some(slot) = doc.painted.get_mut(from + at) {
                    *slot = line;
                }
            }
            (from, count, doc.path.clone())
        };
        if added > 0 {
            self.rows.added(first + 1, added as usize);
        } else if added < 0 {
            self.rows.removed(first + 1, (-added) as usize);
        }
        self.rows
            .changed(first.min(from), from.max(first) + count.max(1));
        self.tell(&path);
    }

    fn show_caret(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(doc) = self.current.borrow().clone() else {
            return;
        };
        let doc = doc.borrow();
        let (line, col) = doc.caret;
        let x = self.metrics.borrow().x_of(&doc.lines[line], col);
        let code = window.global::<Code>();
        code.set_caret_line(line as i32);
        code.set_caret_x(x);
        // Keep the caret on screen.
        let row = code.get_row_height();
        let top = line as f32 * row;
        let view = code.get_view_height();
        let scrolled = -code.get_scroll_y();
        if top < scrolled {
            code.set_scroll_y(-top);
        } else if view > 0.0 && top + row > scrolled + view {
            code.set_scroll_y(-(top + row - view));
        }
        drop(doc);
        if let Some(tell) = self.on_caret.borrow().as_ref() {
            tell();
        }
    }
}

/// The byte a character column starts at.
fn byte_at(text: &str, col: usize) -> usize {
    text.char_indices()
        .nth(col)
        .map_or(text.len(), |(at, _)| at)
}
