//! Does what a display driver draws reach the host as the picture it is?
//!
//! The bus model already carries bytes both ways, and for a sensor that is
//! the whole story: the host puts a register there and the firmware reads
//! it. A display is the other direction and a different shape — no register
//! is ever read, the whole of it is written, and what a person wants to see
//! is not a list of transactions but the screen.
//!
//! Two things have to be true for that, and neither was:
//!
//! - **Every write has to be reported.** The bus says a transaction only
//!   when it differs from the one before it, which is right for a driver
//!   polling a sensor at a kilohertz and wrong for a framebuffer: clearing
//!   a screen is the same sixteen zero bytes sixty-four times, each landing
//!   somewhere else. Suppressed, the picture loses all but the first.
//! - **A continued transaction has to say so.** A 1 KiB framebuffer crosses
//!   a 32-byte FIFO, so one message is a run of steps, and a decoder that
//!   read the first byte of each step as a control byte would make nonsense
//!   of every one after the first.
//!
//! The driver is the `ssd1306` crate and the drawing is
//! `embedded-graphics` — somebody else's code, so what is proven is that
//! *a real driver's* stream decodes, not that the decoder reads a sequence
//! written beside it. The picture is geometry rather than text for the same
//! reason a probe asserts on numbers: a filled square in each far corner
//! and a line across the middle can be checked exactly, and they are only
//! all three right if the addressing window walked the whole of the RAM.

#![no_std]
#![no_main]

use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::i2c::master::{Config, I2c, SoftwareTimeout};
use esp_hal::main;
use esp_hal::time::Duration;
use esp_println::println;
use ssd1306::mode::DisplayConfig;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};

esp_bootloader_esp_idf::esp_app_desc!();

/// Where the module sits on the bus, and where the host declares it.
const DISPLAY: u8 = 0x3c;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[disp] panicked: {info}");
    loop {}
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let delay = Delay::new();

    // A software timeout for the reason every probe here has one: with
    // nothing at the peripheral's registers the driver waits on an
    // interrupt that can never arrive, and a probe that waits with it
    // reports the hole as silence.
    let config = Config::default()
        .with_software_timeout(SoftwareTimeout::Transaction(Duration::from_millis(200)));
    let i2c = match I2c::new(peripherals.I2C0, config) {
        Ok(i2c) => i2c.with_sda(peripherals.GPIO5).with_scl(peripherals.GPIO6),
        Err(error) => {
            println!("[disp] the driver would not start: {error:?}");
            loop {}
        }
    };

    // One write longer than the FIFO, before the driver takes the bus.
    //
    // `ssd1306` sends its framebuffer in sixteen-byte transactions, so
    // nothing it does crosses a FIFO refill — and the report that says
    // "more of the message before this" would never be exercised by the
    // frames below. A driver that hands the whole kilobyte to `write` in
    // one call is an ordinary thing to write (ESP-IDF's does), and this is
    // the shape that breaks a decoder reading each step as a message of its
    // own. Data bytes at the pointer, overwritten by the first frame.
    let mut i2c = i2c;
    let mut block = [0u8; 48];
    block[0] = 0x40;
    for (at, byte) in block.iter_mut().enumerate().skip(1) {
        *byte = at as u8;
    }
    loop {
        match i2c.write(DISPLAY, &block) {
            Ok(()) => {
                println!("[disp] wrote {} bytes in one go", block.len());
                break;
            }
            Err(error) => {
                println!("[disp] no panel yet: {error:?}");
                delay.delay_millis(200);
            }
        }
    }

    // The host declares the display a moment after boot, so the first
    // attempt to talk to it can land before it is there. Say so and go
    // round rather than stopping: an init that failed once is not an init
    // that cannot work, and a probe that gave up here would report a race
    // as a broken model.
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();

    println!("[disp] waiting for the panel");
    loop {
        match display.init() {
            Ok(()) => break,
            Err(error) => {
                println!("[disp] no panel yet: {error:?}");
                delay.delay_millis(200);
            }
        }
    }
    println!("[disp] the panel is up");

    let on = PrimitiveStyle::with_fill(BinaryColor::On);
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);

    // Every frame, not once: the bus reports a transaction when it differs
    // from the one before it, so a picture drawn in one moment is a picture
    // whoever connects a moment later never sees. Drawing it again is also
    // what makes each flush a change from the one beside it — the rule the
    // other bus gates learned the hard way.
    let mut frames: u32 = 0;
    loop {
        display.clear_buffer();
        // A square in the near corner and one in the far one: the second is
        // the last page of the last columns, so it is drawn only if the
        // addressing window walked the whole of the RAM.
        Rectangle::new(Point::new(0, 0), Size::new(8, 8))
            .into_styled(on)
            .draw(&mut display)
            .ok();
        Rectangle::new(Point::new(120, 56), Size::new(8, 8))
            .into_styled(on)
            .draw(&mut display)
            .ok();
        // And a line across the middle, which is one run of 128 pixels on
        // one row and nothing on the rows either side — a shape a decoder
        // that had the pages or the bits the wrong way round could not
        // produce.
        Line::new(Point::new(0, 32), Point::new(127, 32))
            .into_styled(stroke)
            .draw(&mut display)
            .ok();
        match display.flush() {
            Ok(()) => {
                frames += 1;
                println!("[disp] drew frame {frames}");
            }
            Err(error) => println!("[disp] the flush failed: {error:?}"),
        }
        delay.delay_millis(500);
    }
}
