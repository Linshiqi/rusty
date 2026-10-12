//! Does a device the host put on the I2C bus answer the firmware's driver?
//!
//! The other probes are about one pin. This is about the bus almost every
//! real board has something on: a display, an IMU, a temperature sensor.
//! Without a model the driver's first transaction waits on an interrupt
//! nothing can raise, which — like the ADC — is a hang inside the user's own
//! call rather than a wrong answer.
//!
//! Three addresses and not a sweep of the whole bus. A scan is what a person
//! does; what this has to prove is narrower and sharper, and each address is
//! here for its own assertion:
//!
//! - `0x68` is a sensor the host has declared, with registers behind it. It
//!   must answer, and answer *what the host put there* — a model returning
//!   zeros would pass "it answered" and fail everything that matters.
//! - `0x3c` is a display: declared, no registers worth reading, and what
//!   matters is that the bytes written to it reach the host.
//! - `0x50` is declared by nobody. It must **not** answer. That is the
//!   assertion the others cannot make: a bus where every address ACKs is a
//!   bus where a missing part looks exactly like a present one, and firmware
//!   that probes for an optional device finds it every time.
//!
//! Each line is printed only when it changes, so the log says what the bus
//! did rather than how often the loop ran — and so a device the host adds or
//! takes away mid-run shows up as a change rather than being lost in
//! repetition.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::i2c::master::{Config, I2c, SoftwareTimeout};
use esp_hal::main;
use esp_hal::time::Duration;
use esp_println::println;

/// The sensor, its identity register, and the burst a driver would read.
const SENSOR: u8 = 0x68;
const WHO_AM_I: u8 = 0x75;
const BURST: u8 = 0x3b;
/// The display, and the two bytes a driver sends it.
const DISPLAY: u8 = 0x3c;
const DISPLAY_BYTES: [u8; 2] = [0x00, 0xae];
/// The address nobody declared.
const ABSENT: u8 = 0x50;

/// The peripheral's own registers, read as raw words.
///
/// A probe that only reports *changes* cannot tell "the bus is quiet" from
/// "the loop stopped", and this one ended two rounds on exactly that
/// ambiguity: three lines of output and then nothing, which was read as a
/// stalled guest and was in fact a working guest finding an empty bus. So it
/// counts its scans aloud, and says what the controller looked like the first
/// time a read failed. Addresses from esp-idf's soc/esp32c3/i2c_reg.h.
mod reg {
    pub const I2C: usize = 0x6001_3000;
    pub const CTR: usize = I2C + 0x04;
    pub const SR: usize = I2C + 0x08;
    pub const INT_RAW: usize = I2C + 0x20;
    pub const COMD0: usize = I2C + 0x58;
}

fn peek(address: usize) -> u32 {
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[i2c] panic: {info}");
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

/// Bytes as hex from an offset, answering with the new offset — so the two
/// accounts of one transaction are spelled the same way and can be compared
/// character for character.
///
/// The offset is the whole point and the first version did not have it: it
/// always wrote from the start, so building a list of two addresses put the
/// second one on top of the first and advanced the length anyway. `3c,68`
/// came out as `68,` followed by two NUL bytes, which prints as `68,` and
/// reads as a scan that found one device and a stray comma.
fn hex_at(bytes: &[u8], out: &mut [u8; 32], at: usize) -> usize {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut n = at;
    for byte in bytes {
        if n + 2 > out.len() {
            break;
        }
        out[n] = DIGITS[(byte >> 4) as usize];
        out[n + 1] = DIGITS[(byte & 0xf) as usize];
        n += 2;
    }
    n
}

fn hex(bytes: &[u8], out: &mut [u8; 32]) -> usize {
    hex_at(bytes, out, 0)
}

fn say(label: &str, bytes: &[u8]) {
    let mut buffer = [0u8; 32];
    let at = hex(bytes, &mut buffer);
    println!("[i2c] {label}{}", core::str::from_utf8(&buffer[..at]).unwrap_or("?"));
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // A software timeout, for the same reason the analog probe bounds its
    // poll: with nothing at the peripheral's registers the driver waits on
    // an interrupt that can never arrive, and a probe that waits with it
    // reports the hole as silence. The bus timeout in the hardware would not
    // do — that is a register a missing model does not have either.
    let config = Config::default()
        .with_software_timeout(SoftwareTimeout::Transaction(Duration::from_millis(200)));
    // GPIO5 and GPIO6 because nothing else here wants them: 0 is blinky's
    // LED, 3 the analog probe's, 4 the interrupt probe's, 2/8/9 are
    // strapping pins and 12..21 the flash, the native USB and the console.
    // The model does not route through the GPIO matrix, so the choice is
    // about not colliding rather than about the bus.
    let mut i2c = match I2c::new(peripherals.I2C0, config) {
        Ok(i2c) => i2c.with_sda(peripherals.GPIO5).with_scl(peripherals.GPIO6),
        Err(error) => {
            println!("[i2c] the driver would not start: {error:?}");
            loop {}
        }
    };

    println!("[i2c] listening on the bus");

    // `Option`, so the *first* scan is a change and gets printed. Starting
    // it at "nothing is there" would make an empty bus — the state of every
    // run before the host has declared anything — silent, and silence is
    // what this whole probe exists not to report.
    let mut last_present: Option<[bool; 3]> = None;
    let mut last_who = [0u8; 1];
    let mut last_burst = [0u8; 6];
    let mut wrote = false;
    let mut said_absent = false;
    let mut scans: u32 = 0;
    let mut said_registers = false;
    // A rate, because a driver has one. Spinning the bus flat out is not
    // something firmware does and it would put thousands of transactions a
    // second down the channel the console shares.
    let delay = Delay::new();

    loop {
        delay.delay_millis(100);
        scans += 1;
        // Aloud, every few seconds. Without it a loop that stopped and a bus
        // with nothing on it produce the same log — no output — and telling
        // those two apart is the whole difficulty of this gate.
        if scans % 50 == 0 {
            println!("[i2c] alive scans={scans}");
        }
        // Present is "it acknowledged its address", which is the only
        // question a scan can ask. One byte, because a zero-length read is
        // not a transaction on this bus.
        let mut present = [false; 3];
        for (slot, address) in [DISPLAY, ABSENT, SENSOR].into_iter().enumerate() {
            let mut one = [0u8; 1];
            present[slot] = i2c.read(address, &mut one).is_ok();
            // The controller's own account, once, the first time a read does
            // not succeed. What the command list held and what the status
            // register says about it is the difference between "the model ran
            // the wrong steps" and "the model ran the right ones and answered
            // nothing" — and no amount of staring at the driver settles that.
            if !present[slot] && !said_registers {
                said_registers = true;
                println!(
                    "[i2c] regs ctr={:#010x} sr={:#010x} int={:#010x} \
                     cmd={:#010x},{:#010x},{:#010x},{:#010x}",
                    peek(reg::CTR),
                    peek(reg::SR),
                    peek(reg::INT_RAW),
                    peek(reg::COMD0),
                    peek(reg::COMD0 + 4),
                    peek(reg::COMD0 + 8),
                    peek(reg::COMD0 + 12),
                );
            }
        }
        if last_present != Some(present) {
            let mut buffer = [0u8; 32];
            let mut at = 0;
            for (slot, address) in [DISPLAY, ABSENT, SENSOR].into_iter().enumerate() {
                if present[slot] {
                    at = hex_at(&[address], &mut buffer, at);
                    // A separator after each, and the last one is dropped
                    // below — so the line is a list and not a list with a
                    // trailing comma.
                    if at < buffer.len() {
                        buffer[at] = b',';
                        at += 1;
                    }
                }
            }
            let at = at.saturating_sub(1);
            println!(
                "[i2c] scan {}",
                if at == 0 {
                    "-"
                } else {
                    core::str::from_utf8(&buffer[..at]).unwrap_or("?")
                }
            );
            last_present = Some(present);
        }

        // The address nobody declared must refuse, and the refusal is worth
        // saying once: a bus that answered here would make every later
        // assertion meaningless.
        if !present[1] && !said_absent {
            println!("[i2c] {ABSENT:02x} did not answer, which is right");
            said_absent = true;
        } else if present[1] {
            println!("[i2c] {ABSENT:02x} answered and nobody put it there");
            said_absent = false;
        }

        if present[2] {
            let mut who = [0u8; 1];
            if i2c.write_read(SENSOR, &[WHO_AM_I], &mut who).is_ok() && who != last_who {
                say("who=", &who);
                last_who = who;
            }
            // Six bytes from one register onwards, which is how a driver
            // reads an accelerometer: the pointer moves as the bytes come
            // out, and a model that answered the same byte six times would
            // pass a one-byte read and fail here.
            let mut burst = [0u8; 6];
            if i2c.write_read(SENSOR, &[BURST], &mut burst).is_ok() && burst != last_burst {
                say("burst=", &burst);
                last_burst = burst;
            }
        }

        // Every time round, which is what a display gets — and what makes it
        // assertable. The emulator says the same transaction only the first
        // time, so a one-shot write is reported in exactly one moment; if
        // nobody is on the channel in that moment, nobody ever hears it. The
        // line to the serial log is still said once, because that one is
        // about the firmware and not about the bus.
        if present[0] {
            match i2c.write(DISPLAY, &DISPLAY_BYTES) {
                Ok(()) => {
                    if !wrote {
                        say("wrote ", &DISPLAY_BYTES);
                        wrote = true;
                    }
                }
                Err(error) => {
                    if !wrote {
                        println!("[i2c] the display refused: {error:?}");
                        wrote = true;
                    }
                }
            }
        }
    }
}
