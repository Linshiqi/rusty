//! The finder's `@` showing a symbol in the code while its row is the one
//! selected, VS Code's: the line brought into view and lit across, the
//! caret left where it was — so Escape has only the view to put back
//! (`Surface::set_scroll`, with where it was before), and Enter jumps from
//! the caret, which is what Alt+Left comes back to.

use super::{Code, Surface};

impl Surface {
    /// `line` of the document on screen, counted from zero, in view and
    /// lit; or with `None`, no line lit. The finder hangs over the top of
    /// the code, `covered` pixels of it: a line under it is not in view (see
    /// [`scroll_for`]). A line a fold hides is lit on its header's row,
    /// where it is drawn.
    pub fn preview(&self, line: Option<usize>, covered: f32) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let code = window.global::<Code>();
        let Some((line, doc)) = line.zip(self.doc()) else {
            code.set_preview_row(-1);
            return;
        };
        let row = {
            let d = doc.borrow();
            d.row_of(line.min(d.lines.count().saturating_sub(1)))
        };
        let height = code.get_row_height();
        let place = Place {
            top: row as f32 * height,
            height,
            scrolled: -code.get_scroll_y(),
            view: code.get_view_height(),
            covered,
        };
        if let Some(scrolled) = scroll_for(place) {
            code.set_scroll_y(-scrolled);
            self.view_changed();
        }
        code.set_preview_row(row as i32);
    }
}

/// A row and the view it is to be shown in, in pixels down the document.
#[derive(Clone, Copy, Debug)]
struct Place {
    /// Where the row starts, and how tall it is.
    top: f32,
    height: f32,
    /// How far down the view is scrolled, and how tall it is.
    scrolled: f32,
    view: f32,
    /// How much of the view's top the finder hangs over.
    covered: f32,
}

/// How far down to scroll for the row to show — `None` where it already
/// shows below the finder. Brought into view, it goes to the middle of what
/// the finder leaves; with no room left under the finder, to the middle of
/// the whole view.
fn scroll_for(place: Place) -> Option<f32> {
    let Place {
        top,
        height,
        scrolled,
        view,
        covered,
    } = place;
    let covered = if covered + height < view {
        covered.max(0.0)
    } else {
        0.0
    };
    let shows = top >= scrolled + covered && top + height <= scrolled + view;
    (!shows).then(|| (top - (covered + (view - covered - height) / 2.0)).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(top: f32, covered: f32) -> Option<f32> {
        scroll_for(Place {
            top,
            height: 20.0,
            scrolled: 0.0,
            view: 700.0,
            covered,
        })
    }

    /// A view of 700 with the finder over its top 400.
    #[test]
    fn a_line_under_the_finder_is_brought_below_it() {
        // On screen, but under the finder: the document's top cannot come
        // down any further.
        assert_eq!(at(100.0, 400.0), Some(0.0));
        // Far down: to the middle of the 300 below the finder.
        assert_eq!(at(2000.0, 400.0), Some(1460.0));
        // Below the finder already: left where it is.
        assert_eq!(at(500.0, 400.0), None);
        // Nothing over the code: the middle of the view, as VS Code puts it.
        assert_eq!(at(2000.0, 0.0), Some(1660.0));
        // A finder as tall as the view: the middle of the whole view.
        assert_eq!(at(2000.0, 900.0), Some(1660.0));
    }
}
