//! Does the *circuit* reach the firmware — not just a number the host chose?
//!
//! Every other probe here asks whether one peripheral works. This one asks
//! the question stage 5 of `docs/kicad.md` exists for, and it is a different
//! kind of question: not "did the value the host sent arrive" but "did the
//! value the host sent come from solving the board somebody drew".
//!
//! So it does both halves at once. It **drives GPIO2** on a slow square
//! wave, and it **reads GPIO3's converter** as fast as it can. Between those
//! two pins the sheet draws a resistor and a capacitor, so what GPIO3 reads
//! is not GPIO2's level — it is that level *through an RC*, and the whole
//! claim is visible in the shape of the numbers:
//!
//! * a host that echoed the pin level would step from nothing to full scale
//!   in one reading, and back;
//! * a host that solved the circuit makes it **climb**, over about as many
//!   milliseconds as the sheet's time constant.
//!
//! The counts are printed raw and only on change, for the same two reasons
//! `adc-probe` prints them that way: two accounts of one reading are
//! comparable only if neither has scaled it, and a firmware printing every
//! conversion fills the channel with one repeated line.
//!
//! GPIO2 and GPIO3 because nothing else here wants them together: 0 is
//! blinky's LED, 4 is the interrupt probe's, 5 and 6 are the bus probe's,
//! and 12..21 are the flash, the native USB and the console.

#![no_std]
#![no_main]

use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::main;
use esp_println::println;

/// How many times to ask whether the conversion has finished before calling
/// it stuck. `adc-probe`'s reason, unchanged: with nothing at the
/// converter's registers the driver polls a done bit that can never be set,
/// and an unbounded poll would report that hang as silence.
const PATIENCE: u32 = 200_000;

/// How long each half of the square wave lasts, in milliseconds.
///
/// Long against the sheet's time constant on purpose. The reading has to
/// have room to climb *and* to arrive, because both halves are the
/// assertion: something that only ever climbed would be a host sending a
/// ramp of its own, and something that only ever arrived would be a host
/// echoing the pin.
const HALF: u32 = 40;

/// Talks, for the same reason every other probe's does: a silent panic
/// handler reports a driver that died exactly as it reports one that never
/// ran.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[live] panic: {info}");
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let mut drive = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

    // 11 dB, the widest range. What the counts would have meant in volts on
    // a real part is the attenuation's business; what they mean here is the
    // sheet's `fullscale`, which is the one thing it has to say.
    let mut config = AdcConfig::new();
    let mut sense = config.enable_pin(peripherals.GPIO3, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, config);

    let delay = Delay::new();
    println!("[live] driving GPIO2, reading GPIO3");

    let mut last: Option<u16> = None;
    let mut high = false;
    loop {
        high = !high;
        drive.set_level(if high { Level::High } else { Level::Low });
        println!("[live] drove gpio2={}", u8::from(high));

        // Read across the whole half-period, so the climb is sampled rather
        // than only its ends.
        for _ in 0..HALF {
            let mut reading = None;
            for _ in 0..PATIENCE {
                if let Ok(counts) = adc.read_oneshot(&mut sense) {
                    reading = Some(counts);
                    break;
                }
            }
            match reading {
                None => {
                    println!("[live] the conversion never finished");
                    loop {}
                }
                Some(counts) if last != Some(counts) => {
                    println!("[live] gpio3={counts}");
                    last = Some(counts);
                }
                Some(_) => {}
            }
            delay.delay_millis(1);
        }
    }
}
