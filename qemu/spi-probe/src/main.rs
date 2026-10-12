//! Does an SPI transfer reach a device, and does what the host declared come
//! back?
//!
//! Espressif's QEMU models `SPI1`, the flash controller the machine boots
//! through, and nothing at `SPI2` — the one a project puts a display or a
//! sensor on. So a driver's first transfer sets the start bit and polls it
//! for ever: the third hang of this shape, inside the user's own call, after
//! the converter's and the bus's.
//!
//! Two assertions, and they are different questions:
//!
//! - A **write** completes and its bytes reach the host. That is the whole
//!   of what a display needs, and it is the case where nothing comes back.
//! - A **transfer** returns the bytes the host declared for that chip
//!   select. SPI has no addressing, so what a model can honestly answer is
//!   a buffer read from its start — and a driver that sends a command byte
//!   and reads the reply out of the same transfer gets it where full duplex
//!   puts it.
//!
//! Printed on change, like every other probe here, so the log says what the
//! wire did rather than how often the loop ran.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::main;
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config, Spi};
use esp_hal::time::Rate;
use esp_println::println;

/// What a driver sends to ask an imaginary sensor who it is: a register
/// number with the read bit set, then a byte of nothing to clock the answer
/// out.
const ASK: [u8; 2] = [0xf5, 0x00];
/// And what a display gets sent, which nobody answers.
const TELL: [u8; 3] = [0xae, 0xa5, 0x01];

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[spi] panic: {info}");
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

fn hex(bytes: &[u8], out: &mut [u8; 32]) -> usize {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut at = 0;
    for byte in bytes {
        if at + 2 > out.len() {
            break;
        }
        out[at] = DIGITS[(byte >> 4) as usize];
        out[at + 1] = DIGITS[(byte & 0xf) as usize];
        at += 2;
    }
    at
}

fn say(label: &str, bytes: &[u8]) {
    let mut buffer = [0u8; 32];
    let at = hex(bytes, &mut buffer);
    println!(
        "[spi] {label}{}",
        core::str::from_utf8(&buffer[..at]).unwrap_or("?")
    );
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // The peripheral's own chip select, so the model sees which line is
    // asserted rather than a GPIO the driver waggles itself. GPIO2, 6, 7 and
    // 10 because the flash has 12..17, the native USB 18/19 and the console
    // 20/21; the model does not route through the GPIO matrix, so the choice
    // is about not colliding.
    let config = Config::default()
        .with_frequency(Rate::from_khz(100))
        .with_mode(Mode::_0);
    let mut spi = match Spi::new(peripherals.SPI2, config) {
        Ok(spi) => spi
            .with_sck(peripherals.GPIO6)
            .with_mosi(peripherals.GPIO7)
            .with_miso(peripherals.GPIO2)
            .with_cs(peripherals.GPIO10),
        Err(error) => {
            println!("[spi] the driver would not start: {error:?}");
            loop {}
        }
    };

    println!("[spi] listening on SPI2");

    let delay = Delay::new();
    // `Option`, so the first answer is a change and gets printed. Starting
    // it at zeros would make "nothing is driving MISO" — the state of every
    // run before the host has declared anything, and the one this probe most
    // needs to report — silent.
    let mut last_answer: Option<[u8; 2]> = None;
    let mut told = false;

    loop {
        delay.delay_millis(100);

        // The write every time round, which is what a display gets. It used
        // to happen once, and once is not enough to assert on: the emulator
        // says the same transaction only the first time — a driver polling a
        // sensor would otherwise put twenty kilobytes a second down the
        // channel — so a one-shot write is reported in exactly one moment,
        // and if nobody is listening in that moment nobody ever hears it.
        // Repeating it makes the write alternate with the transfer below, so
        // each is a change from the one before and both are said every loop.
        match spi.write(&TELL) {
            Ok(()) => {
                if !told {
                    say("wrote ", &TELL);
                    told = true;
                }
            }
            Err(error) => {
                if !told {
                    // Said once: a failing transfer every hundred
                    // milliseconds is a log nobody can read.
                    println!("[spi] the write failed: {error:?}");
                    told = true;
                }
            }
        }

        let mut answer = ASK;
        match spi.transfer(&mut answer) {
            Ok(()) => {
                if last_answer != Some(answer) {
                    say("read ", &answer);
                    last_answer = Some(answer);
                }
            }
            Err(error) => println!("[spi] the transfer failed: {error:?}"),
        }
    }
}
