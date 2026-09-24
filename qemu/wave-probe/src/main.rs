//! Does a signal the host rendered play against the firmware's own clock?
//!
//! The host puts a table of samples on a pin and another on a sensor's
//! register block, and the emulator plays both against its virtual clock —
//! the clock the systimer and every timer here keep. This reads both once a
//! millisecond, timed by that systimer, and prints each reading with the
//! time it was taken:
//!
//! ```text
//! [wave] <µs> <counts> <sample>
//! ```
//!
//! The gate asks three things of that, each a way a model could be wrong on
//! its own. That every conversion the emulator reports is the table's value
//! at that instant — the playing is right. That the firmware read what was
//! reported — the register path is right. And that the tone it read has the
//! table's frequency *by this clock* — a table played against the host's
//! clock instead would pass the first two and fail this one, which is the
//! failure the whole mechanism exists to remove.
//!
//! The sensor's table carries its own sample number in every one of the
//! block's seven words, so a burst read that took some words from one
//! sample and some from the next shows it on its face: the words disagree.
//! Those are counted here, because a torn sample is exactly what latching
//! the block when a transaction addresses it is for, and one that slipped
//! through would otherwise read as noise on a gyro.
//!
//! GPIO3 and the bus on GPIO5 and GPIO6, for the reasons the analog and I2C
//! probes give: nothing else here wants them.

#![no_std]
#![no_main]

use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::i2c::master::{Config, I2c, SoftwareTimeout};
use esp_hal::main;
use esp_hal::time::{Duration, Instant};
use esp_println::println;

/// The sensor and the first register of its burst: an MPU-6050's
/// accelerometer, temperature and gyro, fourteen bytes read as one.
const SENSOR: u8 = 0x68;
const BURST: u8 = 0x3b;
/// Two seconds of readings, one a millisecond: a hundred periods of the
/// gate's 50 Hz tone to time, and two thousand conversions to compare.
const READINGS: u32 = 2000;
const EVERY: Duration = Duration::from_micros(1000);
/// How many times to ask whether a conversion finished before calling it
/// stuck — the analog probe's rule, for the analog probe's reason: a probe
/// that spun on a missing converter would report the hole as silence.
const PATIENCE: u32 = 200_000;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[wave] panic: {info}");
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

/// The block's seven words, big-endian as the sensor sends them.
fn words(block: &[u8; 14]) -> [u16; 7] {
    let mut out = [0u16; 7];
    for (word, pair) in out.iter_mut().zip(block.chunks_exact(2)) {
        *word = u16::from_be_bytes([pair[0], pair[1]]);
    }
    out
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let mut config = AdcConfig::new();
    let mut pin = config.enable_pin(peripherals.GPIO3, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, config);

    let bus = Config::default()
        .with_software_timeout(SoftwareTimeout::Transaction(Duration::from_millis(200)));
    let mut i2c = match I2c::new(peripherals.I2C0, bus) {
        Ok(i2c) => i2c.with_sda(peripherals.GPIO5).with_scl(peripherals.GPIO6),
        Err(error) => {
            println!("[wave] no bus: {error:?}");
            loop {}
        }
    };

    println!("[wave] reading GPIO3 and the sensor every millisecond");

    let mut torn = 0u32;
    let mut next = Instant::now();
    for _ in 0..READINGS {
        next += EVERY;
        while Instant::now() < next {}
        let at = Instant::now().duration_since_epoch().as_micros();

        let mut counts = None;
        for _ in 0..PATIENCE {
            if let Ok(read) = adc.read_oneshot(&mut pin) {
                counts = Some(read);
                break;
            }
        }
        let Some(counts) = counts else {
            println!("[wave] the conversion never finished");
            loop {}
        };

        let mut block = [0u8; 14];
        let sample = match i2c.write_read(SENSOR, &[BURST], &mut block) {
            Ok(()) => {
                let words = words(&block);
                if words.iter().any(|word| *word != words[0]) {
                    torn += 1;
                    println!("[wave] torn {words:?}");
                }
                i32::from(words[0])
            }
            Err(_) => -1,
        };
        println!("[wave] {at} {counts} {sample}");
    }
    println!("[wave] done torn={torn}");
    loop {}
}
