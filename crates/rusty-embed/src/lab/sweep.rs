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

use crate::dsp;
use crate::signal::{Component, Signal};

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

/// Why two records gave no response.
#[derive(Clone, Debug, PartialEq)]
pub enum Unread {
    /// They share no stretch of time to compare.
    Apart,
    /// Nothing in the input stands out as a tone, so there is nothing to
    /// hold the output's against.
    NoTones,
    /// Every tone found was refused: the first one's reason.
    Missed(Missed),
}

/// Most tones read off one record: a signal a person writes by hand has a
/// few, and past that it is noise being read as tones.
const MOST_TONES: usize = 8;

/// The tones `record` carries, lowest first: every peak of its spectrum that
/// stands out — at least a hundredth of the strongest — and is no closer to
/// a stronger one than the window's own width, the strongest eight. Its
/// mean is taken off first, so a converter's offset is not the strongest
/// thing in it. Through a Blackman window, whose sidelobes sit below that
/// hundredth: through a Hann one, the first sidelobe of a strong tone
/// stands at a thirtieth and would be read as a tone beside it. Each peak
/// is placed between bins by a parabola through the logarithms of it and
/// its neighbours, and given to three figures, as a sweep's steps are.
///
/// Where a peak is placed matters less than it looks: `measure` fits the
/// input and the output at the same frequency, and a filter's output at a
/// tone is the input's times the response there, so the ratio of the two
/// fits is the response whatever frequency both were fitted at.
pub fn tones_in(record: &Record) -> Vec<f64> {
    let n = record.samples.len();
    if n < 8 {
        return Vec::new();
    }
    let mean = record.samples.iter().sum::<f64>() / n as f64;
    let centred: Vec<f64> = record.samples.iter().map(|x| x - mean).collect();
    let spectrum = dsp::spectrum(&centred, record.rate, dsp::Window::Blackman);
    let a = &spectrum.amplitude;
    if a.len() < 4 {
        return Vec::new();
    }
    let bin = spectrum.freqs[1];
    // Blackman's main lobe is three of the record's own bins either side,
    // and the spectrum's bins are finer than the record's by the padding.
    let lobe = 3.5 * record.rate / n as f64;
    let strongest = a[1..].iter().copied().fold(0.0, f64::max);
    if !strongest.is_finite() || strongest <= 0.0 {
        return Vec::new();
    }
    let mut peaks: Vec<(f64, f64)> = (1..a.len() - 1)
        .filter(|&k| a[k] > a[k - 1] && a[k] >= a[k + 1] && a[k] >= strongest / 100.0)
        .filter(|&k| spectrum.freqs[k] > lobe)
        .map(|k| {
            let (l, c, r) = (a[k - 1].ln(), a[k].ln(), a[k + 1].ln());
            let bend = l - 2.0 * c + r;
            let shift = if bend < 0.0 {
                0.5 * (l - r) / bend
            } else {
                0.0
            };
            ((k as f64 + shift) * bin, a[k])
        })
        .collect();
    peaks.sort_by(|x, y| y.1.total_cmp(&x.1));
    let mut kept: Vec<(f64, f64)> = Vec::new();
    for (freq, amplitude) in peaks {
        if kept.len() == MOST_TONES {
            break;
        }
        if freq < record.rate / 2.0 && kept.iter().all(|(f, _)| (f - freq).abs() > lobe) {
            kept.push((freq, amplitude));
        }
    }
    let mut tones: Vec<f64> = kept.into_iter().map(|(freq, _)| figures(freq)).collect();
    tones.sort_by(f64::total_cmp);
    tones
}

/// What a filter did to each tone of a recording: `input` went in and
/// `output` came out, both on one clock — a test's trace, a firmware's
/// telemetry, a converter's reports — and every tone the input carries
/// (`tones_in`) is measured as a sweep's step would be. Over the stretch
/// both records cover, less its first quarter, which is the filter's own
/// start: a filter starting from its first sample, or from zero, is still
/// arriving at the tones there, and a fit across it reads the arrival as
/// part of the response.
pub fn recorded(input: &Record, output: &Record) -> Result<Vec<Point>, Unread> {
    let end = |r: &Record| r.start_us + (r.samples.len() as f64 * 1e6 / r.rate) as u64;
    let from = input.start_us.max(output.start_us);
    let to = end(input).min(end(output));
    if to <= from {
        return Err(Unread::Apart);
    }
    let span = (to - from) as f64 / 1e6;
    let settled = from + ((to - from) / 4);
    let heard = span * 0.75;
    let going_in = input.between(settled, heard);
    let coming_out = output.between(settled, heard);
    let tones = tones_in(&going_in);
    if tones.is_empty() {
        return Err(Unread::NoTones);
    }
    let mut points = Vec::new();
    let mut first_miss = None;
    for freq in tones {
        match measure(&going_in, &coming_out, freq, settled) {
            Ok(point) => points.push(point),
            Err(missed) => {
                first_miss.get_or_insert(missed);
            }
        }
    }
    match first_miss {
        Some(missed) if points.is_empty() => Err(Unread::Missed(missed)),
        _ => Ok(points),
    }
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

    /// An exponential average, run as the filter in a test runs it: the
    /// first sample taken whole, then `y += α(x − y)`.
    fn smoothed(input: &Record, alpha: f64) -> Record {
        let mut y = None;
        let samples = input
            .samples
            .iter()
            .map(|&x| {
                let next = match y {
                    Some(previous) => previous + alpha * (x - previous),
                    None => x,
                };
                y = Some(next);
                next
            })
            .collect();
        Record {
            samples,
            ..input.clone()
        }
    }

    /// `α / (1 − (1 − α)·e^(−jω))`, the average's response, by hand.
    fn smoothed_response(alpha: f64, freq: f64, rate: f64) -> (f64, f64) {
        let w = 2.0 * PI * freq / rate;
        let (re, im) = (1.0 - (1.0 - alpha) * w.cos(), (1.0 - alpha) * w.sin());
        (alpha / re.hypot(im), -im.atan2(re))
    }

    /// The two tones a person writes by hand to try a low-pass are found,
    /// on an offset and under a little noise, and nothing beside them.
    #[test]
    fn the_tones_a_record_carries_are_found_and_nothing_else() {
        let noise = |t: f64| 0.01 * (t * 12_345.678).sin() * (t * 987.65).cos();
        let two = record(0, 1000.0, 4.0, |t| {
            1.5 + (2.0 * PI * 2.0 * t).sin() + 0.5 * (2.0 * PI * 200.0 * t).sin() + noise(t)
        });
        assert_eq!(tones_in(&two), [2.0, 200.0]);
        let off_bin = record(0, 1000.0, 3.0, |t| 0.2 * (2.0 * PI * 37.3 * t).sin());
        assert_eq!(tones_in(&off_bin), [37.3]);
        assert!(tones_in(&record(0, 1000.0, 1.0, |_| 3.0)).is_empty());
    }

    /// A recording of an exponential average reads its response at each
    /// tone of what went in — the slow one through, the fast one cut — to
    /// within a tenth of a decibel and a degree of the closed form.
    #[test]
    fn a_recording_reads_the_response_at_each_tone() {
        let rate = 1000.0;
        let alpha = 0.05;
        let input = record(0, rate, 4.0, |t| {
            (2.0 * PI * 2.0 * t).sin() + 0.5 * (2.0 * PI * 200.0 * t).sin()
        });
        let output = smoothed(&input, alpha);
        let points = recorded(&input, &output).unwrap();
        assert_eq!(
            points.iter().map(|p| p.freq).collect::<Vec<_>>(),
            [2.0, 200.0]
        );
        for point in &points {
            let (gain, phase) = smoothed_response(alpha, point.freq, rate);
            assert!(
                (dsp::db(point.gain) - dsp::db(gain)).abs() < 0.1,
                "{point:?} against {} dB",
                dsp::db(gain)
            );
            assert!(
                (point.phase - phase).abs().to_degrees() < 1.0,
                "{point:?} against {phase}"
            );
        }
        // A corner near 8 Hz: the slow tone loses a quarter of a decibel.
        assert!(dsp::db(points[0].gain) > -1.0, "the slow tone goes through");
        assert!(dsp::db(points[1].gain) < -20.0, "the fast one is cut");
    }

    #[test]
    fn records_that_cannot_be_compared_say_why() {
        let early = record(0, 1000.0, 1.0, |t| (2.0 * PI * 5.0 * t).sin());
        let late = record(2_000_000, 1000.0, 1.0, |t| (2.0 * PI * 5.0 * t).sin());
        assert_eq!(recorded(&early, &late), Err(Unread::Apart));
        let flat = record(0, 1000.0, 1.0, |_| 0.5);
        assert_eq!(recorded(&flat, &early), Err(Unread::NoTones));
    }

    #[test]
    fn a_phase_wraps_into_one_turn() {
        assert!((wrap(3.0 * PI / 2.0) + PI / 2.0).abs() < 1e-12);
        assert!((wrap(-3.0 * PI / 2.0) - PI / 2.0).abs() < 1e-12);
        assert!((wrap(PI) - PI).abs() < 1e-12);
    }
}
