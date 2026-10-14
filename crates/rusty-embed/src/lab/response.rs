//! The Response view's designed curve: the filter's own gain and phase,
//! drawn beside what a sweep measured.

use crate::dsp;

use super::sweep::Point;

/// The design's curve is drawn at this many frequencies across the band.
const CURVE: usize = 240;

/// The design's own gain and phase across `from..=to`, at the rate it runs
/// at — or nothing, for a median or a design that cannot be realised.
pub fn design_curve(design: &dsp::Design, rate: f64, from: f64, to: f64) -> Vec<Point> {
    let Ok(filter) = design.realize(rate) else {
        return Vec::new();
    };
    let top = to.min(rate / 2.0 * 0.999);
    if top <= from {
        return Vec::new();
    }
    let ratio = (top / from).ln() / (CURVE - 1) as f64;
    (0..CURVE)
        .filter_map(|n| {
            let freq = from * (ratio * n as f64).exp();
            let response = filter.response(freq)?;
            Some(Point {
                freq,
                gain: response.gain,
                phase: response.phase,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::{Design, Pass};

    /// The design's curve is its own response: a Butterworth low-pass reads
    /// −3 dB at its cutoff, and a curve asked for past half the rate stops
    /// there.
    #[test]
    fn the_designed_curve_is_the_filters_own() {
        let design = Design::Butterworth {
            pass: Pass::Low,
            order: 2,
            cutoff: 10.0,
        };
        let curve = design_curve(&design, 1000.0, 1.0, 100.0);
        let at_cutoff = curve
            .iter()
            .min_by(|a, b| (a.freq - 10.0).abs().total_cmp(&(b.freq - 10.0).abs()))
            .unwrap();
        assert!((dsp::db(at_cutoff.gain) + 3.0).abs() < 0.2, "{at_cutoff:?}");
        let past = design_curve(&design, 100.0, 1.0, 1000.0);
        assert!(past.iter().all(|p| p.freq < 50.0));
        assert!(design_curve(&Design::Median { taps: 5 }, 1000.0, 1.0, 100.0).is_empty());
    }
}
