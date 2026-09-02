//! Running a flash or monitor session.
//!
//! Separate from `commands.rs` for the same reason `ai.rs` is: this is
//! long-running and streams, and everything else is request/response.
//!
//! Every session here is registered in `AppState`'s session slot and released
//! from it *by identity* when its reader ends. A reader that released whatever
//! was in the slot killed the session that had replaced it — see
//! `AppState::release_session`.

use std::path::PathBuf;

use rusty_embed::{CommandPlan, LogLine, WizardChoice, process, toolchain, wizard};
use tauri::{State, ipc::Channel};

use crate::{
    error::CommandError,
    state::{AppState, blocking},
    stream,
};

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
    let working_dir = state.firmware_root().await;

    // Spawning is quick and non-blocking; only the reading blocks.
    let session = process::spawn(&plan, working_dir.as_deref())?;
    let ours = state.start_session(session.stopper()).await;

    // The reader loop is synchronous by nature — it sits on a pipe — so it
    // belongs on a blocking thread rather than starving an async worker for
    // however long a flash takes.
    let code = blocking("the flash session", move || {
        stream::forward(|| session.recv(), &on_line);
        session.wait()
    })
    .await?;

    state.release_session(&ours).await;
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

/// Hold a serial port open in both directions.
///
/// The difference from [`run_flash`] with `FlashAction::Monitor` is the return
/// path: `espflash monitor` reads its keyboard through the console rather than
/// through stdin, so nothing rusty spawns can talk back to the board. This
/// opens the port itself, which is what makes a tunable writable — and costs
/// defmt decoding, since that is espflash's.
#[tauri::command]
pub async fn serial_link(
    port: String,
    baud: u32,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    let link = blocking("opening the serial port", move || {
        rusty_embed::serial::open(&port, baud)
    })
    .await??;
    let ours = state.start_session(link.stopper()).await;
    state.set_session_input(Some(link.input())).await;

    // Same shape as a spawned tool: a blocking reader on its own thread.
    blocking("the serial link", move || {
        stream::forward(|| link.recv(), &on_line);
    })
    .await?;

    state.release_session(&ours).await;
    // No exit code: nothing exited. `None` is what the frontend already reads
    // as "it finished without a status", which is exactly true here.
    Ok(None)
}

/// Generate a project, streaming the generator's output.
///
/// Actually creates it rather than handing the user a command to paste. Showing
/// the command is still worth doing — it is what makes the tool inspectable —
/// but showing it *instead* of acting makes the panel a very slow way to type.
///
/// The user picks the parent directory, so the one decision rusty must not make
/// silently is still theirs; everything after that is mechanical.
#[tauri::command]
pub async fn create_project(
    choice: WizardChoice,
    directory: String,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    let parent = PathBuf::from(&directory);
    let plan = wizard::plan(&choice)?;
    let destination = wizard::destination(&parent, &choice);

    // Refuse before spawning. `esp-generate` into an existing directory either
    // fails halfway or overwrites, and neither is something to discover from a
    // log line after the fact.
    if destination.exists() {
        return Err(CommandError::new(format!(
            "{} already exists. Choose another name or another folder — rusty will not \
             generate into a directory that is already there.",
            destination.display(),
        )));
    }

    // A missing generator is the most likely failure here and the one that most
    // needs an answer rather than a diagnosis. The tool table already knows how
    // to install every tool rusty drives; saying "not found" without it leaves
    // the user to search for a crate name.
    let session =
        process::spawn(&plan, Some(&parent)).map_err(|e| {
            match toolchain::install_command(&plan.program) {
                Some(install) => CommandError::new(format!(
                    "`{}` is not installed, so there is nothing to generate the project with. \
                 Install it with `{install}` — you can run that in the terminal below.",
                    plan.program,
                )),
                None => CommandError::from(e),
            }
        })?;
    let ours = state.start_session(session.stopper()).await;

    let code = blocking("the generator", move || {
        stream::forward(|| session.recv(), &on_line);
        session.wait()
    })
    .await?;

    state.release_session(&ours).await;

    match code {
        Some(0) | None if destination.exists() => Ok(destination.display().to_string()),
        // The generator ran and refused — it cannot be missing, or the spawn
        // above would have failed. Suggesting `cargo install` here was simply
        // wrong, and it sent people to reinstall a tool that had just printed a
        // perfectly good reason two lines below.
        _ => Err(CommandError::new(format!(
            "`{}` ran but did not create {}. Its own message is in the output below — that is \
             the reason.",
            plan.program,
            destination.display(),
        ))),
    }
}

/// Run an arbitrary command in the project, streaming its output.
///
/// Not a terminal emulator: there is no pty, so nothing that wants a prompt or
/// draws with cursor movement will behave. It exists because the commands this
/// workbench is *about* — cargo, espflash, probe-rs, git — are all
/// non-interactive, and making people leave the window to run them is how a
/// tool becomes something you alt-tab away from.
#[tauri::command]
pub async fn run_command(
    program: String,
    args: Vec<String>,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    let working_dir = state.firmware_root().await;
    let plan = CommandPlan {
        display: std::iter::once(program.clone())
            .chain(args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" "),
        program,
        args,
        rationale: String::new(),
        warning: None,
    };

    let session = process::spawn(&plan, working_dir.as_deref())?;
    let ours = state.start_session(session.stopper()).await;

    let code = blocking("the command", move || {
        stream::forward(|| session.recv(), &on_line);
        session.wait()
    })
    .await?;

    state.release_session(&ours).await;
    Ok(code)
}
