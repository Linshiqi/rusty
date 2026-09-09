//! The sheet's circuit, driven by the firmware that is running.
//!
//! Stage 5 of `docs/kicad.md`, and the reason for every stage before it.
//! KiCad and ngspice can simulate a circuit and have no firmware; rusty
//! runs the firmware — really runs it, on the chip's own instruction set,
//! with a channel carrying its pins. Neither half alone answers the
//! question an embedded engineer actually has, which is whether *this*
//! firmware works on *this* circuit.
//!
//! **The coupling is event-driven, and that is what makes it tractable.**
//! The difficulty was never electrical: QEMU runs at roughly wall-clock and
//! a transient steps in microseconds, so advancing the circuit a microsecond
//! per microsecond of guest time would be a million solves a second, nearly
//! all of them computing that nothing changed. The circuit only has to be
//! advanced when something *asks* — a pin the firmware drove, reported with
//! the systimer's own microseconds (`[rusty:gpio@1234]`), or a converter it
//! read. Between those it is only relaxing.
//!
//! Which is where backward Euler earns its place a second time. Over a long
//! idle gap one big step is not merely stable — it lands *on* the steady
//! state, because the method is implicit and the steady state is its fixed
//! point. So a coarse step across a quiet stretch is not an approximation
//! that degrades; it is the answer. Accuracy only matters while something
//! is moving, and that is exactly where the events are.

use std::collections::{BTreeMap, HashSet};

use crate::circuit::{self, Bridged, Unstated};
use crate::model::{PinRef, Sheet};
use crate::nets::Row;
use crate::solve::{Element, Transient, Trouble};

/// How finely the circuit is resolved while something is moving.
///
/// A stated parameter rather than one derived from the circuit's own time
/// constants: working those out means an eigenvalue problem, and guessing
/// them from the nearest resistor is the kind of heuristic that is right
/// until it is quietly wrong. The panel picks this and can show it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pace {
    /// The step taken while resolving an interval, in seconds.
    pub finest: f64,
    /// How many steps one advance may take. A gap longer than
    /// `finest × steps` is crossed in coarser steps, which costs accuracy
    /// only where nothing is happening — see the note above about where
    /// backward Euler lands.
    pub steps: usize,
}

impl Default for Pace {
    fn default() -> Self {
        // Ten microseconds resolves a GPIO edge against the RC of anything
        // a person puts on a pin, and a thousand of them is ten
        // milliseconds — longer than the gap between two events in any
        // firmware that is doing something.
        Pace {
            finest: 10e-6,
            steps: 1_000,
        }
    }
}

/// The converter's own resolution when the sheet does not say.
///
/// Twelve bits, which is the ESP32 family's SAR converter. It has a default
/// where the full-scale voltage does not, and the difference is the point:
/// the resolution is a fact about the chip rusty already knows, and the
/// voltage is a fact about how the firmware configured it, which only the
/// firmware knows.
const DEFAULT_COUNTS: u16 = 4095;

/// What a converter turns volts into, as the sheet states it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    /// The voltage that reads full scale. Not the rail — see
    /// [`Live::counts_at`].
    pub full_volts: f64,
    pub max: u16,
}

/// A circuit being driven by a running firmware.
#[derive(Debug, Clone)]
pub struct Live {
    sheet: Sheet,
    rows: Vec<Row>,
    pressed: HashSet<String>,
    /// What the firmware has told us about each GPIO.
    levels: BTreeMap<u8, bool>,
    bridged: Bridged,
    run: Transient,
    pace: Pace,
    /// The guest instant the circuit has been advanced to, in the
    /// systimer's own microseconds.
    at: u64,
    /// The counts last said for each pin, so the same number is not sent
    /// twice — the change suppression every other report on this channel
    /// has.
    said: BTreeMap<u8, u16>,
}

impl Live {
    /// Start from rest — the power has just come on — with whatever the
    /// firmware has already said about its pins.
    pub fn at_rest(
        sheet: Sheet,
        rows: Vec<Row>,
        levels: BTreeMap<u8, bool>,
        pace: Pace,
    ) -> Result<Self, Unstated> {
        let pressed = HashSet::new();
        let bridged = circuit::of(&sheet, &rows, &pressed, &levels)?;
        let run = Transient::at_rest(bridged.circuit.clone());
        Ok(Live {
            sheet,
            rows,
            pressed,
            levels,
            bridged,
            run,
            pace,
            at: 0,
            said: BTreeMap::new(),
        })
    }

    /// Advance the circuit to a guest instant.
    ///
    /// Going backwards is not an error and not a step: the systimer can
    /// repeat a microsecond across two reports, and refusing that would
    /// turn an ordinary log into a failure.
    pub fn advance_to(&mut self, micros: u64) -> Result<(), Trouble> {
        let Some(gap) = micros.checked_sub(self.at).filter(|gap| *gap > 0) else {
            return Ok(());
        };
        let seconds = gap as f64 * 1e-6;
        let wanted = (seconds / self.pace.finest).ceil() as usize;
        let steps = wanted.clamp(1, self.pace.steps);
        let each = seconds / steps as f64;
        for _ in 0..steps {
            self.run.step(each)?;
        }
        self.at = micros;
        Ok(())
    }

    /// Advance by a length of time nobody reported, and answer with what
    /// moved.
    ///
    /// **The event-driven advance is not enough on its own, and a real run
    /// is what proved it.** The emulator reports a conversion only when the
    /// value *changed*, and the value only changes when the host sends a
    /// new one — so after a pin moves, nothing is said, nothing advances,
    /// and the circuit sits at the instant of the edge until the next one.
    /// The firmware's reading then steps from nothing to full scale in a
    /// single conversion: a host echoing a pin level wearing a circuit's
    /// clothes, which is exactly what the gate exists to catch and exactly
    /// what it caught. The headless tests could not see it, because a test
    /// that feeds a dense stream of lines never stops advancing time.
    ///
    /// So a caller with a clock follows the circuit forward while it is
    /// settling. This only fills the silence: the guest's own timestamps
    /// still correct it whenever one arrives, and [`Live::advance_to`]
    /// treats going backwards as a no-op, so an over-eager clock costs
    /// nothing but a little resolution.
    pub fn advance_by(&mut self, seconds: f64) -> Result<Vec<(u8, u16)>, Trouble> {
        if !seconds.is_finite() || seconds <= 0.0 {
            return Ok(Vec::new());
        }
        let ahead = self.at.saturating_add((seconds * 1e6).round() as u64);
        self.advance_to(ahead)?;
        Ok(self.changed_counts())
    }

    /// Is anything still moving?
    ///
    /// What a caller uses to decide whether following the circuit forward
    /// is worth the solves: a settled circuit answers the same counts for
    /// ever, and stepping it is arithmetic nobody reads.
    pub fn settling(&self) -> bool {
        self.bridged.circuit.elements.iter().any(|element| {
            matches!(
                element,
                Element::Capacitor { .. } | Element::Inductor { .. }
            )
        })
    }

    /// What the firmware reported: a pin it is driving, at the instant it
    /// said so.
    ///
    /// The circuit is advanced to that instant *first*, so the edge lands
    /// where the firmware put it rather than at the end of whatever the
    /// last step happened to be — which is the difference between a pulse
    /// width the panel can be believed about and one it rounded.
    pub fn drove(&mut self, gpio: u8, high: bool, micros: u64) -> Result<(), Trouble> {
        self.advance_to(micros)?;
        let known = self.levels.insert(gpio, high);
        if known.is_none() {
            // A pin nobody had reported was not a source, because a pin
            // rusty has heard nothing about is not one it may claim to
            // know the voltage of. Now that it is, the circuit has one more
            // element in it — so it is rebuilt, and the capacitors keep
            // what they were holding.
            self.rebuild();
            return Ok(());
        }
        if let Some(row) = self.rows.iter().find(|row| row.gpio == Some(gpio))
            && let Some(element) = self.bridged.element_of.get(&row.name).copied()
        {
            let volts = if high { self.rail() } else { 0.0 };
            self.run.drive(element, volts);
        }
        Ok(())
    }

    /// Read one line off the emulator's pin channel, and answer with what
    /// should go back down it.
    ///
    /// **The one place the two halves meet**, so the example that proves it
    /// and the app that ships it cannot drift about what the coupling
    /// means — the same rule that put the serial protocol's reading in one
    /// `absorb` after telemetry worked in the simulator and vanished on
    /// hardware.
    ///
    /// A `[rusty:gpio@…]` line is the firmware driving a pin, at the
    /// instant it drove it. A `[rusty:adc@…]` line is the firmware having
    /// *read* one, which is also a statement about time: the circuit is
    /// advanced to that instant, because a converter reads the present.
    ///
    /// What comes back is `(gpio, counts)` for every readable pin whose
    /// counts have moved — change-suppressed like every other report on
    /// this channel, because a firmware polling a sensor would otherwise
    /// put twenty kilobytes a second down the line the console shares.
    pub fn absorb(&mut self, line: &str) -> Result<Vec<(u8, u16)>, Trouble> {
        if let Some(report) = crate::protocol::parse_gpio_report(line) {
            let at = report.at_us.unwrap_or(self.at);
            for (pin, level) in report.pins {
                self.drove(pin, level, at)?;
            }
        } else if let Some(report) = crate::protocol::parse_adc_report(line) {
            if let Some(at) = report.at_us {
                self.advance_to(at)?;
            }
        } else {
            return Ok(Vec::new());
        }
        Ok(self.changed_counts())
    }

    /// Every readable pin whose counts have moved since it was last said.
    fn changed_counts(&mut self) -> Vec<(u8, u16)> {
        let mut out = Vec::new();
        let gpios: Vec<u8> = self.rows.iter().filter_map(|row| row.gpio).collect();
        for gpio in gpios {
            let Some(counts) = self.counts_at(gpio) else {
                continue;
            };
            if self.said.get(&gpio) != Some(&counts) {
                self.said.insert(gpio, counts);
                out.push((gpio, counts));
            }
        }
        out
    }

    /// A switch held or let go, which changes what conducts.
    pub fn pressing(&mut self, part: &str, down: bool) {
        let changed = if down {
            self.pressed.insert(part.to_string())
        } else {
            self.pressed.remove(part)
        };
        if changed {
            self.rebuild();
        }
    }

    /// What a pin is at, in volts, or nothing when the sheet gives it no
    /// node — a pin nothing is wired to has no voltage rather than zero.
    pub fn volts_at(&self, pin: &PinRef) -> Option<f64> {
        Some(self.run.volts_at(*self.bridged.node_of.get(pin)?))
    }

    /// The counts a converter on `gpio` would report.
    ///
    /// **The full scale is not the rail**, and that is the whole reason
    /// this returns an `Option`. An ESP32's SAR converter reads about 1.1 V
    /// at its default attenuation and about 3.1 V at 11 dB — neither is
    /// 3.3 — so turning a solved voltage into counts against the supply
    /// would be out by a factor of three and look entirely plausible. The
    /// sheet has to say, on a part sitting on that net, as `fullscale`; the
    /// resolution comes from the same part's `max` and is the converter's
    /// own, which is why it has a default and the voltage does not.
    ///
    /// This is the rule the whole simulator already runs on, arrived at
    /// from the other side: `A<pin>=<counts>` has always carried counts
    /// rather than volts because rusty did not know anybody's divider. It
    /// knows the divider now — it solved it — and it still does not know
    /// the converter, so it still refuses.
    pub fn counts_at(&self, gpio: u8) -> Option<u16> {
        let volts = self.run.volts_at(self.gpio_node(gpio)?);
        let scale = self.scale_for(gpio)?;
        let counts = (volts / scale.full_volts * f64::from(scale.max)).round();
        Some(counts.clamp(0.0, f64::from(scale.max)) as u16)
    }

    /// What the sheet says the converter on this pin turns volts into, or
    /// nothing when nobody has said.
    ///
    /// By GPIO and not by node: the node numbering is this module's own and
    /// changes whenever the circuit is rebuilt, so a caller outside could
    /// never have obtained one. Found by the review after the fact, which
    /// is what the review is for.
    pub fn scale_for(&self, gpio: u8) -> Option<Scale> {
        let node = self.gpio_node(gpio)?;
        self.sheet.parts.iter().find_map(|part| {
            let full_volts = part.prop::<f64>("fullscale").filter(|v| *v > 0.0)?;
            let symbol = self.sheet.symbol_of(&part.reference)?;
            let touches = symbol.pins.iter().any(|pin| {
                self.bridged
                    .node_of
                    .get(&PinRef::new(&part.reference, &pin.number))
                    == Some(&node)
            });
            touches.then_some(Scale {
                full_volts,
                max: part.prop::<u16>("max").unwrap_or(DEFAULT_COUNTS),
            })
        })
    }

    /// The node a GPIO row sits on, by name or by number.
    fn gpio_node(&self, gpio: u8) -> Option<usize> {
        let row = self.rows.iter().find(|row| row.gpio == Some(gpio))?;
        self.bridged
            .node_of
            .get(&PinRef::new(crate::model::KIT_REFERENCE, &row.name))
            .copied()
    }

    /// The instant the circuit has been advanced to.
    pub fn micros(&self) -> u64 {
        self.at
    }

    /// The rail a driven pin is driven to. Read off the sheet, never
    /// assumed: the devkit's own header names it.
    fn rail(&self) -> f64 {
        self.bridged
            .circuit
            .elements
            .iter()
            .filter_map(|e| match e {
                Element::Source { volts, .. } => Some(*volts),
                _ => None,
            })
            .fold(0.0f64, f64::max)
    }

    /// Rebuild the circuit and carry the energy across.
    ///
    /// The set of elements changes when a pin is reported for the first
    /// time or a switch closes, and a rebuilt circuit that started from
    /// rest would discharge every capacitor on the sheet — a glitch the
    /// firmware never caused, at the moment it first touched a pin. So the
    /// voltages are carried by *node*, and the new transient starts there.
    fn rebuild(&mut self) {
        let held: Vec<(PinRef, f64)> = self
            .bridged
            .node_of
            .iter()
            .filter_map(|(pin, node)| {
                Some((pin.clone(), self.run.now().volts.get(*node).copied()?))
            })
            .collect();
        let Ok(fresh) = circuit::of(&self.sheet, &self.rows, &self.pressed, &self.levels) else {
            // The sheet stopped saying enough only because a switch moved,
            // which cannot happen: what it says does not depend on that.
            // Keep what works rather than losing the run.
            return;
        };
        let mut volts = vec![0.0; fresh.circuit.nodes];
        for (pin, was) in held {
            if let Some(node) = fresh.node_of.get(&pin) {
                volts[*node] = was;
            }
        }
        self.bridged = fresh;
        self.run = Transient::carrying(self.bridged.circuit.clone(), &volts);
        // And what every known pin is driving, in the new numbering.
        let rail = self.rail();
        for (gpio, high) in &self.levels {
            if let Some(row) = self.rows.iter().find(|row| row.gpio == Some(*gpio))
                && let Some(element) = self.bridged.element_of.get(&row.name).copied()
            {
                self.run.drive(element, if *high { rail } else { 0.0 });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fill, Graphic, Instance, Pin, PinKind, Symbol, Wire};
    use crate::nets::kit_rows;

    fn symbol(library: &str, name: &str, reference: &str, pins: &[(&str, &str)]) -> Symbol {
        Symbol {
            library: library.into(),
            name: name.into(),
            reference: reference.into(),
            value: name.into(),
            description: None,
            pins: pins
                .iter()
                .enumerate()
                .map(|(i, (number, pin_name))| Pin {
                    number: (*number).into(),
                    name: (*pin_name).into(),
                    kind: PinKind::Passive,
                    at: (if i == 0 { -2.54 } else { 2.54 }, 0.0),
                    length: 2.54,
                    angle: if i == 0 { 0 } else { 180 },
                    hidden: false,
                })
                .collect(),
            graphics: vec![Graphic::Rectangle {
                start: (-1.0, 1.0),
                end: (1.0, -1.0),
                width: 0.0,
                fill: Fill::None,
            }],
        }
    }

    /// A GPIO through a resistor into a capacitor to ground: an RC on a
    /// pin, which is the circuit a firmware-in-the-loop run is about.
    /// R = 1k and C = 1µF, so the time constant is a millisecond.
    fn rc_on_a_pin() -> (Sheet, Vec<Row>) {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.symbols = vec![
            symbol("Device", "R", "R", &[("1", "~"), ("2", "~")]),
            symbol("Device", "C", "C", &[("1", "~"), ("2", "~")]),
            symbol("rusty", "GND", "#PWR", &[("1", "GND")]),
        ];
        let mut place = |reference: &str, id: &str, value: &str| {
            sheet.parts.push(Instance {
                reference: reference.into(),
                symbol: id.into(),
                value: value.into(),
                x: 0.0,
                y: 0.0,
                rot: 0,
                mirror: false,
                props: Default::default(),
            });
        };
        place("R1", "Device:R", "1k");
        place("C1", "Device:C", "1u");
        place("GND1", "rusty:GND", "GND");
        let mut wire = |from: &str, to: &str| {
            sheet.wires.push(Wire {
                from: PinRef::parse(from).unwrap(),
                to: PinRef::parse(to).unwrap(),
                bends: Vec::new(),
            });
        };
        wire("U1.GPIO2", "R1.1");
        wire("R1.2", "C1.1");
        wire("C1.2", "GND1.GND");
        let rows = kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5]);
        (sheet, rows)
    }

    fn started() -> Live {
        let (sheet, rows) = rc_on_a_pin();
        Live::at_rest(sheet, rows, BTreeMap::new(), Pace::default()).expect("the sheet says enough")
    }

    /// The whole of stage 5 in one test: the firmware drives a pin at an
    /// instant it names, and the circuit beyond it follows with the time
    /// constant the parts have.
    #[test]
    fn a_pin_the_firmware_drove_charges_the_circuit_beyond_it() {
        let mut live = started();
        let across = PinRef::new("C1", "1");

        // Nothing has been said about the pin yet, so nothing is driving.
        live.advance_to(1_000).expect("advanced");
        assert!(live.volts_at(&across).unwrap_or(0.0).abs() < 1e-9);

        // High at one millisecond of guest time.
        live.drove(2, true, 1_000).expect("drove");
        assert_eq!(live.micros(), 1_000);

        // One time constant later it should be at about 63% of the rail.
        live.advance_to(2_000).expect("advanced");
        let at_tau = live.volts_at(&across).expect("a voltage");
        let want = 3.3 * (1.0 - (-1.0f64).exp());
        assert!(
            (at_tau - want).abs() < 0.05 * want,
            "one time constant of a 1k and a 1uF: {at_tau} against {want}"
        );

        // And five more, near the rail.
        live.advance_to(7_000).expect("advanced");
        let settled = live.volts_at(&across).expect("a voltage");
        assert!(
            (settled - 3.3).abs() < 0.05,
            "and then near the rail: {settled}"
        );
    }

    /// A long quiet gap is crossed in coarse steps, and lands *on* the
    /// steady state rather than near it — the property that makes the
    /// event-driven coupling honest rather than merely fast.
    #[test]
    fn a_quiet_gap_lands_on_the_steady_state_however_coarsely_it_is_crossed() {
        let mut live = started();
        live.drove(2, true, 0).expect("drove");
        // Ten seconds of guest time, which at the stated pace is far more
        // than the step budget: the gap is crossed in a thousand coarse
        // steps and the answer is still the rail.
        live.advance_to(10_000_000).expect("advanced");
        let settled = live.volts_at(&PinRef::new("C1", "1")).expect("a voltage");
        assert!(
            (settled - 3.3).abs() < 1e-9,
            "an implicit method's fixed point is the steady state: {settled}"
        );
    }

    /// The first report of a pin adds a source, which rebuilds the circuit
    /// — and the capacitor must keep what it was holding. Losing it would
    /// be a discharge the firmware never caused, at the exact moment it
    /// first touched a pin.
    #[test]
    fn the_charge_survives_the_circuit_being_rebuilt() {
        let (sheet, rows) = rc_on_a_pin();
        // GPIO2 known from the start, so the source exists and charges.
        let mut live = Live::at_rest(
            sheet,
            rows,
            [(2u8, true)].into_iter().collect(),
            Pace::default(),
        )
        .expect("built");
        live.advance_to(5_000).expect("advanced");
        let before = live.volts_at(&PinRef::new("C1", "1")).expect("a voltage");
        assert!(before > 3.0, "charged: {before}");

        // A different pin is reported for the first time, which rebuilds.
        live.drove(3, false, 5_000).expect("drove");
        let after = live.volts_at(&PinRef::new("C1", "1")).expect("a voltage");
        assert!(
            (after - before).abs() < 1e-9,
            "the capacitor kept its charge across the rebuild: {before} then {after}"
        );
    }

    /// Time only goes forward, and a repeated timestamp is ordinary rather
    /// than an error: the systimer can report the same microsecond twice.
    #[test]
    fn a_repeated_instant_is_not_a_step_and_not_a_failure() {
        let mut live = started();
        live.drove(2, true, 1_000).expect("drove");
        live.advance_to(2_000).expect("advanced");
        let at_tau = live.volts_at(&PinRef::new("C1", "1")).expect("a voltage");

        live.advance_to(2_000).expect("the same instant again");
        live.advance_to(1_500).expect("and an earlier one");
        assert_eq!(live.micros(), 2_000);
        assert_eq!(
            live.volts_at(&PinRef::new("C1", "1")),
            Some(at_tau),
            "neither moved the circuit"
        );
    }

    /// A divider from the rail into an ADC pin — the circuit a knob or a
    /// battery monitor is — with the resistors chosen so the midpoint is a
    /// third of the rail.
    fn divider_into_a_pin() -> (Sheet, Vec<Row>) {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.symbols = vec![
            symbol("Device", "R", "R", &[("1", "~"), ("2", "~")]),
            symbol("rusty", "GND", "#PWR", &[("1", "GND")]),
        ];
        let mut place = |reference: &str, id: &str, value: &str| {
            sheet.parts.push(Instance {
                reference: reference.into(),
                symbol: id.into(),
                value: value.into(),
                x: 0.0,
                y: 0.0,
                rot: 0,
                mirror: false,
                props: Default::default(),
            });
        };
        place("R1", "Device:R", "20k");
        place("R2", "Device:R", "10k");
        place("GND1", "rusty:GND", "GND");
        let mut wire = |from: &str, to: &str| {
            sheet.wires.push(Wire {
                from: PinRef::parse(from).unwrap(),
                to: PinRef::parse(to).unwrap(),
                bends: Vec::new(),
            });
        };
        wire("U1.3V3", "R1.1");
        wire("R1.2", "R2.1");
        wire("R2.1", "U1.GPIO3");
        wire("R2.2", "GND1.GND");
        (sheet, kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5]))
    }

    /// Volts become counts only where the sheet said what full scale is,
    /// and the rail is not an answer to that question.
    #[test]
    fn counts_need_a_full_scale_the_sheet_stated() {
        let (sheet, rows) = divider_into_a_pin();
        let mut live = Live::at_rest(
            sheet.clone(),
            rows.clone(),
            BTreeMap::new(),
            Pace::default(),
        )
        .expect("built");
        live.advance_to(1_000).expect("advanced");

        let at_pin = live
            .volts_at(&PinRef::new("U1", "GPIO3"))
            .expect("the divider gives it a voltage");
        assert!((at_pin - 1.1).abs() < 1e-9, "a third of 3V3: {at_pin}");
        assert_eq!(
            live.counts_at(3),
            None,
            "and nobody has said what the converter reads full scale at"
        );

        // Said, on the part sitting on that net: 1.1 V, which is what an
        // ESP32's SAR reads at its default attenuation — and is a third of
        // the rail, so reading it against the rail instead would be out by
        // three and look perfectly plausible.
        let mut stated = sheet;
        if let Some(part) = stated.parts.iter_mut().find(|p| p.reference == "R2") {
            part.props.insert("fullscale".into(), "1.1".into());
        }
        let mut live =
            Live::at_rest(stated, rows, BTreeMap::new(), Pace::default()).expect("built");
        live.advance_to(1_000).expect("advanced");
        assert_eq!(
            live.counts_at(3),
            Some(DEFAULT_COUNTS),
            "1.1 V against a 1.1 V full scale is the top of the range"
        );
    }

    /// The arithmetic anybody would check: half of full scale is half the
    /// counts, and past it saturates rather than wrapping.
    #[test]
    fn a_solved_voltage_becomes_the_counts_the_scale_says() {
        let (mut sheet, rows) = divider_into_a_pin();
        if let Some(part) = sheet.parts.iter_mut().find(|p| p.reference == "R2") {
            // Full scale at twice the divider's midpoint, so it should read
            // exactly half, with a round thousand of counts to check by eye.
            part.props.insert("fullscale".into(), "2.2".into());
            part.props.insert("max".into(), "1000".into());
        }
        let mut live = Live::at_rest(
            sheet.clone(),
            rows.clone(),
            BTreeMap::new(),
            Pace::default(),
        )
        .expect("built");
        live.advance_to(1_000).expect("advanced");
        assert_eq!(live.counts_at(3), Some(500), "half of full scale");

        // And the scale the sheet stated, read back.
        let scale = live.scale_for(3).expect("stated");
        assert_eq!(scale.full_volts, 2.2);
        assert_eq!(scale.max, 1000);

        // A pin the sheet gives no node has no counts either.
        assert_eq!(live.counts_at(5), None);
    }

    /// A GPIO driving an RC whose midpoint another pin reads — the shape
    /// the firmware-in-the-loop gate boots, headless.
    fn driven_rc_read_by_a_pin() -> (Sheet, Vec<Row>) {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.symbols = vec![
            symbol("Device", "R", "R", &[("1", "~"), ("2", "~")]),
            symbol("Device", "C", "C", &[("1", "~"), ("2", "~")]),
            symbol("rusty", "GND", "#PWR", &[("1", "GND")]),
        ];
        let mut place = |reference: &str, id: &str, value: &str, scale: bool| {
            let mut props: std::collections::BTreeMap<String, String> = Default::default();
            if scale {
                // The converter's full scale, said on the part sitting on
                // the net it reads — the one thing the sheet has to state.
                props.insert("fullscale".into(), "3.3".into());
                props.insert("max".into(), "1000".into());
            }
            sheet.parts.push(Instance {
                reference: reference.into(),
                symbol: id.into(),
                value: value.into(),
                x: 0.0,
                y: 0.0,
                rot: 0,
                mirror: false,
                props,
            });
        };
        place("R1", "Device:R", "1k", false);
        place("C1", "Device:C", "1u", true);
        place("GND1", "rusty:GND", "GND", false);
        let mut wire = |from: &str, to: &str| {
            sheet.wires.push(Wire {
                from: PinRef::parse(from).unwrap(),
                to: PinRef::parse(to).unwrap(),
                bends: Vec::new(),
            });
        };
        // GPIO2 drives through the resistor; GPIO3 reads the capacitor.
        wire("U1.GPIO2", "R1.1");
        wire("R1.2", "C1.1");
        wire("C1.1", "U1.GPIO3");
        wire("C1.2", "GND1.GND");
        (sheet, kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5]))
    }

    /// **The coupling, end to end and headless.** The emulator's own lines
    /// go in and the counts that should go back come out, and what they
    /// have to show is that the reading *ramps* — because that is the whole
    /// claim. A converter fed straight from a pin level would jump from
    /// nothing to full scale in one report; one fed from a solved circuit
    /// climbs through the RC that is drawn.
    #[test]
    fn the_emulators_lines_go_in_and_the_converters_counts_come_back() {
        let (sheet, rows) = driven_rc_read_by_a_pin();
        let mut live = Live::at_rest(sheet, rows, BTreeMap::new(), Pace::default()).expect("built");

        // Nothing yet: no pin has been reported, so nothing is driving and
        // the first reading is the bottom of the range.
        let first = live.absorb("[rusty:adc@0] 3=0").expect("absorbed");
        assert_eq!(
            first,
            vec![(3u8, 0u16)],
            "the pin sits at nothing: {first:?}"
        );

        // The firmware drives GPIO2 high at one millisecond.
        live.absorb("[rusty:gpio@1000] 2=1").expect("absorbed");

        // Then reads its converter every two hundred microseconds. The
        // counts have to climb rather than arrive.
        let mut seen: Vec<u16> = Vec::new();
        for step in 1..=10 {
            let at = 1_000 + step * 200;
            let back = live
                .absorb(&format!("[rusty:adc@{at}] 3=0"))
                .expect("absorbed");
            if let Some((_, counts)) = back.iter().find(|(pin, _)| *pin == 3) {
                seen.push(*counts);
            }
        }
        assert!(seen.len() >= 8, "a reading per poll: {seen:?}");
        assert!(
            seen.windows(2).all(|pair| pair[1] >= pair[0]),
            "and it only climbs: {seen:?}"
        );
        assert!(
            seen[0] > 0 && *seen.last().unwrap() < 1_000,
            "still climbing through the time constant rather than arrived: {seen:?}"
        );

        // Two milliseconds after the edge is two time constants: about 86%.
        live.absorb("[rusty:adc@3000] 3=0").expect("absorbed");
        let at_two = live.counts_at(3).expect("a reading");
        let want = 1_000.0 * (1.0 - (-2.0f64).exp());
        assert!(
            (f64::from(at_two) - want).abs() < 0.05 * want,
            "two time constants: {at_two} against {want:.0}"
        );

        // And the pin goes low again, so it discharges.
        live.absorb("[rusty:gpio@3000] 2=0").expect("absorbed");
        live.absorb("[rusty:adc@9000] 3=0").expect("absorbed");
        assert!(
            live.counts_at(3).expect("a reading") < 10,
            "back down: {:?}",
            live.counts_at(3)
        );
    }

    /// **The deadlock a real run found, and the headless tests could not.**
    ///
    /// The emulator reports a conversion only when the value changed, and
    /// the value only changes when the host sends one. So after a pin
    /// moves, nobody says anything: the circuit stays at the instant of the
    /// edge, and the next thing the firmware sees is a jump to wherever it
    /// had got to by the *following* edge. The reading steps instead of
    /// climbing, which is a host echoing a pin level in a circuit's
    /// clothes.
    ///
    /// Every test above feeds a dense stream of lines, so time always
    /// advanced and none of them could see it. This one absorbs the edge
    /// and then says nothing at all, exactly as the emulator does.
    #[test]
    fn a_circuit_still_settling_is_followed_while_the_emulator_says_nothing() {
        let (sheet, rows) = driven_rc_read_by_a_pin();
        let mut live = Live::at_rest(sheet, rows, BTreeMap::new(), Pace::default()).expect("built");
        assert!(live.settling(), "there is a capacitor on this sheet");

        // The one line the emulator will say, and then silence.
        live.absorb("[rusty:gpio@0] 2=1").expect("absorbed");

        // A caller with a clock follows it forward. Two hundred
        // microseconds at a time, through one time constant.
        let mut seen: Vec<u16> = Vec::new();
        for _ in 0..10 {
            for (pin, counts) in live.advance_by(200e-6).expect("advanced") {
                if pin == 3 {
                    seen.push(counts);
                }
            }
        }
        assert!(
            seen.len() >= 8,
            "the circuit is followed rather than frozen: {seen:?}"
        );
        assert!(
            seen.windows(2).all(|pair| pair[1] > pair[0]),
            "and it climbs: {seen:?}"
        );
        assert!(
            *seen.last().unwrap() < 1_000,
            "still short of full scale after one time constant, which is the \
             whole difference from an echo: {seen:?}"
        );

        // And a sheet with nothing to settle says so, so a caller does not
        // spend solves on arithmetic nobody reads.
        let (flat, rows) = divider_into_a_pin();
        let live = Live::at_rest(flat, rows, BTreeMap::new(), Pace::default()).expect("built");
        assert!(!live.settling(), "a divider is already where it is going");
    }

    /// The same number is not sent twice, which is the change suppression
    /// every other report on this channel has: a firmware polling its
    /// converter would otherwise fill the line the console shares.
    #[test]
    fn counts_that_have_not_moved_are_not_said_again() {
        let (sheet, rows) = driven_rc_read_by_a_pin();
        let mut live = Live::at_rest(sheet, rows, BTreeMap::new(), Pace::default()).expect("built");
        live.absorb("[rusty:gpio@0] 2=1").expect("absorbed");
        live.absorb("[rusty:adc@200000] 3=0").expect("absorbed");

        // Settled: polling again says nothing.
        let quiet = live.absorb("[rusty:adc@201000] 3=0").expect("absorbed");
        assert!(quiet.is_empty(), "nothing has moved: {quiet:?}");
        let quiet = live.absorb("[rusty:adc@202000] 3=0").expect("absorbed");
        assert!(quiet.is_empty());
    }

    /// A switch held while the firmware runs changes what conducts, and
    /// the reading follows. Without a test this was a correct function
    /// nothing called, which is how dead code starts.
    #[test]
    fn a_switch_held_down_changes_what_the_converter_reads() {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.symbols = vec![
            symbol("Device", "R", "R", &[("1", "~"), ("2", "~")]),
            symbol("Device", "SW_Push", "SW", &[("1", "1"), ("2", "2")]),
            symbol("rusty", "GND", "#PWR", &[("1", "GND")]),
        ];
        let mut props: std::collections::BTreeMap<String, String> = Default::default();
        props.insert("fullscale".into(), "3.3".into());
        props.insert("max".into(), "1000".into());
        for (reference, id, value, scale) in [
            ("R1", "Device:R", "10k", true),
            ("SW1", "Device:SW_Push", "", false),
            ("GND1", "rusty:GND", "GND", false),
        ] {
            sheet.parts.push(Instance {
                reference: reference.into(),
                symbol: id.into(),
                value: value.into(),
                x: 0.0,
                y: 0.0,
                rot: 0,
                mirror: false,
                props: if scale {
                    props.clone()
                } else {
                    Default::default()
                },
            });
        }
        let mut wire = |from: &str, to: &str| {
            sheet.wires.push(Wire {
                from: PinRef::parse(from).unwrap(),
                to: PinRef::parse(to).unwrap(),
                bends: Vec::new(),
            });
        };
        // A pull-up off the rail onto GPIO3, with a switch to ground.
        wire("U1.3V3", "R1.1");
        wire("R1.2", "U1.GPIO3");
        wire("R1.2", "SW1.1");
        wire("SW1.2", "GND1.GND");

        let rows = kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5]);
        let mut live = Live::at_rest(sheet, rows, BTreeMap::new(), Pace::default()).expect("built");
        live.advance_to(1_000).expect("advanced");
        assert_eq!(
            live.counts_at(3),
            Some(1000),
            "the pull-up holds it at the rail"
        );

        live.pressing("SW1", true);
        live.advance_to(2_000).expect("advanced");
        assert_eq!(live.counts_at(3), Some(0), "and the switch pulls it down");

        live.pressing("SW1", false);
        live.advance_to(3_000).expect("advanced");
        assert_eq!(live.counts_at(3), Some(1000), "and lets it go again");
    }

    /// A line that is neither is not the coupling's business, and saying
    /// nothing about it is different from failing on it.
    #[test]
    fn a_line_that_is_not_a_report_is_passed_over() {
        let (sheet, rows) = driven_rc_read_by_a_pin();
        let mut live = Live::at_rest(sheet, rows, BTreeMap::new(), Pace::default()).expect("built");
        assert!(
            live.absorb("hello from the firmware")
                .expect("fine")
                .is_empty()
        );
        assert!(
            live.absorb("[rusty:i2c@10] 3c w 00ae")
                .expect("fine")
                .is_empty()
        );
        assert_eq!(live.micros(), 0, "and none of them moved time");
    }

    /// A pin nothing is wired to has no voltage, which is not zero.
    #[test]
    fn a_pin_the_sheet_does_not_place_has_no_voltage() {
        let live = started();
        assert_eq!(live.volts_at(&PinRef::new("R9", "1")), None);
    }
}
