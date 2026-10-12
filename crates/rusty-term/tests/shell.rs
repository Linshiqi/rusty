//! A real shell, driven end to end.
//!
//! The renderer's unit tests feed the emulator bytes directly, which proves the
//! painting but not the part most likely to be wrong: whether a pseudo-terminal
//! opens at all, whether the shell believes it has a terminal, and whether what
//! is typed comes back. On Windows that is ConPTY, which either works or fails
//! in ways no amount of unit testing predicts.

use std::time::{Duration, Instant};

use rusty_term::Terminal;

/// Everything currently on screen, as plain text.
fn text(terminal: &Terminal) -> String {
    terminal
        .screen()
        .rows
        .iter()
        .map(|row| {
            row.spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Wait for the screen to contain `needle`, or give up.
///
/// Polling rather than waiting on one update: a shell prints its banner, its
/// prompt and the command's output as several separate writes, and which of
/// them lands first is not something a test should depend on.
fn wait_for(terminal: &Terminal, needle: &str, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if text(terminal).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn a_shell_starts_and_answers() {
    let (terminal, _updates) = Terminal::spawn(None, 80, 24, None).expect("a pseudo-terminal");

    // A marker rather than a word the shell might print by itself: on Windows
    // the banner mentions "PowerShell", and matching that would pass without
    // the command ever running.
    terminal
        .write(b"echo rusty-pty-works\r")
        .expect("write to the shell");

    let seen = wait_for(&terminal, "rusty-pty-works", Duration::from_secs(20));
    let screen = text(&terminal);
    terminal.kill();

    assert!(
        seen,
        "the shell never echoed the marker back; screen was:\n{screen}",
    );
}

#[test]
fn resizing_reaches_the_emulator() {
    let (terminal, _updates) = Terminal::spawn(None, 80, 24, None).expect("a pseudo-terminal");

    assert_eq!(terminal.screen().cols, 80);
    terminal.resize(120, 30).expect("resize");

    let screen = terminal.screen();
    terminal.kill();

    // The shell learns its width from the pty and the emulator has to agree, or
    // a full-screen program draws to one width while the screen stores another.
    assert_eq!(screen.cols, 120);
    assert_eq!(screen.rows.len(), 30);
}

#[test]
fn the_screen_reports_the_shell_exiting() {
    let (terminal, _updates) = Terminal::spawn(None, 80, 24, None).expect("a pseudo-terminal");
    terminal.write(b"exit\r").expect("write to the shell");

    // Polled rather than waiting on an update. `Updates::wait` blocks with no
    // deadline, so a shell that goes quiet without exiting would hang the test
    // forever instead of failing it — which is exactly what it did.
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline && terminal.screen().exited.is_none() {
        std::thread::sleep(Duration::from_millis(50));
    }

    let exited = terminal.screen().exited;
    terminal.kill();
    assert!(exited.is_some(), "the exit was never reported");
}
