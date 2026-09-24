//! Every design against the formula it is meant to be.
//!
//! The recursive ones are held to their analog prototypes at the warped
//! frequency `tan(πf/rate)/tan(πf_c/rate)`, which the bilinear transform
//! makes exact rather than approximate — so these are equalities to the
//! last few digits, across the whole band, not tolerances around a curve.

use std::f64::consts::{FRAC_1_SQRT_2, PI};

use super::*;
use crate::dsp::db;

const RATE: f64 = 1000.0;

fn realized(design: Design) -> Filter {
    design.realize(RATE).expect("realises")
}

fn gain(filter: &Filter, freq: f64) -> f64 {
    filter.response(freq).expect("linear").gain
}

fn phase(filter: &Filter, freq: f64) -> f64 {
    filter.response(freq).expect("linear").phase
}

/// The frequency the analog prototype is asked at.
fn warped(freq: f64, cutoff: f64) -> f64 {
    (PI * freq / RATE).tan() / (PI * cutoff / RATE).tan()
}

/// Half a hertz in from every whole one across the band, clear of the
/// ends where a closed form can be nought over nought.
fn across() -> impl Iterator<Item = f64> {
    (1..500).map(|f| f as f64 - 0.5)
}

fn close(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-13 + 1e-9 * want.abs()
}

/// The angle between two phases, whichever way round is shorter.
fn apart(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(2.0 * PI);
    d.min(2.0 * PI - d)
}

/// `|H|² = 1/(1 + Ω^2N)` at every order, which puts half the power —
/// −3.01 dB — at the cutoff whatever the order and unity at DC; and far
/// enough below half the rate that the transform hardly warps, the fourth
/// order's attenuation at twice its cutoff is the analog one,
/// `10·log₁₀(1 + 2⁸)` = 24.0993 dB.
#[test]
fn a_butterworth_low_pass_is_its_closed_form_at_every_order() {
    for order in 1..=8 {
        let filter = realized(Design::Butterworth {
            pass: Pass::Low,
            order,
            cutoff: 50.0,
        });
        assert!(close(gain(&filter, 0.0), 1.0), "order {order} at DC");
        assert!((gain(&filter, 50.0).powi(2) - 0.5).abs() < 1e-12);
        assert!((db(gain(&filter, 50.0)) + 3.010_299_956_639_812).abs() < 1e-9);
        for f in across() {
            let want = 1.0 / (1.0 + warped(f, 50.0).powi(2 * order as i32)).sqrt();
            let got = gain(&filter, f);
            assert!(close(got, want), "order {order} at {f}: {got}, not {want}");
        }
    }
    let far = Design::Butterworth {
        pass: Pass::Low,
        order: 4,
        cutoff: 10.0,
    }
    .realize(10_000.0)
    .expect("realises");
    let at_twice = db(gain(&far, 20.0));
    assert!((at_twice + 24.0993).abs() < 0.01, "{at_twice} dB");
}

/// The high-pass is the low-pass mirrored, `|H|² = 1/(1 + Ω^−2N)`: half
/// power at the cutoff, unity at half the rate, nothing at DC, and 24.1 dB
/// down at half its cutoff at the fourth order.
#[test]
fn a_butterworth_high_pass_mirrors_the_low_pass() {
    for order in 1..=8 {
        let filter = realized(Design::Butterworth {
            pass: Pass::High,
            order,
            cutoff: 50.0,
        });
        assert!(gain(&filter, 0.0) < 1e-15, "order {order} at DC");
        assert!(close(gain(&filter, RATE / 2.0), 1.0));
        assert!((gain(&filter, 50.0).powi(2) - 0.5).abs() < 1e-12);
        for f in across() {
            let want = 1.0 / (1.0 + warped(f, 50.0).powi(-2 * order as i32)).sqrt();
            let got = gain(&filter, f);
            assert!(close(got, want), "order {order} at {f}: {got}, not {want}");
        }
    }
    let far = Design::Butterworth {
        pass: Pass::High,
        order: 4,
        cutoff: 20.0,
    }
    .realize(10_000.0)
    .expect("realises");
    let at_half = db(gain(&far, 10.0));
    assert!((at_half + 24.0993).abs() < 0.01, "{at_half} dB");
}

/// Each cookbook section is its analog prototype, `N(Ω)/|1 − Ω² + iΩ/Q|`
/// with `N` one, `Ω²`, `Ω/Q` or `|1 − Ω²|`, at every Q — which puts a
/// low-pass or high-pass of Q = 1/√2 at −3.01 dB at its cutoff, a band-pass
/// at 0 dB at its centre, and a notch's centre at nothing while a decade
/// below it is all but untouched.
#[test]
fn the_cookbook_biquads_are_their_prototypes_warped() {
    let kinds = [
        BiquadKind::LowPass,
        BiquadKind::HighPass,
        BiquadKind::BandPass,
        BiquadKind::Notch,
    ];
    for q in [0.5, FRAC_1_SQRT_2, 2.0, 10.0] {
        for kind in kinds {
            let filter = realized(Design::Biquad {
                kind,
                cutoff: 50.0,
                q,
            });
            for f in across() {
                let w = warped(f, 50.0);
                let under = ((1.0 - w * w).powi(2) + (w / q).powi(2)).sqrt();
                let over = match kind {
                    BiquadKind::LowPass => 1.0,
                    BiquadKind::HighPass => w * w,
                    BiquadKind::BandPass => w / q,
                    BiquadKind::Notch => (1.0 - w * w).abs(),
                };
                let got = gain(&filter, f);
                assert!(close(got, over / under), "{kind:?} Q {q} at {f}: {got}");
            }
        }
    }
    let at = |kind, q, freq| {
        gain(
            &realized(Design::Biquad {
                kind,
                cutoff: 50.0,
                q,
            }),
            freq,
        )
    };
    let half_power = -3.010_299_956_639_812;
    assert!((db(at(BiquadKind::LowPass, FRAC_1_SQRT_2, 50.0)) - half_power).abs() < 1e-9);
    assert!((db(at(BiquadKind::HighPass, FRAC_1_SQRT_2, 50.0)) - half_power).abs() < 1e-9);
    assert!(db(at(BiquadKind::BandPass, 2.0, 50.0)).abs() < 1e-9);
    assert!(db(at(BiquadKind::Notch, FRAC_1_SQRT_2, 50.0)) < -60.0);
    assert!(db(at(BiquadKind::Notch, FRAC_1_SQRT_2, 5.0)).abs() < 0.1);
}

/// A moving average of `N` is the Dirichlet kernel,
/// `|sin(πfN/rate) / (N·sin(πf/rate))|` — nothing at every multiple of
/// `rate/N` — delaying everything by `(N − 1)/2` samples.
#[test]
fn a_moving_average_is_the_dirichlet_kernel() {
    let taps = 8;
    let filter = realized(Design::MovingAverage { taps });
    let n = taps as f64;
    for f in across() {
        let want = ((PI * f * n / RATE).sin() / (n * (PI * f / RATE).sin())).abs();
        assert!(close(gain(&filter, f), want), "{f} Hz");
        if f < RATE / n {
            let delay = -PI * f / RATE * (n - 1.0);
            assert!(apart(phase(&filter, f), delay) < 1e-12, "{f} Hz");
        }
    }
    assert!(close(gain(&filter, 0.0), 1.0));
    for zero in [125.0, 250.0, 375.0] {
        assert!(gain(&filter, zero) < 1e-15, "{zero} Hz");
    }
}

/// `y += α(x − y)` is `α / (1 − (1 − α)e^(−iω))`: unity at DC, and the
/// gain and phase of that one pole everywhere else.
#[test]
fn an_exponential_filter_is_its_one_pole() {
    for alpha in [0.01, 0.2, 1.0] {
        let filter = realized(Design::Exponential { alpha });
        assert!(close(gain(&filter, 0.0), 1.0));
        let keep = 1.0 - alpha;
        for f in across() {
            let w = 2.0 * PI * f / RATE;
            let (re, im) = (1.0 - keep * w.cos(), keep * w.sin());
            assert!(
                close(gain(&filter, f), alpha / re.hypot(im)),
                "α {alpha} at {f}"
            );
            assert!(apart(phase(&filter, f), -im.atan2(re)) < 1e-12);
        }
    }
}

/// The worst gain from `from` to half the rate, in decibels.
fn worst_above(filter: &Filter, from: f64) -> f64 {
    let steps = 2000;
    (0..=steps)
        .map(|i| from + (RATE / 2.0 - from) * i as f64 / steps as f64)
        .map(|f| db(gain(filter, f)))
        .fold(f64::NEG_INFINITY, f64::max)
}

/// A windowed-sinc low-pass sums to one, so DC passes exactly; it is half
/// amplitude at its cutoff; its phase is a straight line, `(N − 1)/2`
/// samples of delay; and its stop band is the window's: 44 dB down past two
/// bins for Hann and 74 past three for Blackman — the textbook figures —
/// where a bare rectangle never gets far past twenty.
#[test]
fn a_windowed_sinc_low_pass_is_its_windows() {
    let (taps, cutoff) = (101, 100.0);
    let bin = RATE / taps as f64;
    let fir = |window| {
        realized(Design::Fir {
            band: Band::LowPass { cutoff },
            taps,
            window,
        })
    };
    for window in [Window::Hann, Window::Blackman] {
        let filter = fir(window);
        assert!((gain(&filter, 0.0) - 1.0).abs() < 1e-14);
        assert!((gain(&filter, cutoff) - 0.5).abs() < 0.01, "{window:?}");
        for f in (1..90).map(f64::from) {
            let delay = -PI * f / RATE * (taps - 1) as f64;
            assert!(apart(phase(&filter, f), delay) < 1e-9, "{window:?} at {f}");
        }
    }
    assert!(worst_above(&fir(Window::Hann), cutoff + 2.0 * bin) < -43.0);
    assert!(worst_above(&fir(Window::Blackman), cutoff + 3.0 * bin) < -74.0);
    assert!(worst_above(&fir(Window::Rectangular), cutoff + 3.0 * bin) > -35.0);
}

/// A high-pass is an impulse less a low-pass, so it takes out DC exactly
/// and passes half the rate; a band-pass is two low-passes' difference, so
/// it takes out DC exactly too and passes its middle. Both are half
/// amplitude at their edges.
#[test]
fn a_windowed_sinc_high_pass_and_band_pass_are_low_passes_combined() {
    let high = realized(Design::Fir {
        band: Band::HighPass { cutoff: 200.0 },
        taps: 61,
        window: Window::Blackman,
    });
    assert!(gain(&high, 0.0) < 1e-15);
    assert!((gain(&high, RATE / 2.0) - 1.0).abs() < 0.01);
    assert!((gain(&high, 200.0) - 0.5).abs() < 0.01);

    let band = realized(Design::Fir {
        band: Band::BandPass {
            low: 100.0,
            high: 300.0,
        },
        taps: 61,
        window: Window::Blackman,
    });
    assert!(gain(&band, 0.0) < 1e-15);
    assert!((gain(&band, 200.0) - 1.0).abs() < 0.01);
    assert!((gain(&band, 100.0) - 0.5).abs() < 0.01);
    assert!((gain(&band, 300.0) - 0.5).abs() < 0.01);
    assert!(db(gain(&band, RATE / 2.0)) < -70.0);
}

/// Each refusal names what it refused.
#[test]
fn what_cannot_be_realised_is_refused_by_name() {
    let refused = |design: Design| design.realize(RATE).expect_err("refused");
    assert_eq!(
        refused(Design::Butterworth {
            pass: Pass::Low,
            order: 4,
            cutoff: 500.0
        }),
        Refusal::Frequency {
            what: "cutoff",
            freq: 500.0,
            nyquist: 500.0
        }
    );
    assert_eq!(
        refused(Design::Butterworth {
            pass: Pass::Low,
            order: 0,
            cutoff: 50.0
        }),
        Refusal::Order
    );
    assert_eq!(
        refused(Design::MovingAverage { taps: 0 }),
        Refusal::Taps { taps: 0, least: 1 }
    );
    assert_eq!(
        refused(Design::Median { taps: 0 }),
        Refusal::Taps { taps: 0, least: 1 }
    );
    assert_eq!(
        refused(Design::Exponential { alpha: 1.5 }),
        Refusal::Alpha { alpha: 1.5 }
    );
    assert_eq!(
        refused(Design::Biquad {
            kind: BiquadKind::Notch,
            cutoff: 50.0,
            q: 0.0
        }),
        Refusal::Q { q: 0.0 }
    );
    assert_eq!(
        refused(Design::Fir {
            band: Band::HighPass { cutoff: 100.0 },
            taps: 30,
            window: Window::Hann
        }),
        Refusal::Even { taps: 30 }
    );
    assert_eq!(
        refused(Design::Fir {
            band: Band::BandPass {
                low: 200.0,
                high: 100.0
            },
            taps: 31,
            window: Window::Hann
        }),
        Refusal::Edges {
            low: 200.0,
            high: 100.0
        }
    );
    assert_eq!(
        refused(Design::Fir {
            band: Band::LowPass { cutoff: 100.0 },
            taps: 2,
            window: Window::Hann
        }),
        Refusal::Taps { taps: 2, least: 3 }
    );
    assert_eq!(
        Design::Median { taps: 3 }.realize(0.0),
        Err(Refusal::Rate { rate: 0.0 })
    );
}

/// A cutoff a few millionths of the rate puts the poles closer to one
/// than a double can say, and the sections that come out no longer have
/// unit gain at DC. That is refused, not handed over: at 10⁻⁷ of the rate
/// the gain is off by more than a part in a million, where at 10⁻⁵ the
/// same design still holds.
#[test]
fn sections_that_cannot_hold_their_design_are_refused() {
    let low = |cutoff| Design::Butterworth {
        pass: Pass::Low,
        order: 2,
        cutoff,
    };
    assert!(matches!(
        low(1e-4).realize(RATE),
        Err(Refusal::Inexact { .. })
    ));
    assert!(low(1e-2).realize(RATE).is_ok());
    assert!(matches!(
        Design::Exponential { alpha: 1e-17 }.realize(RATE),
        Err(Refusal::Inexact { .. })
    ));
}

/// The wire form a stored design has: tagged by `design`, with the
/// cookbook's kind, a band's shape and a window in kebab case.
#[test]
fn a_design_is_stored_as_it_reads() {
    let designs = [
        Design::Butterworth {
            pass: Pass::High,
            order: 3,
            cutoff: 20.0,
        },
        Design::Biquad {
            kind: BiquadKind::BandPass,
            cutoff: 60.0,
            q: 2.0,
        },
        Design::Fir {
            band: Band::BandPass {
                low: 40.0,
                high: 120.0,
            },
            taps: 61,
            window: Window::Blackman,
        },
        Design::MovingAverage { taps: 8 },
        Design::Exponential { alpha: 0.25 },
        Design::Median { taps: 5 },
    ];
    for design in designs {
        let wire = serde_json::to_value(&design).expect("serialises");
        let back: Design = serde_json::from_value(wire).expect("deserialises");
        assert_eq!(back, design);
    }
    let wire = serde_json::to_value(Design::Fir {
        band: Band::HighPass { cutoff: 5.0 },
        taps: 31,
        window: Window::Hann,
    })
    .expect("serialises");
    assert_eq!(
        wire,
        serde_json::json!({
            "design": "fir",
            "band": { "shape": "high-pass", "cutoff": 5.0 },
            "taps": 31,
            "window": "hann"
        })
    );
    let wire = serde_json::to_value(Design::Biquad {
        kind: BiquadKind::LowPass,
        cutoff: 50.0,
        q: 0.5,
    })
    .expect("serialises");
    assert_eq!(wire["design"], "biquad");
    assert_eq!(wire["kind"], "low-pass");
}
