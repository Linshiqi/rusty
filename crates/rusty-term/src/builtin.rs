//! The built-in shell.
//!
//! Compiled into rusty itself and started as `rusty --builtin-shell` inside
//! the pty, so the terminal works the moment the app is installed — no
//! download, no PowerShell profile spending two seconds before the prompt,
//! and the same verbs on every OS because we define them.
//!
//! It is deliberately a *small* shell: a prompt, history, `cd` and friends,
//! and everything else spawned as a real process inheriting the pty. The
//! commands this workbench is about — cargo, espflash, probe-rs, git, gdb —
//! are all of that shape. Pipes and redirection are refused by name rather
//! than half-implemented: a user who needs them switches to the system shell
//! in Settings, which is one honest sentence instead of a shell that almost
//! works.
//!
//! Input arrives as VT sequences (ConPTY translates the keyboard), output is
//! ANSI the frontend's vt100 already renders. Line editing is ours: echo,
//! backspace, Ctrl+C, Ctrl+D/`exit`, and arrow-key history.

use std::io::{Read, Write};
use std::path::PathBuf;

/// Run the shell until EOF or `exit`. Never returns to the caller.
pub fn run() -> ! {
    let code = repl();
    std::process::exit(code);
}

fn repl() -> i32 {
    let mut stdout = std::io::stdout();
    let stdin = std::io::stdin();
    let mut bytes = stdin.lock().bytes();

    let mut cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut history: Vec<String> = Vec::new();

    let _ = writeln!(
        stdout,
        "rusty shell — plain commands, `cd`, `ls`, history on the arrow keys.\r"
    );

    loop {
        prompt(&mut stdout, &cwd);
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

        // Refuse shell syntax by name. Guessing at pipes would produce a
        // shell that almost works, which is worse than one that says no.
        if line.contains('|') || line.contains('>') || line.contains('<') {
            let _ = writeln!(
                stdout,
                "the built-in shell keeps to plain commands — pipes and redirection \
                 need a full shell (Settings > Terminal > System shell)\r"
            );
            continue;
        }

        let words = split_words(&line);
        let Some(head) = words.first().map(String::as_str) else {
            continue;
        };

        match head {
            "exit" => return 0,
            "clear" => {
                // Wipe and home, the sequence every terminal understands.
                let _ = write!(stdout, "\x1b[2J\x1b[H");
                let _ = stdout.flush();
            }
            "pwd" => {
                let _ = writeln!(stdout, "{}\r", cwd.display());
            }
            "cd" => match change_dir(&cwd, words.get(1).map(String::as_str)) {
                Ok(next) => cwd = next,
                Err(error) => {
                    let _ = writeln!(stdout, "cd: {error}\r");
                }
            },
            "ls" => list(&mut stdout, &cwd, words.get(1).map(String::as_str)),
            _ => run_command(&mut stdout, &cwd, &words),
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
    let _ = write!(stdout, "\x1b[36m{name}\x1b[0m> ");
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
            // UTF-8 continuation and lead bytes accumulate silently until a
            // full scalar lands; echo per byte would tear glyphs.
            _ => {
                line.push(byte as char);
                if byte.is_ascii() && !byte.is_ascii_control() {
                    let _ = stdout.write_all(&[byte]);
                    let _ = stdout.flush();
                } else if !byte.is_ascii() {
                    // Re-encode the accumulated bytes so multi-byte input
                    // still echoes; byte-as-char above mangled it, so fix
                    // the tail of the line first.
                    line.pop();
                    push_utf8(&mut line, byte, bytes, stdout);
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

/// Split on whitespace, honouring double quotes — enough for paths with
/// spaces, which is what quoting is for in a plain-command shell.
fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for ch in line.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn change_dir(cwd: &std::path::Path, target: Option<&str>) -> Result<PathBuf, String> {
    let home = || {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .map(PathBuf::from)
            .map_err(|_| "no home directory in the environment".to_string())
    };
    let next = match target {
        None | Some("~") => home()?,
        Some(rest) if rest.starts_with("~/") || rest.starts_with("~\\") => {
            home()?.join(&rest[2..])
        }
        Some(path) => {
            let candidate = PathBuf::from(path);
            if candidate.is_absolute() { candidate } else { cwd.join(candidate) }
        }
    };
    let next = next
        .canonicalize()
        .map_err(|e| format!("{}: {e}", next.display()))?;
    if !next.is_dir() {
        return Err(format!("{} is not a directory", next.display()));
    }
    Ok(next)
}

/// `ls`, the same on every OS: directories first and marked, then files —
/// consistency is the point of having our own.
fn list(stdout: &mut impl Write, cwd: &std::path::Path, target: Option<&str>) {
    let dir = match target {
        Some(path) => {
            let candidate = PathBuf::from(path);
            if candidate.is_absolute() { candidate } else { cwd.join(candidate) }
        }
        None => cwd.to_path_buf(),
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) => {
            let _ = writeln!(stdout, "ls: {}: {error}\r", dir.display());
            return;
        }
    };
    let mut names: Vec<(bool, String)> = entries
        .flatten()
        .map(|entry| {
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            (is_dir, entry.file_name().to_string_lossy().into_owned())
        })
        .collect();
    names.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    for (is_dir, name) in names {
        if is_dir {
            let _ = writeln!(stdout, "\x1b[36m{name}/\x1b[0m\r");
        } else {
            let _ = writeln!(stdout, "{name}\r");
        }
    }
}

/// Spawn a real process on the pty's own stdio and wait for it. gdb, cargo
/// and every other interactive tool take the terminal over directly, which
/// is the whole reason the shell runs inside the pty at all.
fn run_command(stdout: &mut impl Write, cwd: &std::path::Path, words: &[String]) {
    let status = std::process::Command::new(&words[0])
        .args(&words[1..])
        .current_dir(cwd)
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => {
            let _ = writeln!(stdout, "\x1b[31m— exited with {status}\x1b[0m\r");
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let _ = writeln!(stdout, "not found: {}\r", words[0]);
        }
        Err(error) => {
            let _ = writeln!(stdout, "{}: {error}\r", words[0]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_keeps_paths_with_spaces_whole() {
        assert_eq!(
            split_words(r#"cd "E:\My Projects\blinky""#),
            vec!["cd".to_string(), r"E:\My Projects\blinky".to_string()],
        );
        assert_eq!(split_words("cargo  build   --release"), vec![
            "cargo".to_string(),
            "build".to_string(),
            "--release".to_string(),
        ]);
    }
}
