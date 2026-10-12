//! A toy car's drive and a fan, on the board view.
//!
//! A lamp is on or off, and `[rusty:gpio]` says which. A motor is a *speed*,
//! and a speed lives in a duty cycle that a boolean channel cannot carry — so
//! there is a second channel for it:
//!
//! - `[rusty:pwm@<µs>] <pin>=<0.0..1.0>` — how hard a pin is being driven.
//!   Reported when the firmware *changes* it, not once per PWM cycle: a motor
//!   at 20 kHz produces forty thousand edges a second, and putting those on
//!   the same serial line as the console would drown everything else the
//!   firmware has to say.
//! - `[rusty:gpio@<µs>] <pin>=<0|1>` — unchanged, and what the H-bridge's two
//!   direction inputs travel on. Direction really is a pair of levels.
//!
//! **The board shows commanded drive, not a measured shaft speed.** Nothing
//! here is a dynamometer: there is no inertia, no load, no back-EMF. A motor
//! told 40% reads 40% the instant it is told, where a real one takes time to
//! get there against a load rusty knows nothing about. The panel says so.
//!
//! ## The H-bridge, which is the part worth watching
//!
//! Two direction inputs and an enable, and the table catches people:
//!
//! | IN1 | IN2 | what happens |
//! |---|---|---|
//! | 1 | 0 | forward |
//! | 0 | 1 | reverse |
//! | 0 | 0 | coast — the bridge opens and the motor freewheels |
//! | 1 | 1 | **brake** — the winding is shorted and the shaft is held |
//!
//! `1,1` is the one to have seen. It looks like "both on, so full speed" and
//! it is a stop. This example walks all four states in order so the board
//! draws each of them; the rotor keeps turning through COAST for as long as a
//! real one would coast, which is to say rusty does not model that either and
//! stops it dead. That is the honest end of what a duty cycle can tell you.
//!
//! ## Pins
//!
//! GPIO0 drive PWM, GPIO1/GPIO2 the H-bridge inputs, GPIO3 the fan. The C3
//! has already spent 12..17 on the SPI flash, 18/19 on native USB and 20/21
//! on the console, which is what rusty's pin panel is for.
//!
//! Wire it in the Simulate panel: drop a `motor`, pull its `pwm` stub to
//! GPIO0 and `in1`/`in2` to GPIO1 and GPIO2. Drop a second one and wire only
//! its `pwm` to GPIO3 — a motor with no direction pins is a fan, and the
//! sheet is where that is decided rather than a menu.
//!
//! For changing the speed from the panel rather than watching a fixed
//! sequence, `examples/pid-tune` is the other half: `[rusty:param]` announces
//! a tunable and `S<name>=<value>` sets it, without a reflash.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::gpio::{DriveMode, Level, Output, OutputConfig};
use esp_hal::ledc::channel::{self, ChannelIFace};
use esp_hal::ledc::timer::{self, TimerIFace};
use esp_hal::ledc::{LSGlobalClkSource, Ledc, LowSpeed};
use esp_hal::main;
use esp_hal::time::{Duration, Instant, Rate};
use esp_println::println;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

/// Above hearing. A motor driven at 1 kHz whines, and the whine is the first
/// thing anyone notices about a toy that is otherwise finished.
const PWM_HZ: u32 = 20_000;

/// How long each step of the walk below holds, so a person watching the board
/// has time to read the state before it changes.
const STEP: Duration = Duration::from_millis(600);

/// One leg of the demonstration: what the bridge is told, and how hard.
struct Step {
    in1: bool,
    in2: bool,
    /// Percent, which is what `Channel::set_duty` takes.
    duty_pct: u8,
    /// What this leg is for, printed once so the console explains the board.
    note: &'static str,
}

/// Every state an H-bridge has, in an order that reads as a story: pull away,
/// hold, let go, come back, stop hard.
const WALK: &[Step] = &[
    Step { in1: true,  in2: false, duty_pct: 0,   note: "forward, stopped" },
    Step { in1: true,  in2: false, duty_pct: 25,  note: "forward, pulling away" },
    Step { in1: true,  in2: false, duty_pct: 60,  note: "forward, cruising" },
    Step { in1: true,  in2: false, duty_pct: 100, note: "forward, flat out" },
    // Both low. The bridge opens; a real motor freewheels down. rusty does
    // not model momentum, so the board stops the rotor at once and says
    // COAST -- the label is the honest part, the stillness is not a claim.
    Step { in1: false, in2: false, duty_pct: 60,  note: "coast -- duty still 60%, and it goes nowhere" },
    Step { in1: false, in2: true,  duty_pct: 40,  note: "reverse" },
    Step { in1: false, in2: true,  duty_pct: 80,  note: "reverse, harder" },
    // The one people get wrong. Not full speed: a short across the winding.
    Step { in1: true,  in2: true,  duty_pct: 80,  note: "BRAKE -- both inputs high is a stop, not full speed" },
    Step { in1: false, in2: false, duty_pct: 0,   note: "coast, idle" },
];

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    // The H-bridge's direction inputs. Ordinary outputs -- direction is two
    // levels, and levels are exactly what the boolean channel carries.
    let mut in1 = Output::new(peripherals.GPIO1, Level::Low, OutputConfig::default());
    let mut in2 = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

    let mut timer0 = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    timer0
        .configure(timer::config::Config {
            // Eight bits is 256 steps, which is finer than a motor's own
            // dead band and far finer than anyone can see on the board.
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_hz(PWM_HZ),
        })
        .expect("the timer takes 20 kHz at 8 bits");

    let mut drive = ledc.channel(channel::Number::Channel0, peripherals.GPIO0);
    drive
        .configure(channel::config::Config {
            timer: &timer0,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("channel 0 on timer 0");

    let mut fan = ledc.channel(channel::Number::Channel1, peripherals.GPIO3);
    fan.configure(channel::config::Config {
        timer: &timer0,
        duty_pct: 0,
        drive_mode: DriveMode::PushPull,
    })
    .expect("channel 1 on the same timer");

    println!("[rusty:disp] motor demo");

    // What was last announced, so the duty channel carries changes rather
    // than a running commentary. A pin held at 60% for a second is one line,
    // not fifty.
    let mut said_drive: Option<u8> = None;
    let mut said_fan: Option<u8> = None;
    let mut step = 0usize;
    // The fan sweeps on its own clock, to show two independent duties on one
    // board -- and because a fan that only ever ran with the wheels would
    // look like it was wired to them.
    let mut fan_pct: u8 = 0;
    let mut fan_rising = true;

    loop {
        let leg = &WALK[step % WALK.len()];
        step += 1;

        in1.set_level(if leg.in1 { Level::High } else { Level::Low });
        in2.set_level(if leg.in2 { Level::High } else { Level::Low });
        drive.set_duty(leg.duty_pct).expect("duty is a percentage");
        fan.set_duty(fan_pct).expect("duty is a percentage");

        let now = Instant::now().duration_since_epoch().as_micros();

        // Direction on the boolean channel, exactly as any other pair of
        // outputs would report themselves.
        println!("[rusty:gpio@{now}] 1={},2={}", u8::from(leg.in1), u8::from(leg.in2));

        // Duty on its own channel, and only when it moved.
        if said_drive != Some(leg.duty_pct) {
            println!("[rusty:pwm@{now}] 0={:.2}", f32::from(leg.duty_pct) / 100.0);
            said_drive = Some(leg.duty_pct);
        }
        if said_fan != Some(fan_pct) {
            println!("[rusty:pwm@{now}] 3={:.2}", f32::from(fan_pct) / 100.0);
            said_fan = Some(fan_pct);
        }

        // The same two numbers as a curve, for the Plot panel. The board says
        // what the motor is doing now; the plot says what it has been doing,
        // which is the question a ramp is actually about.
        println!("[rusty:tel@{now}] drive={:.2},fan={:.2}", f32::from(leg.duty_pct) / 100.0, f32::from(fan_pct) / 100.0);
        println!("{}", leg.note);

        fan_pct = match (fan_rising, fan_pct) {
            (true, p) if p >= 100 => {
                fan_rising = false;
                80
            }
            (true, p) => p + 20,
            (false, p) if p <= 20 => {
                fan_rising = true;
                40
            }
            (false, p) => p - 20,
        };

        let started = Instant::now();
        while started.elapsed() < STEP {}
    }
}
