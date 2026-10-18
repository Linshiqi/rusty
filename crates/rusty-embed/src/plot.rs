//! Named numeric channels over time, as the firmware prints them on its
//! `[rusty:tel]` line, and the range each is drawn against — the Plot
//! panel's arithmetic.

use crate::protocol::Telemetry;

/// Samples kept per channel. A 1 kHz loop fills this in twenty seconds,
/// which is the window a tuning change is judged in; older than that and
/// you re-run rather than scroll back.
pub const CAP: usize = 20_000;

/// Whose clock the trace timestamps are on.
///
/// Mixing the two silently is how a waveform lies, so the panel shows which
/// one it got. Firmware means `[rusty:gpio@µs]` stamps from the systimer;
/// Host means the firmware sent none and arrival time stood in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceClock {
    Firmware,
    Host,
}

impl TraceClock {
    /// The moment a report lands at. A trace takes its clock at its first
    /// report — the firmware's when that one carried a stamp, the host's
    /// otherwise — and from then on a stamp is the firmware's moment, an
    /// unstamped report on a firmware-clocked trace takes `newest`, the
    /// newest moment already in it, rather than mixing the two clocks, and a
    /// host-clocked trace takes `host_us`. The plot and the waves stamp alike.
    pub fn stamp(
        clock: &mut Option<TraceClock>,
        at_us: Option<u64>,
        host_us: u64,
        newest: impl FnOnce() -> u64,
    ) -> u64 {
        let clock = *clock.get_or_insert(if at_us.is_some() {
            TraceClock::Firmware
        } else {
            TraceClock::Host
        });
        match (clock, at_us) {
            (TraceClock::Firmware, Some(us)) => us,
            (TraceClock::Firmware, None) => newest(),
            (TraceClock::Host, _) => host_us,
        }
    }
}

/// Named numeric channels over time — the analog half of the trace.
///
/// Kept per channel rather than as rows of samples because that is how it is
/// drawn and how it arrives: firmware prints whichever channels it has this
/// loop, and a channel that starts appearing halfway through a flight is
/// normal, not a schema change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plot {
    /// Channel name → its samples, `(µs, value)`, oldest first.
    pub channels: Vec<(String, Vec<(u64, f32)>)>,
    pub clock: Option<TraceClock>,
    /// True once the cap started dropping the oldest samples.
    pub truncated: bool,
}

impl Plot {
    /// The samples for a channel, creating it on first sight.
    pub fn channel(&mut self, name: &str) -> &mut Vec<(u64, f32)> {
        if let Some(index) = self.channels.iter().position(|(known, _)| known == name) {
            return &mut self.channels[index].1;
        }
        self.channels.push((name.to_string(), Vec::new()));
        &mut self.channels.last_mut().expect("just pushed").1
    }
}

impl Plot {
    /// A sample, stamped on the clock the trace took at its first sample:
    /// the firmware's when that sample carried a stamp, the host's (`host_us`)
    /// otherwise. A stamp missing later in a firmware-clocked trace takes
    /// the newest time already in it, rather than mixing the two clocks.
    /// Each channel keeps its newest [`CAP`] samples.
    pub fn record(&mut self, sample: Telemetry, host_us: u64) {
        let at = TraceClock::stamp(&mut self.clock, sample.at_us, host_us, || {
            self.channels
                .iter()
                .filter_map(|(_, points)| points.last().map(|(t, _)| *t))
                .max()
                .unwrap_or(0)
        });
        let mut dropped = false;
        for (name, value) in sample.channels {
            let points = self.channel(&name);
            points.push((at, value));
            if points.len() > CAP {
                let over = points.len() - CAP;
                points.drain(..over);
                dropped = true;
            }
        }
        self.truncated |= dropped;
    }
}

/// A channel's smallest drawn swing, as a fraction of its own magnitude.
///
/// Pure autoscale is wrong here and the real board proved it: a loop settled
/// at 88.0 ± 0.5 has a 1% ripple, and scaling that to the full height drew a
/// converged controller as maximum-amplitude static. "Has it settled?" is the
/// single question this panel exists to answer, so a small wobble must *look*
/// small. Five percent: a ripple under a twentieth of the signal is drawn
/// within a twentieth of the height, and anything larger scales normally.
pub const FLOOR: f32 = 0.05;

/// The value range a channel is drawn against, and its midpoint behaviour.
///
/// Returns a band that always has width, so the caller never divides by zero
/// and a constant lands in the middle of the plot rather than on its floor —
/// a setpoint pinned to the bottom edge reads as "at minimum", which is a
/// different claim from "not changing".
pub fn band(points: &[(u64, f32)]) -> (f32, f32) {
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for (_, value) in points {
        low = low.min(*value);
        high = high.max(*value);
    }
    let middle = (low + high) / 2.0;
    let swing = high - low;
    let floor = (middle.abs().max(swing) * FLOOR).max(f32::EPSILON);
    let span = if swing < floor { floor } else { swing };
    (middle - span / 2.0, middle + span / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trace that began on the firmware's clock stays on it: a sample
    /// with no stamp takes the newest time already there, not the host's.
    #[test]
    fn a_trace_keeps_the_clock_it_began_on() {
        let mut plot = Plot::default();
        plot.record(
            Telemetry {
                at_us: Some(1_000),
                channels: vec![("x".into(), 1.0)],
            },
            99_999_999,
        );
        plot.record(
            Telemetry {
                at_us: None,
                channels: vec![("x".into(), 2.0)],
            },
            99_999_999,
        );
        assert_eq!(plot.clock, Some(TraceClock::Firmware));
        assert_eq!(plot.channels[0].1, vec![(1_000, 1.0), (1_000, 2.0)]);
    }

    /// The bug the real board found: a loop settled at 88.0 ± 0.5 was scaled
    /// to the full height and drew a converged controller as maximum-amplitude
    /// static. A ripple that small must occupy a small share of the plot.
    #[test]
    fn a_small_ripple_on_a_large_signal_is_drawn_small() {
        let settled: Vec<(u64, f32)> = (0..20)
            .map(|at| (at, if at % 2 == 0 { 87.5 } else { 88.5 }))
            .collect();
        let (low, high) = band(&settled);
        let share = 1.0 / (high - low);
        assert!(
            share < 0.3,
            "a 1.0 ripple on an 88.0 signal filled {:.0}% of the height",
            share * 100.0
        );
        // Still centred on the data, not shoved to an edge.
        assert!(low < 87.5 && high > 88.5);
    }

    /// A swing worth looking at keeps the whole plot. The floor is a floor,
    /// not a ceiling — clamping a real oscillation would hide the thing the
    /// panel exists to show.
    #[test]
    fn a_real_swing_still_uses_the_full_height() {
        let ringing: Vec<(u64, f32)> = (0..20)
            .map(|at| (at, if at % 2 == 0 { 20.0 } else { 100.0 }))
            .collect();
        let (low, high) = band(&ringing);
        assert_eq!((low, high), (20.0, 100.0));
    }

    /// A constant lands in the middle. On the floor it reads as "at minimum",
    /// which is a different claim from "not changing" — and a setpoint nobody
    /// has touched is the most common constant there is.
    #[test]
    fn a_constant_is_centred_rather_than_on_the_floor() {
        let flat: Vec<(u64, f32)> = (0..10).map(|at| (at, 90.0)).collect();
        let (low, high) = band(&flat);
        assert!(high > low, "a constant must still have a band to divide by");
        let middle = (low + high) / 2.0;
        assert!((middle - 90.0).abs() < 1e-3, "centred on {middle}, not 90");
    }

    /// A channel sitting at zero has no magnitude to take a fraction of, and
    /// `0.0 / 0.0` would put every point at NaN — which SVG renders as a
    /// silently missing curve rather than an error.
    #[test]
    fn a_constant_zero_channel_still_divides() {
        let zeros: Vec<(u64, f32)> = (0..10).map(|at| (at, 0.0)).collect();
        let (low, high) = band(&zeros);
        assert!(high > low);
        let y = (0.0 - low) / (high - low);
        assert!(y.is_finite(), "a zero channel produced {y}");
    }
}
