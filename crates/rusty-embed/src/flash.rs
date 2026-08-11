//! Getting the binary onto the board, and reading what it says back.
//!
//! Split deliberately in two:
//!
//! - [`plan`] decides *what to run* and is a pure function. It can be tested
//!   without a board, and its output is shown to the user before anything
//!   happens. Embedded developers live in a terminal; a tool that hides the
//!   command behind a button becomes something to work around.
//! - [`spawn`] runs it and streams the output.
//!
//! rusty does not reimplement flashing. espflash and probe-rs are the tools the
//! ecosystem actually maintains, and wrapping them means a user's existing
//! knowledge — and their existing bug reports — still apply.

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
    chip,
    error::{Error, Result},
    model::{CommandPlan, FlashAction, LogLevel, LogLine, LogStream, Transport},
    toolchain::no_console_window,
};

/// Everything needed to decide on a command.
#[derive(Debug, Clone)]
pub struct FlashRequest {
    pub chip_id: String,
    pub transport: Transport,
    pub action: FlashAction,
    /// The linked ELF. espflash and probe-rs both take the ELF rather than a
    /// raw binary — they derive the flash layout from it.
    pub firmware: std::path::PathBuf,
    /// Decode defmt frames instead of showing raw bytes. Only meaningful when
    /// the firmware was built with defmt; the ELF carries the string table.
    pub defmt: bool,
    /// Serial only. Leave unset for the tool's default.
    pub baud: Option<u32>,
}

/// Decide what to run. Pure — no process is started.
pub fn plan(request: &FlashRequest) -> Result<CommandPlan> {
    let firmware = request.firmware.display().to_string();
    let chip = chip::by_id(&request.chip_id);

    let (program, args, rationale) = match &request.transport {
        Transport::Serial { port } => {
            // Only Espressif parts have a serial bootloader in ROM. Offering
            // this path for anything else sends the user somewhere that cannot
            // work, with an error that will not explain why.
            if let Some(chip) = &chip
                && !chip.flashers.contains(&crate::model::Flasher::Espflash)
            {
                return Err(Error::NoSerialBootloader {
                    chip: chip.name.clone(),
                });
            }

            let mut args = vec!["--chip".into(), request.chip_id.clone(), "--port".into(), port.clone()];
            if let Some(baud) = request.baud {
                args.push("--baud".into());
                args.push(baud.to_string());
            }
            if request.defmt {
                args.push("--log-format".into());
                args.push("defmt".into());
            }

            match request.action {
                FlashAction::Monitor => {
                    let mut monitor = vec!["monitor".to_string()];
                    monitor.extend(args);
                    // The ELF is what turns defmt indices back into strings,
                    // and what maps a panic address to a line.
                    monitor.push("--elf".into());
                    monitor.push(firmware.clone());
                    (
                        "espflash",
                        monitor,
                        "Attaching over serial without rewriting flash.",
                    )
                }
                action => {
                    let mut flash = vec!["flash".to_string()];
                    flash.extend(args);
                    if action == FlashAction::FlashAndMonitor {
                        flash.push("--monitor".into());
                    }
                    flash.push(firmware.clone());
                    (
                        "espflash",
                        flash,
                        "Flashing over the ROM serial bootloader — no probe needed.",
                    )
                }
            }
        }

        Transport::Probe { identifier } => {
            let target = chip
                .as_ref()
                .and_then(|c| c.probe_rs_target.clone())
                .ok_or_else(|| Error::UnknownProbeTarget {
                    chip: request.chip_id.clone(),
                })?;

            // `probe-rs run` flashes, attaches, and decodes RTT in one step,
            // which is the whole inner loop. `download` is the flash-only form.
            let subcommand = match request.action {
                FlashAction::Flash => "download",
                _ => "run",
            };
            let mut args = vec![subcommand.to_string(), "--chip".into(), target];
            if let Some(identifier) = identifier {
                args.push("--probe".into());
                args.push(identifier.clone());
            }
            args.push(firmware.clone());

            (
                "probe-rs",
                args,
                "Flashing through the debug probe; defmt arrives over RTT.",
            )
        }
    };

    let display = std::iter::once(program.to_string())
        .chain(args.iter().map(|a| quote_if_needed(a)))
        .collect::<Vec<_>>()
        .join(" ");

    Ok(CommandPlan {
        program: program.to_string(),
        args,
        display,
        rationale: rationale.to_string(),
    })
}

/// Quote an argument that would not survive being pasted into a shell.
fn quote_if_needed(arg: &str) -> String {
    if arg.contains(' ') {
        format!("\"{arg}\"")
    } else {
        arg.to_string()
    }
}

/// A running flash or monitor session.
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
    use std::path::PathBuf;

    fn request(chip_id: &str, transport: Transport, action: FlashAction) -> FlashRequest {
        FlashRequest {
            chip_id: chip_id.to_string(),
            transport,
            action,
            firmware: PathBuf::from("target/blinky"),
            defmt: false,
            baud: None,
        }
    }

    fn serial() -> Transport {
        Transport::Serial {
            port: "COM3".to_string(),
        }
    }

    #[test]
    fn serial_flash_uses_espflash_with_chip_and_port() {
        let plan = plan(&request("esp32c3", serial(), FlashAction::FlashAndMonitor)).unwrap();

        assert_eq!(plan.program, "espflash");
        assert_eq!(plan.args[0], "flash");
        assert!(plan.args.contains(&"--chip".to_string()));
        assert!(plan.args.contains(&"esp32c3".to_string()));
        assert!(plan.args.contains(&"COM3".to_string()));
        assert!(plan.args.contains(&"--monitor".to_string()));
        // The command shown to the user must be the command that runs.
        assert!(plan.display.starts_with("espflash flash "));
        assert!(plan.display.contains("COM3"));
    }

    #[test]
    fn flash_without_monitor_does_not_attach() {
        let plan = plan(&request("esp32c3", serial(), FlashAction::Flash)).unwrap();
        assert!(!plan.args.contains(&"--monitor".to_string()));
    }

    #[test]
    fn monitor_passes_the_elf_so_defmt_and_backtraces_resolve() {
        let mut req = request("esp32c3", serial(), FlashAction::Monitor);
        req.defmt = true;
        let plan = plan(&req).unwrap();

        assert_eq!(plan.args[0], "monitor");
        // Without --elf the decoder has no string table and shows indices.
        assert!(plan.args.contains(&"--elf".to_string()));
        assert!(plan.args.contains(&"defmt".to_string()));
    }

    #[test]
    fn baud_is_only_sent_when_asked_for() {
        let plain = plan(&request("esp32c3", serial(), FlashAction::Flash)).unwrap();
        assert!(!plain.args.contains(&"--baud".to_string()));

        let mut fast = request("esp32c3", serial(), FlashAction::Flash);
        fast.baud = Some(921_600);
        let fast = plan(&fast).unwrap();
        assert!(fast.args.contains(&"921600".to_string()));
    }

    /// STM32 has no serial bootloader. Producing an espflash command for it
    /// would fail with an error about the chip, sending the user to debug
    /// entirely the wrong thing.
    #[test]
    fn a_part_without_a_serial_bootloader_refuses_the_serial_path() {
        let err = plan(&request("stm32f411", serial(), FlashAction::Flash))
            .unwrap_err()
            .to_string();
        assert!(err.contains("STM32F411"), "{err}");
        assert!(err.contains("probe"), "the message has to point at the fix: {err}");
    }

    #[test]
    fn probe_flash_uses_run_so_rtt_is_attached() {
        let plan = plan(&request(
            "esp32c3",
            Transport::Probe { identifier: None },
            FlashAction::FlashAndMonitor,
        ))
        .unwrap();

        assert_eq!(plan.program, "probe-rs");
        assert_eq!(plan.args[0], "run");
        assert!(plan.args.contains(&"esp32c3".to_string()));
    }

    #[test]
    fn probe_flash_only_uses_download() {
        let plan = plan(&request(
            "esp32c3",
            Transport::Probe { identifier: None },
            FlashAction::Flash,
        ))
        .unwrap();
        assert_eq!(plan.args[0], "download");
    }

    /// probe-rs target names for STM32 depend on package and flash size, which
    /// the die alone does not determine. Guessing one produces a plausible name
    /// with the wrong memory map — a far worse outcome than refusing.
    #[test]
    fn an_unknown_probe_target_refuses_rather_than_guessing() {
        let err = plan(&request(
            "stm32f411",
            Transport::Probe { identifier: None },
            FlashAction::Flash,
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("probe-rs chip list"), "{err}");
    }

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
