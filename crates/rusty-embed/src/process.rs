//! Running an external tool and streaming what it says.
//!
//! Lived in `flash.rs` until four callers wanted it — flashing, monitoring,
//! generating a project, and the terminal. None of those is about flashing, and
//! a module named for one of its callers is a module the next caller has to
//! justify importing.
//!
//! rusty drives other people's tools rather than reimplementing them: espflash,
//! probe-rs, esp-generate and cargo are what the ecosystem maintains, so a
//! user's existing knowledge and existing bug reports still apply.

use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread,
};

use crate::{
    error::{Error, Result},
    model::{CommandPlan, LogLevel, LogLine, LogStream},
    toolchain::no_console_window,
};

/// A running child process.
///
/// stdout and stderr are read on their own threads and merged into one ordered
/// channel: espflash writes progress to one and errors to the other, and a UI
/// that showed them in separate panes would split the story of a failed flash
/// down the middle.
pub struct Session {
    child: Arc<Mutex<Child>>,
    lines: Receiver<LogLine>,
}

impl Session {
    /// The next line, or `None` once the process has exited and both streams
    /// are drained.
    pub fn recv(&self) -> Option<LogLine> {
        self.lines.recv().ok()
    }

    /// A handle that can end this session from another thread.
    ///
    /// Separate from the session itself because the reader loop blocks on
    /// `recv`: whoever wants to stop a monitor cannot be the thread that is
    /// sitting inside it.
    pub fn stopper(&self) -> Stopper {
        Stopper {
            child: Arc::clone(&self.child),
        }
    }

    /// Wait for the process and return its exit code.
    ///
    /// Call after `recv` returns `None`, which is when both streams are drained.
    pub fn wait(&self) -> Option<i32> {
        let mut child = self.child.lock().expect("session lock");
        child.wait().ok().and_then(|status| status.code())
    }

}

/// Ends a session from outside its reader loop.
#[derive(Clone)]
pub struct Stopper {
    child: Arc<Mutex<Child>>,
}

impl Stopper {
    /// A monitor session runs until the user leaves, so this is the normal way
    /// one ends, not an error path. Stopping an already-exited process is
    /// success — the caller wanted it stopped, and it is.
    pub fn stop(&self) {
        let mut child = self.child.lock().expect("session lock");
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Start the planned command.
pub fn spawn(plan: &CommandPlan, working_dir: Option<&Path>) -> Result<Session> {
    let mut command = Command::new(&plan.program);
    command
        .args(&plan.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    if let Some(dir) = working_dir {
        command.current_dir(dir);
    }
    no_console_window(&mut command);

    let mut child = command.spawn().map_err(|source| Error::Spawn {
        tool: plan.program.clone(),
        source,
    })?;

    let (tx, lines) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, LogStream::Stdout, tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, LogStream::Stderr, tx);
    }

    Ok(Session {
        child: Arc::new(Mutex::new(child)),
        lines,
    })
}

fn pump<R: std::io::Read + Send + 'static>(
    reader: R,
    stream: LogStream,
    tx: mpsc::Sender<LogLine>,
) {
    thread::spawn(move || {
        // Not `lines()`: espflash draws a progress bar with carriage returns
        // and no newline, so a line-based reader would show nothing at all
        // until the flash finished.
        let mut reader = BufReader::new(reader);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match read_record(&mut reader, &mut buffer) {
                Ok(0) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&buffer).trim_end().to_string();
                    if text.is_empty() {
                        continue;
                    }
                    let level = parse_level(&text);
                    if tx.send(LogLine { stream, text, level }).is_err() {
                        // Receiver gone: the user closed the panel.
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
}

/// Read up to the next `\n` or `\r`, whichever comes first.
fn read_record<R: BufRead>(reader: &mut R, buffer: &mut Vec<u8>) -> std::io::Result<usize> {
    let mut total = 0;
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(total);
        }
        match available.iter().position(|b| *b == b'\n' || *b == b'\r') {
            Some(at) => {
                buffer.extend_from_slice(&available[..at]);
                reader.consume(at + 1);
                return Ok(total + at + 1);
            }
            None => {
                buffer.extend_from_slice(available);
                let len = available.len();
                reader.consume(len);
                total += len;
            }
        }
    }
}

/// Severity from the shapes these tools actually emit.
///
/// Three formats reach this: defmt through probe-rs (`INFO  message`), the
/// `log` crate through esp-println (`INFO - message`), and ESP-IDF's own
/// (`I (1234) tag: message`). Missing the third would leave every std project's
/// logs unlevelled.
fn parse_level(line: &str) -> Option<LogLevel> {
    let trimmed = line.trim_start();

    // ESP-IDF: a single letter, then a timestamp in parentheses.
    if let Some(rest) = trimmed.get(1..2)
        && rest.starts_with(" ")
        && trimmed.get(2..3) == Some("(")
    {
        return match trimmed.as_bytes().first() {
            Some(b'V') => Some(LogLevel::Trace),
            Some(b'D') => Some(LogLevel::Debug),
            Some(b'I') => Some(LogLevel::Info),
            Some(b'W') => Some(LogLevel::Warn),
            Some(b'E') => Some(LogLevel::Error),
            _ => None,
        };
    }

    let first = trimmed.split_whitespace().next()?;
    match first.trim_end_matches([':', '-']).to_ascii_uppercase().as_str() {
        "TRACE" => Some(LogLevel::Trace),
        "DEBUG" => Some(LogLevel::Debug),
        "INFO" => Some(LogLevel::Info),
        "WARN" | "WARNING" => Some(LogLevel::Warn),
        "ERROR" => Some(LogLevel::Error),
        _ => None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn log_levels_are_parsed_from_all_three_formats_in_use() {
        // defmt via probe-rs
        assert_eq!(parse_level("INFO  boot complete"), Some(LogLevel::Info));
        // log crate via esp-println
        assert_eq!(parse_level("WARN - low battery"), Some(LogLevel::Warn));
        assert_eq!(parse_level("ERROR: i2c nack"), Some(LogLevel::Error));
        // ESP-IDF's own format, which none of the above patterns match
        assert_eq!(parse_level("I (1234) wifi: connected"), Some(LogLevel::Info));
        assert_eq!(parse_level("E (99) heap: no mem"), Some(LogLevel::Error));
        assert_eq!(parse_level("V (7) trace: tick"), Some(LogLevel::Trace));

        assert_eq!(parse_level("Hello, world!"), None);
        assert_eq!(parse_level(""), None);
    }

    #[test]
    fn progress_bars_are_split_on_carriage_returns() {
        // espflash redraws its progress bar with \r and no newline. A
        // line-based reader would show nothing until the flash finished.
        let input = b"Writing 10%\rWriting 55%\rWriting 100%\nDone\n";
        let mut reader = BufReader::new(&input[..]);
        let mut records = Vec::new();
        loop {
            let mut buffer = Vec::new();
            match read_record(&mut reader, &mut buffer) {
                Ok(0) => break,
                Ok(_) => records.push(String::from_utf8(buffer).unwrap()),
                Err(_) => break,
            }
        }
        assert_eq!(
            records,
            vec!["Writing 10%", "Writing 55%", "Writing 100%", "Done"]
        );
    }
}
