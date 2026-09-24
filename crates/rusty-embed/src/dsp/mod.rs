//! Signal processing for filters that run on a microcontroller: what a
//! record is made of, and what a filter does to it.
//!
//! The filter a firmware runs is designed at a desk against a signal like
//! the one it will meet, and this is the arithmetic both halves of that
//! need. A spectrum and a single-frequency measurement to see a signal
//! with; a [`Design`] — what a person asks for — realised at a sample rate
//! as a [`Filter`], whose coefficients say what it does to a record
//! (`apply`) and to a sine (`response`).
//!
//! **Every answer here is held to a closed form**, the same rule as the
//! circuit solver's: a sine at a bin reads its own amplitude, a Butterworth
//! filter is its analog prototype at the warped frequency to the last few
//! digits, a moving average is the Dirichlet kernel, and `apply` on a long
//! sine comes out where `response` says. The tests say which formula each
//! is checked against. A response that is roughly right is how somebody
//! tunes a notch to the wrong hum.
//!
//! Compiled unconditionally and free of IO, like `signal` beside it: the
//! frontend draws what this computes, on the one thread it has.

mod code;
mod design;
mod fft;
mod filter;
mod refusal;
mod spectrum;

pub use code::{LIMIT, check_name};
pub use design::{Band, BiquadKind, Design, Pass};
pub use filter::{Coefficients, Filter, Response, Section};
pub use refusal::Refusal;
pub use spectrum::{Spectrum, Tone, Window, db, spectrum, tone};
