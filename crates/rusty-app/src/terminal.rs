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
/// `preference` is `workbench.toml`'s `terminal_shell`; `builtin` is what
/// runs the built-in shell on this machine ([`builtin_argv`]). Auto is the
/// built-in shell — compiled in, so it exists wherever rusty does, starts in
/// the time an exec takes, and reads the same on every OS. `None` means "let
/// rusty-term pick the system default", which is also what auto comes to
/// where nothing can host the built-in shell.
fn shell_argv(preference: Option<&str>, builtin: Option<Vec<String>>) -> Option<Vec<String>> {
    match preference.map(str::trim) {
        Some("system") => None,
        Some(custom) if !custom.is_empty() && custom != "auto" => Some(vec![custom.to_string()]),
        _ => builtin,
    }
}

/// The built-in shell as a program of its own (`rusty-term`'s `rusty-shell`).
const SHELL_PROGRAM: &str = if cfg!(windows) {
    "rusty-shell.exe"
} else {
    "rusty-shell"
};

/// What runs the built-in shell: `rusty-shell` where it ships beside the
/// app — the installer puts it among the bundled tools — or this executable
/// re-entered with `--builtin-shell` when it can be a pseudoconsole's program
/// itself. `None` when neither can.
///
/// **A GUI-subsystem executable cannot be.** A release build on Windows is
/// one (`windows_subsystem`, so no console window opens behind the app), and
/// Windows connects a pseudoconsole only to console programs: re-entered as
/// the shell, the app had no standard handles, read end-of-input at once and
/// exited 0 having printed nothing — "The shell exited with status 0." in an
/// empty terminal, every time on an installed app and never in a debug build,
/// which is a console program. Whether the executable can is read off its
/// header ([`console_subsystem`]) rather than assumed from the build.
///
/// Pure over its probes, like [`shell_choices`].
fn builtin_argv(
    exe: &Path,
    is_file: &dyn Fn(&Path) -> bool,
    is_console_program: &dyn Fn(&Path) -> bool,
) -> Option<Vec<String>> {
    let dir = exe.parent()?;
    for candidate in [
        dir.join(SHELL_PROGRAM),
        dir.join("bundled").join(SHELL_PROGRAM),
    ] {
        if is_file(&candidate) {
            return Some(vec![candidate.to_string_lossy().into_owned()]);
        }
    }
    is_console_program(exe).then(|| {
        vec![
            exe.to_string_lossy().into_owned(),
            "--builtin-shell".to_string(),
        ]
    })
}

/// Whether a Windows executable image is a console program, read off its PE
/// header: the optional header's subsystem field, 3 for the console and 2
/// for a window. `None` for anything that is not a PE image.
///
/// The field sits 68 bytes into the optional header in both the 32- and the
/// 64-bit layouts, which differ only after it.
fn console_subsystem(image: &[u8]) -> Option<bool> {
    if image.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32::from_le_bytes(image.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if image.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let field = pe + 4 + 20 + 68;
    let subsystem = u16::from_le_bytes(image.get(field..field + 2)?.try_into().ok()?);
    Some(subsystem == 3)
}

/// Whether this machine can run `exe` as a pseudoconsole's program. Anything
/// but Windows can; on Windows, a console program can and a window program
/// cannot, and a header that cannot be read is taken at its word as "no",
/// since a shell that exits at once is the failure being avoided.
fn hosts_a_console(exe: &Path) -> bool {
    if !cfg!(windows) {
        return true;
    }
    use std::io::Read;
    let mut head = Vec::new();
    let read = std::fs::File::open(exe).and_then(|file| file.take(4096).read_to_end(&mut head));
    read.is_ok() && console_subsystem(&head) == Some(true)
}

/// [`builtin_argv`] for this process.
fn builtin_here() -> Option<Vec<String>> {
    let exe = std::env::current_exe().ok()?;
    builtin_argv(&exe, &|path| path.is_file(), &hosts_a_console)
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
        // Spelled with backslashes rather than `join`ed: `Path::join` inserts
        // the *host's* separator, so on the Linux CI runner the Windows list
        // came out as `C:\Program Files/Git\usr\bin\bash.exe` and the test
        // of it, which asserts the spelling a Windows user sees, failed there
        // and nowhere else. This branch only ever runs for Windows.
        for tail in [r"Git\bin\bash.exe", r"Git\usr\bin\bash.exe"] {
            let candidate = PathBuf::from(format!("{}\\{tail}", program_files.display()));
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
        let builtin = builtin_here();
        let active = match shell_argv(preference.as_deref(), builtin.clone()) {
            Some(argv) if Some(&argv) == builtin.as_ref() => "rusty's built-in shell".to_string(),
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
        shell_argv(
            storage::workbench().terminal_shell.as_deref(),
            builtin_here(),
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
        let builtin = || {
            Some(vec![
                "/opt/rusty/rusty".to_string(),
                "--builtin-shell".to_string(),
            ])
        };

        assert_eq!(
            shell_argv(None, builtin()),
            builtin(),
            "absent is the built-in"
        );
        assert_eq!(shell_argv(Some("auto"), builtin()), builtin());
        assert_eq!(
            shell_argv(Some("  "), builtin()),
            builtin(),
            "blank is absent, not a program called nothing",
        );
        assert_eq!(
            shell_argv(Some("system"), builtin()),
            None,
            "None hands the choice to rusty-term's default",
        );
        assert_eq!(
            shell_argv(Some(" C:\\Program Files\\Git\\bin\\bash.exe "), builtin()),
            Some(vec!["C:\\Program Files\\Git\\bin\\bash.exe".to_string()]),
            "a program runs as itself, trimmed",
        );
        assert_eq!(
            shell_argv(None, None),
            None,
            "nothing to run the built-in shell falls back to the system shell rather than to nothing",
        );
    }

    /// The shell's own program wins wherever it ships — beside the app or
    /// among the bundled tools — and the app re-enters itself only when it
    /// can be a pseudoconsole's program: never as a window program, which
    /// exits at once there having printed nothing.
    #[test]
    fn the_built_in_shell_runs_as_a_console_program_or_not_at_all() {
        let exe = Path::new("/opt/rusty/rusty");
        let shell = |path: &Path| path.ends_with(format!("bundled/{SHELL_PROGRAM}"));
        let window_program = |_: &Path| false;
        let console_program = |_: &Path| true;
        assert_eq!(
            builtin_argv(exe, &shell, &window_program),
            Some(vec![
                Path::new("/opt/rusty")
                    .join("bundled")
                    .join(SHELL_PROGRAM)
                    .to_string_lossy()
                    .into_owned()
            ]),
        );
        let nothing = |_: &Path| false;
        assert_eq!(
            builtin_argv(exe, &nothing, &console_program),
            Some(vec![
                "/opt/rusty/rusty".to_string(),
                "--builtin-shell".to_string()
            ]),
        );
        assert_eq!(
            builtin_argv(exe, &nothing, &window_program),
            None,
            "a window program with no shell beside it runs the system shell instead",
        );
    }

    /// A PE header's subsystem, in a built image and in the test's own
    /// executable — which is a console program, as `cargo test` builds it.
    #[test]
    fn a_console_program_is_told_from_a_window_program_by_its_header() {
        let image = |subsystem: u16| {
            let mut bytes = vec![0u8; 0x200];
            bytes[..2].copy_from_slice(b"MZ");
            bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
            bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
            let field = 0x80 + 4 + 20 + 68;
            bytes[field..field + 2].copy_from_slice(&subsystem.to_le_bytes());
            bytes
        };
        assert_eq!(console_subsystem(&image(3)), Some(true));
        assert_eq!(console_subsystem(&image(2)), Some(false));
        assert_eq!(console_subsystem(b"#!/bin/sh"), None);
        assert_eq!(console_subsystem(&image(3)[..0x90]), None, "cut short");
        if cfg!(windows) {
            let exe = std::env::current_exe().expect("the test's executable");
            assert!(hosts_a_console(&exe), "a test runner is a console program");
        }
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
        // On the text, not through `Path::ends_with`: that compares components,
        // and on Linux a backslash is not a separator, so the Windows path is
        // one component and the suffix never matches. The test runs on every
        // OS on purpose — see `shell_choices`.
        let is_file = |path: &Path| path.to_string_lossy().ends_with(r"Git\usr\bin\bash.exe");

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
