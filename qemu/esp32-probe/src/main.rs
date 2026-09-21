//! Does an ESP32 get the board the C3 already gets?
//!
//! One probe rather than six, because every Xtensa build costs a toolchain
//! the rest of this directory does not need — espup's fork, since there is
//! no prebuilt `core` for this target — and one firmware that walks the
//! peripherals pays for it once.
//!
//! What it is looking for is not "does the peripheral respond". Each of
//! these had a failure mode of its own on this part, and each one reads as
//! something other than a missing model:
//!
//! - **The pads' pulls.** `Input::new(pin, Pull::Up)` with `is_low()` is
//!   how nearly every button on every board is read. With IO_MUX unmapped
//!   an input nobody drives keeps whatever it last read, which from reset
//!   is zero — so every such button is *held down* before the firmware has
//!   done anything. And this part's IO_MUX registers are a table in pad-name
//!   order rather than pin order, so a model that computed the offset would
//!   put one pin's pull on another's register: the pins here are chosen so
//!   that a table read as arithmetic answers wrongly for all of them.
//! - **The pads that have no pull at all.** GPIO34..39 are input-only on
//!   this part: no output driver and no pull circuitry, so their two bits
//!   read back zero however they are written. Firmware asking for a pull
//!   there floats on the desk and must float here.
//! - **The converter.** This part has no `APB_SARADC`; its SAR is driven
//!   from `SENS`, where one register per unit carries the pad enable, the
//!   start, the done bit and the counts. With nothing there
//!   `read_oneshot()` polls a done bit nothing can set and the firmware
//!   hangs inside the user's own call — so the poll here is bounded, and a
//!   conversion that never finishes is printed rather than waited on.
//! - **The bus.** Every register the model touches is at the same offset on
//!   both parts; what is not the same is that this one has sixteen command
//!   slots where the C3 has eight, and — found by this probe on its first
//!   run, as `?op0` — that its command op codes are 0, 1, 2, 3, 4 where the
//!   C3's are 6, 1, 3, 2, 4.
//! - **The wire.** SPI2 shares only the address of `CMD` between the parts,
//!   down to which bit starts a transfer — 18 here, 24 there.
//! - **The duty.** This part's LEDC is two halves, eight high-speed channels
//!   and eight low-speed, and the high-speed half has no `para_up` at all:
//!   what is written to it is what is driving. Both halves are exercised,
//!   because a model that waited for a latch that does not exist would
//!   report every high-speed channel as silent for ever.
//! - **The strip.** RMT here is eight channels with their control bits in a
//!   second register, their RAM twice as far into the window, and their
//!   interrupts three bits to a channel rather than banded by event — and
//!   its memory size in a register the model used to drop, so every
//!   transmission failed with the RAM untouched until the window was
//!   shadowed like every other peripheral's.
//! - **The edge.** A GPIO interrupt crosses more than the pin model: the
//!   dispatcher learns *which* source fired from three status words that on
//!   this part are DPORT's and read as zero, and the handler's context save
//!   touches the FPU, which upstream's emulator leaves switched off at reset
//!   where the silicon does not. Either alone is an edge the pin reports
//!   and no handler hears. `say_state` prints every link of that chain, so
//!   a run that counts no edges says which one was open.
//!
//! The pins avoid what this part has already spent: 6..11 are the flash,
//! 1 and 3 the console, 34..39 input-only. GPIO4, 13, 32, 27 and 23 are
//! ordinary pads whose IO_MUX words are 17, 13, 6, 10 and 34 — scattered
//! on purpose. GPIO39 is input-only and takes the no-pull case; GPIO34 and
//! GPIO35 are ADC1's channels 6 and 7.

#![no_std]
#![no_main]
// Xtensa inline assembly is still unstable, and two of this part's
// interrupt gates are CPU special registers rather than memory.
#![feature(asm_experimental_arch)]

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use critical_section::Mutex;
use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{AnyPin, Event, Input, InputConfig, Io, Level, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c, SoftwareTimeout};
use esp_hal::ledc::channel::{self, ChannelIFace};
use esp_hal::ledc::timer::{self, TimerIFace};
use esp_hal::ledc::{HighSpeed, LSGlobalClkSource, Ledc, LowSpeed};
use esp_hal::handler;
use esp_hal::main;
use esp_hal::rmt::{PulseCode, Rmt, TxChannelConfig, TxChannelCreator};
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::{Duration, Rate};
use esp_println::println;

/// How many times to ask whether a conversion has finished before calling
/// it stuck. Far more than the microseconds the silicon needs, far less
/// than the gate's patience.
const PATIENCE: u32 = 200_000;

/// The sensor the host declares, and its identity register.
const SENSOR: u8 = 0x68;
const WHO_AM_I: u8 = 0x75;

/// Talks, because a silent panic handler reports a driver that died
/// exactly as it reports one that never ran.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[esp32] panic: {info}");
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

/// The pin the handler holds, so it can clear what it fired for. A latch
/// the handler never clears is an interrupt that re-enters for ever, which
/// reads as a hang rather than as a working model.
static LISTENING: Mutex<RefCell<Option<Input<'static>>>> = Mutex::new(RefCell::new(None));

/// How many edges the handler has seen through to the end. An atomic
/// rather than a print: `println!` from an interrupt takes a lock this has
/// no reason to take, and the count is the whole of the finding.
static EDGES: AtomicU32 = AtomicU32::new(0);

#[handler]
fn on_edge() {
    critical_section::with(|cs| {
        if let Some(pin) = LISTENING.borrow_ref_mut(cs).as_mut() {
            pin.clear_interrupt();
        }
        EDGES.store(EDGES.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
    });
}

/// Every link of the chain a GPIO interrupt crosses on this part, in one
/// line. Printed once after setup and again whenever the latch moves, so a
/// run that counts no edges still says which link was open. Raw volatile
/// words rather than through the PAC: what is being checked is whether the
/// *emulator* and the CPU agree about these exact addresses.
mod reg {
    pub const GPIO: usize = 0x3FF4_4000;
    pub const STATUS: usize = GPIO + 0x44;
    /// The latch as the CPU sees it: `STATUS` masked by each pin's
    /// `INT_ENA`. What the peripheral turns into a request.
    pub const PCPU_INT: usize = GPIO + 0x68;
    /// Per-pin configuration; this part puts pin 0's at 0x88.
    pub const PIN0: usize = GPIO + 0x88;

    pub const DPORT: usize = 0x3FF0_0000;
    /// Which sources are asserting, three words — what esp-hal reads to
    /// learn whose handler to call.
    pub const INTR_STATUS: usize = DPORT + 0xEC;
    /// One word per source, holding the CPU line it is routed to.
    pub const INTR_MAP: usize = DPORT + 0x104;
    /// GPIO's source number on this part.
    pub const GPIO_SOURCE: usize = 22;
}

fn peek(address: usize) -> u32 {
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

/// An Xtensa special register by name. `INTENABLE` and `INTERRUPT` are the
/// last two gates a peripheral interrupt passes, and firmware that never
/// opened them looks exactly like an emulator that never raised the line.
macro_rules! sr {
    ($name:literal) => {{
        let value: u32;
        unsafe { core::arch::asm!(concat!("rsr.", $name, " {0}"), out(reg) value) };
        value
    }};
}

fn say_state(what: &str) {
    const PIN: usize = 17;
    println!(
        "[esp32] {what}: pin17={:#010x} status={:#010x} pcpu={:#010x}          map={:#x} src-status={:#010x} intenable={:#010x} interrupt={:#010x} ps={:#010x}",
        peek(reg::PIN0 + PIN * 4),
        peek(reg::STATUS),
        peek(reg::PCPU_INT),
        peek(reg::INTR_MAP + reg::GPIO_SOURCE * 4),
        peek(reg::INTR_STATUS),
        sr!("intenable"),
        sr!("interrupt"),
        sr!("ps"),
    );
}

/// Bytes as lowercase hex, into a fixed buffer.
fn hex(bytes: &[u8], out: &mut [u8; 16]) -> usize {
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

/// One pulse code carrying one bit, the way every one-wire strip sends it:
/// a one is the long half high, a zero the short one.
fn code(bit: bool) -> PulseCode {
    if bit {
        PulseCode::new(Level::High, 64, Level::Low, 36)
    } else {
        PulseCode::new(Level::High, 36, Level::Low, 64)
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let delay = Delay::new();

    // The edge, before anything else takes IO_MUX. GPIO17 rests low behind
    // its pull-down and the host drives it either way; every crossing that
    // reaches the handler is counted.
    let mut io = Io::new(peripherals.IO_MUX);
    io.set_interrupt_handler(on_edge);
    let mut listening = Input::new(
        peripherals.GPIO17,
        InputConfig::default().with_pull(Pull::Down),
    );
    critical_section::with(|cs| {
        listening.listen(Event::AnyEdge);
        LISTENING.borrow_ref_mut(cs).replace(listening);
    });

    // Up, down, up, down, up. Alternating on purpose: a model that answered
    // every pad's pull from one shared register would pass a test where
    // every pin wanted the same thing.
    let up = InputConfig::default().with_pull(Pull::Up);
    let down = InputConfig::default().with_pull(Pull::Down);
    let pulled: [(u8, Input); 5] = [
        (4, Input::new(AnyPin::from(peripherals.GPIO4), up)),
        (13, Input::new(AnyPin::from(peripherals.GPIO13), down)),
        (32, Input::new(AnyPin::from(peripherals.GPIO32), up)),
        (27, Input::new(AnyPin::from(peripherals.GPIO27), down)),
        (23, Input::new(AnyPin::from(peripherals.GPIO23), up)),
    ];

    // Asked for a pull-up it cannot have. Nothing on the host drives this
    // pad, so an honoured pull would read 1 and the silicon reads 0.
    let no_pull = Input::new(AnyPin::from(peripherals.GPIO39), up);

    let mut adc_config = AdcConfig::new();
    // 11 dB, the widest range, because the host speaks in the converter's
    // own counts and the attenuation only decides what voltage those counts
    // would have meant on a real part.
    let mut adc6 = adc_config.enable_pin(peripherals.GPIO34, Attenuation::_11dB);
    let mut adc7 = adc_config.enable_pin(peripherals.GPIO35, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, adc_config);

    // The bus, bounded: with nothing at the master's registers a driver's
    // first transaction waits on an interrupt nothing can raise, and the
    // firmware hangs in `write_read` rather than answering.
    let i2c_config = I2cConfig::default()
        .with_software_timeout(SoftwareTimeout::Transaction(Duration::from_millis(200)));
    let mut i2c = I2c::new(peripherals.I2C0, i2c_config)
        .expect("the I2C master")
        .with_sda(peripherals.GPIO21)
        .with_scl(peripherals.GPIO22);

    let mut spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_khz(100))
            .with_mode(Mode::_0),
    )
    .expect("SPI2")
    .with_sck(peripherals.GPIO18)
    .with_mosi(peripherals.GPIO19)
    .with_miso(peripherals.GPIO25)
    .with_cs(peripherals.GPIO5);

    // Both halves of this part's LEDC. The low-speed one latches with
    // `para_up`; the high-speed one has no such bit and takes what is
    // written at once, and a model that waited for the latch anyway would
    // report GPIO33 as silent for ever.
    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

    let mut ls_timer = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    ls_timer
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(24),
        })
        .expect("a 24 kHz low-speed timer at eight bits");
    let mut ls_channel = ledc.channel(channel::Number::Channel0, peripherals.GPIO16);
    ls_channel
        .configure(channel::config::Config {
            timer: &ls_timer,
            duty_pct: 0,
            drive_mode: esp_hal::gpio::DriveMode::PushPull,
        })
        .expect("low-speed channel 0 on GPIO16");
    ls_channel.set_duty(75).expect("three quarters");

    let mut hs_timer = ledc.timer::<HighSpeed>(timer::Number::Timer0);
    hs_timer
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty14Bit,
            clock_source: timer::HSClockSource::APBClk,
            frequency: Rate::from_hz(50),
        })
        .expect("a 50 Hz high-speed timer at fourteen bits");
    let mut hs_channel = ledc.channel(channel::Number::Channel0, peripherals.GPIO33);
    hs_channel
        .configure(channel::config::Config {
            timer: &hs_timer,
            duty_pct: 0,
            drive_mode: esp_hal::gpio::DriveMode::PushPull,
        })
        .expect("high-speed channel 0 on GPIO33");
    // Eight percent of a 50 Hz period is 1.6 ms, which is a servo a little
    // past its middle — a duty a board can be read against.
    hs_channel.set_duty(8).expect("a servo's middle");

    // Three bytes down the strip, as a one-wire LED is sent them.
    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80)).expect("the RMT clock");
    let mut strip = rmt
        .channel0
        .configure_tx(
            &TxChannelConfig::default()
                .with_clk_divider(1)
                .with_idle_output_level(Level::Low)
                .with_idle_output(true),
        )
        .expect("RMT channel 0")
        .with_pin(peripherals.GPIO26);

    println!("[esp32] pulls on 4,13,32,27,23; no pull on 39; ADC1 on 34,35");
    println!("[esp32] i2c on 21/22, spi2 on 18/19/25/5, ledc on 16 and 33, rmt on 26");
    println!("[esp32] listening for edges on 17");
    say_state("programmed");

    // Only a change is printed for the pins, so the log says what the board
    // did rather than how often the loop ran. `None` to start with, so the
    // first round says something whatever it finds: silence must not be the
    // resting state of a gate looking for "the pull answered".
    let mut said: Option<([u8; 5], u8, u16, u16)> = None;
    let mut edges_said: u32 = u32::MAX;
    let mut latch_said: u32 = u32::MAX;
    let mut round: u32 = 0;

    loop {
        let mut levels = [0u8; 5];
        for (slot, (_, pin)) in levels.iter_mut().zip(pulled.iter()) {
            *slot = u8::from(pin.is_high());
        }
        let floating = u8::from(no_pull.is_high());

        let mut counts = [0u16; 2];
        for (slot, reading) in counts.iter_mut().zip([
            read_bounded(&mut adc, &mut adc6),
            read_bounded(&mut adc, &mut adc7),
        ]) {
            match reading {
                Some(value) => *slot = value,
                None => {
                    println!("[esp32] the conversion never finished");
                    loop {}
                }
            }
        }

        let now = (levels, floating, counts[0], counts[1]);
        if said != Some(now) {
            let mut at = 0;
            for (name, _) in pulled.iter() {
                println!("[esp32] pull {name}={}", levels[at]);
                at += 1;
            }
            println!("[esp32] nopull 39={floating}");
            println!("[esp32] adc 34={} 35={}", counts[0], counts[1]);
            said = Some(now);
        }

        let edges = EDGES.load(Ordering::Relaxed);
        if edges != edges_said {
            println!("[esp32] edges on 17: {edges}");
            edges_said = edges;
        }
        let latch = peek(reg::STATUS);
        if latch != latch_said {
            say_state("latched");
            latch_said = latch;
        }

        // The bus and the wire every round rather than on change: both
        // report per *change* on the emulator's side, so a transaction that
        // happened once in the first tenth of a second is a transaction
        // nobody listening from outside ever hears.
        let mut who = [0u8; 1];
        match i2c.write_read(SENSOR, &[WHO_AM_I], &mut who) {
            Ok(()) => {
                let mut out = [0u8; 16];
                let n = hex(&who, &mut out);
                println!("[esp32] i2c 68:75 -> {}", core::str::from_utf8(&out[..n]).unwrap());
            }
            Err(error) => println!("[esp32] the bus refused: {error:?}"),
        }

        let mut answer = [0xae_u8, 0xa5, 0x01];
        match spi.transfer(&mut answer) {
            Ok(()) => {
                let mut out = [0u8; 16];
                let n = hex(&answer, &mut out);
                println!("[esp32] spi read {}", core::str::from_utf8(&out[..n]).unwrap());
            }
            Err(error) => println!("[esp32] the wire refused: {error:?}"),
        }

        // Twenty-four bits, one LED's worth, green first as the WS2812
        // family sends it.
        let mut codes = [PulseCode::end_marker(); 25];
        for (slot, bit) in codes.iter_mut().zip(
            (0..24).map(|i| (0x10_00_20u32 >> (23 - i)) & 1 == 1),
        ) {
            *slot = code(bit);
        }
        codes[24] = PulseCode::end_marker();
        // `wait()` consumes the channel and hands it back, so the loop
        // takes it again rather than transmitting once and stopping.
        strip = match strip.transmit(&codes).expect("a transmission").wait() {
            Ok(back) => {
                println!("[esp32] rmt sent {round}");
                back
            }
            Err((error, back)) => {
                println!("[esp32] the strip refused: {error:?}");
                back
            }
        };

        round = round.wrapping_add(1);
        delay.delay_millis(20);
    }
}

/// One conversion, or `None` if the done bit never came.
///
/// Bounded because the interesting failure here is a converter that never
/// finishes, and a probe that spun on that would report it as silence —
/// the one thing a witness must never do.
fn read_bounded<'d, ADCI, PIN>(
    adc: &mut Adc<'d, ADCI, esp_hal::Blocking>,
    pin: &mut esp_hal::analog::adc::AdcPin<PIN, ADCI>,
) -> Option<u16>
where
    ADCI: esp_hal::analog::adc::RegisterAccess + 'd,
    PIN: esp_hal::analog::adc::AdcChannel,
{
    for _ in 0..PATIENCE {
        if let Ok(counts) = adc.read_oneshot(pin) {
            return Some(counts);
        }
    }
    None
}
