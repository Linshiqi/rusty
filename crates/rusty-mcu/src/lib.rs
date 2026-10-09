//! Microcontrollers rusty runs itself, instruction by instruction.
//!
//! Espressif's parts are simulated in rusty's build of QEMU, which models
//! them; nobody's QEMU models a WCH part, and the CH32V003 is small enough —
//! sixteen registers, 16 KB of flash, a handful of peripherals — that
//! emulating it here costs less than a QEMU machine would. [`ch32v003`] is
//! that part.
//!
//! Pure: no threads, no clock, no I/O. The firmware is an ELF's bytes in;
//! out come [`ch32v003::Event`]s stamped with the part's own time — a pin's
//! level, a PWM duty, a line printed, a thing the model cannot do. Pacing it
//! against the wall clock and putting the events on rusty's pin channel is
//! the host's job (rusty-embed's `simulate::mcu`), and everything here is
//! tested without one.

pub mod ch32v003;
pub mod cpu;
pub mod elf;

/// The machine's time base: 48 MHz, the fastest the CH32V003 runs. HCLK is
/// always 24 or 48 MHz divided by an integer, so its period is a whole number
/// of these ticks — no clock this part can have drifts against it.
pub const BASE_HZ: u64 = 48_000_000;
