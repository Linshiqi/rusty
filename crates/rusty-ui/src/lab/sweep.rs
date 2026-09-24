//! A frequency response measured a tone at a time: the frequencies a sweep
//! steps through, how long each step lets the filter settle and then
//! listens, and what one step measured — the tone going in and the tone
//! coming out, compared at one instant.
//!
//! Measured and not computed, because what is being judged is the filter
//! the firmware actually runs, at the rate it actually samples, with every
//! rounding it actually does: a design's own curve is the other half of the
//! chart, and the two disagreeing is the finding.

use std::f64::consts::PI;

use rusty_embed::dsp;
use rusty_embed::signal::{Component, Signal};

use super::record::Record;

/// What a sweep asks for: `points` tones from `from` to `to` hertz, spaced
/// evenly on a logarithmic axis, each `amplitude` either side of `offset`
/// in the source's own units.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub from: f64,
    pub to: f64,
    pub points: usize,
    pub amplitude: f64,
    pub offset: f64,
}

impl Default for Plan {
    /// A decade and more of a slow sensor's band, a volt either side of the
    /// middle of an ESP32-C3's converter at its widest attenuation.
    fn default() -> Self {
        Plan {
            from: 1.0,
            to: 100.0,
            points: 12,
            amplitude: 0.5,
            offset: 1.25,
        }
    }
}

/// What one step measured: the output's tone over the input's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub freq: f64,
    /// Amplitude out over amplitude in.
    pub gain: f64,
    /// Radians, in `(−π, π]`: negative is a lag, as a low-pass has.
    pub phase: f64,
}

/// Why a step measured nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum Missed {
    /// A record too short, or a frequency out of its range.
    Tone(dsp::Refusal),
    /// Nothing of the tone in the input: a gain against it would be a
    /// division by nothing.
    Silent,
}

/// The frequencies a sweep steps through, both ends included and each the
/// same ratio from the last — to three significant figures, which is as
/// finely as a response is read and what the step's signal then says:
/// `f=100.00000000000004` is the logarithm's rounding and nothing anybody
/// asked for. A plan that is not one — no points, an end that is not a
/// positive frequency — steps through nothing.
pub fn frequencies(plan: &Plan) -> Vec<f64> {
    let good = |f: f64| f.is_finite() && f > 0.0;
    if plan.points == 0 || !good(plan.from) || !good(plan.to) {
        return Vec::new();
    }
    if plan.points == 1 {
        return vec![figures(plan.from)];
    }
    let ratio = (plan.to / plan.from).ln() / (plan.points - 1) as f64;
    (0..plan.points)
        .map(|n| figures(plan.from * (ratio * n as f64).exp()))
        .collect()
}

/// `value` to three significant figures.
fn figures(value: f64) -> f64 {
    let scale = 10f64.powi(2 - value.abs().log10().floor() as i32);
    (value * scale).round() / scale
}

/// The signal one step plays: the offset and a sine at `freq`, written as
/// the text form writes it.
pub fn step_signal(plan: &Plan, freq: f64) -> String {
    Signal {
        components: vec![
            Component::Dc { level: plan.offset },
            Component::Sine {
                freq,
                amplitude: plan.amplitude,
                phase: 0.0,
            },
        ],
    }
    .to_string()
}

/// How long a step lets the filter settle before listening: five periods,
/// and never less than a fifth of a second — a filter's own transient from
/// the change of tone has to be over, and a slow filter's is longer than a
/// fast tone's periods.
pub fn settle_seconds(freq: f64) -> f64 {
    (5.0 / freq).max(0.2)
}

/// And how long it listens: ten periods, and never less than half a second,
/// so a fast tone is still heard over enough samples to fit.
pub fn listen_seconds(freq: f64) -> f64 {
    (10.0 / freq).max(0.5)
}

/// The tone at `freq` in `input` and in `output`, each carried to the
/// instant `at_us` — the two records need not begin on the same sample,
/// and a phase compared across two different instants is a phase error of
/// `2πf` times the gap.
pub fn measure(input: &Record, output: &Record, freq: f64, at_us: u64) -> Result<Point, Missed> {
    let going_in = dsp::tone(&input.samples, input.rate, freq).map_err(Missed::Tone)?;
    let coming_out = dsp::tone(&output.samples, output.rate, freq).map_err(Missed::Tone)?;
    if going_in.amplitude <= f64::EPSILON * (1.0 + coming_out.amplitude) {
        return Err(Missed::Silent);
    }
    // `amplitude · sin(2πf(t − t0) + phase)` is, at `t = at`, a phase of
    // `phase + 2πf(at − t0)`.
    let carried = |record: &Record, phase: f64| {
        phase + 2.0 * PI * freq * (at_us as f64 - record.start_us as f64) / 1e6
    };
    Ok(Point {
        freq,
        gain: coming_out.amplitude / going_in.amplitude,
        phase: wrap(carried(output, coming_out.phase) - carried(input, going_in.phase)),
    })
}

/// A phase into `(−π, π]`.
pub fn wrap(phase: f64) -> f64 {
    let turned = phase.rem_euclid(2.0 * PI);
    if turned > PI {
        turned - 2.0 * PI
    } else {
        turned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(start_us: u64, rate: f64, seconds: f64, f: impl Fn(f64) -> f64) -> Record {
        let count = (rate * seconds) as usize;
        Record {
            rate,
            start_us,
            samples: (0..count)
                .map(|n| f(start_us as f64 / 1e6 + n as f64 / rate))
                .collect(),
        }
    }

    #[test]
    fn a_sweep_steps_by_equal_ratios() {
        let steps = frequencies(&Plan {
            from: 1.0,
            to: 1000.0,
            points: 4,
            ..Plan::default()
        });
        assert_eq!(steps, [1.0, 10.0, 100.0, 1000.0]);
        let twelve = frequencies(&Plan::default());
        assert_eq!(twelve.first(), Some(&1.0));
        assert_eq!(twelve.last(), Some(&100.0));
        assert_eq!(twelve[1], 1.52, "{twelve:?}");
        let text = step_signal(&Plan::default(), twelve[11]);
        assert!(text.contains("f=100 "), "{text}");
        assert!(
            frequencies(&Plan {
                points: 0,
                ..Plan::default()
            })
            .is_empty()
        );
        assert!(
            frequencies(&Plan {
                from: 0.0,
                ..Plan::default()
            })
            .is_empty()
        );
    }

    /// A step plays what the text form reads back as the same signal.
    #[test]
    fn a_step_plays_its_tone_on_the_offset() {
        let text = step_signal(&Plan::default(), 12.5);
        let read = Signal::parse(&text).unwrap();
        assert_eq!(read.to_string(), text);
        assert!(text.contains("f=12.5"), "{text}");
    }

    /// A filter that halves a tone and delays it by a quarter period reads
    /// as −6 dB and −90°, whatever sample each record begins on and at
    /// whatever rate each was taken.
    #[test]
    fn a_step_measures_the_gain_and_the_lag() {
        let freq = 5.0;
        let lag = PI / 2.0;
        let input = record(1_000_000, 1000.0, 2.0, |t| {
            1.2 + 0.5 * (2.0 * PI * freq * t).sin()
        });
        // Printed at 200 Hz and beginning 7 ms later.
        let output = record(1_007_000, 200.0, 2.0, |t| {
            0.3 + 0.25 * (2.0 * PI * freq * t - lag).sin()
        });
        let point = measure(&input, &output, freq, 1_500_000).unwrap();
        assert!((point.gain - 0.5).abs() < 1e-6, "{point:?}");
        assert!((point.phase + lag).abs() < 1e-6, "{point:?}");
    }

    #[test]
    fn a_silent_input_measures_nothing() {
        let input = record(0, 1000.0, 1.0, |_| 1.0);
        let output = record(0, 1000.0, 1.0, |t| (2.0 * PI * 5.0 * t).sin());
        assert_eq!(measure(&input, &output, 5.0, 0), Err(Missed::Silent));
    }

    #[test]
    fn a_phase_wraps_into_one_turn() {
        assert!((wrap(3.0 * PI / 2.0) + PI / 2.0).abs() < 1e-12);
        assert!((wrap(-3.0 * PI / 2.0) - PI / 2.0).abs() < 1e-12);
        assert!((wrap(PI) - PI).abs() < 1e-12);
    }
}
