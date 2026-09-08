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
//! **It also says what it programmed, and what the controller does with it.**
//! "Nothing happened" is the least useful sentence a probe can end on: the
//! peripheral may never have raised the line, the matrix may never have
//! carried it, the CPU may have it masked, or the handler may have run and
//! died. Those are four different fixes. So the pin's configuration, the
//! interrupt matrix's view of the GPIO source and the machine CSRs are
//! printed once after setup, the interrupt latch is printed whenever it
//! changes, the handler counts its *entry* separately from its completion,
//! and the panic handler talks. None of it reads the pin: `GPIO_STATUS` is
//! the interrupt latch, not the level, and `edges=` still comes from
//! nowhere but a handler that ran to the end.
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

/// The pin this probe listens on. Named once, because three register
/// offsets are computed from it.
const PIN: u8 = 4;

/// ESP32-C3 register addresses, from the technical reference manual. Read
/// as raw volatile words rather than through the peripheral access crate:
/// what is being checked here is whether the *emulator* and the HAL agree
/// about these exact addresses, and a PAC accessor would be a third
/// opinion between them.
mod reg {
    pub const GPIO: usize = 0x6000_4000;
    pub const GPIO_ENABLE: usize = GPIO + 0x20;
    /// The interrupt latch — one bit per pin, set by an edge and cleared by
    /// the firmware. Not the pin's level, which lives in `GPIO_IN`.
    pub const GPIO_STATUS: usize = GPIO + 0x44;
    /// The latch as the CPU sees it: `STATUS` masked by each pin's
    /// `INT_ENA`. What the peripheral turns into an interrupt request.
    pub const GPIO_PCPU_INT: usize = GPIO + 0x5C;
    /// Per-pin configuration; `INT_TYPE` is bits 9:7 and `INT_ENA` 17:13.
    pub const GPIO_PIN0: usize = GPIO + 0x74;

    pub const INTMTX: usize = 0x600C_2000;
    /// One word per interrupt source, holding the CPU line it is routed to.
    /// The GPIO source is number 16 on this part.
    pub const GPIO_SOURCE: usize = 16;
    pub const INT_ENABLE: usize = INTMTX + 0x104;
    pub const INT_TYPE: usize = INTMTX + 0x108;
    pub const INT_EIP_STATUS: usize = INTMTX + 0x110;
    /// Priority of CPU line `n`, `PRI_0` first.
    pub const INT_PRI0: usize = INTMTX + 0x114;
    pub const INT_THRESH: usize = INTMTX + 0x194;
}

fn peek(address: usize) -> u32 {
    unsafe { core::ptr::read_volatile(address as *const u32) }
}

/// A machine CSR by name. `mstatus.MIE` and `mie.MEIE` are the last two
/// gates an external interrupt passes, and firmware that never opened them
/// looks exactly like an emulator that never raised the line.
macro_rules! csr {
    ($name:literal) => {{
        let value: u32;
        unsafe { core::arch::asm!(concat!("csrr {0}, ", $name), out(reg) value) };
        value
    }};
}

/// Talks. A probe whose panic handler is a silent `loop {}` reports a
/// handler that died exactly as it reports a handler that never ran, and
/// those are opposite findings.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[irq] panic: {info}");
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

/// The pin, so the handler can clear what it fired for. A level the handler
/// never clears is an interrupt that re-enters for ever, which looks like a
/// hang rather than like a working model.
static LISTENING: Mutex<RefCell<Option<Input<'static>>>> = Mutex::new(RefCell::new(None));

/// How many edges the handler has seen through to the end. An atomic rather
/// than a print in the handler: `println!` from an interrupt is a lock this
/// has no reason to take, and the count is the whole of what the test
/// reads.
///
/// Load and store rather than `fetch_add`: this core is an RV32IMC, which
/// has no atomic read-modify-write at all, and the increment is inside the
/// critical section the handler already takes.
static EDGES: AtomicU32 = AtomicU32::new(0);

/// How many times the handler was *entered*, counted before it touches
/// anything. `ENTERED` ahead of `EDGES` is a handler that ran and did not
/// finish — a fault inside it, which is a different bug from never being
/// called, and indistinguishable from it with one counter.
static ENTERED: AtomicU32 = AtomicU32::new(0);

#[handler]
fn on_edge() {
    ENTERED.store(ENTERED.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
    critical_section::with(|cs| {
        if let Some(pin) = LISTENING.borrow_ref_mut(cs).as_mut() {
            pin.clear_interrupt();
        }
        EDGES.store(EDGES.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
    });
}

/// Everything the CPU side of one GPIO interrupt depends on, in one line.
/// Printed after setup and again whenever the latch moves, so a run that
/// ends in silence still says which link of the chain was open.
fn say_state(what: &str) {
    let source = peek(reg::INTMTX + reg::GPIO_SOURCE * 4);
    let line = source & 0x1f;
    let priority = peek(reg::INT_PRI0 + (line as usize) * 4);
    println!(
        "[irq] {what}: pin{PIN}={:#010x} enable={:#010x} status={:#010x} pcpu={:#010x} \
         map={line} prio={priority} cpu-enable={:#010x} cpu-type={:#010x} eip={:#010x} \
         thresh={} mstatus={:#010x} mie={:#010x} mip={:#010x} mtvec={:#010x}",
        peek(reg::GPIO_PIN0 + (PIN as usize) * 4),
        peek(reg::GPIO_ENABLE),
        peek(reg::GPIO_STATUS),
        peek(reg::GPIO_PCPU_INT),
        peek(reg::INT_ENABLE),
        peek(reg::INT_TYPE),
        peek(reg::INT_EIP_STATUS),
        peek(reg::INT_THRESH),
        csr!("mstatus"),
        csr!("mie"),
        csr!("mip"),
        csr!("mtvec"),
    );
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
        LISTENING.borrow_ref_mut(cs).replace(pin);
    });

    say_state("programmed");
    println!("[irq] listening on GPIO{PIN}");

    // The pin is never read here. A line below means the handler ran, which
    // means the peripheral raised an interrupt and the CPU took it — the
    // one thing this firmware exists to witness. The latch beside it is the
    // interrupt controller's own account, printed only when it moves.
    let mut last = 0;
    let mut last_entered = 0;
    let mut last_latch = peek(reg::GPIO_PCPU_INT);
    loop {
        let now = EDGES.load(Ordering::Relaxed);
        if now != last {
            println!("[irq] edges={now}");
            last = now;
        }
        let entered = ENTERED.load(Ordering::Relaxed);
        if entered != last_entered {
            println!("[irq] entered={entered}");
            last_entered = entered;
        }
        let latch = peek(reg::GPIO_PCPU_INT);
        if latch != last_latch {
            say_state("latched");
            last_latch = latch;
        }
    }
}
