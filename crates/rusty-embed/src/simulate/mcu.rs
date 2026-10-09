//! WCH's CH32V003 and CH32X035, run here rather than in QEMU.
//!
//! Nobody's QEMU has a machine for a WCH part, so rusty emulates these
//! itself (`rusty-mcu`) and runs one on a thread of its own process. To
//! everything that drives a simulation it is a QEMU with rusty's models in
//! it: a [`Session`] whose lines are the console, whose input is USART1's
//! receiver, and which stops when asked; a pin channel on a TCP port, which
//! the same [`super::connect`] connects to and which carries the same
//! `[rusty:gpio@<us>]` and `[rusty:pwm@<us>]` lines; and a monitor that
//! answers QMP's `stop` and `cont`, so the panel's pause works unchanged.
//!
//! The plan names it as a program like any other — [`PROGRAM`] — so the
//! command a run shows in the dock says what ran, and [`super::launch`] is
//! the one place that knows it is not a file.
//!
//! **Paced to the wall clock.** The machine runs in slices of at most 20 ms
//! of its own time, never ahead of real time, so a blink is a blink and a
//! 10 ms PWM step lands every 10 ms. A host too slow to keep up falls behind
//! rather than skipping: every event still carries the part's own time.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusty_mcu::ch32::{EventKind, Machine, Part};

use crate::error::{Error, Result};
use crate::model::{CommandPlan, LogLine, LogStream};
use crate::process::{Input, Session, Stopper};

/// The name the plan runs the emulator by.
pub const PROGRAM: &str = "rusty-mcu";

/// The most of its own time the machine runs before looking at the clock,
/// its inputs and whether it has been stopped.
const SLICE_US: u64 = 20_000;

/// Whether rusty-mcu has a model of `chip`. Which parts are *run* on it is
/// the catalogue's to say (`emulator = { kind = "rusty-mcu" }`); a test
/// holds the two in step.
#[cfg(test)]
fn emulates(chip: &str) -> bool {
    Part::for_chip(chip).is_some()
}

/// Whether a plan's step is this emulator.
pub fn is_program(program: &str) -> bool {
    program == PROGRAM
}

/// The boot step's arguments: which part and which image.
pub fn boot_args(chip: &str, elf: &str) -> Vec<String> {
    vec!["--chip".to_string(), chip.to_string(), elf.to_string()]
}

/// The pin channel, on `port`. The emulator listens and waits for the
/// connection before the firmware runs, as rusty's QEMU does (`wait=on`).
pub fn pins_args(port: u16) -> Vec<String> {
    vec!["--pins".to_string(), port.to_string()]
}

/// The monitor, on `port`: enough of QMP for `stop` and `cont`.
pub fn qmp_args(port: u16) -> Vec<String> {
    vec!["--qmp".to_string(), port.to_string()]
}

struct Options {
    part: &'static Part,
    elf: PathBuf,
    pins: Option<u16>,
    qmp: Option<u16>,
}

fn options(args: &[String]) -> std::result::Result<Options, String> {
    let mut elf = None;
    let mut pins = None;
    let mut qmp = None;
    let mut chip = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        let port = |text: String| {
            text.parse::<u16>()
                .map_err(|_| format!("`{text}` is not a port"))
        };
        match arg.as_str() {
            "--chip" => chip = Some(value("--chip")?),
            "--pins" => pins = Some(port(value("--pins")?)?),
            "--qmp" => qmp = Some(port(value("--qmp")?)?),
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => elf = Some(PathBuf::from(other)),
        }
    }
    let part = match chip.as_deref() {
        Some(chip) => {
            Part::for_chip(chip).ok_or_else(|| format!("{PROGRAM} does not model {chip}"))?
        }
        None => return Err(format!("{PROGRAM} needs --chip")),
    };
    Ok(Options {
        part,
        elf: elf.ok_or_else(|| format!("{PROGRAM} needs the firmware's ELF"))?,
        pins,
        qmp,
    })
}

/// Bytes typed into the session, on their way to USART1.
struct Typed(mpsc::Sender<Vec<u8>>);

impl Write for Typed {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .send(bytes.to_vec())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Start the emulator the plan's step describes. The image is read and
/// checked before anything starts, so an ELF built for another part fails
/// here, as a command that cannot start fails at its spawn.
pub fn launch(plan: &CommandPlan, dir: Option<&Path>) -> Result<Session> {
    let options = options(&plan.args).map_err(Error::refused)?;
    let elf = match dir {
        Some(dir) if options.elf.is_relative() => dir.join(&options.elf),
        _ => options.elf.clone(),
    };
    let image = std::fs::read(&elf).map_err(|error| {
        Error::refused(format!(
            "could not read the firmware at {}: {error}",
            elf.display()
        ))
    })?;
    let machine = Machine::new(options.part, &image).map_err(Error::refused)?;

    let (tx, lines) = mpsc::channel();
    let (typed, typing) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread = std::thread::Builder::new()
        .name("rusty-mcu".to_string())
        .spawn({
            let stop = Arc::clone(&stop);
            move || run(machine, options, &tx, &typing, &stop)
        })
        .map_err(|error| Error::refused(format!("could not start the emulator: {error}")))?;
    let thread: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::new(Mutex::new(Some(thread)));

    let stopper = Stopper::new({
        let stop = Arc::clone(&stop);
        move || stop.store(true, Ordering::Relaxed)
    });
    Ok(Session::from_parts(
        lines,
        Input::new(Some(Box::new(Typed(typed)))),
        stopper,
        move || {
            if let Some(thread) = thread.lock().ok().and_then(|mut slot| slot.take()) {
                let _ = thread.join();
            }
            Some(0)
        },
    ))
}

fn say(tx: &mpsc::Sender<LogLine>, text: String) {
    let _ = tx.send(LogLine {
        stream: LogStream::Stdout,
        text,
        level: None,
    });
}

/// The emulator's thread: wait for the pin channel, then run the machine in
/// step with the clock until stopped.
fn run(
    mut machine: Machine,
    options: Options,
    tx: &mpsc::Sender<LogLine>,
    typing: &mpsc::Receiver<Vec<u8>>,
    stop: &AtomicBool,
) {
    let paused = Arc::new(AtomicBool::new(false));
    if let Some(port) = options.qmp {
        monitor(port, Arc::clone(&paused), tx.clone());
    }
    let mut channel = None;
    let (heard_tx, heard) = mpsc::channel::<String>();
    if let Some(port) = options.pins {
        match wait_for_channel(port, stop) {
            Ok(Some(stream)) => {
                if let Ok(reader) = stream.try_clone() {
                    std::thread::spawn(move || {
                        for line in BufReader::new(reader).lines() {
                            let Ok(line) = line else { break };
                            if heard_tx.send(line).is_err() {
                                break;
                            }
                        }
                    });
                }
                channel = Some(stream);
            }
            // Stopped before anybody connected.
            Ok(None) => return,
            Err(error) => say(
                tx,
                format!("[rusty:mcu] the pin channel could not listen on {port}: {error}"),
            ),
        }
    }

    let mut unsupported = Unsupported::default();
    let start = Instant::now();
    let mut paused_for = Duration::ZERO;
    let mut paused_since: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        for bytes in typing.try_iter() {
            machine.receive(&bytes);
        }
        for line in heard.try_iter() {
            if let Some(note) = host_line(&mut machine, &line, &mut unsupported) {
                say(tx, format!("[rusty:mcu] {note}"));
            }
        }
        if paused.load(Ordering::Relaxed) {
            paused_since.get_or_insert_with(Instant::now);
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        if let Some(since) = paused_since.take() {
            paused_for += since.elapsed();
        }
        let wall = (start.elapsed().saturating_sub(paused_for)).as_micros() as u64;
        let now = machine.now_us();
        if wall > now {
            machine.run_until(wall.min(now + SLICE_US));
        }
        for event in machine.take_events() {
            let at = event.at_us;
            let report = match event.kind {
                EventKind::Console(line) => {
                    say(tx, line);
                    continue;
                }
                EventKind::Note(note) if note.starts_with("[rusty:") => {
                    say(tx, note);
                    continue;
                }
                EventKind::Note(note) => {
                    say(tx, format!("[rusty:mcu] {note}"));
                    continue;
                }
                EventKind::Level { pin, high } => {
                    format!("[rusty:gpio@{at}] {pin}={}", u8::from(high))
                }
                EventKind::Pwm { pin, duty, hz } => {
                    format!("[rusty:pwm@{at}] {pin}={duty:.4}@{hz:.1}")
                }
            };
            // On the pin channel when there is one, as rusty's QEMU reports
            // them; on the console otherwise, which the same reader reads.
            match channel.as_mut() {
                Some(stream) => {
                    if writeln!(stream, "{report}").is_err() {
                        channel = None;
                    }
                }
                None => say(tx, report),
            }
        }
        if machine.now_us() >= wall {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// Listen on `port` and wait for rusty's side to connect, as `wait=on`
/// does — `Ok(None)` if the run was stopped first.
fn wait_for_channel(port: u16, stop: &AtomicBool) -> std::io::Result<Option<TcpStream>> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    listener.set_nonblocking(true)?;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                stream.set_nodelay(true).ok();
                return Ok(Some(stream));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
}

/// Which of the host's messages the model has said it cannot act on.
#[derive(Default)]
struct Unsupported {
    analog: bool,
    bus: bool,
    switch: bool,
    wave: bool,
}

/// One line from the pin channel: a level for a pin, or something the model
/// has no peripheral for — said once, so a knob on the sheet that changes
/// nothing is explained rather than silent.
fn host_line(machine: &mut Machine, line: &str, said: &mut Unsupported) -> Option<String> {
    let line = line.trim();
    // Named by the part that is running, not by the first one modelled.
    let part = machine.part().name;
    let once = |flag: &mut bool, text: &str| {
        (!std::mem::replace(flag, true)).then(|| format!("the {part} model {text}"))
    };
    if let Some((pin, level)) = line.split_once('=')
        && let Ok(pin) = pin.parse::<u8>()
    {
        match level {
            "1" => machine.drive(pin, Some(true)),
            "0" => machine.drive(pin, Some(false)),
            _ => {}
        }
        return None;
    }
    if line.starts_with('A') {
        return once(
            &mut said.analog,
            "has no ADC: analog values on the sheet do not reach the firmware",
        );
    }
    if line.starts_with("i2c") || line.starts_with("spi") {
        return once(
            &mut said.bus,
            "has no I2C or SPI: devices on the sheet's buses do not answer",
        );
    }
    if line.starts_with("sw") {
        return once(
            &mut said.switch,
            "does not join two pins through a switch: a switch between two \
             GPIOs changes nothing",
        );
    }
    if line.starts_with('W') {
        return once(
            &mut said.wave,
            "plays no signals: a generator on the sheet stays silent",
        );
    }
    None
}

/// Enough of QMP for the panel's pause: the greeting, any command answered
/// with an empty return, and `stop` and `cont` doing what they say.
fn monitor(port: u16, paused: Arc<AtomicBool>, tx: mpsc::Sender<LogLine>) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => listener,
        Err(error) => {
            say(
                &tx,
                format!("[rusty:mcu] the monitor could not listen on {port}: {error}"),
            );
            return;
        }
    };
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let paused = Arc::clone(&paused);
            std::thread::spawn(move || {
                let Ok(mut writer) = stream.try_clone() else {
                    return;
                };
                let _ = writeln!(
                    writer,
                    "{{\"QMP\": {{\"version\": {{}}, \"capabilities\": []}}}}"
                );
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if line.contains("\"stop\"") {
                        paused.store(true, Ordering::Relaxed);
                    } else if line.contains("\"cont\"") {
                        paused.store(false, Ordering::Relaxed);
                    }
                    if writeln!(writer, "{{\"return\": {{}}}}").is_err() {
                        break;
                    }
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_read_back_or_refused() {
        let args: Vec<String> = ["--chip", "ch32v003j4m6", "--pins", "4000", "fw.elf"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let options = options(&args).unwrap_or_else(|refused| panic!("{refused}"));
        assert_eq!(options.elf, PathBuf::from("fw.elf"));
        assert_eq!(options.pins, Some(4000));
        assert_eq!(options.part.name, "CH32V003");
        let other: Vec<String> = ["--chip", "esp32c3", "fw.elf"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(
            super::options(&other)
                .err()
                .is_some_and(|refused| refused.contains("does not model esp32c3"))
        );
        assert!(emulates("ch32x035f8u6") && !emulates("ch32v203c8t6"));
    }

    /// Every part the catalogue sends to rusty-mcu has a model there, and
    /// every model is reached from the catalogue: a part named in one and
    /// not the other is a plan that boots nothing, or a model nobody runs.
    #[test]
    fn the_catalogue_and_the_emulator_agree_on_what_it_runs() {
        for chip in crate::chip::catalogue() {
            assert_eq!(
                chip.emulated_by(crate::model::EmulatorKind::RustyMcu),
                emulates(&chip.id),
                "{}",
                chip.id
            );
        }
    }

    /// The real firmware, end to end through the host's half: launched as a
    /// session, its pin channel connected over TCP the way `connect` does,
    /// and PWM reports arriving on it in the protocol `absorb` reads.
    #[test]
    fn a_launched_run_reports_pwm_on_its_pin_channel_and_prints_on_its_console() {
        let elf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../rusty-mcu/tests/fixtures/ch32v003-pwm/pwm.elf");
        let port = super::super::free_port().expect("a free port");
        let mut args = boot_args("ch32v003j4m6", &elf.display().to_string());
        args.extend(pins_args(port));
        let plan = CommandPlan::new(PROGRAM, args, "test");
        let session = launch(&plan, None).expect("launched");

        let stream = loop {
            if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
                break stream;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reports = BufReader::new(stream).lines();
        let first = reports.next().expect("a report").expect("readable");
        let pwm = crate::protocol::parse_pwm_report(&first)
            .unwrap_or_else(|| panic!("a PWM report: {first}"));
        assert_eq!(pwm.pins[0].0, 20, "PC4");

        let console = session.recv().expect("a line");
        assert!(console.text.starts_with("pwm max duty"), "{}", console.text);
        session.stopper().stop();
        while session.recv().is_some() {}
        assert_eq!(session.wait(), Some(0));
    }

    /// The pin channel numbers a WCH pin as many to a port as the part's
    /// GPIO registers are wide; the emulator that sends the number and the
    /// name the window gives it must agree on the width, or PB12 is drawn
    /// as some other pin.
    #[test]
    fn the_window_and_the_emulator_count_a_port_alike() {
        for chip in ["ch32v003j4m6", "ch32v003f4p6", "ch32x035f8u6"] {
            let part = Part::for_chip(chip).expect(chip);
            let width = crate::chip::by_id(chip).expect(chip).port_width;
            assert_eq!(width, Some(part.width), "{chip}");
            for pin in part.pins() {
                assert_eq!(
                    crate::nets::pin_label(width, pin),
                    part.pin_name(pin),
                    "{chip}"
                );
            }
        }
    }

    /// The CH32X035 through the same door: `--chip` picks the part, and its
    /// pins arrive numbered 24 to a port — PB12 is 36.
    #[test]
    fn a_ch32x035_run_reports_pb12_by_its_own_numbering() {
        let elf = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../rusty-mcu/tests/fixtures/ch32x035-pwm/pwm.elf");
        let port = super::super::free_port().expect("a free port");
        let mut args = boot_args("ch32x035f8u6", &elf.display().to_string());
        args.extend(pins_args(port));
        let plan = CommandPlan::new(PROGRAM, args, "test");
        let session = launch(&plan, None).expect("launched");
        let stream = loop {
            if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
                break stream;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let first = BufReader::new(stream)
            .lines()
            .next()
            .expect("a report")
            .expect("readable");
        let pwm = crate::protocol::parse_pwm_report(&first)
            .unwrap_or_else(|| panic!("a PWM report: {first}"));
        assert_eq!(pwm.pins[0].0, 36, "PB12");
        let width = crate::chip::by_id("ch32x035f8u6").unwrap().port_width;
        assert_eq!(crate::nets::pin_label(width, 36), "PB12");
        let console = session.recv().expect("a line");
        assert_eq!(console.text, "pwm max duty 8000");
        session.stopper().stop();
        while session.recv().is_some() {}
        assert_eq!(session.wait(), Some(0));
    }
}
