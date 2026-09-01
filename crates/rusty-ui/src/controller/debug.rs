//! Breakpoints, stepping, and the registers underneath.

use leptos::prelude::*;
use leptos::task::spawn_local;

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

/// Attach the debugger to a target that is already listening, and keep the
/// panel's state fed until the session ends.
///
/// Which binary and which port are the backend's to know. This used to pass
/// them up out of the cached simulation plan — a plan built for a *release*
/// run — so a debug run booted the unoptimised image while gdb read the
/// optimised one and no breakpoint could ever hit.
pub fn debug_start(state: AppState, hardware: bool) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    #[derive(serde::Serialize)]
    struct Args {
        hardware: bool,
    }

    // Generations, exactly as the terminal needed them: a replaced
    // session's late frames must not overwrite the one that replaced it.
    let epoch = state.debug.epoch.get_untracked() + 1;
    state.debug.epoch.set(epoch);
    // Whether this session has been told about the standing breakpoints.
    let placed = RwSignal::new(false);
    // Frame 0's address at the last stop the editor was moved to.
    let stopped_at = RwSignal::new(None::<String>);
    state
        .debug
        .session
        .set(Some(rusty_dbg::DebugState::default()));
    state.show_dock(crate::state::DockTab::Debug);

    let channel = ipc::Channel::new();
    let on_state = Closure::wrap(Box::new(move |value: JsValue| {
        if state.debug.epoch.get_untracked() != epoch {
            return;
        }
        if let Ok(update) = serde_wasm_bindgen::from_value::<rusty_dbg::DebugState>(value) {
            // The standing list, once there is a session to place it in.
            // Sending before attach would be refused: the backend has no
            // debugger registered until gdb has answered.
            if update.attached && !placed.get_untracked() {
                placed.set(true);
                for (file, line) in state.debug.breakpoints.get_untracked() {
                    send_breakpoint(state, file, line, None);
                }
                // And then run. `-S` froze the CPU at the reset vector so
                // the breakpoints could be placed before a single
                // instruction executed; without this the session sits there
                // for ever showing `?? 0x40000400`, which is what "the
                // breakpoint never hits" looked like. The commands share
                // one stdin, so the placements are already in front of it.
                debug_control(state, "resume");
            }
            // gdb moves a breakpoint to the next line that has code —
            // an optimised build has none on many lines. The dot follows
            // it, as VSCode's does: it must mark where execution will
            // actually stop, not the line the compiler deleted.
            for placed in &update.breakpoints {
                let Some(asked) = placed.requested else {
                    continue;
                };
                if asked == placed.line {
                    continue;
                }
                let (file, landed) = (placed.file.clone(), placed.line);
                state.debug.breakpoints.update(|list| {
                    if let Some(entry) = list.iter_mut().find(|(f, l)| f == &file && *l == asked) {
                        entry.1 = landed;
                    }
                });
            }

            // Landing on the stopped line is the whole point of stopping —
            // but only when it has *newly* stopped. Every update carries the
            // whole stack, so revealing on each one dragged the editor back
            // to frame 0 a microtask after any click that asked for somewhere
            // else: placing a second breakpoint threw you back to the first,
            // and selecting an outer frame looked like it did nothing at all.
            //
            // Cleared while running, so stopping twice in the same place —
            // a breakpoint in a loop — still reveals.
            if update.running {
                stopped_at.set(None);
            } else if let Some(frame) = update.stack.first() {
                let arrived =
                    stopped_at.with_untracked(|last| last.as_deref() != Some(&frame.address));
                if arrived {
                    stopped_at.set(Some(frame.address.clone()));
                    if let (Some(file), Some(line)) = (frame.file.clone(), frame.line) {
                        open_at(state, file, line, 0);
                    }
                }
            }
            state.debug.session.set(Some(update));
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_state);
    on_state.forget();

    let args = Args { hardware };
    spawn_local(async move {
        let outcome =
            ipc::call_streaming::<_, ()>(cmd::debug::START, &args, "onState", &channel).await;
        if state.debug.epoch.get_untracked() != epoch {
            return;
        }
        if let Err(error) = outcome {
            state.app.error.set(Some(error));
        }
        state.debug.session.set(None);
    });
}

/// Toggle a breakpoint. Lines are zero-based, as everywhere.
///
/// The list is the editor's and survives sessions; a live session is told
/// about the change as it happens, and a session starting later is told
/// the whole list.
pub fn debug_breakpoint(state: AppState, file: String, line: u32) {
    let existed = state
        .debug
        .breakpoints
        .with_untracked(|list| list.iter().any(|(f, l)| f == &file && *l == line));
    state.debug.breakpoints.update(|list| {
        if existed {
            list.retain(|(f, l)| !(f == &file && *l == line));
        } else {
            list.push((file.clone(), line));
        }
    });

    // Nothing to tell gdb about if gdb is not running — the list is the
    // record, and it will be sent when a session starts.
    if state.debug.session.with_untracked(Option::is_none) {
        return;
    }
    let number = state.debug.session.with_untracked(|debug| {
        debug.as_ref().and_then(|debug| {
            debug
                .breakpoints
                .iter()
                .find(|b| b.file == file && b.line == line)
                .and_then(|b| b.number)
        })
    });
    send_breakpoint(state, file, line, existed.then_some(number).flatten());
}

/// One breakpoint over the wire: placing when `remove` is absent, clearing
/// by gdb's own number when it is.
fn send_breakpoint(state: AppState, file: String, line: u32, remove: Option<u32>) {
    #[derive(serde::Serialize)]
    struct Args {
        file: String,
        line: u32,
        remove: Option<u32>,
    }
    let args = Args { file, line, remove };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::debug::BREAKPOINT, &args).await },
        move |()| {},
    );
}

/// Resume, pause, or step.
pub fn debug_control(state: AppState, action: &'static str) {
    #[derive(serde::Serialize)]
    struct Args {
        action: &'static str,
    }
    let args = Args { action };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::debug::CONTROL, &args).await },
        move |()| {},
    );
}

/// Select a stack frame and read its variables.
pub fn debug_frame(state: AppState, level: u32) {
    #[derive(serde::Serialize)]
    struct Args {
        level: u32,
    }
    let args = Args { level };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::debug::FRAME, &args).await },
        move |()| {},
    );
}

/// End the session.
pub fn debug_stop(state: AppState) {
    state.debug.epoch.update(|epoch| *epoch += 1);
    state.debug.session.set(None);
    spawn_local(async move {
        let _ = ipc::get::<serde_json::Value>(cmd::debug::STOP).await;
    });
}

/// Write the C-interop scaffolding, then say what landed and run the one
/// command it needs. The written files go to the dock, because a workbench
/// that creates four files without naming them is a workbench you stop
/// trusting with your project.
pub fn scaffold_c_interop(state: AppState, direction: &'static str) {
    #[derive(serde::Serialize)]
    struct Args {
        direction: &'static str,
    }
    let args = Args { direction };
    track(
        state,
        async move { ipc::call::<_, rusty_embed::ScaffoldReport>(cmd::wizard::C_INTEROP, &args).await },
        move |report| {
            state.show_dock(crate::state::DockTab::Output);
            for path in &report.written {
                state.push_log(rusty_embed::LogLine {
                    stream: rusty_embed::LogStream::Stdout,
                    text: format!("wrote {path}"),
                    level: None,
                });
            }
            state.push_log(rusty_embed::LogLine {
                stream: rusty_embed::LogStream::Stdout,
                text: format!("next: {}", report.next),
                level: None,
            });
            refresh_tree(state);
            if let Some(command) = report.command {
                run_session(state, command, "build");
            }
        },
    );
}

/// Read the chip's SVD, if this machine has one.
pub fn load_registers(state: AppState) {
    track(
        state,
        async move { ipc::get::<Option<rusty_embed::RegisterMap>>(cmd::debug::REGISTERS).await },
        move |map| state.debug.registers.set(Some(map)),
    );
}

/// Fetch the chip's SVD, then read it. The download's progress goes to the
/// dock like every other download's.
pub fn fetch_svd(state: AppState) {
    let channel = stream_to_terminal(state);
    state.show_dock(crate::state::DockTab::Output);
    track_session(
        state,
        async move {
            ipc::call_streaming::<_, ()>(cmd::debug::FETCH_SVD, &(), "onLine", &channel).await
        },
        move |()| load_registers(state),
    );
}

/// Read a peripheral's register block from the target — one request for the
/// whole span rather than a round trip per register.
pub fn read_peripheral(state: AppState, base: u64, bytes: u32) {
    #[derive(serde::Serialize)]
    struct Args {
        address: u64,
        bytes: u32,
    }
    let args = Args {
        address: base,
        bytes,
    };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::debug::READ, &args).await },
        move |()| {},
    );
}
