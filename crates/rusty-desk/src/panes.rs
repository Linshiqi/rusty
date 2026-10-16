//! Where a divider between two panes may stand.
//!
//! Every divider the History page lets somebody drag keeps both panes beside
//! it usable: each at least its least, while the room holds both. When it
//! does not — a window made narrow under a page laid out wide — the room is
//! shared in proportion to the two leasts, so neither pane is the one that
//! vanishes. Whole pixels, because a pane that starts half a pixel in draws
//! its clipped text soft (CLAUDE.md, the patched renderer). Pure, so what a
//! drag leaves is a test rather than something found by dragging; the page
//! asks it through `Shell.pane-fit` both while a divider is dragged and
//! whenever the room changes, so the two cannot disagree.

/// Where a divider stands, as a distance from the start of the `room` the
/// two panes share (the divider's own pixel left out), when it was asked to
/// stand at `asked`.
pub fn fit(asked: f32, room: f32, least_before: f32, least_after: f32) -> f32 {
    if !room.is_finite() || room <= 0.0 {
        return 0.0;
    }
    let asked = if asked.is_finite() { asked } else { room / 2.0 };
    let (before, after) = (least_before.max(0.0), least_after.max(0.0));
    let both = before + after;
    let at = if both <= room {
        asked.clamp(before, room - after)
    } else {
        room * before / both
    };
    at.round()
}

#[cfg(test)]
mod tests {
    use super::fit;

    /// With room for both, a divider stands where it was dragged, to the
    /// pixel.
    #[test]
    fn a_divider_stands_where_it_was_dragged() {
        assert_eq!(fit(300.0, 1000.0, 120.0, 300.0), 300.0);
        assert_eq!(fit(300.4, 1000.0, 120.0, 300.0), 300.0, "whole pixels");
    }

    /// Dragged past a pane's least, it stops there, whichever side.
    #[test]
    fn neither_pane_is_dragged_smaller_than_its_least() {
        assert_eq!(fit(20.0, 1000.0, 120.0, 300.0), 120.0);
        assert_eq!(fit(990.0, 1000.0, 120.0, 300.0), 700.0);
    }

    /// A room too small for both leasts is shared between them in
    /// proportion, so neither pane is the one that vanishes.
    #[test]
    fn a_room_too_small_for_both_is_shared() {
        assert_eq!(fit(50.0, 200.0, 100.0, 300.0), 50.0);
        assert_eq!(fit(190.0, 200.0, 100.0, 300.0), 50.0);
        assert_eq!(fit(10.0, 0.0, 100.0, 100.0), 0.0);
    }

    /// A share read as not a number stands in the middle, not at the edge.
    #[test]
    fn not_a_number_is_the_middle() {
        assert_eq!(fit(f32::NAN, 600.0, 100.0, 100.0), 300.0);
    }
}
