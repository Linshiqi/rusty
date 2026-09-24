//! Does a filter the lab designed do in firmware what the lab says it does,
//! fed by a generator on the sheet?
//!
//! ```text
//! cargo run -p rusty-embed --example filter_probe -- examples/filter-lab
//! ```
//!
//! The whole chain the Signals tab stands on, run once with nothing mocked:
//! the sheet's generator rendered into a table by `generator` and put on the
//! pin channel by the headless run, played by rusty's emulator against the
//! firmware's clock, converted by the firmware's own `read_oneshot()`,
//! filtered by the code the Design view exported, printed as telemetry — and
//! then changed while the firmware runs, by a scenario's `play` step.
//!
//! **The assertion is the design's own numbers.** `raw` has to carry each
//! tone at the counts the sheet's full scale makes of its volts, and `y` over
//! `raw` has to be the design's gain and phase at each: the slow tone passed,
//! the hum taken out by the factor the design says, and the tone the `play`
//! step switched to attenuated by what the curve says there. A generator that
//! played at the host's pace would still pass the counts and fail the phase;
//! a table that was never replaced would fail the last tone outright.
//!
//! It exits non-zero when any of that does not hold, which is what makes it
//! a gate. It needs rusty's QEMU with the tables (`[rusty:wave@`).

use std::path::PathBuf;
use std::process::ExitCode;

use rusty_embed::dsp::{self, Design, Pass};
use rusty_embed::simulate::headless::{self, Event, PlaySignal, Scenario, Step, Verdict};

/// The design `examples/filter-lab/src/low_pass.rs` was exported from.
const DESIGN: Design = Design::Butterworth {
    pass: Pass::Low,
    order: 2,
    cutoff: 10.0,
};
const RATE: f64 = 1000.0;

/// What the sheet plays, in counts: half a volt and a quarter of one, of a
/// 2.5 V full scale at twelve bits.
const TONE_COUNTS: f64 = 0.5 / 2.5 * 4095.0;
const HUM_COUNTS: f64 = 0.25 / 2.5 * 4095.0;

/// What the `play` step switches to.
const SWITCHED: &str = "dc 1.25; sine f=20 a=0.5";

fn main() -> ExitCode {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "examples/filter-lab".to_string()),
    );
    let scenario = Scenario {
        timeout: Some(14.0),
        fail: vec!["[filter] panic".into(), "never finished".into()],
        steps: vec![
            Step {
                wait_serial: Some("[filter] ready".into()),
                ..Step::default()
            },
            Step {
                delay: Some(5.0),
                ..Step::default()
            },
            Step {
                play: Some(PlaySignal {
                    part: "V1".into(),
                    reading: None,
                    signal: SWITCHED.into(),
                }),
                ..Step::default()
            },
            Step {
                delay: Some(4.0),
                ..Step::default()
            },
        ],
        ..Scenario::default()
    };

    let outcome = headless::run(&root, &scenario, &mut |event| match event {
        Event::Command(line) => eprintln!("$ {line}"),
        Event::Output(line) if line.contains("error") => eprintln!("{line}"),
        Event::Note(line) => eprintln!("{line}"),
        _ => {}
    });
    if !matches!(outcome.verdict, Verdict::Passed) {
        eprintln!("filter_probe: the run did not pass: {:?}", outcome.verdict);
        for line in outcome.serial.iter().rev().take(10).rev() {
            eprintln!("  | {line}");
        }
        return ExitCode::FAILURE;
    }

    // Every sample the firmware printed, on its own clock.
    let samples: Vec<(u64, f64, f64)> = outcome
        .serial
        .iter()
        .filter_map(|line| {
            let telemetry = rusty_embed::protocol::parse_telemetry(line)?;
            let value = |name: &str| {
                telemetry
                    .channels
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| f64::from(*v))
            };
            Some((telemetry.at_us?, value("raw")?, value("y")?))
        })
        .collect();
    let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
        eprintln!("filter_probe: the firmware printed no telemetry");
        return ExitCode::FAILURE;
    };
    let seconds = (last.0 - first.0) as f64 / 1e6;
    let rate = (samples.len() - 1) as f64 / seconds;
    println!(
        "{} samples over {seconds:.2} s of the firmware's clock, {rate:.1} a second",
        samples.len()
    );
    if (rate - RATE).abs() > RATE * 0.01 {
        eprintln!("filter_probe: the firmware sampled at {rate:.1} Hz, not {RATE}");
        return ExitCode::FAILURE;
    }

    // Before the switch: from half a second in, for three seconds — the
    // filter settled, and well short of the step's five-second delay. After
    // it: the last two seconds, the step four seconds behind them.
    let window = |from: u64, to: u64| -> (Vec<f64>, Vec<f64>) {
        samples
            .iter()
            .filter(|(at, _, _)| *at >= from && *at < to)
            .map(|(_, raw, y)| (*raw, *y))
            .unzip()
    };
    let before = window(first.0 + 500_000, first.0 + 3_500_000);
    let after = window(last.0 - 2_000_000, last.0 + 1);

    let filter = DESIGN.realize(RATE).expect("the example's design realises");
    let mut failed = false;
    for (name, (raw, y), freq, counts) in [
        ("the tone", &before, 5.0, TONE_COUNTS),
        ("the hum", &before, 50.0, HUM_COUNTS),
        ("the switched tone", &after, 20.0, TONE_COUNTS),
    ] {
        let (Ok(going_in), Ok(coming_out)) = (dsp::tone(raw, RATE, freq), dsp::tone(y, RATE, freq))
        else {
            eprintln!("filter_probe: {name} at {freq} Hz could not be measured");
            failed = true;
            continue;
        };
        let want = filter.response(freq).expect("a linear design has one");
        let gain = coming_out.amplitude / going_in.amplitude;
        let phase = wrap(coming_out.phase - going_in.phase);
        let counts_off = (going_in.amplitude - counts).abs() / counts;
        let gain_off = (gain - want.gain).abs() / want.gain;
        let phase_off = wrap(phase - want.phase).abs();
        println!(
            "{name} at {freq} Hz: {:.1} counts in (the sheet says {counts:.1}), gain {gain:.4} \
             (designed {:.4}), phase {phase:.3} rad (designed {:.3})",
            going_in.amplitude, want.gain, want.phase
        );
        if counts_off > 0.02 {
            eprintln!("filter_probe: {name} reached the converter at the wrong size");
            failed = true;
        }
        if gain_off > 0.03 || phase_off > 0.03 {
            eprintln!("filter_probe: the firmware's filter does not do what its design says");
            failed = true;
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        println!("the generator, the emulator, the converter and the exported filter agree");
        ExitCode::SUCCESS
    }
}

/// A phase into `(−π, π]`.
fn wrap(phase: f64) -> f64 {
    let turned = phase.rem_euclid(std::f64::consts::TAU);
    if turned > std::f64::consts::PI {
        turned - std::f64::consts::TAU
    } else {
        turned
    }
}
