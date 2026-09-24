//! What a signal generator produces: a sum of components, each a function
//! of time, in the units of whatever it feeds.
//!
//! A generated signal is what the emulator's converter plays against the
//! firmware's own clock, what a sensor's reading wanders by, and what a
//! filter is designed against. So it has to be three things: exact,
//! repeatable, and writable by hand.
//!
//! **Exact.** Every component is a function of the instant a sample is
//! taken, never of the sample before it. A phase accumulated sample by
//! sample carries its rounding into every period after it, and ten million
//! samples into a render the sine has drifted; here the ten-millionth sample
//! of a whole-numbered frequency is as good as the first (`turns`).
//!
//! **Repeatable.** The noise comes from rusty's own generator (`random`), so
//! a seed renders the same noise on every run and after every upgrade — a
//! library generator's algorithm is its own business, and changes between
//! versions. Each random component draws from a stream of its own, fixed by
//! the seed, its kind and how many of its kind came before it: putting an
//! offset in front of a noisy signal does not change its noise, and the
//! first second of a render is the same whether a second or a minute was
//! asked for. What is not rusty's is the last bit of a logarithm or a
//! cosine, which is the platform's; nothing here branches on one, so two
//! platforms agree sample for sample to within that bit.
//!
//! **Writable.** A signal is stored as a line of text a person can read and
//! change — `dc 1.2; sine f=50 a=0.1; white rms=0.005` — in a part's
//! properties and in `.rusty/sim.toml`, and the reader (`text`) is strict:
//! a key it does not know is an error naming it, never a component quietly
//! missing from what the firmware was fed.
//!
//! Compiled unconditionally, like `protocol` and `plant`: the frontend
//! renders what it draws, and the backend renders what the emulator plays.

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};

mod looping;
mod random;
mod text;

pub use text::{Rule, SignalError};

use random::{Noise, Pink, Spikes};

/// A generated signal: the sum of its components, in the units of whatever
/// it feeds — volts at a pin, rad/s on a gyro axis, °C on a thermometer.
///
/// No components is silence, which is a signal too.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Signal {
    pub components: Vec<Component>,
}

/// One term of a [`Signal`].
///
/// Frequencies are in hertz, times in seconds and phases in degrees. Every
/// periodic waveform's fundamental is in phase with the sine of the same
/// `phase`: with `phase` zero a sine, a 50% square, a triangle and a
/// sawtooth all start their period at `t = 0` on the way up, so exchanging
/// one for another moves no edge a filter is being judged by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Component {
    /// A constant.
    Dc { level: f64 },
    /// `amplitude · sin(2π·freq·t + phase)`.
    Sine {
        freq: f64,
        amplitude: f64,
        phase: f64,
    },
    /// `+amplitude` for the first `duty` of every period and `−amplitude`
    /// for the rest; `duty` is a fraction, 0 to 1.
    Square {
        freq: f64,
        amplitude: f64,
        duty: f64,
        phase: f64,
    },
    /// Straight lines between `±amplitude`: up from zero, at the top a
    /// quarter of the way through each period, at the bottom three
    /// quarters of the way.
    Triangle {
        freq: f64,
        amplitude: f64,
        phase: f64,
    },
    /// A ramp rising through zero at the start of every period, from
    /// `−amplitude` to `+amplitude`, and falling straight back halfway
    /// through.
    Sawtooth {
        freq: f64,
        amplitude: f64,
        phase: f64,
    },
    /// A sine sweeping from `from` to `to` hertz over `period` seconds, and
    /// then again from `from` — in equal hertz per second, or in equal
    /// ratios per second when `log`. Each sweep starts from phase zero, so a
    /// sweep is the same every time round.
    Chirp {
        from: f64,
        to: f64,
        period: f64,
        amplitude: f64,
        log: bool,
    },
    /// Gaussian noise with a standard deviation of `rms`, every sample
    /// independent of every other.
    White { rms: f64 },
    /// Noise with the same power in every octave — its spectrum falling as
    /// 1/f, 3 dB an octave — of RMS `rms`, over the sixteen octaves below
    /// half the rate it is rendered at.
    Pink { rms: f64 },
    /// Pulses of `±amplitude`, `width` seconds long, arriving at random
    /// instants `rate` a second on average, each one's sign a coin toss.
    ///
    /// The arrivals are instants in time, not samples, so a render at any
    /// rate has the same spikes in the same places — and a spike narrower
    /// than the gap between two samples can fall between them, as it does
    /// at a real converter's input.
    Spikes {
        rate: f64,
        amplitude: f64,
        width: f64,
    },
    /// `size` from `at` seconds on, and nothing before.
    Step { at: f64, size: f64 },
}

impl Component {
    /// The word the text form and the wire form both call it by.
    pub fn kind(&self) -> &'static str {
        match self {
            Component::Dc { .. } => "dc",
            Component::Sine { .. } => "sine",
            Component::Square { .. } => "square",
            Component::Triangle { .. } => "triangle",
            Component::Sawtooth { .. } => "sawtooth",
            Component::Chirp { .. } => "chirp",
            Component::White { .. } => "white",
            Component::Pink { .. } => "pink",
            Component::Spikes { .. } => "spikes",
            Component::Step { .. } => "step",
        }
    }

    /// Whether it draws on the seed at all. Noise with no power and spikes
    /// that never come are as repeatable as a sine.
    fn is_random(&self) -> bool {
        match *self {
            Component::White { rms } | Component::Pink { rms } => rms != 0.0,
            Component::Spikes {
                rate, amplitude, ..
            } => rate > 0.0 && amplitude != 0.0,
            _ => false,
        }
    }
}

impl Signal {
    /// `count` samples at `rate` hertz, the first at `t = 0`.
    ///
    /// `seed` fixes the noise and nothing else: a signal with no random
    /// component renders the same whatever it is. The first `n` samples of
    /// a render are the first `n` of any longer render of the same signal,
    /// rate and seed. A rate that is not a positive number of hertz renders
    /// nothing, because there is no instant to put a sample at.
    ///
    /// The numbers are rendered as they stand: a signal [`Signal::check`]
    /// would refuse renders whatever its arithmetic gives, which is why the
    /// reader of the text form checks before it answers.
    pub fn render(&self, rate: f64, count: usize, seed: u64) -> Vec<f64> {
        if !(rate.is_finite() && rate > 0.0) {
            return Vec::new();
        }
        let mut out = vec![0.0; count];
        // How many of each random kind have been rendered, so each one's
        // stream is fixed by its place among its own kind and nothing else.
        let mut drawn = [0u64; 3];
        let mut stream = |noise: Noise| {
            let ordinal = &mut drawn[noise as usize];
            *ordinal += 1;
            random::stream(seed, noise, *ordinal - 1)
        };
        for component in &self.components {
            match *component {
                Component::Dc { level } => out.iter_mut().for_each(|sample| *sample += level),
                Component::Sine {
                    freq,
                    amplitude,
                    phase,
                } => {
                    for (n, sample) in out.iter_mut().enumerate() {
                        *sample += amplitude * (TAU * turns(n, freq, rate, phase)).sin();
                    }
                }
                Component::Square {
                    freq,
                    amplitude,
                    duty,
                    phase,
                } => {
                    for (n, sample) in out.iter_mut().enumerate() {
                        let high = turns(n, freq, rate, phase) < duty;
                        *sample += if high { amplitude } else { -amplitude };
                    }
                }
                Component::Triangle {
                    freq,
                    amplitude,
                    phase,
                } => {
                    for (n, sample) in out.iter_mut().enumerate() {
                        // A quarter turn on, the peak sits in the middle of
                        // the period and the line out of it is `1 − 4|q − ½|`.
                        let q = fraction(turns(n, freq, rate, phase) + 0.25);
                        *sample += amplitude * (1.0 - 4.0 * (q - 0.5).abs());
                    }
                }
                Component::Sawtooth {
                    freq,
                    amplitude,
                    phase,
                } => {
                    for (n, sample) in out.iter_mut().enumerate() {
                        let q = fraction(turns(n, freq, rate, phase) + 0.5);
                        *sample += amplitude * (2.0 * q - 1.0);
                    }
                }
                Component::Chirp {
                    from,
                    to,
                    period,
                    amplitude,
                    log,
                } => {
                    // The sweep's own clock is taken from the sample's index,
                    // not from a sum of steps, for the reason `turns` gives.
                    let span = period * rate;
                    for (n, sample) in out.iter_mut().enumerate() {
                        let into = (n as f64 % span) / rate;
                        let cycles = sweep_turns(from, to, period, log, into);
                        *sample += amplitude * (TAU * fraction(cycles)).sin();
                    }
                }
                Component::White { rms } => {
                    let mut draws = stream(Noise::White);
                    out.iter_mut()
                        .for_each(|sample| *sample += rms * draws.gaussian());
                }
                Component::Pink { rms } => {
                    let mut pink = Pink::new(stream(Noise::Pink));
                    out.iter_mut()
                        .for_each(|sample| *sample += rms * pink.sample());
                }
                Component::Spikes {
                    rate: often,
                    amplitude,
                    width,
                } => {
                    let mut spikes = Spikes::new(stream(Noise::Spikes), often, width);
                    for (n, sample) in out.iter_mut().enumerate() {
                        *sample += amplitude * spikes.at(n as f64 / rate);
                    }
                }
                Component::Step { at, size } => {
                    for (n, sample) in out.iter_mut().enumerate() {
                        if n as f64 / rate >= at {
                            *sample += size;
                        }
                    }
                }
            }
        }
        out
    }

    /// Whether anything in it depends on the seed.
    ///
    /// Beside [`Signal::loop_length`], because it is the other half of what
    /// a table that loops has to know: a loop of a random signal plays the
    /// same noise every time round, which a signal with no noise in it
    /// cannot be caught doing.
    pub fn is_random(&self) -> bool {
        self.components.iter().any(Component::is_random)
    }
}

/// Where sample `n`, taken at `rate` hertz, falls in a cycle of `freq`
/// shifted by `phase` degrees: a fraction of a turn, in `[0, 1)`.
///
/// `n·freq mod rate` is exact whenever `n·freq` is — every whole-numbered
/// frequency, for any render short of 2⁵³ samples, and every frequency with
/// a few binary places for nearly as long — so there is one rounding here
/// however late the sample. Dividing by the rate first, or adding a phase
/// increment sample by sample, puts the rounding of `freq / rate` into
/// every period after it, and the error grows with the render.
pub(crate) fn turns(n: usize, freq: f64, rate: f64, phase: f64) -> f64 {
    fraction((n as f64 * freq) % rate / rate + phase / 360.0)
}

/// `x − ⌊x⌋`, kept below 1: a negative sliver of a turn rounds up to a
/// whole one, which is the start of the next.
fn fraction(x: f64) -> f64 {
    let part = x - x.floor();
    if part < 1.0 { part } else { 0.0 }
}

/// How many turns a sweep has made `into` seconds after it started.
///
/// The phase is the integral of the frequency: `f₀τ + (f₁ − f₀)τ²/2T`
/// for a straight sweep, and `f₀T/g · (e^{gτ/T} − 1)` for one that grows by
/// equal ratios, where `g = ln(f₁/f₀)` — with `exp_m1`, so the start of the
/// sweep keeps its digits.
fn sweep_turns(from: f64, to: f64, period: f64, log: bool, into: f64) -> f64 {
    if !log {
        return from * into + (to - from) * into * into / (2.0 * period);
    }
    let growth = (to / from).ln();
    if growth == 0.0 {
        from * into
    } else {
        from * period / growth * (growth * into / period).exp_m1()
    }
}

/// A signal the frontend offers by name: a case from a real bench.
///
/// `id` is what the name is translated by, so there is no English in here
/// to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub id: &'static str,
    pub signal: Signal,
}

/// The cases a filter is usually designed against, each written the way a
/// person would type it.
///
/// Levels are in the units of the reading each one imitates — volts, m/s²,
/// °C — so the numbers read as the thing they stand for.
const PRESETS: [(&str, &str); 7] = [
    // A slow sensor on a long lead: a reading wandering at a fifth of a
    // hertz, the mains coupled in at 50 Hz, and the converter's own noise.
    (
        "mains-hum",
        "dc 1.2; sine f=0.2 a=0.05; sine f=50 a=0.1; white rms=0.005",
    ),
    // An accelerometer axis on a motor mount, in m/s²: gravity, the shaft's
    // imbalance at 120 Hz and its second harmonic, and broadband buzz.
    (
        "vibration",
        "dc 9.81; sine f=120 a=0.8; sine f=240 a=0.3; white rms=0.05",
    ),
    // A thermistor in a room, in °C: a reading that drifts rather than hums.
    ("thermistor", "dc 25; pink rms=0.2; white rms=0.05"),
    // Impulsive interference on a slow reading — what a median filter is for.
    (
        "spikes",
        "dc 1.65; sine f=1 a=0.5; spikes rate=5 a=1 w=0.0005; white rms=0.002",
    ),
    // A sweep across the band, what a frequency response is measured with.
    ("sweep", "chirp from=1 to=200 t=10 a=0.5 log"),
    // A step with a little noise on it: settling time and overshoot.
    ("step", "step at=0.5 size=1; white rms=0.01"),
    // Two tones a decade apart, what a filter's separation is judged by.
    ("two-tones", "sine f=5 a=1; sine f=60 a=0.5"),
];

/// The presets, in the order they are offered.
pub fn presets() -> Vec<Preset> {
    PRESETS
        .iter()
        .map(|&(id, text)| Preset {
            id,
            // Every one is read by `every_preset_reads_back_as_written`,
            // so this cannot fail on anything that passed the tests.
            signal: Signal::parse(text).expect("a preset reads"),
        })
        .collect()
}

#[cfg(test)]
mod tests;
