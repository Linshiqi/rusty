//! A board that *reads*: a knob on the ADC and a sensor on I2C.
//!
//! Every other example here drives pins or prints numbers it made up.
//! This one asks the hardware, with the ordinary drivers and no knowledge
//! of the simulator: `adc.read_oneshot()` and `i2c.write_read()`, exactly
//! as they would be written for the part on your desk.
//!
//! What makes that possible in the simulator is that the emulator models
//! the converter and the bus, and the sheet in `.rusty/sim.toml` says what
//! is on them — a source on GPIO3 you can drag, and a sensor at 0x68 whose
//! registers the sheet spells out. Move the slider and the number here
//! moves; change the registers and this reads the new ones. Nothing in this
//! file knows any of that happened.
//!
//! It prints on rusty's telemetry channel, so the Plot panel draws the knob
//! and the three axes without anything further being configured.

#![no_std]
#![no_main]

use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::i2c::master::{Config as I2cConfig, I2c, SoftwareTimeout};
use esp_hal::main;
use esp_hal::time::Duration;
use esp_println::println;

/// The sensor this board has on it, and the two registers a driver reads:
/// who it is, and where it is pointing.
const SENSOR: u8 = 0x68;
const WHO_AM_I: u8 = 0x75;
const AXES: u8 = 0x3b;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {info}");
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

/// Two bytes as the signed number an accelerometer reports, big-endian —
/// which is how nearly every I2C sensor lays its axes out.
fn axis(high: u8, low: u8) -> i16 {
    i16::from_be_bytes([high, low])
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let mut adc_config = AdcConfig::new();
    let mut knob = adc_config.enable_pin(peripherals.GPIO3, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, adc_config);

    // A software timeout so a bus with nothing on it says so instead of
    // waiting for ever. On the desk that is a sensor you forgot to plug in;
    // in the simulator it is a sheet with no device declared.
    let i2c_config = I2cConfig::default()
        .with_software_timeout(SoftwareTimeout::Transaction(Duration::from_millis(100)));
    let mut i2c = match I2c::new(peripherals.I2C0, i2c_config) {
        Ok(i2c) => i2c.with_sda(peripherals.GPIO5).with_scl(peripherals.GPIO6),
        Err(error) => {
            println!("the I2C driver would not start: {error:?}");
            loop {}
        }
    };

    let mut who = [0u8; 1];
    match i2c.write_read(SENSOR, &[WHO_AM_I], &mut who) {
        Ok(()) => println!("[rusty:disp] sensor {:#04x}", who[0]),
        Err(error) => println!("[rusty:disp] no sensor: {error:?}"),
    }

    let delay = Delay::new();
    loop {
        // The knob, in the converter's own counts. No volts printed: this
        // firmware does not know the divider either.
        let counts: u16 = loop {
            if let Ok(value) = adc.read_oneshot(&mut knob) {
                break value;
            }
        };

        let mut axes = [0u8; 6];
        if i2c.write_read(SENSOR, &[AXES], &mut axes).is_ok() {
            println!(
                "[rusty:tel] knob={},ax={},ay={},az={}",
                counts,
                axis(axes[0], axes[1]),
                axis(axes[2], axes[3]),
                axis(axes[4], axes[5]),
            );
        } else {
            println!("[rusty:tel] knob={counts}");
        }

        delay.delay_millis(100);
    }
}
