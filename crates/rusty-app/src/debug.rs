//! The debugger's commands.
//!
//! One session at a time, like the terminal, and for the same reason: the
//! panel shows one, and a stranded gdb holds the ELF open so the next build
//! fails with a link error nobody connects to a debugger.
//!
//! State is pushed rather than polled. A stopped target changes nothing for
//! minutes at a time and then changes everything in one step; asking sixty
//! times a second would burn a core to learn nothing.

use std::sync::Arc;

use rusty_dbg::{DebugState, Debugger, Launch, Target};
use tauri::{State, ipc::Channel};

use crate::{error::CommandError, state::AppState};

/// Start gdb against a target that is already listening, and stream the
/// session's state until it ends.
///
/// The target — QEMU frozen at reset, or probe-rs serving hardware — is
/// started by whoever asked for the debug run; this attaches to it. Two
/// things starting QEMU would be two QEMUs.
#[tauri::command]
pub async fn debug_start(
    port: u16,
    hardware: bool,
    elf: String,
    on_state: Channel<DebugState>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let project = tokio::task::spawn_blocking({
        let root = root.clone();
        move || rusty_embed::project::detect(&root)
    })
    .await
    .map_err(|e| CommandError::new(format!("project detection panicked: {e}")))??;

    // Which gdb: the chip's architecture decides, and refusing beats
    // guessing — an Xtensa gdb cannot debug a RISC-V image, and the
    // failure it produces names neither.
    let gdb = rusty_embed::simulate::gdb_for(&project).ok_or_else(|| {
        CommandError::new(
            "No gdb for this chip is installed. The Simulate panel's tools card \
             installs the matching one.",
        )
    })?;

    let launch = Launch {
        gdb,
        elf: std::path::PathBuf::from(elf),
        target: if hardware {
            Target::Probe { port }
        } else {
            Target::Qemu { port }
        },
        root,
    };

    let (debugger, events) = tokio::task::spawn_blocking(move || Debugger::start(&launch))
        .await
        .map_err(|e| CommandError::new(format!("the debugger panicked while starting: {e}")))?
        .map_err(|e| CommandError::new(e.to_string()))?;

    let debugger = Arc::new(debugger);
    let ours = Arc::clone(&debugger);
    state.set_debugger(Some(Arc::clone(&debugger))).await;

    // Blocking by nature — it sits on a channel — so it belongs on a
    // blocking thread rather than starving an async worker for the life of
    // a debug session.
    tokio::task::spawn_blocking(move || {
        while let Some(update) = events.next() {
            let ended = update.exited.is_some();
            if on_state.send(update).is_err() {
                // The panel is gone; a debugger nobody can reach is a
                // process holding the ELF hostage.
                debugger.stop();
                break;
            }
            if ended {
                break;
            }
        }
    })
    .await
    .map_err(|e| CommandError::new(format!("the debug reader panicked: {e}")))?;

    state.release_debugger(&ours).await;
    Ok(())
}

/// Place or clear a breakpoint. Lines are zero-based on this side, as
/// everywhere else that crosses this boundary.
#[tauri::command]
pub async fn debug_breakpoint(
    file: String,
    line: u32,
    remove: Option<u32>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let debugger = state
        .debugger()
        .await
        .ok_or_else(|| CommandError::new("No debug session is running."))?;
    match remove {
        Some(number) => debugger.remove_breakpoint(number),
        None => debugger.add_breakpoint(&file, line),
    }
    .map_err(|e| CommandError::new(e.to_string()))
}

/// Resume, pause, or step. One command with a verb rather than five
/// commands: they differ by one MI line and share every guard.
#[tauri::command]
pub async fn debug_control(action: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    let debugger = state
        .debugger()
        .await
        .ok_or_else(|| CommandError::new("No debug session is running."))?;
    let result = match action.as_str() {
        "resume" => debugger.resume(),
        "pause" => debugger.pause(),
        "over" => debugger.step_over(),
        "into" => debugger.step_into(),
        "out" => debugger.step_out(),
        other => {
            return Err(CommandError::new(format!(
                "{other} is not something the debugger can do",
            )));
        }
    };
    result.map_err(|e| CommandError::new(e.to_string()))
}

/// Select a stack frame and read its variables.
#[tauri::command]
pub async fn debug_frame(level: u32, state: State<'_, AppState>) -> Result<(), CommandError> {
    let debugger = state
        .debugger()
        .await
        .ok_or_else(|| CommandError::new("No debug session is running."))?;
    debugger
        .refresh(level)
        .map_err(|e| CommandError::new(e.to_string()))
}

/// End the session.
#[tauri::command]
pub async fn debug_stop(state: State<'_, AppState>) -> Result<(), CommandError> {
    if let Some(debugger) = state.debugger().await {
        debugger.stop();
    }
    Ok(())
}
