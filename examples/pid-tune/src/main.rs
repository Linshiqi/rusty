//! A control loop you tune while it runs.
//!
//! The half of embedded work a debugger cannot help with. Stopping a control
//! loop to inspect a variable stops the thing being controlled — a flight
//! controller pauses and the craft falls — so what the loop knows leaves as
//! text on the serial line it is already printing to, and the gains come back
//! the same way. No reflash between one gain and the next.
//!
//! Three lines of protocol, and rusty's Plot panel is the other end of all
//! three:
//!
//! - `[rusty:tel@<µs>] name=value,…` — a sample. Timestamped with the
//!   systimer, so the plot's x-axis is the firmware's own clock rather than
//!   whenever the host happened to read the port.
//! - `[rusty:param] name=value min..max` — "this is a tunable, here is what I
//!   currently hold, and here is the range I accept". The panel draws no
//!   slider without a range: a range the *tool* invented is how somebody
//!   sends a gain of 500 to a motor loop.
//! - `S<name>=<value>` inbound — set it. The firmware answers with the
//!   `[rusty:param]` line above carrying what it actually took, so a value it
//!   clamped reads as clamped instead of as the number that was typed.
//!
//! The plant is simulated in the firmware rather than wired to a motor: this
//! is an example, and a first-order lag is enough to make a P gain that is too
//! high visibly ring. Everything above the plant — the loop, the protocol, the
//! clamping — is what real firmware would do.
//!
//! Pins: GPIO0 carries a "driving hard" indicator so the board view has
//! something to show. The C3 has already spent 12..17 on the SPI flash, 18/19
//! on native USB and 20/21 on the console.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::main;
use esp_hal::time::{Duration, Instant};
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_println::println;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

/// One control period. 50 Hz: fast enough to look like a loop, slow enough
/// that the serial line is nowhere near saturated — a real 1 kHz loop would
/// print every Nth sample rather than every one, and that decimation is the
/// firmware's decision to make, not the tool's.
const PERIOD: Duration = Duration::from_millis(20);
const DT: f32 = 0.02;

/// Everything the loop will accept a change to, with the bounds it accepts.
///
/// One array, so announcing and setting cannot drift apart — a tunable that
/// the firmware accepts but never announces is one no panel can show, and one
/// it announces but ignores is worse.
struct Tunable {
    name: &'static str,
    value: f32,
    min: f32,
    max: f32,
}

impl Tunable {
    /// Take a new value, clamped. Returns what was actually taken, which is
    /// what gets announced — the caller never assumes the write landed whole.
    fn take(&mut self, value: f32) -> f32 {
        self.value = value.clamp(self.min, self.max);
        self.value
    }

    fn announce(&self) {
        println!(
            "[rusty:param] {}={} {}..{}",
            self.name, self.value, self.min, self.max
        );
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let mut hard = Output::new(peripherals.GPIO0, Level::Low, OutputConfig::default());

    // UART0 is the console QEMU wires to stdin and the board's USB bridge
    // carries. Reading it is how a set arrives; esp-println keeps writing to
    // the same peripheral, which is exactly the point — one line, both ways.
    let uart = Uart::new(peripherals.UART0, UartConfig::default()).expect("uart0");
    let (mut rx, _tx) = uart.split();

    println!("pid-tune: a first-order plant under PID, tunable while it runs");

    let mut tunables = [
        Tunable { name: "kp", value: 2.0, min: 0.0, max: 20.0 },
        Tunable { name: "ki", value: 0.5, min: 0.0, max: 10.0 },
        Tunable { name: "kd", value: 0.05, min: 0.0, max: 5.0 },
        Tunable { name: "setpoint", value: 50.0, min: 0.0, max: 100.0 },
    ];
    // Announced once at boot: the panel is built from what the firmware says
    // it has, not from a config file that could describe a different binary.
    for tunable in &tunables {
        tunable.announce();
    }

    let mut measured = 0.0f32;
    let mut integral = 0.0f32;
    let mut previous = 0.0f32;
    let mut line = [0u8; 64];
    let mut filled = 0usize;

    loop {
        let started = Instant::now();

        // Drain whatever arrived since the last period. Buffered rather than
        // blocking: a control loop that waits for a byte that never comes has
        // stopped controlling anything.
        let mut byte = [0u8; 1];
        while rx.read_ready() {
            match rx.read_buffered(&mut byte) {
                Ok(1) => {
                    if byte[0] == b'\n' || byte[0] == b'\r' {
                        if let Some(name) = apply(&line[..filled], &mut tunables) {
                            // Answer with what was taken. A clamp is
                            // information; swallowing it would leave the panel
                            // showing a value the loop is not using.
                            if let Some(t) = tunables.iter().find(|t| t.name == name) {
                                t.announce();
                            }
                        }
                        filled = 0;
                    } else if filled < line.len() {
                        line[filled] = byte[0];
                        filled += 1;
                    } else {
                        // Overlong garbage: drop the line rather than
                        // truncating it into a different command.
                        filled = 0;
                    }
                }
                _ => break,
            }
        }

        let (kp, ki, kd, setpoint) = (
            tunables[0].value,
            tunables[1].value,
            tunables[2].value,
            tunables[3].value,
        );

        let error = setpoint - measured;
        integral = (integral + error * DT).clamp(-100.0, 100.0);
        let derivative = (error - previous) / DT;
        previous = error;
        let output = (kp * error + ki * integral + kd * derivative).clamp(-100.0, 100.0);

        // A first-order lag: the plant follows its input with inertia. Enough
        // for too much P to ring visibly and for too little I to leave a
        // standing error, which is what a tuning panel is read for.
        measured += (output - measured) * DT * 4.0;

        hard.set_level(if output.abs() > 50.0 {
            Level::High
        } else {
            Level::Low
        });

        let now = Instant::now().duration_since_epoch().as_micros();
        println!(
            "[rusty:tel@{now}] setpoint={setpoint:.2},measured={measured:.2},error={error:.2},output={output:.2}"
        );
        println!("[rusty:gpio@{now}] 0={}", u8::from(output.abs() > 50.0));

        while started.elapsed() < PERIOD {}
    }
}

/// Apply one inbound line, if it is a set. Returns the name that changed.
///
/// Deliberately strict: an unknown name changes nothing and says nothing. A
/// firmware that silently accepted a misspelled gain would leave somebody
/// turning a knob that is not connected to anything.
fn apply(line: &[u8], tunables: &mut [Tunable]) -> Option<&'static str> {
    let text = core::str::from_utf8(line).ok()?;
    let body = text.trim().strip_prefix('S')?;
    let (name, value) = body.split_once('=')?;
    let value: f32 = value.trim().parse().ok()?;
    let tunable = tunables.iter_mut().find(|t| t.name == name.trim())?;
    tunable.take(value);
    Some(tunable.name)
}
