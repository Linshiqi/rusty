//! How long a table has to be to loop without a seam.
//!
//! A rendered table played over and over has its last sample followed by
//! its first, and that joint is invisible only when the table holds a whole
//! number of periods of every periodic component *and* a whole number of
//! samples. A sine cut at seven tenths of a cycle jumps at every loop — tens
//! of times a second on a short table — and a filter under test sees a burst
//! of broadband energy the signal does not have.
//!
//! So the length is a common multiple, found exactly: every frequency and
//! period becomes the fraction it was typed as (`50 Hz` → 50/1, `0.3 Hz` →
//! 3/10), and the least common multiple of fractions is the least common
//! multiple of their numerators over the greatest common divisor of their
//! denominators. Found in floating point, "a whole number of periods" would
//! be a tolerance somebody had to choose; found this way, ten seconds is a
//! whole number of 0.3 Hz periods because 3/10 × 10 is 3.

use super::{Component, Signal};

impl Signal {
    /// The fewest samples at `rate`, lasting from `min` to `max` seconds,
    /// after which every periodic component is back where it started — the
    /// length a table can loop at without a seam — or nothing when there is
    /// no such length.
    ///
    /// Nothing is an answer: a step that has not happened yet cannot happen
    /// again at the joint, and frequencies with no common period short of
    /// `max` have none a table could hold. The caller then renders `max`
    /// and knows the joint is there, rather than being told it is not.
    /// Noise has no period and asks for none; whether the loop replays it
    /// is [`Signal::is_random`]'s answer, not this one's.
    pub fn loop_length(&self, rate: f64, min: f64, max: f64) -> Option<usize> {
        if !(min.is_finite() && max.is_finite() && 0.0 <= min && min <= max) {
            return None;
        }
        let rate = Ratio::of(rate)?;
        // One sample is a period like any other: the loop has to be a
        // whole number of them as well.
        let mut every = rate.recip();
        for component in &self.components {
            let period = match *component {
                Component::Sine { freq, .. }
                | Component::Square { freq, .. }
                | Component::Triangle { freq, .. }
                | Component::Sawtooth { freq, .. } => Ratio::of(freq)?.recip(),
                Component::Chirp { period, .. } => Ratio::of(period)?,
                Component::Step { at, size } if at > 0.0 && size != 0.0 => return None,
                _ => continue,
            };
            every = every.lcm(period)?;
        }
        // `every` is a whole number of samples by construction; how many is
        // `every · rate`, and in integers it divides exactly.
        let top = every.num.checked_mul(rate.num)?;
        let bottom = every.den.checked_mul(rate.den)?;
        if top % bottom != 0 {
            return None;
        }
        let samples = top / bottom;
        // The bounds in samples, with a hair of room either way: `0.3 · 1000`
        // is 299.99999999999997 in floating point, and a loop of exactly
        // 300 samples is not longer than 0.3 s.
        let least = (min * rate.value() * (1.0 - 1e-12)).ceil() as u128;
        let most = (max * rate.value() * (1.0 + 1e-12)).floor() as u128;
        let total = least.div_ceil(samples).max(1).checked_mul(samples)?;
        if total > most {
            return None;
        }
        usize::try_from(total).ok()
    }
}

/// A positive fraction in lowest terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ratio {
    num: u128,
    den: u128,
}

/// The largest numerator or denominator a double can stand for exactly.
const EXACT: u128 = 1 << 53;

impl Ratio {
    fn new(num: u128, den: u128) -> Ratio {
        let common = gcd(num, den);
        Ratio {
            num: num / common,
            den: den / common,
        }
    }

    /// The fraction a positive number stands for: the first convergent of
    /// its continued fraction within a part in 10¹² of it.
    ///
    /// `0.3` is not three tenths in binary, but three tenths is the first
    /// fraction that close to it, which is what a person who typed `0.3`
    /// meant. A number no fraction with parts below 2⁵³ comes that close
    /// to has no period a table could hold.
    fn of(x: f64) -> Option<Ratio> {
        if !(x.is_finite() && x > 0.0) {
            return None;
        }
        // h/k is the latest convergent, the one before it beside it.
        let (mut h, mut h_before) = (1u128, 0u128);
        let (mut k, mut k_before) = (0u128, 1u128);
        let mut rest = x;
        for _ in 0..64 {
            let whole = rest.floor();
            if whole >= EXACT as f64 {
                return None;
            }
            let a = whole as u128;
            let next_h = a.checked_mul(h)?.checked_add(h_before)?;
            let next_k = a.checked_mul(k)?.checked_add(k_before)?;
            if next_h > EXACT || next_k > EXACT {
                return None;
            }
            (h_before, h, k_before, k) = (h, next_h, k, next_k);
            if h > 0 && (x - h as f64 / k as f64).abs() <= 1e-12 * x {
                return Some(Ratio::new(h, k));
            }
            let part = rest - whole;
            if part <= 0.0 {
                return None;
            }
            rest = 1.0 / part;
        }
        None
    }

    fn recip(self) -> Ratio {
        Ratio {
            num: self.den,
            den: self.num,
        }
    }

    /// The least common multiple of two fractions in lowest terms: the
    /// least common multiple of the numerators over the greatest common
    /// divisor of the denominators, itself in lowest terms. Nothing when it
    /// outgrows the integers.
    fn lcm(self, other: Ratio) -> Option<Ratio> {
        let num = (self.num / gcd(self.num, other.num)).checked_mul(other.num)?;
        Some(Ratio {
            num,
            den: gcd(self.den, other.den),
        })
    }

    fn value(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn looped(text: &str, rate: f64, min: f64, max: f64) -> Option<usize> {
        Signal::parse(text)
            .expect("reads")
            .loop_length(rate, min, max)
    }

    /// A table of the length it names, played twice, is the render twice
    /// as long: the joint is invisible because there is no joint.
    fn assert_seamless(text: &str, rate: f64, samples: usize) {
        let signal = Signal::parse(text).expect("reads");
        let twice = signal.render(rate, 2 * samples, 0);
        let (first, second) = twice.split_at(samples);
        let worst = first
            .iter()
            .zip(second)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        assert!(worst < 1e-9, "{text}: the second pass is off by {worst}");
    }

    #[test]
    fn a_decimal_is_the_fraction_that_was_typed() {
        assert_eq!(Ratio::of(50.0), Some(Ratio { num: 50, den: 1 }));
        assert_eq!(Ratio::of(0.3), Some(Ratio { num: 3, den: 10 }));
        assert_eq!(
            Ratio::of(12.345),
            Some(Ratio {
                num: 2469,
                den: 200
            })
        );
        assert_eq!(Ratio::of(0.02), Some(Ratio { num: 1, den: 50 }));
        assert_eq!(Ratio::of(0.0), None);
        assert_eq!(Ratio::of(-1.0), None);
        assert_eq!(Ratio::of(f64::NAN), None);
    }

    /// 50 Hz, 0.3 Hz and 10 Hz come round together every ten seconds:
    /// 3/10 of a hertz is three cycles in ten seconds, and the others fit
    /// ten seconds in whole cycles too.
    #[test]
    fn commensurate_tones_loop_at_their_common_period() {
        let text = "sine f=50 a=1; sine f=0.3 a=0.5; square f=10 a=1 duty=0.25";
        assert_eq!(looped(text, 1000.0, 1.0, 60.0), Some(10_000));
        assert_seamless(text, 1000.0, 10_000);
        // A floor above one period is met by the next whole number of them.
        assert_eq!(looped(text, 1000.0, 25.0, 60.0), Some(30_000));
        // And a ceiling below it has no answer.
        assert_eq!(looped(text, 1000.0, 1.0, 9.0), None);
    }

    /// The loop has to hold whole samples too: 300 Hz at 1 kHz is a period
    /// of three and a third samples, so the loop is ten samples, three
    /// periods.
    #[test]
    fn a_loop_is_a_whole_number_of_samples() {
        assert_eq!(looped("sine f=300 a=1", 1000.0, 0.0, 1.0), Some(10));
        assert_seamless("sine f=300 a=1", 1000.0, 10);
        // A triangle and a sweep, whose periods are a sweep's length.
        let text = "triangle f=4 a=1; chirp from=1 to=50 t=2.5 a=1";
        assert_eq!(looped(text, 1000.0, 0.0, 10.0), Some(2500));
        assert_seamless(text, 1000.0, 2500);
    }

    /// A tone with no common period short of the ceiling, and a step, have
    /// no loop to give; saying one anyway is the seam the caller asked to
    /// avoid.
    #[test]
    fn no_seamless_length_is_said_as_none() {
        // A third of a hertz typed as six decimals comes round every
        // million seconds, not every three.
        assert_eq!(looped("sine f=0.333333 a=1", 1000.0, 1.0, 60.0), None);
        assert_eq!(looped("step at=0.5 size=1", 1000.0, 1.0, 60.0), None);
        // A step at the start is a constant from the first sample.
        assert_eq!(looped("step at=0 size=1", 1000.0, 1.0, 60.0), Some(1000));
    }

    /// Noise asks for no period; the shortest length the floor allows is
    /// the answer, and `is_random` is what says the loop will replay it.
    #[test]
    fn noise_asks_for_no_period_and_says_it_is_random() {
        let signal = Signal::parse("dc 1; white rms=0.1").expect("reads");
        assert_eq!(signal.loop_length(1000.0, 2.0, 60.0), Some(2000));
        assert!(signal.is_random());
        assert!(!Signal::parse("sine f=1 a=1").expect("reads").is_random());
        // Noise with no power draws on the seed for nothing.
        assert!(!Signal::parse("white rms=0").expect("reads").is_random());
    }

    #[test]
    fn a_rate_or_a_range_that_is_not_one_has_no_loop() {
        assert_eq!(looped("sine f=1 a=1", 0.0, 1.0, 60.0), None);
        assert_eq!(looped("sine f=1 a=1", 1000.0, 5.0, 1.0), None);
        assert_eq!(looped("sine f=1 a=1", 1000.0, -1.0, 1.0), None);
    }
}
