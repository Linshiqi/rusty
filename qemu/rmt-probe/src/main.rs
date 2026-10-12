//! Do the codes a strip's driver writes reach the pin as the bits they are?
//!
//! An addressable LED strip is the one part on a hobby board that is not
//! wired to anything the other probes here cover: no level, no duty, no
//! bus — a stream of pulses on one pin, clocked out by RMT. With nothing
//! mapped at its registers the driver's buffer goes into a hole and the
//! transmission never ends, so `wait()` never returns: a strip that stays
//! dark and a firmware apparently stuck in the user's own `write`.
//!
//! Three LEDs, which is the case that matters: a channel holds 48 codes and
//! three LEDs are 72, so the driver sends half, waits for the threshold,
//! writes the next half over the first and goes round — the refill loop the
//! model has to keep step with. One LED would fit in the RAM and prove
//! nothing about it.
//!
//! The codes are built here rather than with `esp-hal-smartled` so the
//! probe depends on nothing the emulator's own crates do not, and they are
//! the same codes that crate writes: the WS2812 timings, most significant
//! bit first, green then red then blue.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::Level;
use esp_hal::main;
use esp_hal::rmt::{PulseCode, Rmt, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[rmt] panicked: {info}");
    loop {}
}

/// A WS2812 bit at 80 MHz with no divider: 0.8 µs high then 0.45 µs low for
/// a one, and the other way round for a zero.
fn code(bit: bool) -> PulseCode {
    if bit {
        PulseCode::new(Level::High, 64, Level::Low, 36)
    } else {
        PulseCode::new(Level::High, 36, Level::Low, 64)
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let delay = Delay::new();

    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80)).expect("the RMT clock");
    let mut channel = rmt
        .channel0
        .configure_tx(
            &TxChannelConfig::default()
                .with_clk_divider(1)
                .with_idle_output_level(Level::Low)
                .with_idle_output(true),
        )
        .expect("channel 0")
        .with_pin(peripherals.GPIO8);

    // Green, red, blue — one bright channel each, so the bytes are
    // unmistakable in the report and in the order the wire carries them.
    let colours: [[u8; 3]; 3] = [[0x10, 0x00, 0x00], [0x00, 0x20, 0x00], [0x00, 0x00, 0x30]];
    let mut codes = [PulseCode::end_marker(); 3 * 24 + 1];
    let mut at = 0;
    for colour in colours {
        for byte in colour {
            for bit in (0..8).rev() {
                codes[at] = code((byte >> bit) & 1 == 1);
                at += 1;
            }
        }
    }
    codes[at] = PulseCode::end_marker();

    println!("[rmt] sending {} codes", at + 1);
    match channel.transmit(&codes).expect("a transmission").wait() {
        Ok(_) => println!("[rmt] sent 100000002000000030"),
        Err((error, _)) => println!("[rmt] failed: {error:?}"),
    }

    loop {
        delay.delay_millis(500);
        println!("[rmt] holding");
    }
}
