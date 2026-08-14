//! Simulation commands: plan the three steps, then run them end to end.

use rusty_embed::{LogLine, LogStream, SimBoard, SimPlan, process, project, simulate};
use tauri::{State, ipc::Channel};

use crate::{error::CommandError, state::AppState};

/// Persist the board editor's layout into the project's `.rusty/sim.toml`.
#[tauri::command]
pub async fn save_sim_board(
    board: SimBoard,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    simulate::save_board(&root, &board).map_err(CommandError::new)
}

/// How this project would be simulated, or exactly why it cannot be.
#[tauri::command]
pub async fn plan_simulation(state: State<'_, AppState>) -> Result<SimPlan, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let detected = project::detect(&root)?;
    Ok(simulate::plan(&detected))
}

/// Install one missing tool, streaming every line — the panel's one-click.
#[tauri::command]
pub async fn install_sim_tool(
    name: String,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    if name.starts_with("qemu-system-") || name.ends_with("-gdb") {
        return install_archive(&name, on_line, state).await;
    }
    let steps = simulate::install_steps(&name).map_err(CommandError::new)?;

    let mut last_code = None;
    for step in steps {
        let _ = on_line.send(LogLine {
            stream: LogStream::Stdout,
            text: format!("$ {}", step.display),
            level: None,
        });
        let session = process::spawn(&step, None)?;
        state.start_session(session.stopper()).await;

        let feed = on_line.clone();
        let code = tokio::task::spawn_blocking(move || {
            while let Some(line) = session.recv() {
                if feed.send(line).is_err() {
                    break;
                }
            }
            session.wait()
        })
        .await
        .map_err(|e| CommandError::new(format!("install step panicked: {e}")))?;

        last_code = code;
        if code != Some(0) {
            break;
        }
    }

    state.stop_session().await;
    Ok(last_code)
}

/// The captured waveform, written where the build artefacts already live.
/// Returns the absolute path so the dock can name it.
#[tauri::command]
pub async fn save_sim_trace(text: String, state: State<'_, AppState>) -> Result<String, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let dir = root.join("target/rusty-sim");
    std::fs::create_dir_all(&dir)
        .map_err(|e| CommandError::new(format!("could not create {}: {e}", dir.display())))?;
    let path = dir.join("trace.vcd");
    std::fs::write(&path, text)
        .map_err(|e| CommandError::new(format!("could not write {}: {e}", path.display())))?;
    Ok(path.to_string_lossy().into_owned())
}

/// A line into the running simulation's stdin — how a button press on the
/// board view reaches the firmware's UART.
#[tauri::command]
pub async fn sim_send(text: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    if let Some(input) = state.session_input().await {
        input.send_line(&text);
    }
    Ok(())
}

/// QEMU: in-process download with a mirror fallback, then tar extraction.
///
/// The download runs on rustls rather than through curl — the user's curl
/// died with a schannel abort and then could not reach github.com at all;
/// the Espressif mirror is the second URL for exactly that network.
async fn install_archive(
    name: &str,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    let plan = if name.ends_with("-gdb") {
        simulate::gdb_download(name).map_err(CommandError::new)?
    } else {
        simulate::qemu_download(name).map_err(CommandError::new)?
    };

    let feed = on_line.clone();
    let archive = plan.archive.clone();
    let urls = plan.urls.clone();
    let downloaded = tokio::task::spawn_blocking(move || {
        simulate::download(&urls, &archive, |line| {
            let _ = feed.send(LogLine {
                stream: LogStream::Stdout,
                text: line,
                level: None,
            });
        })
    })
    .await
    .map_err(|e| CommandError::new(format!("download panicked: {e}")))?;
    if let Err(error) = downloaded {
        return Err(CommandError::new(error));
    }

    let _ = on_line.send(LogLine {
        stream: LogStream::Stdout,
        text: format!("$ {}", plan.extract.display),
        level: None,
    });
    let session = process::spawn(&plan.extract, None)?;
    state.start_session(session.stopper()).await;
    let feed = on_line.clone();
    let code = tokio::task::spawn_blocking(move || {
        while let Some(line) = session.recv() {
            if feed.send(line).is_err() {
                break;
            }
        }
        session.wait()
    })
    .await
    .map_err(|e| CommandError::new(format!("extraction panicked: {e}")))?;
    state.stop_session().await;

    if code == Some(0) {
        let _ = std::fs::remove_file(&plan.archive);
    }
    Ok(code)
}

/// Build, image, boot — streaming every line, stoppable at any step.
///
/// The first two steps must exit zero before the next runs; QEMU itself runs
/// until the user stops it (the same session Stop every panel shares).
#[tauri::command]
pub async fn run_simulation(
    debug: bool,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let detected = project::detect(&root)?;
    let plan = simulate::plan(&detected);

    if !plan.supported {
        return Err(CommandError::new(plan.reason.unwrap_or_else(|| {
            "this project cannot be simulated".to_string()
        })));
    }
    if !plan.missing.is_empty() {
        let mut lines = vec!["simulation needs tools that are not installed:".to_string()];
        for tool in &plan.missing {
            lines.push(format!("  {} — {}", tool.name, tool.install));
        }
        return Err(CommandError::new(lines.join("\n")));
    }

    simulate::prepare(&root)
        .map_err(|e| CommandError::new(format!("could not create target/rusty-sim: {e}")))?;

    let mut last_code = None;
    let total = plan.steps.len();
    for (index, step) in plan.steps.into_iter().enumerate() {
        let _ = on_line.send(LogLine {
            stream: LogStream::Stdout,
            text: format!("$ {}", step.display),
            level: None,
        });

        let step = if debug && step.program.contains("qemu-system") {
            // Frozen at the first instruction with the gdbstub listening —
            // and a deterministic virtual clock, because a debugger that
            // perturbs timing hides the bugs people came to see.
            let mut armed = step.clone();
            for flag in ["-s", "-S", "-icount", "shift=auto,sleep=on"] {
                armed.args.push(flag.to_string());
            }
            armed.display = format!("{} -s -S -icount shift=auto,sleep=on", armed.display);
            let _ = on_line.send(LogLine {
                stream: LogStream::Stdout,
                text: "[rusty:debug] gdbstub on :1234, cpu frozen — attaching gdb in the \
                       terminal"
                    .to_string(),
                level: None,
            });
            armed
        } else {
            step.clone()
        };
        let session = process::spawn(&step, Some(root.as_path()))?;
        state.start_session(session.stopper()).await;
        // The boot step is QEMU; its stdin is the board's input path.
        state.set_session_input(Some(session.input())).await;

        let feed = on_line.clone();
        let code = tokio::task::spawn_blocking(move || {
            while let Some(line) = session.recv() {
                if feed.send(line).is_err() {
                    break;
                }
            }
            session.wait()
        })
        .await
        .map_err(|e| CommandError::new(format!("simulation step panicked: {e}")))?;

        last_code = code;
        let is_boot = index + 1 == total;
        if !is_boot && code != Some(0) {
            // A failed build or image stops the pipeline; the lines that
            // explain it are already in the dock.
            break;
        }
    }

    state.stop_session().await;
    Ok(last_code)
}
