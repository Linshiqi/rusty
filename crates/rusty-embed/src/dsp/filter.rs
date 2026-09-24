//! A realised filter: the coefficients a design comes to at one rate, what
//! they do to a record, and what they do to a sine.

use super::design::Design;
use super::fft::Complex;

/// One second-order section with `a0` divided out:
///
/// ```text
/// y[n] = b0·x[n] + b1·x[n−1] + b2·x[n−2] − a1·y[n−1] − a2·y[n−2]
/// ```
///
/// `a` holds `[a1, a2]`. A first-order section is one whose second-order
/// terms, `b2` and `a2`, are zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Section {
    pub b: [f64; 3],
    pub a: [f64; 2],
}

/// What a design is realised as.
#[derive(Debug, Clone, PartialEq)]
pub enum Coefficients {
    /// Recursive: second-order sections run one into the next.
    Sections(Vec<Section>),
    /// A finite impulse response, the tap that meets the newest sample
    /// first.
    Taps(Vec<f64>),
    /// The median of the last this-many samples. Not linear, so no
    /// coefficients and no frequency response.
    Median(usize),
}

/// What a linear filter does to a sine: scales it by `gain` and turns it by
/// `phase` radians, so `sin(ωt)` comes out as `gain · sin(ωt + phase)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Response {
    pub gain: f64,
    pub phase: f64,
}

/// A design realised at a sample rate, which only [`Design::realize`]
/// makes: its coefficients mean what they mean at that rate and no other,
/// so the rate travels with them rather than being asked for again by
/// every question put to them.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub(crate) design: Design,
    pub(crate) rate: f64,
    pub(crate) coefficients: Coefficients,
}

impl Filter {
    /// What it was realised from.
    pub fn design(&self) -> &Design {
        &self.design
    }

    /// The sample rate its coefficients are for, in hertz.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    pub fn coefficients(&self) -> &Coefficients {
        &self.coefficients
    }

    /// Run over `samples` from rest — every delay zero, every window full
    /// of zeros — as a firmware that has just started would.
    ///
    /// Sections in Direct Form II Transposed, which is what the exported
    /// code runs too: two delays a section, half of Direct Form I's, each
    /// holding a value the size of the signal — where Direct Form II's
    /// hold the signal divided by the denominator, which near a low cutoff
    /// is close to nothing, and the quotient a large number to round.
    pub fn apply(&self, samples: &[f64]) -> Vec<f64> {
        match &self.coefficients {
            Coefficients::Sections(sections) => {
                let mut state = vec![[0.0; 2]; sections.len()];
                samples
                    .iter()
                    .map(|&x| {
                        sections
                            .iter()
                            .zip(state.iter_mut())
                            .fold(x, |y, (section, delay)| section.step(y, delay))
                    })
                    .collect()
            }
            Coefficients::Taps(taps) => (0..samples.len())
                .map(|n| {
                    taps.iter()
                        .zip(samples[..=n].iter().rev())
                        .map(|(tap, x)| tap * x)
                        .sum()
                })
                .collect(),
            Coefficients::Median(taps) => median(samples, *taps),
        }
    }

    /// What it does to a sine of `freq` hertz once the start has passed —
    /// or nothing for a median, which does not answer a sine with a sine.
    ///
    /// `H(z)` on the unit circle, `z = e^(2πi·freq/rate)`: the product of
    /// the sections' ratios, or the sum of the taps turned by their delays.
    /// The phase is the principal value, in `(−π, π]`.
    pub fn response(&self, freq: f64) -> Option<Response> {
        let turns = freq / self.rate;
        let h = match &self.coefficients {
            Coefficients::Sections(sections) => {
                // z⁻¹ and z⁻², each from its own angle.
                let (one, two) = (Complex::turn(-turns), Complex::turn(-2.0 * turns));
                sections.iter().fold(Complex::new(1.0, 0.0), |h, s| {
                    let top = Complex::new(s.b[0], 0.0) + one * s.b[1] + two * s.b[2];
                    let bottom = Complex::new(1.0, 0.0) + one * s.a[0] + two * s.a[1];
                    h * (top / bottom)
                })
            }
            Coefficients::Taps(taps) => taps
                .iter()
                .enumerate()
                .fold(Complex::default(), |h, (k, &tap)| {
                    h + Complex::turn(-(k as f64) * turns) * tap
                }),
            Coefficients::Median(_) => return None,
        };
        Some(Response {
            gain: h.norm(),
            phase: h.arg(),
        })
    }
}

impl Section {
    /// One sample through, Direct Form II Transposed.
    fn step(&self, x: f64, delay: &mut [f64; 2]) -> f64 {
        let y = self.b[0] * x + delay[0];
        delay[0] = self.b[1] * x - self.a[0] * y + delay[1];
        delay[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// The running median of a window `taps` long that starts full of zeros.
///
/// The window is kept sorted as it slides — the sample leaving found by
/// halving and taken out, the one arriving put in where it belongs — so a
/// window of a few hundred costs a copy of it per sample rather than a
/// sort. An even window's median is the mean of its middle two.
fn median(samples: &[f64], taps: usize) -> Vec<f64> {
    let mut ring = vec![0.0f64; taps];
    let mut sorted = vec![0.0f64; taps];
    let mut next = 0;
    samples
        .iter()
        .map(|&x| {
            let leaving = std::mem::replace(&mut ring[next], x);
            next = (next + 1) % taps;
            let at = sorted.partition_point(|v| v.total_cmp(&leaving).is_lt());
            sorted.remove(at);
            let to = sorted.partition_point(|v| v.total_cmp(&x).is_lt());
            sorted.insert(to, x);
            middle(&sorted)
        })
        .collect()
}

/// The middle of a sorted window: its middle value, or the mean of its
/// middle two.
pub(crate) fn middle(sorted: &[f64]) -> f64 {
    let half = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[half]
    } else {
        (sorted[half - 1] + sorted[half]) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::dsp::{Band, BiquadKind, Pass, Window, tone};
    use crate::signal::Signal;

    const RATE: f64 = 1000.0;

    fn realized(design: Design) -> Filter {
        design.realize(RATE).expect("realises")
    }

    /// The angle between two phases, whichever way round the circle is
    /// shorter.
    fn apart(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(2.0 * PI);
        d.min(2.0 * PI - d)
    }

    /// `apply` and `response` are two accounts of one filter: a long sine
    /// run through `apply` comes out, once the start has passed, scaled by
    /// the gain `response` gives and turned by its phase — measured by
    /// `tone` on the input and the output over the same samples. Every
    /// linear design, in its passband, its stopband and on its edge.
    #[test]
    fn apply_reaches_the_sine_that_response_predicts() {
        let designs = [
            Design::MovingAverage { taps: 8 },
            Design::Exponential { alpha: 0.05 },
            Design::Biquad {
                kind: BiquadKind::BandPass,
                cutoff: 60.0,
                q: 2.0,
            },
            Design::Biquad {
                kind: BiquadKind::Notch,
                cutoff: 50.0,
                q: 5.0,
            },
            Design::Butterworth {
                pass: Pass::Low,
                order: 4,
                cutoff: 50.0,
            },
            Design::Butterworth {
                pass: Pass::High,
                order: 3,
                cutoff: 20.0,
            },
            Design::Fir {
                band: Band::BandPass {
                    low: 40.0,
                    high: 120.0,
                },
                taps: 61,
                window: Window::Blackman,
            },
        ];
        for design in designs {
            let filter = realized(design.clone());
            for freq in [3.0, 20.0, 50.0, 60.0, 90.0, 200.0, 410.0] {
                let input = Signal::parse(&format!("sine f={freq} a=1 ph=10"))
                    .expect("reads")
                    .render(RATE, 12_000, 0);
                let output = filter.apply(&input);
                // The last eight thousand samples: long after every
                // transient here has died away by e¹⁰⁰ or more.
                let (before, after) = (&input[4000..], &output[4000..]);
                let went_in = tone(before, RATE, freq).expect("measures");
                let came_out = tone(after, RATE, freq).expect("measures");
                let want = filter.response(freq).expect("linear");
                let gain = came_out.amplitude / went_in.amplitude;
                assert!(
                    (gain - want.gain).abs() < 1e-10 + 1e-9 * want.gain,
                    "{design:?} at {freq} Hz: {gain} against {}",
                    want.gain
                );
                if want.gain > 1e-3 {
                    let turned = came_out.phase - went_in.phase;
                    assert!(
                        apart(turned, want.phase) < 1e-8,
                        "{design:?} at {freq} Hz: turned {turned}, not {}",
                        want.phase
                    );
                }
            }
        }
    }

    /// From rest, an exponential filter's answer to a step is the closed
    /// form of its recurrence, `1 − (1 − α)^(n+1)`, and a FIR's answer to
    /// one sample is its own taps.
    #[test]
    fn apply_starts_from_rest() {
        let alpha = 0.2;
        let smooth = realized(Design::Exponential { alpha });
        for (n, y) in smooth.apply(&[1.0; 40]).iter().enumerate() {
            let want = 1.0 - (1.0 - alpha).powi(n as i32 + 1);
            assert!((y - want).abs() < 1e-14, "step {n}: {y}");
        }
        let fir = realized(Design::Fir {
            band: Band::LowPass { cutoff: 100.0 },
            taps: 15,
            window: Window::Hann,
        });
        let Coefficients::Taps(taps) = fir.coefficients() else {
            panic!("a FIR is taps");
        };
        let mut impulse = vec![0.0; 20];
        impulse[0] = 1.0;
        let answer = fir.apply(&impulse);
        assert_eq!(&answer[..15], taps.as_slice());
        assert!(answer[15..].iter().all(|&v| v == 0.0));
    }

    /// A median passes an edge without smearing it and takes a spike out
    /// without trace: fewer than half its window's samples cannot move it.
    /// It delays both by half the window.
    #[test]
    fn a_median_removes_spikes_and_keeps_edges() {
        let filter = realized(Design::Median { taps: 5 });
        assert_eq!(filter.response(50.0), None);
        let mut samples = vec![1.0; 200];
        for spike in (10..200).step_by(20) {
            samples[spike] = if spike % 40 == 10 { 9.0 } else { -7.0 };
        }
        for level in &mut samples[100..] {
            *level += 2.0;
        }
        let out = filter.apply(&samples);
        // From rest the window holds zeros, so the first two answers are
        // the zeros' median.
        assert_eq!(&out[..2], [0.0, 0.0]);
        assert!(out[2..102].iter().all(|&v| v == 1.0), "{:?}", &out[2..102]);
        assert!(out[102..].iter().all(|&v| v == 3.0), "{:?}", &out[102..]);
        // An even window answers the mean of its middle two: [0 0 0 4],
        // [0 0 2 4], [0 2 4 8] and [2 4 6 8] sorted.
        let even = realized(Design::Median { taps: 4 });
        assert_eq!(even.apply(&[4.0, 2.0, 8.0, 6.0]), [0.0, 1.0, 3.0, 5.0]);
    }
}
