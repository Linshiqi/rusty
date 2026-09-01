//! Running firmware without hardware, and the board beside it.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_embed::{LogLevel, LogLine, LogStream};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

/// Ask how this project would be simulated.
pub fn load_sim_plan(state: AppState) {
    if !state.has_project() {
        return;
    }
    track(
        state,
        ipc::get::<rusty_embed::SimPlan>(cmd::sim::PLAN),
        move |plan| state.sim.plan.set(Some(plan)),
    );
}

/// Build, image and boot in QEMU, streaming into the dock. One at a time —
/// the shared session slot enforces it the same way flashing does.
pub fn run_simulation(state: AppState, debug: bool) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    #[derive(serde::Serialize)]
    struct Args {
        debug: bool,
    }

    if state.app.session_running.get_untracked() {
        return;
    }
    state.dock.source.set("simulate");
    clear_capture(state);

    // Like stream_to_terminal, with one interception: the firmware's pin
    // reports drive the board view instead of scrolling the dock at 2Hz.
    let channel = ipc::Channel::new();
    let on_line = Closure::wrap(Box::new(move |value: JsValue| {
        match serde_wasm_bindgen::from_value::<LogLine>(value) {
            Ok(line) => {
                // The debug sentinel: QEMU is frozen and listening. Open the
                // terminal and type the attach line for the user — the gdb
                // REPL is theirs from there.
                if line.text.starts_with("[rusty:debug]") {
                    state.push_log(line);
                    // QEMU is frozen with its gdbstub listening. Attach the
                    // in-app debugger: breakpoints in the gutter, the stack
                    // in the dock. gdb's own REPL is still a terminal away
                    // for anything the panel does not model. The run that
                    // armed it knows which ELF it booted and on which port —
                    // reading that from the panel's cached plan attached gdb
                    // to the release binary while the unoptimised one ran.
                    debug_start(state, false);
                    return;
                }
                absorb(state, line);
            }
            Err(e) => state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: format!("[rusty could not decode a line from the tool: {e}]"),
                level: Some(LogLevel::Warn),
            }),
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_line);
    on_line.forget();
    state.app.session_running.set(true);
    state.show_dock(crate::state::DockTab::Output);
    spawn_local(async move {
        match ipc::call_streaming::<_, Option<i32>>(
            cmd::sim::RUN,
            &Args { debug },
            "onLine",
            &channel,
        )
        .await
        {
            Ok(code) => note_exit(state, code),
            Err(error) => {
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: error.message,
                    level: Some(LogLevel::Error),
                });
                note_exit(state, Some(-1));
            }
        }
    });
}

/// One-click install of a missing simulator tool, streamed to the dock.
/// Success refreshes the plan; failure reveals the manual instructions.
pub fn install_sim_tool(state: AppState, name: String) {
    state.dock.source.set("tools");
    #[derive(serde::Serialize)]
    struct Args {
        name: String,
    }

    if state.app.session_running.get_untracked() {
        return;
    }
    let args = Args { name: name.clone() };
    let channel = stream_to_terminal(state);
    spawn_local(async move {
        let outcome =
            ipc::call_streaming::<_, Option<i32>>(cmd::sim::INSTALL, &args, "onLine", &channel)
                .await;
        match outcome {
            Ok(Some(0)) => {
                note_exit(state, Some(0));
                state
                    .sim
                    .install_failed
                    .update(|failed| failed.retain(|t| t != &name));
                // The world just changed: re-probe the machine, and give the
                // editor its language server the moment it exists.
                refresh_toolchain(state);
                if name == "rust-analyzer" {
                    start_lsp(state);
                }
            }
            Ok(code) => {
                note_exit(state, code);
                state.sim.install_failed.update(|failed| {
                    if !failed.contains(&name) {
                        failed.push(name.clone());
                    }
                });
            }
            Err(error) => {
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: error.message,
                    level: Some(LogLevel::Error),
                });
                note_exit(state, Some(-1));
                state.sim.install_failed.update(|failed| {
                    if !failed.contains(&name) {
                        failed.push(name.clone());
                    }
                });
            }
        }
        // Either way the plan is re-asked: a success clears the card, and
        // even a failure may have changed the world (a partial unpack).
        load_sim_plan(state);
    });
}

/// One line into the running firmware's UART.
///
/// Fire-and-forget: a press against a stopped board lands nowhere, which is
/// what pressing a powered-off board does. Every input the panels have —
/// buttons, pots, tunables — is one of these, so the running check lives here
/// rather than being re-remembered at each call site.
pub(super) fn sim_send(state: AppState, text: String) {
    #[derive(serde::Serialize)]
    struct Args {
        text: String,
    }
    if !state.app.session_running.get_untracked() {
        return;
    }
    let args = Args { text };
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::sim::SEND, &args).await;
    });
}

/// A button transition on the board view.
pub fn sim_press(state: AppState, pin: u8, down: bool) {
    sim_send(state, format!("B{pin}={}", if down { 1 } else { 0 }));
}

/// A potentiometer moved: `P<pin>=<0..255>` into the firmware's UART.
pub fn sim_pot(state: AppState, pin: u8, value: u8) {
    sim_send(state, format!("P{pin}={value}"));
}

/// Inject one sensor sample — the whole sample, in one line.
///
/// Kept together deliberately: see [`rusty_embed::sensor_line`] for why a
/// torn sample is worse than a late one.
pub fn sim_sensor(state: AppState, name: String, values: Vec<f32>) {
    sim_send(state, rusty_embed::sensor_line(&name, &values));
    state.sim.sensor_values.update(|held| {
        held.insert(name, values);
    });
}

/// How often the plant steps and injects. 50 Hz.
///
/// Far slower than the loop it feeds, and deliberately: the firmware runs at
/// 200 Hz and will read the same sample four times, which is fine because the
/// aircraft's own dynamics are slower than either. Injecting at the loop rate
/// would put 200 lines a second on the same serial line the console shares,
/// for no more truth.
const PLANT_PERIOD: std::time::Duration = std::time::Duration::from_millis(20);
const PLANT_DT: f32 = 0.02;

/// Close the physical loop, or open it again.
///
/// Off by default and never self-starting. Firmware that reads a real IMU
/// *and* accepts injection would otherwise see two sources disagreeing, and
/// the resulting drift would be blamed on the sensor.
pub fn set_plant_closed(state: AppState, closed: bool) {
    let generation = state.sim.plant_gen.get_untracked() + 1;
    state.sim.plant_gen.set(generation);
    state.sim.plant_closed.set(closed);
    // A plant left spinning would hand the next run a rate nobody commanded.
    state.sim.plant.update(rusty_embed::Plant::reset);
    if closed {
        plant_step(state, generation);
    }
}

/// One step of the simulated aircraft, then schedule the next.
///
/// Stops on its own when the flag clears or a newer generation exists, which
/// is the editor pulse's rule and for the same reason: a timer that outlives
/// what started it keeps injecting into a session that has moved on.
fn plant_step(state: AppState, generation: u64) {
    if !state.sim.plant_closed.get_untracked() || state.sim.plant_gen.get_untracked() != generation
    {
        return;
    }

    // Four motors, in pin order — the quad-X order `Plant::step` mixes for.
    let mut motors: Vec<(u8, f32)> = state
        .sim
        .pwm
        .with_untracked(|pwm| pwm.iter().map(|(p, d)| (*p, *d)).collect());
    motors.sort_by_key(|(pin, _)| *pin);

    // Fewer than four driven pins is not an aircraft. Waiting rather than
    // padding with zeros: a firmware that has not reported all four yet
    // would otherwise be flown as though half its motors were dead.
    if motors.len() >= 4 {
        let duties = [motors[0].1, motors[1].1, motors[2].1, motors[3].1];
        let stepped = state.sim.plant.try_update(|plant| {
            plant.step(duties, PLANT_DT);
            (plant.rate(), plant.accelerometer())
        });
        if let Some((rate, accel)) = stepped {
            for (name, feed) in plant_feeds(state) {
                let sample = match feed {
                    Feed::Rates => rate,
                    Feed::Accelerometer => accel,
                };
                sim_sensor(state, name, sample.to_vec());
            }
        }
    }

    set_timeout(move || plant_step(state, generation), PLANT_PERIOD);
}

/// What the plant has to offer a sensor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    /// Body rates, what a gyro reads.
    Rates,
    /// Gravity in body coordinates, what an accelerometer reads at rest.
    Accelerometer,
}

impl Feed {
    pub fn label(self) -> &'static str {
        match self {
            Feed::Rates => "rates",
            Feed::Accelerometer => "gravity",
        }
    }
}

/// Which declared sensors the plant can fill, and with what.
///
/// **Matched by name, and the panel says so.** A sensor called `gyro` gets
/// rates and one called `accel` gets gravity, because those are what firmware
/// calls them — but it is a convention shown on screen rather than a rule
/// hidden in here, and anything the plant does not recognise is left alone.
/// Inventing a meaning for a sensor called `range` is how a panel starts
/// feeding a loop numbers nobody asked for.
///
/// Only three-component sensors: the plant has nothing sensible to put in a
/// single number, and padding one out would be the same invention.
pub fn plant_feeds(state: AppState) -> Vec<(String, Feed)> {
    state.sim.sensors.with_untracked(|sensors| {
        sensors
            .iter()
            .filter(|def| def.components == 3)
            .filter_map(|def| {
                let lower = def.name.to_lowercase();
                let feed = if lower.contains("gyro") || lower.contains("rate") {
                    Feed::Rates
                } else if lower.contains("acc") {
                    Feed::Accelerometer
                } else {
                    return None;
                };
                Some((def.name.clone(), feed))
            })
            .collect()
    })
}

/// Put a raw ADC count on a pin — a battery, a divider, any analog source.
pub fn sim_analog(state: AppState, pin: u8, count: u16) {
    sim_send(state, rusty_embed::analog_line(pin, count));
    state.sim.analog.update(|held| {
        held.insert(pin, count);
    });
}

/// Persist the board editor's layout, then re-plan so the panel shows what
/// the file now says.
pub fn save_sim_board(state: AppState, board: rusty_embed::SimBoard, dirty: RwSignal<bool>) {
    #[derive(serde::Serialize)]
    struct Args {
        board: rusty_embed::SimBoard,
    }
    let args = Args { board };
    spawn_local(async move {
        match ipc::call::<_, ()>(cmd::sim::SAVE_BOARD, &args).await {
            Ok(()) => {
                dirty.set(false);
                load_sim_plan(state);
            }
            Err(error) => {
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: format!("could not save the board: {}", error.message),
                    level: Some(LogLevel::Error),
                });
            }
        }
    });
}

/// Open the dock terminal and type the gdb attach line into it.
///
/// The shell does the launching, so the user sees exactly what ran and owns
/// the REPL afterwards — break, step, print are theirs, not wrapped.
/// Open gdb's own REPL in the terminal — the escape hatch for anything the
/// debug panel does not model (raw registers, `x/16x`, a scripted `commands`
/// block). The in-app session owns the ordinary path.
pub fn attach_debugger_terminal(state: AppState, command: String) {
    state.show_dock(crate::state::DockTab::Terminal);
    // `terminal` holds the shell's latest frame; None means no shell yet.
    if state.term.screen.with_untracked(Option::is_none) {
        open_terminal(state, 100, 24);
    }
    #[derive(serde::Serialize)]
    struct Args {
        bytes: Vec<u8>,
    }
    // A freshly opened terminal needs a beat before the shell reads keys.
    set_timeout(
        move || {
            let args = Args {
                bytes: format!("{command}\r").into_bytes(),
            };
            spawn_local(async move {
                let _ = ipc::call::<_, ()>(cmd::terminal::WRITE, &args).await;
            });
        },
        std::time::Duration::from_millis(700),
    );
}

/// The panel-facing spelling of "stop whatever session is running".
pub fn stop_session_now(state: AppState) {
    stop_session(state);
}
