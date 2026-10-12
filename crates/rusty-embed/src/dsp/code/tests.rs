//! The exports that are checked in, compiled here and run against the
//! arithmetic they were generated from.

use std::path::Path;
use std::process::Command;

use super::*;

/// An IIR, a FIR and a median — one of each shape the generator writes —
/// with the name their struct is exported under and the file it lives in.
fn fixtures() -> [(Filter, &'static str, &'static str, &'static str); 3] {
    let rate = 1000.0;
    let realized = |design: Design| design.realize(rate).expect("realises");
    [
        (
            // Three poles: one first-order section and one second-order.
            realized(Design::Butterworth {
                pass: Pass::Low,
                order: 3,
                cutoff: 50.0,
            }),
            "LowPass",
            "low_pass.rs",
            include_str!("../../../tests/fixtures/dsp/low_pass.rs"),
        ),
        (
            realized(Design::Fir {
                band: Band::LowPass { cutoff: 100.0 },
                taps: 31,
                window: Window::Hann,
            }),
            "Smooth",
            "smooth.rs",
            include_str!("../../../tests/fixtures/dsp/smooth.rs"),
        ),
        (
            realized(Design::Median { taps: 5 }),
            "Despike",
            "despike.rs",
            include_str!("../../../tests/fixtures/dsp/despike.rs"),
        ),
    ]
}

mod low_pass {
    include!("../../../tests/fixtures/dsp/low_pass.rs");
}

mod smooth {
    include!("../../../tests/fixtures/dsp/smooth.rs");
}

mod despike {
    include!("../../../tests/fixtures/dsp/despike.rs");
}

/// The generator still writes what is checked in. When it is meant not
/// to, `RUSTY_DSP_FIXTURES=write cargo test -p rusty-embed dsp::code`
/// writes the new text over the old — the run that writes still fails,
/// having compiled the old one, and the next passes — and the change is a
/// diff to read before it is committed.
#[test]
fn the_generator_still_writes_the_code_checked_in() {
    let write = std::env::var_os("RUSTY_DSP_FIXTURES").is_some();
    let mut stale = Vec::new();
    for (filter, name, file, checked_in) in fixtures() {
        let code = filter.code(name).expect("exports");
        if code != checked_in {
            if write {
                let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/dsp")
                    .join(file);
                std::fs::write(&path, &code).expect("writes the fixture");
            }
            stale.push(format!("tests/fixtures/dsp/{file}:\n\n{code}"));
        }
    }
    assert!(
        stale.is_empty(),
        "the generator no longer writes what is checked in; if that is meant, \
         run with RUSTY_DSP_FIXTURES=write and read the diff\n\n{}",
        stale.join("\n")
    );
}

/// The exports build where firmware builds them: in a crate with no `std`,
/// every warning an error. Compiled in here, they would build even if the
/// generator reached for something only `std` has — `f32::sqrt`, say — so
/// they are handed to `rustc` in a `#![no_std]` library of their own. A
/// machine with no `rustc` to run is said and skipped.
#[test]
fn the_exports_build_without_std() {
    let dir = tempfile::tempdir().expect("a directory");
    let mut source = String::from("#![no_std]\n#![deny(warnings)]\n");
    for (filter, name, file, _) in fixtures() {
        let module = file.trim_end_matches(".rs");
        let code = filter.code(name).expect("exports");
        source.push_str(&format!("pub mod {module} {{\n{code}}}\n"));
    }
    let lib = dir.path().join("lib.rs");
    std::fs::write(&lib, source).expect("writes the library");
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let built = Command::new(&rustc)
        .args([
            "--edition",
            "2024",
            "--crate-type",
            "lib",
            "--emit",
            "metadata",
        ])
        .arg("--out-dir")
        .arg(dir.path())
        .arg(&lib)
        .output();
    match built {
        Ok(built) => assert!(
            built.status.success(),
            "the exports do not build without std:\n{}",
            String::from_utf8_lossy(&built.stderr)
        ),
        Err(error) => eprintln!("skipped: {rustc:?} could not be run: {error}"),
    }
}

/// A signal with something in it for every filter: an offset, two tones,
/// broadband noise and spikes.
fn test_signal() -> Vec<f64> {
    Signal::parse(
        "dc 1.2; sine f=50 a=0.3; sine f=180 a=0.2; white rms=0.1; spikes rate=20 a=2 w=0.002",
    )
    .expect("reads")
    .render(1000.0, 5000, 7)
}

/// Each export, compiled from the file checked in, answers bit for bit
/// what `single` — the arithmetic `code` vets an export by — answers, and
/// within single precision of `apply`: the largest difference over the
/// signal is under a part in 10⁵ of the output's peak, where `f32` carries
/// about seven digits and the sections spend one or two of them.
#[test]
fn the_checked_in_code_computes_what_apply_computes() {
    let input = test_signal();
    let narrow: Vec<f32> = input.iter().map(|&x| x as f32).collect();
    let [iir, fir, median] = fixtures().map(|(filter, ..)| filter);
    let stepped = [
        {
            let mut code = low_pass::LowPass::new();
            narrow.iter().map(|&x| code.step(x)).collect::<Vec<f32>>()
        },
        {
            let mut code = smooth::Smooth::new();
            narrow.iter().map(|&x| code.step(x)).collect()
        },
        {
            let mut code = despike::Despike::default();
            narrow.iter().map(|&x| code.step(x)).collect()
        },
    ];
    for (filter, stepped) in [iir, fir, median].iter().zip(&stepped) {
        assert_eq!(stepped, &single(filter.coefficients(), &narrow));
        let exact = filter.apply(&input);
        let peak = exact.iter().fold(0.0f64, |most, v| most.max(v.abs()));
        let worst = exact
            .iter()
            .zip(stepped)
            .fold(0.0f64, |most, (e, s)| most.max((e - f64::from(*s)).abs()));
        assert!(
            worst < 1e-5 * peak,
            "{:?}: off by {worst} of {peak}",
            filter.design()
        );
    }
}

/// A name the struct cannot have is refused, by the rule the frontend can
/// ask while it is being typed.
#[test]
fn a_name_that_is_not_a_type_is_refused() {
    let filter = Design::Median { taps: 3 }.realize(100.0).expect("realises");
    for name in [
        "lowPass", "Low Pass", "Low_Pass", "", "9Lives", "Self", "Größe",
    ] {
        assert_eq!(
            filter.code(name),
            Err(Refusal::Name {
                name: name.to_string()
            }),
            "{name}"
        );
    }
    for name in ["LowPass", "LPF", "Stage2"] {
        assert!(check_name(name).is_ok(), "{name}");
    }
}

/// Four poles a ten-thousandth of the rate from DC set their gain at DC by
/// how far apart two coefficients are that agree to their seventh digit,
/// which is as far as `f32` goes: in single precision that filter strays
/// by some tens of percent, and the export is refused, naming how far. The
/// same filter at a hundredth of the rate strays by a few parts in 10⁵ and
/// is written.
#[test]
fn a_filter_single_precision_cannot_follow_is_refused() {
    let low = |cutoff| {
        Design::Butterworth {
            pass: Pass::Low,
            order: 4,
            cutoff,
        }
        .realize(10_000.0)
        .expect("realises in f64")
    };
    match low(1.0).code("Slow") {
        Err(Refusal::Precision { error, limit }) => {
            assert!(error > limit, "{error}");
            assert_eq!(limit, LIMIT);
        }
        other => panic!("exported: {other:?}"),
    }
    assert!(low(100.0).code("Quick").is_ok());
}

/// The paragraphs of an export's doc comment, each joined back into one
/// line — and every line of it inside eighty columns.
fn told(code: &str) -> Vec<String> {
    let mut paragraphs = vec![String::new()];
    for line in code.lines().take_while(|line| line.starts_with("///")) {
        assert!(line.len() <= 80, "a doc line past eighty columns: {line}");
        match line.strip_prefix("/// ") {
            Some(words) if paragraphs.last().is_some_and(String::is_empty) => {
                paragraphs.last_mut().expect("a paragraph").push_str(words);
            }
            Some(words) => {
                let paragraph = paragraphs.last_mut().expect("a paragraph");
                paragraph.push(' ');
                paragraph.push_str(words);
            }
            None => paragraphs.push(String::new()),
        }
    }
    paragraphs
}

/// The comment above the struct says what the filter is and at what rate,
/// in words for every design, wrapped as a person would wrap it.
#[test]
fn the_comment_says_what_the_filter_is() {
    let rate = 1000.0;
    let say = |design: Design| {
        let code = design.realize(rate).expect("realises").code("Filter");
        told(&code.expect("exports"))
    };
    assert_eq!(
        say(Design::MovingAverage { taps: 8 })[0],
        "Moving average of 8 samples, for samples taken at 1000 Hz."
    );
    assert_eq!(
        say(Design::Exponential { alpha: 0.25 })[0],
        "Exponential smoothing, y += 0.25 * (x - y), for samples taken at 1000 Hz."
    );
    assert_eq!(
        say(Design::Biquad {
            kind: BiquadKind::Notch,
            cutoff: 50.0,
            q: 5.0
        })[0],
        "Biquad notch at 50 Hz, Q 5, for samples taken at 1000 Hz."
    );
    assert_eq!(
        say(Design::Fir {
            band: Band::BandPass {
                low: 40.0,
                high: 120.0
            },
            taps: 41,
            window: Window::Blackman
        })[0],
        "Windowed-sinc band-pass from 40 to 120 Hz, 41 taps through a Blackman \
         window, for samples taken at 1000 Hz."
    );
    let even = say(Design::Median { taps: 4 });
    assert!(
        even[1].starts_with("Delays everything by 1.5 samples, 1.5 ms at that rate,"),
        "{even:?}"
    );
    let even = Design::Median { taps: 4 }
        .realize(rate)
        .expect("realises")
        .code("Even")
        .expect("exports");
    assert!(even.contains("(sorted[1] + sorted[2]) / 2.0"));
}

#[test]
fn a_short_number_keeps_four_figures_and_no_more() {
    assert_eq!(short(15.0), "15");
    assert_eq!(short(7.5), "7.5");
    assert_eq!(short(0.340136), "0.3401");
    assert_eq!(short(1234.5678), "1235");
    assert_eq!(short(0.0), "0");
}
