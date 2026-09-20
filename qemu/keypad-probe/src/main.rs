//! Does a matrix keypad read the way one does on a desk?
//!
//! Every other probe here drives a pin or reads a pin. A keypad does
//! neither: a key **joins two pins**, and which way the level then flows is
//! whichever of them the firmware is driving at that instant. Scanning is
//! the whole of it — one row is pulled low for a moment, the columns are
//! read, and a key that is down carries that low to its column.
//!
//! Two holes in the emulator made this impossible, and each one alone is
//! enough to make a working keypad read as broken:
//!
//! - **No pull resistors.** The columns rest high because the pad pulls
//!   them there. With no pull modelled, an input reads whatever it last
//!   read — zero from reset — so every key in the matrix reads as held down
//!   before the firmware has done anything at all.
//! - **No way to join two pins.** The host could drive a level onto a pad
//!   (`4=0`), which is not what a key does: during a scan the row is an
//!   output for a moment, and a host driving the column low instead would
//!   be holding down every key in that column.
//!
//! So the gate around this firmware asserts both directions. At rest it
//! must report **no key** — which is only true if the pull-ups answer — and
//! with `sw <row>-<col>=1` on the channel it must report exactly the key
//! that joins those two pins, and lose it again when the switch opens.
//!
//! Four by four, because a matrix's whole difficulty is telling one key
//! from fifteen others sharing its wires, and a 2x2 would not exercise a
//! scan that has to move on to the next row. The pins avoid what the C3
//! has already spent: 12..17 on the flash, 18/19 on the native USB, 20/21
//! on the console, 2/8/9 strapping.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{AnyPin, Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[keypad] panicked: {info}");
    loop {}
}

/// The key at a row and a column, named as the keys are printed on a pad.
const KEYS: [[char; 4]; 4] = [
    ['1', '2', '3', 'A'],
    ['4', '5', '6', 'B'],
    ['7', '8', '9', 'C'],
    ['*', '0', '#', 'D'],
];

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let delay = Delay::new();

    // Rows drive, columns are read. **High** while idle and low only while
    // their own row is being scanned, which is what stops one row's key
    // from answering for another's.
    let mut rows: [Output; 4] = [
        Output::new(
            AnyPin::from(peripherals.GPIO0),
            Level::High,
            OutputConfig::default(),
        ),
        Output::new(
            AnyPin::from(peripherals.GPIO1),
            Level::High,
            OutputConfig::default(),
        ),
        Output::new(
            AnyPin::from(peripherals.GPIO3),
            Level::High,
            OutputConfig::default(),
        ),
        Output::new(
            AnyPin::from(peripherals.GPIO4),
            Level::High,
            OutputConfig::default(),
        ),
    ];
    // `Pull::Up` and `is_low()`: the ordinary way a button is read, and the
    // one that needs the pad's own pull to be modelled at all.
    let up = InputConfig::default().with_pull(Pull::Up);
    let columns: [Input; 4] = [
        Input::new(AnyPin::from(peripherals.GPIO5), up),
        Input::new(AnyPin::from(peripherals.GPIO6), up),
        Input::new(AnyPin::from(peripherals.GPIO7), up),
        Input::new(AnyPin::from(peripherals.GPIO10), up),
    ];

    println!("[keypad] scanning 4x4 on rows 0,1,3,4 and columns 5,6,7,10");

    // Only a change is printed, so the log says what the pad did rather
    // than how often the loop ran — and a key held down is one line, not a
    // thousand. `None` at the start, so the first scan says something
    // whatever it finds: silence must not be the resting state of a gate
    // that is looking for "no key".
    let mut said: Option<Option<char>> = None;

    loop {
        let mut down = None;

        for (r, row) in rows.iter_mut().enumerate() {
            row.set_low();
            // A moment for the pad to settle — on silicon this is the RC of
            // the line, here it is a no-op that keeps the firmware honest.
            delay.delay_micros(50);
            for (c, column) in columns.iter().enumerate() {
                if column.is_low() {
                    down = Some(KEYS[r][c]);
                }
            }
            row.set_high();
        }

        if said != Some(down) {
            match down {
                Some(key) => println!("[keypad] down {key}"),
                None => println!("[keypad] none"),
            }
            said = Some(down);
        }
        delay.delay_millis(20);
    }
}
