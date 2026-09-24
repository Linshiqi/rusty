//! Filter designs — what a person asks for — and how each is realised as
//! coefficients at a sample rate.
//!
//! The recursive designs all go through one function, `bilinear`: an
//! analog second-order prototype carried across by the bilinear transform,
//! prewarped so the prototype's frequency 1 lands exactly on the cutoff.
//! Robert Bristow-Johnson's cookbook writes the same four biquads in sines
//! and cosines of `ω₀`; written in `K = tan(ω₀/2)` they are one expression
//! each, without the `1 − cos ω₀` that loses its digits at a low cutoff,
//! and a Butterworth filter is the same function called once for each pair
//! of its poles with that pair's Q.

use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

use super::filter::{Coefficients, Filter, Section};
use super::refusal::{self, Refusal};
use super::spectrum::Window;

/// A filter as a person specifies it. Stored by the frontend, so it is
/// serialisable and says nothing about any one sample rate: [`realize`]
/// turns it into coefficients at one.
///
/// [`realize`]: Design::realize
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "design", rename_all = "kebab-case")]
pub enum Design {
    /// The mean of the last `taps` samples.
    MovingAverage { taps: usize },
    /// `y += alpha·(x − y)`: one pole, the cheapest smoothing there is.
    Exponential { alpha: f64 },
    /// One section from the cookbook, `cutoff` in hertz: the corner for a
    /// low- or high-pass, the centre of a band-pass (0 dB there) or a
    /// notch.
    Biquad {
        kind: BiquadKind,
        cutoff: f64,
        q: f64,
    },
    /// Maximally flat: `order` poles evenly round a circle, as a cascade of
    /// sections, −3.01 dB at `cutoff` whatever the order.
    Butterworth {
        pass: Pass,
        order: usize,
        cutoff: f64,
    },
    /// A windowed sinc: `taps` coefficients, linear phase, half amplitude
    /// at each cutoff.
    Fir {
        band: Band,
        taps: usize,
        window: Window,
    },
    /// The median of the last `taps` samples. Not linear: it takes a spike
    /// out without smearing an edge, which nothing above can.
    Median { taps: usize },
}

/// The four cookbook sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BiquadKind {
    LowPass,
    HighPass,
    BandPass,
    Notch,
}

/// Which side of a Butterworth filter's cutoff passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pass {
    Low,
    High,
}

/// What a windowed sinc passes, its edges in hertz.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "kebab-case")]
pub enum Band {
    LowPass { cutoff: f64 },
    HighPass { cutoff: f64 },
    BandPass { low: f64, high: f64 },
}

impl Design {
    /// The coefficients this design comes to at `rate` hertz — or why it
    /// cannot be realised there: a cutoff at or above half the rate, no
    /// taps, an order of zero, a Q or an alpha that is no such thing.
    ///
    /// A recursive design is checked after it is built, against where its
    /// gain has to be exactly one: a cutoff a few millionths of the rate
    /// puts the poles closer to the unit circle than a double can place
    /// them, and sections that no longer have the gain they were designed
    /// for are refused rather than handed over.
    pub fn realize(&self, rate: f64) -> Result<Filter, Refusal> {
        let rate = refusal::rate(rate)?;
        let coefficients = match *self {
            Design::MovingAverage { taps } => {
                at_least(taps, 1)?;
                Coefficients::Taps(vec![1.0 / taps as f64; taps])
            }
            Design::Exponential { alpha } => {
                if !(alpha > 0.0 && alpha <= 1.0) {
                    return Err(Refusal::Alpha { alpha });
                }
                Coefficients::Sections(vec![Section {
                    b: [alpha, 0.0, 0.0],
                    a: [alpha - 1.0, 0.0],
                }])
            }
            Design::Biquad { kind, cutoff, q } => {
                let cutoff = refusal::inside("cutoff", cutoff, rate)?;
                if !(q.is_finite() && q > 0.0) {
                    return Err(Refusal::Q { q });
                }
                Coefficients::Sections(vec![bilinear(kind, prewarp(cutoff, rate), q)])
            }
            Design::Butterworth {
                pass,
                order,
                cutoff,
            } => {
                if order == 0 {
                    return Err(Refusal::Order);
                }
                let cutoff = refusal::inside("cutoff", cutoff, rate)?;
                Coefficients::Sections(butterworth(pass, order, prewarp(cutoff, rate)))
            }
            Design::Fir { band, taps, window } => {
                Coefficients::Taps(windowed_sinc(band, taps, window, rate)?)
            }
            Design::Median { taps } => {
                at_least(taps, 1)?;
                Coefficients::Median(taps)
            }
        };
        let filter = Filter {
            design: self.clone(),
            rate,
            coefficients,
        };
        if let Some(freq) = self.unity(rate) {
            let gain = filter.response(freq).map_or(f64::NAN, |r| r.gain);
            if gain.is_nan() || (gain - 1.0).abs() > 1e-6 {
                return Err(Refusal::Inexact { gain });
            }
        }
        Ok(filter)
    }

    /// Where a recursive design's gain is exactly one by construction: DC
    /// for what passes DC, half the rate for a high-pass, the centre of a
    /// band-pass.
    fn unity(&self, rate: f64) -> Option<f64> {
        match *self {
            Design::Exponential { .. } => Some(0.0),
            Design::Biquad { kind, cutoff, .. } => Some(match kind {
                BiquadKind::LowPass | BiquadKind::Notch => 0.0,
                BiquadKind::HighPass => rate / 2.0,
                BiquadKind::BandPass => cutoff,
            }),
            Design::Butterworth { pass, .. } => Some(match pass {
                Pass::Low => 0.0,
                Pass::High => rate / 2.0,
            }),
            _ => None,
        }
    }
}

fn at_least(taps: usize, least: usize) -> Result<(), Refusal> {
    if taps >= least {
        Ok(())
    } else {
        Err(Refusal::Taps { taps, least })
    }
}

/// `K = tan(π·cutoff/rate)`: the analog frequency the bilinear transform
/// carries onto the cutoff. With it the digital response at any `f` is
/// the prototype's at `tan(πf/rate)/K` — exactly, which is what the tests
/// hold every recursive design to.
fn prewarp(cutoff: f64, rate: f64) -> f64 {
    (PI * cutoff / rate).tan()
}

/// One second-order section: the prototype with poles at `s² + s/Q + 1`
/// and the numerator of `kind` — `1`, `s²`, `s/Q` or `s² + 1` — through
/// `s = (1 − z⁻¹) / (K·(1 + z⁻¹))`, which with top and bottom multiplied
/// by `K²(1 + z⁻¹)²` is:
///
/// ```text
/// denominator  (1 + K/Q + K²) + 2(K² − 1)·z⁻¹ + (1 − K/Q + K²)·z⁻²
/// low-pass     K²·(1 + 2z⁻¹ + z⁻²)
/// high-pass    1 − 2z⁻¹ + z⁻²
/// band-pass    (K/Q)·(1 − z⁻²)
/// notch        (1 + K²) + 2(K² − 1)·z⁻¹ + (1 + K²)·z⁻²
/// ```
fn bilinear(kind: BiquadKind, k: f64, q: f64) -> Section {
    let k2 = k * k;
    let norm = 1.0 / (1.0 + k / q + k2);
    let b = match kind {
        BiquadKind::LowPass => [k2, 2.0 * k2, k2],
        BiquadKind::HighPass => [1.0, -2.0, 1.0],
        BiquadKind::BandPass => [k / q, 0.0, -k / q],
        BiquadKind::Notch => [1.0 + k2, 2.0 * (k2 - 1.0), 1.0 + k2],
    };
    Section {
        b: b.map(|c| c * norm),
        a: [2.0 * (k2 - 1.0) * norm, (1.0 - k / q + k2) * norm],
    }
}

/// The first-order prototype, `1/(s + 1)` or `s/(s + 1)`, the same way.
fn first_order(pass: Pass, k: f64) -> Section {
    let norm = 1.0 / (1.0 + k);
    let b = match pass {
        Pass::Low => [k * norm, k * norm, 0.0],
        Pass::High => [norm, -norm, 0.0],
    };
    Section {
        b,
        a: [(k - 1.0) * norm, 0.0],
    }
}

/// A Butterworth filter's sections: its poles are `order` points evenly
/// round the left half of the unit circle, `e^(iπ(2m + N + 1)/2N)`, and a
/// pair of them at angle θ is a section with `Q = −1/(2 cos θ)`. An odd
/// order has one real pole left over, a first-order section.
///
/// The gentlest sections come first and the sharpest last — MATLAB's
/// order, and the one in which whatever reaches the resonant pair has
/// already been smoothed by the rest.
fn butterworth(pass: Pass, order: usize, k: f64) -> Vec<Section> {
    let kind = match pass {
        Pass::Low => BiquadKind::LowPass,
        Pass::High => BiquadKind::HighPass,
    };
    let mut sections = Vec::with_capacity(order.div_ceil(2));
    if order % 2 == 1 {
        sections.push(first_order(pass, k));
    }
    for m in (0..order / 2).rev() {
        let angle = PI * (2 * m + order + 1) as f64 / (2 * order) as f64;
        sections.push(bilinear(kind, k, -1.0 / (2.0 * angle.cos())));
    }
    sections
}

/// A windowed sinc's taps: the ideal response's impulse response, cut to
/// `taps` and tapered by `window`.
///
/// A low-pass is `sin(2πf·x)/(πx)` about the middle tap, scaled so its taps
/// sum to exactly one — unity at DC. A high-pass is an impulse less that,
/// which needs a middle tap to put the impulse on, so an odd number of
/// taps; with an even number the response is forced to zero at half the
/// rate, where a high-pass most has to pass. A band-pass is the low-pass at
/// its high edge less the low-pass at its low edge. Each is half amplitude,
/// −6.02 dB, at its cutoffs, which is where the window's smoothing of the
/// ideal edge puts the middle of the fall.
fn windowed_sinc(band: Band, taps: usize, window: Window, rate: f64) -> Result<Vec<f64>, Refusal> {
    at_least(taps, 3)?;
    let low_pass = |cutoff: f64| -> Vec<f64> {
        let fraction = cutoff / rate;
        let middle = (taps - 1) as f64 / 2.0;
        let raw: Vec<f64> = (0..taps)
            .map(|i| {
                let x = i as f64 - middle;
                let ideal = if x == 0.0 {
                    2.0 * fraction
                } else {
                    sin_pi(2.0 * fraction * x) / (PI * x)
                };
                ideal * window.at((i + 1) as f64 / (taps + 1) as f64)
            })
            .collect();
        let sum: f64 = raw.iter().sum();
        raw.iter().map(|tap| tap / sum).collect()
    };
    match band {
        Band::LowPass { cutoff } => Ok(low_pass(refusal::inside("cutoff", cutoff, rate)?)),
        Band::HighPass { cutoff } => {
            let cutoff = refusal::inside("cutoff", cutoff, rate)?;
            if taps.is_multiple_of(2) {
                return Err(Refusal::Even { taps });
            }
            let mut high: Vec<f64> = low_pass(cutoff).iter().map(|tap| -tap).collect();
            high[taps / 2] += 1.0;
            Ok(high)
        }
        Band::BandPass { low, high } => {
            let low = refusal::inside("low", low, rate)?;
            let high = refusal::inside("high", high, rate)?;
            if low >= high {
                return Err(Refusal::Edges { low, high });
            }
            let (wide, narrow) = (low_pass(high), low_pass(low));
            Ok(wide.iter().zip(&narrow).map(|(w, n)| w - n).collect())
        }
    }
}

/// `sin(πt)`, and exactly zero at every whole `t`.
///
/// The sinc's zeros land on taps whenever the cutoff is a round fraction
/// of the rate — every fifth tap at a tenth — and `sin(π·t)` computed as
/// written puts π's own rounding there instead: taps of 10⁻¹⁸ that an
/// export then prints as `7.5e-20`, a multiply by nothing that reads as a
/// number somebody chose. Reduced to within half a turn of zero first, a
/// whole `t` is zero exactly.
fn sin_pi(t: f64) -> f64 {
    // Exact for any |t| a tap's argument can reach: halving, rounding and
    // doubling a double move no bits, and the difference is a multiple of
    // t's own last place.
    let r = t - 2.0 * (t / 2.0).round();
    let folded = if r > 0.5 {
        1.0 - r
    } else if r < -0.5 {
        -1.0 - r
    } else {
        r
    };
    (PI * folded).sin()
}

#[cfg(test)]
mod tests;
