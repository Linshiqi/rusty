//! The terminal's commands.
//!
//! One session at a time, held in [`AppState`], because the panel shows one.
//! Opening a second replaces the first rather than leaking a shell nobody can
//! see or stop.
//!
//! Frames are pushed rather than polled: a shell produces output in bursts, and
//! a frontend asking "anything new?" sixty times a second would burn CPU doing
//! nothing for most of them.

use std::{sync::Arc, time::Duration};

use rusty_embed::config as storage;
use rusty_term::{Screen, Terminal};
use tauri::{State, ipc::Channel};

use crate::{error::CommandError, state::AppState};

/// Longest a frame may be held back to batch what follows it.
///
/// A `cargo build` writes thousands of lines a second; rendering each one would
/// spend the whole budget serialising screens that are replaced before anyone
/// sees them. Eight milliseconds is under a frame at 120 Hz, so batching is
/// invisible while cutting the work by orders of magnitude.
const COALESCE: Duration = Duration::from_millis(8);

/// What the next shell start will run, per the stored preference.
///
/// The command is argv: Auto is rusty itself re-entered as the built-in
/// shell — compiled in, so it exists wherever rusty does, starts in the
/// time an exec takes, and reads the same on every OS. `None` means "let
/// rusty-term pick the system default".
fn resolved_shell() -> Option<Vec<String>> {
    match storage::workbench().terminal_shell.as_deref().map(str::trim) {
        Some("system") => None,
        Some(custom) if !custom.is_empty() => Some(vec![custom.to_string()]),
        _ => std::env::current_exe().ok().map(|exe| {
            vec![exe.to_string_lossy().into_owned(), "--builtin-shell".to_string()]
        }),
    }
}

/// The shell picture for the settings page: what will run, whether the
/// bundled Nushell exists, and what the user asked for.
#[tauri::command]
pub async fn terminal_shell_info() -> Result<rusty_embed::ShellInfo, CommandError> {
    let preference = storage::workbench().terminal_shell;
    let active = match resolved_shell() {
        Some(argv) if argv.len() > 1 => "rusty's built-in shell".to_string(),
        Some(argv) => argv.into_iter().next().unwrap_or_default(),
        None => rusty_term::default_shell(),
    };
    Ok(rusty_embed::ShellInfo {
        active,
        preference,
    })
}

/// Store the shell preference: null/"auto" = prefer the bundled Nushell,
/// "system" = the OS shell, anything else = a program to run.
#[tauri::command]
pub async fn set_terminal_shell(value: Option<String>) -> Result<(), CommandError> {
    let mut state = storage::workbench();
    state.terminal_shell = match value.as_deref().map(str::trim) {
        None | Some("") | Some("auto") => None,
        Some(other) => Some(other.to_string()),
    };
    storage::save_workbench(&state).map_err(CommandError::from)?;
    Ok(())
}

/// The shells this machine can offer: the built-in first, then whatever
/// the OS actually carries — detected, not assumed, so the picker never
/// lists a shell that fails to start.
#[tauri::command]
pub async fn terminal_shells() -> Result<Vec<rusty_embed::ShellChoice>, CommandError> {
    fn on_path(program: &str) -> bool {
        let Some(paths) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
    }

    let mut out = vec![rusty_embed::ShellChoice {
        label: "rusty bash (built-in)".to_string(),
        value: "auto".to_string(),
    }];
    let candidates: &[(&str, &str)] = if cfg!(windows) {
        &[
            ("PowerShell 7", "pwsh.exe"),
            ("Windows PowerShell", "powershell.exe"),
            ("Command Prompt", "cmd.exe"),
            ("Git Bash", "bash.exe"),
            ("Nushell", "nu.exe"),
        ]
    } else {
        &[("bash", "bash"), ("zsh", "zsh"), ("fish", "fish"), ("Nushell", "nu")]
    };
    for (label, program) in candidates {
        if on_path(program) {
            out.push(rusty_embed::ShellChoice {
                label: (*label).to_string(),
                value: (*program).to_string(),
            });
        }
    }
    Ok(out)
}

/// Open a shell and stream its screen until it exits.
#[tauri::command]
pub async fn terminal_open(
    cols: u16,
    rows: u16,
    on_frame: Channel<Screen>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let cwd = state.root().await;
    let shell = resolved_shell();
    let (terminal, updates) =
        Terminal::spawn(cwd.as_deref(), cols.max(2), rows.max(1), shell.as_deref())?;
    let terminal = Arc::new(terminal);
    state.set_terminal(Some(Arc::clone(&terminal))).await;

    // The first frame is sent before anything arrives, so the view has a shape
    // to draw immediately instead of a blank rectangle until the shell prints
    // its prompt.
    let _ = on_frame.send(terminal.screen());

    // Blocking by nature — it sits on a channel — so it belongs on a blocking
    // thread rather than starving an async worker for the life of a shell.
    tokio::task::spawn_blocking(move || {
        while updates.wait() {
            std::thread::sleep(COALESCE);
            let screen = terminal.screen();
            let done = screen.exited.is_some();
            if on_frame.send(screen).is_err() {
                // The panel is gone. Leaving the shell running would leak a
                // process with no way to reach it.
                terminal.kill();
                break;
            }
            if done {
                break;
            }
        }
    })
    .await
    .map_err(|e| CommandError::new(format!("the terminal reader panicked: {e}")))?;

    state.set_terminal(None).await;
    Ok(())
}

/// Send keystrokes.
#[tauri::command]
pub async fn terminal_write(bytes: Vec<u8>, state: State<'_, AppState>) -> Result<(), CommandError> {
    let terminal = state.terminal().await.ok_or_else(|| {
        CommandError::new("No terminal is open, so there is nothing to type into.")
    })?;
    Ok(terminal.write(&bytes)?)
}

/// Tell the shell the window changed size.
#[tauri::command]
pub async fn terminal_resize(
    cols: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    // Silently fine when nothing is open: resizes fire from a layout observer,
    // which does not know or care whether a shell is running.
    if let Some(terminal) = state.terminal().await {
        terminal.resize(cols.max(2), rows.max(1))?;
    }
    Ok(())
}

/// Move the view through scrollback. Positive scrolls back.
#[tauri::command]
pub async fn terminal_scroll(delta: i32, state: State<'_, AppState>) -> Result<Screen, CommandError> {
    let terminal = state
        .terminal()
        .await
        .ok_or_else(|| CommandError::new("No terminal is open."))?;
    terminal.scroll(delta);
    // Returned directly rather than waiting for a frame: scrolling changes what
    // is shown without the shell writing anything, so no update would ever come.
    Ok(terminal.screen())
}

/// End the session.
#[tauri::command]
pub async fn terminal_close(state: State<'_, AppState>) -> Result<(), CommandError> {
    if let Some(terminal) = state.terminal().await {
        terminal.kill();
    }
    Ok(())
}
