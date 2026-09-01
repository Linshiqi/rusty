//! What the board editor's commands do to the sheet, with no signals in them.
//!
//! The sibling of [`super::geometry`], and there for the same reason: the
//! canvas is a 1,900-line component, and everything inside it that is really
//! arithmetic or bookkeeping was unreachable from a test. Geometry moved out
//! first because it had been wrong three times; this is the other half — what
//! rotating, deleting, duplicating and undoing actually *do* to the part list.
//!
//! Every function here takes the list and mutates it. The component keeps the
//! signals, calls these, and sets `dirty` — so a command is one line there and
//! its behaviour is pinned here.

use super::geometry::{EditPart, PartKind, Snapshot, UNWIRED, single_pin_label};

/// How many steps of undo the editor keeps.
///
/// Bounded because each snapshot is a whole copy of the sheet and a long
/// session otherwise grows without limit; a hundred is far past what anyone
/// walks back through in one sitting.
const HISTORY: usize = 100;

/// Record the sheet before a change, dropping the oldest step once the cap is
/// reached.
pub(super) fn remember(past: &mut Vec<Snapshot>, now: Snapshot) {
    past.push(now);
    if past.len() > HISTORY {
        past.remove(0);
    }
}

/// The label a freshly placed part carries.
///
/// An em dash rather than a pin number, because a new part is deliberately
/// unwired: connecting it is the user's move, made by pulling its stub to a
/// chip pin. A label naming a pin nothing is attached to is a lie the user
/// has to notice.
pub(super) fn new_label(kind: &PartKind, stub: &str) -> String {
    match kind {
        PartKind::Led { .. } => "GPIO —".to_string(),
        PartKind::Button => "BTN —".to_string(),
        PartKind::Rgb if stub.is_empty() => "RGB".to_string(),
        PartKind::Rgb => stub.to_string(),
        PartKind::Seven => "7SEG".to_string(),
        PartKind::Display => "DISPLAY".to_string(),
        PartKind::Pot => "POT —".to_string(),
    }
}

/// Place a new part, returning its index — which is what the caller selects.
pub(super) fn add(list: &mut Vec<EditPart>, kind: PartKind, stub: &str, x: f64, y: f64) -> usize {
    let label = new_label(&kind, stub);
    list.push(EditPart {
        kind,
        pins: [UNWIRED; 7],
        label,
        x,
        y,
        waypoints: Default::default(),
        rot: 0,
        flip: false,
    });
    list.len() - 1
}

/// Turn a part a quarter turn clockwise.
pub(super) fn rotate(list: &mut [EditPart], index: usize) {
    let Some(part) = list.get_mut(index) else {
        return;
    };
    part.rot = (part.rot + 90) % 360;
    // The stubs have moved, so hand-drawn routes to them are about a shape
    // that no longer exists.
    for route in &mut part.waypoints {
        route.clear();
    }
}

/// Mirror a part left-to-right.
///
/// Not a rotation: a 180° turn brings the stubs to the near edge and reverses
/// their order, so seven wires to a seven-segment cross on the way in.
pub(super) fn flip(list: &mut [EditPart], index: usize) {
    if let Some(part) = list.get_mut(index) {
        part.flip = !part.flip;
        for route in &mut part.waypoints {
            route.clear();
        }
    }
}

pub(super) fn nudge(list: &mut [EditPart], index: usize, dx: f64, dy: f64) {
    if let Some(part) = list.get_mut(index) {
        part.x += dx;
        part.y += dy;
    }
}

/// Drop the hand-drawn bends on one wire, leaving it to route itself.
pub(super) fn straighten(list: &mut [EditPart], index: usize, slot: usize) {
    if let Some(part) = list.get_mut(index) {
        part.waypoints[slot].clear();
    }
}

/// Unwire one slot: the pin goes, and so do the bends that were drawn for it.
///
/// A single-pin part's label names its pin, so it goes back to naming nothing
/// — a lamp still labelled `GPIO26` with no wire on it is the confident wrong
/// answer in miniature.
pub(super) fn disconnect(list: &mut [EditPart], index: usize, slot: usize) {
    let Some(part) = list.get_mut(index) else {
        return;
    };
    part.pins[slot] = UNWIRED;
    part.waypoints[slot].clear();
    if part.kind.wires() == 1 {
        part.label = single_pin_label(&part.kind, UNWIRED);
    }
}

pub(super) fn remove(list: &mut Vec<EditPart>, index: usize) {
    if index < list.len() {
        list.remove(index);
    }
}

/// Copy a part beside itself, returning the copy's index.
pub(super) fn duplicate(list: &mut Vec<EditPart>, index: usize) -> Option<usize> {
    let original = list.get(index).cloned()?;
    // Same pins — two lamps on one GPIO is legal wiring — but its own routes,
    // because the copy sits somewhere else.
    list.push(EditPart {
        x: original.x + 24.0,
        y: original.y + 24.0,
        waypoints: Default::default(),
        ..original
    });
    Some(list.len() - 1)
}

/// The rectangle everything on the sheet occupies: `((min_x, min_y), (max_x,
/// max_y))`, the devkit included.
///
/// Its own function because "fit the view" is the one place the editor has to
/// be right about every part's *extent* rather than its origin, and a part
/// whose width is forgotten is one that ends up half off screen.
pub(super) fn bounds(
    list: &[EditPart],
    kit: (f64, f64),
    kit_size: (f64, f64),
) -> ((f64, f64), (f64, f64)) {
    let mut min = kit;
    let mut max = (kit.0 + kit_size.0, kit.1 + kit_size.1);
    for part in list {
        min.0 = min.0.min(part.x);
        min.1 = min.1.min(part.y);
        max.0 = max.0.max(part.x + part.kind.width());
        max.1 = max.1.max(part.y + part.kind.height());
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet() -> Vec<EditPart> {
        let mut list = Vec::new();
        let at = add(
            &mut list,
            PartKind::Led {
                color: "green".to_string(),
            },
            "",
            60.0,
            40.0,
        );
        list[at].pins[0] = 26;
        list[at].label = "GPIO26".to_string();
        list[at].waypoints[0] = vec![(120.0, 40.0)];
        list
    }

    #[test]
    fn a_new_part_arrives_unwired_and_says_so() {
        let mut list = Vec::new();
        let at = add(&mut list, PartKind::Button, "", 10.0, 20.0);
        assert_eq!(at, 0);
        assert_eq!(
            list[0].pins, [UNWIRED; 7],
            "connecting it is the user's move"
        );
        assert_eq!(
            list[0].label, "BTN —",
            "and the label must not name a pin nothing is attached to",
        );
        // An RGB carries whatever the library entry called it, when it has one.
        let mut list = Vec::new();
        add(&mut list, PartKind::Rgb, "STATUS", 0.0, 0.0);
        assert_eq!(list[0].label, "STATUS");
    }

    /// Turning or mirroring moves the stubs, so routes drawn to where they
    /// *were* describe a shape that no longer exists.
    #[test]
    fn turning_a_part_drops_the_routes_drawn_to_its_old_stubs() {
        let mut list = sheet();
        rotate(&mut list, 0);
        assert_eq!(list[0].rot, 90);
        assert!(list[0].waypoints[0].is_empty());

        let mut list = sheet();
        flip(&mut list, 0);
        assert!(list[0].flip);
        assert!(list[0].waypoints[0].is_empty());
        flip(&mut list, 0);
        assert!(!list[0].flip, "mirroring twice is not mirrored");
    }

    #[test]
    fn a_full_turn_comes_back_upright() {
        let mut list = sheet();
        for _ in 0..4 {
            rotate(&mut list, 0);
        }
        assert_eq!(list[0].rot, 0);
    }

    #[test]
    fn disconnecting_takes_the_pin_the_route_and_the_label() {
        let mut list = sheet();
        disconnect(&mut list, 0, 0);
        assert_eq!(list[0].pins[0], UNWIRED);
        assert!(list[0].waypoints[0].is_empty());
        assert!(
            !list[0].label.contains("26"),
            "a lamp still labelled GPIO26 with no wire on it is a lie: {}",
            list[0].label,
        );
    }

    #[test]
    fn straightening_keeps_the_wire_and_drops_only_the_bends() {
        let mut list = sheet();
        straighten(&mut list, 0, 0);
        assert!(list[0].waypoints[0].is_empty());
        assert_eq!(list[0].pins[0], 26, "the wire itself stays connected");
    }

    #[test]
    fn a_duplicate_shares_the_pins_and_not_the_routes() {
        let mut list = sheet();
        let copy = duplicate(&mut list, 0).expect("a copy");
        assert_eq!(list.len(), 2);
        assert_eq!(
            list[copy].pins[0], 26,
            "two lamps on one GPIO is legal wiring"
        );
        assert!(
            list[copy].waypoints[0].is_empty(),
            "but the copy sits elsewhere, so its route is its own",
        );
        assert!(list[copy].x > list[0].x && list[copy].y > list[0].y);
    }

    /// Every operation has to survive an index that is not there: the panel
    /// holds a selection across edits, and a stale one arriving here must do
    /// nothing rather than panic in a WebView with no stack trace.
    #[test]
    fn a_stale_index_does_nothing() {
        let mut list = sheet();
        rotate(&mut list, 9);
        flip(&mut list, 9);
        nudge(&mut list, 9, 1.0, 1.0);
        straighten(&mut list, 9, 0);
        disconnect(&mut list, 9, 0);
        remove(&mut list, 9);
        assert_eq!(duplicate(&mut list, 9), None);
        assert_eq!(list, sheet(), "nothing moved");
    }

    #[test]
    fn history_is_capped_and_keeps_the_newest() {
        let mut past: Vec<Snapshot> = Vec::new();
        for n in 0..HISTORY + 10 {
            remember(&mut past, (Vec::new(), (n as f64, 0.0)));
        }
        assert_eq!(past.len(), HISTORY);
        assert_eq!(
            past.last().unwrap().1.0,
            (HISTORY + 9) as f64,
            "the newest step is the one undo reaches first",
        );
        assert_eq!(past[0].1.0, 10.0, "and the oldest fell off the front");
    }

    /// The bug this prevents: framing the sheet by part *origins* leaves
    /// every part's body hanging off the right and bottom edges.
    #[test]
    fn bounds_cover_each_parts_body_not_just_its_corner() {
        let list = sheet();
        let (min, max) = bounds(&list, (460.0, 40.0), (150.0, 300.0));
        assert_eq!(min, (60.0, 40.0), "the leftmost part sets the left edge");
        assert_eq!(max.0, 610.0, "the devkit's right edge is furthest right");
        let part_right = list[0].x + list[0].kind.width();
        assert!(max.0 >= part_right, "and no body may fall outside");
        assert!(max.1 >= 340.0);
    }
}
