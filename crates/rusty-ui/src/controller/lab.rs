//! The frequency response, measured: a sine stepped across a band on the
//! source being studied, each step heard going in and coming out once the
//! filter has settled (`crate::lab::sweep`).
//!
//! Driven from here and not from the backend because what comes out is the
//! firmware's telemetry, which only this side reads — and timed by the
//! firmware's clock rather than the host's: a step listens until the
//! emulator's own stamps say it has, so a host that stalled for a second
//! measures what one that did not would.

use std::time::Duration;

use leptos::prelude::*;

use rusty_i18n::t;

use super::*;
use crate::lab::record::{self, Of, Record};
use crate::lab::sweep::{self, Missed, Plan};
use crate::state::{AppState, Source, TraceClock};

/// How often a waiting step looks at the clock.
const LOOK_EVERY: Duration = Duration::from_millis(100);

/// How long, on the host's clock, one step waits before giving up: a
/// firmware that stopped printing would otherwise hold the sweep for ever,
/// and the source with it.
const PATIENCE_MS: f64 = 30_000.0;

/// One sweep in flight.
#[derive(Clone)]
struct Run {
    number: u64,
    source: Source,
    plan: Plan,
    input: Of,
    output: String,
    freqs: Vec<f64>,
    index: usize,
    /// What the source played before the sweep, to put back after it:
    /// the lab's own change if it made one, the sheet's signal otherwise.
    restore: Option<String>,
}

/// Start stepping a sine across the plan's band on the studied source,
/// reading `sweep_input` going in and `sweep_output` coming out.
pub fn start_sweep(state: AppState) {
    if !state.app.session_running.get_untracked() {
        return;
    }
    let (Some(source), Some(output)) = (
        state.lab.source.get_untracked(),
        state.lab.sweep_output.get_untracked(),
    ) else {
        return;
    };
    let plan = state.lab.sweep.get_untracked();
    let freqs = sweep::frequencies(&plan);
    if freqs.is_empty() {
        return;
    }
    let number = state.lab.sweep_number.get_untracked() + 1;
    state.lab.sweep_number.set(number);
    state.lab.sweeping.set(Some(number));
    state.lab.swept.set(Vec::new());
    state.lab.sweep_missed.set(None);
    let restore = state
        .lab
        .playing
        .with_untracked(|playing| playing.get(&source).cloned())
        .or_else(|| sheet_text(state, &source));
    play(
        state,
        Run {
            number,
            input: state.lab.sweep_input.get_untracked().unwrap_or(Of::Played),
            source,
            plan,
            output,
            freqs,
            index: 0,
            restore,
        },
    );
}

/// Stop the sweep: the step waiting now wakes, finds itself stale and puts
/// the source back.
pub fn stop_sweep(state: AppState) {
    state.lab.sweeping.set(None);
}

fn current(state: AppState, number: u64) -> bool {
    state.lab.sweeping.get_untracked() == Some(number) && state.app.session_running.get_untracked()
}

/// Play the next step's tone, or finish when there is none.
fn play(state: AppState, run: Run) {
    if !current(state, run.number) {
        return finish(state, run, None);
    }
    let Some(&freq) = run.freqs.get(run.index) else {
        return finish(state, run, None);
    };
    state
        .lab
        .sweep_step
        .set(Some((run.index + 1, run.freqs.len(), freq)));
    let before = last_switch(state);
    sim_signal_set(
        state,
        run.source.clone(),
        Some(sweep::step_signal(&run.plan, freq)),
    );
    listen(state, run, freq, before, js_sys::Date::now());
}

/// Wait until the emulator has switched to the step's tone and its clock has
/// run past the settling and the listening, then measure the step.
fn listen(state: AppState, run: Run, freq: f64, before: Option<u64>, asked: f64) {
    set_timeout(
        move || {
            if !current(state, run.number) {
                return finish(state, run, None);
            }
            if js_sys::Date::now() - asked > PATIENCE_MS {
                let why = t!("lab.sweep-waited", channel = run.output.clone());
                return finish(state, run, Some(why));
            }
            // The switch this step asked for, on the emulator's clock.
            let switched = last_switch(state).filter(|at| before.is_none_or(|b| *at > b));
            let Some(switched) = switched else {
                return listen(state, run, freq, before, asked);
            };
            let settle = (sweep::settle_seconds(freq) * 1e6) as u64;
            let heard = sweep::listen_seconds(freq);
            let from = switched + settle;
            if newest(state).is_none_or(|now| now < from + (heard * 1e6) as u64) {
                return listen(state, run, freq, before, asked);
            }
            match measure(state, &run, freq, from, heard) {
                Ok(point) => {
                    state.lab.swept.update(|swept| swept.push(point));
                    let mut next = run;
                    next.index += 1;
                    play(state, next);
                }
                Err(why) => finish(state, run, Some(why)),
            }
        },
        LOOK_EVERY,
    );
}

/// One step's point: the tone going in and coming out over `heard` seconds
/// from `from` µs.
fn measure(
    state: AppState,
    run: &Run,
    freq: f64,
    from: u64,
    heard: f64,
) -> Result<sweep::Point, String> {
    let missing = |of: &Of| t!("lab.sweep-no-record", record = describe(of));
    let output_of = Of::Channel(run.output.clone());
    let output = channel(state, &run.output)
        .map(|r| r.between(from, heard))
        .ok_or_else(|| missing(&output_of))?;
    let input = match &run.input {
        Of::Channel(name) => channel(state, name).map(|r| r.between(from, heard)),
        Of::Converter(pin) => state.lab.conversions.with_untracked(|all| {
            all.get(pin)
                .and_then(|r| record::from_conversions(r))
                .map(|r| r.between(from, heard))
        }),
        Of::Played => played_now(state, &run.source).and_then(|played| {
            let start = state
                .lab
                .switched
                .with_untracked(|switched| crate::lab::start_of(&played, switched))?;
            Some(record::from_played(
                &played,
                start,
                from,
                output.rate,
                heard,
            ))
        }),
    }
    .ok_or_else(|| missing(&run.input))?;
    sweep::measure(&input, &output, freq, from).map_err(|missed| match missed {
        Missed::Tone(refusal) => crate::lab::dsp_refusal_text(&refusal),
        Missed::Silent => t!("lab.sweep-silent", record = describe(&run.input)),
    })
}

/// The sweep is over — finished, stopped, or gone wrong: the source plays
/// what it played before, unless a newer sweep has it now.
fn finish(state: AppState, run: Run, missed: Option<String>) {
    let superseded = state
        .lab
        .sweeping
        .get_untracked()
        .is_some_and(|number| number != run.number);
    if superseded {
        return;
    }
    if state.app.session_running.get_untracked() {
        sim_signal_set(state, run.source.clone(), run.restore.clone());
    }
    state.lab.sweeping.set(None);
    state.lab.sweep_step.set(None);
    if missed.is_some() {
        state.lab.sweep_missed.set(missed);
    }
}

/// What the sheet says `source` plays.
fn sheet_text(state: AppState, source: &Source) -> Option<String> {
    state.sim.plan.with_untracked(|plan| {
        let sheet = plan.as_ref()?.board.as_ref()?;
        let part = sheet.parts.iter().find(|p| p.reference == source.part)?;
        part.props.get(&source.key).cloned()
    })
}

/// What `source` plays now, rendered as the backend renders it.
fn played_now(state: AppState, source: &Source) -> Option<crate::lab::Played> {
    let texts = state.lab.playing.get_untracked();
    state.sim.plan.with_untracked(|plan| {
        let sheet = plan.as_ref()?.board.as_ref()?;
        crate::lab::played(sheet, source, &texts).ok()
    })
}

/// A channel the firmware prints on its own clock, as a record.
fn channel(state: AppState, name: &str) -> Option<Record> {
    state.sim.plot.with_untracked(|plot| {
        if plot.clock != Some(TraceClock::Firmware) {
            return None;
        }
        let (_, points) = plot.channels.iter().find(|(known, _)| known == name)?;
        record::from_channel(points)
    })
}

/// When the emulator last switched a table, on its clock.
fn last_switch(state: AppState) -> Option<u64> {
    state
        .lab
        .switched
        .with_untracked(|switched| switched.values().map(|s| s.at_us).max())
}

/// The newest instant anything on the firmware's clock has said.
fn newest(state: AppState) -> Option<u64> {
    let converted = state.lab.conversions.with_untracked(|all| {
        all.values()
            .filter_map(|record| record.last().map(|(at, _)| *at))
            .max()
    });
    let printed = state.sim.plot.with_untracked(|plot| {
        (plot.clock == Some(TraceClock::Firmware))
            .then(|| {
                plot.channels
                    .iter()
                    .filter_map(|(_, points)| points.last().map(|(at, _)| *at))
                    .max()
            })
            .flatten()
    });
    converted.max(printed)
}

/// How often a sensor the firmware declared on its console is fed: fifty
/// samples a second, what a console line can carry beside everything else
/// it carries — and no claim to the firmware's clock, which only a table in
/// the emulator has.
pub const CONSOLE_RATE: u32 = 50;

/// Feed the console's declared sensors their signals, at the host's pace,
/// for as long as any axis has one and the run goes on. Called again when a
/// signal changes: the feed running now finds itself stale and stops.
pub fn console_signals(state: AppState) {
    let generation = state.lab.console_gen.get_untracked() + 1;
    state.lab.console_gen.set(generation);
    // Each axis rendered once, a loop at the feed's rate, the noise its own.
    let rendered: Vec<(String, usize, Vec<f64>)> = state.lab.console.with_untracked(|all| {
        all.iter()
            .flat_map(|(name, axes)| {
                axes.iter().enumerate().filter_map(move |(axis, text)| {
                    if text.trim().is_empty() {
                        return None;
                    }
                    let signal = rusty_embed::signal::Signal::parse(text).ok()?;
                    let looped = rusty_embed::generator::loop_of([&signal], CONSOLE_RATE);
                    let seed = console_seed(name, axis);
                    let samples = signal.render(f64::from(CONSOLE_RATE), looped.samples, seed);
                    Some((name.clone(), axis, samples))
                })
            })
            .collect()
    });
    if rendered.is_empty() {
        return;
    }
    feed(
        state,
        generation,
        std::rc::Rc::new(rendered),
        js_sys::Date::now(),
    );
}

/// The seed a console axis draws its noise with: its sensor's name and its
/// axis, so three axes given one signal are not one axis three times.
fn console_seed(name: &str, axis: usize) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^ axis as u64
}

fn feed(
    state: AppState,
    generation: u64,
    rendered: std::rc::Rc<Vec<(String, usize, Vec<f64>)>>,
    started: f64,
) {
    set_timeout(
        move || {
            if state.lab.console_gen.get_untracked() != generation
                || !state.app.session_running.get_untracked()
            {
                return;
            }
            let sample =
                ((js_sys::Date::now() - started) / 1000.0 * f64::from(CONSOLE_RATE)) as usize;
            let declared = state.sim.sensors.get_untracked();
            let mut names: Vec<&String> = rendered.iter().map(|(name, _, _)| name).collect();
            names.dedup();
            for name in names {
                let Some(def) = declared.iter().find(|def| &def.name == name) else {
                    continue;
                };
                // The axes with no signal stay where their sliders are, and
                // a sample travels whole — every axis in one line.
                let mut values = state.sim.sensor_values.with_untracked(|held| {
                    held.get(name)
                        .cloned()
                        .unwrap_or_else(|| vec![0.0; usize::from(def.components)])
                });
                for (_, axis, samples) in rendered.iter().filter(|(n, _, _)| n == name) {
                    let Some(slot) = values.get_mut(*axis) else {
                        continue;
                    };
                    // Within the range the firmware declared, when it did:
                    // the tunables' rule, pointed the other way.
                    let mut value = samples[sample % samples.len()] as f32;
                    if let Some(low) = def.min {
                        value = value.max(low);
                    }
                    if let Some(high) = def.max {
                        value = value.min(high);
                    }
                    *slot = value;
                }
                sim_sensor(state, name.clone(), values);
            }
            feed(state, generation, rendered, started);
        },
        Duration::from_millis(u64::from(1000 / CONSOLE_RATE)),
    );
}

/// A record, the way a refusal names it.
fn describe(of: &Of) -> String {
    match of {
        Of::Played => t!("lab.record-played"),
        Of::Converter(pin) => t!("lab.lane-converter", pin = *pin),
        Of::Channel(name) => name.clone(),
    }
}
