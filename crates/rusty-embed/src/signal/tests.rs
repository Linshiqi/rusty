//! What a render is held to: the formula at every sample, the statistics
//! each kind of noise is named for, and the same noise from the same seed.

use std::f64::consts::TAU;

use super::*;

fn render(text: &str, rate: f64, count: usize, seed: u64) -> Vec<f64> {
    Signal::parse(text)
        .expect("reads")
        .render(rate, count, seed)
}

/// A sine at 1234 Hz sampled at 48 kHz, three million samples in — about
/// a minute of it — is still the formula at every sample, worked out here in
/// integers where nothing rounds. The same render computed as
/// `n·(f/rate)` is shown failing the same bound, so the bound is one a
/// drifting phase cannot meet.
#[test]
fn a_sine_is_its_formula_at_every_sample_of_a_long_render() {
    let (rate, count) = (48_000.0, 3_000_000);
    let samples = render("sine f=1234 a=0.8 ph=30", rate, count, 0);
    let mut worst = 0.0f64;
    let mut naive = 0.0f64;
    for (n, got) in samples.iter().enumerate() {
        let turn = ((n as u64 * 1234) % 48_000) as f64 / 48_000.0 + 30.0 / 360.0;
        let want = 0.8 * (TAU * turn).sin();
        worst = worst.max((got - want).abs());
        let drifting = 0.8 * (TAU * (n as f64 * (1234.0 / rate) + 30.0 / 360.0)).sin();
        naive = naive.max((drifting - want).abs());
    }
    assert!(worst < 1e-12, "off the formula by {worst}");
    assert!(naive > 1e-12, "the bound does not tell a drift: {naive}");
}

/// At eight samples a period every waveform lands on the eighths of its
/// cycle, where its closed form is a short binary fraction — so these are
/// exact, and a quarter turn of phase is exactly two samples later in the
/// cycle for every one of them.
#[test]
fn the_waveforms_are_their_closed_forms_at_the_eighths() {
    let eighths = |text: &str| render(text, 8.0, 8, 0);
    assert_eq!(
        eighths("square f=1 a=2 duty=0.25"),
        [2.0, 2.0, -2.0, -2.0, -2.0, -2.0, -2.0, -2.0]
    );
    assert_eq!(
        eighths("square f=1 a=2"),
        [2.0, 2.0, 2.0, 2.0, -2.0, -2.0, -2.0, -2.0]
    );
    assert_eq!(
        eighths("triangle f=1 a=2"),
        [0.0, 1.0, 2.0, 1.0, 0.0, -1.0, -2.0, -1.0]
    );
    assert_eq!(
        eighths("sawtooth f=1 a=2"),
        [0.0, 0.5, 1.0, 1.5, -2.0, -1.5, -1.0, -0.5]
    );
    for kind in ["square f=1 a=2", "triangle f=1 a=2", "sawtooth f=1 a=2"] {
        let plain = eighths(kind);
        let turned = eighths(&format!("{kind} ph=90"));
        for n in 0..8 {
            assert_eq!(turned[n], plain[(n + 2) % 8], "{kind} at {n}");
        }
    }
    let sine = eighths("sine f=1 a=2");
    for (n, got) in sine.iter().enumerate() {
        assert!((got - 2.0 * (TAU * n as f64 / 8.0).sin()).abs() < 1e-15);
    }
}

/// The frequency a sweep is at, read off the gap between its rising zero
/// crossings, against `f₀ + (f₁ − f₀)τ/T` for a straight sweep and
/// `f₀(f₁/f₀)^(τ/T)` for one by ratios — from `f₀` at the start of each
/// sweep to `f₁` at its end, and back at `f₀` when the next one begins.
///
/// A gap is the frequency averaged over one cycle, which is the frequency
/// at the cycle's middle exactly for a straight sweep and to within
/// `(gΔt)²/24` for one growing at `g` nepers a second: 0.2% for the first
/// tenth-of-a-second cycle of the ratio sweep here, hence the bound.
#[test]
fn a_chirp_sweeps_from_where_it_starts_to_where_it_ends() {
    sweeps_as("chirp from=10 to=100 t=2 a=1", 100.0, |tau| {
        10.0 + 90.0 * tau / 2.0
    });
    sweeps_as("chirp from=10 to=1000 t=2 a=1 log", 1000.0, |tau| {
        10.0 * 100f64.powf(tau / 2.0)
    });
    // Each sweep starts from phase zero.
    let samples = render("chirp from=10 to=100 t=2 a=1", 20_000.0, 40_001, 0);
    assert_eq!(samples[0], 0.0);
    assert!(samples[40_000].abs() < 1e-9);
}

/// Two sweeps of `text` — ten hertz to `top` over two seconds — heard cycle
/// by cycle against the frequency `at` says it has `τ` seconds in.
fn sweeps_as(text: &str, top: f64, at: fn(f64) -> f64) {
    let rate = 20_000.0;
    let samples = render(text, rate, 80_000, 0);
    let rising: Vec<f64> = samples
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0] < 0.0 && pair[1] >= 0.0)
        .map(|(n, pair)| (n as f64 + pair[0] / (pair[0] - pair[1])) / rate)
        .collect();
    let mut heard = Vec::new();
    for pair in rising.windows(2) {
        // A cycle that straddles the restart belongs to neither sweep.
        if (pair[0] / 2.0).floor() != (pair[1] / 2.0).floor() {
            continue;
        }
        let middle = (pair[0] + pair[1]) / 2.0;
        let frequency = 1.0 / (pair[1] - pair[0]);
        let want = at(middle % 2.0);
        assert!(
            (frequency / want - 1.0).abs() < 5e-3,
            "{text}: {frequency} Hz at {middle} s, not {want}"
        );
        heard.push((middle, frequency));
    }
    assert!(heard.len() > 100, "{text}: {} cycles read", heard.len());
    // Near the bottom at the start of each sweep and at the top at its end:
    // the first cycle is a tenth of the way up at most.
    let near_bottom = 10.0..10.0 + 0.1 * (top - 10.0);
    let first = heard.first().expect("cycles").1;
    let last = heard.iter().rfind(|(middle, _)| *middle < 2.0);
    let again = heard.iter().find(|(middle, _)| *middle > 2.0);
    let (last, again) = (last.expect("a sweep").1, again.expect("a second").1);
    assert!(near_bottom.contains(&first), "{text} starts at {first}");
    assert!(last > 0.98 * top, "{text} ends at {last}");
    assert!(near_bottom.contains(&again), "{text} restarts at {again}");
}

#[test]
fn a_step_is_nothing_before_its_time_and_its_size_after() {
    let samples = render("step at=0.25 size=-3", 100.0, 50, 0);
    assert!(samples[..25].iter().all(|&v| v == 0.0));
    assert!(samples[25..].iter().all(|&v| v == -3.0));
}

/// White noise is what it says: an RMS of what was asked, a mean of
/// nothing, and — being Gaussian — 68.27% of it within one RMS of zero,
/// which uniform noise of the same RMS (57.7%) would miss by a mile.
#[test]
fn white_noise_is_gaussian_with_the_rms_it_was_given() {
    let count = 200_000;
    let samples = render("white rms=0.5", 1000.0, count, 42);
    let n = count as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let rms = (samples.iter().map(|v| v * v).sum::<f64>() / n).sqrt();
    let within = samples.iter().filter(|v| v.abs() <= 0.5).count() as f64 / n;
    // Four standard errors each: 0.5/√n, 1/√(2n) of the RMS, √(p(1−p)/n).
    assert!(mean.abs() < 4.0 * 0.5 / n.sqrt(), "mean {mean}");
    assert!(
        (rms / 0.5 - 1.0).abs() < 4.0 / (2.0 * n).sqrt(),
        "rms {rms}"
    );
    let p = 0.682_689_492;
    assert!(
        (within - p).abs() < 4.0 * (p * (1.0 - p) / n).sqrt(),
        "{within} within one RMS"
    );
}

/// The same seed, the same noise; another seed, other noise; a signal
/// with no noise in it, the same whatever the seed.
#[test]
fn a_seed_decides_the_noise_and_nothing_else() {
    let noisy = "dc 1; white rms=0.1; pink rms=0.1; spikes rate=50 a=1 w=0.002";
    assert_eq!(
        render(noisy, 1000.0, 5000, 7),
        render(noisy, 1000.0, 5000, 7)
    );
    assert_ne!(
        render(noisy, 1000.0, 5000, 7),
        render(noisy, 1000.0, 5000, 8)
    );
    let quiet = "sine f=3 a=1; square f=1 a=0.5";
    assert_eq!(
        render(quiet, 1000.0, 500, 1),
        render(quiet, 1000.0, 500, 99)
    );
}

/// A render's first samples do not depend on how many follow: the preview
/// the frontend draws of the first second is the first second of the
/// minute the emulator plays.
#[test]
fn a_short_render_is_the_start_of_a_long_one() {
    let noisy = "dc 1; white rms=0.1; pink rms=0.1; spikes rate=50 a=1 w=0.002";
    let long = render(noisy, 1000.0, 20_000, 3);
    let short = render(noisy, 1000.0, 1000, 3);
    assert_eq!(short, long[..1000]);
}

/// Each noise draws from a stream of its own kind and place, so moving or
/// adding a component of another kind leaves it alone — the sums below are
/// the same additions in the other order, which floating point keeps exact.
#[test]
fn noise_does_not_change_when_other_kinds_move_around_it() {
    let rate = 1000.0;
    assert_eq!(
        render("dc 3; white rms=0.5", rate, 4000, 11),
        render("white rms=0.5; dc 3", rate, 4000, 11)
    );
    assert_eq!(
        render("pink rms=0.2; sine f=5 a=1", rate, 4000, 11),
        render("sine f=5 a=1; pink rms=0.2", rate, 4000, 11)
    );
    // The second white noise is a stream of its own, not the first again.
    let one = render("white rms=1", rate, 4000, 11);
    let two = render("white rms=1; white rms=1", rate, 4000, 11);
    let second: Vec<f64> = two.iter().zip(&one).map(|(b, a)| b - a).collect();
    let alike = second
        .iter()
        .zip(&one)
        .filter(|(a, b)| (*a - *b).abs() < 1e-9);
    assert!(alike.count() < 10);
}

/// Half the mean square of the step between neighbouring block means —
/// the Allan variance — at `τ` samples a block.
fn allan(samples: &[f64], tau: usize) -> f64 {
    let means: Vec<f64> = samples
        .chunks_exact(tau)
        .map(|block| block.iter().sum::<f64>() / tau as f64)
        .collect();
    let steps: f64 = means
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).powi(2))
        .sum();
    steps / (2.0 * (means.len() - 1) as f64)
}

/// Pink noise has the same power in every octave, and the Allan variance
/// says so without a spectrum: for noise falling as 1/f it is the same at
/// every block length, where for white noise it is exactly σ²/τ — halving
/// with every doubling. Both are held to their closed forms here, over ten
/// octaves of block length, each estimate from at least a thousand blocks
/// so its own scatter is a few percent.
#[test]
fn pink_noise_has_the_same_power_in_every_octave() {
    let count = 1 << 21;
    let pink = render("pink rms=1", 1.0, count, 5);
    let white = render("white rms=1", 1.0, count, 5);
    let rms = (pink.iter().map(|v| v * v).sum::<f64>() / count as f64).sqrt();
    assert!((rms - 1.0).abs() < 0.05, "pink rms {rms}");

    let taus: Vec<usize> = (1..=11).map(|octave| 1 << octave).collect();
    let flat: Vec<f64> = taus.iter().map(|&tau| allan(&pink, tau)).collect();
    let (low, high) = flat
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    assert!(high / low < 1.3, "pink's Allan variance moves: {flat:?}");
    for &tau in &taus {
        let ratio = allan(&white, tau) * tau as f64;
        assert!((ratio - 1.0).abs() < 0.15, "white at {tau}: {ratio}");
    }
}

/// Spikes arrive `rate` a second — a Poisson count, so within four of its
/// standard deviations, √N — each as many samples wide as `width` covers,
/// and heads as often as tails.
#[test]
fn spikes_come_as_often_and_as_wide_as_they_were_asked() {
    let rate = 1000.0;
    let samples = render("spikes rate=2 a=0.5 w=0.004", rate, 1_000_000, 9);
    // Runs of samples under a spike, and the sign each run has.
    let mut runs: Vec<(usize, f64)> = Vec::new();
    let mut length = 0;
    for pair in samples.windows(2) {
        if pair[1] != 0.0 {
            length += 1;
        } else if length > 0 {
            runs.push((length, pair[0]));
            length = 0;
        }
    }
    let expected = 2.0 * 1000.0;
    let count = runs.len() as f64;
    assert!(
        (count - expected).abs() < 4.0 * expected.sqrt(),
        "{count} spikes in 1000 s"
    );
    // Four samples fall under four milliseconds wherever it starts; two
    // spikes that overlap make a longer run, rarely.
    let four = runs.iter().filter(|&&(len, _)| len == 4).count() as f64;
    assert!(four / count > 0.98, "{four} of {count} four samples wide");
    let heads = runs.iter().filter(|&&(_, sign)| sign > 0.0).count() as f64;
    assert!((heads - count / 2.0).abs() < 4.0 * (count / 4.0).sqrt());
    assert!(runs.iter().all(|&(_, v)| v.abs() == 0.5 || v.abs() == 1.0));
}

/// The spikes are instants in time, not samples, so a render at ten times
/// the rate, taken every tenth sample, is the render at the rate — to the
/// bit, since `10n/10000` and `n/1000` are the same division.
#[test]
fn spikes_are_in_the_same_places_at_any_rate() {
    let text = "spikes rate=5 a=1 w=0.003";
    let coarse = render(text, 1000.0, 20_000, 4);
    let fine = render(text, 10_000.0, 200_000, 4);
    let tenth: Vec<f64> = fine.iter().step_by(10).copied().collect();
    assert_eq!(coarse, tenth);
    assert!(coarse.iter().filter(|v| **v != 0.0).count() > 200);
}

#[test]
fn a_rate_that_is_not_one_renders_nothing() {
    for rate in [0.0, -1000.0, f64::NAN, f64::INFINITY] {
        assert!(render("sine f=1 a=1", rate, 100, 0).is_empty());
    }
    assert_eq!(Signal::default().render(1000.0, 3, 0), [0.0, 0.0, 0.0]);
}

/// Every preset reads, reads back as written, and has a name of its own
/// for the frontend to translate.
#[test]
fn every_preset_reads_back_as_written() {
    let presets = presets();
    assert_eq!(presets.len(), PRESETS.len());
    for (preset, (id, text)) in presets.iter().zip(PRESETS) {
        assert_eq!(preset.id, id);
        assert_eq!(preset.signal.to_string(), text, "{id}");
        assert!(!preset.signal.components.is_empty());
    }
    let mut ids: Vec<&str> = presets.iter().map(|p| p.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), presets.len());
}
