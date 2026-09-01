//! The board's shapes and arithmetic: what a part is, where its wires leave
//! it, which chip pin a point lands on.
//!
//! Pure functions and plain data, no view code — the canvas got its geometry
//! wrong three times in a row while none of it was testable, so this half
//! lives where a test can reach it.

use rusty_embed::{Placement, SimBoard};

/// One thing on the canvas, whatever its kind — the editor edits these and
/// splits them back into the wire model on save.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum PartKind {
    Led {
        color: String,
    },
    Button,
    Rgb,
    /// pins are segments a..g.
    Seven,
    /// Shows the `[rusty:disp]` channel; needs no pins.
    Display,
    /// A slider sending `P<pin>=<0..255>`.
    Pot,
    /// A toy car's drive or a fan. Slot 0 is the duty pin; slots 1 and 2 are
    /// the H-bridge's direction inputs, and leaving them unwired is what
    /// makes it a fan.
    Motor,
    /// A voltage on a pin — a battery through a divider, a thermistor. Sends
    /// raw ADC counts, because rusty does not know your divider.
    Analog,
}

impl PartKind {
    /// How many wires this part runs to the chip.
    pub(super) fn wires(&self) -> usize {
        match self {
            PartKind::Rgb => 3,
            PartKind::Seven => 7,
            // SDA and SCL, like the I2C module it stands for. The screen
            // content still arrives over the serial channel today; the pins
            // are where the coming I2C decode will attach, and a part that
            // floats outside the circuit reads as a mistake either way.
            PartKind::Display => 2,
            // Duty, then the two direction inputs. A fan wires only the
            // first and leaves the other two stubs hanging, which is how the
            // sheet shows that it turns one way.
            PartKind::Motor => 3,
            _ => 1,
        }
    }

    /// Fixed body height, so a turned part's anchors are exact rather than
    /// whatever the browser laid out. Tall enough for every stub at the
    /// chip's own row pitch: seven pins crowded into 52px read as one
    /// smudge, and KiCad's oldest rule is that pins sit a full grid step
    /// apart.
    pub(super) fn height(&self) -> f64 {
        let wired = self.wires().max(1) as f64;
        wired.mul_add(SLOT_PITCH, 12.0).max(28.0)
    }

    /// Fixed body width, so wire anchors land on the body edge exactly.
    /// Widths are grid multiples, like everything an anchor derives from —
    /// a 110px body put every stub 6px off the grid however the part snapped.
    pub(super) fn width(&self) -> f64 {
        match self {
            PartKind::Seven => 80.0,
            PartKind::Display => 144.0,
            PartKind::Pot => 128.0,
            // Room for the slider and the count beside it.
            PartKind::Analog => 136.0,
            // Wide enough for the rotor and the readout beside it: "BRAKE"
            // and "100%" have to fit without the body growing when they
            // appear, or the wires move every time the motor changes state.
            PartKind::Motor => 128.0,
            _ => 112.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EditPart {
    /// Quarter turns, clockwise. The body rotates in CSS and the wire
    /// anchors rotate with the same arithmetic, so the two never disagree.
    pub(super) rot: u16,
    /// Mirrored left-to-right, KiCad's X key. This is what a part on the
    /// chip's right wants: its stubs face back towards the pins *without*
    /// reversing their order, which is the one thing rotating by 180 gets
    /// wrong — and the reason wires to a turned seven-segment crossed.
    pub(super) flip: bool,
    pub(super) kind: PartKind,
    /// pins[0] for LEDs, buttons and pots; RGB uses three; seven uses all.
    pub(super) pins: [u8; 7],
    pub(super) label: String,
    pub(super) x: f64,
    pub(super) y: f64,
    /// User-drawn bends per wire slot; empty routes automatically.
    pub(super) waypoints: [Vec<(f64, f64)>; 7],
}

impl EditPart {
    /// A part from the wire model's shared half. `fallback` is where one with
    /// no recorded position goes — a hand-written board file has none.
    ///
    /// One conversion for all six kinds, in each direction. They used to be
    /// six copies each way, and a field added to five of them and missed on
    /// the sixth is invisible until somebody uses that part.
    fn placed(
        kind: PartKind,
        pins: [u8; 7],
        label: &str,
        place: &Placement,
        fallback: (f64, f64),
    ) -> Self {
        EditPart {
            kind,
            pins,
            label: label.to_string(),
            x: place.x.unwrap_or(fallback.0),
            y: place.y.unwrap_or(fallback.1),
            waypoints: waypoints_of(&place.routes),
            rot: place.rot,
            flip: place.flip,
        }
    }

    /// The shared half, back on the wire.
    ///
    /// The wire count comes from the kind rather than from the caller: it is
    /// a property of what the part *is*, and passing it in is one more place
    /// to write 3 where a seven-segment wanted 7.
    fn place(&self) -> Placement {
        Placement {
            x: Some(self.x),
            y: Some(self.y),
            routes: routes_of(&self.waypoints, self.kind.wires()),
            rot: self.rot,
            flip: self.flip,
        }
    }
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
        out.push(EditPart::placed(
            PartKind::Led {
                color: led.color.clone(),
            },
            pins3(led.pin, 0, 0),
            &led.label,
            &led.place,
            // Stacked down the left edge, so a hand-written file with several
            // LEDs and no positions does not pile them all in one spot.
            (60.0, 40.0 + index as f64 * 56.0),
        ));
    }
    for button in &board.buttons {
        out.push(EditPart::placed(
            PartKind::Button,
            pins3(button.pin, 0, 0),
            &button.label,
            &button.place,
            (60.0, 180.0),
        ));
    }
    for rgb in &board.rgbs {
        out.push(EditPart::placed(
            PartKind::Rgb,
            pins3(rgb.r, rgb.g, rgb.b),
            &rgb.label,
            &rgb.place,
            (60.0, 240.0),
        ));
    }
    for seven in &board.sevens {
        out.push(EditPart::placed(
            PartKind::Seven,
            seven.pins,
            &seven.label,
            &seven.place,
            (160.0, 60.0),
        ));
    }
    for display in &board.displays {
        out.push(EditPart::placed(
            PartKind::Display,
            [display.sda, display.scl, 0, 0, 0, 0, 0],
            &display.label,
            &display.place,
            (160.0, 160.0),
        ));
    }
    for analog in &board.analogs {
        out.push(EditPart::placed(
            PartKind::Analog,
            pins3(analog.pin, 0, 0),
            &analog.label,
            &analog.place,
            (60.0, 360.0),
        ));
    }
    for motor in &board.motors {
        out.push(EditPart::placed(
            PartKind::Motor,
            [motor.pwm, motor.in1, motor.in2, 0, 0, 0, 0],
            &motor.label,
            &motor.place,
            (160.0, 300.0),
        ));
    }
    for pot in &board.pots {
        out.push(EditPart::placed(
            PartKind::Pot,
            pins3(pot.pin, 0, 0),
            &pot.label,
            &pot.place,
            (60.0, 280.0),
        ));
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
        motors: Vec::new(),
        analogs: Vec::new(),
    };
    for part in parts {
        let place = part.place();
        match &part.kind {
            PartKind::Led { color } => board.leds.push(rusty_embed::SimLed {
                pin: part.pins[0],
                color: color.clone(),
                label: part.label.clone(),
                place,
            }),
            PartKind::Button => board.buttons.push(rusty_embed::SimButton {
                pin: part.pins[0],
                label: part.label.clone(),
                place,
            }),
            PartKind::Rgb => board.rgbs.push(rusty_embed::SimRgb {
                r: part.pins[0],
                g: part.pins[1],
                b: part.pins[2],
                label: part.label.clone(),
                place,
            }),
            PartKind::Seven => board.sevens.push(rusty_embed::SimSeven {
                pins: part.pins,
                label: part.label.clone(),
                place,
            }),
            PartKind::Display => board.displays.push(rusty_embed::SimDisplay {
                label: part.label.clone(),
                sda: part.pins[0],
                scl: part.pins[1],
                place,
            }),
            PartKind::Pot => board.pots.push(rusty_embed::SimPot {
                pin: part.pins[0],
                label: part.label.clone(),
                place,
            }),
            PartKind::Analog => board.analogs.push(rusty_embed::SimAnalog {
                pin: part.pins[0],
                label: part.label.clone(),
                // The editor does not edit these two yet; a hand-written file
                // that set them keeps them because `board_of` is only reached
                // from the canvas, and the canvas only ever adds defaults.
                max: 4095,
                start: 0,
                note: None,
                place,
            }),
            PartKind::Motor => board.motors.push(rusty_embed::SimMotor {
                pwm: part.pins[0],
                in1: part.pins[1],
                in2: part.pins[2],
                label: part.label.clone(),
                place,
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
        (true, false, false) => {
            "background: #ff5c5c; box-shadow: 0 0 12px 3px rgba(255,92,92,0.55)"
        }
        (false, true, false) => {
            "background: #3ddc84; box-shadow: 0 0 12px 3px rgba(61,220,132,0.55)"
        }
        (false, false, true) => {
            "background: #4aa8ff; box-shadow: 0 0 12px 3px rgba(74,168,255,0.55)"
        }
        (true, true, false) => {
            "background: #ffd75c; box-shadow: 0 0 12px 3px rgba(255,215,92,0.55)"
        }
        (true, false, true) => {
            "background: #d97cff; box-shadow: 0 0 12px 3px rgba(217,124,255,0.55)"
        }
        (false, true, true) => {
            "background: #5ce8e8; box-shadow: 0 0 12px 3px rgba(92,232,232,0.55)"
        }
        (true, true, true) => "background: #f4f4f4; box-shadow: 0 0 12px 3px rgba(255,255,255,0.5)",
    }
}

/// One editor state the undo stack holds: every part plus the kit position.
pub(super) type Snapshot = (Vec<EditPart>, (f64, f64));

/// What the pointer is currently moving.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Drag {
    Part {
        index: usize,
        dx: f64,
        dy: f64,
        /// Each route's first-segment axis, judged once when the drag began
        /// — the first bend slides along it so the segment stays parallel
        /// to itself (see [`follow_first_bend`]).
        axes: [Option<bool>; 7],
    },
    Kit {
        dx: f64,
        dy: f64,
    },
    /// Panning the sheet: screen-space start of view translation.
    Pan {
        start_tx: f64,
        start_ty: f64,
        px: f64,
        py: f64,
    },
    /// Pulling a new connection out of a part's pin stub.
    Wire {
        part: usize,
        slot: usize,
    },
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
/// Pin-row pitch on the kit, and the wire-stub pitch on a part. Multiples of
/// the base grid on purpose — KiCad's oldest rule is that pins live on the
/// grid, because a snapped segment can only ever meet an anchor that is
/// itself snapped. The old 14px/6px pitches put every anchor 2px off the
/// 8px grid, which is why nothing could align no matter how carefully it
/// was dragged.
pub(super) const ROW_PITCH: f64 = 16.0;
pub(super) const STUB_OFFSET: f64 = 16.0;
pub(super) const SLOT_PITCH: f64 = 16.0;
/// "This pin is not wired to anything" — new parts start here, and
/// disconnecting returns here. 255 is no GPIO on any supported chip.
pub(super) const UNWIRED: u8 = 255;

/// The same rounding on a user-chosen grid — the toolbar offers 1/4/8/16px,
/// because "the grid is too coarse to align" deserved a dial, even after
/// the real cause (off-grid anchors) was fixed.
pub(super) fn snap_to(value: f64, grid: f64) -> f64 {
    if grid <= 1.0 {
        return value.round();
    }
    (value / grid).round() * grid
}

/// One row of the drawn part: its label, and the GPIO it carries.
///
/// `None` rows — power, ground, EN — refuse wires: an LED soldered to GND on
/// both ends is a diagram nobody meant.
pub(super) type Row = (String, Option<u8>);

/// The classic 30-pin ESP32 devkit pinout, top to bottom, left then right.
///
/// The one module whose *header order* rusty knows. That order is a property
/// of the board, not the die, so it cannot be derived — and it used to be
/// drawn for every chip, which is why an ESP32-C3 board showed GPIO36, 39, 34
/// and 35, none of which the part has.
const ESP32_DEVKIT: [(&str, Option<u8>); 30] = [
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
];

/// The rows to draw for a part, given the GPIOs it actually has.
///
/// Two different drawings, and the difference is honest rather than
/// cosmetic. For the ESP32 the answer is a *module*: a real 30-pin devkit
/// whose header order somebody can match against the board on their desk.
/// For everything else rusty knows the die's pins and not any module's
/// header, so it draws a *chip* — the pins in numeric order, split down the
/// middle, with the rails around them. Every row on screen is then a pin
/// that exists, which is the whole of the bug this replaced.
///
/// An empty `gpio` means the catalogue does not say, and the part is drawn
/// with rails only rather than with somebody else's pins.
pub(super) fn kit_rows(chip: &str, gpio: &[u32]) -> Vec<Row> {
    if chip == "esp32" {
        return ESP32_DEVKIT
            .iter()
            .map(|(name, pin)| ((*name).to_string(), *pin))
            .collect();
    }

    let half = gpio.len().div_ceil(2);
    let mut rows: Vec<Row> = Vec::with_capacity(gpio.len() + 4);
    rows.push(("EN".to_string(), None));
    rows.extend(gpio[..half].iter().map(|p| (p.to_string(), Some(*p as u8))));
    rows.push(("GND".to_string(), None));
    // The right column starts here, so the rails sit at the top of each side
    // the way they do on a module.
    rows.push(("3V3".to_string(), None));
    rows.extend(gpio[half..].iter().map(|p| (p.to_string(), Some(*p as u8))));
    rows.push(("GND".to_string(), None));
    rows
}

/// Which row carries a GPIO, if the part has it at all.
///
/// `None` for a pin the part does not have is the answer, not a gap: a wire
/// to GPIO26 on a C3 has nowhere to land, and drawing it somewhere would be
/// the confident wrong answer.
pub(super) fn row_of_gpio(rows: &[Row], pin: u8) -> Option<usize> {
    rows.iter().position(|(_, gpio)| *gpio == Some(pin))
}

/// How many rows go down the left side. The split is the same arithmetic the
/// row builder used, kept in one place so a drawing and a hit-test cannot
/// disagree about which side a row is on.
pub(super) fn left_rows(rows: usize) -> usize {
    rows.div_ceil(2)
}

/// World coordinates of a kit row's pin circle.
pub(super) fn row_point(kit: (f64, f64), rows: usize, row: usize) -> (f64, f64) {
    let left = left_rows(rows);
    if row < left {
        (kit.0 + 10.0, kit.1 + 16.0 + row as f64 * ROW_PITCH)
    } else {
        (
            kit.0 + KIT_W - 10.0,
            kit.1 + 16.0 + (row - left) as f64 * ROW_PITCH,
        )
    }
}

/// How tall the drawn part is for a given row count — the rails and pins
/// decide it now, rather than a constant that only ever suited one module.
pub(super) fn kit_height(rows: usize) -> f64 {
    32.0 + left_rows(rows) as f64 * ROW_PITCH
}

/// Which kit row a world point lands on, if any.
pub(super) fn row_under(kit: (f64, f64), rows: usize, point: (f64, f64)) -> Option<usize> {
    let (kx, ky) = kit;
    let (x, y) = point;
    let left = left_rows(rows);
    if y < ky + 9.0 || y > ky + 16.0 + left as f64 * ROW_PITCH {
        return None;
    }
    let row = (((y - ky - 16.0) / ROW_PITCH).round().max(0.0)) as usize;
    if row >= left {
        return None;
    }
    if x >= kx - 8.0 && x <= kx + 30.0 {
        Some(row)
    } else if x >= kx + KIT_W - 30.0 && x <= kx + KIT_W + 8.0 {
        // The right column can be shorter than the left when the count is
        // odd; a hit past its end is a miss, not the row below.
        (row + left < rows).then_some(row + left)
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
    // Mirroring moves the stubs to the left edge and leaves their order
    // alone; rotation then applies to whichever edge they ended on.
    let x = if part.flip { part.x } else { part.x + w };
    let upright = (x, part.y + STUB_OFFSET + slot as f64 * SLOT_PITCH);
    rotate_about(upright, (part.x + w / 2.0, part.y + h / 2.0), part.rot)
}

/// The whole drawn path of one wire: the part's stub, the bends the user
/// has placed, and the chip pin — orthogonal by construction, as every
/// schematic wire is.
pub(super) fn wire_path(
    part: &EditPart,
    slot: usize,
    kit: (f64, f64),
    rows: &[Row],
) -> Option<Vec<(f64, f64)>> {
    let pin = part.pins[slot];
    if pin == UNWIRED {
        return None;
    }
    let row = row_of_gpio(rows, pin)?;
    let from = stub_point(part, slot);
    let to = row_point(kit, rows.len(), row);

    let mut points = vec![from];
    if part.waypoints[slot].is_empty() {
        // An untouched wire takes the tidy way round: out of the stub, along
        // a lane of its own, into the pin.
        let left = left_rows(rows.len());
        let lane = if row < left {
            to.0 - 24.0 - (row as f64 * 8.0)
        } else {
            to.0 + 24.0 + ((row - left) as f64 * 8.0)
        };
        points.push((lane, from.1));
        points.push((lane, to.1));
    } else {
        points.extend(part.waypoints[slot].iter().copied());
    }
    points.push(to);
    Some(orthogonalize(points))
}

/// Drop the points a route no longer needs: consecutive duplicates, and any
/// bend whose neighbours run straight through it. This is what merges two
/// segments the user has dragged into line — the KiCad behaviour: aligned
/// segments become one segment, and the next grab moves them as one.
pub(super) fn simplify_route(full: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(full.len());
    for point in full {
        if let Some(last) = out.last()
            && (last.0 - point.0).abs() < 0.01
            && (last.1 - point.1).abs() < 0.01
        {
            continue;
        }
        out.push(point);
        while out.len() >= 3 {
            let c = out[out.len() - 1];
            let b = out[out.len() - 2];
            let a = out[out.len() - 3];
            let collinear = ((a.0 - b.0).abs() < 0.01 && (b.0 - c.0).abs() < 0.01)
                || ((a.1 - b.1).abs() < 0.01 && (b.1 - c.1).abs() < 0.01);
            if collinear {
                out.remove(out.len() - 2);
            } else {
                break;
            }
        }
    }
    out
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

/// Which way a route's first segment runs — stub against first planted
/// bend. `Some(true)` is horizontal. Judged once, when the drag starts:
/// judging it live would flip as the part crosses the bend.
pub(super) fn first_leg_axis(stub: (f64, f64), first: (f64, f64)) -> Option<bool> {
    if (stub.1 - first.1).abs() < 0.01 {
        Some(true)
    } else if (stub.0 - first.0).abs() < 0.01 {
        Some(false)
    } else {
        None
    }
}

/// KiCad's stretch, completed: dragging a part slides the first bend along
/// the first segment's own axis, so the segment stays parallel to itself
/// and only changes length. Without this, a planted bend at the old stub
/// height makes the route run out, all the way back up to where the part
/// used to be, and down again — a wall of wire the user never drew.
pub(super) fn follow_first_bend(stub: (f64, f64), axis: Option<bool>, first: &mut (f64, f64)) {
    match axis {
        Some(true) => first.1 = stub.1,
        Some(false) => first.0 = stub.0,
        None => {}
    }
}

/// The label a single-pin part wears for its wiring state.
pub(super) fn single_pin_label(kind: &PartKind, pin: u8) -> String {
    let base = match kind {
        PartKind::Button => "BTN",
        PartKind::Pot => "POT",
        PartKind::Motor => "PWM",
        PartKind::Analog => "ADC",
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

    /// Mirroring is what a part on the chip's right needs: stubs on the
    /// near edge, in the same order. Rotating by 180 also brings them
    /// near, but reverses them — which is what made seven wires cross.
    #[test]
    fn mirroring_moves_the_stubs_without_reordering_them() {
        let mut part = led(200.0, 100.0, 26);
        part.kind = PartKind::Seven;
        part.pins = [1, 2, 3, 4, 5, 6, 7];

        let upright: Vec<(f64, f64)> = (0..7).map(|s| stub_point(&part, s)).collect();
        part.flip = true;
        let mirrored: Vec<(f64, f64)> = (0..7).map(|s| stub_point(&part, s)).collect();
        part.flip = false;
        part.rot = 180;
        let turned: Vec<(f64, f64)> = (0..7).map(|s| stub_point(&part, s)).collect();

        let width = PartKind::Seven.width();
        assert!(
            mirrored.iter().all(|p| (p.0 - 200.0).abs() < 0.01),
            "mirrored stubs sit on the left edge: {mirrored:?}",
        );
        assert!(
            upright.iter().all(|p| (p.0 - (200.0 + width)).abs() < 0.01),
            "upright stubs sit on the right edge: {upright:?}",
        );
        let order: Vec<f64> = mirrored.iter().map(|p| p.1).collect();
        let mut sorted = order.clone();
        sorted.sort_by(f64::total_cmp);
        assert_eq!(order, sorted, "mirroring keeps slot order top to bottom");

        let turned_order: Vec<f64> = turned.iter().map(|p| p.1).collect();
        let mut turned_sorted = turned_order.clone();
        turned_sorted.sort_by(f64::total_cmp);
        assert_ne!(
            turned_order, turned_sorted,
            "rotating by 180 reverses them — the case mirroring exists for",
        );
    }

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
            flip: false,
        }
    }

    /// ESP32's GPIO set, as the catalogue carries it.
    fn esp32_gpio() -> Vec<u32> {
        vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            25, 26, 27, 32, 33, 34, 35, 36, 37, 38, 39,
        ]
    }

    #[test]
    fn the_esp32_keeps_its_real_devkit_header() {
        let rows = kit_rows("esp32", &esp32_gpio());
        assert_eq!(rows.len(), 30);
        assert_eq!(row_of_gpio(&rows, 26), Some(8));
        assert_eq!(row_of_gpio(&rows, 23), Some(29));
        // RX and TX carry GPIO numbers; EN, GND, VIN and 3V3 carry none.
        assert_eq!(row_of_gpio(&rows, 3), Some(26));
        for name in ["EN", "GND", "VIN", "3V3"] {
            assert!(
                rows.iter().all(|(n, gpio)| n != name || gpio.is_none()),
                "{name} must not offer a GPIO",
            );
        }
        assert_eq!(row_of_gpio(&rows, UNWIRED), None);
    }

    /// The bug this replaced: every board was drawn as the 30-pin ESP32
    /// devkit, so an ESP32-C3 showed GPIO36, 39, 34 and 35 — none of which
    /// the part has — and a wire could be dropped on one.
    #[test]
    fn another_part_is_drawn_with_its_own_pins_and_no_others() {
        let c3: Vec<u32> = (0..=21).collect();
        let rows = kit_rows("esp32c3", &c3);

        for absent in [26u8, 32, 33, 34, 35, 36, 39] {
            assert_eq!(
                row_of_gpio(&rows, absent),
                None,
                "GPIO{absent} is not on a C3 and must have no row to land on",
            );
        }
        for present in [0u8, 9, 21] {
            assert!(row_of_gpio(&rows, present).is_some(), "GPIO{present} is");
        }
        // Rails at the top of each side, pins between them.
        assert_eq!(rows.len(), c3.len() + 4, "22 pins and four rails");
        assert!(rows.iter().filter(|(_, gpio)| gpio.is_none()).count() == 4);
    }

    /// A part rusty has no pin list for is drawn with rails and nothing else,
    /// rather than with somebody else's pins.
    #[test]
    fn an_unknown_part_offers_no_pins_at_all() {
        let rows = kit_rows("mystery", &[]);
        assert!(rows.iter().all(|(_, gpio)| gpio.is_none()));
        assert_eq!(row_of_gpio(&rows, 0), None);
    }

    #[test]
    fn a_turned_part_moves_its_stub_with_it() {
        let mut part = led(100.0, 100.0, 26);
        let upright = stub_point(&part, 0);
        // Width 112, height 28 → centre (156, 114); the stub sits on the
        // grid at (212, 116), and a quarter turn swings it to the bottom.
        assert_eq!(upright, (212.0, 116.0));

        part.rot = 90;
        let turned = stub_point(&part, 0);
        assert_eq!(turned, (154.0, 170.0));

        part.rot = 180;
        assert_eq!(stub_point(&part, 0), (100.0, 112.0));

        part.rot = 360;
        assert_eq!(stub_point(&part, 0), upright, "a full turn is no turn");
    }

    #[test]
    fn a_wire_runs_only_in_right_angles_and_ends_on_its_pin() {
        let part = led(60.0, 40.0, 26);
        let kit = (460.0, 40.0);
        let path = wire_path(&part, 0, kit, &kit_rows("esp32", &esp32_gpio()))
            .expect("a wired pin has a path");

        assert_eq!(path[0], stub_point(&part, 0), "starts at the stub");
        assert_eq!(
            *path.last().unwrap(),
            {
                let rows = kit_rows("esp32", &esp32_gpio());
                row_point(kit, rows.len(), row_of_gpio(&rows, 26).unwrap())
            },
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
        assert!(wire_path(&part, 0, (460.0, 40.0), &kit_rows("esp32", &esp32_gpio())).is_none());
        // A pin the chip does not have is refused rather than drawn to
        // nowhere — this is how a board file from another chip fails.
        let alien = led(60.0, 40.0, 99);
        assert!(wire_path(&alien, 0, (460.0, 40.0), &kit_rows("esp32", &esp32_gpio())).is_none());
    }

    #[test]
    fn user_bends_survive_the_orthogonal_pass() {
        let mut part = led(60.0, 40.0, 26);
        part.waypoints[0] = vec![(200.0, 54.0), (200.0, 168.0)];
        let path =
            wire_path(&part, 0, (460.0, 40.0), &kit_rows("esp32", &esp32_gpio())).expect("path");
        assert!(path.contains(&(200.0, 54.0)));
        assert!(path.contains(&(200.0, 168.0)));
        // Idempotent: a path already square gains no extra corners.
        assert_eq!(orthogonalize(path.clone()), path);
    }

    #[test]
    fn only_pin_rows_accept_a_dropped_wire() {
        let kit = (460.0, 40.0);
        let rows = kit_rows("esp32", &esp32_gpio());
        let n = rows.len();
        let row = row_of_gpio(&rows, 26).expect("gpio 26");
        let (px, py) = row_point(kit, n, row);
        assert_eq!(row_under(kit, n, (px, py)), Some(row));
        // A few pixels off still lands, because a pin is a small target.
        assert_eq!(row_under(kit, n, (px + 6.0, py + 3.0)), Some(row));
        // The middle of the board is not a pin.
        assert_eq!(row_under(kit, n, (kit.0 + 75.0, py)), None);
        // Neither is empty sheet.
        assert_eq!(row_under(kit, n, (kit.0 - 200.0, py)), None);
    }

    /// Every field that can be non-default *is* non-default here. The old
    /// fixture set `rot` and left `flip` alone, and a `flip` that never
    /// crossed the wire looked exactly like one that did.
    #[test]
    fn parts_survive_the_round_trip_through_the_wire_model() {
        let mut part = led(60.0, 40.0, 26);
        part.rot = 90;
        part.flip = true;
        part.waypoints[0] = vec![(200.0, 54.0)];
        let board = board_of("esp32", (460.0, 40.0), &[part.clone()]);

        assert_eq!(board.leds.len(), 1);
        assert_eq!(board.leds[0].place.rot, 90);
        assert!(board.leds[0].place.flip, "a mirrored part reaches the wire");
        assert_eq!(board.leds[0].place.routes, vec![vec![(200.0, 54.0)]]);
        assert_eq!(board.kit_x, Some(460.0));

        let back = parts_of(&board);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], part);
    }

    #[test]
    fn aligned_segments_merge_into_one() {
        // Two horizontal runs at the same height, joined by a redundant
        // bend: the bend must go, leaving one segment.
        let route = vec![(0.0, 10.0), (40.0, 10.0), (80.0, 10.0), (80.0, 50.0)];
        assert_eq!(
            simplify_route(route),
            vec![(0.0, 10.0), (80.0, 10.0), (80.0, 50.0)],
        );

        // A zero-length jog — the residue a drag leaves — vanishes whole.
        let jog = vec![
            (0.0, 10.0),
            (40.0, 10.0),
            (40.0, 10.0),
            (90.0, 10.0),
            (90.0, 40.0),
        ];
        assert_eq!(
            simplify_route(jog),
            vec![(0.0, 10.0), (90.0, 10.0), (90.0, 40.0)],
        );

        // Genuine corners survive untouched.
        let l = vec![(0.0, 0.0), (50.0, 0.0), (50.0, 50.0)];
        assert_eq!(simplify_route(l.clone()), l);
    }

    #[test]
    fn a_display_wires_its_two_i2c_pins() {
        let display = EditPart {
            kind: PartKind::Display,
            pins: [21, 22, 0, 0, 0, 0, 0],
            label: "DISPLAY".to_string(),
            x: 100.0,
            y: 100.0,
            waypoints: Default::default(),
            rot: 0,
            flip: false,
        };
        assert_eq!(display.kind.wires(), 2);
        assert!(
            wire_path(
                &display,
                0,
                (460.0, 40.0),
                &kit_rows("esp32", &esp32_gpio())
            )
            .is_some(),
            "sda routes"
        );
        assert!(
            wire_path(
                &display,
                1,
                (460.0, 40.0),
                &kit_rows("esp32", &esp32_gpio())
            )
            .is_some(),
            "scl routes"
        );
    }

    #[test]
    fn the_snap_grid_rounds_both_ways() {
        assert_eq!(snap_to(0.0, SNAP), 0.0);
        assert_eq!(snap_to(3.0, SNAP), 0.0);
        assert_eq!(snap_to(5.0, SNAP), 8.0);
        assert_eq!(snap_to(-3.0, SNAP), -0.0);
        assert_eq!(snap_to(-5.0, SNAP), -8.0);
        assert_eq!(snap_to(13.0, 4.0), 12.0);
        assert_eq!(snap_to(13.4, 1.0), 13.0);
        assert_eq!(snap_to(23.0, 16.0), 16.0);
    }

    /// The screenshot bug: a part dragged far below its planted bend grew a
    /// wall of wire back up to the old height. The first bend must slide
    /// along the first segment's own axis, and the route afterwards must
    /// never double back on itself.
    #[test]
    fn dragging_a_part_slides_the_first_bend_not_the_history() {
        let mut part = led(96.0, 96.0, 26);
        let stub = stub_point(&part, 0);
        // A user-planted bend, dead level with the stub: a horizontal first
        // leg, exactly the screenshot's shape.
        part.waypoints[0] = vec![(stub.0 + 160.0, stub.1)];
        let axis = first_leg_axis(stub, part.waypoints[0][0]);
        assert_eq!(axis, Some(true), "level with the stub means horizontal");

        // Drag the part a long way down.
        part.y += 160.0;
        let moved_stub = stub_point(&part, 0);
        follow_first_bend(moved_stub, axis, &mut part.waypoints[0][0]);
        assert_eq!(
            part.waypoints[0][0].1, moved_stub.1,
            "a horizontal first leg follows the stub's height",
        );

        // And the rendered route must have no U-turn: no two consecutive
        // segments on the same axis in opposite directions.
        let route =
            wire_path(&part, 0, (560.0, 96.0), &kit_rows("esp32", &esp32_gpio())).expect("wired");
        for window in route.windows(3) {
            let (a, b, c) = (window[0], window[1], window[2]);
            let vertical = (a.0 - b.0).abs() < 0.01 && (b.0 - c.0).abs() < 0.01;
            let horizontal = (a.1 - b.1).abs() < 0.01 && (b.1 - c.1).abs() < 0.01;
            let doubles_back = (vertical && (b.1 - a.1) * (c.1 - b.1) < 0.0)
                || (horizontal && (b.0 - a.0) * (c.0 - b.0) < 0.0);
            assert!(!doubles_back, "route doubles back: {a:?} {b:?} {c:?}");
        }
    }

    #[test]
    fn every_anchor_lies_on_the_base_grid() {
        // The root cause of "cannot align, ever": anchors 2px off the grid.
        // Pin rows and stubs must land on multiples of the base step when
        // the part and kit themselves are snapped.
        let part = led(96.0, 96.0, 26);
        let (sx, sy) = stub_point(&part, 0);
        assert_eq!(sx % SNAP, 0.0, "stub x on grid");
        assert_eq!(sy % SNAP, 0.0, "stub y on grid");

        // Every layout, not just the devkit: an odd pin count puts one more
        // row on the left, and a half-step there would put every anchor off
        // the grid on that side only.
        let kit = (456.0, 40.0);
        for gpio in [esp32_gpio(), (0..=21).collect(), (0..=20).collect()] {
            let rows = kit_rows("other", &gpio);
            for row in 0..rows.len() {
                let (_, py) = row_point(kit, rows.len(), row);
                assert_eq!(py % SNAP, 0.0, "row {row} y on grid");
            }
        }
    }
}
