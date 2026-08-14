//! C calls Rust on an ESP32-C3: the C driver asks Rust for each frame's value.
//!
//! The migration direction — a Rust module inside firmware whose logic is
//! already C. On a part with no operating system this is one binary rather
//! than a `staticlib` linked by somebody else's build: `csrc/driver.c` owns
//! the frame, and calls back into a `#[unsafe(no_mangle)] extern "C"` Rust
//! function for the value. That is a real C→Rust call across a real FFI
//! boundary, and it is the shape that actually runs on the board.
//!
//! The other shape — Rust built as a `staticlib` and linked by an ESP-IDF C
//! project — is what `scaffold::c_interop(CCallsRust)` writes, and it needs
//! ESP-IDF's build system to produce anything flashable. That is a project
//! type this workbench deliberately does not own.
//!
//! Building it needs `riscv32-esp-elf-gcc` on PATH; the Toolchain panel
//! probes for it and the C scaffold refuses without it.
//!
//! Pins 0/1/3: free on the C3, and none of them a strapping pin.

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

/// What C is allowed to call.
///
/// A public API in a language with no namespaces, so the name carries a
/// prefix and `driver.h` declares exactly this. Panicking across an FFI
/// boundary is undefined behaviour, so anything that can fail returns a value
/// rather than unwinding — here nothing can.
#[unsafe(no_mangle)]
pub extern "C" fn rust_brightness(tick: u32) -> u8 {
    // A triangle wave, so the three thresholds in the C light in turn and
    // then go out again. A constant would look identical to a callback that
    // was never linked.
    let phase = tick % 512;
    if phase < 256 {
        phase as u8
    } else {
        (511 - phase) as u8
    }
}

unsafe extern "C" {
    fn driver_frame(tick: core::ffi::c_uint) -> core::ffi::c_uchar;
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let mut lamps = [
        Output::new(peripherals.GPIO0, Level::Low, OutputConfig::default()),
        Output::new(peripherals.GPIO1, Level::Low, OutputConfig::default()),
        Output::new(peripherals.GPIO3, Level::Low, OutputConfig::default()),
    ];

    println!("c-calls-rust: driver_frame() in C, calling back into rust_brightness()");

    let mut tick: u32 = 0;
    loop {
        // Safe because `driver_frame` reads only its argument and calls back
        // into a Rust function that cannot panic and holds no state.
        let mask = unsafe { driver_frame(tick) };
        for (index, lamp) in lamps.iter_mut().enumerate() {
            let on = mask & (1 << index) != 0;
            lamp.set_level(if on { Level::High } else { Level::Low });
        }

        let now = Instant::now().duration_since_epoch().as_micros();
        println!(
            "[rusty:gpio@{now}] 0={},1={},3={}",
            u8::from(mask & 1 != 0),
            u8::from(mask & 2 != 0),
            u8::from(mask & 4 != 0),
        );
        println!("[rusty:disp] rust={} mask={mask:#05b}", rust_brightness(tick));

        tick = tick.wrapping_add(8);
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(120) {}
    }
}
