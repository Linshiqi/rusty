//! Does an analog value on a pin reach `adc.read_oneshot()`?
//!
//! Every other probe here is about a *digital* pin. This one is about the
//! half of the board that a knob, a divider or a light sensor sits on, and
//! about a failure mode worse than a wrong answer: with nothing mapped at
//! the converter's registers, the driver polls a done bit that can never be
//! set, and the firmware hangs inside the user's own `read` call with
//! nothing anywhere to say why.
//!
//! So the poll here is *bounded*. A conversion that does not finish is the
//! interesting result, and a probe that spun on it for ever would report
//! that result as silence — which is the one thing a witness must never do.
//!
//! GPIO3 because it is ADC1's channel 3 on this part and nothing else here
//! wants it: 0 is blinky's LED, 2 is a strapping pin, 4 belongs to the
//! interrupt probe, and 12..21 are the flash, the native USB and the
//! console.
//!
//! The counts are printed raw. The emulator says what it converted on the
//! pin channel and this says what the firmware read; two accounts of one
//! reading are only comparable if neither has scaled it first — which is
//! also why the host sends counts rather than volts, and why the model does
//! not know anybody's divider.

#![no_std]
#![no_main]

use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::main;
use esp_println::println;

/// How many times to ask whether the conversion has finished before calling
/// it stuck. Far more than the microseconds the silicon needs, far less than
/// the test's patience.
const PATIENCE: u32 = 200_000;

/// Talks, for the same reason the interrupt probe's does: a silent panic
/// handler reports a driver that died exactly as it reports one that never
/// ran.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[adc] panic: {info}");
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // 11 dB, the widest range, because the host speaks in the converter's
    // own counts and the attenuation only decides what voltage those counts
    // would have meant on a real part. The model stores it and ignores it,
    // and says so.
    let mut config = AdcConfig::new();
    let mut pin = config.enable_pin(peripherals.GPIO3, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, config);

    println!("[adc] listening on GPIO3");

    let mut last: Option<u16> = None;
    loop {
        let mut reading = None;
        for _ in 0..PATIENCE {
            if let Ok(counts) = adc.read_oneshot(&mut pin) {
                reading = Some(counts);
                break;
            }
        }
        match reading {
            None => {
                println!("[adc] the conversion never finished");
                loop {}
            }
            // Only on change: a firmware printing every conversion would
            // fill the channel with one repeated line, and the question this
            // answers is whether the value *follows* the pin.
            Some(counts) if last != Some(counts) => {
                println!("[adc] gpio3={counts}");
                last = Some(counts);
            }
            Some(_) => {}
        }
    }
}
