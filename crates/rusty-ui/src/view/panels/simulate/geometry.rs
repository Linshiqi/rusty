//! The board's shapes and arithmetic: what a part is, where its wires leave
//! it, which chip pin a point lands on.
//!
//! Pure functions and plain data, no view code — the canvas got its geometry
//! wrong three times in a row while none of it was testable, so this half
//! lives where a test can reach it.

use rusty_embed::SimBoard;

/// One thing on the canvas, whatever its kind — the editor edits these and
/// splits them back into the wire model on save.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum PartKind {
    Led { color: String },
    Button,
    Rgb,
    /// pins are segments a..g.
    Seven,
    /// Shows the `[rusty:disp]` channel; needs no pins.
    Display,
    /// A slider sending `P<pin>=<0..255>`.
    Pot,
}

impl PartKind {
    /// How many wires this part runs to the chip.
    pub(super) fn wires(&self) -> usize {
        match self {
            PartKind::Rgb => 3,
            PartKind::Seven => 7,
            PartKind::Display => 0,
            _ => 1,
        }
    }

    /// Fixed body height, so a turned part's anchors are exact rather than
    /// whatever the browser laid out.
    pub(super) fn height(&self) -> f64 {
        match self {
            PartKind::Seven => 52.0,
            PartKind::Display => 44.0,
            _ => 28.0,
        }
    }

    /// Fixed body width, so wire anchors land on the body edge exactly.
    pub(super) fn width(&self) -> f64 {
        match self {
            PartKind::Seven => 78.0,
            PartKind::Display => 140.0,
            PartKind::Pot => 130.0,
            _ => 110.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EditPart {
    /// Quarter turns, clockwise. The body rotates in CSS and the wire
    /// anchors rotate with the same arithmetic, so the two never disagree.
    pub(super) rot: u16,
    pub(super) kind: PartKind,
    /// pins[0] for LEDs, buttons and pots; RGB uses three; seven uses all.
    pub(super) pins: [u8; 7],
    pub(super) label: String,
    pub(super) x: f64,
    pub(super) y: f64,
    /// User-drawn bends per wire slot; empty routes automatically.
    pub(super) waypoints: [Vec<(f64, f64)>; 7],
}

pub(super) fn pins3(a: u8, b: u8, c: u8) -> [u8; 7] {
    [a, b, c, 0, 0, 0, 0]
}

/// The file's flat route list, spread into per-slot waypoint arrays.
pub(super) fn waypoints_of(routes: &[Vec<(f64, f64)>]) -> [Vec<(f64, f64)>; 7] {
    let mut out: [Vec<(f64, f64)>; 7] = Default::default();
    for (slot, route) in routes.iter().take(7).enumerate() {
        out[slot] = route.clone();
    }
    out
}

/// Back to the file shape: one route per wire slot, trailing empties kept so
/// slot indexes stay aligned, fully-empty lists collapsed to nothing.
pub(super) fn routes_of(waypoints: &[Vec<(f64, f64)>; 7], wires: usize) -> Vec<Vec<(f64, f64)>> {
    let slice = &waypoints[..wires];
    if slice.iter().all(Vec::is_empty) {
        Vec::new()
    } else {
        slice.to_vec()
    }
}

pub(super) fn parts_of(board: &SimBoard) -> Vec<EditPart> {
    let mut out = Vec::new();
    for (index, led) in board.leds.iter().enumerate() {
        out.push(EditPart {
            kind: PartKind::Led {
                color: led.color.clone(),
            },
            pins: pins3(led.pin, 0, 0),
            label: led.label.clone(),
            x: led.x.unwrap_or(60.0),
            y: led.y.unwrap_or(40.0 + index as f64 * 56.0),
            waypoints: waypoints_of(&led.routes),
            rot: led.rot,
        });
    }
    for button in &board.buttons {
        out.push(EditPart {
            kind: PartKind::Button,
            pins: pins3(button.pin, 0, 0),
            label: button.label.clone(),
            x: button.x.unwrap_or(60.0),
            y: button.y.unwrap_or(180.0),
            waypoints: waypoints_of(&button.routes),
            rot: button.rot,
        });
    }
    for rgb in &board.rgbs {
        out.push(EditPart {
            kind: PartKind::Rgb,
            pins: pins3(rgb.r, rgb.g, rgb.b),
            label: rgb.label.clone(),
            x: rgb.x.unwrap_or(60.0),
            y: rgb.y.unwrap_or(240.0),
            waypoints: waypoints_of(&rgb.routes),
            rot: rgb.rot,
        });
    }
    for seven in &board.sevens {
        out.push(EditPart {
            kind: PartKind::Seven,
            pins: seven.pins,
            label: seven.label.clone(),
            x: seven.x.unwrap_or(160.0),
            y: seven.y.unwrap_or(60.0),
            waypoints: waypoints_of(&seven.routes),
            rot: seven.rot,
        });
    }
    for display in &board.displays {
        out.push(EditPart {
            kind: PartKind::Display,
            pins: [0; 7],
            label: display.label.clone(),
            x: display.x.unwrap_or(160.0),
            y: display.y.unwrap_or(160.0),
            waypoints: Default::default(),
            rot: display.rot,
        });
    }
    for pot in &board.pots {
        out.push(EditPart {
            kind: PartKind::Pot,
            pins: pins3(pot.pin, 0, 0),
            label: pot.label.clone(),
            x: pot.x.unwrap_or(60.0),
            y: pot.y.unwrap_or(280.0),
            waypoints: waypoints_of(&pot.routes),
            rot: pot.rot,
        });
    }
    out
}

pub(super) fn board_of(chip: &str, kit: (f64, f64), parts: &[EditPart]) -> SimBoard {
    let mut board = SimBoard {
        chip: chip.to_string(),
        kit_x: Some(kit.0),
        kit_y: Some(kit.1),
        leds: Vec::new(),
        buttons: Vec::new(),
        rgbs: Vec::new(),
        sevens: Vec::new(),
        displays: Vec::new(),
        pots: Vec::new(),
    };
    for part in parts {
        match &part.kind {
            PartKind::Led { color } => board.leds.push(rusty_embed::SimLed {
                pin: part.pins[0],
                color: color.clone(),
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                routes: routes_of(&part.waypoints, 1),
                rot: part.rot,
            }),
            PartKind::Button => board.buttons.push(rusty_embed::SimButton {
                pin: part.pins[0],
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                routes: routes_of(&part.waypoints, 1),
                rot: part.rot,
            }),
            PartKind::Rgb => board.rgbs.push(rusty_embed::SimRgb {
                r: part.pins[0],
                g: part.pins[1],
                b: part.pins[2],
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                routes: routes_of(&part.waypoints, 3),
                rot: part.rot,
            }),
            PartKind::Seven => board.sevens.push(rusty_embed::SimSeven {
                pins: part.pins,
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                routes: routes_of(&part.waypoints, 7),
                rot: part.rot,
            }),
            PartKind::Display => board.displays.push(rusty_embed::SimDisplay {
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                rot: part.rot,
            }),
            PartKind::Pot => board.pots.push(rusty_embed::SimPot {
                pin: part.pins[0],
                label: part.label.clone(),
                x: Some(part.x),
                y: Some(part.y),
                routes: routes_of(&part.waypoints, 1),
                rot: part.rot,
            }),
        }
    }
    board
}

/// Colour classes for a single-hue lamp.
pub(super) fn lamp_classes(color: &str, lit: bool) -> &'static str {
    match (color, lit) {
        ("green", true) => "bg-[#3ddc84] shadow-[0_0_12px_3px_rgba(61,220,132,0.55)]",
        ("green", false) => "bg-[#1d4a2f]",
        ("blue", true) => "bg-[#4aa8ff] shadow-[0_0_12px_3px_rgba(74,168,255,0.55)]",
        ("blue", false) => "bg-[#1d3350]",
        ("red", true) => "bg-[#ff5c5c] shadow-[0_0_12px_3px_rgba(255,92,92,0.55)]",
        ("red", false) => "bg-[#4a1d1d]",
        ("yellow", true) => "bg-[#ffd75c] shadow-[0_0_12px_3px_rgba(255,215,92,0.5)]",
        ("yellow", false) => "bg-[#4a3f1d]",
        (_, true) => "bg-label shadow-[0_0_12px_3px_rgba(255,255,255,0.4)]",
        (_, false) => "bg-line-strong",
    }
}

/// Additive mix for the RGB lens, from three channel levels.
pub(super) fn rgb_style(r: bool, g: bool, b: bool) -> &'static str {
    match (r, g, b) {
        (false, false, false) => "background: #2a2d33",
        (true, false, false) => "background: #ff5c5c; box-shadow: 0 0 12px 3px rgba(255,92,92,0.55)",
        (false, true, false) => "background: #3ddc84; box-shadow: 0 0 12px 3px rgba(61,220,132,0.55)",
        (false, false, true) => "background: #4aa8ff; box-shadow: 0 0 12px 3px rgba(74,168,255,0.55)",
        (true, true, false) => "background: #ffd75c; box-shadow: 0 0 12px 3px rgba(255,215,92,0.55)",
        (true, false, true) => "background: #d97cff; box-shadow: 0 0 12px 3px rgba(217,124,255,0.55)",
        (false, true, true) => "background: #5ce8e8; box-shadow: 0 0 12px 3px rgba(92,232,232,0.55)",
        (true, true, true) => "background: #f4f4f4; box-shadow: 0 0 12px 3px rgba(255,255,255,0.5)",
    }
}

/// One editor state the undo stack holds: every part plus the kit position.
pub(super) type Snapshot = (Vec<EditPart>, (f64, f64));

/// What the pointer is currently moving.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Drag {
    Part { index: usize, dx: f64, dy: f64 },
    Kit { dx: f64, dy: f64 },
    /// Panning the sheet: screen-space start of view translation.
    Pan { start_tx: f64, start_ty: f64, px: f64, py: f64 },
    /// Pulling a new connection out of a part's pin stub.
    Wire { part: usize, slot: usize },
    /// Pushing one segment of a wire sideways, the way a schematic editor
    /// moves a corner: the segment keeps its direction and the two bends at
    /// its ends follow it.
    Segment {
        part: usize,
        slot: usize,
        first: usize,
        second: usize,
        horizontal: bool,
        /// Pointer position along the moving axis when the drag began.
        grab: f64,
        /// The segment's own position along that axis when it began.
        base: f64,
    },
}

/// What a right-click landed on. The menu is about this and nothing else —
/// that is the entire point of a context menu.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum MenuTarget {
    Wire(usize, usize),
    Part(usize),
    Sheet,
}

pub(super) const SNAP: f64 = 8.0;
pub(super) const KIT_W: f64 = 150.0;
pub(super) const KIT_H: f64 = 230.0;
/// Pin rows per side of the schematic devkit.
pub(super) const KIT_ROWS: usize = 15;
/// "This pin is not wired to anything" — new parts start here, and
/// disconnecting returns here. 255 is no GPIO on any supported chip.
pub(super) const UNWIRED: u8 = 255;

pub(super) fn snap(value: f64) -> f64 {
    (value / SNAP).round() * SNAP
}

/// The classic 30-pin ESP32 devkit pinout, top to bottom, left then right.
/// `None` rows (power, ground, EN) refuse wires — an LED soldered to GND on
/// both ends is a diagram nobody meant.
pub(super) fn kit_rows() -> [(&'static str, Option<u8>); 30] {
    [
        ("EN", None),
        ("36", Some(36)),
        ("39", Some(39)),
        ("34", Some(34)),
        ("35", Some(35)),
        ("32", Some(32)),
        ("33", Some(33)),
        ("25", Some(25)),
        ("26", Some(26)),
        ("27", Some(27)),
        ("14", Some(14)),
        ("12", Some(12)),
        ("13", Some(13)),
        ("GND", None),
        ("VIN", None),
        ("3V3", None),
        ("GND", None),
        ("15", Some(15)),
        ("2", Some(2)),
        ("4", Some(4)),
        ("16", Some(16)),
        ("17", Some(17)),
        ("5", Some(5)),
        ("18", Some(18)),
        ("19", Some(19)),
        ("21", Some(21)),
        ("RX", Some(3)),
        ("TX", Some(1)),
        ("22", Some(22)),
        ("23", Some(23)),
    ]
}

pub(super) fn row_of_gpio(pin: u8) -> Option<usize> {
    kit_rows().iter().position(|(_, gpio)| *gpio == Some(pin))
}

/// World coordinates of a kit row's pin circle.
pub(super) fn row_point(kit: (f64, f64), row: usize) -> (f64, f64) {
    if row < KIT_ROWS {
        (kit.0 + 10.0, kit.1 + 16.0 + row as f64 * 14.0)
    } else {
        (
            kit.0 + KIT_W - 10.0,
            kit.1 + 16.0 + (row - KIT_ROWS) as f64 * 14.0,
        )
    }
}

/// Which kit row a world point lands on, if any.
pub(super) fn row_under(kit: (f64, f64), point: (f64, f64)) -> Option<usize> {
    let (kx, ky) = kit;
    let (x, y) = point;
    if y < ky + 9.0 || y > ky + 16.0 + KIT_ROWS as f64 * 14.0 {
        return None;
    }
    let row = (((y - ky - 16.0) / 14.0).round().max(0.0)) as usize;
    if row >= KIT_ROWS {
        return None;
    }
    if x >= kx - 8.0 && x <= kx + 30.0 {
        Some(row)
    } else if x >= kx + KIT_W - 30.0 && x <= kx + KIT_W + 8.0 {
        Some(row + KIT_ROWS)
    } else {
        None
    }
}

/// A point turned about a centre, in quarter turns.
pub(super) fn rotate_about(point: (f64, f64), centre: (f64, f64), rot: u16) -> (f64, f64) {
    let (dx, dy) = (point.0 - centre.0, point.1 - centre.1);
    let (rx, ry) = match rot % 360 {
        90 => (-dy, dx),
        180 => (-dx, -dy),
        270 => (dy, -dx),
        _ => (dx, dy),
    };
    (centre.0 + rx, centre.1 + ry)
}

/// Where a part's `slot`-th wire leaves its body — after the part's own
/// rotation, because CSS turns the body about its centre and the wires have
/// to arrive at the same place the eye sees the stub.
pub(super) fn stub_point(part: &EditPart, slot: usize) -> (f64, f64) {
    let (w, h) = (part.kind.width(), part.kind.height());
    let upright = (part.x + w, part.y + 14.0 + slot as f64 * 6.0);
    rotate_about(upright, (part.x + w / 2.0, part.y + h / 2.0), part.rot)
}

/// The whole drawn path of one wire: the part's stub, the bends the user
/// has placed, and the chip pin — orthogonal by construction, as every
/// schematic wire is.
pub(super) fn wire_path(part: &EditPart, slot: usize, kit: (f64, f64)) -> Option<Vec<(f64, f64)>> {
    let pin = part.pins[slot];
    if pin == UNWIRED {
        return None;
    }
    let row = row_of_gpio(pin)?;
    let from = stub_point(part, slot);
    let to = row_point(kit, row);

    let mut points = vec![from];
    if part.waypoints[slot].is_empty() {
        // An untouched wire takes the tidy way round: out of the stub, along
        // a lane of its own, into the pin.
        let lane = if row < KIT_ROWS {
            to.0 - 24.0 - (row as f64 * 4.0)
        } else {
            to.0 + 24.0 + ((row - KIT_ROWS) as f64 * 4.0)
        };
        points.push((lane, from.1));
        points.push((lane, to.1));
    } else {
        points.extend(part.waypoints[slot].iter().copied());
    }
    points.push(to);
    Some(orthogonalize(points))
}

/// Insert elbows so every segment runs purely horizontally or vertically.
/// Idempotent, so a path that came back from a drag survives a round trip.
pub(super) fn orthogonalize(points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(points.len() + 2);
    out.push(points[0]);
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if (a.0 - b.0).abs() > 0.01 && (a.1 - b.1).abs() > 0.01 {
            out.push((b.0, a.1));
        }
        out.push(b);
    }
    out
}

/// The label a single-pin part wears for its wiring state.
pub(super) fn single_pin_label(kind: &PartKind, pin: u8) -> String {
    let base = match kind {
        PartKind::Button => "BTN",
        PartKind::Pot => "POT",
        _ => "GPIO",
    };
    if pin == UNWIRED {
        format!("{base} —")
    } else {
        format!("{base}{pin}")
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn led(x: f64, y: f64, pin: u8) -> EditPart {
        EditPart {
            kind: PartKind::Led {
                color: "green".to_string(),
            },
            pins: [pin, 0, 0, 0, 0, 0, 0],
            label: "GPIO".to_string(),
            x,
            y,
            waypoints: Default::default(),
            rot: 0,
        }
    }

    #[test]
    fn the_pin_map_is_the_real_devkit_and_power_rows_refuse_wires() {
        assert_eq!(kit_rows().len(), 30);
        assert_eq!(row_of_gpio(26), Some(8));
        assert_eq!(row_of_gpio(23), Some(29));
        // RX and TX carry GPIO numbers; EN, GND, VIN and 3V3 carry none.
        assert_eq!(row_of_gpio(3), Some(26));
        for name in ["EN", "GND", "VIN", "3V3"] {
            assert!(
                kit_rows().iter().all(|(n, gpio)| *n != name || gpio.is_none()),
                "{name} must not offer a GPIO",
            );
        }
        assert_eq!(row_of_gpio(UNWIRED), None);
    }

    #[test]
    fn a_turned_part_moves_its_stub_with_it() {
        let mut part = led(100.0, 100.0, 26);
        let upright = stub_point(&part, 0);
        // Width 110, height 28 → centre (155, 114); the stub sits at
        // (210, 114), so a quarter turn swings it to the bottom of the body.
        assert_eq!(upright, (210.0, 114.0));

        part.rot = 90;
        let turned = stub_point(&part, 0);
        assert_eq!(turned, (155.0, 169.0));

        part.rot = 180;
        assert_eq!(stub_point(&part, 0), (100.0, 114.0));

        part.rot = 360;
        assert_eq!(stub_point(&part, 0), upright, "a full turn is no turn");
    }

    #[test]
    fn a_wire_runs_only_in_right_angles_and_ends_on_its_pin() {
        let part = led(60.0, 40.0, 26);
        let kit = (460.0, 40.0);
        let path = wire_path(&part, 0, kit).expect("a wired pin has a path");

        assert_eq!(path[0], stub_point(&part, 0), "starts at the stub");
        assert_eq!(
            *path.last().unwrap(),
            row_point(kit, row_of_gpio(26).unwrap()),
            "ends on the pin",
        );
        for pair in path.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(
                (a.0 - b.0).abs() < 0.01 || (a.1 - b.1).abs() < 0.01,
                "every segment is horizontal or vertical: {a:?} → {b:?}",
            );
        }
    }

    #[test]
    fn an_unwired_pin_has_no_path_at_all() {
        let part = led(60.0, 40.0, UNWIRED);
        assert!(wire_path(&part, 0, (460.0, 40.0)).is_none());
        // A pin the chip does not have is refused rather than drawn to
        // nowhere — this is how a board file from another chip fails.
        let alien = led(60.0, 40.0, 99);
        assert!(wire_path(&alien, 0, (460.0, 40.0)).is_none());
    }

    #[test]
    fn user_bends_survive_the_orthogonal_pass() {
        let mut part = led(60.0, 40.0, 26);
        part.waypoints[0] = vec![(200.0, 54.0), (200.0, 168.0)];
        let path = wire_path(&part, 0, (460.0, 40.0)).expect("path");
        assert!(path.contains(&(200.0, 54.0)));
        assert!(path.contains(&(200.0, 168.0)));
        // Idempotent: a path already square gains no extra corners.
        assert_eq!(orthogonalize(path.clone()), path);
    }

    #[test]
    fn only_pin_rows_accept_a_dropped_wire() {
        let kit = (460.0, 40.0);
        let row = row_of_gpio(26).expect("gpio 26");
        let (px, py) = row_point(kit, row);
        assert_eq!(row_under(kit, (px, py)), Some(row));
        // A few pixels off still lands, because a pin is a small target.
        assert_eq!(row_under(kit, (px + 6.0, py + 3.0)), Some(row));
        // The middle of the board is not a pin.
        assert_eq!(row_under(kit, (kit.0 + 75.0, py)), None);
        // Neither is empty sheet.
        assert_eq!(row_under(kit, (kit.0 - 200.0, py)), None);
    }

    #[test]
    fn parts_survive_the_round_trip_through_the_wire_model() {
        let mut part = led(60.0, 40.0, 26);
        part.rot = 90;
        part.waypoints[0] = vec![(200.0, 54.0)];
        let board = board_of("esp32", (460.0, 40.0), &[part.clone()]);

        assert_eq!(board.leds.len(), 1);
        assert_eq!(board.leds[0].rot, 90);
        assert_eq!(board.leds[0].routes, vec![vec![(200.0, 54.0)]]);
        assert_eq!(board.kit_x, Some(460.0));

        let back = parts_of(&board);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], part);
    }

    #[test]
    fn the_snap_grid_rounds_both_ways() {
        assert_eq!(snap(0.0), 0.0);
        assert_eq!(snap(3.0), 0.0);
        assert_eq!(snap(5.0), 8.0);
        assert_eq!(snap(-3.0), -0.0);
        assert_eq!(snap(-5.0), -8.0);
    }
}
