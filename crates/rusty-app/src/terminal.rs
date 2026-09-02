//! The terminal's commands.
//!
//! One session at a time, held in [`AppState`], because the panel shows one.
//! Opening a second replaces the first rather than leaking a shell nobody can
//! see or stop.
//!
//! Frames are pushed rather than polled: a shell produces output in bursts, and
//! a frontend asking "anything new?" sixty times a second would burn CPU doing
//! nothing for most of them.
//!
//! Which shell to run, and which shells exist, are answered by two pure
//! functions below with the machine passed in — the stored preference, the
//! executable, the PATH probe. They belong in `rusty-term` beside the pty they
//! configure (`rusty_term::shells::discover()` is the shape), and are kept here
//! with their tests so that move is mechanical.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use rusty_embed::config as storage;
use rusty_term::{Screen, ShellChoice, ShellInfo, Terminal};
use tauri::{State, ipc::Channel};

use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

/// Longest a frame may be held back to batch what follows it.
///
/// A `cargo build` writes thousands of lines a second; rendering each one would
/// spend the whole budget serialising screens that are replaced before anyone
/// sees them. Eight milliseconds is under a frame at 120 Hz, so batching is
/// invisible while cutting the work by orders of magnitude.
const COALESCE: Duration = Duration::from_millis(8);

/// The argv the stored preference means.
///
/// `preference` is `workbench.toml`'s `terminal_shell`; `exe` is this
/// executable. Auto is rusty itself re-entered as the built-in shell —
/// compiled in, so it exists wherever rusty does, starts in the time an exec
/// takes, and reads the same on every OS. `None` means "let rusty-term pick
/// the system default".
fn shell_argv(preference: Option<&str>, exe: Option<&Path>) -> Option<Vec<String>> {
    match preference.map(str::trim) {
        Some("system") => None,
        Some(custom) if !custom.is_empty() && custom != "auto" => Some(vec![custom.to_string()]),
        _ => exe.map(|exe| {
            vec![
                exe.to_string_lossy().into_owned(),
                "--builtin-shell".to_string(),
            ]
        }),
    }
}

/// What `set_terminal_shell` stores for what the picker sent: null, "" and
/// "auto" are all the built-in shell, which is the absent value.
fn shell_preference(value: Option<&str>) -> Option<String> {
    match value.map(str::trim) {
        None | Some("") | Some("auto") => None,
        Some(other) => Some(other.to_string()),
    }
}

/// The shells a machine offers: the built-in first, then whatever the OS
/// actually carries — detected, not assumed, so the picker never lists a shell
/// that fails to start.
///
/// Pure over its probes. `find_on_path` resolves a program name to the full
/// path it would run as, `is_file` says whether a candidate exists, and
/// `windows` picks the list — a parameter rather than `cfg!`, so both lists
/// are tested on every OS.
///
/// Full paths, found — never bare names. `bash.exe` on PATH is System32's WSL
/// relay on most Windows machines, which is how picking "Git Bash" once
/// produced a WSL error about /bin/bash; Git Bash is looked for in its real
/// homes and PATH is never asked for it.
fn shell_choices(
    windows: bool,
    program_files: &Path,
    find_on_path: &dyn Fn(&str) -> Option<PathBuf>,
    is_file: &dyn Fn(&Path) -> bool,
) -> Vec<ShellChoice> {
    let mut out = vec![ShellChoice {
        label: "rusty bash (built-in)".to_string(),
        value: "auto".to_string(),
    }];
    let mut push = |label: &str, path: PathBuf| {
        out.push(ShellChoice {
            label: label.to_string(),
            value: path.to_string_lossy().into_owned(),
        });
    };
    if windows {
        for (label, program) in [
            ("PowerShell 7", "pwsh.exe"),
            ("Windows PowerShell", "powershell.exe"),
            ("Command Prompt", "cmd.exe"),
            ("Nushell", "nu.exe"),
        ] {
            if let Some(path) = find_on_path(program) {
                push(label, path);
            }
        }
        for candidate in [
            program_files.join(r"Git\bin\bash.exe"),
            program_files.join(r"Git\usr\bin\bash.exe"),
        ] {
            if is_file(&candidate) {
                push("Git Bash", candidate);
                break;
            }
        }
    } else {
        for (label, program) in [
            ("bash", "bash"),
            ("zsh", "zsh"),
            ("fish", "fish"),
            ("Nushell", "nu"),
        ] {
            if let Some(path) = find_on_path(program) {
                push(label, path);
            }
        }
    }
    out
}

/// The shell picture for the settings page: what will run, and what the user
/// asked for.
#[tauri::command]
pub async fn terminal_shell_info() -> Result<ShellInfo, CommandError> {
    blocking("reading the shell preference", || {
        let preference = storage::workbench().terminal_shell;
        let exe = std::env::current_exe().ok();
        let active = match shell_argv(preference.as_deref(), exe.as_deref()) {
            Some(argv) if argv.len() > 1 => "rusty's built-in shell".to_string(),
            Some(argv) => argv.into_iter().next().unwrap_or_default(),
            None => rusty_term::default_shell(),
        };
        ShellInfo { active, preference }
    })
    .await
}

/// Store the shell preference: null/"auto" = the built-in shell, "system" =
/// the OS shell, anything else = a program to run.
#[tauri::command]
pub async fn set_terminal_shell(
    value: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let preference = shell_preference(value.as_deref());
    state
        .update_workbench(move |workbench| workbench.terminal_shell = preference)
        .await
}

/// The shells this machine can offer. See [`shell_choices`].
#[tauri::command]
pub async fn terminal_shells() -> Result<Vec<ShellChoice>, CommandError> {
    blocking("looking for shells", || {
        let find_on_path = |program: &str| -> Option<PathBuf> {
            let paths = std::env::var_os("PATH")?;
            std::env::split_paths(&paths)
                .map(|dir| dir.join(program))
                .find(|p| p.is_file())
        };
        let program_files = std::env::var("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(r"C:\Program Files"));
        shell_choices(cfg!(windows), &program_files, &find_on_path, &|p| {
            p.is_file()
        })
    })
    .await
}

/// Open a shell and stream its screen until it exits.
#[tauri::command]
pub async fn terminal_open(
    cols: u16,
    rows: u16,
    on_frame: Channel<Screen>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let cwd = state.firmware_root().await;
    let shell = blocking("reading the shell preference", || {
        let exe = std::env::current_exe().ok();
        shell_argv(
            storage::workbench().terminal_shell.as_deref(),
            exe.as_deref(),
        )
    })
    .await?;
    let (terminal, updates) =
        Terminal::spawn(cwd.as_deref(), cols.max(2), rows.max(1), shell.as_deref())?;
    let terminal = Arc::new(terminal);
    // Kept for the cleanup below: the reader closure consumes the other
    // handle, and the cleanup has to know which session it is ending.
    let ours = Arc::clone(&terminal);
    state.set_terminal(Some(Arc::clone(&terminal))).await;

    // The first frame is sent before anything arrives, so the view has a shape
    // to draw immediately instead of a blank rectangle until the shell prints
    // its prompt.
    let _ = on_frame.send(terminal.screen());

    // Blocking by nature — it sits on a channel — so it belongs on a blocking
    // thread rather than starving an async worker for the life of a shell.
    blocking("the terminal reader", move || {
        while updates.wait() {
            std::thread::sleep(COALESCE);
            let screen = terminal.screen();
            let done = screen.exited.is_some();
            if on_frame.send(screen).is_err() {
                // The WebView itself is gone — the only failure a send
                // reports. A shell with no window to draw into is a process
                // with no way to reach it.
                terminal.kill();
                break;
            }
            if done {
                break;
            }
        }
    })
    .await?;

    state.release_terminal(&ours).await;
    Ok(())
}

/// Send keystrokes.
#[tauri::command]
pub async fn terminal_write(
    bytes: Vec<u8>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
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
pub async fn terminal_scroll(
    delta: i32,
    state: State<'_, AppState>,
) -> Result<Screen, CommandError> {
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn the_preference_names_the_shell_that_will_run() {
        let exe = Path::new("/opt/rusty/rusty");
        let builtin = Some(vec![
            "/opt/rusty/rusty".to_string(),
            "--builtin-shell".to_string(),
        ]);

        assert_eq!(
            shell_argv(None, Some(exe)),
            builtin,
            "absent is the built-in"
        );
        assert_eq!(shell_argv(Some("auto"), Some(exe)), builtin);
        assert_eq!(
            shell_argv(Some("  "), Some(exe)),
            builtin,
            "blank is absent, not a program called nothing",
        );
        assert_eq!(
            shell_argv(Some("system"), Some(exe)),
            None,
            "None hands the choice to rusty-term's default",
        );
        assert_eq!(
            shell_argv(Some(" C:\\Program Files\\Git\\bin\\bash.exe "), Some(exe)),
            Some(vec!["C:\\Program Files\\Git\\bin\\bash.exe".to_string()]),
            "a program runs as itself, trimmed",
        );
        assert_eq!(
            shell_argv(None, None),
            None,
            "no executable to re-enter falls back to the system shell rather than to nothing",
        );
    }

    #[test]
    fn the_stored_preference_has_one_spelling_for_the_default() {
        assert_eq!(shell_preference(None), None);
        assert_eq!(shell_preference(Some("")), None);
        assert_eq!(shell_preference(Some("auto")), None);
        assert_eq!(
            shell_preference(Some(" system ")),
            Some("system".to_string())
        );
        assert_eq!(
            shell_preference(Some("pwsh.exe")),
            Some("pwsh.exe".to_string())
        );
    }

    /// Windows: PATH is asked for the shells that are safe to take from it,
    /// Git Bash is taken from its real home, and `bash.exe` on PATH — the WSL
    /// relay — is never so much as asked for.
    #[test]
    fn windows_shells_come_from_path_except_git_bash_which_comes_from_its_home() {
        let asked = RefCell::new(Vec::new());
        let find_on_path = |program: &str| -> Option<PathBuf> {
            asked.borrow_mut().push(program.to_string());
            (program == "pwsh.exe").then(|| PathBuf::from(r"C:\Tools\pwsh.exe"))
        };
        let is_file = |path: &Path| path.ends_with(r"Git\usr\bin\bash.exe");

        let choices = shell_choices(
            true,
            Path::new(r"C:\Program Files"),
            &find_on_path,
            &is_file,
        );
        let listed: Vec<(&str, &str)> = choices
            .iter()
            .map(|c| (c.label.as_str(), c.value.as_str()))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("rusty bash (built-in)", "auto"),
                ("PowerShell 7", r"C:\Tools\pwsh.exe"),
                ("Git Bash", r"C:\Program Files\Git\usr\bin\bash.exe"),
            ],
            "the built-in first, full paths after, only what exists",
        );
        assert!(
            !asked.borrow().iter().any(|p| p == "bash.exe"),
            "PATH must never be asked for bash.exe: {:?}",
            asked.borrow(),
        );
    }

    #[test]
    fn unix_shells_come_from_path_and_only_the_ones_that_exist() {
        let find_on_path = |program: &str| -> Option<PathBuf> {
            (program == "zsh").then(|| PathBuf::from("/bin/zsh"))
        };
        let choices = shell_choices(false, Path::new("/nonexistent"), &find_on_path, &|_| false);
        let listed: Vec<(&str, &str)> = choices
            .iter()
            .map(|c| (c.label.as_str(), c.value.as_str()))
            .collect();
        assert_eq!(
            listed,
            vec![("rusty bash (built-in)", "auto"), ("zsh", "/bin/zsh")],
        );
    }
}
