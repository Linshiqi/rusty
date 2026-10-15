//! The other editor group's file, drawn beside the one being edited.
//!
//! Two groups side by side, and the keys in one of them: that one is the
//! surface — the document on screen, its caret, its popups — and the other
//! is drawn from a second rows model over its own document, with nothing to
//! type into. A press there brings its group to front (`App::swap_groups`)
//! and is made again in the code view, which now shows that document where
//! the other view had it. One file open in both is one document in two
//! views: an edit in either draws both.
//!
//! A window a tab was moved into is another view of the same kind, with a
//! surface of its own (`host.rs`): what one surface does to a document, the
//! other group's view and every other window showing it follow.

use std::{cell::RefCell, rc::Rc};

use slint::ComponentHandle;

use super::{Document, Shared, Surface, document::Change, host::Strong, inlays, rows::Rows};
use crate::Code;

pub(super) struct Shade {
    pub doc: Shared,
    pub rows: Rc<Rows>,
}

impl Surface {
    /// The other group's file — none, while the window is not split — and
    /// where its view was scrolled.
    pub fn show_shade(&self, path: Option<&str>, scroll: (f32, f32)) {
        let doc = path.and_then(|path| self.open.borrow().get(path).cloned());
        let widest = doc
            .as_ref()
            .map(|doc| self.metrics.borrow().widest(&doc.borrow()));
        *self.shade.doc.borrow_mut() = doc;
        self.shade.rows.reset();
        if let Some(Strong::Main(window)) = self.window.upgrade() {
            let shade = window.global::<crate::Shade>();
            shade.set_has_document(widest.is_some());
            shade.set_content_width(widest.unwrap_or(0.0) + 200.0);
            shade.set_scroll_y(scroll.0);
            shade.set_scroll_x(scroll.1);
        }
    }

    /// Where the other group's view is scrolled.
    pub fn shade_scroll(&self) -> (f32, f32) {
        match self.window.upgrade() {
            Some(Strong::Main(window)) => {
                let shade = window.global::<crate::Shade>();
                (shade.get_scroll_y(), shade.get_scroll_x())
            }
            _ => (0.0, 0.0),
        }
    }

    /// Where the code view is scrolled.
    pub fn scroll(&self) -> (f32, f32) {
        self.window.upgrade().map_or((0.0, 0.0), |window| {
            let code = window.global::<Code>();
            (code.get_scroll_y(), code.get_scroll_x())
        })
    }

    /// The code view scrolled to `at` — the place its group had it.
    pub fn set_scroll(&self, at: (f32, f32)) {
        if let Some(window) = self.window.upgrade() {
            let code = window.global::<Code>();
            code.set_scroll_y(at.0);
            code.set_scroll_x(at.1);
        }
        self.view_changed();
    }

    /// `doc` changed — an edit, a painting, the server's colours or what it
    /// found: the other group's view is drawn again when it shows it.
    pub(super) fn shade_follows(&self, doc: &Rc<RefCell<Document>>) {
        let showing = self
            .shade
            .doc
            .borrow()
            .as_ref()
            .is_some_and(|shown| Rc::ptr_eq(shown, doc));
        if showing {
            self.shade.rows.reset();
        }
    }

    /// `doc` changed here — its text, by `change`, or only how it is drawn:
    /// the other group's view follows, and so does every other window that
    /// shows it.
    pub(super) fn follows(&self, doc: &Rc<RefCell<Document>>, change: Option<Change>) {
        self.shade_follows(doc);
        for other in self.others() {
            other.follow(doc, change);
        }
    }

    /// [`Surface::follows`] for a file named by its path, whose drawing
    /// changed — what a server found in it.
    pub(super) fn follows_path(&self, path: &str) {
        let doc = self.open.borrow().get(path).cloned();
        if let Some(doc) = doc {
            self.follows(&doc, None);
        }
    }

    /// The cursors of `doc` moved here: every other window showing it draws
    /// them where they are now. They are the document's, so a key pressed
    /// there lands where that window draws its caret.
    pub(super) fn cursors_follow(&self, doc: &Rc<RefCell<Document>>) {
        for other in self.others() {
            if other.doc().is_some_and(|shown| Rc::ptr_eq(&shown, doc)) {
                other.rows.reset();
                other.place_caret();
                other.tell_caret();
            }
        }
    }

    /// Another surface changed `doc`: drawn again here if this one shows it,
    /// the view left where it is. The cursors are the document's, as the
    /// split's are; what this view had open over the text — a popup, the
    /// card, the caret's marks — was about the text before, and goes.
    fn follow(&self, doc: &Rc<RefCell<Document>>, change: Option<Change>) {
        self.shade_follows(doc);
        let showing = self.doc().is_some_and(|shown| Rc::ptr_eq(&shown, doc));
        if !showing {
            return;
        }
        if let Some(change) = change {
            self.hide_hover();
            self.close_completion();
            self.close_signature();
            self.rest_nowhere();
            *self.marked.borrow_mut() = 0..0;
            let widest = {
                let d = doc.borrow();
                let metrics = self.metrics.borrow();
                (change.first..change.first + change.added)
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
            if self.finding_open() {
                self.research(Some(change));
            }
        }
        self.rows.reset();
        self.place_caret();
        self.tell_caret();
        self.overview
            .paintings
            .set(self.overview.paintings.get() + 1);
        self.view_changed();
    }

    /// A press made in the other group's view, made again here once its
    /// group has the keys: on the same row, at the same point.
    pub fn press_at(&self, row: usize, x: f32, y: f32) {
        self.pointer(0, row, x, y, (false, false, false));
        self.pointer(2, row, x, y, (false, false, false));
    }
}
