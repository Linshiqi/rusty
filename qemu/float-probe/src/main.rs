//! Does an ESP32 application survive its first floating-point instruction?
//!
//! This is the one probe here whose subject is the *CPU* rather than a
//! peripheral, and it exists because rusty told every ESP32 user that
//! "Espressif's QEMU stops at the first floating-point instruction" — a
//! claim bisected once, written down, and then never reproduced with the
//! output let out. The message it named, `Fatal error: divide by zero`,
//! appears nowhere in QEMU's source, so whatever prints it is the guest.
//!
//! So: count in integers, then multiply two floats the optimiser cannot
//! fold, then count again. Whatever ends the run, ends it between two
//! lines that say exactly where it was.

#![no_std]
#![no_main]
// Xtensa inline assembly is still unstable, which is itself part of the
// finding: the one instruction an ESP32 application needs before it touches
// a float is not something it can write on stable Rust.
#![feature(asm_experimental_arch)]

use core::hint::black_box;

use esp_hal::clock::CpuClock;
use esp_hal::main;
use esp_println::println;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // Printed, not swallowed. A probe whose whole job is to find out what
    // kills the run cannot be the thing that hides it.
    println!("[float] panic: {info}");
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let _ = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // Enable coprocessor 0 — the FPU — unless this probe is asked to show
    // what happens without it. Nothing in `esp-hal`, `xtensa-lx` or
    // `xtensa-lx-rt` writes this register, and it resets to zero, so an
    // ESP32 application that touches a float without it takes a
    // coprocessor-disabled exception whose handler is itself full of
    // floating-point instructions: a double exception, for ever.
    if option_env!("FLOAT_PROBE_NO_CPENABLE").is_none() {
        unsafe {
            core::arch::asm!("wsr.cpenable {0}", "rsync", in(reg) 1u32, options(nostack));
        }
        println!("[float] cpenable set");
    }

    println!("[float] integers");
    let mut sum: u32 = 0;
    for n in 0..4u32 {
        sum = sum.wrapping_add(black_box(n));
        println!("[float] int {n} sum {sum}");
    }

    println!("[float] about to multiply");
    let product = black_box(1.25f32) * black_box(3.0f32);
    println!("[float] product {product}");

    let mut angle = black_box(0.5f32);
    for n in 0..4u32 {
        angle = black_box(angle) * black_box(1.5f32) + black_box(0.25f32);
        println!("[float] step {n} angle {angle}");
    }

    println!("[float] survived");
    loop {}
}
