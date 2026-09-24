//! Does a filter the lab designed do in firmware what the lab says it does,
//! fed by a generator on the sheet?
//!
//! ```text
//! cargo run -p rusty-embed --example filter_probe -- examples/filter-lab
//! cargo run -p rusty-embed --example filter_probe -- --replay serial.log
//! ```
//!
//! The whole chain the Signals tab stands on, run once with nothing mocked:
//! the sheet's generator rendered into a table by `generator` and put on the
//! pin channel by the headless run, played by rusty's emulator against the
//! firmware's clock, converted by the firmware's own `read_oneshot()`,
//! filtered by the code the Design view exported, printed as telemetry — and
//! then changed while the firmware runs, by a scenario's `play` step.
//!
//! **Two claims, each held to the design's own numbers.** The chain: `raw`
//! carries each tone at the counts the sheet's full scale makes of its
//! volts, *at the instant the firmware's own clock says it converted* — so a
//! table played at the host's pace, or at the wrong rate, loses its tones,
//! and one never replaced has no 20 Hz tone after the `play` step. The
//! filter: `y` is the design's filter run over the firmware's own `raw`,
//! sample for sample, to within the half count `y` is rounded to — which is
//! the design's gain and phase at every frequency at once, where three tones
//! measured through an uneven clock were only near them.
//!
//! **Why the stamps and not the sample count.** Rusty's QEMU keeps its
//! virtual clock with the host's, so a host that holds the emulator up —
//! a shared runner, a busy desk — lets the clock run on while the firmware
//! stands still, and the firmware's loop then catches up in a burst of
//! samples taken a conversion apart. Every value is still right for the
//! instant it was taken; read as evenly spaced, the burst smears a 50 Hz
//! hum to a third of its size. The first version measured that way and
//! failed on a run whose chain was perfect. So the chain is fitted by least
//! squares at the stamps, and the filter compared in sample order — its own
//! order, whatever the clock did — and the probe says how uneven the clock
//! was, since a filter written for even samples is fed uneven ones by such
//! a host.
//!
//! It exits non-zero when either claim does not hold, which is what makes
//! it a gate. It needs rusty's QEMU with the tables (`[rusty:wave@`).
//! `--replay` judges a log of the firmware's serial lines instead of a run,
//! which is how a failure on a runner is read at a desk.

use std::path::PathBuf;
use std::process::ExitCode;

use rusty_embed::dsp::{Design, Pass};
use rusty_embed::simulate::headless::{self, Event, PlaySignal, Scenario, Step, Verdict};

/// The design `examples/filter-lab/src/low_pass.rs` was exported from.
const DESIGN: Design = Design::Butterworth {
    pass: Pass::Low,
    order: 2,
    cutoff: 10.0,
};
const RATE: f64 = 250.0;

/// What the sheet plays, in counts: half a volt and a quarter of one, of a
/// 2.5 V full scale at twelve bits.
const TONE_COUNTS: f64 = 0.5 / 2.5 * 4095.0;
const HUM_COUNTS: f64 = 0.25 / 2.5 * 4095.0;

/// What the `play` step switches to.
const SWITCHED: &str = "dc 1.25; sine f=20 a=0.5";

/// How far `y` may sit from the design's filter of the same `raw`: the half
/// count it is rounded to, and a little for `f32` against `f64`.
const ROUNDING: f64 = 0.55;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let serial = match args.as_slice() {
        [flag, log] if flag == "--replay" => match std::fs::read_to_string(log) {
            Ok(text) => text.lines().map(str::to_string).collect(),
            Err(error) => {
                eprintln!("filter_probe: could not read {log}: {error}");
                return ExitCode::from(2);
            }
        },
        [root] => match run(PathBuf::from(root)) {
            Some(serial) => serial,
            None => return ExitCode::FAILURE,
        },
        [] => match run(PathBuf::from("examples/filter-lab")) {
            Some(serial) => serial,
            None => return ExitCode::FAILURE,
        },
        _ => {
            eprintln!("usage: filter_probe [<project> | --replay <serial.log>]");
            return ExitCode::from(2);
        }
    };
    if judge(&serial) {
        println!("the generator, the emulator, the converter and the exported filter agree");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Boot the project, play the sheet, switch the signal once, and hand back
/// every line the firmware printed — or nothing, having said why.
fn run(root: PathBuf) -> Option<Vec<String>> {
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
        return None;
    }
    Some(outcome.serial)
}

/// One sample as the firmware printed it: when its clock said it converted
/// (seconds), the counts, and what its filter made of them.
#[derive(Clone, Copy)]
struct Sample {
    at: f64,
    raw: f64,
    y: f64,
}

/// Both claims, over every line the firmware printed.
fn judge(serial: &[String]) -> bool {
    let samples: Vec<Sample> = serial
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
            Some(Sample {
                at: telemetry.at_us? as f64 / 1e6,
                raw: value("raw")?,
                y: value("y")?,
            })
        })
        .collect();
    let (Some(first), Some(last)) = (samples.first().copied(), samples.last().copied()) else {
        eprintln!("filter_probe: the firmware printed no telemetry");
        return false;
    };
    let seconds = last.at - first.at;
    let rate = (samples.len() - 1) as f64 / seconds;
    println!(
        "{} samples over {seconds:.2} s of the firmware's clock, {rate:.1} a second",
        samples.len()
    );
    if (rate - RATE).abs() > RATE * 0.01 {
        eprintln!("filter_probe: the firmware sampled at {rate:.1} Hz, not {RATE}");
        return false;
    }
    say_how_even(&samples);

    let mut passed = true;

    // The chain. Before the switch: from half a second in, for three
    // seconds — well short of the step's five-second delay. After it: the
    // last two seconds, the step four seconds behind them.
    let window = |from: f64, to: f64| -> Vec<Sample> {
        samples
            .iter()
            .filter(|s| s.at >= from && s.at < to)
            .copied()
            .collect()
    };
    let before = window(first.at + 0.5, first.at + 3.5);
    let after = window(last.at - 2.0, last.at + 1e-6);
    for (window, tones) in [
        (
            &before,
            vec![
                ("the tone", 5.0, TONE_COUNTS),
                ("the hum", 50.0, HUM_COUNTS),
            ],
        ),
        (&after, vec![("the switched tone", 20.0, TONE_COUNTS)]),
    ] {
        let freqs: Vec<f64> = tones.iter().map(|(_, freq, _)| *freq).collect();
        let Some(fitted) = fit(window, &freqs) else {
            eprintln!("filter_probe: too few samples to fit {freqs:?} Hz");
            passed = false;
            continue;
        };
        for ((name, freq, counts), amplitude) in tones.iter().zip(fitted) {
            println!(
                "{name} at {freq} Hz: {amplitude:.1} counts in at the firmware's instants \
                 (the sheet says {counts:.1})"
            );
            if (amplitude - counts).abs() > counts * 0.02 {
                eprintln!("filter_probe: {name} reached the converter at the wrong size");
                passed = false;
            }
        }
    }

    // The filter, in the order the firmware ran it.
    let filter = DESIGN.realize(RATE).expect("the example's design realises");
    let raw: Vec<f64> = samples.iter().map(|s| s.raw).collect();
    let designed = filter.apply(&raw);
    let worst = samples
        .iter()
        .zip(&designed)
        .enumerate()
        .map(|(n, (sample, want))| (n, (sample.y - want).abs()))
        .fold(
            (0, 0.0),
            |worst, next| if next.1 > worst.1 { next } else { worst },
        );
    println!(
        "y is the design's filter of raw to within {:.3} counts over all {} samples",
        worst.1,
        samples.len()
    );
    if worst.1 > ROUNDING {
        let (n, _) = worst;
        eprintln!(
            "filter_probe: the firmware's filter does not do what its design says — at sample \
             {n} it printed {} where the design makes {:.3} of the same input",
            samples[n].y, designed[n]
        );
        passed = false;
    }
    let gains: Vec<String> = [5.0, 50.0, 20.0]
        .iter()
        .filter_map(|&freq| {
            filter
                .response(freq)
                .map(|r| format!("{:.4} at {freq} Hz", r.gain))
        })
        .collect();
    println!("so it passes what the design passes: {}", gains.join(", "));
    passed
}

/// Say how evenly the firmware's samples fell on its clock — what a filter
/// written for even samples was actually fed.
fn say_how_even(samples: &[Sample]) {
    let period = 1.0 / RATE;
    let gaps: Vec<f64> = samples.windows(2).map(|w| w[1].at - w[0].at).collect();
    let held = gaps.iter().filter(|gap| **gap > 1.5 * period).count();
    let bunched = gaps.iter().filter(|gap| **gap < 0.5 * period).count();
    let longest = gaps.iter().copied().fold(0.0, f64::max);
    if held == 0 {
        println!("every sample {:.1} ms after the one before", period * 1e3);
    } else {
        println!(
            "the emulator was held up {held} times (the longest gap {:.1} ms) and the firmware \
             caught up with {bunched} samples taken less than half a period apart",
            longest * 1e3
        );
    }
}

/// The amplitude of each of `freqs` in the window's `raw`, fitted by least
/// squares at the samples' own instants with a constant beside them — exact
/// however unevenly the samples fell, where a transform over the sample
/// count assumes they fell evenly.
fn fit(window: &[Sample], freqs: &[f64]) -> Option<Vec<f64>> {
    let columns = 1 + 2 * freqs.len();
    if window.len() < 4 * columns {
        return None;
    }
    let start = window[0].at;
    let row = |at: f64| -> Vec<f64> {
        let t = at - start;
        std::iter::once(1.0)
            .chain(freqs.iter().flat_map(|freq| {
                let turn = std::f64::consts::TAU * freq * t;
                [turn.cos(), turn.sin()]
            }))
            .collect()
    };
    // The normal equations, `AᵀA p = Aᵀx`.
    let mut normal = vec![vec![0.0; columns + 1]; columns];
    for sample in window {
        let a = row(sample.at);
        for i in 0..columns {
            for j in 0..columns {
                normal[i][j] += a[i] * a[j];
            }
            normal[i][columns] += a[i] * sample.raw;
        }
    }
    let p = solve(normal)?;
    Some(
        (0..freqs.len())
            .map(|k| p[1 + 2 * k].hypot(p[2 + 2 * k]))
            .collect(),
    )
}

/// Gaussian elimination with partial pivoting over an augmented matrix.
fn solve(mut m: Vec<Vec<f64>>) -> Option<Vec<f64>> {
    let n = m.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        if m[pivot][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, pivot);
        let pivot = m[col].clone();
        for (r, row) in m.iter_mut().enumerate() {
            if r != col {
                let factor = row[col] / pivot[col];
                for (cell, above) in row.iter_mut().zip(&pivot).skip(col) {
                    *cell -= factor * above;
                }
            }
        }
    }
    Some((0..n).map(|i| m[i][n] / m[i][i]).collect())
}
