//! Does a host-driven pin actually reach `is_high()`?
//!
//! The output half of the GPIO model is checked by `blink-rust`, which drives
//! a pin and narrates it — two accounts of one pin that must agree. The input
//! half has no such witness, so this is it: firmware that configures GPIO4 as
//! an ordinary input and prints its level whenever it changes. Nothing here
//! knows about the simulator. It reads the pin the way any firmware reads a
//! button, which is the entire point — if this sees the level move, then a
//! real button in the board view can stop being a `B14=1` message injected
//! into the UART and become a pin.
//!
//! Pull::Down, not Up: the model has no notion of a pull resistor, so `in`
//! comes out of reset at zero. Asking for a pull-down means the firmware's
//! first reading agrees with the model's reset state, and the 0 -> 1 the test
//! looks for is a real transition rather than a disagreement about the start.
//!
//! GPIO4 because the C3 has already spent 12..17 on the SPI flash, 18/19 on
//! the native USB and 20/21 on the console, 2/8/9 are strapping pins, and
//! GPIO0 belongs to blinky's LED.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Pull};
use esp_hal::main;
use esp_println::println;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let pin = Input::new(
        peripherals.GPIO4,
        InputConfig::default().with_pull(Pull::Down),
    );

    println!("[probe] watching GPIO4");

    // On change only. A level printed every iteration would say nothing about
    // whether the pin moved — the log would look identical whether the write
    // arrived or not, which is the shape of a check that cannot fail.
    let mut last: Option<bool> = None;
    loop {
        let now = pin.is_high();
        if last != Some(now) {
            println!("[probe] gpio4={}", u8::from(now));
            last = Some(now);
        }
    }
}
