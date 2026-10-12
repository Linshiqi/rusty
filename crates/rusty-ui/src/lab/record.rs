//! A record: evenly spaced samples at a known rate, which is what every
//! instrument in the lab reads — a spectrum, a tone, a filter run over it.
//!
//! Three things become one. A telemetry channel is already a record, one
//! sample a line, and its rate is read off the firmware's own stamps. A
//! converter's reports are a record with the repeats left out — the
//! emulator says a conversion only when it changed — so they are put back
//! by holding each value until the next, on the interval the reports
//! themselves keep. And what a source played is the table itself, taken at
//! whatever rate the question asks for, the way the converter took it.

use std::collections::BTreeMap;

use super::Played;

/// Evenly spaced samples, the first at `start_us` on the firmware's clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub rate: f64,
    pub start_us: u64,
    pub samples: Vec<f64>,
}

impl Record {
    /// The part of it from `from` µs on, `seconds` long at most.
    pub fn between(&self, from: u64, seconds: f64) -> Record {
        let skip = ((from.saturating_sub(self.start_us)) as f64 * self.rate / 1e6).ceil() as usize;
        let keep = (seconds * self.rate).floor() as usize;
        let samples: Vec<f64> = self.samples.iter().skip(skip).take(keep).copied().collect();
        Record {
            rate: self.rate,
            start_us: self.start_us + (skip as f64 * 1e6 / self.rate).round() as u64,
            samples,
        }
    }
}

/// Which record an instrument reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Of {
    /// What the studied source plays.
    Played,
    /// A converter's reports on a pin.
    Converter(u8),
    /// A channel the firmware prints.
    Channel(String),
}

/// A telemetry channel as a record: one sample a line, at the rate its
/// stamps keep over the span. `None` for fewer than two samples or stamps
/// that do not move, which have no rate to speak of.
pub fn from_channel(points: &[(u64, f32)]) -> Option<Record> {
    let (first, last) = (points.first()?.0, points.last()?.0);
    if points.len() < 2 || last <= first {
        return None;
    }
    let rate = (points.len() - 1) as f64 * 1e6 / (last - first) as f64;
    Some(Record {
        rate,
        start_us: first,
        samples: points.iter().map(|(_, v)| f64::from(*v)).collect(),
    })
}

/// A converter's reports as a record: the interval the reports keep —
/// the commonest gap between two of them, which with a signal playing is
/// the firmware's own sampling interval — and every value held until the
/// next, which is what a report left out meant.
pub fn from_conversions(record: &[(u64, u16)]) -> Option<Record> {
    if record.len() < 3 {
        return None;
    }
    let mut gaps: BTreeMap<u64, usize> = BTreeMap::new();
    for pair in record.windows(2) {
        let gap = pair[1].0.saturating_sub(pair[0].0);
        if gap > 0 {
            *gaps.entry(gap).or_default() += 1;
        }
    }
    let (&interval, _) = gaps
        .iter()
        .max_by_key(|(gap, count)| (**count, std::cmp::Reverse(**gap)))?;
    let (first, last) = (record[0].0, record[record.len() - 1].0);
    let count = ((last - first) / interval + 1) as usize;
    let mut samples = Vec::with_capacity(count);
    let mut next = 0usize;
    let mut held = f64::from(record[0].1);
    for n in 0..count {
        let at = first + n as u64 * interval;
        while next < record.len() && record[next].0 <= at {
            held = f64::from(record[next].1);
            next += 1;
        }
        samples.push(held);
    }
    Some(Record {
        rate: 1e6 / interval as f64,
        start_us: first,
        samples,
    })
}

/// What `played` plays, taken `rate` times a second for `seconds` from
/// `from` µs, its loop repeating from `start_us` — straight lines between
/// the table's samples, as the emulator reads a pin between them.
pub fn from_played(played: &Played, start_us: u64, from: u64, rate: f64, seconds: f64) -> Record {
    let len = played.samples.len();
    let count = (seconds * rate).floor().max(0.0) as usize;
    let table = f64::from(played.rate.max(1));
    let samples = (0..count)
        .map(|n| {
            if len == 0 {
                return 0.0;
            }
            let at = from as f64 + n as f64 * 1e6 / rate;
            let position = ((at - start_us as f64) * table / 1e6).max(0.0);
            let whole = position.floor();
            let fraction = position - whole;
            let here = (whole as usize) % len;
            let next = (here + 1) % len;
            played.samples[here] * (1.0 - fraction) + played.samples[next] * fraction
        })
        .collect();
    Record {
        rate,
        start_us: from,
        samples,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Source;
    use rusty_embed::signal::Signal;

    /// A channel printed a thousand times a second reads as a kilohertz
    /// record, whatever it holds.
    #[test]
    fn a_channel_keeps_the_rate_of_its_stamps() {
        let points: Vec<(u64, f32)> = (0..500).map(|n| (10_000 + n * 1000, n as f32)).collect();
        let record = from_channel(&points).unwrap();
        assert!((record.rate - 1000.0).abs() < 1e-9);
        assert_eq!(record.samples.len(), 500);
        assert_eq!(record.start_us, 10_000);
        assert!(from_channel(&points[..1]).is_none());
    }

    /// Reports left out because nothing changed come back as the value
    /// held, on the interval the other reports keep.
    #[test]
    fn a_converter_record_puts_its_repeats_back() {
        // Every millisecond, but the value repeated at 3 ms and 4 ms and
        // was not reported.
        let record = [(0, 10), (1000, 11), (2000, 12), (5000, 13), (6000, 14)];
        let rebuilt = from_conversions(&record).unwrap();
        assert!((rebuilt.rate - 1000.0).abs() < 1e-9);
        assert_eq!(rebuilt.samples, [10.0, 11.0, 12.0, 12.0, 12.0, 13.0, 14.0]);
    }

    /// Taken at the firmware's rate, the played table is what the emulator
    /// hands the converter: sample for sample where the instants coincide,
    /// and on the straight line between two samples where they do not.
    #[test]
    fn a_played_signal_is_taken_as_the_converter_takes_it() {
        let signal = Signal::parse("sine f=10 a=1").unwrap();
        let played = Played {
            source: Source {
                part: "V1".into(),
                key: "signal".into(),
            },
            samples: signal.render(1000.0, 1000, 0),
            signal,
            rate: 1000,
            target: None,
        };
        let start = 1_000_000;
        // At 500 Hz every other table sample, starting a quarter second in.
        let record = from_played(&played, start, start + 250_000, 500.0, 0.1);
        assert_eq!(record.samples.len(), 50);
        assert!((record.samples[0] - played.samples[250]).abs() < 1e-12);
        assert!((record.samples[1] - played.samples[252]).abs() < 1e-12);
        // Halfway between two samples, halfway between their values.
        let between = from_played(&played, start, start + 500, 1000.0, 0.001);
        let want = (played.samples[0] + played.samples[1]) / 2.0;
        assert!((between.samples[0] - want).abs() < 1e-12);
    }

    #[test]
    fn a_record_is_cut_to_the_span_asked_for() {
        let record = Record {
            rate: 1000.0,
            start_us: 0,
            samples: (0..1000).map(f64::from).collect(),
        };
        let part = record.between(250_000, 0.1);
        assert_eq!(part.samples.len(), 100);
        assert_eq!(part.samples[0], 250.0);
        assert_eq!(part.start_us, 250_000);
    }
}
