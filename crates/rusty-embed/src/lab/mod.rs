//! The signal lab's arithmetic, apart from anything it draws: what a source
//! on the sheet plays, the records every instrument reads, a sweep's
//! frequencies and what each step measured — for the window, which draws
//! the instruments and runs the sweep on it, and words what it refuses
//! (`docs/signals.md`).

use std::collections::BTreeMap;

use crate::generator::{self, GENERATOR_RATE, SENSOR_RATE, SIGNAL, SIGNAL_OF, SIGNAL_RATE};
use crate::model::{Instance, Sheet};
use crate::nets::{Behaviour, behaviour_of};
use crate::signal::{Signal, SignalError};
use crate::wave::WaveTarget;

pub mod chart;
pub mod design;
pub mod record;
pub mod response;
pub mod spectrum;
pub mod sweep;
pub mod time;

/// Conversions kept per pin: a minute of a kilohertz loop. Past it the
/// oldest go, as a stream's should — what the converter just did is what
/// somebody is looking at.
pub const CONVERSIONS_KEPT: usize = 60_000;

/// Where a signal is kept on the sheet: a generator's `signal`, or a sensor
/// reading's `signal.<reading>`.
#[derive(Clone, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct Source {
    pub part: String,
    pub key: String,
}

impl Source {
    /// How the lab names it: `V1`, or `U2 · gz` for a sensor's reading.
    pub fn label(&self) -> String {
        match self.key.strip_prefix(SIGNAL_OF) {
            Some(reading) => format!("{} · {reading}", self.part),
            None => self.part.clone(),
        }
    }
}

/// The emulator's account of one table being switched, from
/// `[rusty:wave@<us>]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Switched {
    /// When, on the emulator's virtual clock.
    pub at_us: u64,
    /// Where sample 0 of what is playing sits on that clock; `None` once
    /// it stopped.
    pub start_us: Option<u64>,
}

/// One conversion onto a pin's record, the oldest dropped past the cap.
pub fn record_conversion(record: &mut Vec<(u64, u16)>, at_us: u64, counts: u16) {
    record.push((at_us, counts));
    if record.len() > CONVERSIONS_KEPT {
        let over = record.len() - CONVERSIONS_KEPT;
        record.drain(..over);
    }
}

/// Why a source plays nothing: its part is gone from the sheet, its text
/// is not a signal, or its rate is not one. Worded by whoever shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum Unplayable {
    Gone(String),
    Signal(SignalError),
    Rate(String),
}

/// Every signal on `sheet` — each generator's, and each sensor reading a
/// signal moves — with what the sheet says it plays, in the sheet's order.
/// A generator whose signal was never set plays silence, and is listed so
/// it can be given one.
pub fn sources_of(sheet: &Sheet) -> Vec<(Source, String)> {
    let mut out = Vec::new();
    for part in &sheet.parts {
        let is_generator = sheet
            .symbol_of(&part.reference)
            .is_some_and(|symbol| behaviour_of(symbol) == Behaviour::Generator);
        if is_generator {
            out.push((
                Source {
                    part: part.reference.clone(),
                    key: SIGNAL.to_string(),
                },
                part.props.get(SIGNAL).cloned().unwrap_or_default(),
            ));
        }
        for (key, text) in &part.props {
            if is_reading(key) {
                out.push((
                    Source {
                        part: part.reference.clone(),
                        key: key.clone(),
                    },
                    text.clone(),
                ));
            }
        }
    }
    out
}

fn is_reading(key: &str) -> bool {
    key.starts_with(SIGNAL_OF) && key != SIGNAL_RATE && key.len() > SIGNAL_OF.len()
}

/// One loop of what a source plays, rendered exactly as a run renders the
/// table: the same rate, the same loop and the same seed.
#[derive(Clone, Debug, PartialEq)]
pub struct Played {
    pub source: Source,
    pub signal: Signal,
    pub rate: u32,
    pub samples: Vec<f64>,
    /// What the table plays on when the lab can say from the sheet alone:
    /// a sensor's device. A generator's pins are whichever converters the
    /// circuit carries it to, which the emulator's own reports name.
    pub target: Option<WaveTarget>,
}

/// What `source` plays on `sheet`, with `texts` standing in for the sheet's
/// own where the lab changed one in this run.
///
/// A generator's loop is the one every generator on the sheet shares, at
/// the fastest rate any of them asks for, because that is how a run plays
/// them; a sensor's is the one its own readings share.
pub fn played(
    sheet: &Sheet,
    source: &Source,
    texts: &BTreeMap<Source, String>,
) -> Result<Played, Unplayable> {
    let text_of = |part: &Instance, key: &str| -> Option<String> {
        let own = Source {
            part: part.reference.clone(),
            key: key.to_string(),
        };
        texts
            .get(&own)
            .cloned()
            .or_else(|| part.props.get(key).cloned())
    };
    let read = |text: &str| Signal::parse(text).map_err(Unplayable::Signal);
    let Some(part) = sheet.parts.iter().find(|p| p.reference == source.part) else {
        return Err(Unplayable::Gone(source.part.clone()));
    };

    let (rate, looped, target) = if source.key == SIGNAL {
        let mut rate = 0u32;
        let mut signals = Vec::new();
        for (other, _) in sources_of(sheet)
            .into_iter()
            .filter(|(other, _)| other.key == SIGNAL)
        {
            let Some(generator) = sheet.parts.iter().find(|p| p.reference == other.part) else {
                continue;
            };
            let Ok(own) = generator::rate_at(generator, "rate", GENERATOR_RATE) else {
                continue;
            };
            rate = rate.max(own);
            if let Ok(signal) = read(&text_of(generator, SIGNAL).unwrap_or_default()) {
                signals.push(signal);
            }
        }
        (rate.max(1), generator::loop_of(&signals, rate.max(1)), None)
    } else {
        let rate = generator::rate_at(part, SIGNAL_RATE, SENSOR_RATE).map_err(Unplayable::Rate)?;
        let signals: Vec<Signal> = part
            .props
            .keys()
            .filter(|key| is_reading(key))
            .filter_map(|key| read(&text_of(part, key).unwrap_or_default()).ok())
            .collect();
        let address = part
            .props
            .get("addr")
            .and_then(|text| crate::nets::hex_address(text));
        (
            rate,
            generator::loop_of(&signals, rate),
            address.map(WaveTarget::Device),
        )
    };
    let signal = read(&text_of(part, &source.key).unwrap_or_default())?;
    let samples = signal.render(
        f64::from(rate),
        looped.samples,
        generator::seed_of(part, &source.key),
    );
    Ok(Played {
        source: source.clone(),
        signal,
        rate,
        samples,
        target,
    })
}

/// Where the played loop's sample 0 sits on the emulator's clock, from its
/// own account of switching the table on: a sensor's by its device, a
/// generator's by whichever pin it plays on — one generator's tables start
/// together, in one write, before the guest has run an instruction.
pub fn start_of(played: &Played, switched: &BTreeMap<WaveTarget, Switched>) -> Option<u64> {
    match played.target {
        Some(target) => switched.get(&target)?.start_us,
        None => switched.iter().find_map(|(target, switch)| {
            matches!(target, WaveTarget::Pin(_))
                .then_some(switch.start_us)
                .flatten()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pin, PinKind, Symbol};

    fn symbol(library: &str, name: &str) -> Symbol {
        Symbol {
            library: library.into(),
            name: name.into(),
            reference: "X".into(),
            value: name.into(),
            description: None,
            pins: vec![Pin {
                number: "1".into(),
                name: "OUT".into(),
                kind: PinKind::Passive,
                at: (0.0, 0.0),
                length: 2.54,
                angle: 0,
                hidden: false,
            }],
            graphics: Vec::new(),
        }
    }

    fn part(reference: &str, symbol: &str, props: &[(&str, &str)]) -> Instance {
        Instance {
            reference: reference.into(),
            symbol: symbol.into(),
            value: String::new(),
            x: 0.0,
            y: 0.0,
            rot: 0,
            mirror: false,
            props: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn sheet() -> Sheet {
        let mut sheet = Sheet::empty("esp32c3");
        sheet.symbols = vec![symbol("rusty", "SignalGen"), symbol("rusty", "Sensor")];
        sheet.parts = vec![
            part("V1", "rusty:SignalGen", &[("signal", "sine f=50 a=1")]),
            part("V2", "rusty:SignalGen", &[]),
            part(
                "U2",
                "rusty:Sensor",
                &[
                    ("model", "mpu6050"),
                    ("addr", "68"),
                    ("signal.gz", "sine f=2 a=100"),
                    ("signal.rate", "500"),
                ],
            ),
        ];
        sheet
    }

    /// Every generator is a source, set or not, and every reading a signal
    /// moves; the rate kept beside the readings is not a reading.
    #[test]
    fn the_sheets_signals_are_its_generators_and_moving_readings() {
        let found: Vec<(String, String)> = sources_of(&sheet())
            .into_iter()
            .map(|(source, text)| (source.label(), text))
            .collect();
        assert_eq!(
            found,
            [
                ("V1".to_string(), "sine f=50 a=1".to_string()),
                ("V2".to_string(), String::new()),
                ("U2 · gz".to_string(), "sine f=2 a=100".to_string()),
            ]
        );
    }

    /// What the lab renders is what a run plays: the generators'
    /// shared loop at their rate, and a sensor's at its own, with the
    /// sensor's device as the target — and a text the lab changed stands
    /// in for the sheet's.
    #[test]
    fn a_source_plays_what_a_run_plays() {
        let sheet = sheet();
        let v1 = Source {
            part: "V1".into(),
            key: SIGNAL.into(),
        };
        let played_v1 = played(&sheet, &v1, &BTreeMap::new()).unwrap();
        assert_eq!(played_v1.rate, GENERATOR_RATE);
        assert_eq!(played_v1.samples.len(), GENERATOR_RATE as usize);
        let rendered = Signal::parse("sine f=50 a=1").unwrap().render(
            f64::from(GENERATOR_RATE),
            GENERATOR_RATE as usize,
            generator::seed_of(&sheet.parts[0], SIGNAL),
        );
        assert_eq!(played_v1.samples, rendered);

        let gz = Source {
            part: "U2".into(),
            key: "signal.gz".into(),
        };
        let played_gz = played(&sheet, &gz, &BTreeMap::new()).unwrap();
        assert_eq!(played_gz.rate, 500);
        assert_eq!(played_gz.target, Some(WaveTarget::Device(0x68)));

        let changed = BTreeMap::from([(v1.clone(), "dc 2".to_string())]);
        let now = played(&sheet, &v1, &changed).unwrap();
        assert!(now.samples.iter().all(|v| *v == 2.0));

        let broken = BTreeMap::from([(v1.clone(), "sine fq=5".to_string())]);
        assert!(matches!(
            played(&sheet, &v1, &broken),
            Err(Unplayable::Signal(_))
        ));
    }

    fn tone_played(rate: u32) -> Played {
        let signal = Signal::parse("sine f=10 a=1").unwrap();
        Played {
            source: Source {
                part: "V1".into(),
                key: "signal".into(),
            },
            samples: signal.render(f64::from(rate), rate as usize, 0),
            signal,
            rate,
            target: None,
        }
    }

    /// A generator is lined up by any pin it plays on, a sensor by its own
    /// device, and nothing is lined up once it stopped.
    #[test]
    fn a_loop_is_lined_up_by_the_emulators_own_account() {
        let played = tone_played(1000);
        let mut switched = BTreeMap::new();
        assert_eq!(start_of(&played, &switched), None);
        switched.insert(
            WaveTarget::Pin(3),
            Switched {
                at_us: 10,
                start_us: Some(10),
            },
        );
        assert_eq!(start_of(&played, &switched), Some(10));
        let mut sensor = tone_played(1000);
        sensor.target = Some(WaveTarget::Device(0x68));
        assert_eq!(start_of(&sensor, &switched), None);
        switched.insert(
            WaveTarget::Device(0x68),
            Switched {
                at_us: 20,
                start_us: Some(20),
            },
        );
        assert_eq!(start_of(&sensor, &switched), Some(20));
    }

    #[test]
    fn a_record_keeps_the_newest_conversions() {
        let mut record = Vec::new();
        for n in 0..(CONVERSIONS_KEPT as u64 + 10) {
            record_conversion(&mut record, n, 1);
        }
        assert_eq!(record.len(), CONVERSIONS_KEPT);
        assert_eq!(record.first().map(|(at, _)| *at), Some(10));
    }

    #[test]
    fn a_source_is_named_by_its_part_and_reading() {
        let generator = Source {
            part: "V1".into(),
            key: "signal".into(),
        };
        let reading = Source {
            part: "U2".into(),
            key: "signal.gz".into(),
        };
        assert_eq!(generator.label(), "V1");
        assert_eq!(reading.label(), "U2 · gz");
    }
}
