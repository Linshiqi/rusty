//! Run a CH32V003 firmware in rusty's emulator of the part, with no window,
//! and print what it did: every pin it drove, every PWM duty, every line it
//! printed, every note about something the model does not have — stamped
//! with the part's own time — and how long the host took to run it.
//!
//! ```text
//! cargo run --release -p rusty-mcu --example mcu_probe -- <elf> [milliseconds]
//! ```
//!
//! The check that a firmware boots here at all, before the window is in the
//! way, and the measure of how far ahead of real time the emulator runs on
//! this machine — which is what lets the window pace it to the clock.

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: mcu_probe <elf> [milliseconds]");
        std::process::exit(2);
    };
    let ms: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1_000);
    let image = match std::fs::read(&path) {
        Ok(image) => image,
        Err(error) => {
            eprintln!("could not read {path}: {error}");
            std::process::exit(2);
        }
    };
    let mut machine = match rusty_mcu::ch32v003::Machine::new(&image) {
        Ok(machine) => machine,
        Err(refused) => {
            eprintln!("{refused}");
            std::process::exit(2);
        }
    };
    let started = std::time::Instant::now();
    machine.run_until(ms * 1_000);
    let took = started.elapsed();
    let events = machine.take_events();
    for event in &events {
        println!("{:>10} µs  {:?}", event.at_us, event.kind);
    }
    eprintln!(
        "{} events in {ms} ms of the part's time, run in {took:.1?}",
        events.len()
    );
}
