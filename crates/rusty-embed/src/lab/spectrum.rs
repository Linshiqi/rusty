//! The spectrum's axis: where a frequency sits across it, on a linear or a
//! log scale, the round frequencies to mark, and the tones a played signal
//! names so the chart can mark where they should be.

use crate::signal::Component;

use super::Played;

/// The frequencies a signal is made of, where it has any to mark: each
/// tone's fundamental. Noise, steps and sweeps have no line of their own.
pub fn tones_of(played: Option<&Played>) -> Vec<f64> {
    let Some(played) = played else {
        return Vec::new();
    };
    played
        .signal
        .components
        .iter()
        .filter_map(|component| match *component {
            Component::Sine { freq, .. }
            | Component::Square { freq, .. }
            | Component::Triangle { freq, .. }
            | Component::Sawtooth { freq, .. } => Some(freq),
            _ => None,
        })
        .collect()
}

/// Where `freq` sits across a chart from `low` to `high` hertz.
pub fn place(freq: f64, low: f64, high: f64, log: bool) -> f64 {
    if log {
        let (low, high) = (low.max(1e-9).ln(), high.max(1e-9).ln());
        (freq.max(1e-9).ln() - low) / (high - low).max(1e-12)
    } else {
        (freq - low) / (high - low).max(1e-12)
    }
}

/// Round numbers along a frequency axis: every decade's 1, 2 and 5 on a
/// logarithmic one, five even steps on a straight one.
pub fn frequency_ticks(low: f64, high: f64, log: bool) -> Vec<f64> {
    if log {
        let mut ticks = Vec::new();
        let mut decade = 10f64.powf(low.max(1e-6).log10().floor());
        while decade <= high {
            for step in [1.0, 2.0, 5.0] {
                let at = decade * step;
                if at >= low && at <= high {
                    ticks.push(at);
                }
            }
            decade *= 10.0;
        }
        ticks
    } else {
        let step = nice_step((high - low) / 5.0);
        let mut at = (low / step).ceil() * step;
        let mut ticks = Vec::new();
        while at <= high + step * 1e-9 {
            ticks.push(at);
            at += step;
        }
        ticks
    }
}

/// 1, 2 or 5 times a power of ten, the nearest at or above `rough`.
fn nice_step(rough: f64) -> f64 {
    let power = 10f64.powf(rough.max(1e-12).log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * power)
        .find(|step| *step >= rough)
        .unwrap_or(10.0 * power)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frequency_is_placed_on_either_axis() {
        assert!((place(250.0, 0.0, 500.0, false) - 0.5).abs() < 1e-12);
        assert!((place(10.0, 1.0, 100.0, true) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn ticks_are_round_numbers() {
        assert_eq!(
            frequency_ticks(0.0, 500.0, false),
            [0.0, 100.0, 200.0, 300.0, 400.0, 500.0]
        );
        assert_eq!(
            frequency_ticks(1.0, 100.0, true),
            [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0]
        );
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(0.07), 0.1);
    }
}
