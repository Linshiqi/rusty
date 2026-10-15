//! Pin waveforms: the trace a run's `[rusty:gpio]` reports make, and the
//! arithmetic of drawing it — the Waves panel's.
//!
//! One clock per trace: the firmware's systimer when the first report was
//! stamped, the host's arrival time when it was not, and never both,
//! because a trace that mixes time bases is a trace that lies.

use crate::{plot::TraceClock, protocol::GpioReport};

/// Edges kept: an hour of simulation must not eat the window.
pub const CAP: usize = 200_000;

/// A captured pin trace: time-ordered `(µs, pin, level)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trace {
    pub events: Vec<(u64, u8, bool)>,
    pub clock: Option<TraceClock>,
    /// True when the cap was hit and the oldest events were dropped.
    pub truncated: bool,
}

impl Trace {
    /// One gpio report into the trace, stamped on the clock the trace took
    /// at its first report — `host_us` when that is the host's. A late
    /// unstamped report in a firmware-clocked trace takes the last moment
    /// already there, which keeps the event without inventing a time.
    pub fn record(&mut self, report: GpioReport, host_us: u64) {
        let clock = *self.clock.get_or_insert(if report.at_us.is_some() {
            TraceClock::Firmware
        } else {
            TraceClock::Host
        });
        let at = match (clock, report.at_us) {
            (TraceClock::Firmware, Some(us)) => us,
            (TraceClock::Firmware, None) => self.events.last().map(|(t, ..)| *t).unwrap_or(0),
            (TraceClock::Host, _) => host_us,
        };
        for (pin, level) in report.pins {
            self.events.push((at, pin, level));
        }
        if self.events.len() > CAP {
            let drop = self.events.len() - CAP;
            self.events.drain(..drop);
            self.truncated = true;
        }
    }

    /// The pins in the order they first moved: one lane each.
    pub fn pins(&self) -> Vec<u8> {
        let mut pins: Vec<u8> = Vec::new();
        for (_, pin, _) in &self.events {
            if !pins.contains(pin) {
                pins.push(*pin);
            }
        }
        pins
    }

    /// The first and last moments in the trace.
    pub fn span(&self) -> Option<(u64, u64)> {
        Some((self.events.first()?.0, self.events.last()?.0))
    }

    /// One lane's steps over `start..end` µs, drawn `width` units wide with
    /// high at `high` and low at `low`: the level before the window, each
    /// change inside it, and on to the right edge. A change finer than a
    /// unit is still drawn, as the step it is.
    pub fn lane(&self, pin: u8, start: f64, end: f64, width: f64, high: f64, low: f64) -> String {
        let per_unit = (end - start).max(f64::MIN_POSITIVE) / width;
        // Events are in time order: the window is found by halving rather
        // than by walking two hundred thousand edges per lane per frame.
        let first = self.events.partition_point(|(t, ..)| (*t as f64) < start);
        let mut level = self.events[..first]
            .iter()
            .rev()
            .find(|(_, p, _)| *p == pin)
            .map(|(_, _, l)| *l)
            .unwrap_or(false);
        let y = |level: bool| if level { high } else { low };
        let mut path = format!("M 0 {:.1}", y(level));
        for (t, p, l) in &self.events[first..] {
            if (*t as f64) > end {
                break;
            }
            if *p != pin || *l == level {
                continue;
            }
            let x = ((*t as f64) - start) / per_unit;
            path.push_str(&format!(" H {x:.2} V {:.1}", y(*l)));
            level = *l;
        }
        path.push_str(&format!(" H {width:.2}"));
        path
    }

    /// A pin's level at `at` µs: what it last changed to.
    pub fn level_at(&self, pin: u8, at: f64) -> Option<bool> {
        let upto = self.events.partition_point(|(t, ..)| (*t as f64) <= at);
        self.events[..upto]
            .iter()
            .rev()
            .find(|(_, p, _)| *p == pin)
            .map(|(_, _, l)| *l)
    }
}

/// The tick spacing for a scale of `per_px` µs a pixel: the first 1, 2 or
/// 5 × 10ⁿ at least a hundred pixels wide.
pub fn tick_step(per_px: f64) -> f64 {
    let raw = per_px * 100.0;
    let decade = 10f64.powf(raw.log10().floor());
    [decade, decade * 2.0, decade * 5.0, decade * 10.0]
        .into_iter()
        .find(|s| *s >= raw)
        .unwrap_or(decade)
}

/// Every multiple of `step` inside `start..=end`, capped so a runaway scale
/// cannot draw thousands of labels.
pub fn ticks_between(start: f64, end: f64, step: f64) -> Vec<f64> {
    let first = (start / step).ceil() * step;
    (0..)
        .map(|i| first + i as f64 * step)
        .take_while(|t| *t <= end)
        .take(40)
        .collect()
}

/// A tick label in the unit the scale calls for: seconds once ticks are a
/// second apart (or the time is past ten), milliseconds from a millisecond
/// step, microseconds below that.
pub fn format_us(step: f64, us: f64) -> String {
    if step >= 1_000_000.0 || us.abs() >= 10_000_000.0 {
        format!("{:.2}s", us / 1_000_000.0)
    } else if step >= 1_000.0 {
        format!("{:.1}ms", us / 1_000.0)
    } else {
        format!("{us:.0}µs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(at_us: Option<u64>, pins: &[(u8, bool)]) -> GpioReport {
        GpioReport {
            at_us,
            pins: pins.to_vec(),
        }
    }

    /// Ticks land on 1, 2, 5 × 10ⁿ and never closer than a hundred pixels:
    /// a scale of 3 µs/px wants a step of at least 300 µs, and the nearest
    /// round step above that is 500.
    #[test]
    fn ticks_are_round_and_at_least_a_hundred_pixels_apart() {
        assert_eq!(tick_step(3.0), 500.0);
        assert_eq!(tick_step(1.0), 100.0);
        assert_eq!(tick_step(0.15), 20.0);
        assert_eq!(tick_step(12.0), 2000.0);
    }

    /// The first tick is the first multiple at or after the window's start,
    /// and none lands past its end.
    #[test]
    fn ticks_start_on_a_multiple_inside_the_window() {
        assert_eq!(
            ticks_between(130.0, 700.0, 100.0),
            vec![200.0, 300.0, 400.0, 500.0, 600.0, 700.0]
        );
        assert_eq!(ticks_between(0.0, 250.0, 100.0), vec![0.0, 100.0, 200.0]);
    }

    /// A window many steps wide is capped rather than labelled to death.
    #[test]
    fn a_runaway_window_is_capped() {
        assert_eq!(ticks_between(0.0, 1.0e9, 1.0).len(), 40);
    }

    /// The unit follows the step, so a millisecond axis does not read
    /// "1000µs, 2000µs".
    #[test]
    fn labels_take_the_unit_of_the_step() {
        assert_eq!(format_us(100.0, 300.0), "300µs");
        assert_eq!(format_us(1_000.0, 2_500.0), "2.5ms");
        assert_eq!(format_us(1_000_000.0, 1_500_000.0), "1.50s");
        // A late time reads in seconds whatever the step.
        assert_eq!(format_us(1_000.0, 12_000_000.0), "12.00s");
    }

    /// A trace that began on the firmware's clock stays on it: an
    /// unstamped report takes the last moment there, not the host's time.
    #[test]
    fn a_trace_keeps_the_clock_it_began_on() {
        let mut trace = Trace::default();
        trace.record(report(Some(500), &[(2, true)]), 9_999_999);
        trace.record(report(None, &[(2, false)]), 9_999_999);
        assert_eq!(trace.clock, Some(TraceClock::Firmware));
        assert_eq!(trace.events, vec![(500, 2, true), (500, 2, false)]);
    }

    /// A lane starts at the level the pin had before the window and steps
    /// at each change inside it, in the window's own units.
    #[test]
    fn a_lane_steps_where_its_pin_changed() {
        let mut trace = Trace::default();
        trace.record(report(Some(0), &[(2, true), (3, false)]), 0);
        trace.record(report(Some(1_000), &[(2, false)]), 0);
        trace.record(report(Some(1_500), &[(3, true)]), 0);
        trace.record(report(Some(3_000), &[(2, true)]), 0);
        // 500..2500 µs over 100 units: the fall at 1000 is 25 units in, and
        // the rise at 3000 is past the window.
        assert_eq!(
            trace.lane(2, 500.0, 2_500.0, 100.0, 4.0, 26.0),
            "M 0 4.0 H 25.00 V 26.0 H 100.00"
        );
        assert_eq!(trace.pins(), vec![2, 3]);
        assert_eq!(trace.level_at(3, 1_499.0), Some(false));
        assert_eq!(trace.level_at(3, 1_500.0), Some(true));
        assert_eq!(trace.level_at(9, 1_500.0), None);
    }

    /// The oldest edges go first once the cap is reached, and the trace
    /// says it dropped them.
    #[test]
    fn the_oldest_edges_go_past_the_cap() {
        let mut trace = Trace::default();
        for at in 0..(CAP as u64 + 10) {
            trace.record(report(Some(at), &[(1, at % 2 == 0)]), 0);
        }
        assert_eq!(trace.events.len(), CAP);
        assert_eq!(trace.events[0].0, 10);
        assert!(trace.truncated);
    }
}
