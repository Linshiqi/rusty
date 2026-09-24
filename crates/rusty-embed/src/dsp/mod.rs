//! Signal processing for filters that run on a microcontroller: what a
//! record is made of, and what a filter does to it.
//!
//! The filter a firmware runs is designed at a desk against a signal like
//! the one it will meet, and this is the arithmetic both halves of that
//! need — a spectrum and a single-frequency measurement to see a signal
//! and a filter's effect on it with.
//!
//! **Every answer here is held to a closed form**, the same rule as the
//! circuit solver's: a sine at a bin reads its own amplitude, a tone is
//! fitted to the digit, and the tests say which formula each is checked
//! against. A spectrum that is roughly right is how somebody tunes a notch
//! to the wrong hum.
//!
//! Compiled unconditionally and free of IO, like `signal` beside it: the
//! frontend draws what this computes, on the one thread it has.

mod fft;
mod refusal;
mod spectrum;

pub use refusal::Refusal;
pub use spectrum::{Spectrum, Tone, Window, db, spectrum, tone};
