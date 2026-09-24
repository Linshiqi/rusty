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
