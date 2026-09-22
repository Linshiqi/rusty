//! A sheet read over one period of the PWM the firmware drives.
//!
//! A pin under PWM has no level. It is high for a share of every period and
//! low for the rest, thousands of times a second, and the emulator says the
//! share (`[rusty:pwm]`) rather than every edge. What anybody sees on the
//! desk is the average — a lamp at 30% duty is a dim lamp, not one
//! flickering at 24 kHz — and what a meter shows is the average too. So the
//! sheet is read at each *moment* of one period in which a different set of
//! pins is high, by the same rules and the same solver that read a sheet
//! with no PWM on it, and every answer here is those readings weighted by
//! how long each moment lasts.
//!
//! **The moments assume the pins rise together** at the start of a period,
//! which is what LEDC does for every channel of one timer with its `hpoint`
//! at zero — esp-hal's default. The pins high at any instant are then the
//! ones whose duty has not yet run out, a prefix of the pins ranked by
//! duty, so `n` pins make `n + 1` moments rather than `2ⁿ`. A lamp on one
//! pin, which is every lamp on an ordinary sheet, does not depend on the
//! assumption at all: its moments are its pin high and its pin low, whatever
//! the other pins do.
//!
//! With no PWM there is one moment, the sheet as it stands, lasting the
//! whole period, and everything here answers exactly what
//! [`nets::evaluate`] and [`circuit::operating_point`] answer on their own.
//! The board reads the sheet through this one door either way, so a lamp on
//! a PWM pin and a lamp on an ordinary one are never two code paths that
//! could disagree.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::circuit::{self, Solved, Unsolved};
use crate::model::{KIT_REFERENCE, PinRef, Sheet, Wire};
use crate::nets::{self, Evaluation, Row, Warning};
use crate::protocol::Duty;

/// The GPIOs a wire on the sheet lands on — the only pins whose PWM can
/// change anything drawn. A motor driven on a pin nothing on the sheet
/// reaches would otherwise cost a reading of the whole sheet and change
/// nothing in it.
pub fn wired_gpios(wires: &[Wire], rows: &[Row]) -> BTreeSet<u8> {
    wires
        .iter()
        .flat_map(|wire| [&wire.from, &wire.to])
        .filter(|end| end.part == KIT_REFERENCE)
        .filter_map(|end| nets::kit_pin(rows, &end.pin))
        .filter_map(|row| rows[row].gpio)
        .collect()
}

/// The PWM pins that reach the sheet, largest duty first: the order they
/// fall in within a period. Ties go by pin number, so the order is total and
/// two equal sets of duties always rank alike.
pub fn ranking(duties: &HashMap<u8, Duty>, wired: &BTreeSet<u8>) -> Vec<u8> {
    let mut pins: Vec<(u8, f64)> = duties
        .iter()
        .filter(|(pin, _)| wired.contains(pin))
        .map(|(pin, drive)| (*pin, share(drive)))
        .collect();
    pins.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    pins.into_iter().map(|(pin, _)| pin).collect()
}

/// Two readings of one thing at two moments that differ by no more than the
/// arithmetic does. Each moment is a solve of its own, and a part nothing
/// switches comes out of two of them the same to a millionth — which is
/// three places further than anything shows it.
fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()) + 1e-12
}

/// A duty as the share of a period it is: clamped, and none of it for a
/// number that is not one.
fn share(drive: &Duty) -> f64 {
    let duty = f64::from(drive.duty);
    if duty.is_finite() {
        duty.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// One instant of a period, read by the rules and by the solver.
#[derive(Debug, Clone, PartialEq)]
struct Moment {
    rules: Evaluation,
    solved: Result<Solved, Unsolved>,
}

/// The sheet at every moment of one period.
///
/// Read once per set of PWM pins and per order of their duties, and
/// weighted by [`Period::weights`] on every change of duty. The split is the
/// point: a breathing lamp changes its duty a hundred times a second, and a
/// reading of the whole sheet per change would be the rules and the solver
/// run for a number that only moves the weights.
#[derive(Debug, Clone, PartialEq)]
pub struct Period {
    /// The PWM pins, largest duty first ([`ranking`]).
    ranking: Vec<u8>,
    /// One reading more than there are pins: the `k`th with the first `k`
    /// pins of the ranking high and every other one low. Never empty.
    moments: Vec<Moment>,
}

/// What a meter on a part shows over a period.
///
/// Three averages rather than two and a product: a lamp lit for half of
/// every period dissipates half its power, and its average voltage times its
/// average current is a quarter of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measured {
    pub across: f64,
    pub through: f64,
    pub watts: f64,
    /// Whether it reads the same at every moment of the period. When it does
    /// not, the three numbers are averages that no instant has — a lamp
    /// under PWM never sits at its average voltage — and whoever shows them
    /// should say so.
    pub steady: bool,
}

/// What a net does over a period.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Low,
    High,
    /// Low for part of every period and high for the rest — high for this
    /// share of it.
    Switching(f64),
}

impl Period {
    /// Read the sheet at every moment of a period of `ranking`'s pins, with
    /// every other pin where the firmware last reported it.
    pub fn read(
        sheet: &Sheet,
        rows: &[Row],
        pressed: &HashSet<String>,
        gpio: &HashMap<u8, bool>,
        ranking: Vec<u8>,
    ) -> Period {
        let moments = (0..=ranking.len())
            .map(|k| {
                let mut levels = gpio.clone();
                for (place, pin) in ranking.iter().enumerate() {
                    levels.insert(*pin, place < k);
                }
                let rules = nets::evaluate(nets::Inputs {
                    sheet,
                    rows,
                    gpio: &levels,
                    pressed,
                });
                // The bridge takes its levels in order, because the element
                // order decides the node numbering and a circuit that
                // renumbered itself between two identical sheets would be a
                // reading that never settles.
                let ordered: BTreeMap<u8, bool> = levels.into_iter().collect();
                let solved = circuit::operating_point(sheet, rows, pressed, &ordered);
                Moment { rules, solved }
            })
            .collect();
        Period { ranking, moments }
    }

    /// How long each moment lasts, as a share of the period: one number per
    /// moment, together one. A ranked pin with no duty — a report the
    /// ranking has not caught up with — counts as held low.
    pub fn weights(&self, duties: &HashMap<u8, Duty>) -> Vec<f64> {
        // Where each moment ends: the period's end, then each pin's duty in
        // ranking order, then its start. A duty larger than the one ranked
        // above it (the ranking lagging a report) is held to it rather than
        // given a moment of negative length.
        let mut ends = Vec::with_capacity(self.ranking.len() + 2);
        ends.push(1.0);
        let mut above: f64 = 1.0;
        for pin in &self.ranking {
            above = duties.get(pin).map_or(0.0, share).min(above);
            ends.push(above);
        }
        ends.push(0.0);
        ends.windows(2).map(|pair| pair[0] - pair[1]).collect()
    }

    /// The moments that happen at all, each with how long it lasts.
    fn lasting<'a>(&'a self, weights: &[f64]) -> impl Iterator<Item = (&'a Moment, f64)> {
        self.moments
            .iter()
            .zip(weights.iter().copied())
            .filter(|(_, weight)| *weight > 0.0)
    }

    /// Which net every pin is in. The same at every moment — a level never
    /// joins two nets or parts them — so the first answers for all of them.
    pub fn nets(&self) -> &Evaluation {
        &self.moments[0].rules
    }

    /// How much of the period any lamp of `part` is lit, from 0 to 1.
    pub fn lit(&self, weights: &[f64], part: &str) -> f64 {
        self.lasting(weights)
            .filter(|(moment, _)| moment.rules.is_lit(part))
            .map(|(_, weight)| weight)
            .sum()
    }

    /// How much of the period one lamp of a part is lit — a channel of an
    /// RGB lens, a segment of a digit.
    pub fn pin_lit(&self, weights: &[f64], part: &str, pin: &str) -> f64 {
        self.lasting(weights)
            .filter(|(moment, _)| moment.rules.is_pin_lit(part, pin))
            .map(|(_, weight)| weight)
            .sum()
    }

    /// What a pin's net does over the period, or nothing when it floats at
    /// any moment of it: a net driven for part of a period and left for the
    /// rest has no level anybody could put a meter on.
    pub fn level(&self, weights: &[f64], pin: &PinRef) -> Option<Level> {
        let mut high = 0.0;
        let (mut rises, mut falls) = (false, false);
        for (moment, weight) in self.lasting(weights) {
            match moment.rules.levels.get(pin).copied().flatten() {
                Some(true) => {
                    high += weight;
                    rises = true;
                }
                Some(false) => falls = true,
                None => return None,
            }
        }
        match (rises, falls) {
            (true, false) => Some(Level::High),
            (false, true) => Some(Level::Low),
            (true, true) => Some(Level::Switching(high)),
            (false, false) => None,
        }
    }

    /// Everything the rules found at any moment of the period, each once.
    /// Two GPIOs driven apart for a moment of every period are fighting,
    /// however short the moment.
    pub fn warnings(&self, weights: &[f64]) -> Vec<Warning> {
        let mut found: Vec<Warning> = Vec::new();
        for (moment, _) in self.lasting(weights) {
            for warning in &moment.rules.warnings {
                if !found.contains(warning) {
                    found.push(warning.clone());
                }
            }
        }
        found
    }

    /// What a meter across `reference` and in series with it reads: nothing
    /// for a part that is not an element of the circuit, and the refusal of
    /// the first moment that refuses — a part that can be measured for part
    /// of a period has no average.
    pub fn reading(&self, weights: &[f64], reference: &str) -> Result<Option<Measured>, &Unsolved> {
        let mut total = Measured {
            across: 0.0,
            through: 0.0,
            watts: 0.0,
            steady: true,
        };
        let mut first = None;
        for (moment, weight) in self.lasting(weights) {
            let Some(reading) = moment.solved.as_ref()?.reading(reference) else {
                return Ok(None);
            };
            let (across, through) = *first.get_or_insert((reading.across, reading.through));
            total.steady &= same(across, reading.across) && same(through, reading.through);
            total.across += weight * reading.across;
            total.through += weight * reading.through;
            total.watts += weight * reading.watts();
        }
        Ok(Some(total))
    }

    /// What a pin sits at on average, which is what a meter's DC range shows
    /// on it — or nothing where the sheet gives it no node.
    pub fn volts_at(&self, weights: &[f64], pin: &PinRef) -> Result<Option<f64>, &Unsolved> {
        let mut total = 0.0;
        for (moment, weight) in self.lasting(weights) {
            let Some(volts) = moment.solved.as_ref()?.volts_at(pin) else {
                return Ok(None);
            };
            total += weight * volts;
        }
        Ok(Some(total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fill, Graphic, Instance, Pin, PinKind, Symbol};

    fn pin(number: &str, name: &str, x: f64) -> Pin {
        Pin {
            number: number.into(),
            name: name.into(),
            kind: PinKind::Passive,
            at: (x, 0.0),
            length: 2.54,
            angle: if x < 0.0 { 0 } else { 180 },
            hidden: false,
        }
    }

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
                .map(|(i, (number, name))| pin(number, name, if i == 0 { -2.54 } else { 2.54 }))
                .collect(),
            graphics: vec![Graphic::Rectangle {
                start: (-1.0, 1.0),
                end: (1.0, -1.0),
                width: 0.0,
                fill: Fill::None,
            }],
        }
    }

    fn sheet() -> Sheet {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.symbols = vec![
            symbol("Device", "R", "R", &[("1", "~"), ("2", "~")]),
            symbol("Device", "LED", "D", &[("1", "K"), ("2", "A")]),
            symbol("rusty", "GND", "#PWR", &[("1", "GND")]),
            symbol("rusty", "Supply", "#PWR", &[("1", "VCC")]),
        ];
        sheet
    }

    fn place(sheet: &mut Sheet, reference: &str, id: &str, value: &str) {
        let mut props = BTreeMap::new();
        if id == "Device:LED" {
            props.insert("vf".to_string(), "2.0".to_string());
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
    }

    fn wire(sheet: &mut Sheet, from: &str, to: &str) {
        sheet.wires.push(Wire {
            from: PinRef::parse(from).unwrap(),
            to: PinRef::parse(to).unwrap(),
            bends: Vec::new(),
        });
    }

    fn rows() -> Vec<Row> {
        nets::kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5])
    }

    /// A lamp with its resistor from `gpio` to ground: the playground's.
    fn lamp_to_ground(sheet: &mut Sheet, lamp: &str, resistor: &str, gpio: u8) {
        place(sheet, resistor, "Device:R", "220");
        place(sheet, lamp, "Device:LED", "red");
        wire(sheet, &format!("U1.GPIO{gpio}"), &format!("{resistor}.1"));
        wire(sheet, &format!("{resistor}.2"), &format!("{lamp}.A"));
        let ground = format!("#PWR{gpio}");
        place(sheet, &ground, "rusty:GND", "GND");
        wire(sheet, &format!("{lamp}.K"), &format!("{ground}.GND"));
    }

    fn duties(of: &[(u8, f32)]) -> HashMap<u8, Duty> {
        of.iter()
            .map(|(pin, duty)| {
                (
                    *pin,
                    Duty {
                        duty: *duty,
                        hz: Some(24_000.0),
                    },
                )
            })
            .collect()
    }

    /// The period of `pwm` over `sheet`, read and weighted as the board
    /// reads it, with `gpio` the levels of every other pin.
    fn over(sheet: &Sheet, gpio: &[(u8, bool)], pwm: &[(u8, f32)]) -> (Period, Vec<f64>) {
        let rows = rows();
        let pwm = duties(pwm);
        let ranking = ranking(&pwm, &wired_gpios(&sheet.wires, &rows));
        let period = Period::read(
            sheet,
            &rows,
            &HashSet::new(),
            &gpio.iter().copied().collect(),
            ranking,
        );
        let weights = period.weights(&pwm);
        (period, weights)
    }

    /// Equal to what a duty carries: the protocol's duty is an `f32`, so 0.3
    /// arrives as 0.30000001 and a tighter test would be testing that.
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6 * a.abs().max(b.abs()).max(1e-3)
    }

    /// With nothing on PWM the period is the sheet as it stands, and every
    /// answer is the rules' and the solver's own — so the board reading
    /// through here changed nothing for a sheet without PWM.
    #[test]
    fn with_no_pwm_the_period_is_the_sheet_as_it_stands() {
        let mut s = sheet();
        lamp_to_ground(&mut s, "D1", "R1", 2);
        let rows = rows();
        let gpio: HashMap<u8, bool> = [(2, true)].into_iter().collect();

        let (period, weights) = over(&s, &[(2, true)], &[]);
        assert_eq!(weights, vec![1.0]);
        let rules = nets::evaluate(nets::Inputs {
            sheet: &s,
            rows: &rows,
            gpio: &gpio,
            pressed: &HashSet::new(),
        });
        assert_eq!(period.nets(), &rules);
        assert_eq!(period.lit(&weights, "D1"), 1.0);

        let solved = circuit::operating_point(
            &s,
            &rows,
            &HashSet::new(),
            &gpio.iter().map(|(p, l)| (*p, *l)).collect(),
        )
        .expect("solved");
        let alone = solved.reading("D1").expect("an element");
        let read = period.reading(&weights, "D1").unwrap().unwrap();
        assert_eq!((read.across, read.through), (alone.across, alone.through));
        assert!(close(read.watts, alone.watts()));
        assert!(read.steady);
    }

    /// The playground's lamp at 30%: lit for 30% of every period, carrying
    /// 30% of its steady current on average, and dissipating 30% of its
    /// power — not the 9% that its average voltage times its average current
    /// would claim.
    #[test]
    fn a_lamp_on_a_pwm_pin_is_lit_for_its_duty() {
        let mut s = sheet();
        lamp_to_ground(&mut s, "D1", "R1", 2);
        let (steady, all) = over(&s, &[(2, true)], &[]);
        let on = steady.reading(&all, "D1").unwrap().unwrap();
        assert!(on.through > 0.004 && on.through < 0.008, "{on:?}");

        let (period, weights) = over(&s, &[], &[(2, 0.3)]);
        assert_eq!(period.ranking, vec![2]);
        assert!(close(period.lit(&weights, "D1"), 0.3));
        let read = period.reading(&weights, "D1").unwrap().unwrap();
        assert!(close(read.through, 0.3 * on.through), "{read:?}");
        assert!(close(read.across, 0.3 * on.across), "{read:?}");
        assert!(close(read.watts, 0.3 * on.watts), "{read:?} against {on:?}");
        // And not the product of the two averages, which is 9% of it.
        assert!(close(read.across * read.through, 0.09 * on.watts));
        assert!(!read.steady, "an average no instant has, and said so");

        match period.level(&weights, &PinRef::new("R1", "1")) {
            Some(Level::Switching(high)) => assert!(close(high, 0.3)),
            other => panic!("the pin's net switches: {other:?}"),
        }
        assert_eq!(
            period.level(&weights, &PinRef::new("D1", "K")),
            Some(Level::Low)
        );
        let volts = period
            .volts_at(&weights, &PinRef::new("R1", "1"))
            .unwrap()
            .unwrap();
        assert!(close(volts, 0.3 * 3.3), "{volts}");
    }

    /// A lamp from the supply into the pin is lit while the pin is *low*, so
    /// a quarter of duty is three quarters of light — which lamps are lit
    /// comes from the rules and the solver, never from assuming which way
    /// round a lamp is wired.
    #[test]
    fn a_lamp_sinking_into_a_pwm_pin_is_lit_while_it_is_low() {
        let mut s = sheet();
        place(&mut s, "PWR1", "rusty:Supply", "3V3");
        place(&mut s, "R1", "Device:R", "220");
        place(&mut s, "D1", "Device:LED", "red");
        wire(&mut s, "PWR1.VCC", "R1.1");
        wire(&mut s, "R1.2", "D1.A");
        wire(&mut s, "D1.K", "U1.GPIO3");

        let (period, weights) = over(&s, &[], &[(3, 0.25)]);
        assert!(close(period.lit(&weights, "D1"), 0.75));
        let (steady, all) = over(&s, &[(3, false)], &[]);
        let on = steady.reading(&all, "D1").unwrap().unwrap();
        let read = period.reading(&weights, "D1").unwrap().unwrap();
        assert!(close(read.through, 0.75 * on.through), "{read:?}");
    }

    /// Two lamps on two pins, each at its own duty, answer their own duty
    /// and not each other's: the assumption about phase never reaches a
    /// lamp that one pin drives.
    #[test]
    fn two_lamps_on_two_pins_each_answer_their_own_duty() {
        let mut s = sheet();
        lamp_to_ground(&mut s, "D1", "R1", 2);
        lamp_to_ground(&mut s, "D2", "R2", 3);

        let (period, weights) = over(&s, &[], &[(2, 0.8), (3, 0.2)]);
        assert_eq!(period.ranking, vec![2, 3]);
        assert_eq!(weights.len(), 3);
        assert!(close(weights.iter().sum(), 1.0));
        assert!(close(period.lit(&weights, "D1"), 0.8));
        assert!(close(period.lit(&weights, "D2"), 0.2));

        let (steady, all) = over(&s, &[(2, true), (3, true)], &[]);
        for (lamp, duty) in [("D1", 0.8), ("D2", 0.2)] {
            let on = steady.reading(&all, lamp).unwrap().unwrap();
            let read = period.reading(&weights, lamp).unwrap().unwrap();
            assert!(close(read.through, duty * on.through), "{lamp}: {read:?}");
        }
    }

    /// A part nothing switches reads as steady beside one that switches:
    /// each moment is a solve of its own, and a lamp on a pin held high must
    /// not be called an average because another pin's lamp breathes.
    #[test]
    fn a_part_nothing_switches_is_steady_beside_one_that_is() {
        let mut s = sheet();
        lamp_to_ground(&mut s, "D1", "R1", 2);
        lamp_to_ground(&mut s, "D2", "R2", 3);
        let (period, weights) = over(&s, &[(3, true)], &[(2, 0.4)]);
        assert!(!period.reading(&weights, "D1").unwrap().unwrap().steady);
        let held = period.reading(&weights, "D2").unwrap().unwrap();
        assert!(held.steady, "{held:?}");
        assert_eq!(period.lit(&weights, "D2"), 1.0);
    }

    /// A duty at either end is a level, and reads as one: nothing switches
    /// at 0% or at 100%, and a probe calling that PWM would be describing a
    /// pin that is simply low or high.
    #[test]
    fn a_duty_at_either_end_is_a_level() {
        let mut s = sheet();
        lamp_to_ground(&mut s, "D1", "R1", 2);
        let net = PinRef::new("R1", "1");

        let (period, weights) = over(&s, &[], &[(2, 0.0)]);
        assert_eq!(weights, vec![1.0, 0.0]);
        assert_eq!(period.level(&weights, &net), Some(Level::Low));
        assert_eq!(period.lit(&weights, "D1"), 0.0);

        let (period, weights) = over(&s, &[], &[(2, 1.0)]);
        assert_eq!(weights, vec![0.0, 1.0]);
        assert_eq!(period.level(&weights, &net), Some(Level::High));
        assert_eq!(period.lit(&weights, "D1"), 1.0);
    }

    /// Only a pin with a wire on it is a moment: a motor on a pin the sheet
    /// never reaches changes nothing drawn, and must not cost a reading.
    #[test]
    fn a_pin_the_sheet_does_not_reach_is_not_ranked() {
        let mut s = sheet();
        lamp_to_ground(&mut s, "D1", "R1", 2);
        let (period, weights) = over(&s, &[(2, true)], &[(4, 0.5), (5, 0.25)]);
        assert!(period.ranking.is_empty());
        assert_eq!(weights, vec![1.0]);
    }

    /// Largest duty first, ties by pin — a total order, so the same duties
    /// always rank the same and a reading keyed on the ranking settles.
    #[test]
    fn the_ranking_is_by_duty_then_by_pin() {
        let wired: BTreeSet<u8> = [1, 3, 5, 7].into_iter().collect();
        let pwm = duties(&[(5, 0.5), (3, 0.5), (1, 0.9), (7, 0.1), (9, 1.0)]);
        assert_eq!(ranking(&pwm, &wired), vec![1, 3, 5, 7]);
    }

    /// A ranking that has not caught up with the duties is held to its own
    /// order rather than given a moment of negative length: every weight is
    /// a length of time, and together they are the period.
    #[test]
    fn weights_are_lengths_of_time_even_when_the_ranking_lags() {
        let mut s = sheet();
        lamp_to_ground(&mut s, "D1", "R1", 2);
        lamp_to_ground(&mut s, "D2", "R2", 3);
        let (period, _) = over(&s, &[], &[(2, 0.8), (3, 0.2)]);
        // Now the other way round, and a pin the ranking never heard of.
        let weights = period.weights(&duties(&[(2, 0.1), (3, 0.6), (4, 0.9)]));
        assert!(weights.iter().all(|w| *w >= 0.0), "{weights:?}");
        assert!(close(weights.iter().sum(), 1.0), "{weights:?}");
    }

    /// Two GPIOs wired together, one held high and the other under PWM,
    /// fight for the low part of every period — and that is said, however
    /// short the part.
    #[test]
    fn a_fight_for_part_of_a_period_is_a_fight() {
        let mut s = sheet();
        lamp_to_ground(&mut s, "D1", "R1", 2);
        wire(&mut s, "U1.GPIO2", "U1.GPIO3");
        let (period, weights) = over(&s, &[(3, true)], &[(2, 0.9)]);
        assert!(
            period
                .warnings(&weights)
                .iter()
                .any(|w| matches!(w, Warning::Conflict { .. })),
            "{:?}",
            period.warnings(&weights)
        );
    }
}
