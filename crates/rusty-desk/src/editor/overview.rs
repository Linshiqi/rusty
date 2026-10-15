//! What the editor draws over the text besides the text: sticky scroll —
//! the lines that open the blocks the top of the view is in — and the
//! minimap down the right-hand edge, VS Code's two. Both read the rows as
//! they are on screen, folds and all, and are worked out again whenever the
//! view moves, the text changes or a painting lands
//! ([`Surface::view_changed`]); the arithmetic is `rusty_edit::sticky` and
//! `rusty_edit::minimap`.
//!
//! The minimap is an image painted here, two pixels a row and one a
//! character, and only for the rows it shows: drawn as elements, a file of
//! any length would be thousands of them.

use std::cell::{Cell, RefCell};

use rusty_edit::{Line, Span, Token, guides::TAB_SIZE, minimap, sticky};
use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use super::{Document, Surface, rows};
use crate::{Code, Theme};

/// What the switches in Settings ▸ Editor say, and what was last handed to
/// the view, so an unchanged answer is not handed over again.
pub(super) struct Overview {
    pub sticky: Cell<bool>,
    pub minimap: Cell<bool>,
    pub hints: Cell<bool>,
    /// The lines stuck last, for which text and which painting.
    stuck: RefCell<(String, u64, u64, Vec<u32>)>,
    /// What the minimap last painted.
    painted: RefCell<Option<Painted>>,
    /// Counts the paintings that landed, which change the colours without
    /// changing the text.
    pub paintings: Cell<u64>,
}

/// What a painted minimap shows: the text and its painting, its first row
/// and how many, in which theme at which scale.
#[derive(PartialEq)]
struct Painted {
    path: String,
    version: u64,
    paintings: u64,
    first: u32,
    count: u32,
    dark: bool,
    scale: u32,
}

impl Default for Overview {
    fn default() -> Self {
        Overview {
            sticky: Cell::new(true),
            minimap: Cell::new(true),
            hints: Cell::new(true),
            stuck: RefCell::default(),
            painted: RefCell::default(),
            paintings: Cell::new(0),
        }
    }
}

impl Surface {
    /// The switches in Settings ▸ Editor: what is drawn around the code.
    pub fn set_view(&self, view: rusty_embed::EditorView) {
        self.overview.sticky.set(view.sticky_scroll);
        self.overview.minimap.set(view.minimap);
        self.gutter.borrow_mut().no_guides = !view.indent_guides;
        let hints = self.overview.hints.replace(view.inlay_hints);
        if hints && !view.inlay_hints {
            for doc in self.open.borrow().values() {
                doc.borrow_mut().hints.clear();
            }
            self.hints_changed();
        } else if !hints && view.inlay_hints {
            self.analyse_soon();
        }
        self.rows.reset();
        self.view_changed();
    }

    /// The view scrolled or was resized, the theme changed, the text or
    /// its painting did: the stuck lines and the minimap follow.
    pub fn view_changed(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let code = window.global::<Code>();
        // Above the top is the list view's mistake, not a place
        // (`shade::within`); put back, the view lays out right from there.
        let y = code.get_scroll_y();
        if super::shade::within(y) != y {
            code.set_scroll_y(super::shade::within(y));
        }
        let Some(doc) = self.doc() else {
            code.set_sticky_rows(ModelRc::default());
            code.set_minimap_on(false);
            return;
        };
        let doc = doc.borrow();
        self.stick(&code, &doc);
        self.minimap(&window, &code, &doc);
        drop(doc);
        self.analysis_follows_view();
    }

    fn stick(&self, code: &Code, doc: &Document) {
        let row_height = code.get_row_height().max(1.0);
        let first = ((-code.get_scroll_y()) / row_height).floor().max(0.0) as u32;
        let lines = if self.overview.sticky.get() && doc.rows() > 0 {
            let regions = self.rows.regions(doc);
            sticky::stuck(&regions, |row| {
                doc.line_at_row((first + row) as usize) as u32
            })
        } else {
            Vec::new()
        };
        let key = (
            doc.path.clone(),
            doc.version,
            self.overview.paintings.get(),
            lines,
        );
        if *self.overview.stuck.borrow() == key {
            return;
        }
        let metrics = self.metrics.borrow();
        let shown: Vec<crate::Row> = key
            .3
            .iter()
            .map(|&line| rows::line_row(&metrics, doc, line as usize))
            .collect();
        code.set_sticky_rows(ModelRc::new(VecModel::from(shown)));
        *self.overview.stuck.borrow_mut() = key;
    }

    /// The minimap's slider and its image, painted again only when what it
    /// shows changed.
    fn minimap(&self, window: &super::host::Strong, code: &Code, doc: &Document) {
        let on = self.overview.minimap.get();
        code.set_minimap_on(on);
        if !on {
            return;
        }
        let row_px = f64::from(code.get_row_height().max(1.0));
        let view = f64::from(code.get_view_height());
        let height = view;
        let rows = doc.rows() as u32;
        let at = minimap::frame(
            rows,
            row_px,
            0.0,
            -f64::from(code.get_scroll_y()),
            view,
            height,
        );
        code.set_minimap_slider_top(at.slider_top as f32);
        code.set_minimap_slider_height(at.slider_height as f32);
        let first = (at.scroll / minimap::ROW).floor() as u32;
        code.set_minimap_offset(-(at.scroll - f64::from(first) * minimap::ROW) as f32);
        let count = ((height / minimap::ROW).ceil() as u32 + 1).min(rows.saturating_sub(first));
        let theme = window.global::<Theme>();
        let scale = window.window().scale_factor().max(1.0);
        let key = Painted {
            path: doc.path.clone(),
            version: doc.version,
            paintings: self.overview.paintings.get(),
            first,
            count,
            dark: theme.get_dark(),
            scale: (scale * 100.0) as u32,
        };
        if self.overview.painted.borrow().as_ref() == Some(&key) {
            return;
        }
        // Each kind of token's colour, as the text is drawn in it.
        let ink: Vec<Rgba8Pixel> = (0..11)
            .map(|kind| {
                let c = theme.invoke_token(kind);
                Rgba8Pixel::new(c.red(), c.green(), c.blue(), 200)
            })
            .collect();
        let scale = f64::from(scale);
        let width = (minimap::WIDTH * scale).round() as u32;
        let tall = ((f64::from(count.max(1)) * minimap::ROW) * scale).round() as u32;
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, tall);
        let pixels = buffer.make_mut_slice();
        for at in 0..count {
            let line = doc.line_at_row((first + at) as usize);
            let text = doc.lines.text(&doc.text, line);
            let plain;
            let painted = match doc.painted.get(line).filter(|l| rows::painted_is(l, text)) {
                Some(painted) => painted,
                None => {
                    plain = Line {
                        spans: vec![Span {
                            text: text.to_string(),
                            token: Token::Plain,
                        }],
                    };
                    &plain
                }
            };
            let top = (f64::from(at) * minimap::ROW * scale).round() as u32;
            let bottom =
                ((f64::from(at) * minimap::ROW + minimap::ROW - 0.5) * scale).floor() as u32;
            for (col, len, token) in minimap::ink(painted, TAB_SIZE as u32) {
                let from = (f64::from(col) * minimap::CHAR * scale) as u32;
                let to = ((f64::from(col + len) * minimap::CHAR * scale) as u32).min(width);
                let colour = ink[rows::kind(token) as usize];
                for y in top..bottom.min(tall) {
                    let row = (y * width) as usize;
                    for x in from..to {
                        pixels[row + x as usize] = colour;
                    }
                }
            }
        }
        code.set_minimap(Image::from_rgba8(buffer));
        code.set_minimap_rows(count as i32);
        *self.overview.painted.borrow_mut() = Some(key);
    }

    /// A press or a drag on the minimap: that part of the file in the
    /// middle of the view.
    pub(super) fn minimap_to(&self, y: f32) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let code = window.global::<Code>();
        let Some(doc) = self.doc() else { return };
        let rows = doc.borrow().rows() as u32;
        let row_px = f64::from(code.get_row_height().max(1.0));
        let view = f64::from(code.get_view_height());
        let at = minimap::frame(
            rows,
            row_px,
            0.0,
            -f64::from(code.get_scroll_y()),
            view,
            view,
        );
        let top = minimap::scroll_for(f64::from(y), at, row_px, 0.0, view);
        let most = (f64::from(rows) * row_px - view).max(0.0);
        code.set_scroll_y(-(top.min(most)) as f32);
        self.view_changed();
    }

    /// A stuck line clicked: the view moved so that line is the first one
    /// under the lines still stuck above it.
    pub(super) fn sticky_pick(&self, index: usize) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let code = window.global::<Code>();
        let Some(&line) = self.overview.stuck.borrow().3.get(index) else {
            return;
        };
        let Some(doc) = self.doc() else { return };
        let row = doc.borrow().row_of(line as usize).saturating_sub(index);
        code.set_scroll_y(-(row as f32 * code.get_row_height()));
        self.view_changed();
    }
}
