//! A filter at work on a signal the sheet plays: the worked end of the
//! Signals tab (`docs/signals.md`).
//!
//! The sheet puts a signal generator on GPIO3 — a slow tone with mains hum
//! on it — and the emulator plays it against this firmware's own clock. The
//! firmware reads the converter 250 times a second by its systimer, runs the
//! reading through the filter the Design view exported (`low_pass.rs`, not a
//! line of it written by hand), and prints both on every sample:
//!
//! ```text
//! [rusty:tel@<µs>] raw=<counts>,y=<filtered, to the nearest count>
//! ```
//!
//! **The rate is what a line on the console can carry, not what the
//! converter can.** Printing a sample costs the emulated core about a
//! millisecond and a half, so at a thousand a second the loop fell behind
//! its own deadlines and ran at 643 — and a filter whose coefficients are
//! right at one rate and no other then does something else, which the gate
//! below caught. A design that filters faster than it reports would decimate
//! the telemetry; this one keeps the two equal, and prints whole counts,
//! because formatting a float with no FPU is much of what a line costs.
//!
//! which is all the Signals tab needs: the Time view lays the two over what
//! was played, the Spectrum shows the hum gone from `y`, and the Response
//! view sweeps a tone across the band with `raw` or the converter going in
//! and `y` coming out, beside the design's own curve.
//!
//! `cargo run -p rusty-embed --example filter_probe -- examples/filter-lab`
//! is the same, as a gate: it plays the sheet, changes the signal once while
//! the firmware runs, and requires `y` over `raw` at each tone to be the
//! design's gain and phase.

#![no_std]
#![no_main]

mod low_pass;

use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::main;
use esp_hal::time::{Duration, Instant};
use esp_println::println;

use low_pass::LowPass;

/// The rate the filter was designed for, 250 a second: its coefficients are
/// right at this rate and no other.
const EVERY: Duration = Duration::from_micros(4000);

/// How many times to ask whether a conversion finished before calling it
/// stuck — a firmware that spun on a converter that is not there would
/// report the hole as silence.
const PATIENCE: u32 = 200_000;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[filter] panic: {info}");
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let mut config = AdcConfig::new();
    let mut pin = config.enable_pin(peripherals.GPIO3, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, config);
    let mut filter = LowPass::new();

    println!("[filter] ready: GPIO3 250 times a second through a 10 Hz low-pass");

    let mut next = Instant::now();
    loop {
        next += EVERY;
        while Instant::now() < next {}
        let at = Instant::now().duration_since_epoch().as_micros();

        let mut counts = None;
        for _ in 0..PATIENCE {
            if let Ok(read) = adc.read_oneshot(&mut pin) {
                counts = Some(read);
                break;
            }
        }
        let Some(counts) = counts else {
            println!("[filter] the conversion never finished");
            loop {}
        };

        // To the nearest count, which is what `raw` is in too.
        let y = filter.step(f32::from(counts));
        let y = if y < 0.0 { y - 0.5 } else { y + 0.5 } as i32;
        println!("[rusty:tel@{at}] raw={counts},y={y}");
    }
}
