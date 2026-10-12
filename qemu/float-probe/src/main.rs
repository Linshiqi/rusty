//! Does an ESP32 application survive its first floating-point instruction?
//!
//! This is the one probe here whose subject is the *CPU* rather than a
//! peripheral, and it has been wrong about that CPU twice. rusty told every
//! ESP32 user for months that "Espressif's QEMU stops at the first
//! floating-point instruction", which it does not; then, measured properly,
//! that the *application* had to switch its FPU on, because `CPENABLE` read
//! zero at the float and nothing in esp-hal writes it. That was measured and
//! was still wrong: nothing in the ESP32's ROM or its bootloader writes it
//! either, esp-hal's interrupt entry saves the floating-point registers
//! unconditionally, and esp-hal's interrupts work on real boards — so on
//! the silicon the FPU is on from reset, and it was the emulator that left
//! it off. rusty's emulator now comes out of reset the way the board does.
//!
//! So the default build does **nothing** to `CPENABLE`, exactly as an
//! ordinary application does, and has to run to its last line. Built with
//! `FLOAT_PROBE_DISABLE=1` it switches coprocessor 0 off itself before the
//! float, and then the run has to go quiet *and the emulator has to say
//! why*, because the symptom on its own is silence.

#![no_std]
#![no_main]
// Xtensa inline assembly is still unstable; only the disabling build uses
// it, and it is the one instruction this probe needs to reach the case the
// emulator's diagnostic exists for.
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

    // Switch coprocessor 0 — the FPU — off, when asked to show what that
    // costs. xtensa-lx-rt does the same inside every interrupt when esp-hal's
    // `float-save-restore` is off, and a float taken then faults; with the
    // floating-point save on, the handler faults too and the CPU spins.
    if option_env!("FLOAT_PROBE_DISABLE").is_some() {
        unsafe {
            core::arch::asm!("wsr.cpenable {0}", "rsync", in(reg) 0u32, options(nostack));
        }
        println!("[float] cpenable cleared");
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
