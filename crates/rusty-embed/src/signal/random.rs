//! Where a signal's noise comes from: rusty's own generator, so a seed means
//! the same noise everywhere and for good.
//!
//! **SplitMix64** (Steele, Lea & Flood, 2014): sixty-four bits of state, one
//! add and two multiply–xorshifts a draw. It passes BigCrush, and its output
//! is fixed by three constants and nothing else — which is the property
//! wanted, because a seed stored beside a board has to render the noise it
//! rendered last year. A library generator promises neither its algorithm
//! nor its stream across versions.

use std::f64::consts::TAU;

/// The generator's increment, the golden ratio in sixty-four bits.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// SplitMix64's finaliser: every input bit reaches every output bit.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The kinds of component that draw from a stream, each with streams of its
/// own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Noise {
    White = 0,
    Pink = 1,
    Spikes = 2,
}

/// The stream for the `ordinal`th component of one kind of noise.
///
/// Fixed by the seed, the kind and the ordinal alone, so a component's noise
/// does not change when something of another kind is put in front of it —
/// an offset added to a noisy reading leaves the noise it was being judged
/// by exactly where it was.
///
/// The three are folded into the key one after another, each through the
/// finaliser, never side by side: XORed together they could stand in for
/// each other, and seed 1's second white noise was seed 2's first.
pub(crate) fn stream(seed: u64, noise: Noise, ordinal: u64) -> Stream {
    let mut key = seed;
    for word in [noise as u64, ordinal] {
        key = mix(key.wrapping_add(GOLDEN)) ^ word;
    }
    Stream::new(mix(key.wrapping_add(GOLDEN)))
}

/// One SplitMix64 generator, with the second of each pair of normal draws
/// kept for the next ask.
#[derive(Debug, Clone)]
pub(crate) struct Stream {
    state: u64,
    spare: Option<f64>,
}

impl Stream {
    pub(crate) fn new(state: u64) -> Self {
        Stream { state, spare: None }
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN);
        mix(self.state)
    }

    /// Uniform on `(0, 1]`: never zero, so its logarithm is never infinite.
    pub(crate) fn open(&mut self) -> f64 {
        open_from(self.next_u64())
    }

    /// Uniform on `[0, 1)`.
    pub(crate) fn unit(&mut self) -> f64 {
        unit_from(self.next_u64())
    }

    /// A standard normal draw, by Box–Muller: two uniforms make two
    /// independent normals, `√(−2 ln u)` times the cosine and the sine of a
    /// turn `v`.
    ///
    /// Exact in its tails, where a sum of uniforms is not; and it takes no
    /// branch on a result the platform's logarithm rounded, which is what
    /// keeps two platforms' streams from ever going their separate ways.
    pub(crate) fn gaussian(&mut self) -> f64 {
        if let Some(kept) = self.spare.take() {
            return kept;
        }
        let radius = (-2.0 * self.open().ln()).sqrt();
        let angle = TAU * self.unit();
        self.spare = Some(radius * angle.sin());
        radius * angle.cos()
    }
}

/// The top fifty-three bits, plus one, over 2⁵³: `(0, 1]` in even steps.
fn open_from(bits: u64) -> f64 {
    ((bits >> 11) + 1) as f64 / (1u64 << 53) as f64
}

/// The top fifty-three bits over 2⁵³: `[0, 1)` in even steps.
fn unit_from(bits: u64) -> f64 {
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

/// How many octaves of pink noise a [`Pink`] makes below half the rate.
pub(crate) const PINK_ROWS: usize = 16;

/// Pink noise by Voss and McCartney: the sum of sixteen random values, row
/// `k` drawn afresh every `2^(k+1)` samples, and one drawn every sample.
///
/// A value held for `L` samples has a flat spectrum up to about `rate/2L`
/// and little above it, so rows held for 2, 4, 8… samples stack into a
/// spectrum that doubles for every halving of frequency — equal power in
/// every octave, which is 1/f. The rows take turns, one per sample (the row
/// whose number is how many times two divides the sample's index), so no
/// sample changes more than two terms of the sum.
///
/// It is stationary from the first sample, because every row starts with a
/// value of its own rather than at zero; a filter shaping white noise into
/// pink would need its slowest pole's worth of samples to settle first.
/// Every term has unit variance, so the sum over `√17` does too.
#[derive(Debug, Clone)]
pub(crate) struct Pink {
    stream: Stream,
    rows: [f64; PINK_ROWS],
    index: u64,
}

impl Pink {
    pub(crate) fn new(mut stream: Stream) -> Self {
        let mut rows = [0.0; PINK_ROWS];
        rows.iter_mut().for_each(|row| *row = stream.gaussian());
        Pink {
            stream,
            rows,
            index: 0,
        }
    }

    /// The next sample, of unit variance.
    pub(crate) fn sample(&mut self) -> f64 {
        if self.index > 0 {
            let row = self.index.trailing_zeros() as usize;
            if let Some(held) = self.rows.get_mut(row) {
                *held = self.stream.gaussian();
            }
        }
        self.index += 1;
        // Summed afresh rather than kept as a running total, which would
        // gather a rounding per sample for as long as the render runs.
        let held: f64 = self.rows.iter().sum();
        (held + self.stream.gaussian()) / ((PINK_ROWS + 1) as f64).sqrt()
    }
}

/// Spikes as a Poisson process in time: arrivals `rate` a second on
/// average, the gaps between them exponential, each spike `width` seconds of
/// `+1` or `−1`.
#[derive(Debug, Clone)]
pub(crate) struct Spikes {
    stream: Stream,
    rate: f64,
    width: f64,
    /// When the next spike starts, in seconds.
    next: f64,
    /// When each spike under way ends, and its sign.
    live: Vec<(f64, f64)>,
}

impl Spikes {
    pub(crate) fn new(stream: Stream, rate: f64, width: f64) -> Self {
        let mut spikes = Spikes {
            stream,
            rate,
            width,
            next: 0.0,
            live: Vec::new(),
        };
        spikes.next = spikes.gap();
        spikes
    }

    /// The seconds to the next arrival: `−ln(u)/rate`, an exponential
    /// draw, and for ever when none are coming.
    fn gap(&mut self) -> f64 {
        if self.rate > 0.0 && self.rate.is_finite() {
            -self.stream.open().ln() / self.rate
        } else {
            f64::INFINITY
        }
    }

    /// The sum of the signs of every spike under way at `t` seconds. Asked
    /// in order of `t`, which is the order a render asks in.
    pub(crate) fn at(&mut self, t: f64) -> f64 {
        while self.next <= t {
            let sign = if self.stream.next_u64() >> 63 == 0 {
                1.0
            } else {
                -1.0
            };
            self.live.push((self.next + self.width, sign));
            self.next += self.gap();
        }
        self.live.retain(|&(end, _)| end > t);
        self.live.iter().map(|&(_, sign)| sign).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SplitMix64 as its authors wrote it, and as a second implementation
    /// written from their paper computes it: the first five draws from state
    /// 0 and from state 1234567. A change to a constant or a shift, or a
    /// "harmless" refactor of the generator, is a change to every stored
    /// seed's noise, and this is what says so.
    #[test]
    fn the_generator_is_splitmix64_to_the_bit() {
        for (state, draws) in [
            (
                0,
                [
                    0xe220_a839_7b1d_cdaf,
                    0x6e78_9e6a_a1b9_65f4,
                    0x06c4_5d18_8009_454f,
                    0xf88b_b8a8_724c_81ec,
                    0x1b39_896a_51a8_749b,
                ],
            ),
            (
                1_234_567,
                [
                    0x599e_d017_fb08_fc85,
                    0x2c73_f084_5854_0fa5,
                    0x883e_bce5_a3f2_7c77,
                    0x3fbe_f740_e917_7b3f,
                    0xe3b8_3467_08cb_5ecd,
                ],
            ),
        ] {
            let mut stream = Stream::new(state);
            let got: Vec<u64> = (0..5).map(|_| stream.next_u64()).collect();
            assert_eq!(got, draws, "state {state}");
        }
    }

    /// The two ends of the two uniforms. `open` feeds a logarithm, so zero
    /// is the one value it must never produce; `unit` is a fraction of a
    /// turn, so one would be a whole turn counted twice.
    #[test]
    fn the_uniforms_stay_inside_their_ends() {
        assert_eq!(open_from(0), 1.0 / (1u64 << 53) as f64);
        assert_eq!(open_from(u64::MAX), 1.0);
        assert_eq!(unit_from(0), 0.0);
        assert!(unit_from(u64::MAX) < 1.0);
        assert_eq!(unit_from(u64::MAX), 1.0 - 1.0 / (1u64 << 53) as f64);
    }

    /// The first normal draws of stream `(7, White, 0)`, worked out
    /// independently: the same splitmix, key and Box–Muller written again
    /// in Python, with its own integers and its own `math.log` and
    /// `math.cos`. The tolerance is for the last bit of a logarithm or a
    /// cosine, which is each platform's own; anything larger is the stream
    /// having changed under every seed somebody stored.
    #[test]
    fn a_seed_draws_the_normals_it_always_drew() {
        let mut stream = stream(7, Noise::White, 0);
        let got: Vec<f64> = (0..4).map(|_| stream.gaussian()).collect();
        let want = [
            0.5068534246538531,
            1.8073957446966165,
            -0.02080817852838921,
            0.30512928233335745,
        ];
        for (got, want) in got.iter().zip(want) {
            assert!((got - want).abs() < 1e-12, "{got} against {want}");
        }
    }

    /// Streams for neighbouring seeds, kinds and ordinals are unrelated:
    /// none of their first draws coincide. The key once XORed the seed and
    /// the ordinal side by side, and nine of these forty-eight streams were
    /// another one's under a different seed.
    #[test]
    fn neighbouring_streams_share_nothing() {
        let mut firsts = Vec::new();
        for seed in 0..4 {
            for noise in [Noise::White, Noise::Pink, Noise::Spikes] {
                for ordinal in 0..4 {
                    firsts.push(stream(seed, noise, ordinal).next_u64());
                }
            }
        }
        let count = firsts.len();
        firsts.sort_unstable();
        firsts.dedup();
        assert_eq!(firsts.len(), count);
    }
}
