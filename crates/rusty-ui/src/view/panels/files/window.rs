//! Which rows the two layers draw.
//!
//! The echo and the gutter draw the rows in the scroller's view and a margin
//! either side of it — never the file. Both used to draw every line: a file
//! of 24,000 lines was a hundred thousand elements in the echo and a
//! breakpoint signal per line in the gutter, all rebuilt on every keystroke.
//! The textarea still holds the whole text, because the caret, the selection
//! and every key the browser handles need it; spacers above and below the
//! drawn rows keep both layers as tall as the file, so the scrollbar and every
//! overlay's `row_top` are what they always were.

use std::ops::Range;

use super::PAD_PX;

/// Rows drawn past each edge of the view, so a short scroll lands on rows
/// that are already there.
const SPARE_ROWS: u32 = 40;

/// The drawn range moves in steps of this many rows. Rows are keyed, so a
/// step redraws only the rows it brings in; the steps are what keep a scroll
/// of a line or two from changing the range at all.
const STEP_ROWS: u32 = 32;

/// The screen rows to draw for a scroller `top` pixels down and `height`
/// tall, over `total` rows `row` pixels each.
pub(super) fn rows_to_draw(top: f64, height: f64, row: f64, total: u32) -> Range<u32> {
    if total == 0 || row <= 0.0 {
        return 0..0;
    }
    let first = ((top - PAD_PX).max(0.0) / row).floor() as u32;
    let shown = (height.max(0.0) / row).ceil() as u32 + 1;
    let start = first.saturating_sub(SPARE_ROWS) / STEP_ROWS * STEP_ROWS;
    let end = first
        .saturating_add(shown)
        .saturating_add(SPARE_ROWS)
        .div_ceil(STEP_ROWS)
        .saturating_mul(STEP_ROWS);
    start.min(total)..end.min(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f64 = 19.0;

    /// Whatever the scroll, every row in view is drawn, and a margin past it.
    #[test]
    fn every_row_in_view_is_drawn_with_a_margin() {
        for top in [0.0, 7.0, 190.0, 5_000.0, 123_456.0] {
            for height in [0.0, 400.0, 1_300.0] {
                let range = rows_to_draw(top, height, ROW, 100_000);
                let first = ((top - PAD_PX).max(0.0) / ROW).floor() as u32;
                let last = ((top - PAD_PX + height) / ROW).ceil() as u32;
                let shown = (height / ROW).ceil() as u32 + 1;
                assert!(
                    range.start + SPARE_ROWS <= first.max(SPARE_ROWS),
                    "{top} {height}: {range:?}"
                );
                assert!(range.end >= last + SPARE_ROWS, "{top} {height}: {range:?}");
                assert!(
                    range.end - range.start <= shown + 2 * (SPARE_ROWS + STEP_ROWS),
                    "{top} {height}: {range:?} is not a window"
                );
            }
        }
    }

    #[test]
    fn a_short_file_is_drawn_whole_and_nothing_past_its_end() {
        assert_eq!(rows_to_draw(0.0, 800.0, ROW, 12), 0..12);
        assert_eq!(rows_to_draw(0.0, 800.0, ROW, 0), 0..0);
        assert_eq!(rows_to_draw(9_000.0, 800.0, ROW, 12), 12..12);
    }

    /// A line or two of scrolling mostly draws the same rows: the range moves
    /// in steps, not with every pixel.
    #[test]
    fn a_small_scroll_usually_keeps_the_range() {
        let at = |line: f64| rows_to_draw(PAD_PX + line * ROW, 600.0, ROW, 10_000);
        let same = (100..164)
            .filter(|&line| at(f64::from(line)) == at(f64::from(line + 1)))
            .count();
        assert!(
            same >= 60,
            "only {same} of 64 one-line scrolls kept the range"
        );
    }
}
