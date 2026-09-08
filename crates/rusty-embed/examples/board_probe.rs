//! Does the *board* work — not just the pipeline?
//!
//! ```text
//! cargo run -p rusty-embed --example board_probe -- <project-dir> [seconds]
//! ```
//!
//! `sim_probe` proves a project builds, images and boots, and counts serial
//! lines. It says nothing about the sheet: whether the lamp somebody drew is
//! wired to the pin the firmware drives, whether it ever lights, whether the
//! button they placed reaches the pin the firmware reads. Those are the
//! questions the Simulate panel exists to answer, and until this they were
//! answered only by a person looking at it.
//!
//! So this one boots the same image with the pin channel attached, reads the
//! emulator's own account of every pin, runs the sheet's rules over it, and
//! reports what each part did — then presses every button on the sheet and
//! requires the pin it reaches to move. It exits non-zero when the board is
//! not proven, which is what makes it a test rather than a demonstration:
//!
//! * no pin reports at all — the emulator has no GPIO model, or the channel
//!   never opened;
//! * a lamp wired to a GPIO that never lights while the firmware runs;
//! * a button whose press does not move the pin it is wired to.
//!
//! It needs rusty's own QEMU. Espressif's build keeps no pin state, so there
//! is nothing to read and the probe says so rather than failing the board.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use rusty_embed::nets::{self, Behaviour, Row, behaviour_of};
use rusty_embed::{Sheet, parse_gpio_report};

/// What the run saw, part by part.
struct Seen {
    /// Every level the emulator reported, per GPIO, in order.
    levels: HashMap<u8, Vec<bool>>,
}

impl Seen {
    fn latest(&self) -> HashMap<u8, bool> {
        self.levels
            .iter()
            .filter_map(|(pin, seq)| seq.last().map(|level| (*pin, *level)))
            .collect()
    }

    /// The levels as they stood after each report, oldest first — what the
    /// rules are replayed over, so "did this lamp ever light" is a question
    /// about the whole run rather than about its last instant.
    fn frames(&self, order: &[(u8, bool)]) -> Vec<HashMap<u8, bool>> {
        let mut now: HashMap<u8, bool> = HashMap::new();
        let mut out = Vec::new();
        for (pin, level) in order {
            now.insert(*pin, *level);
            out.push(now.clone());
        }
        out
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(
        args.next()
            .unwrap_or_else(|| usage("a project directory is the first argument")),
    );
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);

    let project = match rusty_embed::project::detect(&root) {
        Ok(project) => project,
        Err(error) => usage(&format!(
            "{root:?} is not a project rusty can read: {error}"
        )),
    };
    let chip = project.chip.clone().unwrap_or_default();
    let mut plan = rusty_embed::simulate::plan(&project, false);
    if !plan.supported || !plan.missing.is_empty() {
        println!("this project cannot be simulated here:");
        if let Some(reason) = &plan.reason {
            println!("  {reason}");
        }
        for tool in &plan.missing {
            println!("  missing {} — {}", tool.name, tool.install);
        }
        std::process::exit(2);
    }
    for note in &plan.notes {
        println!("note: {note}");
    }

    let Some(sheet) = plan.board.clone() else {
        println!("this project has no .rusty/sim.toml, so there is no board to prove");
        std::process::exit(2);
    };
    let rows = rusty_embed::simulate::kit_rows_for(&root, &chip);
    describe(&sheet, &rows);

    // The emulator has to be rusty's: the stock build keeps no pin state,
    // and a board read off a stub would be a board reading zero for ever.
    let emulator = plan.emulator.clone();
    match &emulator {
        Some(emulator) if emulator.gpio_model => {}
        Some(emulator) => {
            println!(
                "the emulator at {} is Espressif's stock build, which keeps no pin state — \
                 install rusty's (the Simulate panel's Upgrade, or `qemu_download`) and run \
                 this again",
                emulator.path
            );
            std::process::exit(2);
        }
        None => {
            println!("no emulator in the plan");
            std::process::exit(2);
        }
    }

    let port = free_port().unwrap_or_else(|| usage("no free port for the pin channel"));
    let total = plan.steps.len();
    let mut steps = std::mem::take(&mut plan.steps);
    if let Some(boot) = steps.last_mut() {
        let extra = rusty_embed::simulate::pins_args(port);
        boot.display = format!("{} {}", boot.display, extra.join(" "));
        boot.args.extend(extra);
    }

    rusty_embed::simulate::prepare(&root).expect("prepare the image directory");

    let mut order: Vec<(u8, bool)> = Vec::new();
    let mut seen = Seen {
        levels: HashMap::new(),
    };
    let mut pressed_ok: Vec<String> = Vec::new();
    let mut pressed_failed: Vec<String> = Vec::new();

    for (index, step) in steps.into_iter().enumerate() {
        println!("\n$ {}", step.display);
        let session = rusty_embed::process::spawn(&step, Some(&root)).expect("spawn");
        let is_boot = index + 1 == total;
        if !is_boot {
            while let Some(line) = session.recv() {
                println!("{}", line.text);
            }
            if session.wait() != Some(0) {
                eprintln!("that step failed; the board cannot be proven");
                std::process::exit(1);
            }
            continue;
        }

        // The boot. The serial console goes to the terminal as usual; the
        // pins arrive on their own socket, which is what this reads.
        let stopper = session.stopper();
        std::thread::spawn(move || {
            while let Some(line) = session.recv() {
                println!("  {}", line.text);
            }
        });

        let Some(pins) = connect(port) else {
            eprintln!(
                "the pin channel never opened on {port} — the emulator did not create the \
                 device, or it is not rusty's build"
            );
            stopper.stop();
            std::process::exit(1);
        };
        let mut writer = pins.try_clone().expect("clone the pin socket");
        let (tx, rx) = std::sync::mpsc::channel::<(u8, bool)>();
        std::thread::spawn(move || {
            for line in BufReader::new(pins).lines().map_while(Result::ok) {
                if let Some(report) = parse_gpio_report(&line) {
                    for (pin, level) in report.pins {
                        let _ = tx.send((pin, level));
                    }
                }
            }
        });

        // Watch the firmware run itself.
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok((pin, level)) => {
                    order.push((pin, level));
                    seen.levels.entry(pin).or_default().push(level);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
        }

        // Then press every button the sheet has, and require the pin it
        // reaches to move. A button nothing is wired to is a warning the
        // rules already give; this is about the ones that are.
        for part in &sheet.parts {
            let Some(symbol) = sheet.symbol_of(&part.reference) else {
                continue;
            };
            if behaviour_of(symbol) != Behaviour::Switch {
                continue;
            }
            let Some((gpio, level)) = nets::button_drives(&sheet, &rows, &part.reference) else {
                println!(
                    "  {} reaches no GPIO on one side, or no rail on the other — nothing to press",
                    part.reference
                );
                continue;
            };
            // Released first, then pressed. The emulator models no pull
            // resistor, so a pin whose released level is high starts out
            // low — and pressing a pull-up button then asks it to go to the
            // level it is already at, which reports nothing and reads as a
            // button that does not work. Driving the released level first
            // makes the press an edge, which is what a press is.
            let before = seen.levels.get(&gpio).and_then(|s| s.last()).copied();
            let _ = writeln!(writer, "{gpio}={}", u8::from(!level));
            let _ = writer.flush();
            wait_for(
                &rx,
                &mut order,
                &mut seen,
                gpio,
                !level,
                Duration::from_secs(2),
            );

            let _ = writeln!(writer, "{gpio}={}", u8::from(level));
            let _ = writer.flush();
            let moved = wait_for(
                &rx,
                &mut order,
                &mut seen,
                gpio,
                level,
                Duration::from_secs(3),
            );

            // And let go, so the next press is a press rather than a hold.
            let _ = writeln!(writer, "{gpio}={}", u8::from(!level));
            let _ = writer.flush();
            wait_for(
                &rx,
                &mut order,
                &mut seen,
                gpio,
                !level,
                Duration::from_secs(3),
            );

            if moved {
                pressed_ok.push(format!(
                    "{} drives GPIO{gpio} {} (it read {:?} before)",
                    part.reference,
                    if level { "high" } else { "low" },
                    before
                ));
            } else {
                pressed_failed.push(format!(
                    "{} was pressed and GPIO{gpio} never went {}",
                    part.reference,
                    if level { "high" } else { "low" }
                ));
            }
        }

        stopper.stop();
    }

    report(&sheet, &rows, &seen, &order, &pressed_ok, &pressed_failed);
}

/// What the sheet says before anything runs — the half a person reads.
fn describe(sheet: &Sheet, rows: &[Row]) {
    println!("board: {} with {} parts", sheet.chip, sheet.parts.len());
    for part in &sheet.parts {
        let symbol = sheet.symbol_of(&part.reference);
        let what = symbol.map(behaviour_of);
        let on = symbol
            .map(|symbol| {
                symbol
                    .pins
                    .iter()
                    .filter_map(|pin| {
                        nets::gpio_of(sheet, rows, &part.reference, &pin.number)
                            .map(|gpio| format!("{}=GPIO{gpio}", pin.name))
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        println!(
            "  {:<6} {:<20} {:?} {on}",
            part.reference,
            part.symbol,
            what.unwrap_or(Behaviour::Other)
        );
    }
}

/// Everything the run proved, and the exit code that says whether it did.
fn report(
    sheet: &Sheet,
    rows: &[Row],
    seen: &Seen,
    order: &[(u8, bool)],
    pressed_ok: &[String],
    pressed_failed: &[String],
) {
    println!("\n─── what the emulator reported ───");
    if order.is_empty() {
        eprintln!(
            "no pin report at all in the whole run. The emulator created no GPIO device, or \
             the firmware drives nothing."
        );
        std::process::exit(1);
    }
    let mut pins: Vec<&u8> = seen.levels.keys().collect();
    pins.sort();
    for pin in pins {
        let seq = &seen.levels[pin];
        let edges = seq.windows(2).filter(|pair| pair[0] != pair[1]).count();
        println!("  GPIO{pin}: {} reports, {edges} changes", seq.len());
    }

    // Replay the rules over the run: a lamp counts as proven when it was
    // lit at some point *and* dark at another, which is what a firmware
    // driving it looks like. Lit throughout is a lamp wired to a rail.
    let frames = seen.frames(order);
    let mut lit_ever: HashMap<String, (bool, bool)> = HashMap::new();
    for levels in &frames {
        let reading = nets::evaluate(nets::Inputs {
            sheet,
            rows,
            gpio: levels,
            pressed: &HashSet::new(),
        });
        for part in &sheet.parts {
            let Some(symbol) = sheet.symbol_of(&part.reference) else {
                continue;
            };
            if !matches!(
                behaviour_of(symbol),
                Behaviour::Led | Behaviour::Rgb | Behaviour::Seven
            ) {
                continue;
            }
            let lit = reading.is_lit(&part.reference);
            let entry = lit_ever
                .entry(part.reference.clone())
                .or_insert((false, false));
            if lit {
                entry.0 = true;
            } else {
                entry.1 = true;
            }
        }
    }

    println!("\n─── what the sheet's parts did ───");
    let mut dark: Vec<String> = Vec::new();
    for (part, (was_lit, was_dark)) in &lit_ever {
        let on_a_gpio = sheet
            .symbol_of(part)
            .map(|symbol| {
                symbol
                    .pins
                    .iter()
                    .any(|pin| nets::gpio_of(sheet, rows, part, &pin.number).is_some())
            })
            .unwrap_or(false);
        let verdict = match (was_lit, was_dark) {
            (true, true) => "lit and went out — the firmware is driving it",
            (true, false) => "lit throughout",
            (false, _) if on_a_gpio => "never lit",
            (false, _) => "never lit, and reaches no GPIO",
        };
        println!("  {part}: {verdict}");
        if !was_lit && on_a_gpio {
            dark.push(part.clone());
        }
    }

    for line in pressed_ok {
        println!("  {line}");
    }

    let final_reading = nets::evaluate(nets::Inputs {
        sheet,
        rows,
        gpio: &seen.latest(),
        pressed: &HashSet::new(),
    });
    if !final_reading.warnings.is_empty() {
        println!("\n─── what the rules say about this sheet ───");
        for warning in &final_reading.warnings {
            println!("  {warning}");
        }
    }

    let mut failed = false;
    if !dark.is_empty() {
        eprintln!(
            "\nthese lamps are wired to a GPIO and never lit: {} — the firmware drives \
             another pin, or the wiring is the other way round",
            dark.join(", ")
        );
        failed = true;
    }
    for line in pressed_failed {
        eprintln!("{line}");
        failed = true;
    }
    if failed {
        std::process::exit(1);
    }
    println!("\nthe board is proven: every lamp on a pin lit, every button moved its pin");
}

/// Wait for one pin to reach a level, keeping everything that arrives on
/// the way — a press is proven by the pin moving, and the reports in
/// between are still the run's history.
fn wait_for(
    rx: &std::sync::mpsc::Receiver<(u8, bool)>,
    order: &mut Vec<(u8, bool)>,
    seen: &mut Seen,
    pin: u8,
    level: bool,
    within: Duration,
) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok((got, at)) => {
                order.push((got, at));
                seen.levels.entry(got).or_default().push(at);
                if got == pin && at == level {
                    return true;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return false,
        }
    }
    false
}

/// Connect to the pin channel once QEMU has opened it. Retried rather than
/// assumed: the socket appears after argument parsing and machine creation.
fn connect(port: u16) -> Option<TcpStream> {
    for _ in 0..100 {
        if let Ok(socket) = TcpStream::connect(("127.0.0.1", port)) {
            return Some(socket);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

fn usage(why: &str) -> ! {
    eprintln!("board_probe: {why}");
    eprintln!("usage: cargo run -p rusty-embed --example board_probe -- <project> [seconds]");
    std::process::exit(2);
}
