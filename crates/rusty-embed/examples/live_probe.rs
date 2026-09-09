//! Does the circuit the sheet draws reach the firmware that is running?
//!
//! ```text
//! cargo run -p rusty-embed --example live_probe -- <project-dir> [seconds]
//! ```
//!
//! Stage 5 of `docs/kicad.md`, proven rather than reasoned about. `Live`'s
//! own tests drive it with lines somebody typed; this drives it with lines a
//! real firmware produced, on the chip's own instruction set, and reads back
//! what that firmware's converter actually took off the pin.
//!
//! **The assertion is the shape of the numbers, and that is the whole
//! claim.** The probe firmware drives one pin and reads another, with a
//! resistor and a capacitor between them on the sheet. A host that echoed
//! the pin level would make the reading step from nothing to full scale in
//! one conversion; a host that *solved the board* makes it climb through the
//! time constant somebody drew. So this requires:
//!
//! * readings at all — otherwise nothing is coupled;
//! * a run of them that climbs, several conversions long, which an echo
//!   cannot produce;
//! * and an arrival, because something that only ever ramped would be a host
//!   sending a ramp of its own rather than solving anything.
//!
//! It exits non-zero when the board is not proven, which is what makes it a
//! gate rather than a demonstration. It needs rusty's QEMU: the stock build
//! has no converter to read and says so.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use rusty_embed::live::{Live, Pace};
use rusty_embed::parse_adc_report;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().unwrap_or_else(|| usage("no project given")));
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(20);

    let detected = rusty_embed::project::detect(&root).unwrap_or_else(|e| {
        eprintln!("live_probe: {root:?} is not a project this can read: {e}");
        std::process::exit(2);
    });
    let mut plan = rusty_embed::simulate::plan(&detected, false);
    let chip = detected.chip.clone().unwrap_or_default();

    let Some(sheet) = plan.board.clone() else {
        println!("this project has no .rusty/sim.toml, so there is no circuit to couple");
        std::process::exit(2);
    };
    let rows = rusty_embed::simulate::kit_rows_for(&root, &chip);

    // The circuit, before anything is booted: if the sheet does not say
    // enough there is nothing to prove and the reason is the finding.
    let mut live = match Live::at_rest(sheet, rows, BTreeMap::new(), Pace::default()) {
        Ok(live) => live,
        Err(unstated) => {
            println!("the sheet does not say enough to solve: {unstated}");
            std::process::exit(1);
        }
    };

    // The *converter's* marker, not the pin model's. A build from before
    // `qemu-v3` has the pins and no SAR at all, and there is one of those in
    // the data directory of every machine that installed rusty early — the
    // ladder prefers it, a `gpio_model` check passes, and the run then hangs
    // inside the firmware's own read, with the failure naming a conversion
    // instead of an emulator. Measured rather than feared: the copy on this
    // machine carries `[rusty:gpio@` and neither `[rusty:adc@` nor
    // `[rusty:i2c@`.
    match &plan.emulator {
        Some(emulator)
            if rusty_embed::simulate::has_adc_model(std::path::Path::new(&emulator.path)) => {}
        Some(emulator) => {
            println!(
                "the emulator at {} models no SAR converter — it is Espressif's build, or one \
                 of rusty's from before qemu-v3. Install the current one (the Simulate panel's \
                 Upgrade) and run this again.",
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

    // What the firmware's own converter reported, in order.
    let mut read: Vec<u16> = Vec::new();

    for (index, step) in steps.into_iter().enumerate() {
        println!("\n$ {}", step.display);
        let session = rusty_embed::process::spawn(&step, Some(&root)).expect("spawn");
        if index + 1 != total {
            while let Some(line) = session.recv() {
                println!("{}", line.text);
            }
            if session.wait() != Some(0) {
                eprintln!("that step failed; the coupling cannot be proven");
                std::process::exit(1);
            }
            continue;
        }

        // The console carries the firmware's own account; the pin channel
        // carries the emulator's, and is where the coupling lives.
        let stopper = session.stopper();
        let (console, from_console) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            while let Some(line) = session.recv() {
                let _ = console.send(line.text);
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
        let (lines, from_pins) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(pins).lines().map_while(Result::ok) {
                if lines.send(line).is_err() {
                    return;
                }
            }
        });

        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline {
            // The emulator's lines drive the circuit; what comes back is
            // what its converter should read. This is the coupling, and it
            // is `Live::absorb` here exactly as it is in the app.
            // A line advances the circuit to the instant the guest names.
            // Silence advances it by the clock — because the emulator says
            // nothing while a capacitor charges, and a circuit frozen at
            // the last edge makes the reading step instead of climb. The
            // slice is shorter than the firmware's own read interval, so
            // the host is never what limits the shape that can be seen, and
            // the guest's own timestamps re-sync it whenever one arrives.
            const SLICE: Duration = Duration::from_micros(500);
            let moved = match from_pins.recv_timeout(SLICE) {
                Ok(line) => live.absorb(&line),
                Err(_) if live.settling() => live.advance_by(SLICE.as_secs_f64()),
                Err(_) => Ok(Vec::new()),
            };
            match moved {
                Ok(counts) => {
                    for (pin, count) in counts {
                        let _ = writer.write_all(format!("A{pin}={count}\n").as_bytes());
                        let _ = writer.flush();
                    }
                }
                Err(trouble) => {
                    eprintln!("the circuit stopped having an answer: {trouble}");
                    stopper.stop();
                    std::process::exit(1);
                }
            }
            // And the firmware's own side, which is the witness: what its
            // converter actually took off the pin.
            while let Ok(text) = from_console.try_recv() {
                println!("  {text}");
                if let Some(counts) = firmware_read(&text) {
                    read.push(counts);
                }
            }
            // The emulator says the same thing on its channel; either is a
            // reading, and both are the firmware's.
            if let Some(report) = parse_adc_report(&from_pins.try_recv().unwrap_or_default()) {
                read.push(report.counts);
            }
        }
        stopper.stop();
    }

    report(&read);
}

/// `[live] gpio3=1234` — the probe firmware's own line.
fn firmware_read(line: &str) -> Option<u16> {
    line.trim()
        .strip_prefix("[live] gpio3=")?
        .trim()
        .parse()
        .ok()
}

/// The three things that have to be true, each said in the terms a person
/// would check.
fn report(read: &[u16]) {
    println!("\n— what the firmware's converter read —");
    if read.is_empty() {
        println!(
            "nothing at all. The circuit is not reaching the converter: either the sheet \
             states no `fullscale`, or nothing joined the two halves."
        );
        std::process::exit(1);
    }
    println!("{} readings: {:?}", read.len(), head(read));

    let climb = longest_climb(read);
    if climb < 4 {
        println!(
            "the reading never climbed for more than {climb} conversions in a row. A host \
             echoing the pin level would look exactly like this — the point of the resistor \
             and the capacitor on the sheet is that it should not."
        );
        std::process::exit(1);
    }
    println!("the longest run of rising readings is {climb} — the circuit is being solved");

    let top = read.iter().copied().max().unwrap_or(0);
    let bottom = read.iter().copied().min().unwrap_or(0);
    if top < 100 || top.saturating_sub(bottom) < 100 {
        println!(
            "it climbed but never arrived ({bottom}..{top}). Something is sending a ramp \
             rather than solving a circuit."
        );
        std::process::exit(1);
    }
    println!("and it settles between {bottom} and {top}, so it arrives as well as climbs");
    println!("\nthe firmware is driving the circuit the sheet draws");
}

fn head(read: &[u16]) -> Vec<u16> {
    read.iter().copied().take(24).collect()
}

/// The longest run of strictly rising readings.
fn longest_climb(read: &[u16]) -> usize {
    let mut best = 0;
    let mut run = 1;
    for pair in read.windows(2) {
        if pair[1] > pair[0] {
            run += 1;
            best = best.max(run);
        } else {
            run = 1;
        }
    }
    best
}

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
    eprintln!("live_probe: {why}");
    eprintln!("usage: cargo run -p rusty-embed --example live_probe -- <project> [seconds]");
    std::process::exit(2);
}
