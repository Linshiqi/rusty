//! The code surface's Rust half: the document, where every character sits,
//! and the rows the view draws.
//!
//! Slint draws; this measures and places. A row is handed over as pieces —
//! a run of one colour at the x it starts at — so the view needs no text
//! layout of its own, and the caret, a click and every overlay are placed by
//! the same arithmetic ([`Metrics`]) that placed the text.

mod metrics;
mod rows;

use std::{cell::RefCell, path::Path, rc::Rc, time::Instant};

use rusty_edit::{Files, Line};
use slint::{ComponentHandle, ModelRc, SharedString, platform::Key};

use crate::AppWindow;
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
}

impl Document {
    fn text(&self) -> String {
        self.lines.join("\n")
    }
}

/// The surface: the document, its rows, and the window that shows them.
pub struct Surface {
    files: Files,
    doc: Rc<RefCell<Document>>,
    metrics: Rc<RefCell<Metrics>>,
    rows: Rc<Rows>,
    window: slint::Weak<AppWindow>,
}

impl Surface {
    pub fn open(window: &AppWindow, root: &Path, relative: &str) -> Rc<Surface> {
        let started = Instant::now();
        let files = Files::new();
        let opened = files.open(root, relative).expect("the file opens");
        let read = started.elapsed();
        let doc = Rc::new(RefCell::new(Document {
            path: relative.to_string(),
            lines: opened.text.split('\n').map(str::to_string).collect(),
            painted: opened.lines,
            paint: opened.paint,
            caret: (0, 0),
        }));
        let metrics = Rc::new(RefCell::new(Metrics::measure(window)));
        metrics.borrow_mut().learn(window, &opened.text);
        let measured = started.elapsed();
        let rows = Rc::new(Rows::new(doc.clone(), metrics.clone()));
        window.set_rows(ModelRc::from(rows.clone()));
        window.set_content_width(metrics.borrow().content_width(&doc.borrow().lines) + 200.0);
        eprintln!(
            "opened {relative}: {} lines, read and painted in {:?}, measured in {:?}",
            doc.borrow().lines.len(),
            read,
            measured - read,
        );
        let surface = Rc::new(Surface {
            files,
            doc,
            metrics,
            rows,
            window: window.as_weak(),
        });
        surface.wire(window);
        surface.show_caret();
        window.invoke_focus_input();
        surface
    }

    fn wire(self: &Rc<Self>, window: &AppWindow) {
        let this = Rc::downgrade(self);
        window.on_typed(move |text| {
            if let Some(this) = this.upgrade() {
                this.insert(&text);
            }
        });
        let this = Rc::downgrade(self);
        window.on_key(move |event| {
            this.upgrade()
                .is_some_and(|this| this.key(&event.text, event.modifiers.control))
        });
        let this = Rc::downgrade(self);
        window.on_pressed(move |line, x| {
            if let Some(this) = this.upgrade() {
                let col = {
                    let doc = this.doc.borrow();
                    let text = doc.lines.get(line as usize).map_or("", String::as_str);
                    this.metrics.borrow().column_at(text, x)
                };
                this.doc.borrow_mut().caret = (line as usize, col);
                this.show_caret();
            }
        });
    }

    /// A key that is not text. Text arrives through `insert`, typed or
    /// composed alike.
    fn key(&self, text: &SharedString, control: bool) -> bool {
        let Some(key) = text.chars().next() else {
            return false;
        };
        let handled = match key {
            k if k == char::from(Key::LeftArrow) => self.move_caret(0, -1),
            k if k == char::from(Key::RightArrow) => self.move_caret(0, 1),
            k if k == char::from(Key::UpArrow) => self.move_caret(-1, 0),
            k if k == char::from(Key::DownArrow) => self.move_caret(1, 0),
            k if k == char::from(Key::Home) => {
                self.doc.borrow_mut().caret.1 = 0;
                true
            }
            k if k == char::from(Key::End) => {
                let mut doc = self.doc.borrow_mut();
                let (line, _) = doc.caret;
                doc.caret.1 = doc.lines[line].chars().count();
                true
            }
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
            _ if control => false,
            _ => return false,
        };
        if handled {
            self.show_caret();
        }
        handled
    }

    fn move_caret(&self, lines: isize, chars: isize) -> bool {
        let mut doc = self.doc.borrow_mut();
        let (line, col) = doc.caret;
        let line = (line as isize + lines).clamp(0, doc.lines.len() as isize - 1) as usize;
        let width = doc.lines[line].chars().count() as isize;
        let col = (col as isize + chars).clamp(0, width) as usize;
        doc.caret = (line, col);
        true
    }

    fn insert(&self, typed: &str) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        self.metrics.borrow_mut().learn(&window, typed);
        let (first, added) = {
            let mut doc = self.doc.borrow_mut();
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
        self.repaint(first, added as isize);
        self.show_caret();
    }

    fn backspace(&self) {
        let (first, removed) = {
            let mut doc = self.doc.borrow_mut();
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
        self.repaint(first, -(removed as isize));
    }

    /// Repaint after an edit at `first` that added (or, negative, removed)
    /// `added` lines below it, and tell the view which rows changed.
    fn repaint(&self, first: usize, added: isize) {
        let mut doc = self.doc.borrow_mut();
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
        drop(doc);
        if added > 0 {
            self.rows.added(first + 1, added as usize);
        } else if added < 0 {
            self.rows.removed(first + 1, (-added) as usize);
        }
        self.rows
            .changed(first.min(from), from.max(first) + count.max(1));
    }

    fn show_caret(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let doc = self.doc.borrow();
        let (line, col) = doc.caret;
        let x = self.metrics.borrow().x_of(&doc.lines[line], col);
        window.set_caret_line(line as i32);
        window.set_caret_x(x);
        // Keep the caret on screen.
        let row = window.get_row_height();
        let top = line as f32 * row;
        let view = window.get_view_height();
        let scrolled = -window.get_scroll_y();
        if top < scrolled {
            window.set_scroll_y(-top);
        } else if top + row > scrolled + view {
            window.set_scroll_y(-(top + row - view));
        }
    }
}

/// The byte a character column starts at.
fn byte_at(text: &str, col: usize) -> usize {
    text.char_indices()
        .nth(col)
        .map_or(text.len(), |(at, _)| at)
}
