//! Does a pin edge actually interrupt the firmware?
//!
//! `gpio-probe` proves a host-driven level reaches `is_high()` — by polling
//! it in a loop. That is the half of the input story a stub interrupt
//! controller cannot fail: firmware that asks to be *interrupted* by a
//! button, which is how nearly every real button is read, would sit in
//! `wfi` for ever while the pin moved underneath it.
//!
//! So this one never reads the pin outside the handler. It configures GPIO4
//! for both edges, counts what the handler saw, and prints the count when
//! it changes — a line here means the CPU took an interrupt the GPIO
//! peripheral raised, and nothing else can produce one.
//!
//! Pull::Down for the same reason `gpio-probe` uses it: the model has no
//! notion of a pull resistor, so the pin comes out of reset low and the
//! first edge the test drives is a real 0 -> 1.
//!
//! GPIO4 because the C3 has already spent 12..17 on the SPI flash, 18/19 on
//! the native USB and 20/21 on the console, 2/8/9 are strapping pins, and
//! GPIO0 belongs to blinky's LED.

#![no_std]
#![no_main]

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use critical_section::Mutex;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Event, Input, InputConfig, Io, Pull};
use esp_hal::handler;
use esp_hal::main;
use esp_println::println;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

/// The pin, so the handler can clear what it fired for. A level the handler
/// never clears is an interrupt that re-enters for ever, which looks like a
/// hang rather than like a working model.
static PIN: Mutex<RefCell<Option<Input<'static>>>> = Mutex::new(RefCell::new(None));

/// How many edges the handler has seen. An atomic rather than a print in
/// the handler: `println!` from an interrupt is a lock this has no reason
/// to take, and the count is the whole of what the test reads.
///
/// Load and store rather than `fetch_add`: this core is an RV32IMC, which
/// has no atomic read-modify-write at all, and the increment is inside the
/// critical section the handler already takes.
static EDGES: AtomicU32 = AtomicU32::new(0);

#[handler]
fn on_edge() {
    critical_section::with(|cs| {
        if let Some(pin) = PIN.borrow_ref_mut(cs).as_mut() {
            pin.clear_interrupt();
        }
        EDGES.store(EDGES.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
    });
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let mut io = Io::new(peripherals.IO_MUX);
    io.set_interrupt_handler(on_edge);

    let mut pin = Input::new(
        peripherals.GPIO4,
        InputConfig::default().with_pull(Pull::Down),
    );
    critical_section::with(|cs| {
        pin.listen(Event::AnyEdge);
        PIN.borrow_ref_mut(cs).replace(pin);
    });

    println!("[irq] listening on GPIO4");

    // The pin is never read here. A line below means the handler ran, which
    // means the peripheral raised an interrupt and the CPU took it — the
    // one thing this firmware exists to witness.
    let mut last = 0;
    loop {
        let now = EDGES.load(Ordering::Relaxed);
        if now != last {
            println!("[irq] edges={now}");
            last = now;
        }
    }
}
