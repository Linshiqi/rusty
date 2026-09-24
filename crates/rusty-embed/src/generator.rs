//! A signal generator on the sheet, and the tables a run plays: at every
//! converter a generator reaches, and in the registers of every sensor a
//! signal moves (`docs/signals.md`, "The host").
//!
//! A generator is a source in the sheet's circuit — `circuit::of` puts one
//! between its `OUT` and `GND` — and what a converter reads while it plays
//! is that circuit stepped with the source at each sample: through whatever
//! the sheet puts between them, an RC as the solver says and a divider as it
//! divides. The table is one loop of that, in the converter's counts at the
//! pin's `fullscale`, which the emulator then plays against the firmware's
//! own clock. A sensor's is simpler, because a reading is not a voltage:
//! `signal.<reading>` on a part with a `model` is that reading's value over
//! time, encoded into the part's register block sample by sample.
//!
//! Pure, and every choice a run makes is a function here — the rate, the
//! loop, the seed — so the frontend renders the signal it draws with exactly
//! what the backend hands the emulator.

use std::collections::{BTreeMap, HashSet};

use crate::circuit::{self, Unsolved};
use crate::live::{Scale, counts_of, scale_at};
use crate::model::{Instance, KIT_REFERENCE, PinRef, Sheet};
use crate::nets::{Behaviour, Row, behaviour_of};
use crate::sensor;
use crate::signal::{Signal, SignalError};
use crate::solve::{Element, Transient, Trouble};
use crate::wave::Table;

/// The property a generator keeps its signal in.
pub const SIGNAL: &str = "signal";

/// The prefix a sensor's moving readings are kept under: `signal.gz`.
pub const SIGNAL_OF: &str = "signal.";

/// The one key under that prefix that is not a reading: how fast a sensor's
/// block plays.
pub const SIGNAL_RATE: &str = "signal.rate";

/// How fast a generator plays unless its `rate` says otherwise. Twenty
/// thousand samples a second is well past what a firmware reading its
/// converter samples at, so what it reads between two samples is the
/// straight line the emulator draws between them rather than a staircase.
/// It is also the bandwidth of the generator's noise, and noise that wide
/// folds into a slower firmware's readings the way it does at a real
/// converter — which a filter has to be designed for.
pub const GENERATOR_RATE: u32 = 20_000;

/// And a sensor's readings, unless its `signal.rate` says otherwise: an IMU
/// run at a kilohertz, and faster than a barometer converts at all. A
/// sensor's block is not interpolated — a register holds one sample until
/// the next — which is what a part's output data rate means.
pub const SENSOR_RATE: u32 = 1_000;

/// The fastest either plays. A generator's table is the sheet's circuit
/// stepped a sample at a time, and past a megahertz a second of it is a
/// million solves.
pub const FASTEST: u32 = 1_000_000;

/// The shortest and longest a table lasts. A second at least, so a table
/// of noise does not replay itself fast enough to show up as a tone; ten
/// at most, which holds a chirp's whole sweep and costs a third of a second
/// to render at the default rate.
pub const SHORTEST_LOOP: f64 = 1.0;
pub const LONGEST_LOOP: f64 = 10.0;

/// The most samples one table holds, whatever its rate: two megabytes in
/// the emulator for a pin, and four of hex down the channel.
pub const MOST_SAMPLES: usize = 1 << 20;

/// One generator and what it plays: one loop of volts, `rate` a second.
#[derive(Debug, Clone, Copy)]
pub struct Drive<'a> {
    pub part: &'a str,
    pub volts: &'a [f64],
}

/// Whether anything on the sheet plays a signal: a generator, or a sensor
/// with a reading a signal moves (`signal.<reading>` in its properties).
/// What decides whether a run needs an emulator that plays tables.
pub fn plays_anything(sheet: &Sheet) -> bool {
    sheet.parts.iter().any(|part| {
        part.props.keys().any(|key| is_reading_key(key))
            || sheet
                .symbol_of(&part.reference)
                .is_some_and(|symbol| behaviour_of(symbol) == Behaviour::Generator)
    })
}

/// `signal.<reading>`, and not the rate kept beside the readings.
fn is_reading_key(key: &str) -> bool {
    key.starts_with(SIGNAL_OF) && key != SIGNAL_RATE && key.len() > SIGNAL_OF.len()
}

/// What `part`'s `key` property plays, read strictly. `None` when there is
/// no such property; an empty one is silence, which is a signal too.
pub fn signal_at(part: &Instance, key: &str) -> Option<Result<Signal, SignalError>> {
    part.props.get(key).map(|text| Signal::parse(text))
}

/// The rate a part's tables play at: its `key` property, or `default`. A
/// rate that is not a whole number of samples a second from one to
/// [`FASTEST`] is refused by name rather than replaced — a table played at
/// a rate nobody asked for is a tone at a frequency nobody asked for.
pub fn rate_at(part: &Instance, key: &str, default: u32) -> Result<u32, String> {
    let Some(text) = part.props.get(key) else {
        return Ok(default);
    };
    text.trim()
        .parse::<u32>()
        .ok()
        .filter(|rate| (1..=FASTEST).contains(rate))
        .ok_or_else(|| {
            format!("{key} = \"{text}\" is not a rate from 1 to {FASTEST} samples a second")
        })
}

/// The seed a signal's noise is drawn with: the part's reference and the
/// property the signal is kept in, so two generators given one signal do
/// not hiss in unison and a sensor's three axes are not one axis three
/// times over — mixed with the part's `seed`, when it states one, for
/// another draw of the same noise. The frontend renders with the same
/// seed, so the signal the panel draws is the one the firmware is fed.
pub fn seed_of(part: &Instance, key: &str) -> u64 {
    // FNV-1a: rusty's own, so a seed means the same noise after an upgrade.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in part.reference.bytes().chain([0]).chain(key.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^ part.prop::<u64>("seed").unwrap_or(0)
}

/// How long a table lasts, and what happens where its end meets its start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loop {
    pub samples: usize,
    /// Every periodic component is back where it started at the joint.
    /// False when no length up to [`LONGEST_LOOP`] does that — tones with no
    /// common period that short, or a step still to come — and the table
    /// then has a seam the run says it has.
    pub seamless: bool,
    /// The noise in it is the same draw every loop, not new noise.
    pub repeats_noise: bool,
}

impl Loop {
    pub fn seconds(&self, rate: u32) -> f64 {
        self.samples as f64 / f64::from(rate.max(1))
    }
}

/// The loop every one of `signals` fits when they play together at `rate`:
/// the fewest samples from a second up after which every periodic
/// component of all of them is back where it started, or the longest a
/// table may be when there is none.
pub fn loop_of<'a>(signals: impl IntoIterator<Item = &'a Signal>, rate: u32) -> Loop {
    let together = Signal {
        components: signals
            .into_iter()
            .flat_map(|signal| signal.components.iter().cloned())
            .collect(),
    };
    let hertz = f64::from(rate.max(1));
    let longest = LONGEST_LOOP.min(MOST_SAMPLES as f64 / hertz);
    let repeats_noise = together.is_random();
    match together.loop_length(hertz, SHORTEST_LOOP.min(longest), longest) {
        Some(samples) => Loop {
            samples,
            seamless: true,
            repeats_noise,
        },
        None => Loop {
            samples: ((longest * hertz).floor() as usize).max(1),
            seamless: false,
            repeats_noise,
        },
    }
}

/// The volts a generator's source stands at before anything plays: its
/// signal's first sample, which is what the circuit is solved at when
/// nothing is moving — the editor's readings, and the operating point a
/// table starts from. A generator whose signal does not read stands at
/// nothing, as one switched off does.
pub fn volts_at_start(part: &Instance) -> f64 {
    let rate = rate_at(part, "rate", GENERATOR_RATE).unwrap_or(GENERATOR_RATE);
    match signal_at(part, SIGNAL) {
        Some(Ok(signal)) => signal
            .render(f64::from(rate), 1, seed_of(part, SIGNAL))
            .first()
            .copied()
            .filter(|volts| volts.is_finite())
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

/// What a run plays, and what it should say about it: one line per thing
/// played, and one per thing that could not be.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Played {
    pub tables: Vec<Table>,
    pub said: Vec<String>,
}

/// Everything the generators on `sheet` play: a table for each converter
/// they reach, rendered at the fastest rate any of them asks for over the
/// loop all of them fit.
///
/// A generator whose signal or rate does not read plays nothing and says
/// why, and the others play regardless; a circuit with no answer plays
/// nothing at all, because every table is read off it.
pub fn generators(sheet: &Sheet, rows: &[Row]) -> Played {
    let mut said = Vec::new();
    let mut playing: Vec<(&Instance, Signal)> = Vec::new();
    let mut rate = 0u32;
    for part in &sheet.parts {
        let is_generator = sheet
            .symbol_of(&part.reference)
            .is_some_and(|symbol| behaviour_of(symbol) == Behaviour::Generator);
        if !is_generator {
            continue;
        }
        let signal = match signal_at(part, SIGNAL) {
            // Placed and never set: a generator with its output at zero.
            None => Signal::default(),
            Some(Ok(signal)) => signal,
            Some(Err(why)) => {
                said.push(format!(
                    "[rusty:signal] {}: its signal does not read, so it plays nothing — {why}",
                    part.reference
                ));
                continue;
            }
        };
        match rate_at(part, "rate", GENERATOR_RATE) {
            Ok(own) => rate = rate.max(own),
            Err(why) => {
                said.push(format!(
                    "[rusty:signal] {}: it plays nothing — {why}",
                    part.reference
                ));
                continue;
            }
        }
        playing.push((part, signal));
    }
    if playing.is_empty() {
        return Played {
            tables: Vec::new(),
            said,
        };
    }

    let looped = loop_of(playing.iter().map(|(_, signal)| signal), rate);
    let rendered: Vec<(&str, Vec<f64>)> = playing
        .iter()
        .map(|(part, signal)| {
            (
                part.reference.as_str(),
                signal.render(f64::from(rate), looped.samples, seed_of(part, SIGNAL)),
            )
        })
        .collect();
    let drives: Vec<Drive<'_>> = rendered
        .iter()
        .map(|(part, volts)| Drive { part, volts })
        .collect();
    let names: Vec<&str> = rendered.iter().map(|(part, _)| *part).collect();

    match pin_tables(sheet, rows, &drives, rate) {
        Ok(tables) if tables.is_empty() => said.push(format!(
            "[rusty:signal] {} reach no converter whose full scale a part states \
             (`fullscale` on a part on the pin's net), so nothing plays",
            names.join(", ")
        )),
        Ok(tables) => {
            let pins: Vec<String> = tables
                .iter()
                .filter_map(|table| match table {
                    Table::Pin { gpio, .. } => Some(format!("GPIO{gpio}")),
                    Table::Block { .. } => None,
                })
                .collect();
            said.push(format!(
                "[rusty:signal] {} play on {}: {rate} samples a second, {}",
                names.join(", "),
                pins.join(", "),
                describe(looped, rate)
            ));
            return Played { tables, said };
        }
        Err(why) => said.push(format!(
            "[rusty:signal] the sheet's circuit has no answer, so no generator plays — {why}"
        )),
    }
    Played {
        tables: Vec::new(),
        said,
    }
}

/// A loop, for a line in the run's log.
fn describe(looped: Loop, rate: u32) -> String {
    let mut text = format!("a {} s loop", looped.seconds(rate));
    if !looped.seamless {
        text.push_str(" with a seam where it repeats: its components share no period that short");
    }
    if looped.repeats_noise {
        text.push_str(", its noise the same draw each time round");
    }
    text
}

/// What moves one sensor's readings: each moving reading's samples, all one
/// loop at one rate, and what the run should say about them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Moved {
    pub signals: Vec<(String, Vec<f64>)>,
    pub rate: u32,
    pub said: Vec<String>,
}

/// The readings `signal.<reading>` moves on a sensor rusty answers for. A
/// key naming no reading of the part is said with the ones it has — a
/// signal on `gx` of a barometer is a typo somebody wants to hear about,
/// not a signal quietly dropped.
pub fn sensor_signals(part: &Instance, device: &sensor::Device) -> Moved {
    let mut said = Vec::new();
    let rate = match rate_at(part, SIGNAL_RATE, SENSOR_RATE) {
        Ok(rate) => rate,
        Err(why) => {
            return Moved {
                said: vec![format!(
                    "[rusty:signal] {}: no reading moves — {why}",
                    part.reference
                )],
                ..Moved::default()
            };
        }
    };
    let mut read: Vec<(String, Signal, u64)> = Vec::new();
    for key in part.props.keys().filter(|key| is_reading_key(key)) {
        let reading = &key[SIGNAL_OF.len()..];
        if device.value(reading).is_none() {
            said.push(format!(
                "[rusty:signal] {}: {key} names no reading of this part, which reads {}",
                part.reference,
                device.keys().join(", ")
            ));
            continue;
        }
        match signal_at(part, key) {
            Some(Ok(signal)) => read.push((reading.to_string(), signal, seed_of(part, key))),
            Some(Err(why)) => said.push(format!(
                "[rusty:signal] {}: {key} does not read, so it stays where its slider is — {why}",
                part.reference
            )),
            None => {}
        }
    }
    if read.is_empty() {
        return Moved {
            signals: Vec::new(),
            rate,
            said,
        };
    }
    let looped = loop_of(read.iter().map(|(_, signal, _)| signal), rate);
    let signals: Vec<(String, Vec<f64>)> = read
        .iter()
        .map(|(reading, signal, seed)| {
            (
                reading.clone(),
                signal.render(f64::from(rate), looped.samples, *seed),
            )
        })
        .collect();
    let names: Vec<&str> = signals
        .iter()
        .map(|(reading, _)| reading.as_str())
        .collect();
    said.push(format!(
        "[rusty:signal] {} moves {}: {rate} samples a second, {}",
        part.reference,
        names.join(", "),
        describe(looped, rate)
    ));
    Moved {
        signals,
        rate,
        said,
    }
}

/// What each converter the generators reach reads while they play, as a
/// table per pin: the sheet's circuit stepped at `rate` with every
/// generator at its sample, read at each GPIO a part on its net gives a
/// `fullscale` for.
///
/// Only the pins a generator actually reaches: a converter the generators
/// cannot move is left to whatever else is on it — a knob, the live circuit
/// — and a table there would hold it still. A reached pin whose counts
/// never move gets a table of one sample, so it reads the generator's level
/// rather than whatever was said before.
///
/// One loop is stepped and dropped before the loop that is kept, so the
/// table starts where its own end leaves the circuit rather than from rest:
/// an RC charged by a tone is in the middle of its swing when the loop
/// comes round, and a table that began from rest would put a seam there.
pub fn pin_tables(
    sheet: &Sheet,
    rows: &[Row],
    drives: &[Drive<'_>],
    rate: u32,
) -> Result<Vec<Table>, Unsolved> {
    let len = drives
        .iter()
        .map(|drive| drive.volts.len())
        .min()
        .unwrap_or(0);
    if len == 0 || rate == 0 {
        return Ok(Vec::new());
    }
    let bridged = circuit::of(sheet, rows, &HashSet::new(), &BTreeMap::new())?;
    let sources: Vec<(usize, &[f64])> = drives
        .iter()
        .filter_map(|drive| Some((*bridged.element_of.get(drive.part)?, drive.volts)))
        .collect();
    let pins: Vec<(u8, usize, Scale)> = rows
        .iter()
        .filter_map(|row| {
            let gpio = row.gpio?;
            let node = *bridged
                .node_of
                .get(&PinRef::new(KIT_REFERENCE, &row.name))?;
            Some((gpio, node, scale_at(sheet, &bridged, node)?))
        })
        .collect();
    if sources.is_empty() || pins.is_empty() {
        return Ok(Vec::new());
    }

    let mut circuit = bridged.circuit.clone();
    for (element, volts) in &sources {
        if let Some(Element::Source { volts: at, .. }) = circuit.elements.get_mut(*element) {
            *at = volts[0];
        }
    }
    // From where the circuit settles with every generator at its first
    // sample — or, where a node is reached only through a capacitor and
    // there is no such place, from rest: the loop stepped first carries it
    // to where it would be anyway.
    let mut run = match Transient::settled(circuit.clone()) {
        Ok(run) => run,
        Err(Trouble::Floating { .. }) => Transient::at_rest(circuit),
        Err(why) => return Err(circuit::unsolved(&bridged, why)),
    };
    let step = 1.0 / f64::from(rate);
    let unsolved = |why| circuit::unsolved(&bridged, why);

    // Which pins the generators reach: one step with each generator a volt
    // from where it stands, against one step with none moved. A step and
    // not a DC solve, so a pin behind a capacitor — a high-pass, a pin
    // AC-coupled to the source — counts as reached, which it is.
    let still: Vec<f64> = {
        let mut probe = run.clone();
        probe.step(step).map_err(unsolved)?;
        pins.iter()
            .map(|(_, node, _)| probe.volts_at(*node))
            .collect()
    };
    let mut reached = vec![false; pins.len()];
    for (element, volts) in &sources {
        let mut probe = run.clone();
        probe.drive(*element, volts[0] + 1.0);
        probe.step(step).map_err(unsolved)?;
        for (at, (_, node, _)) in pins.iter().enumerate() {
            reached[at] |= (probe.volts_at(*node) - still[at]).abs() > 1e-9;
        }
    }
    let pins: Vec<(u8, usize, Scale)> = pins
        .into_iter()
        .zip(&reached)
        .filter(|(_, reached)| **reached)
        .map(|(pin, _)| pin)
        .collect();
    if pins.is_empty() {
        return Ok(Vec::new());
    }

    let mut counts: Vec<Vec<u16>> = vec![Vec::with_capacity(len); pins.len()];
    for sample in 0..2 * len {
        let at = sample % len;
        for (element, volts) in &sources {
            run.drive(*element, volts[at]);
        }
        run.step(step).map_err(unsolved)?;
        if sample >= len {
            for ((_, node, scale), table) in pins.iter().zip(counts.iter_mut()) {
                table.push(counts_of(run.volts_at(*node), *scale));
            }
        }
    }

    Ok(pins
        .iter()
        .zip(counts)
        .map(|((gpio, _, _), mut counts)| {
            if counts.iter().all(|c| *c == counts[0]) {
                counts.truncate(1);
            }
            Table::Pin {
                gpio: *gpio,
                rate,
                counts,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Graphic, Pin, PinKind, Symbol, Wire};
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
                .map(|(i, (number, name))| Pin {
                    number: (*number).into(),
                    name: (*name).into(),
                    kind: PinKind::Passive,
                    at: (if i % 2 == 0 { -5.08 } else { 5.08 }, 0.0),
                    length: 2.54,
                    angle: 0,
                    hidden: false,
                })
                .collect(),
            graphics: Vec::<Graphic>::new(),
        }
    }

    fn sheet() -> Sheet {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.symbols = vec![
            symbol("rusty", "SignalGen", "V", &[("1", "OUT"), ("2", "GND")]),
            symbol("Device", "R", "R", &[("1", "~"), ("2", "~")]),
            symbol("Device", "C", "C", &[("1", "~"), ("2", "~")]),
        ];
        sheet
    }

    fn place(
        sheet: &mut Sheet,
        reference: &str,
        symbol: &str,
        value: &str,
        props: &[(&str, &str)],
    ) {
        sheet.parts.push(Instance {
            reference: reference.into(),
            symbol: symbol.into(),
            value: value.into(),
            x: 0.0,
            y: 0.0,
            rot: 0,
            mirror: false,
            props: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
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
        kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5])
    }

    /// Straight onto the pin, a generator's table is its volts in counts at
    /// the full scale it states — sample for sample, nothing in between.
    #[test]
    fn a_generator_wired_to_a_pin_is_its_volts_in_counts() {
        let mut s = sheet();
        place(
            &mut s,
            "V1",
            "rusty:SignalGen",
            "Signal",
            &[("fullscale", "3.3")],
        );
        wire(&mut s, "V1.1", "U1.GPIO3");
        wire(&mut s, "V1.2", "U1.GND");
        let volts: Vec<f64> = (0..100).map(|n| 3.3 * f64::from(n) / 99.0).collect();
        let tables = pin_tables(
            &s,
            &rows(),
            &[Drive {
                part: "V1",
                volts: &volts,
            }],
            1000,
        )
        .unwrap();
        let [Table::Pin { gpio, rate, counts }] = tables.as_slice() else {
            panic!("{tables:?}");
        };
        assert_eq!((*gpio, *rate), (3, 1000));
        let want: Vec<u16> = volts
            .iter()
            .map(|v| (v / 3.3 * 4095.0).round() as u16)
            .collect();
        assert_eq!(counts, &want);
    }

    /// Through an RC, the table is the solver's answer: a square wave into
    /// 10 kΩ and 1 µF charges the pin as 1 − e^(−t/τ) with τ = 10 ms, and
    /// the table reads that one time constant after each rising edge —
    /// within what backward Euler at the table's step allows, which is a
    /// fraction of a percent at 20 samples a millisecond.
    #[test]
    fn a_generator_through_an_rc_charges_the_pin_as_the_circuit_does() {
        let mut s = sheet();
        place(&mut s, "V1", "rusty:SignalGen", "Signal", &[]);
        place(&mut s, "R1", "Device:R", "10k", &[]);
        place(&mut s, "C1", "Device:C", "1u", &[("fullscale", "3.3")]);
        wire(&mut s, "V1.1", "R1.1");
        wire(&mut s, "R1.2", "U1.GPIO3");
        wire(&mut s, "C1.1", "U1.GPIO3");
        wire(&mut s, "C1.2", "U1.GND");
        wire(&mut s, "V1.2", "U1.GND");
        // 5 Hz, square: 100 ms high and 100 ms low, at 20 kHz.
        let rate = 20_000u32;
        let volts: Vec<f64> = (0..4000)
            .map(|n| if n < 2000 { 3.3 } else { 0.0 })
            .collect();
        let tables = pin_tables(
            &s,
            &rows(),
            &[Drive {
                part: "V1",
                volts: &volts,
            }],
            rate,
        )
        .unwrap();
        let [Table::Pin { counts, .. }] = tables.as_slice() else {
            panic!("{tables:?}");
        };
        // One time constant after the rising edge, from a pin that had
        // fully discharged over the low half before it.
        let at_tau = f64::from(counts[200]);
        let want = 4095.0 * (1.0 - (-1.0f64).exp());
        assert!(
            (at_tau - want).abs() < want * 0.005,
            "{at_tau} counts one τ in, not {want:.0}"
        );
        // And the loop has no seam: the last sample, at the end of the low
        // half, is the fully discharged pin the first sample rises from.
        assert!(counts[3999] < 5, "{}", counts[3999]);
    }

    /// A pin nobody states a converter for is not given a table: counts
    /// against a full scale nobody said would be the confident wrong
    /// number the live circuit already refuses.
    #[test]
    fn a_pin_without_a_stated_converter_gets_no_table() {
        let mut s = sheet();
        place(&mut s, "V1", "rusty:SignalGen", "Signal", &[]);
        wire(&mut s, "V1.1", "U1.GPIO3");
        wire(&mut s, "V1.2", "U1.GND");
        let volts = [0.0, 1.0, 2.0];
        let tables = pin_tables(
            &s,
            &rows(),
            &[Drive {
                part: "V1",
                volts: &volts,
            }],
            1000,
        )
        .unwrap();
        assert!(tables.is_empty(), "{tables:?}");
    }

    /// A converter the generator cannot move is not the generator's: a knob
    /// on another pin keeps its own value, where a table of the generator's
    /// would have held it still. And a pin the generator reaches through a
    /// capacitor alone is reached — a high-pass is still a path.
    #[test]
    fn only_the_pins_a_generator_reaches_are_played() {
        let mut s = sheet();
        place(
            &mut s,
            "V1",
            "rusty:SignalGen",
            "Signal",
            &[("fullscale", "3.3")],
        );
        wire(&mut s, "V1.1", "U1.GPIO3");
        wire(&mut s, "V1.2", "U1.GND");
        // A divider on another pin, with its own converter, that the
        // generator is nowhere near.
        place(&mut s, "R1", "Device:R", "10k", &[("fullscale", "3.3")]);
        place(&mut s, "R2", "Device:R", "10k", &[]);
        wire(&mut s, "R1.1", "U1.3V3");
        wire(&mut s, "R1.2", "U1.GPIO4");
        wire(&mut s, "R2.1", "U1.GPIO4");
        wire(&mut s, "R2.2", "U1.GND");
        // And a high-pass from the generator to a third pin.
        place(&mut s, "C1", "Device:C", "1u", &[]);
        place(&mut s, "R3", "Device:R", "10k", &[("fullscale", "3.3")]);
        wire(&mut s, "V1.1", "C1.1");
        wire(&mut s, "C1.2", "U1.GPIO5");
        wire(&mut s, "R3.1", "U1.GPIO5");
        wire(&mut s, "R3.2", "U1.GND");

        let volts: Vec<f64> = (0..200)
            .map(|n| 1.65 + (f64::from(n) * 0.1).sin())
            .collect();
        let tables = pin_tables(
            &s,
            &rows(),
            &[Drive {
                part: "V1",
                volts: &volts,
            }],
            1000,
        )
        .unwrap();
        let gpios: Vec<u8> = tables
            .iter()
            .map(|table| match table {
                Table::Pin { gpio, .. } => *gpio,
                Table::Block { .. } => panic!("{table:?}"),
            })
            .collect();
        assert_eq!(gpios, [3, 5], "{tables:?}");
    }

    /// A generator holding still still owns its pin: one sample, at its
    /// level, rather than no table and whatever was said before.
    #[test]
    fn a_still_generator_plays_one_sample() {
        let mut s = sheet();
        place(
            &mut s,
            "V1",
            "rusty:SignalGen",
            "Signal",
            &[("fullscale", "3.3"), (SIGNAL, "dc 1.65")],
        );
        wire(&mut s, "V1.1", "U1.GPIO3");
        wire(&mut s, "V1.2", "U1.GND");
        let played = generators(&s, &rows());
        let [Table::Pin { counts, .. }] = played.tables.as_slice() else {
            panic!("{played:?}");
        };
        assert_eq!(counts, &[2048], "1.65 V of 3.3 at twelve bits");
    }

    /// What a run says: where each generator plays and for how long a loop,
    /// and a generator whose signal does not read is named with the reason
    /// while the others play.
    #[test]
    fn a_run_says_what_plays_and_what_does_not() {
        let mut s = sheet();
        place(
            &mut s,
            "V1",
            "rusty:SignalGen",
            "Signal",
            &[("fullscale", "3.3"), (SIGNAL, "dc 1.65; sine f=50 a=1")],
        );
        wire(&mut s, "V1.1", "U1.GPIO3");
        wire(&mut s, "V1.2", "U1.GND");
        place(
            &mut s,
            "V2",
            "rusty:SignalGen",
            "Signal",
            &[("fullscale", "3.3"), (SIGNAL, "sine fq=50 a=1")],
        );
        wire(&mut s, "V2.1", "U1.GPIO4");
        wire(&mut s, "V2.2", "U1.GND");

        let played = generators(&s, &rows());
        assert_eq!(played.tables.len(), 1, "{played:?}");
        let [Table::Pin { gpio, counts, .. }] = played.tables.as_slice() else {
            panic!("{played:?}");
        };
        assert_eq!(*gpio, 3);
        // A 50 Hz tone at 20 kHz loops in a second, the shortest allowed.
        assert_eq!(counts.len(), 20_000);
        assert!(
            played.said.iter().any(|line| line.contains("V2")
                && line.contains("fq")
                && line.contains("plays nothing")),
            "{:?}",
            played.said
        );
        assert!(
            played
                .said
                .iter()
                .any(|line| line.contains("V1 play on GPIO3") && line.contains("a 1 s loop")),
            "{:?}",
            played.said
        );
    }

    /// Tones with no common period short enough loop at the longest and
    /// say they have a seam; noise says it repeats.
    #[test]
    fn a_loop_with_a_seam_says_so() {
        let tone = Signal::parse("sine f=0.07 a=1").unwrap();
        let looped = loop_of([&tone], 1000);
        assert!(!looped.seamless);
        assert_eq!(looped.samples, 10_000);
        let noise = Signal::parse("sine f=50 a=1; white rms=0.1").unwrap();
        let looped = loop_of([&noise], 1000);
        assert!(looped.seamless && looped.repeats_noise);
        assert_eq!(looped.samples, 1000);
        assert!(describe(looped, 1000).contains("same draw"));
    }

    /// Two generators with one signal draw different noise, and one part's
    /// stated `seed` draws another; the same part and key always draw the
    /// same.
    #[test]
    fn every_signal_draws_noise_of_its_own() {
        let part = |reference: &str, seed: Option<&str>| {
            let mut props = BTreeMap::new();
            if let Some(seed) = seed {
                props.insert("seed".to_string(), seed.to_string());
            }
            Instance {
                reference: reference.into(),
                symbol: "rusty:SignalGen".into(),
                value: String::new(),
                x: 0.0,
                y: 0.0,
                rot: 0,
                mirror: false,
                props,
            }
        };
        let one = seed_of(&part("V1", None), SIGNAL);
        assert_eq!(one, seed_of(&part("V1", None), SIGNAL));
        assert_ne!(one, seed_of(&part("V2", None), SIGNAL));
        assert_ne!(one, seed_of(&part("V1", Some("7")), SIGNAL));
        assert_ne!(
            seed_of(&part("S1", None), "signal.gx"),
            seed_of(&part("S1", None), "signal.gy")
        );
    }

    /// A rate is a number of samples a second in range, or it is refused
    /// by name; absent, it is the default.
    #[test]
    fn a_rate_that_is_not_one_is_refused() {
        let with = |rate: &str| Instance {
            reference: "V1".into(),
            symbol: "rusty:SignalGen".into(),
            value: String::new(),
            x: 0.0,
            y: 0.0,
            rot: 0,
            mirror: false,
            props: [("rate".to_string(), rate.to_string())].into(),
        };
        assert_eq!(rate_at(&with("8000"), "rate", GENERATOR_RATE), Ok(8000));
        for bad in ["0", "fast", "2000000", "-5", "1.5"] {
            let refused = rate_at(&with(bad), "rate", GENERATOR_RATE).unwrap_err();
            assert!(refused.contains(bad), "{refused}");
        }
    }
}
