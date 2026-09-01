//! What is plugged in, and getting a binary onto it.

use leptos::prelude::*;

use rusty_embed::{
    CommandPlan, FlashAction, LogLevel, LogLine, LogStream, Probe, SerialPort, Transport,
};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

/// Re-enumerate serial ports and debug probes.
///
/// Explicit rather than polled. Enumerating serial ports opens each device on
/// some platforms, and doing that on a timer while a monitor is attached is a
/// good way to disturb the session the user is watching.
pub fn scan_devices(state: AppState) {
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
        },
    );
    track(
        state,
        ipc::get::<Vec<Probe>>(cmd::flash::DEBUG_PROBES),
        move |probes| state.device.probes.set(probes),
    );
}

/// Work out the command for the current device, firmware and action.
///
/// Always run before flashing, and the result is shown verbatim. Embedded work
/// happens in a terminal as much as in a window; a button that hides what it
/// runs is a button people work around.
pub fn plan_session(state: AppState, action: FlashAction) {
    let (Some(transport), Some(firmware)) =
        (state.device.transport.get(), state.current_firmware())
    else {
        state.device.plan.set(None);
        return;
    };

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        transport: Transport,
        action: FlashAction,
        firmware: String,
        defmt: bool,
        baud: Option<u32>,
    }

    // Whether to decode defmt is a property of the binary, not a preference:
    // asking espflash to decode a build without the string table produces
    // gibberish, and not asking on a build with one produces framing bytes.
    let defmt = state
        .project
        .detected
        .with(|p| p.as_ref().is_some_and(|p| p.uses_defmt));

    let args = Args {
        transport,
        action,
        firmware: firmware.path,
        defmt,
        baud: None,
    };
    track(
        state,
        async move { ipc::call::<_, CommandPlan>(cmd::flash::PLAN, &args).await },
        move |plan| state.device.plan.set(Some(plan)),
    );
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
    state.show_dock(crate::state::DockTab::Output);
    channel
}

/// Note how a spawned tool ended, in the terminal where its output is.
pub(super) fn note_exit(state: AppState, code: Option<i32>) {
    state.app.session_running.set(false);
    let source = state.dock.source;
    set_timeout(move || source.set("app"), std::time::Duration::ZERO);
    let text = match code {
        Some(0) | None => "— finished".to_string(),
        Some(code) => format!("— exited with status {code}"),
    };
    state.push_log(LogLine {
        stream: LogStream::Stdout,
        text,
        level: None,
    });
}

/// Run the planned command, streaming its output into the terminal.
pub fn run_session(state: AppState, plan: CommandPlan, channel: &'static str) {
    state.dock.source.set(channel);
    #[derive(serde::Serialize)]
    struct Args {
        plan: CommandPlan,
    }

    let channel = stream_to_terminal(state);
    let args = Args { plan };
    track_session(
        state,
        async move {
            ipc::call_streaming::<_, Option<i32>>(cmd::flash::RUN, &args, "onLine", &channel).await
        },
        move |code| note_exit(state, code),
    );
}
