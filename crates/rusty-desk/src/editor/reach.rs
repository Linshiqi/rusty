//! Where the view stands against the rows, pure and tested: how far down it
//! goes — past the last row until that row is at the top, VS Code's
//! `editor.scrollBeyondLastLine` — and where it goes to show the caret with
//! room round it, VS Code's `editor.cursorSurroundingLines`.
//!
//! Every length is in pixels down from the first row's top; a view's top
//! is how far down the text it starts, the list view's offset turned
//! round. The room below the last row is space the view scrolls into
//! (`ui/listview.slint`), never a row: nothing but these two answers
//! counts it.

/// Rows kept between the caret and either edge of the view when a key, an
/// edit typed or a jump moves it — so typing at the bottom of the view, or
/// at the end of a file, never types on its edge.
pub const ROOM_ROWS: usize = 3;

/// The room past the last row a view `view` tall scrolls into, over rows
/// `row` tall: all of it but a row — `room` in `ui/editor.slint`'s lists.
pub fn room(row: f32, view: f32) -> f32 {
    (view - row).max(0.0)
}

/// The furthest down a view `view` tall goes over `rows` rows `row` tall:
/// the rows and the room past them, so until the last row is at its top —
/// or, through a view shorter than a row, until the last row's foot is at
/// the view's. `lowest` in `ui/listview.slint`, turned round.
pub fn deepest(rows: usize, row: f32, view: f32) -> f32 {
    (rows as f32 * row + room(row, view) - view).max(0.0)
}

/// Where a view `view` tall, its top at `top`, goes so the row whose top is
/// at `at` is shown with `room` rows of the text above and below it — the
/// least move that does, never above the first row nor past `deepest` —
/// or `None` when it is shown so already. The room is at most what leaves
/// the row a place in the view's middle, so a short view still shows it.
///
/// A move lands on a whole row, rounded the way that keeps the room: the
/// list view puts a view that jumps further than a screen and a half on
/// the row above where it was asked to stand (`update_visible_instances`),
/// and a view of 37.8 rows asked for 395.2 stood at 395 and kept 2.8 rows
/// below the caret, not three.
pub fn reveal(top: f32, at: f32, row: f32, view: f32, room: usize, deepest: f32) -> Option<f32> {
    let to = if view <= 0.0 || row <= 0.0 {
        // Not laid out yet: only a row above the view is known to be off it.
        (at < top).then_some(at)?
    } else {
        let fits = ((view / row - 1.0) / 2.0).floor().max(0.0) as usize;
        let room = room.min(fits) as f32 * row;
        if at - room < top {
            ((at - room) / row).floor() * row
        } else if at + row + room > top + view {
            ((at + row + room - view) / row).ceil() * row
        } else if top > deepest {
            // Further down than the rows reach, as a text that got shorter
            // leaves it.
            deepest
        } else {
            return None;
        }
    };
    let to = to.min(deepest).max(0.0);
    (to != top).then_some(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f32 = 19.0;

    /// Forty rows through a view of ten: the last row can come to the top,
    /// which is thirty-nine rows down, not thirty — the room past the end.
    #[test]
    fn the_view_goes_down_until_the_last_row_is_at_its_top() {
        assert_eq!(deepest(40, ROW, 10.0 * ROW), 39.0 * ROW);
        // A file shorter than the view scrolls too, as VS Code's does.
        assert_eq!(deepest(3, ROW, 10.0 * ROW), 2.0 * ROW);
        // One row, or none, does not scroll at all.
        assert_eq!(deepest(1, ROW, 10.0 * ROW), 0.0);
        assert_eq!(deepest(0, ROW, 10.0 * ROW), 0.0);
        // A view shorter than a row shows the last row's foot.
        assert_eq!(deepest(40, ROW, 10.0), 40.0 * ROW - 10.0);
    }

    /// A row already shown with its room around it does not move the view.
    #[test]
    fn a_caret_in_the_middle_moves_nothing() {
        let view = 20.0 * ROW;
        let most = deepest(100, ROW, view);
        assert_eq!(reveal(0.0, 10.0 * ROW, ROW, view, ROOM_ROWS, most), None);
        assert_eq!(reveal(0.0, 16.0 * ROW, ROW, view, ROOM_ROWS, most), None);
    }

    /// Typing down the view's last rows: each Enter moves it by one row,
    /// keeping three below the caret — the view follows line by line.
    #[test]
    fn typing_at_the_bottom_keeps_room_below() {
        let view = 20.0 * ROW;
        let most = deepest(100, ROW, view);
        // Row 17's foot leaves two rows below it: one row down.
        assert_eq!(
            reveal(0.0, 17.0 * ROW, ROW, view, ROOM_ROWS, most),
            Some(ROW)
        );
        assert_eq!(
            reveal(ROW, 18.0 * ROW, ROW, view, ROOM_ROWS, most),
            Some(2.0 * ROW)
        );
        // A caret below the view entirely: to three rows from the foot.
        assert_eq!(
            reveal(0.0, 50.0 * ROW, ROW, view, ROOM_ROWS, most),
            Some(34.0 * ROW)
        );
    }

    /// The last line of a long file is typed on with room below it: the
    /// room past the end is what the view moves into.
    #[test]
    fn the_end_of_a_file_is_typed_with_room_below() {
        let (rows, view) = (100, 20.0 * ROW);
        let most = deepest(rows, ROW, view);
        let last = (rows - 1) as f32 * ROW;
        let at = reveal(80.0 * ROW, last, ROW, view, ROOM_ROWS, most).unwrap();
        assert_eq!(at, 83.0 * ROW);
        assert!(at > rows as f32 * ROW - view, "past where the rows end");
        // And the next Enter makes a row more, and moves one more.
        let most = deepest(rows + 1, ROW, view);
        assert_eq!(
            reveal(at, last + ROW, ROW, view, ROOM_ROWS, most),
            Some(84.0 * ROW)
        );
    }

    /// Up the view: three rows above the caret, and never above the top.
    #[test]
    fn moving_up_keeps_room_above() {
        let view = 20.0 * ROW;
        let most = deepest(100, ROW, view);
        assert_eq!(
            reveal(40.0 * ROW, 42.0 * ROW, ROW, view, ROOM_ROWS, most),
            Some(39.0 * ROW)
        );
        assert_eq!(
            reveal(2.0 * ROW, ROW, ROW, view, ROOM_ROWS, most),
            Some(0.0)
        );
        assert_eq!(reveal(0.0, 0.0, ROW, view, ROOM_ROWS, most), None);
    }

    /// No room asked for — the pointer moved it — is the plain reveal: a
    /// row on the view's last row stays where it is.
    #[test]
    fn no_room_is_only_on_screen() {
        let view = 20.0 * ROW;
        let most = deepest(100, ROW, view);
        assert_eq!(reveal(0.0, 19.0 * ROW, ROW, view, 0, most), None);
        assert_eq!(reveal(0.0, 20.0 * ROW, ROW, view, 0, most), Some(ROW));
    }

    /// A view that is not a whole number of rows stops on a whole row, the
    /// one that keeps at least the room: the list view would put it on the
    /// row above, a fraction of the room short.
    #[test]
    fn a_move_lands_on_a_whole_row() {
        let view = 37.8 * ROW;
        let most = deepest(430, ROW, view);
        let to = reveal(0.0, 429.0 * ROW, ROW, view, ROOM_ROWS, most).unwrap();
        assert_eq!(to, 396.0 * ROW);
        assert!(to + view - 430.0 * ROW >= 3.0 * ROW);
        // Up, the row above: room above the caret as well.
        let to = reveal(100.4 * ROW, 98.0 * ROW, ROW, view, ROOM_ROWS, most).unwrap();
        assert_eq!(to, 95.0 * ROW);
    }

    /// A view of four rows cannot keep three either side; it keeps one.
    #[test]
    fn a_short_view_keeps_what_room_fits() {
        let view = 4.0 * ROW;
        let most = deepest(100, ROW, view);
        assert_eq!(
            reveal(0.0, 3.0 * ROW, ROW, view, ROOM_ROWS, most),
            Some(ROW)
        );
        // Two rows of view: none.
        let view = 2.0 * ROW;
        assert_eq!(reveal(0.0, ROW, ROW, view, ROOM_ROWS, most), None);
    }

    /// A view further down than the text now reaches comes back to the
    /// deepest; a view not laid out yet is moved only up to a row above it.
    #[test]
    fn a_view_past_the_rows_comes_back() {
        let view = 20.0 * ROW;
        let most = deepest(10, ROW, view);
        assert_eq!(
            reveal(30.0 * ROW, 5.0 * ROW, ROW, view, ROOM_ROWS, most),
            Some(2.0 * ROW)
        );
        assert_eq!(
            reveal(9.0 * ROW + 5.0, 9.0 * ROW, ROW, view, 0, most),
            Some(9.0 * ROW)
        );
        assert_eq!(
            reveal(5.0 * ROW, 2.0 * ROW, ROW, 0.0, ROOM_ROWS, 1e9),
            Some(2.0 * ROW)
        );
        assert_eq!(reveal(0.0, 50.0 * ROW, ROW, 0.0, ROOM_ROWS, 1e9), None);
    }
}
