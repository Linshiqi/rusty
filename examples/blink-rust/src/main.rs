//! Pure Rust on an ESP32-C3: one LED, and the serial protocol the board view
//! reads.
//!
//! The smallest thing that is still a whole loop — it builds, flashes, runs on
//! the real part, and lights the simulated board without a line of C. The two
//! sibling examples add a C compiler to exactly this shape, so what changes
//! between them is only the FFI.
//!
//! Pins: the C3 has GPIO0..21 and has already spent 12..17 on the SPI flash,
//! 18/19 on the native USB and 20/21 on the console. GPIO0 is free and is not
//! a strapping pin, which is why the LED is there.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::main;
use esp_hal::time::{Duration, Instant};
use esp_println::println;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let mut led = Output::new(peripherals.GPIO0, Level::Low, OutputConfig::default());

    println!("blink-rust: led on GPIO0, pure Rust");

    let mut on = false;
    loop {
        on = !on;
        led.set_level(if on { Level::High } else { Level::Low });

        // The one line the board view and the Waves panel read. The timestamp
        // is the systimer in microseconds, so a waveform is measured rather
        // than guessed at from arrival time.
        let now = Instant::now().duration_since_epoch().as_micros();
        println!("[rusty:gpio@{now}] 0={}", u8::from(on));
        println!("[rusty:disp] blink {}", if on { "on" } else { "off" });

        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(500) {}
    }
}
