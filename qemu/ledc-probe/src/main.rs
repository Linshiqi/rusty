//! Does a duty the firmware sets reach the pin the matrix sends it to?
//!
//! Every other pin probe here is about a level. This one is about the
//! *shape* of a pin: LEDC is what a servo, a motor and a dimmed lamp are
//! driven by, and with nothing mapped at its registers a firmware
//! configures a timer, starts a channel and drives a pad that never moves —
//! the board shows a servo asleep while the firmware sweeps it, and nothing
//! anywhere says why.
//!
//! So this sets two duties at one frequency and then the same duty at
//! another, printing what it asked for each time. The emulator reports what
//! it made of the registers on the pin channel, and the gate compares the
//! two: a model that reported the duty and ignored the timer would pass the
//! first half and fail the second.
//!
//! GPIO5 because the probes either side of it are spoken for — 0 is
//! blinky's lamp, 3 the converter's, 4 the interrupt's — and because the
//! matrix has to be asked for it, which is the half of this the model gets
//! wrong if it reads `GPIO_OUT` instead.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::ledc::channel::{self, ChannelIFace};
use esp_hal::ledc::timer::{self, TimerIFace};
use esp_hal::ledc::{LSGlobalClkSource, Ledc, LowSpeed};
use esp_hal::main;
use esp_hal::time::Rate;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

/// A panic says so, for the reason every probe here does: a silent death
/// reports a driver that failed exactly as it reports one that never ran.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[ledc] panicked: {info}");
    loop {}
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let delay = Delay::new();
    let pin = peripherals.GPIO5;

    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

    let mut timer = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    timer
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(24),
        })
        .expect("a 24 kHz timer at eight bits");

    let mut channel = ledc.channel(channel::Number::Channel0, pin);
    channel
        .configure(channel::config::Config {
            timer: &timer,
            duty_pct: 25,
            drive_mode: esp_hal::gpio::DriveMode::PushPull,
        })
        .expect("channel 0 on GPIO5");
    println!("[ledc] set 25 at 24000");
    delay.delay_millis(200);

    channel.set_duty(75).expect("three quarters");
    println!("[ledc] set 75 at 24000");
    delay.delay_millis(200);

    // A second timer at a servo's frequency, on a second channel and a
    // second pin: a model that read a duty and ignored the timer it follows
    // would report this one at 24 kHz too, and a model that reported one
    // channel for all six would put it on the wrong pin.
    let mut servo_timer = ledc.timer::<LowSpeed>(timer::Number::Timer1);
    servo_timer
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty12Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_hz(50),
        })
        .expect("a 50 Hz timer at twelve bits");
    let mut servo = ledc.channel(channel::Number::Channel1, peripherals.GPIO6);
    servo
        .configure(channel::config::Config {
            timer: &servo_timer,
            duty_pct: 8,
            drive_mode: esp_hal::gpio::DriveMode::PushPull,
        })
        .expect("channel 1 on GPIO6");
    println!("[ledc] set 8 at 50");

    loop {
        delay.delay_millis(500);
        println!("[ledc] holding");
    }
}
