//! A quadcopter rate loop, flown at a desk.
//!
//! The thing a flight controller is, minus the aircraft: gyro in, PID per
//! axis, four motor duties out. It exists to prove the half of the simulator
//! protocol that carries data *inward*, because without that half a control
//! loop cannot be run at all without hardware.
//!
//! **Why this could not be done before.** QEMU models no I2C and no SPI
//! slave, so firmware that reads an MPU6500 over a bus reads nothing, and the
//! attitude loop — the whole of a flight controller — was the one part of a
//! drone project the simulator could not touch. `Igyro=…` is the way round
//! that: the panel injects the sample the bus would have delivered, and every
//! line of the loop after the read is the code that will fly.
//!
//! What the firmware announces, on a timer rather than at boot (a panel
//! usually connects to a board that has been running for a while):
//!
//! ```text
//! [rusty:sensor] gyro=3 rad/s -35..35     what it wants fed, and the range
//! [rusty:param]  roll_p=0.14 0..0.6       what it will let you turn
//! ```
//!
//! What it reads back:
//!
//! ```text
//! Igyro=1.25,-0.5,0.02   one sample, whole -- see below
//! Sroll_p=0.2            a gain, without a reflash
//! A4=2900                the battery's ADC count
//! B9=1                   arm / disarm
//! ```
//!
//! **The sample arrives whole and is used whole.** Three axes fused from
//! three different moments is an attitude that drifts, and the drift looks
//! exactly like a bad gyro. One line, one sample, one iteration.
//!
//! Built for the C3 so it needs no forked toolchain — the pins below are the
//! C3's free ones. The real target is an ESP32, which is a chip switch from
//! the status bar: the target triple, `build-std`, the toolchain channel and
//! the `esp-hal` feature all move, and the pins are the part rusty refuses to
//! guess at because only this file knows what they should become.

#![no_std]
#![no_main]

use esp_hal::delay::Delay;
use esp_hal::main;
use esp_hal::time::Instant;
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Loop period. 200 Hz is slow for a real rate loop and fast enough that a
/// too-high gain rings visibly, which is what this is read for.
const DT: f32 = 1.0 / 200.0;

/// The four motor pins, quad-X order: front-left, front-right, rear-right,
/// rear-left. Diagonals spin the same way, which is what makes yaw work.
///
/// The C3 has already spent 12..17 on the SPI flash and 18/19 on native USB,
/// and 20/21 are the console. These four are free — and on the ESP32 this
/// project is really for, they are not the same four. That is the one thing
/// a chip switch will not do for you.
const MOTORS: [u8; 4] = [0, 1, 2, 3];

/// Where the battery divider lands. Injected as `A4=<count>` in simulation;
/// on hardware it is whatever the ADC reads.
const BATTERY_PIN: u8 = 4;

/// Full scale of the ADC these counts come from, and the count below which
/// the loop refuses to arm.
///
/// Counts rather than volts on purpose: this file knows its own divider and
/// the panel does not, so the conversion lives here where it can be right.
const BATTERY_FULL: f32 = 4095.0;
const BATTERY_CUTOFF: f32 = 0.62;

struct Tunable {
    name: &'static str,
    value: f32,
    min: f32,
    max: f32,
}

impl Tunable {
    fn announce(&self) {
        println!(
            "[rusty:param] {}={:.3} {}..{}",
            self.name, self.value, self.min, self.max
        );
    }

    /// Take a new value, clamped, and report what was actually taken.
    ///
    /// The clamp is the information: a panel that showed the number typed
    /// rather than the number used would be lying about the loop.
    fn take(&mut self, value: f32) {
        self.value = value.clamp(self.min, self.max);
    }
}

/// One axis of the rate controller.
#[derive(Default)]
struct Axis {
    integral: f32,
    previous: f32,
}

impl Axis {
    /// PID on the *rate* error. A rate loop holds a turn speed, not an angle
    /// — the outer angle loop sits on top of this and is not what tuning
    /// starts with.
    fn step(&mut self, error: f32, p: f32, i: f32, d: f32) -> f32 {
        // Clamped before it is used, not after: an integral that has wound up
        // to a thousand takes a thousand iterations to come back, and the
        // aircraft spends all of them going the wrong way.
        self.integral = (self.integral + error * DT).clamp(-50.0, 50.0);
        let derivative = (error - self.previous) / DT;
        self.previous = error;
        p * error + i * self.integral + d * derivative
    }

    fn reset(&mut self) {
        self.integral = 0.0;
        self.previous = 0.0;
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    // Both consoles, for the reason `pid-tune` records: esp-println's `auto`
    // backend decides at run time which one a host is on, so firmware that
    // reads only one is deaf on some cables and not others. And `with_rx` is
    // not optional — `Uart::new` leaves every pin unconnected, which reads
    // perfectly in QEMU and reads nothing on silicon.
    let uart = Uart::new(peripherals.UART0, UartConfig::default())
        .expect("uart0")
        .with_rx(peripherals.GPIO20);
    let (mut rx, _tx) = uart.split();
    let (mut usb, _usb_tx) =
        esp_hal::usb_serial_jtag::UsbSerialJtag::new(peripherals.USB_DEVICE).split();

    let mut tunables = [
        Tunable { name: "roll_p", value: 0.14, min: 0.0, max: 0.6 },
        Tunable { name: "roll_i", value: 0.02, min: 0.0, max: 0.2 },
        Tunable { name: "roll_d", value: 0.004, min: 0.0, max: 0.05 },
        Tunable { name: "throttle", value: 0.0, min: 0.0, max: 1.0 },
    ];

    let mut gyro = [0.0f32; 3];
    let mut battery = BATTERY_FULL;
    let mut armed = false;
    let mut axes = [Axis::default(), Axis::default(), Axis::default()];

    let mut line = [0u8; 96];
    let mut filled = 0usize;
    let mut announced = Instant::now();

    println!("[rusty:disp] rate loop ready");

    loop {
        let started = Instant::now();

        // Re-announce on a timer. A panel that connects to a board already
        // running would otherwise never learn what it may inject or turn,
        // and would show nothing with no way to find out why.
        if announced.elapsed().as_millis() > 1000 {
            println!("[rusty:sensor] gyro=3 rad/s -35..35");
            for tunable in &tunables {
                tunable.announce();
            }
            announced = Instant::now();
        }

        // Drain whatever arrived, from whichever console has it.
        let mut byte = [0u8; 1];
        loop {
            let read = if rx.read_ready() {
                rx.read_buffered(&mut byte).ok().filter(|got| *got == 1)
            } else {
                usb.read_byte().ok().map(|got| {
                    byte[0] = got;
                    1
                })
            };
            if read.is_none() {
                break;
            }
            if byte[0] == b'\n' || byte[0] == b'\r' {
                apply(
                    &line[..filled],
                    &mut tunables,
                    &mut gyro,
                    &mut battery,
                    &mut armed,
                );
                filled = 0;
            } else if filled < line.len() {
                line[filled] = byte[0];
                filled += 1;
            } else {
                // Overlong garbage: drop it rather than truncating it into a
                // different command.
                filled = 0;
            }
        }

        let (p, i, d, throttle) = (
            tunables[0].value,
            tunables[1].value,
            tunables[2].value,
            tunables[3].value,
        );

        // A flat battery disarms and stays disarmed. Cutting throttle without
        // disarming would let it re-arm itself as the voltage recovers under
        // no load, which is how a drone restarts in somebody's hands.
        if battery / BATTERY_FULL < BATTERY_CUTOFF && armed {
            armed = false;
            println!("[rusty:disp] battery low - disarmed");
        }

        // Setpoint is zero on every axis: hold still. The interesting input
        // is the gyro, which is what the panel injects.
        let corrections = if armed {
            [
                axes[0].step(-gyro[0], p, i, d),
                axes[1].step(-gyro[1], p, i, d),
                axes[2].step(-gyro[2], p, i, d),
            ]
        } else {
            for axis in &mut axes {
                axis.reset();
            }
            [0.0; 3]
        };

        let duties = mix(if armed { throttle } else { 0.0 }, corrections);

        let now = Instant::now().duration_since_epoch().as_micros();
        println!(
            "[rusty:pwm@{now}] {}={:.3},{}={:.3},{}={:.3},{}={:.3}",
            MOTORS[0], duties[0], MOTORS[1], duties[1], MOTORS[2], duties[2], MOTORS[3], duties[3],
        );
        println!(
            "[rusty:tel@{now}] gyro_r={:.2},gyro_p={:.2},gyro_y={:.2},m0={:.3},m1={:.3},m2={:.3},m3={:.3},batt={:.2}",
            gyro[0],
            gyro[1],
            gyro[2],
            duties[0],
            duties[1],
            duties[2],
            duties[3],
            battery / BATTERY_FULL,
        );

        while started.elapsed().as_micros() < (DT * 1e6) as u64 {
            delay.delay_micros(50);
        }
    }
}

/// Quad-X mixing: throttle on every motor, roll and pitch across the
/// diagonals, yaw against the two spin directions.
///
/// Clamped per motor rather than scaled as a group, which is the honest
/// simple version: at full throttle a correction has nowhere to go, and a
/// mixer that pretended otherwise would hide the saturation the plot is being
/// read for.
fn mix(throttle: f32, [roll, pitch, yaw]: [f32; 3]) -> [f32; 4] {
    [
        (throttle - roll + pitch + yaw).clamp(0.0, 1.0),
        (throttle + roll + pitch - yaw).clamp(0.0, 1.0),
        (throttle + roll - pitch + yaw).clamp(0.0, 1.0),
        (throttle - roll - pitch - yaw).clamp(0.0, 1.0),
    ]
}

/// Apply one inbound line.
///
/// Four shapes, one letter each, exactly as the protocol has them — `I` a
/// sensor sample, `S` a tunable, `A` an analog pin, `B` a button. Strict on
/// purpose: an unknown name changes nothing and says nothing, because a
/// firmware that silently accepted a misspelled gain leaves somebody turning
/// a knob attached to nothing.
fn apply(
    raw: &[u8],
    tunables: &mut [Tunable],
    gyro: &mut [f32; 3],
    battery: &mut f32,
    armed: &mut bool,
) {
    let Ok(text) = core::str::from_utf8(raw) else {
        return;
    };
    let text = text.trim();
    let Some((head, value)) = text.split_once('=') else {
        return;
    };
    let Some((letter, name)) = head.split_at_checked(1) else {
        return;
    };

    match letter {
        // The whole sample, or none of it. A partial `Igyro=1.0` would leave
        // pitch and yaw at their previous values and fuse two moments — the
        // drift this protocol exists to avoid.
        "I" if name == "gyro" => {
            let mut parsed = [0.0f32; 3];
            let mut seen = 0;
            for (slot, field) in parsed.iter_mut().zip(value.split(',')) {
                let Ok(number) = field.trim().parse::<f32>() else {
                    return;
                };
                *slot = number;
                seen += 1;
            }
            if seen == 3 {
                *gyro = parsed;
            }
        }
        "S" => {
            if let Ok(number) = value.trim().parse::<f32>()
                && let Some(tunable) = tunables.iter_mut().find(|t| t.name == name)
            {
                tunable.take(number);
                tunable.announce();
            }
        }
        "A" => {
            if name.parse::<u8>() == Ok(BATTERY_PIN)
                && let Ok(count) = value.trim().parse::<f32>()
            {
                *battery = count;
            }
        }
        // Arming is a button so it can be a key on the board view. It refuses
        // while the battery is down: a loop that armed itself flat is one
        // that browns out mid-throttle.
        "B" if name == "9" => {
            let pressed = value.trim() == "1";
            if pressed && *battery / BATTERY_FULL >= BATTERY_CUTOFF {
                *armed = !*armed;
                println!(
                    "[rusty:disp] {}",
                    if *armed { "armed" } else { "disarmed" }
                );
            }
        }
        _ => {}
    }
}
