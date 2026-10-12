//! What a record is made of: its spectrum, and one frequency of it
//! measured exactly.

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};

use super::fft::{Complex, fft};
use super::refusal::{self, Refusal};
use crate::signal::turns;

/// Weights that taper a stretch of samples towards its ends, so the
/// stretch's own edges do not read as frequencies it does not have.
///
/// Rectangular leaks the most and resolves the finest; Hann's first
/// sidelobe is 31 dB down and Blackman's 58, each paid for with a wider
/// peak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Window {
    Rectangular,
    Hann,
    Blackman,
}

impl Window {
    /// The weight `x` of the way through the window, 0 at its start and 1
    /// at its end: the coefficient function, which each caller samples its
    /// own way.
    ///
    /// A spectrum takes `i/n` — the *periodic* window, whose cosines sum to
    /// nothing over the `n` samples, which is what makes a sine at a bin
    /// frequency read exactly its amplitude. A filter's taps take
    /// `(i + 1)/(n + 1)`, symmetric about the middle tap and zero at
    /// neither end, because a tap that is always zero is a multiply the
    /// firmware pays for and nothing else.
    pub fn at(self, x: f64) -> f64 {
        match self {
            Window::Rectangular => 1.0,
            Window::Hann => 0.5 - 0.5 * (TAU * x).cos(),
            Window::Blackman => 0.42 - 0.5 * (TAU * x).cos() + 0.08 * (2.0 * TAU * x).cos(),
        }
    }
}

/// A one-sided amplitude spectrum.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spectrum {
    /// Each bin's frequency in hertz, from zero to half the rate.
    pub freqs: Vec<f64>,
    /// The amplitude at each, in the record's own units: a sine of
    /// amplitude `A` exactly at a bin's frequency reads `A` there.
    pub amplitude: Vec<f64>,
}

/// The spectrum of `samples` taken at `rate`, through `window`.
///
/// Zero-padded to the next power of two, which makes the bins finer
/// without resolving anything finer: the window still spans the record,
/// and its peak is as wide as the record is short. Divided by the window's
/// own sum — its coherent gain — so a Hann window reads a sine's amplitude
/// and not half of it, and doubled on every bin but DC and the top one,
/// because a real signal's power is split between each frequency and its
/// mirror and those two have no mirror.
///
/// Fewer than two samples, or a rate that is not one, have no spectrum:
/// the answer is empty rather than a single bin claiming to be one.
pub fn spectrum(samples: &[f64], rate: f64, window: Window) -> Spectrum {
    let n = samples.len();
    if n < 2 || refusal::rate(rate).is_err() {
        return Spectrum::default();
    }
    let size = n.next_power_of_two();
    let mut data = vec![Complex::default(); size];
    let mut gain = 0.0;
    for (i, (slot, &x)) in data.iter_mut().zip(samples).enumerate() {
        let weight = window.at(i as f64 / n as f64);
        gain += weight;
        *slot = Complex::new(x * weight, 0.0);
    }
    fft(&mut data);
    let top = size / 2;
    let (freqs, amplitude) = (0..=top)
        .map(|k| {
            let sides = if k == 0 || k == top { 1.0 } else { 2.0 };
            (k as f64 * rate / size as f64, sides * data[k].norm() / gain)
        })
        .unzip();
    Spectrum { freqs, amplitude }
}

/// An amplitude — or an amplitude ratio, a gain — in decibels,
/// `20·log₁₀`. Zero is minus infinity, which a plot clamps where it wants
/// its floor and a table prints as what it is.
pub fn db(amplitude: f64) -> f64 {
    20.0 * amplitude.log10()
}

/// One frequency's share of a record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tone {
    /// The peak amplitude, in the record's units.
    pub amplitude: f64,
    /// In radians, as in `amplitude · sin(2π·f·t + phase)` with `t = 0` at
    /// the first sample — the sine convention the signal generator uses,
    /// there in degrees.
    pub phase: f64,
}

/// How much of `freq` there is in `samples`, and at what phase: what a
/// frequency-response sweep reads off the input and the output at each
/// step.
///
/// A least-squares fit of `a·sin + b·cos + c` at exactly `freq`, weighted
/// by a Blackman window. The fit is exact for a sine and an offset,
/// whatever the record's length — no rounding to whole periods, no bin to
/// fall between — and the weights keep other frequencies out of it as far
/// as Blackman's sidelobes do: a tone ten bins away leaks in at about
/// −80 dB. The offset is fitted and put aside, so a sensor's bias is not
/// read as signal.
///
/// Refused for a frequency not strictly between zero and half the rate,
/// where a sine has no phase to fit, and for a record of less than one
/// period, where a sine and an offset cannot be told apart.
pub fn tone(samples: &[f64], rate: f64, freq: f64) -> Result<Tone, Refusal> {
    let rate = refusal::rate(rate)?;
    let freq = refusal::inside("tone", freq, rate)?;
    let periods = samples.len() as f64 * freq / rate;
    if periods < 1.0 {
        return Err(Refusal::Short { periods });
    }
    let n = samples.len() as f64;
    let mut gram = [[0.0; 3]; 3];
    let mut right = [0.0; 3];
    for (i, &x) in samples.iter().enumerate() {
        let weight = Window::Blackman.at((i as f64 + 0.5) / n);
        let (sin, cos) = (TAU * turns(i, freq, rate, 0.0)).sin_cos();
        let basis = [sin, cos, 1.0];
        for (row, &along) in basis.iter().enumerate() {
            right[row] += weight * along * x;
            for (column, &across) in basis.iter().enumerate() {
                gram[row][column] += weight * along * across;
            }
        }
    }
    let [a, b, _offset] = solve(gram, right).ok_or(Refusal::Short { periods })?;
    // a·sin θ + b·cos θ = A·sin(θ + φ) with a = A cos φ and b = A sin φ.
    Ok(Tone {
        amplitude: a.hypot(b),
        phase: b.atan2(a),
    })
}

/// `x` with `gram · x = right`, by elimination with partial pivoting —
/// or nothing, when the system is too close to singular to answer.
fn solve<const N: usize>(mut gram: [[f64; N]; N], mut right: [f64; N]) -> Option<[f64; N]> {
    let scale = gram
        .iter()
        .flatten()
        .fold(0.0f64, |most, v| most.max(v.abs()));
    for column in 0..N {
        let pivot =
            (column..N).max_by(|&i, &j| gram[i][column].abs().total_cmp(&gram[j][column].abs()))?;
        if gram[pivot][column].abs() <= 1e-12 * scale {
            return None;
        }
        gram.swap(column, pivot);
        right.swap(column, pivot);
        let (lead, known) = (gram[column], right[column]);
        for (row, value) in gram.iter_mut().zip(right.iter_mut()).skip(column + 1) {
            let factor = row[column] / lead[column];
            for (entry, above) in row.iter_mut().zip(lead) {
                *entry -= factor * above;
            }
            *value -= factor * known;
        }
    }
    let mut x = [0.0; N];
    for row in (0..N).rev() {
        let settled: f64 = (row + 1..N).map(|k| gram[row][k] * x[k]).sum();
        x[row] = (right[row] - settled) / gram[row][row];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::Signal;

    const RATE: f64 = 1000.0;

    /// `amplitude · sin(2π·k·i/n + phase)`: a sine exactly at bin `k`.
    fn at_bin(n: usize, k: usize, amplitude: f64, phase: f64) -> Vec<f64> {
        (0..n)
            .map(|i| amplitude * (TAU * (k * i % n) as f64 / n as f64 + phase).sin())
            .collect()
    }

    /// A sine exactly at a bin reads its amplitude at that bin through
    /// every window, and its neighbours read what the window's own
    /// coefficients say: nothing through a rectangle, half through Hann
    /// (its cosine's ¼ against its ½), and 0.25/0.42 and 0.04/0.42 through
    /// Blackman. Everything further off reads nothing.
    #[test]
    fn a_sine_at_a_bin_reads_its_amplitude_through_every_window() {
        let (n, k, amplitude) = (4096, 200, 0.7);
        let samples = at_bin(n, k, amplitude, 0.3);
        for (window, beside) in [
            (Window::Rectangular, vec![]),
            (Window::Hann, vec![0.5]),
            (Window::Blackman, vec![0.25 / 0.42, 0.04 / 0.42]),
        ] {
            let spectrum = spectrum(&samples, RATE, window);
            assert_eq!(spectrum.freqs.len(), n / 2 + 1);
            assert_eq!(spectrum.freqs[k], k as f64 * RATE / n as f64);
            for (bin, &got) in spectrum.amplitude.iter().enumerate() {
                let off = bin.abs_diff(k);
                let want = match off {
                    0 => amplitude,
                    _ => beside.get(off - 1).map_or(0.0, |share| share * amplitude),
                };
                assert!((got - want).abs() < 1e-12, "{window:?} bin {bin}: {got}");
            }
        }
    }

    /// Two tones read as two peaks, each where it is and as tall as it is.
    #[test]
    fn two_tones_read_as_two_peaks() {
        let n = 2048;
        let samples: Vec<f64> = at_bin(n, 100, 1.0, 0.0)
            .iter()
            .zip(at_bin(n, 300, 0.25, 1.0))
            .map(|(a, b)| a + b)
            .collect();
        let spectrum = spectrum(&samples, RATE, Window::Hann);
        let peaks: Vec<(f64, f64)> = spectrum
            .amplitude
            .windows(3)
            .enumerate()
            .filter(|(_, three)| three[1] > three[0] && three[1] > three[2] && three[1] > 1e-6)
            .map(|(bin, three)| (spectrum.freqs[bin + 1], three[1]))
            .collect();
        let bin = RATE / n as f64;
        assert_eq!(peaks.len(), 2, "{peaks:?}");
        assert_eq!(peaks[0].0, 100.0 * bin);
        assert_eq!(peaks[1].0, 300.0 * bin);
        assert!((peaks[0].1 - 1.0).abs() < 1e-12);
        assert!((peaks[1].1 - 0.25).abs() < 1e-12);
    }

    /// A record that is not a power of two is padded to one: the bins are
    /// the padded length's, a constant still reads as itself at DC, and a
    /// sine between bins peaks within half a bin of its frequency and no
    /// further below its amplitude than Hann's worst, 1.42 dB.
    #[test]
    fn a_record_that_is_not_a_power_of_two_is_padded() {
        let dc = spectrum(&[1.2; 1000], RATE, Window::Hann);
        assert_eq!(dc.freqs.len(), 513);
        assert_eq!(dc.freqs[1], RATE / 1024.0);
        assert!((dc.amplitude[0] - 1.2).abs() < 1e-12);

        let sine = Signal::parse("sine f=123.4 a=2").expect("reads");
        let between = spectrum(&sine.render(RATE, 1000, 0), RATE, Window::Hann);
        let (at, peak) = between
            .amplitude
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(bin, &peak)| (between.freqs[bin], peak))
            .expect("bins");
        assert!((at - 123.4).abs() <= RATE / 1024.0 / 2.0, "peak at {at}");
        assert!(peak <= 2.0 && db(peak / 2.0) > -1.43, "peak of {peak}");
    }

    /// The spectrum of pink noise falls 3 dB an octave and white noise's
    /// stays flat — the slope of the power, averaged over sixty-four
    /// records and over each octave, fitted across nine octaves. The
    /// tolerance is for the Voss generator's ripple, a few tenths of a
    /// decibel an octave, which is what it is known for.
    #[test]
    fn pink_noise_falls_three_decibels_an_octave() {
        let slope = |text: &str| {
            let samples = Signal::parse(text).expect("reads").render(1.0, 1 << 20, 3);
            let size = 1 << 14;
            let mut power = vec![0.0; size / 2 + 1];
            for record in samples.chunks_exact(size) {
                let spectrum = spectrum(record, 1.0, Window::Hann);
                for (sum, amplitude) in power.iter_mut().zip(&spectrum.amplitude) {
                    *sum += amplitude * amplitude;
                }
            }
            // Octave by octave, from 2⁻¹² up to 2⁻³ of the rate.
            let bands: Vec<(f64, f64)> = (3..12)
                .map(|octave| {
                    let (low, high) = (size >> (octave + 1), size >> octave);
                    let mean = power[low..high].iter().sum::<f64>() / (high - low) as f64;
                    (-(octave as f64) - 0.5, 10.0 * mean.log10())
                })
                .collect();
            let count = bands.len() as f64;
            let (x, y) = bands
                .iter()
                .fold((0.0, 0.0), |(x, y), b| (x + b.0, y + b.1));
            let (x, y) = (x / count, y / count);
            let rise: f64 = bands.iter().map(|b| (b.0 - x) * (b.1 - y)).sum();
            let run: f64 = bands.iter().map(|b| (b.0 - x).powi(2)).sum();
            rise / run
        };
        let pink = slope("pink rms=1");
        let white = slope("white rms=1");
        assert!((pink + 3.01).abs() < 0.4, "pink falls {pink} dB an octave");
        assert!(white.abs() < 0.2, "white moves {white} dB an octave");
    }

    #[test]
    fn nothing_has_no_spectrum() {
        assert_eq!(spectrum(&[1.0], RATE, Window::Hann), Spectrum::default());
        assert_eq!(spectrum(&[1.0; 8], 0.0, Window::Hann), Spectrum::default());
    }

    #[test]
    fn decibels_are_twenty_times_the_logarithm() {
        assert_eq!(db(1.0), 0.0);
        assert_eq!(db(10.0), 20.0);
        assert!((db(0.5f64.sqrt()) + 3.010_299_956_639_812).abs() < 1e-12);
        assert_eq!(db(0.0), f64::NEG_INFINITY);
    }

    #[test]
    fn the_windows_are_their_formulas() {
        for window in [Window::Rectangular, Window::Hann, Window::Blackman] {
            assert!(
                (window.at(0.5) - 1.0).abs() < 1e-15,
                "{window:?} peaks at 1"
            );
            assert!((window.at(0.25) - window.at(0.75)).abs() < 1e-15);
        }
        assert!(Window::Hann.at(0.0).abs() < 1e-15);
        assert!(Window::Blackman.at(0.0).abs() < 1e-15);
        assert!((Window::Hann.at(0.25) - 0.5).abs() < 1e-15);
    }

    /// A sine with an offset, over a record that is no whole number of
    /// periods, is measured exactly: the generator's degrees and the
    /// tone's radians are the same phase.
    #[test]
    fn a_tone_is_measured_to_the_amplitude_and_phase_it_has() {
        let signal = Signal::parse("dc 1.5; sine f=12.3 a=0.7 ph=40").expect("reads");
        let samples = signal.render(RATE, 937, 0);
        let tone = tone(&samples, RATE, 12.3).expect("measures");
        assert!((tone.amplitude - 0.7).abs() < 1e-9, "{tone:?}");
        assert!((tone.phase - 40f64.to_radians()).abs() < 1e-9, "{tone:?}");
    }

    /// Other tones stay out as far as Blackman's sidelobes keep them: nine
    /// hertz off, over a record just under a second long, is more than
    /// eight bins, where the window holds a tone below −70 dB.
    #[test]
    fn a_tone_is_measured_with_others_beside_it() {
        let text = "dc 1.5; sine f=12.3 a=0.7 ph=40; sine f=3 a=1; sine f=60 a=0.5";
        let samples = Signal::parse(text).expect("reads").render(RATE, 937, 0);
        let tone = tone(&samples, RATE, 12.3).expect("measures");
        assert!((tone.amplitude - 0.7).abs() < 1e-3, "{tone:?}");
        assert!((tone.phase - 40f64.to_radians()).abs() < 1e-3, "{tone:?}");
    }

    /// The first terms of the waveforms' Fourier series — `4/π`, `8/π²`
    /// and `2/π` of the amplitude, in phase with the sine — as a record of
    /// `N` samples a period has them: sampling puts `(π/N)/sin(π/N)` on
    /// each coefficient, once for a jump and twice for a corner, and a jump
    /// sampled at its new value leads by half a sample, `π/N`.
    #[test]
    fn the_waveforms_fundamentals_are_their_fourier_series_first_terms() {
        let (rate, freq, per) = (10_000.0, 10.0, 1000.0);
        let sampled = (std::f64::consts::PI / per) / (std::f64::consts::PI / per).sin();
        let lead = std::f64::consts::PI / per;
        let pi = std::f64::consts::PI;
        for (text, amplitude, phase) in [
            ("square f=10 a=1", 4.0 / pi * sampled, lead),
            (
                "triangle f=10 a=1",
                8.0 / (pi * pi) * sampled * sampled,
                0.0,
            ),
            ("sawtooth f=10 a=1", 2.0 / pi * sampled, lead),
        ] {
            let samples = Signal::parse(text).expect("reads").render(rate, 10_000, 0);
            let tone = tone(&samples, rate, freq).expect("measures");
            assert!(
                (tone.amplitude / amplitude - 1.0).abs() < 1e-9,
                "{text}: {} against {amplitude}",
                tone.amplitude
            );
            assert!((tone.phase - phase).abs() < 1e-9, "{text}: {}", tone.phase);
        }
    }

    #[test]
    fn a_tone_that_cannot_be_measured_is_refused() {
        let samples = vec![0.0; 1000];
        assert_eq!(
            tone(&samples, RATE, 500.0),
            Err(Refusal::Frequency {
                what: "tone",
                freq: 500.0,
                nyquist: 500.0
            })
        );
        assert!(matches!(
            tone(&samples, RATE, 0.0),
            Err(Refusal::Frequency { .. })
        ));
        assert_eq!(
            tone(&samples, RATE, 0.5),
            Err(Refusal::Short { periods: 0.5 })
        );
        assert_eq!(
            tone(&samples, -1.0, 10.0),
            Err(Refusal::Rate { rate: -1.0 })
        );
    }
}
