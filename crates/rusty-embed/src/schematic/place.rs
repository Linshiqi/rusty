//! Where a placed symbol's pins are, in KiCad's own schematic space.
//!
//! Stage 0 of `docs/kicad.md`, and the whole of it: the sheet's own model
//! does not change, so what the interoperability needs from geometry is one
//! function — the coordinate a pin of a placed instance sits at, in the
//! space the file is written in.
//!
//! **Two spaces, and the flip between them.** A symbol library
//! (`.kicad_sym`) is millimetres with y *up*; a schematic (`.kicad_sch`) is
//! millimetres with y *down*. So a pin at `(lx, ly)` in the library becomes
//! `(lx, -ly)` before anything else happens to it. Confirmed against a real
//! file rather than remembered: `power:GND`'s graphic runs to `y = -2.54` in
//! the library and is drawn *below* its connection point on the sheet, and
//! `power:VCC`'s runs to `+2.54` and is drawn above.
//!
//! **Two things had to be recovered rather than read**, and each is one
//! boolean that silently swaps the pins of a part that is not symmetric — a
//! diode reversed, a transistor's collector and emitter exchanged, and
//! nothing on screen to say so. Which way the angle turns is [`ROTATION`];
//! whether the mirror comes before or after the turn is [`Mirror`]. Both
//! were settled by drawing the case in KiCad and looking at it, because
//! both are invisible in the file's own geometry: a symmetric part lands on
//! the same points either way and differs only in which pin is which.

use crate::model::{Pin, Symbol};

pub use crate::model::MM_PX;

/// KiCad's angle turns **negative** in the sheet's own frame, after the y
/// flip: a pin at library `(lx, ly)` on an instance placed at angle `a`
/// lands at `R(-a) · (lx, -ly)`.
///
/// One boolean, and it cannot be guessed: get it wrong and every rotated
/// part that is not point-symmetric has its pins swapped — a diode
/// reversed, an IC's netlist scrambled — with nothing on screen to say so.
///
/// **What settled it** was a `Device:LED` turned 90° in `my_flight.kicad_
/// sch`, between a `VCC` above it and a `GND` below, and *what KiCad draws*:
/// the cathode bar is at the bottom, on the ground side. The lamp's pins are
/// at `(-3.81, 0)` and `(3.81, 0)`, so pin 1 — `K` — has to land at the
/// lower wire end, `y = 55.88` against an origin of `52.07`. `R(-90)`
/// answers `(0, +3.81)`, which is that point; `R(+90)` answers `(0, -3.81)`,
/// which is the other wire.
///
/// **What did not settle it, and looked as though it had**, is worth
/// keeping: the autoplaced `Reference` field. `Device:LED`'s sits at
/// `(0, 2.54)` in the library — straight up, nothing else — and KiCad wrote
/// the instance's at `x = 116.84` against an origin of `113.03`, the *+x*
/// side, which `R(+90)` predicts and `R(-90)` does not. A resistor on a
/// second board agreed. Both were wrong, because **KiCad autoplaces field
/// text on whichever side reads well and does not carry it through the
/// symbol's transform** — the tell was a third instance, at 270°, whose
/// field implied the opposite sign from the 90° ones on the same sheet. A
/// witness that contradicts itself is not a witness.
///
/// Pin coordinates alone cannot answer this either: on the only large board
/// available every rotated part was a one-pin power flag or a
/// point-symmetric resistor, and for those both candidates produce the same
/// pair of points and differ only in which pin is which — exactly the case
/// that matters and exactly the one the geometry is silent about. It took a
/// drawing of a diode.
const ROTATION: f64 = -1.0;

/// How a placed instance is flipped, in KiCad's spelling.
///
/// **A mirror acts on the screen, after the turn**, and that had to be
/// measured too — it is right-looking either way at 0° and 180°, and wrong
/// at every quarter turn. The witness is three `Simulation_SPICE:NPN`
/// transistors, whose pins are asymmetric in *both* axes (`C` at
/// `(2.54, 5.08)`, `B` at `(-5.08, 0)`, `E` at `(2.54, -5.08)`), which is
/// what a two-pin part could never be:
///
/// - `(at … 0) (mirror x)` draws with `E` up, `C` down and `B` still left,
///   so `mirror x` negates y and leaves x alone — top to bottom.
/// - `(at … 90) (mirror x)` draws with **`B` up, `C` left, `E` right**.
///   Turning first and mirroring after gives exactly that; mirroring first
///   puts `B` down, `C` right and `E` left — all three wrong, which is the
///   useful kind of wrong: a reader that had it backwards would swap a
///   transistor's collector and emitter on every quarter-turned part and
///   nothing on screen would say so.
///
/// `(mirror y)` was never seen in a file, and there is a reason: mirroring
/// left to right is the same orientation as `mirror x` with 180° added, so
/// KiCad has no need to write it — asked for a left–right flip it stored
/// `(at … 180)` with no mirror at all. It is implemented by the rule the
/// other one established (negate x, after the turn) rather than by a
/// measurement of its own, and that is what this paragraph is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mirror {
    #[default]
    None,
    /// `(mirror x)` — flipped top to bottom, in screen space.
    X,
    /// `(mirror y)` — flipped left to right. Never observed; see above.
    Y,
}

impl Mirror {
    /// From the atom KiCad writes, or `None` for anything else.
    pub fn read(text: &str) -> Mirror {
        match text {
            "x" => Mirror::X,
            "y" => Mirror::Y,
            _ => Mirror::None,
        }
    }
}

/// Where a placed symbol sits: the instance's own anchor, its angle in
/// degrees, and its flip.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Placement {
    pub at: (f64, f64),
    pub angle: f64,
    pub mirror: Mirror,
}

impl Placement {
    /// A library point through the placement, into sheet millimetres.
    ///
    /// The order is KiCad's: flip the library's y, **turn, then mirror**,
    /// then translate. The mirror is last because it acts on the screen and
    /// not on the symbol's own frame — see [`Mirror`], where the drawing
    /// that says so is written down. This file had it the other way round
    /// first, which is right at 0° and 180° and wrong at every quarter
    /// turn.
    pub fn point(&self, local: (f64, f64)) -> (f64, f64) {
        let (x, y) = (local.0, -local.1);
        let a = (self.angle * ROTATION).to_radians();
        let (sin, cos) = a.sin_cos();
        let (x, y) = (x * cos - y * sin, x * sin + y * cos);
        let (x, y) = match self.mirror {
            Mirror::None => (x, y),
            Mirror::X => (x, -y),
            Mirror::Y => (-x, y),
        };
        (self.at.0 + x, self.at.1 + y)
    }

    /// Where a pin's wire attaches — KiCad's `at` *is* the connection
    /// point, so this is the pin's own coordinate through the placement and
    /// never the far end of its lead.
    pub fn pin(&self, pin: &Pin) -> (f64, f64) {
        self.point(pin.at)
    }

    /// Every pin of a placed symbol, with the number a wire names it by.
    pub fn pins<'a>(&'a self, symbol: &'a Symbol) -> impl Iterator<Item = (&'a str, (f64, f64))> {
        symbol
            .pins
            .iter()
            .filter(|pin| !pin.hidden)
            .map(move |pin| (pin.number.as_str(), self.pin(pin)))
    }
}

/// Two sheet points that KiCad would call the same point.
///
/// Everything in a schematic is on a grid measured in mils and written as
/// millimetres, so the numbers are exact in the file and inexact in binary;
/// a tenth of a mil is far below anything KiCad will place and far above
/// the error of parsing `113.03`.
pub fn same_point(a: (f64, f64), b: (f64, f64)) -> bool {
    const CLOSE: f64 = 0.0025;
    (a.0 - b.0).abs() < CLOSE && (a.1 - b.1).abs() < CLOSE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PinKind;

    fn pin(number: &str, name: &str, at: (f64, f64), angle: u16) -> Pin {
        Pin {
            number: number.into(),
            name: name.into(),
            kind: PinKind::Passive,
            at,
            length: 2.54,
            angle,
            hidden: false,
        }
    }

    /// The numbers are `my_flight.kicad_sch`, written in KiCad 10 and read
    /// off the file: a `Device:LED` at `(113.03, 52.07)` turned 90°, with a
    /// `power:VCC` above it at `(113.03, 45.72)` and a `power:GND` below at
    /// `(113.03, 60.96)`, joined by two wires whose free ends are at
    /// `y = 48.26` and `y = 55.88`.
    ///
    /// **This test is the rotation constant**, and what it asserts is not
    /// that the pins land on the wires — they do that whichever way the
    /// angle turns, which is why the geometry alone was silent — but *which
    /// pin lands where*. KiCad draws that lamp with its cathode bar at the
    /// bottom, on the ground side. Anything that flips [`ROTATION`] fails
    /// here, and fails saying so.
    #[test]
    fn a_turned_lamp_puts_its_cathode_where_kicad_draws_it() {
        let led = Placement {
            at: (113.03, 52.07),
            angle: 90.0,
            mirror: Mirror::None,
        };
        let cathode = pin("1", "K", (-3.81, 0.0), 0);
        let anode = pin("2", "A", (3.81, 0.0), 180);

        assert!(
            same_point(led.pin(&cathode), (113.03, 55.88)),
            "the cathode is the ground end, as the drawing shows: {:?}",
            led.pin(&cathode)
        );
        assert!(
            same_point(led.pin(&anode), (113.03, 48.26)),
            "and the anode is the supply end: {:?}",
            led.pin(&anode)
        );
    }

    /// The trap that cost an inference: an autoplaced field is not a
    /// witness to the transform. KiCad put this lamp's `Reference` on the
    /// +x side, which is where `R(+90)` would send `(0, 2.54)` and not
    /// where `R(-90)` does — and `R(-90)` is right. Pinned so that nobody
    /// re-derives the sign from a field and gets the opposite answer.
    #[test]
    fn an_autoplaced_field_does_not_follow_the_symbols_turn() {
        let led = Placement {
            at: (113.03, 52.07),
            angle: 90.0,
            mirror: Mirror::None,
        };
        let rigid = led.point((0.0, 2.54));
        assert!(
            rigid.0 < led.at.0,
            "the rigid transform sends the field to -x: {rigid:?}"
        );
        // KiCad wrote 116.84, on the other side.
        assert!(116.84 > led.at.0);
    }

    /// The y flip on its own, which is not in doubt: a ground symbol's
    /// graphic runs to negative y in the library and is drawn *below* its
    /// connection point on the sheet.
    #[test]
    fn the_library_is_y_up_and_the_sheet_is_y_down() {
        let ground = Placement {
            at: (113.03, 60.96),
            ..Placement::default()
        };
        let tip = ground.point((0.0, -2.54));
        assert!(same_point(tip, (113.03, 63.5)), "{tip:?}");

        let supply = Placement {
            at: (113.03, 45.72),
            ..Placement::default()
        };
        assert!(same_point(supply.point((0.0, 2.54)), (113.03, 43.18)));
    }

    /// Three `Simulation_SPICE:NPN` transistors drawn in KiCad 10, and what
    /// KiCad draws. A transistor because its pins are asymmetric in *both*
    /// axes — `C` up-right, `B` left, `E` down-right — which is the only
    /// shape that can answer this: a two-pin part mirrors onto itself.
    ///
    /// **The turn comes before the mirror**, and this test is that fact.
    /// Mirroring first is indistinguishable at 0° and 180° and wrong at
    /// every quarter turn, so `Q3` is the one that matters and the other two
    /// are the controls that say the rest of the pipeline is right.
    #[test]
    fn a_mirror_acts_on_the_screen_after_the_turn() {
        let collector = pin("1", "C", (2.54, 5.08), 270);
        let base = pin("2", "B", (-5.08, 0.0), 0);
        let emitter = pin("3", "E", (2.54, -5.08), 90);

        // Q1 — turned 180°, not mirrored. The control: an asymmetric part
        // through the flip and a turn.
        let q1 = Placement {
            at: (134.62, 57.15),
            angle: 180.0,
            mirror: Mirror::None,
        };
        assert!(
            same_point(q1.pin(&emitter), (132.08, 52.07)),
            "E is drawn above"
        );
        assert!(same_point(q1.pin(&collector), (132.08, 62.23)), "C below");
        assert!(same_point(q1.pin(&base), (139.7, 57.15)), "B to the right");

        // Q2 — mirrored, not turned. `mirror x` negates y: the collector
        // and emitter swap and the base does not move.
        let q2 = Placement {
            at: (152.4, 67.31),
            angle: 0.0,
            mirror: Mirror::X,
        };
        assert!(same_point(q2.pin(&emitter), (154.94, 62.23)), "E above");
        assert!(same_point(q2.pin(&collector), (154.94, 72.39)), "C below");
        assert!(same_point(q2.pin(&base), (147.32, 67.31)), "B still left");

        // Q3 — mirrored *and* turned, which is the whole question. KiCad
        // draws this one with the base up, the collector left and the
        // emitter right. Mirroring before the turn answers the opposite for
        // all three.
        let q3 = Placement {
            at: (133.35, 76.2),
            angle: 90.0,
            mirror: Mirror::X,
        };
        assert!(
            same_point(q3.pin(&base), (133.35, 71.12)),
            "the base is drawn above: {:?}",
            q3.pin(&base)
        );
        assert!(
            same_point(q3.pin(&collector), (128.27, 78.74)),
            "the collector to the left: {:?}",
            q3.pin(&collector)
        );
        assert!(
            same_point(q3.pin(&emitter), (138.43, 78.74)),
            "and the emitter to the right: {:?}",
            q3.pin(&emitter)
        );
    }

    #[test]
    fn hidden_pins_take_no_wires() {
        let mut symbol = Symbol {
            library: "Device".into(),
            name: "LED".into(),
            reference: "D".into(),
            value: "LED".into(),
            description: None,
            pins: vec![
                pin("1", "K", (-3.81, 0.0), 0),
                pin("2", "A", (3.81, 0.0), 180),
            ],
            graphics: Vec::new(),
        };
        symbol.pins[1].hidden = true;
        let placed = Placement::default();
        let seen: Vec<&str> = placed.pins(&symbol).map(|(number, _)| number).collect();
        assert_eq!(seen, vec!["1"]);
    }
}
