//! Why a measurement, a design or an export was refused, in terms somebody
//! can act on.

use std::fmt;

/// A refusal from `dsp`, naming what was wrong rather than guessing past
/// it. The fields are the numbers the frontend needs to say it in the
/// user's language; `Display` is the English.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// A sample rate that is not a positive number of hertz.
    Rate { rate: f64 },
    /// A frequency not strictly between zero and half the rate — `what`
    /// says which (`tone`, `cutoff`, `low`, `high`).
    Frequency {
        what: &'static str,
        freq: f64,
        nyquist: f64,
    },
    /// A record that holds less than one period of the frequency asked
    /// about, where a sine and an offset cannot be told apart.
    Short { periods: f64 },
    /// Fewer taps than the design can be made of.
    Taps { taps: usize, least: usize },
    /// A Butterworth filter of order zero, which is no filter.
    Order,
    /// A quality factor that is not a positive number.
    Q { q: f64 },
    /// An exponential filter's weight outside `(0, 1]`: at zero it never
    /// moves, and above one it overshoots every sample.
    Alpha { alpha: f64 },
    /// A windowed-sinc high-pass with an even number of taps, whose
    /// response is forced to zero at half the rate — the one frequency a
    /// high-pass most has to pass.
    Even { taps: usize },
    /// A band whose low edge is not below its high edge.
    Edges { low: f64, high: f64 },
    /// Sections whose own arithmetic cannot hold the design at this rate:
    /// where the gain should be exactly one it comes to `gain`. A cutoff
    /// too small a fraction of the rate puts the poles closer to one than
    /// the coefficients can say.
    Inexact { gain: f64 },
    /// A name the exported struct cannot have: it is a type, so an
    /// upper-case letter and then letters and digits, and not `Self`.
    Name { name: String },
    /// A filter whose `f32` arithmetic strays from its design by `error`
    /// of its output's peak — more than an export allows, which is
    /// `limit`. The recursive sections are the ones this happens to, when
    /// the cutoff is so small a fraction of the rate that single precision
    /// cannot place the poles.
    Precision { error: f64, limit: f64 },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Rate { rate } => write!(f, "{rate} Hz is not a sample rate"),
            Refusal::Frequency {
                what,
                freq,
                nyquist,
            } => write!(
                f,
                "the {what} frequency has to be above 0 and below half the rate, \
                 {nyquist} Hz; {freq} Hz is not"
            ),
            Refusal::Short { periods } => write!(
                f,
                "the record holds {periods:.2} periods of that frequency, and \
                 at least one is needed to tell it from an offset"
            ),
            Refusal::Taps { taps, least } => {
                write!(f, "{taps} taps is too few: this needs at least {least}")
            }
            Refusal::Order => f.write_str("a Butterworth filter needs an order of at least 1"),
            Refusal::Q { q } => write!(f, "Q has to be above zero; {q} is not"),
            Refusal::Alpha { alpha } => write!(
                f,
                "alpha has to be above 0 and at most 1; at {alpha} the filter \
                 either never moves or overshoots every sample"
            ),
            Refusal::Even { taps } => write!(
                f,
                "a high-pass needs an odd number of taps: with {taps} its \
                 response is zero at half the rate, the frequency it most has \
                 to pass"
            ),
            Refusal::Edges { low, high } => write!(
                f,
                "the band's low edge, {low} Hz, has to be below its high \
                 edge, {high} Hz"
            ),
            Refusal::Inexact { gain } => write!(
                f,
                "at this rate the coefficients cannot hold the design: where \
                 its gain should be 1 it comes to {gain}. A cutoff this small \
                 a fraction of the rate wants a lower rate"
            ),
            Refusal::Name { name } => write!(
                f,
                "\"{name}\" cannot name the filter's struct: it has to be an \
                 upper-case letter and then letters and digits"
            ),
            Refusal::Precision { error, limit } => write!(
                f,
                "in f32 this filter strays from its design by {error:.1e} of \
                 its output, more than the {limit:.0e} an export allows: its \
                 poles are too close to one for single precision. Filter at a \
                 lower rate, or with a higher cutoff"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// A sample rate, or the refusal that it is not one.
pub(crate) fn rate(rate: f64) -> Result<f64, Refusal> {
    if rate.is_finite() && rate > 0.0 {
        Ok(rate)
    } else {
        Err(Refusal::Rate { rate })
    }
}

/// A frequency strictly inside `(0, rate/2)`, or the refusal naming which
/// one was not.
pub(crate) fn inside(what: &'static str, freq: f64, rate: f64) -> Result<f64, Refusal> {
    let nyquist = rate / 2.0;
    if freq.is_finite() && freq > 0.0 && freq < nyquist {
        Ok(freq)
    } else {
        Err(Refusal::Frequency {
            what,
            freq,
            nyquist,
        })
    }
}
