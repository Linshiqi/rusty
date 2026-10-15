//! The minimap's arithmetic: the whole file, small, down the right-hand
//! edge — VS Code's — as runs of ink two pixels a row and one a character,
//! and where it stands against the editor. Pure, for both windows; each
//! paints the runs its own way.

use crate::{Line, Token};

/// How wide the minimap is, in logical pixels.
pub const WIDTH: f64 = 96.0;
/// How tall one of its rows is, and how wide one character.
pub const ROW: f64 = 2.0;
pub const CHAR: f64 = 1.0;

/// Where the minimap stands against the editor: how far its own rows are
/// scrolled, in its pixels, and where the slider marking the view is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub scroll: f64,
    pub slider_top: f64,
    pub slider_height: f64,
}

/// The minimap's scroll and the slider for an editor `view_top` pixels down a
/// text of `rows` rows `row_px` tall with `pad` pixels above and below them,
/// seen through a view `view_height` tall, on a minimap `height` tall. A file
/// that fits is drawn from its top; one that does not is scrolled in step
/// with the editor, so the slider travels the minimap's whole height as the
/// view travels the file — VS Code's arithmetic.
pub fn frame(
    rows: u32,
    row_px: f64,
    pad: f64,
    view_top: f64,
    view_height: f64,
    height: f64,
) -> Frame {
    let content = f64::from(rows) * ROW;
    let travel = (f64::from(rows) * row_px + 2.0 * pad - view_height).max(0.0);
    let scroll = if content <= height || travel <= 0.0 {
        0.0
    } else {
        (view_top / travel).clamp(0.0, 1.0) * (content - height)
    };
    let first = (view_top - pad).max(0.0) / row_px;
    Frame {
        scroll,
        slider_top: first * ROW - scroll,
        slider_height: (view_height / row_px * ROW).max(ROW),
    }
}

/// The editor scroll that puts minimap `y` — pixels from its top edge — in
/// the middle of the view.
pub fn scroll_for(y: f64, frame: Frame, row_px: f64, pad: f64, view_height: f64) -> f64 {
    let row = (y + frame.scroll) / ROW;
    (row * row_px + pad - view_height / 2.0).max(0.0)
}

/// A painted line's runs of ink, as (first column, columns, token):
/// whitespace is paper, and a tab reaches its stop of `tab`.
pub fn ink(line: &Line, tab: u32) -> Vec<(u32, u32, Token)> {
    let mut runs = Vec::new();
    let mut col = 0u32;
    for span in &line.spans {
        let mut start = None;
        for ch in span.text.chars() {
            if ch.is_whitespace() {
                if let Some(from) = start.take() {
                    runs.push((from, col - from, span.token));
                }
                col = if ch == '\t' {
                    (col / tab + 1) * tab
                } else {
                    col + 1
                };
            } else {
                start.get_or_insert(col);
                col += 1;
            }
        }
        if let Some(from) = start {
            runs.push((from, col - from, span.token));
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Span;

    const PAD: f64 = 8.0;

    fn span(text: &str, token: Token) -> Span {
        Span {
            text: text.to_string(),
            token,
        }
    }

    /// A short file is drawn from its top, and the slider is the view's rows.
    #[test]
    fn a_file_that_fits_is_drawn_from_its_top() {
        let at = frame(100, 20.0, PAD, PAD + 10.0 * 20.0, 400.0, 600.0);
        assert_eq!(at.scroll, 0.0);
        assert_eq!(at.slider_top, 10.0 * ROW);
        assert_eq!(at.slider_height, 20.0 * ROW);
    }

    /// A long one scrolls with the editor: at the top of the file the
    /// minimap is at its top, at the bottom at its bottom, and the slider
    /// never leaves it.
    #[test]
    fn a_long_file_scrolls_in_step_with_the_editor() {
        let (rows, row_px, view, height) = (10_000, 20.0, 400.0, 600.0);
        let travel = f64::from(rows) * row_px + 2.0 * PAD - view;
        let top = frame(rows, row_px, PAD, 0.0, view, height);
        assert_eq!(top.scroll, 0.0);
        let bottom = frame(rows, row_px, PAD, travel, view, height);
        assert_eq!(bottom.scroll, f64::from(rows) * ROW - height);
        assert!(bottom.slider_top + bottom.slider_height <= height + ROW);
        let middle = frame(rows, row_px, PAD, travel / 2.0, view, height);
        assert!(middle.slider_top > 0.0 && middle.slider_top < height);
    }

    /// A press puts the row under it in the middle of the view.
    #[test]
    fn a_press_centres_its_row() {
        let at = frame(100, 20.0, PAD, 0.0, 400.0, 600.0);
        let top = scroll_for(50.0 * ROW, at, 20.0, PAD, 400.0);
        assert_eq!(top, 50.0 * 20.0 + PAD - 200.0);
        assert_eq!(
            scroll_for(0.0, at, 20.0, PAD, 400.0),
            0.0,
            "never above the top"
        );
    }

    #[test]
    fn ink_is_the_runs_between_the_spaces_and_a_tab_reaches_its_stop() {
        let line = Line {
            spans: vec![
                span("\tlet", Token::Keyword),
                span(" x = ", Token::Plain),
                span("\"a b\"", Token::Str),
            ],
        };
        assert_eq!(
            ink(&line, 4),
            [
                (4, 3, Token::Keyword),
                (8, 1, Token::Plain),
                (10, 1, Token::Plain),
                (12, 2, Token::Str),
                (15, 2, Token::Str),
            ]
        );
    }
}
