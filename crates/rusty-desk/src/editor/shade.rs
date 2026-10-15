//! The other editor group's file, drawn beside the one being edited.
//!
//! Two groups side by side, and the keys in one of them: that one is the
//! surface — the document on screen, its caret, its popups — and the other
//! is drawn from a second rows model over its own document, with nothing to
//! type into. A press there brings its group to front (`App::swap_groups`)
//! and is made again in the code view, which now shows that document where
//! the other view had it. One file open in both is one document in two
//! views: an edit in either draws both.

use std::{cell::RefCell, rc::Rc};

use slint::ComponentHandle;

use super::{Document, Shared, Surface, rows::Rows};
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
        if let Some(window) = self.window.upgrade() {
            let shade = window.global::<crate::Shade>();
            shade.set_has_document(widest.is_some());
            shade.set_content_width(widest.unwrap_or(0.0) + 200.0);
            shade.set_scroll_y(scroll.0);
            shade.set_scroll_x(scroll.1);
        }
    }

    /// Where the other group's view is scrolled.
    pub fn shade_scroll(&self) -> (f32, f32) {
        self.window.upgrade().map_or((0.0, 0.0), |window| {
            let shade = window.global::<crate::Shade>();
            (shade.get_scroll_y(), shade.get_scroll_x())
        })
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

    /// [`Surface::shade_follows`] for a file named by its path.
    pub(super) fn shade_follows_path(&self, path: &str) {
        let doc = self.open.borrow().get(path).cloned();
        if let Some(doc) = doc {
            self.shade_follows(&doc);
        }
    }

    /// A press made in the other group's view, made again here once its
    /// group has the keys: on the same row, at the same point.
    pub fn press_at(&self, row: usize, x: f32, y: f32) {
        self.pointer(0, row, x, y, (false, false, false));
        self.pointer(2, row, x, y, (false, false, false));
    }
}
