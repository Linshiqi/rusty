//! The built-in shell.
//!
//! Compiled into rusty itself and started as `rusty --builtin-shell` inside
//! the pty, so the terminal works the moment the app is installed — no
//! download, no PowerShell profile spending two seconds before the prompt,
//! and the same language on every OS.
//!
//! The language is bash. Execution is `brush-core`, a POSIX/bash-compatible
//! shell written in Rust, vendored at the workspace root with a Windows PATH
//! fix (see the root Cargo.toml). Pipes, redirection, globs, variables,
//! command substitution, conditionals and loops all behave; external
//! commands — the cargo, espflash, git and gdb this workbench is about —
//! spawn as real processes inheriting the pty.
//!
//! Only the line editing is ours: echo, backspace, Ctrl+C, and arrow-key
//! history over the raw VT bytes ConPTY delivers. brush's own interactive
//! frontend assumes it owns a real console; inside a pty slave the simple
//! loop is the reliable one.

use std::io::{Read, Write};

/// Run the shell until EOF or `exit`. Never returns to the caller.
pub fn run() -> ! {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a single-threaded runtime");
    let code = runtime.block_on(repl());
    std::process::exit(code);
}

async fn repl() -> i32 {
    let mut stdout = std::io::stdout();

    let builtins = brush_builtins::default_builtins::<
        brush_core::extensions::DefaultShellExtensions,
    >(brush_builtins::BuiltinSet::BashMode);
    let mut shell = match brush_core::Shell::builder().builtins(builtins).build().await {
        Ok(shell) => shell,
        Err(error) => {
            let _ = writeln!(stdout, "the shell engine failed to start: {error}\r");
            return 1;
        }
    };

    let stdin = std::io::stdin();
    let mut bytes = stdin.lock().bytes();
    let mut history: Vec<String> = Vec::new();

    let _ = writeln!(stdout, "rusty shell — bash syntax, built in.\r");

    loop {
        prompt(&mut stdout, shell.working_dir());
        let Some(line) = read_line(&mut bytes, &mut stdout, &history) else {
            let _ = writeln!(stdout, "\r");
            return 0;
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if history.last() != Some(&line) {
            history.push(line.clone());
        }

        // `clear` before brush sees it: there is no terminfo in here, and
        // the two escape bytes are all a clear ever was.
        if line == "clear" {
            let _ = write!(stdout, "\x1b[2J\x1b[H");
            let _ = stdout.flush();
            continue;
        }

        let params = shell.default_exec_params();
        match shell
            .run_string(line, &brush_core::SourceInfo::default(), &params)
            .await
        {
            Ok(result) => {
                if matches!(
                    result.next_control_flow,
                    brush_core::ExecutionControlFlow::ExitShell,
                ) {
                    return if result.is_success() { 0 } else { 1 };
                }
                if !result.is_success() {
                    // The prompt carries no exit-code segment; one quiet
                    // line keeps failures from passing silently.
                    let code = u8::from(&result.exit_code);
                    let _ = writeln!(stdout, "\x1b[31m— exit {code}\x1b[0m\r");
                }
            }
            Err(error) => {
                let _ = writeln!(stdout, "\x1b[31m{error}\x1b[0m\r");
            }
        }
    }
}

fn prompt(stdout: &mut impl Write, cwd: &std::path::Path) {
    // The last component is enough orientation; `pwd` has the rest. Cyan,
    // then reset — the vt100 layer renders it like any other program's.
    let name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.display().to_string());
    let _ = write!(stdout, "\x1b[36m{name}\x1b[0m$ ");
    let _ = stdout.flush();
}

/// One line, edited live: echo, backspace, arrow-key history. `None` on EOF
/// or Ctrl+D at an empty line.
fn read_line(
    bytes: &mut impl Iterator<Item = std::io::Result<u8>>,
    stdout: &mut impl Write,
    history: &[String],
) -> Option<String> {
    let mut line = String::new();
    // Where the arrows are in history; one past the end means "the line
    // being typed".
    let mut at = history.len();

    loop {
        let byte = bytes.next()?.ok()?;
        match byte {
            b'\r' | b'\n' => {
                let _ = write!(stdout, "\r\n");
                let _ = stdout.flush();
                return Some(line);
            }
            // Ctrl+C: abandon the line, keep the shell.
            0x03 => {
                let _ = write!(stdout, "^C\r\n");
                let _ = stdout.flush();
                line.clear();
                return Some(line);
            }
            // Ctrl+D on an empty line is EOF, as everywhere.
            0x04 if line.is_empty() => return None,
            0x7f | 0x08 => {
                if let Some(gone) = line.pop() {
                    // Erase the glyph — twice for a wide char's two cells.
                    let cells = if (gone as u32) > 0xFF { 2 } else { 1 };
                    for _ in 0..cells {
                        let _ = write!(stdout, "\x08 \x08");
                    }
                    let _ = stdout.flush();
                }
            }
            // ESC: an arrow or another CSI. Consume `[X`; up/down walk
            // history, everything else is dropped rather than echoed as
            // garbage.
            0x1b => {
                let Some(Ok(b'[')) = bytes.next() else { continue };
                let Some(Ok(code)) = bytes.next() else { continue };
                let replacement = match code {
                    b'A' if at > 0 => {
                        at -= 1;
                        Some(history[at].clone())
                    }
                    b'B' if at < history.len() => {
                        at += 1;
                        Some(if at == history.len() {
                            String::new()
                        } else {
                            history[at].clone()
                        })
                    }
                    _ => None,
                };
                if let Some(next) = replacement {
                    // Repaint: wipe the current line, write the new one.
                    for _ in 0..line.chars().count() {
                        let _ = write!(stdout, "\x08 \x08");
                    }
                    let _ = write!(stdout, "{next}");
                    let _ = stdout.flush();
                    line = next;
                }
            }
            // UTF-8 lead bytes accumulate to a whole scalar before echoing;
            // echo per byte would tear glyphs.
            _ if !byte.is_ascii() => push_utf8(&mut line, byte, bytes, stdout),
            _ => {
                line.push(byte as char);
                if !byte.is_ascii_control() {
                    let _ = stdout.write_all(&[byte]);
                    let _ = stdout.flush();
                }
            }
        }
    }
}

/// Accumulate the rest of a UTF-8 scalar whose lead byte just arrived, then
/// echo the whole glyph at once.
fn push_utf8(
    line: &mut String,
    lead: u8,
    bytes: &mut impl Iterator<Item = std::io::Result<u8>>,
    stdout: &mut impl Write,
) {
    let need = match lead {
        0xC0..=0xDF => 1,
        0xE0..=0xEF => 2,
        0xF0..=0xF7 => 3,
        _ => return,
    };
    let mut buffer = vec![lead];
    for _ in 0..need {
        match bytes.next() {
            Some(Ok(byte)) => buffer.push(byte),
            _ => return,
        }
    }
    if let Ok(text) = std::str::from_utf8(&buffer) {
        line.push_str(text);
        let _ = stdout.write_all(&buffer);
        let _ = stdout.flush();
    }
}
