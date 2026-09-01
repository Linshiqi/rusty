//! A running session: the serial line, the plot, and the trace.
//!
//! `absorb` is the single reading of the board protocol — one function, three
//! callers. It was per-stream once, and the consequence was telemetry that
//! plotted in the simulator and vanished on hardware.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_embed::{CommandPlan, LogLevel, LogLine, LogStream};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{AppState, TraceClock},
};

/// What proxy is stored, and what detection currently sees.
pub fn load_proxy_setting(stored: RwSignal<Option<String>>, detected: RwSignal<Option<String>>) {
    spawn_local(async move {
        if let Ok(value) = ipc::get::<serde_json::Value>(cmd::workbench::PROXY).await {
            stored.set(value["stored"].as_str().map(str::to_string));
            detected.set(value["detected"].as_str().map(str::to_string));
        }
    });
}

/// Store the proxy choice and re-read, so the preview line tells the truth.
pub fn save_proxy_setting(
    value: Option<String>,
    stored: RwSignal<Option<String>>,
    detected: RwSignal<Option<String>>,
    saved: RwSignal<bool>,
) {
    #[derive(serde::Serialize)]
    struct Args {
        value: Option<String>,
    }
    let args = Args { value };
    spawn_local(async move {
        if ipc::call::<_, ()>(cmd::workbench::SET_PROXY, &args)
            .await
            .is_ok()
        {
            saved.set(true);
            load_proxy_setting(stored, detected);
        }
    });
}

/// Board files that would not parse. Asked for by the Catalogue settings —
/// a user whose board never appears deserves the reason in the window, not
/// only in the CLI.
pub fn load_catalog_problems(state: AppState) {
    track(
        state,
        ipc::get::<Vec<rusty_embed::CatalogProblem>>(cmd::catalog::PROBLEMS),
        move |problems| state.project.catalog_problems.set(problems),
    );
}

/// Whether a key for this profile is on file. The key itself is never read
/// back — the credential store is write-only from here, by design.
pub fn refresh_key_state(state: AppState, profile: String) {
    #[derive(serde::Serialize)]
    struct Args {
        profile: String,
    }
    let args = Args { profile };
    spawn_local(async move {
        if let Ok(stored) = ipc::call::<_, bool>(cmd::ai::KEY_CONFIGURED, &args).await {
            state.ai.key_stored.set(stored);
        }
    });
}

/// Forget the stored key for a profile.
pub fn delete_key(state: AppState, profile: String) {
    #[derive(serde::Serialize)]
    struct Args {
        profile: String,
    }
    let args = Args {
        profile: profile.clone(),
    };
    spawn_local(async move {
        match ipc::call::<_, ()>(cmd::ai::DELETE_KEY, &args).await {
            Ok(()) => {
                state.ai.key_stored.set(false);
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: format!("the key for {profile} was removed"),
                    level: None,
                });
            }
            Err(error) => state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: error.message,
                level: Some(LogLevel::Error),
            }),
        }
    });
}

/// Forget what the last board said.
///
/// Every capture the protocol feeds, dropped together at the start of a run.
/// Partially clearing it is worse than not clearing at all: a plot whose
/// curves come from two different boots, on two different clocks, is read as
/// one signal and is not one.
pub(super) fn clear_capture(state: AppState) {
    state.sim.gpio.set(std::collections::HashMap::new());
    state.sim.pwm.set(std::collections::HashMap::new());
    state.sim.analog.set(std::collections::HashMap::new());
    // The declarations go too: they belong to the run that made them, and
    // offering a sensor the next firmware never asked for is the invented
    // range in another costume.
    state.sim.sensors.set(Vec::new());
    state
        .sim
        .sensor_values
        .set(std::collections::HashMap::new());
    state.sim.trace.set(crate::state::SimTrace::default());
    state.sim.display.set(String::new());
    state.sim.plot.set(crate::state::Plot::default());
    state.sim.params.set(Vec::new());
    // Back to the weaker claim until this run announces otherwise. A run
    // against rusty's QEMU followed by one against a stock build — the user
    // swapped the binary, or is now watching a real board over the serial
    // link — would otherwise keep a caption promising register-level truth
    // about levels the firmware is once again merely narrating.
    state.sim.pin_source.set(rusty_embed::PinSource::Firmware);
}

/// One line from a board, wherever it came from.
///
/// The same firmware prints the same protocol whether it is running in QEMU,
/// under `espflash monitor`, or on a port rusty holds open itself — so the
/// reading of it lives in one place rather than being re-implemented per
/// stream. It was per-stream once, and the consequence was that telemetry
/// plotted in the simulator and vanished on real hardware.
///
/// Anything that is not protocol is log, unchanged.
pub(super) fn absorb(state: AppState, line: LogLine) {
    if let Some(sample) = rusty_embed::protocol::parse_telemetry(&line.text) {
        record_plot(state, sample);
    } else if let Some(param) = rusty_embed::protocol::parse_param(&line.text) {
        // Newest wins, by name: the firmware re-announces after a change, and
        // what it says it took is the truth — a clamp is information, not a
        // mistake to hide.
        state.sim.params.update(|params| {
            match params.iter_mut().find(|known| known.name == param.name) {
                Some(known) => *known = param,
                None => params.push(param),
            }
        });
    } else if let Some(def) = rusty_embed::parse_sensor_def(&line.text) {
        // Newest wins by name, exactly as the tunables do: firmware that
        // re-announces on a timer is how a panel connecting late finds out
        // what it may inject at all.
        state.sim.sensors.update(
            |known| match known.iter_mut().find(|s| s.name == def.name) {
                Some(existing) => *existing = def,
                None => known.push(def),
            },
        );
    } else if let Some(source) = rusty_embed::parse_pin_source(&line.text) {
        // Before the gpio arm on purpose: this line decides what the board's
        // caption may claim about every line after it.
        state.sim.pin_source.set(source);
        state.push_log(line);
    } else if let Some(report) = rusty_embed::parse_gpio_report(&line.text) {
        state.sim.gpio.update(|gpio| {
            for (pin, level) in &report.pins {
                gpio.insert(*pin, *level);
            }
        });
        record_trace(state, report);
    } else if let Some(report) = rusty_embed::parse_pwm_report(&line.text) {
        // The analogue half of the arm above. Not folded into it: a level and
        // a duty are different facts about a pin, and a motor asked to hold
        // 40% is not the same as a pin that happens to be high right now.
        state.sim.pwm.update(|pwm| {
            for (pin, duty) in &report.pins {
                pwm.insert(*pin, *duty);
            }
        });
    } else if let Some(text) = rusty_embed::parse_display_report(&line.text) {
        state.sim.display.set(text);
    } else {
        state.push_log(line);
    }
}

/// The host's own clock in microseconds, for streams the firmware left
/// unstamped. Shared by the pin trace and the plot so the two axes mean the
/// same thing when both fall back to it.
fn host_now_us() -> u64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| (p.now() * 1000.0) as u64)
        .unwrap_or(0)
}

/// One gpio report into the waveform capture.
///
/// The first report decides the clock: stamped reports run on the firmware's
/// systimer, unstamped ones on the host's arrival time — never both, because
/// a trace that mixes time bases is a trace that lies. Capped so an hour of
/// simulation cannot eat the tab.
fn record_trace(state: AppState, report: rusty_embed::GpioReport) {
    const CAP: usize = 200_000;

    state.sim.trace.update(|trace| {
        let clock = *trace.clock.get_or_insert(if report.at_us.is_some() {
            TraceClock::Firmware
        } else {
            TraceClock::Host
        });
        let at = match (clock, report.at_us) {
            (TraceClock::Firmware, Some(us)) => us,
            // A late unstamped line in a firmware-clocked trace (or the
            // reverse) would corrupt the axis; reuse the last moment
            // instead, which keeps the event without inventing a time.
            (TraceClock::Firmware, None) => trace.events.last().map(|(t, ..)| *t).unwrap_or(0),
            (TraceClock::Host, _) => host_now_us(),
        };
        for (pin, level) in report.pins {
            trace.events.push((at, pin, level));
        }
        if trace.events.len() > CAP {
            let drop = trace.events.len() - CAP;
            trace.events.drain(..drop);
            trace.truncated = true;
        }
    });
}

/// Fold one telemetry line into the rolling plot.
///
/// The same clock discipline the pin trace learned: whichever base the first
/// sample arrived on decides the axis, and a later line in the other base
/// reuses the last moment rather than inventing one. Mixing them silently is
/// how a plot lies about *when*, which for a control loop is the only thing
/// it is being read for.
fn record_plot(state: AppState, sample: rusty_embed::protocol::Telemetry) {
    /// Samples kept per channel. A 1 kHz loop fills this in twenty seconds,
    /// which is the window a tuning change is judged in; older than that and
    /// you re-run rather than scroll back.
    const CAP: usize = 20_000;

    state.sim.plot.update(|plot| {
        let clock = *plot.clock.get_or_insert(if sample.at_us.is_some() {
            TraceClock::Firmware
        } else {
            TraceClock::Host
        });
        let at = match (clock, sample.at_us) {
            (TraceClock::Firmware, Some(us)) => us,
            (TraceClock::Firmware, None) => plot
                .channels
                .iter()
                .filter_map(|(_, points)| points.last().map(|(t, _)| *t))
                .max()
                .unwrap_or(0),
            (TraceClock::Host, _) => host_now_us(),
        };
        let mut dropped = false;
        for (name, value) in sample.channels {
            let points = plot.channel(&name);
            points.push((at, value));
            if points.len() > CAP {
                let over = points.len() - CAP;
                points.drain(..over);
                dropped = true;
            }
        }
        plot.truncated |= dropped;
    });
}

/// Set a tunable on the running firmware, over the serial line it is already
/// talking on.
///
/// No reflash: the whole point. The firmware answers with a `[rusty:param]`
/// line carrying what it actually took, and that is what the panel then
/// shows — so a value the firmware clamped reads as clamped rather than as
/// the number that was typed.
pub fn set_param(state: AppState, name: String, value: f32) {
    sim_send(state, rusty_embed::protocol::set_param_line(&name, value));
}

/// Hold a serial port open in both directions, until it is closed.
///
/// The mode a control loop is tuned in. `espflash monitor` reads its keyboard
/// through the console rather than through stdin, so a monitor rusty spawned
/// can only listen — this opens the port itself, which is what makes a
/// tunable writable, and gives up defmt decoding to do it.
pub fn open_link(state: AppState, port: String, baud: u32) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    #[derive(serde::Serialize)]
    struct Args {
        port: String,
        baud: u32,
    }

    if state.app.session_running.get_untracked() {
        return;
    }
    state.dock.source.set("link");
    clear_capture(state);

    let channel = ipc::Channel::new();
    let on_line =
        Closure::wrap(Box::new(move |value: JsValue| {
            match serde_wasm_bindgen::from_value::<LogLine>(value) {
                Ok(line) => absorb(state, line),
                Err(e) => state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: format!("[rusty could not decode a line from the port: {e}]"),
                    level: Some(LogLevel::Warn),
                }),
            }
        }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_line);
    on_line.forget();

    state.app.session_running.set(true);
    state.sim.link_port.set(Some(port.clone()));
    state.show_dock(crate::state::DockTab::Plot);

    let args = Args { port, baud };
    track_session(
        state,
        async move {
            let answer =
                ipc::call_streaming::<_, Option<i32>>(cmd::flash::LINK, &args, "onLine", &channel)
                    .await;
            // The claim has to be given back when the port refuses to open.
            // It is made up front because a streaming call never resolves
            // while it is working, so success is not an event — but leaving
            // it set after a refusal left the panel showing Disconnect and
            // live sliders over a port it did not have, which is the exact
            // failure `link_port` exists to prevent. Seen for real: the port
            // was still held by an earlier session, the error banner said so
            // correctly, and the panel claimed the link anyway.
            if answer.is_err() {
                state.sim.link_port.set(None);
            }
            answer
        },
        move |_| {
            state.sim.link_port.set(None);
            note_exit(state, None);
        },
    );
}

/// Close the link, which is the same stop every other session answers to.
pub fn close_link(state: AppState) {
    state.sim.link_port.set(None);
    stop_session(state);
}

/// Write the captured trace as VCD into the project's target directory,
/// where PulseView and GTKWave can open it.
pub fn export_vcd(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        text: String,
    }

    let trace = state.sim.trace.get_untracked();
    if trace.events.is_empty() {
        return;
    }
    let args = Args {
        text: rusty_embed::to_vcd(&trace.events),
    };
    spawn_local(async move {
        match ipc::call::<_, String>(cmd::sim::SAVE_TRACE, &args).await {
            Ok(path) => {
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: format!("waveform written to {path} — PulseView and GTKWave open it"),
                    level: None,
                });
                state.show_dock(crate::state::DockTab::Output);
            }
            Err(error) => state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: error.message,
                level: Some(LogLevel::Error),
            }),
        }
    });
}

/// `cargo build --release` for the open project, streamed to the dock —
/// the coding toolbar's Build, sharing the one session slot with
/// everything else that runs.
pub fn build_project(state: AppState) {
    if state.app.session_running.get_untracked() {
        return;
    }
    run_session(
        state,
        CommandPlan {
            program: "cargo".to_string(),
            args: vec!["build".to_string(), "--release".to_string()],
            display: "cargo build --release".to_string(),
            rationale: "the project's own toolchain builds the exact firmware a device \
                        would get"
                .to_string(),
            warning: None,
        },
        "build",
    );
}
