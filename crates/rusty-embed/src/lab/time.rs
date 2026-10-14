//! The Time view's placements: what a played table and a converter's
//! reports put on the firmware's clock between two moments.

use super::Played;

/// The played signal across `from..=to` µs: each of the table's samples in
/// that span, at the instant it plays, the loop repeating from `start_us`.
pub fn played_points(played: &Played, start_us: u64, from: u64, to: u64) -> Vec<(u64, f64)> {
    let len = played.samples.len() as u64;
    if len == 0 || to <= from || to < start_us {
        return Vec::new();
    }
    let rate = f64::from(played.rate.max(1));
    let first = ((from.saturating_sub(start_us)) as f64 * rate / 1e6).ceil() as u64;
    let last = ((to - start_us) as f64 * rate / 1e6).floor() as u64;
    if last < first {
        return Vec::new();
    }
    // A window of minutes at a fast rate is millions of samples; every one
    // would be thinned to two a column anyway.
    let stride = ((last - first) / 400_000).max(1) as usize;
    (first..=last)
        .step_by(stride)
        .map(|n| {
            let at = start_us + (n as f64 * 1e6 / rate).round() as u64;
            (at, played.samples[(n % len) as usize])
        })
        .collect()
}

/// A pin's conversions across `from..=to` as the steps they are: each one
/// holds until the next, and the one before the window is what the window
/// opens on. The emulator reports a conversion only when it changed, so a
/// still stretch is a single report and must be drawn as a level, not as a
/// line to the next change.
pub fn conversion_steps(record: &[(u64, u16)], from: u64, to: u64) -> Vec<(u64, f64)> {
    let begin = record.partition_point(|(at, _)| *at < from);
    let mut held = begin.checked_sub(1).map(|i| f64::from(record[i].1));
    let mut out = Vec::new();
    if let Some(value) = held {
        out.push((from, value));
    }
    for &(at, counts) in record[begin..].iter().take_while(|(at, _)| *at <= to) {
        let value = f64::from(counts);
        if let Some(before) = held {
            out.push((at, before));
        }
        out.push((at, value));
        held = Some(value);
    }
    if let Some(value) = held {
        out.push((to, value));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::Source;
    use crate::signal::Signal;

    fn tone(rate: u32) -> Played {
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

    /// Sample `n` of the loop plays at `start + n / rate`, and the loop
    /// repeats: the window a second and a quarter in reads the samples a
    /// quarter of the way through the table.
    #[test]
    fn the_played_signal_is_placed_where_the_emulator_played_it() {
        let played = tone(1000);
        let start = 5_000_000;
        let points = played_points(&played, start, start + 1_250_000, start + 1_252_000);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].0, start + 1_250_000);
        assert_eq!(points[0].1, played.samples[250]);
        assert_eq!(points[2].1, played.samples[252]);
        // Before the table started there is nothing to draw.
        assert!(played_points(&played, start, 0, start - 1).is_empty());
    }

    /// Conversions are levels that hold until the next change, and the one
    /// before the window is what the window opens on.
    #[test]
    fn conversions_are_drawn_as_the_steps_they_are() {
        let record = [(100, 5), (200, 7), (300, 9)];
        let steps = conversion_steps(&record, 150, 250);
        assert_eq!(steps, [(150, 5.0), (200, 5.0), (200, 7.0), (250, 7.0)]);
        assert!(conversion_steps(&record, 0, 50).is_empty());
    }
}
