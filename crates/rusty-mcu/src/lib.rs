//! Microcontrollers rusty runs itself, instruction by instruction.
//!
//! Espressif's parts are simulated in rusty's build of QEMU, which models
//! them; nobody's QEMU models a WCH part, and WCH's small parts are small
//! enough — a CH32V003 is sixteen registers, 16 KB of flash and a handful of
//! peripherals — that emulating them here costs less than a QEMU machine
//! would. [`ch32`] is the machine, and [`ch32::Part`] which part it is: the
//! CH32V003 or the CH32X035.
//!
//! Pure: no threads, no clock, no I/O. The firmware is an ELF's bytes in;
//! out come [`ch32::Event`]s stamped with the part's own time — a pin's
//! level, a PWM duty, a line printed, a thing the model cannot do. Pacing it
//! against the wall clock and putting the events on rusty's pin channel is
//! the host's job (rusty-embed's `simulate::mcu`), and everything here is
//! tested without one.

pub mod ch32;
pub mod cpu;
pub mod elf;

/// The machine's time base: 48 MHz, the fastest either part runs. HCLK is
/// always 24 or 48 MHz divided by an integer, so its period is a whole number
/// of these ticks — no clock these parts can have drifts against it.
pub const BASE_HZ: u64 = 48_000_000;
