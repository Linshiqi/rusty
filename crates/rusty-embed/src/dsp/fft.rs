//! Complex numbers and a radix-2 FFT — the arithmetic under a spectrum and
//! a frequency response, and nothing a crate is worth adding for.

use std::f64::consts::TAU;
use std::ops::{Add, Div, Mul, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Complex {
    pub(crate) re: f64,
    pub(crate) im: f64,
}

impl Complex {
    pub(crate) fn new(re: f64, im: f64) -> Self {
        Complex { re, im }
    }

    /// `e^(2πi·turns)`: a point on the unit circle, a fraction of a turn
    /// round it.
    pub(crate) fn turn(turns: f64) -> Self {
        let (sin, cos) = (TAU * turns).sin_cos();
        Complex { re: cos, im: sin }
    }

    pub(crate) fn norm(self) -> f64 {
        self.re.hypot(self.im)
    }
}

impl Add for Complex {
    type Output = Complex;
    fn add(self, other: Complex) -> Complex {
        Complex::new(self.re + other.re, self.im + other.im)
    }
}

impl Sub for Complex {
    type Output = Complex;
    fn sub(self, other: Complex) -> Complex {
        Complex::new(self.re - other.re, self.im - other.im)
    }
}

impl Mul for Complex {
    type Output = Complex;
    fn mul(self, other: Complex) -> Complex {
        Complex::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }
}

impl Mul<f64> for Complex {
    type Output = Complex;
    fn mul(self, scale: f64) -> Complex {
        Complex::new(self.re * scale, self.im * scale)
    }
}

impl Div for Complex {
    type Output = Complex;
    fn div(self, other: Complex) -> Complex {
        let size = other.re * other.re + other.im * other.im;
        Complex::new(
            (self.re * other.re + self.im * other.im) / size,
            (self.im * other.re - self.re * other.im) / size,
        )
    }
}

/// The discrete Fourier transform in place, `X[k] = Σ x[n]·e^(−2πikn/N)`,
/// for a length that is a power of two — the caller pads to one.
///
/// Cooley–Tukey, decimation in time: the input in bit-reversed order, then
/// butterflies of doubling width. Every twiddle is taken from one table of
/// `e^(−2πik/N)` computed by `sin_cos` rather than by multiplying a unit
/// step into itself, which would carry the step's rounding a little further
/// round the circle with every factor.
pub(crate) fn fft(data: &mut [Complex]) {
    let n = data.len();
    debug_assert!(n.is_power_of_two() || n == 0, "{n} is not a power of two");
    if n < 2 {
        return;
    }
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            data.swap(i, j);
        }
    }
    let twiddles: Vec<Complex> = (0..n / 2)
        .map(|k| Complex::turn(-(k as f64) / n as f64))
        .collect();
    let mut width = 2;
    while width <= n {
        let half = width / 2;
        let stride = n / width;
        for block in data.chunks_exact_mut(width) {
            let (low, high) = block.split_at_mut(half);
            for (k, (a, b)) in low.iter_mut().zip(high.iter_mut()).enumerate() {
                let turned = *b * twiddles[k * stride];
                (*a, *b) = (*a + turned, *a - turned);
            }
        }
        width *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The definition, summed term by term — slow, and nothing to get wrong.
    fn dft(x: &[Complex]) -> Vec<Complex> {
        let n = x.len();
        (0..n)
            .map(|k| {
                x.iter()
                    .enumerate()
                    .fold(Complex::default(), |sum, (i, &v)| {
                        sum + v * Complex::turn(-(((k * i) % n) as f64) / n as f64)
                    })
            })
            .collect()
    }

    /// Numbers with no pattern an FFT could get right by accident.
    fn scattered(n: usize) -> Vec<Complex> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        (0..n).map(|_| Complex::new(next(), next())).collect()
    }

    #[test]
    fn the_fft_is_the_dft_it_stands_for() {
        for n in [2, 4, 8, 64, 256] {
            let x = scattered(n);
            let want = dft(&x);
            let mut got = x.clone();
            fft(&mut got);
            let worst = got
                .iter()
                .zip(&want)
                .map(|(a, b)| (*a - *b).norm())
                .fold(0.0, f64::max);
            assert!(worst < 1e-12, "{n} points: off by {worst}");
        }
    }

    /// One complex exponential at bin 5 of 64 is 64 at bin 5 and nothing
    /// anywhere else; and Parseval holds, the energy the same both sides.
    #[test]
    fn a_tone_lands_in_its_bin_and_energy_is_kept() {
        let n = 64;
        let mut tone: Vec<Complex> = (0..n)
            .map(|i| Complex::turn(5.0 * i as f64 / n as f64))
            .collect();
        fft(&mut tone);
        for (k, bin) in tone.iter().enumerate() {
            let want = if k == 5 { 64.0 } else { 0.0 };
            assert!((bin.norm() - want).abs() < 1e-12, "bin {k}: {bin:?}");
        }

        let x = scattered(1024);
        let before: f64 = x.iter().map(|v| v.norm().powi(2)).sum();
        let mut spectrum = x;
        fft(&mut spectrum);
        let after: f64 = spectrum.iter().map(|v| v.norm().powi(2)).sum::<f64>() / 1024.0;
        assert!((before / after - 1.0).abs() < 1e-12);
    }

    #[test]
    fn division_undoes_multiplication() {
        let (a, b) = (Complex::new(3.0, -2.0), Complex::new(-0.5, 4.0));
        let back = a * b / b;
        assert!((back - a).norm() < 1e-15);
    }
}
