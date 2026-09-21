//! What is plugged in, and getting a binary onto it.
//!
//! Flash is one click, the way PlatformIO's Upload and Arduino's are: build,
//! write the image to the chosen device, and stay attached to what it says.
//! It used to open a dock tab — pick a device in a list, pick a mode, read a
//! command, press a second button — so the verb in the title bar was a door
//! to a form, and a board flashed from it carried whatever image had last
//! been built, however stale. Now the device is a picker beside the verb,
//! chosen once and remembered; with one board plugged in there is nothing
//! to choose; and the command still reaches the dock before it runs, where
//! it can be read, copied and pasted into a bug report.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_i18n::t;

use rusty_embed::{
    CommandPlan, FlashAction, Flasher, LogLevel, LogLine, LogStream, MemoryReport, Probe,
    SerialPort, Transport,
};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    activity::{Activity, Kind, Size},
    ipc::{self, cmd},
    state::{AfterStop, AppState, DeviceAction},
};

/// Re-enumerate serial ports and debug probes.
///
/// Explicit rather than polled. Enumerating serial ports opens each device on
/// some platforms, and doing that on a timer while a monitor is attached is a
/// good way to disturb the session the user is watching. So it happens when
/// the picker opens and when a device verb needs an answer — both moments
/// when nothing is attached.
pub fn scan_devices(state: AppState) {
    scan_then(state, || {});
}

/// Scan, then carry on with what was found. Ports first and on screen at
/// once: `probe-rs list` waits on USB enumeration and can take seconds.
fn scan_then(state: AppState, then: impl FnOnce() + 'static) {
    track(
        state,
        ipc::get::<Vec<SerialPort>>(cmd::flash::SERIAL_PORTS),
        move |ports| {
            // Keep a chosen port only while it is still attached. Silently
            // holding a disconnected one means the next flash fails with an
            // access error naming a device the user already unplugged.
            state.device.transport.update(|transport| {
                if let Some(Transport::Serial { port }) = transport.as_ref()
                    && !ports.iter().any(|p| &p.name == port)
                {
                    *transport = None;
                }
            });
            state.device.ports.set(ports);
            track(
                state,
                ipc::get::<Vec<Probe>>(cmd::flash::DEBUG_PROBES),
                move |probes| {
                    state.device.transport.update(|transport| {
                        if let Some(Transport::Probe {
                            identifier: Some(id),
                        }) = transport.as_ref()
                            && !probes.iter().any(|p| &p.identifier == id)
                        {
                            *transport = None;
                        }
                    });
                    state.device.probes.set(probes);
                    then();
                },
            );
        },
    );
}

/// The device a verb should use when none is chosen, when that is not a
/// guess: the one board plugged in. `None` means ask.
///
/// `serial` is whether the project's chip has a ROM serial bootloader. When
/// it does, only ports that look like boards count — an ESP32-C3 with native
/// USB appears as a port *and* a probe, and counting both would ask every
/// time about one board. When it does not, the probes are all there is.
pub(crate) fn only_candidate(
    ports: &[SerialPort],
    probes: &[Probe],
    serial: bool,
) -> Option<Transport> {
    if serial {
        let mut boards = ports.iter().filter(|port| port.likely_board);
        let first = boards.next()?;
        return boards.next().is_none().then(|| Transport::Serial {
            port: first.name.clone(),
        });
    }
    let mut found = probes.iter();
    let first = found.next()?;
    found.next().is_none().then(|| Transport::Probe {
        identifier: Some(first.identifier.clone()),
    })
}

/// Whether the project's chip is flashed over serial. An unknown chip is
/// taken to be: the serial path is the ordinary one, and the planner refuses
/// it by name for a part that has no bootloader.
fn serial_capable(state: AppState) -> bool {
    let chip = state
        .project
        .detected
        .with_untracked(|p| p.as_ref().and_then(|p| p.chip.clone()));
    let Some(chip) = chip else {
        return true;
    };
    state.project.chips.with_untracked(|chips| {
        chips
            .iter()
            .find(|c| c.id == chip)
            .is_none_or(|c| c.flashers.is_empty() || c.flashers.contains(&Flasher::Espflash))
    })
}

/// Flash, flash without monitoring, or monitor — whatever the title bar,
/// the Device menu or the palette asked for.
///
/// With a device chosen, that one. With none, the only board plugged in if
/// there is exactly one; otherwise the picker opens, and the verb waits in
/// it (`Device::pending`) for the device to be picked rather than asking for
/// the click again.
pub fn device_action(state: AppState, action: DeviceAction) {
    if state.app.session_running.get_untracked() {
        // A monitor or a serial link holds the port the flash needs. Let it
        // go first, the way PlatformIO's Upload and Arduino's do, rather
        // than refusing or failing on a busy port; anything else running —
        // a build, a simulation — is not the flash's to stop.
        if action != DeviceAction::Monitor && holds_port(state) {
            state.app.after_stop.set(Some(AfterStop::Device(action)));
            stop_session(state);
        }
        return;
    }
    scan_then(state, move || {
        let transport = state.device.transport.get_untracked().or_else(|| {
            let serial = serial_capable(state);
            let pick = state.device.ports.with_untracked(|ports| {
                state
                    .device
                    .probes
                    .with_untracked(|probes| only_candidate(ports, probes, serial))
            });
            if let Some(transport) = &pick {
                state.device.transport.set(Some(transport.clone()));
            }
            pick
        });
        match transport {
            Some(transport) => perform(state, action, transport),
            None => {
                state.device.pending.set(Some(action));
                state.device.picker.set(true);
            }
        }
    });
}

/// Run or Debug in the simulator — the title bar's buttons and Ctrl+F5 / F5
/// alike — with the board on screen while the build streams to the dock.
///
/// F5 during a debug session stopped at a breakpoint resumes it, as it does
/// in every editor whose F5 starts one. Run while a simulation runs is
/// Wokwi's restart: that run stops and a new one starts with the code on
/// screen — the loop the playground is for. And a run that cannot start
/// still shows the board, whose panel lists what is missing: a key that did
/// nothing would be a refusal nobody can see. With the board beside the
/// editor the panel stays where it is, since the board is on screen
/// already.
pub fn simulate(state: AppState, debug: bool) {
    if debug
        && state
            .debug
            .session
            .with_untracked(|s| s.as_ref().is_some_and(|s| s.attached && !s.running))
    {
        debug_control(state, "resume");
        return;
    }
    let board_in_view = state.layout.board_beside.get_untracked()
        && state.layout.panel.with_untracked(|p| p == "files");
    if !board_in_view {
        state.layout.panel.set("simulate".to_string());
    }
    if state.app.session_running.get_untracked() {
        if simulating(state) {
            state
                .app
                .after_stop
                .set(Some(AfterStop::Simulate { debug }));
            stop_anything(state);
        }
        return;
    }
    let ready = state.sim.plan.with_untracked(|plan| {
        plan.as_ref()
            .is_some_and(|p| p.supported && p.missing.is_empty() && (!debug || p.debug.is_some()))
    });
    if ready {
        // What runs is what is on screen: the code, and the board.
        save_all_then(state, move || {
            save_sheet_then(state, move || {
                if !state.app.session_running.get_untracked() {
                    run_simulation(state, debug);
                }
            })
        });
    }
}

/// Stop the simulation that is running and start it again with the code
/// and the board on screen — the title bar's Restart, Ctrl+Shift+F5 — the
/// way it was started, under the debugger or not. With nothing running it
/// is Run.
pub fn restart_simulation(state: AppState) {
    let debug = state
        .app
        .activity
        .with_untracked(|activity| activity.as_ref().is_some_and(|a| a.kind == Kind::Debug));
    if simulating(state) {
        state
            .app
            .after_stop
            .set(Some(AfterStop::Simulate { debug }));
        stop_anything(state);
    } else if !state.app.session_running.get_untracked() {
        simulate(state, false);
    }
}

/// Whether what runs now is a simulation — plain or under the debugger —
/// which is what Run restarts rather than refuses.
pub fn simulating(state: AppState) -> bool {
    state.app.activity.with_untracked(|activity| {
        activity
            .as_ref()
            .is_some_and(|a| matches!(a.kind, Kind::Simulate | Kind::Debug))
    })
}

/// Stop whatever runs — the title bar's Stop, Shift+F5. A debug session
/// ends through its debugger, which stops the emulator it booted as well.
pub fn stop_anything(state: AppState) {
    if state.debug.session.with_untracked(Option::is_some) {
        debug_stop(state);
    } else {
        stop_session_now(state);
    }
}

/// Whether what is running now is holding a serial port open: a monitor, a
/// flash that has gone on to monitor, or the Plot panel's link.
pub fn holds_port(state: AppState) -> bool {
    state.app.activity.with_untracked(|activity| {
        activity
            .as_ref()
            .is_some_and(|a| matches!(a.kind, Kind::Monitor | Kind::Link))
    })
}

/// A row in the picker, clicked: the device for every verb from now on, and
/// the verb that opened the picker, done.
pub fn choose_device(state: AppState, transport: Transport) {
    state.device.transport.set(Some(transport.clone()));
    state.device.picker.set(false);
    if let Some(action) = state.device.pending.get_untracked() {
        state.device.pending.set(None);
        if !state.app.session_running.get_untracked() {
            perform(state, action, transport);
        }
    }
}

/// The picker put away without a choice: the verb it was holding goes too.
pub fn close_picker(state: AppState) {
    state.device.picker.set(false);
    state.device.pending.set(None);
}

fn perform(state: AppState, action: DeviceAction, transport: Transport) {
    match action {
        DeviceAction::Monitor => write(state, transport, FlashAction::Monitor),
        DeviceAction::Flash | DeviceAction::FlashOnly => {
            let then = if action == DeviceAction::Flash {
                FlashAction::FlashAndMonitor
            } else {
                FlashAction::Flash
            };
            // Built first, every time: the image on the board is the code
            // on screen. A flash of whatever was last built — which is what
            // this did before — is a board that disagrees with its source
            // for a reason nothing names. An unchanged project builds in a
            // second.
            build_then(state, move |built| {
                if built {
                    write(state, transport, then);
                }
            });
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanArgs {
    transport: Transport,
    action: FlashAction,
    firmware: Option<String>,
    defmt: bool,
    baud: Option<u32>,
}

fn plan_args(state: AppState, transport: Transport, action: FlashAction) -> PlanArgs {
    // Whether to decode defmt is a property of the binary, not a preference:
    // asking espflash to decode a build without the string table produces
    // gibberish, and not asking on a build with one produces framing bytes.
    let defmt = state
        .project
        .detected
        .with_untracked(|p| p.as_ref().is_some_and(|p| p.uses_defmt));
    PlanArgs {
        transport,
        action,
        firmware: state.current_firmware_untracked().map(|f| f.path),
        defmt,
        baud: None,
    }
}

/// Plan against the build that is there now, and run it — asking first when
/// the plan carries a warning, which today means the device on this port
/// does not look like the project's chip.
fn write(state: AppState, transport: Transport, action: FlashAction) {
    let target = device_label(&transport);
    let args = plan_args(state, transport, action);
    track(
        state,
        async move { ipc::call::<_, CommandPlan>(cmd::flash::PLAN, &args).await },
        move |plan| {
            spawn_local(async move {
                if let Some(warning) = plan.warning.clone()
                    && !ipc::confirm(&warning).await
                {
                    return;
                }
                let channel = if action == FlashAction::Monitor {
                    "monitor"
                } else {
                    "flash"
                };
                run_session_then(state, plan, channel, |_| {});
                name_activity(state, target);
            });
        },
    );
}

/// Say what the running activity is aimed at — the port a flash writes to,
/// the tool an install fetches, the command a command runs.
pub(super) fn name_activity(state: AppState, target: String) {
    state.app.activity.update(|activity| {
        if let Some(activity) = activity {
            activity.target = Some(target);
        }
    });
}

/// What the status bar and the picker call a device: the port, or the
/// probe's identifier.
pub fn device_label(transport: &Transport) -> String {
    match transport {
        Transport::Serial { port } => port.clone(),
        Transport::Probe {
            identifier: Some(id),
        } => id.clone(),
        Transport::Probe { identifier: None } => "probe".to_string(),
    }
}

/// Work out the command the chosen device would get, for the picker to
/// show before anything runs.
///
/// The picker's preview, not the run: the run plans again against the build
/// it has just made, since the image path is only known once it exists.
pub fn plan_session(state: AppState, action: FlashAction) {
    let Some(transport) = state.device.transport.get_untracked() else {
        state.device.plan.set(None);
        return;
    };
    if action != FlashAction::Monitor && state.current_firmware_untracked().is_none() {
        state.device.plan.set(None);
        return;
    }
    let args = plan_args(state, transport, action);
    spawn_local(async move {
        // Quietly: a preview that cannot be planned says nothing, and the
        // run that follows says why in the banner and the dock.
        let plan = ipc::call::<_, CommandPlan>(cmd::flash::PLAN, &args)
            .await
            .ok();
        state.device.plan.set(plan);
    });
}

/// `cargo build --release`, then read `target/` again and weigh what it
/// made. Everything that builds goes through here — the title bar's Build,
/// and Flash before it writes — so a build always ends the same way.
pub fn build_then(state: AppState, after: impl FnOnce(bool) + 'static) {
    if state.app.session_running.get_untracked() {
        return;
    }
    // What builds is what is on screen.
    save_all_then(state, move || {
        if !state.app.session_running.get_untracked() {
            build_saved(state, after);
        }
    });
}

fn build_saved(state: AppState, after: impl FnOnce(bool) + 'static) {
    let plan = CommandPlan {
        program: "cargo".to_string(),
        args: vec!["build".to_string(), "--release".to_string()],
        display: "cargo build --release".to_string(),
        // Shown nowhere: the dock echoes the command, and the command is the
        // whole of what there is to say about it.
        rationale: String::new(),
        warning: None,
    };
    run_session_then(state, plan, "build", move |code| {
        let built = matches!(code, Some(0));
        // `target/` is not watched, so nothing else would notice the new
        // image: the Memory panel and the next flash would both be reading
        // the list from before the build.
        refresh_firmware_then(state, move || {
            if built {
                weigh(state);
            }
            after(built);
        });
    });
}

/// What the new image costs the chip — the line PlatformIO and Arduino end
/// every build with — into the dock, the status bar's verdict and the
/// Memory panel at once.
fn weigh(state: AppState) {
    let Some(firmware) = state.current_firmware_untracked() else {
        return;
    };
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        elf_path: String,
    }
    let args = Args {
        elf_path: firmware.path,
    };
    track(
        state,
        async move { ipc::call::<_, MemoryReport>(cmd::memory::REPORT, &args).await },
        move |report| {
            let totals = &report.totals;
            let size = Size {
                flash: totals.flash_bytes,
                ram: totals.ram_bytes,
                ram_capacity: totals.ram_capacity,
            };
            state.push_log(LogLine {
                stream: LogStream::Stdout,
                text: size_line(size),
                level: None,
            });
            state.app.outcome.update(|outcome| {
                if let Some(outcome) = outcome.as_mut().filter(|o| o.kind == Kind::Build && o.ok) {
                    outcome.size = Some(size);
                }
            });
            state.project.memory.set(Some(report));
        },
    );
}

/// `Flash 85.3 KB · RAM 20.1 KB of 320 KB (6%)`, worded for the reader.
pub fn size_line(size: Size) -> String {
    let flash = crate::format::bytes(size.flash);
    let ram = crate::format::bytes(size.ram);
    match size.ram_capacity.filter(|c| *c > 0) {
        Some(capacity) => {
            let percent = (size.ram as f64 / f64::from(capacity) * 100.0).round() as u32;
            t!(
                "activity.size-of",
                flash = flash,
                ram = ram,
                capacity = crate::format::bytes(u64::from(capacity)),
                percent = percent.to_string()
            )
        }
        None => t!("activity.size", flash = flash, ram = ram),
    }
}

/// A channel whose every line lands in the terminal, and the dock brought
/// forward to show it.
///
/// Shared by flashing, monitoring, project generation and the terminal itself:
/// four things that spawn a tool, and one place their output goes. Splitting
/// them into separate views would mean a failed flash and the build that caused
/// it appearing in different panes.
pub(super) fn stream_to_terminal(state: AppState) -> ipc::Channel {
    use wasm_bindgen::{JsValue, prelude::Closure};

    let channel = ipc::Channel::new();
    let on_line = Closure::wrap(Box::new(move |value: JsValue| {
        match serde_wasm_bindgen::from_value::<LogLine>(value) {
            // Through `absorb` rather than straight to the log: a board on the
            // end of `espflash monitor` prints the same telemetry it prints in
            // the simulator, and the plot should draw it. Writing back is what
            // that mode cannot do, not reading.
            Ok(line) => absorb(state, line),
            // A line that will not decode is still worth showing: it means the
            // wire type and the tool disagree, and silently dropping output is
            // the one thing a monitor must never do.
            Err(e) => state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: format!("[rusty could not decode a line from the tool: {e}]"),
                level: Some(LogLevel::Warn),
            }),
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_line);
    // Deliberately leaked: the backend holds this channel for the life of the
    // session, which outlives this call. One closure per run, freed never —
    // bounded by how many times a person presses a button.
    on_line.forget();

    state.app.session_running.set(true);
    // What the status bar says for as long as this runs, and the verdict
    // it replaces: the bar talks about the newest thing.
    let kind = Kind::of_channel(state.dock.source.get_untracked());
    state
        .app
        .activity
        .set(Some(Activity::new(kind, js_sys::Date::now())));
    state.app.outcome.set(None);
    state.show_dock(crate::state::DockTab::Output);
    channel
}

/// Feed one printed line to the running activity — the crate being
/// compiled, cargo's own counts, a flash that has finished writing.
///
/// A cheap look first (`relevant`): a simulation or a monitor prints
/// thousands of lines a second and none of them is about the activity. Set
/// only when something changed, since every set redraws the status bar.
pub(super) fn follow_activity(state: AppState, text: &str) {
    if !crate::activity::relevant(text) {
        return;
    }
    let now = js_sys::Date::now();
    let changed = state.app.activity.with_untracked(|activity| {
        let activity = activity.as_ref()?;
        let mut next = activity.clone();
        let done = next.observe(text, now);
        (next != *activity || done.is_some()).then_some((next, done))
    });
    if let Some((next, done)) = changed {
        state.app.activity.set(Some(next));
        if let Some(done) = done {
            state.app.outcome.set(Some(done));
        }
    }
}

/// The running activity's verdict, from how its process ended.
pub(super) fn end_activity(state: AppState, code: Option<i32>) {
    let Some(activity) = state.app.activity.get_untracked() else {
        return;
    };
    state.app.activity.set(None);
    if let Some(outcome) = activity.finish(code, js_sys::Date::now()) {
        state.app.outcome.set(Some(outcome));
    }
}

/// Stopped on purpose: no verdict, because a run somebody stopped did not
/// fail — and the last thing it finished, a flash before its monitor, keeps
/// its word.
pub(super) fn abandon_activity(state: AppState) {
    state.app.activity.set(None);
}

/// Note how a spawned tool ended, in the terminal where its output is.
pub(super) fn note_exit(state: AppState, code: Option<i32>) {
    state.app.session_running.set(false);
    // Nothing is running, so nothing is paused. Left set, the next run
    // would start with a Resume button over a simulation nobody stopped.
    state.sim.paused.set(false);
    end_activity(state, code);
    let source = state.dock.source;
    set_timeout(move || source.set("app"), std::time::Duration::ZERO);
    let text = match code {
        Some(0) | None => t!("misc.finished"),
        Some(code) => t!("misc.exited-with", code = code.to_string()),
    };
    state.push_log(LogLine {
        stream: LogStream::Stdout,
        text,
        level: None,
    });
    // What was waiting for this session to end: a flash that needed its
    // port, or the same simulation again with the code on screen.
    if let Some(next) = state.app.after_stop.get_untracked() {
        state.app.after_stop.set(None);
        match next {
            AfterStop::Device(action) => device_action(state, action),
            AfterStop::Simulate { debug } => simulate(state, debug),
        }
    }
}

/// Run the planned command, streaming its output into the terminal.
pub fn run_session(state: AppState, plan: CommandPlan, channel: &'static str) {
    run_session_then(state, plan, channel, |_| {});
}

/// [`run_session`], then `after` with the exit code — how Flash builds
/// before it writes.
pub fn run_session_then(
    state: AppState,
    plan: CommandPlan,
    channel: &'static str,
    after: impl FnOnce(Option<i32>) + 'static,
) {
    state.dock.source.set(channel);
    #[derive(serde::Serialize)]
    struct Args {
        plan: CommandPlan,
    }

    // The command first, as a typed one is: the dock is where it is read,
    // copied and pasted into a bug report, and a scrollback of several runs
    // with no headers is unreadable.
    state.push_log(LogLine {
        stream: LogStream::Stdout,
        text: format!("$ {}", plan.display),
        level: None,
    });
    let channel = stream_to_terminal(state);
    let args = Args { plan };
    track_session(
        state,
        async move {
            ipc::call_streaming::<_, Option<i32>>(cmd::flash::RUN, &args, "onLine", &channel).await
        },
        move |code| {
            note_exit(state, code);
            after(code);
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, likely_board: bool) -> SerialPort {
        SerialPort {
            name: name.to_string(),
            bridge: Some("CP210x".to_string()),
            boards: Vec::new(),
            likely_board,
            usb: None,
        }
    }

    fn probe(id: &str) -> Probe {
        Probe {
            identifier: id.to_string(),
            description: "ESP JTAG".to_string(),
        }
    }

    /// One board plugged in is the answer; two is a question. A modem or a
    /// Bluetooth port is never a candidate, so it cannot make one board into
    /// a question either.
    #[test]
    fn the_only_board_is_chosen_and_anything_more_is_asked() {
        let one = [port("COM3", true), port("COM1", false)];
        assert!(matches!(
            only_candidate(&one, &[], true),
            Some(Transport::Serial { port }) if port == "COM3"
        ));
        let two = [port("COM3", true), port("COM7", true)];
        assert!(only_candidate(&two, &[], true).is_none());
        assert!(only_candidate(&[port("COM1", false)], &[], true).is_none());
        assert!(only_candidate(&[], &[], true).is_none());
    }

    /// A C3 on native USB is a port and a probe at once. For a chip with a
    /// serial bootloader the port is the answer and the probe is not a
    /// second candidate; for one without, the probe is all there is.
    #[test]
    fn a_serial_chip_counts_ports_and_a_probe_only_chip_counts_probes() {
        let ports = [port("COM5", true)];
        let probes = [probe("303a:1001:34:85:18:0A:47:F4")];
        assert!(matches!(
            only_candidate(&ports, &probes, true),
            Some(Transport::Serial { .. })
        ));
        assert!(matches!(
            only_candidate(&ports, &probes, false),
            Some(Transport::Probe { identifier: Some(id) }) if id.starts_with("303a")
        ));
        assert!(only_candidate(&ports, &[probe("a"), probe("b")], false).is_none());
    }
}
