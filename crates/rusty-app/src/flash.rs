//! Running a flash or monitor session.
//!
//! Separate from `commands.rs` for the same reason `ai.rs` is: this is
//! long-running and streams, and everything else is request/response.

use rusty_embed::{CommandPlan, LogLine, flash};
use tauri::{State, ipc::Channel};

use crate::{error::CommandError, state::AppState};

/// Run a planned command, streaming its output.
///
/// Returns the process exit code. Anything the tool printed has already gone to
/// the channel by then, including whatever it said on the way to failing —
/// which is the part worth reading.
#[tauri::command]
pub async fn run_flash(
    plan: CommandPlan,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    let working_dir = state.root().await;

    // Spawning is quick and non-blocking; only the reading blocks.
    let session = flash::spawn(&plan, working_dir.as_deref())?;
    state.start_session(session.stopper()).await;

    // The reader loop is synchronous by nature — it sits on a pipe — so it
    // belongs on a blocking thread rather than starving an async worker for
    // however long a flash takes.
    let code = tokio::task::spawn_blocking(move || {
        while let Some(line) = session.recv() {
            // A closed channel means the user left the panel. Stop reading;
            // the session is torn down by whoever replaced or stopped it.
            if on_line.send(line).is_err() {
                break;
            }
        }
        session.wait()
    })
    .await
    .map_err(|e| CommandError::new(format!("flash session panicked: {e}")))?;

    state.stop_session().await;
    Ok(code)
}

/// Stop the running session.
///
/// The normal end of a monitor, not an error path.
#[tauri::command]
pub async fn stop_flash(state: State<'_, AppState>) -> Result<(), CommandError> {
    state.stop_session().await;
    Ok(())
}
