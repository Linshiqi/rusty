//! Rust calls C on an ESP32-C3: three lamps swept by a C function.
//!
//! The direction people reach for first — a vendor driver, a legacy algorithm,
//! a checksum somebody validated a decade ago — kept to the smallest thing
//! that still shows the whole seam:
//!
//! - `csrc/pattern.c` owns the state and decides which lamp is lit.
//! - `build.rs` compiles it with `cc`, naming the cross compiler explicitly.
//! - `src/pattern.rs` declares it and wraps the `unsafe` in one safe call.
//! - this file never mentions C again.
//!
//! Building it needs `riscv32-esp-elf-gcc` on PATH. rusty's Toolchain panel
//! probes for exactly that binary and its C scaffold refuses without it, so
//! the failure arrives before any files are written rather than as a `cc`
//! error that names neither the chip nor the fix.
//!
//! Pins 0/1/3: free on the C3, and none of them a strapping pin.

#![no_std]
#![no_main]

mod pattern;

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
    let mut lamps = [
        Output::new(peripherals.GPIO0, Level::Low, OutputConfig::default()),
        Output::new(peripherals.GPIO1, Level::Low, OutputConfig::default()),
        Output::new(peripherals.GPIO3, Level::Low, OutputConfig::default()),
    ];

    println!("rust-calls-c: lamps on 0/1/3, swept by pattern_step() in csrc/pattern.c");

    loop {
        // The whole point of the example is this line: the decision is made
        // in C, and the pins are driven in Rust.
        let lit = pattern::step();
        for (index, lamp) in lamps.iter_mut().enumerate() {
            let on = lit & (1 << index) != 0;
            lamp.set_level(if on { Level::High } else { Level::Low });
        }

        let now = Instant::now().duration_since_epoch().as_micros();
        println!(
            "[rusty:gpio@{now}] 0={},1={},3={}",
            u8::from(lit & 1 != 0),
            u8::from(lit & 2 != 0),
            u8::from(lit & 4 != 0),
        );
        println!("[rusty:disp] C says {lit:#05b}");

        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(250) {}
    }
}
