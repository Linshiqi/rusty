//! The Design view's choices: which kind of filter, the parameters each
//! kind has, and a change to one of them — a kind changed keeps the corner
//! it was given, so trying another shape is not starting again.

use crate::dsp::{Band, BiquadKind, Design, Pass, Window};

/// The kinds of filter offered, in the order a person reaches for them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    MovingAverage,
    Exponential,
    ButterLow,
    ButterHigh,
    LowBiquad,
    HighBiquad,
    BandBiquad,
    Notch,
    FirLow,
    FirHigh,
    FirBand,
    Median,
}

impl Kind {
    pub const ALL: [Kind; 12] = [
        Kind::MovingAverage,
        Kind::Exponential,
        Kind::ButterLow,
        Kind::ButterHigh,
        Kind::LowBiquad,
        Kind::HighBiquad,
        Kind::BandBiquad,
        Kind::Notch,
        Kind::FirLow,
        Kind::FirHigh,
        Kind::FirBand,
        Kind::Median,
    ];

    pub fn of(design: &Design) -> Kind {
        match design {
            Design::MovingAverage { .. } => Kind::MovingAverage,
            Design::Exponential { .. } => Kind::Exponential,
            Design::Butterworth {
                pass: Pass::Low, ..
            } => Kind::ButterLow,
            Design::Butterworth {
                pass: Pass::High, ..
            } => Kind::ButterHigh,
            Design::Biquad { kind, .. } => match kind {
                BiquadKind::LowPass => Kind::LowBiquad,
                BiquadKind::HighPass => Kind::HighBiquad,
                BiquadKind::BandPass => Kind::BandBiquad,
                BiquadKind::Notch => Kind::Notch,
            },
            Design::Fir { band, .. } => match band {
                Band::LowPass { .. } => Kind::FirLow,
                Band::HighPass { .. } => Kind::FirHigh,
                Band::BandPass { .. } => Kind::FirBand,
            },
            Design::Median { .. } => Kind::Median,
        }
    }

    /// A design of this kind, keeping what the last one said where it can
    /// mean the same — its corner, its taps — so switching kinds compares
    /// two filters at one frequency rather than starting from nothing.
    pub fn design(self, from: &Design) -> Design {
        let corner = corner_of(from).unwrap_or(10.0);
        let taps = taps_of(from).unwrap_or(31);
        let odd = taps | 1;
        match self {
            Kind::MovingAverage => Design::MovingAverage { taps: taps.min(64) },
            Kind::Exponential => Design::Exponential { alpha: 0.1 },
            Kind::ButterLow | Kind::ButterHigh => Design::Butterworth {
                pass: if self == Kind::ButterLow {
                    Pass::Low
                } else {
                    Pass::High
                },
                order: 2,
                cutoff: corner,
            },
            Kind::LowBiquad | Kind::HighBiquad | Kind::BandBiquad | Kind::Notch => Design::Biquad {
                kind: match self {
                    Kind::LowBiquad => BiquadKind::LowPass,
                    Kind::HighBiquad => BiquadKind::HighPass,
                    Kind::BandBiquad => BiquadKind::BandPass,
                    _ => BiquadKind::Notch,
                },
                cutoff: corner,
                q: std::f64::consts::FRAC_1_SQRT_2,
            },
            Kind::FirLow => Design::Fir {
                band: Band::LowPass { cutoff: corner },
                taps,
                window: Window::Hann,
            },
            Kind::FirHigh => Design::Fir {
                band: Band::HighPass { cutoff: corner },
                taps: odd,
                window: Window::Hann,
            },
            Kind::FirBand => Design::Fir {
                band: Band::BandPass {
                    low: corner / 2.0,
                    high: corner * 2.0,
                },
                taps,
                window: Window::Hann,
            },
            Kind::Median => Design::Median { taps: 5 },
        }
    }
}

pub fn corner_of(design: &Design) -> Option<f64> {
    match *design {
        Design::Biquad { cutoff, .. } | Design::Butterworth { cutoff, .. } => Some(cutoff),
        Design::Fir { band, .. } => Some(match band {
            Band::LowPass { cutoff } | Band::HighPass { cutoff } => cutoff,
            Band::BandPass { low, high } => (low * high).sqrt(),
        }),
        _ => None,
    }
}

pub fn taps_of(design: &Design) -> Option<usize> {
    match *design {
        Design::MovingAverage { taps } | Design::Fir { taps, .. } | Design::Median { taps } => {
            Some(taps)
        }
        _ => None,
    }
}

/// One number a design is made of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Param {
    Taps,
    Alpha,
    Cutoff,
    Q,
    Order,
    Low,
    High,
}

/// The numbers `design` is made of, in the order they are asked for.
pub fn params(design: &Design) -> Vec<(Param, f64)> {
    match *design {
        Design::MovingAverage { taps } | Design::Median { taps } => {
            vec![(Param::Taps, taps as f64)]
        }
        Design::Exponential { alpha } => vec![(Param::Alpha, alpha)],
        Design::Biquad { cutoff, q, .. } => vec![(Param::Cutoff, cutoff), (Param::Q, q)],
        Design::Butterworth { order, cutoff, .. } => {
            vec![(Param::Order, order as f64), (Param::Cutoff, cutoff)]
        }
        Design::Fir { band, taps, .. } => {
            let mut out = vec![(Param::Taps, taps as f64)];
            match band {
                Band::LowPass { cutoff } | Band::HighPass { cutoff } => {
                    out.push((Param::Cutoff, cutoff))
                }
                Band::BandPass { low, high } => {
                    out.push((Param::Low, low));
                    out.push((Param::High, high));
                }
            }
            out
        }
    }
}

/// `design` with one of its numbers changed. A count is a whole number, so
/// what is typed is rounded; whether the result can be realised is
/// `realize`'s to say, not this.
pub fn with(design: &Design, param: Param, value: f64) -> Design {
    let count = value.round().max(0.0) as usize;
    let mut out = design.clone();
    match (&mut out, param) {
        (
            Design::MovingAverage { taps } | Design::Median { taps } | Design::Fir { taps, .. },
            Param::Taps,
        ) => *taps = count,
        (Design::Exponential { alpha }, Param::Alpha) => *alpha = value,
        (Design::Biquad { cutoff, .. } | Design::Butterworth { cutoff, .. }, Param::Cutoff) => {
            *cutoff = value
        }
        (Design::Biquad { q, .. }, Param::Q) => *q = value,
        (Design::Butterworth { order, .. }, Param::Order) => *order = count,
        (Design::Fir { band, .. }, _) => match (band, param) {
            (Band::LowPass { cutoff } | Band::HighPass { cutoff }, Param::Cutoff) => {
                *cutoff = value
            }
            (Band::BandPass { low, .. }, Param::Low) => *low = value,
            (Band::BandPass { high, .. }, Param::High) => *high = value,
            _ => {}
        },
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind reads back as itself, and switching kinds keeps the
    /// corner — a notch put where the low-pass was.
    #[test]
    fn a_kind_keeps_the_corner_it_was_given() {
        let start = Design::Butterworth {
            pass: Pass::Low,
            order: 2,
            cutoff: 12.0,
        };
        for kind in Kind::ALL {
            let made = kind.design(&start);
            assert_eq!(Kind::of(&made), kind, "{made:?}");
            // A band's corner is the middle of its two edges, which is where
            // the band is put: half the corner to twice it.
            if let Some(corner) = corner_of(&made) {
                assert!((corner - 12.0).abs() < 1e-9, "{kind:?}: {corner}");
            }
        }
        // A windowed-sinc high-pass is odd, or `realize` refuses it.
        let high = Kind::FirHigh.design(&Design::MovingAverage { taps: 30 });
        assert!(matches!(high, Design::Fir { taps: 31, .. }), "{high:?}");
    }

    /// Every number a design shows can be changed, and changing one
    /// changes that one only.
    #[test]
    fn a_param_changes_what_it_names() {
        let start = Design::Fir {
            band: Band::BandPass {
                low: 5.0,
                high: 20.0,
            },
            taps: 31,
            window: Window::Hann,
        };
        let shown = params(&start);
        assert_eq!(
            shown,
            [(Param::Taps, 31.0), (Param::Low, 5.0), (Param::High, 20.0)]
        );
        let changed = with(&start, Param::High, 40.0);
        assert_eq!(
            params(&changed),
            [(Param::Taps, 31.0), (Param::Low, 5.0), (Param::High, 40.0)]
        );
        let butter = with(
            &Design::Butterworth {
                pass: Pass::High,
                order: 2,
                cutoff: 5.0,
            },
            Param::Order,
            4.4,
        );
        assert_eq!(params(&butter)[0], (Param::Order, 4.0));
    }
}
