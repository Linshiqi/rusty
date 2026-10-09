//! The CH32V003: 16 KB of flash, 2 KB of RAM, a QingKe V2A hart and the
//! peripherals a firmware built on ch32-hal reaches for first — the clock
//! tree, the pins, both timers, SysTick, the interrupt controller, USART1 —
//! and the debug data registers SDI print writes through.
//!
//! **What it does not model it names**, once, on the first access: ADC,
//! I2C, SPI, DMA, the watchdogs. A peripheral nothing models gets its
//! registers back as it wrote them, which keeps a driver's read-modify-write
//! honest, and nothing it waits for ever happens — so the line saying which
//! peripheral it was is the difference between "my firmware hangs" and
//! "the emulator has no ADC".
//!
//! Time is a 48 MHz count of base ticks; each instruction is one HCLK
//! period of them. A QingKe V2A takes about one clock for most instructions
//! and more for a taken branch or a load from flash, so a loop timed by
//! counting instructions runs somewhat fast here, while anything timed by
//! SysTick or a timer — every `Delay` and every PWM — keeps the part's time
//! exactly.

mod gpio;
mod pfic;
mod rcc;
mod systick;
mod timer;
mod usart;

use std::collections::HashSet;

use crate::cpu::{self, Bus, Fault, Hart, Size, Step};
use gpio::{Afio, Exti, Port, Role};
pub use gpio::{exists as pin_exists, name as pin_name};
use pfic::Pfic;
use rcc::{BASE_HZ, Rcc};
use systick::SysTick;
use timer::{Drive, Timer};
use usart::Usart;

const FLASH_SIZE: usize = 16 * 1024;
const RAM_SIZE: usize = 2 * 1024;

const SYSTICK_IRQ: u32 = 12;
const SOFTWARE_IRQ: u32 = 14;
const EXTI7_0_IRQ: u32 = 20;
const USART1_IRQ: u32 = 32;
const TIM1_UP_IRQ: u32 = 35;
const TIM1_CC_IRQ: u32 = 37;
const TIM2_IRQ: u32 = 38;

/// Something the outside world should hear about, at a time in
/// microseconds since reset.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub at_us: u64,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    /// A pin driven to a level.
    Level { pin: u8, high: bool },
    /// A pin a timer drives: high for `duty` of each period, `hz` periods a
    /// second.
    Pwm { pin: u8, duty: f64, hz: f64 },
    /// A line of text the firmware printed, by SDI print or USART1.
    Console(String),
    /// Something about the model the person reading should know: a
    /// peripheral it does not have, a fault the firmware took.
    Note(String),
}

/// What a pin is doing, as last reported.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Out {
    Level(bool),
    Pwm { duty: f64, hz: f64 },
}

/// Every peripheral this machine names, by its 1 KB block.
const BLOCKS: &[(u32, &str)] = &[
    (0x4000_0000, "TIM2"),
    (0x4000_2C00, "WWDG"),
    (0x4000_3000, "IWDG"),
    (0x4000_5400, "I2C1"),
    (0x4000_7000, "PWR"),
    (0x4001_0000, "AFIO"),
    (0x4001_0400, "EXTI"),
    (0x4001_0800, "GPIOA"),
    (0x4001_1000, "GPIOC"),
    (0x4001_1400, "GPIOD"),
    (0x4001_2400, "ADC1"),
    (0x4001_2C00, "TIM1"),
    (0x4001_3000, "SPI1"),
    (0x4001_3800, "USART1"),
    (0x4002_0000, "DMA1"),
    (0x4002_1000, "RCC"),
    (0x4002_2000, "FLASH"),
    (0x4002_3800, "EXTEND"),
];

/// Peripherals whose registers are kept and nothing more, and what the
/// firmware will find missing. FLASH is kept quietly: its one register a
/// firmware writes in the ordinary way is the wait-state count, which an
/// emulator need not honour.
const UNMODELLED: &[(&str, &str)] = &[
    ("WWDG", "the window watchdog never resets the part"),
    ("IWDG", "the independent watchdog never resets the part"),
    (
        "I2C1",
        "no transfer completes, so a driver waits on its first start",
    ),
    ("PWR", "sleep and the voltage detector do nothing"),
    (
        "ADC1",
        "no conversion completes, so a read waits on its end-of-conversion flag",
    ),
    (
        "SPI1",
        "no transfer completes, so a driver waits on its first byte",
    ),
    ("DMA1", "no transfer happens"),
    (
        "EXTEND",
        "the extended configuration and the op-amp do nothing",
    ),
];

struct Board {
    flash: Vec<u8>,
    ram: Vec<u8>,
    rcc: Rcc,
    afio: Afio,
    exti: Exti,
    /// Ports A, C and D, at their flat index 0, 2, 3; 1 is port B, which the
    /// part does not have.
    ports: [Port; 4],
    tim1: Timer,
    tim2: Timer,
    systick: SysTick,
    pfic: Pfic,
    usart: Usart,
    /// The debug data registers SDI print writes through.
    data1: u32,
    /// Registers of peripherals nothing models, by address.
    shadow: Vec<(u32, u32)>,
    /// The time of the access in progress, in base ticks.
    now: u64,
    /// Set by a write that may change what a pin shows.
    pins_dirty: bool,
    /// Set by a write that may move a timer's next event.
    timers_dirty: bool,
    /// Set by any access to a register or anything said: the one thing the
    /// bookkeeping between instructions asks before doing any.
    touched: bool,
    sdi_line: Vec<u8>,
    uart_line: Vec<u8>,
    lines: Vec<String>,
    notes: Vec<String>,
    said: HashSet<String>,
}

impl Board {
    fn new(flash: Vec<u8>) -> Self {
        Self {
            flash,
            ram: vec![0; RAM_SIZE],
            rcc: Rcc::default(),
            afio: Afio::default(),
            exti: Exti::default(),
            ports: [Port::new(), Port::new(), Port::new(), Port::new()],
            tim1: Timer::new(true),
            tim2: Timer::new(false),
            systick: SysTick::default(),
            pfic: Pfic::default(),
            usart: Usart::default(),
            data1: 0,
            shadow: Vec::new(),
            now: 0,
            pins_dirty: true,
            timers_dirty: true,
            touched: true,
            sdi_line: Vec::new(),
            uart_line: Vec::new(),
            lines: Vec::new(),
            notes: Vec::new(),
            said: HashSet::new(),
        }
    }

    fn hclk(&self) -> u64 {
        self.rcc.hclk_period()
    }

    /// Say something once.
    fn note(&mut self, key: &str, text: impl FnOnce() -> String) {
        if self.said.insert(key.to_string()) {
            self.notes.push(text());
            self.touched = true;
        }
    }

    fn console(line: &mut Vec<u8>, lines: &mut Vec<String>, byte: u8) {
        match byte {
            b'\n' => lines.push(String::from_utf8_lossy(&std::mem::take(line)).into_owned()),
            b'\r' => {}
            _ => line.push(byte),
        }
    }

    /// SDI print's protocol: the low byte of DATA0 is how many bytes follow,
    /// up to seven, in DATA0's upper three and DATA1's four; the debugger
    /// takes them and writes zero back, which is what the firmware waits for.
    fn sdi(&mut self, data0: u32) {
        let mut bytes = [0u8; 8];
        bytes[..4].copy_from_slice(&data0.to_le_bytes());
        bytes[4..].copy_from_slice(&self.data1.to_le_bytes());
        let count = usize::from(bytes[0]).min(7);
        for &byte in &bytes[1..=count] {
            Self::console(&mut self.sdi_line, &mut self.lines, byte);
        }
    }

    fn block_name(addr: u32) -> Option<&'static str> {
        BLOCKS
            .iter()
            .find(|(base, _)| addr & !0x3FF == *base)
            .map(|(_, name)| *name)
    }

    fn shadow_read(&self, addr: u32) -> u32 {
        self.shadow
            .iter()
            .find(|(a, _)| *a == addr)
            .map_or(0, |(_, v)| *v)
    }

    fn shadow_write(&mut self, addr: u32, value: u32) {
        match self.shadow.iter_mut().find(|(a, _)| *a == addr) {
            Some(slot) => slot.1 = value,
            None => self.shadow.push((addr, value)),
        }
    }

    fn unmodelled(&mut self, addr: u32) {
        let name = Self::block_name(addr);
        match name.and_then(|name| UNMODELLED.iter().find(|(n, _)| *n == name)) {
            Some((name, missing)) => {
                self.note(name, || {
                    format!("{name} is not modelled: its registers read back what was written, and {missing}")
                });
            }
            None if name.is_none() => {
                self.note(&format!("{:08x}", addr & !0x3FF), || {
                    format!("nothing is mapped at 0x{addr:08x} on a CH32V003")
                });
            }
            None => {}
        }
    }

    /// The level a timer's output pin is at right now, for `INDR`.
    fn driven_level(&self, pin: u8) -> Option<bool> {
        match self.drive_of(pin)? {
            Out::Level(level) => Some(level),
            // Between edges a PWM pin is at whichever level most of the
            // period is; a firmware reading its own PWM pin back is rare.
            Out::Pwm { duty, .. } => Some(duty >= 0.5),
        }
    }

    fn indr(&self, port: usize) -> u32 {
        (0..8u8)
            .filter(|&n| {
                let pin = port as u8 * 8 + n;
                self.ports[port].level(n, self.driven_level(pin))
            })
            .fold(0, |bits, n| bits | (1 << n))
    }

    /// What the part drives on `pin`, or `None` while it drives nothing.
    fn drive_of(&self, pin: u8) -> Option<Out> {
        if !gpio::exists(pin) {
            return None;
        }
        let port = &self.ports[usize::from(pin / 8)];
        match port.role(pin % 8) {
            Role::Input { .. } => None,
            Role::Output(level) => Some(Out::Level(level)),
            Role::Alternate => self.alternate(pin),
        }
    }

    /// Which peripheral reaches an alternate-function pin, and what it puts
    /// there. TIM1 before TIM2 before USART1, as the part resolves two
    /// peripherals mapped to one pin: by which is enabled — here, by which
    /// is driving anything at all.
    fn alternate(&self, pin: u8) -> Option<Out> {
        let hclk = self.hclk();
        let (tim1_main, tim1_comp) = self.afio.tim1_pins();
        for (channel, &at) in tim1_main.iter().enumerate() {
            if at == pin
                && let (Some(drive), _) = self.tim1.outputs(channel, hclk)
            {
                return Some(out(drive));
            }
        }
        for (channel, &at) in tim1_comp.iter().enumerate() {
            if at == pin
                && let (_, Some(drive)) = self.tim1.outputs(channel, hclk)
            {
                return Some(out(drive));
            }
        }
        for (channel, &at) in self.afio.tim2_pins().iter().enumerate() {
            if at == pin
                && let (Some(drive), _) = self.tim2.outputs(channel, hclk)
            {
                return Some(out(drive));
            }
        }
        // A transmitter idles high.
        (self.afio.usart1_tx() == pin).then_some(Out::Level(true))
    }

    fn mmio_read(&mut self, addr: u32) -> u32 {
        // A read can clear a flag (USART's data register) as well as a
        // write can.
        self.touched = true;
        let (now, hclk) = (self.now, self.hclk());
        let offset = addr & 0x3FF;
        match addr & !0x3FF {
            0x4000_0000 => {
                let value = self.tim2.read(offset, now, hclk);
                self.pins_dirty |= self.tim2.take_changed();
                value
            }
            0x4001_0000 => self.afio.read(offset),
            0x4001_0400 => self.exti.read(offset),
            0x4001_0800 | 0x4001_1000 | 0x4001_1400 => {
                let port = ((addr >> 10) & 0b111) as usize - 2;
                let indr = self.indr(port);
                self.ports[port].read(offset, || indr)
            }
            0x4001_2C00 => {
                let value = self.tim1.read(offset, now, hclk);
                self.pins_dirty |= self.tim1.take_changed();
                value
            }
            0x4001_3800 => self.usart.read(offset),
            0x4002_1000 => self.rcc.read(offset),
            0xE000_0000 => match offset {
                // DATA0: zero once the debugger has taken the bytes, which
                // here is at once.
                0xF4 => 0,
                0xF8 => self.data1,
                _ => self.shadow_read(addr),
            },
            0xE000_E000..=0xE000_EC00 => self.pfic.read(addr - 0xE000_E000),
            0xE000_F000 => self.systick.read(offset, now, hclk),
            _ => {
                self.unmodelled(addr);
                self.shadow_read(addr)
            }
        }
    }

    fn mmio_write(&mut self, addr: u32, value: u32) {
        self.touched = true;
        let (now, hclk) = (self.now, self.hclk());
        let offset = addr & 0x3FF;
        match addr & !0x3FF {
            0x4000_0000 => {
                self.pins_dirty |= self.tim2.write(offset, value, now, hclk);
                self.pins_dirty |= self.tim2.take_changed();
                self.timers_dirty = true;
                if self.tim2.counts_unusually() {
                    self.note("tim2-direction", || {
                        "TIM2 counts down or centre-aligned: its pins' duty and frequency are \
                         right, the counter read back is approximate"
                            .to_string()
                    });
                }
            }
            0x4001_0000 => {
                self.afio.write(offset, value);
                self.pins_dirty = true;
            }
            0x4001_0400 => self.exti.write(offset, value),
            0x4001_0800 | 0x4001_1000 | 0x4001_1400 => {
                let port = ((addr >> 10) & 0b111) as usize - 2;
                self.ports[port].write(offset, value);
                self.pins_dirty = true;
            }
            0x4001_2C00 => {
                self.pins_dirty |= self.tim1.write(offset, value, now, hclk);
                self.pins_dirty |= self.tim1.take_changed();
                self.timers_dirty = true;
                if self.tim1.counts_unusually() {
                    self.note("tim1-direction", || {
                        "TIM1 counts down or centre-aligned: its pins' duty and frequency are \
                         right, the counter read back is approximate"
                            .to_string()
                    });
                }
            }
            0x4001_3800 => {
                if let Some(byte) = self.usart.write(offset, value) {
                    Self::console(&mut self.uart_line, &mut self.lines, byte);
                }
            }
            0x4002_1000 => {
                self.rcc.write(offset, value);
                // A new clock is a new PWM frequency and a new tick rate.
                self.pins_dirty = true;
                self.timers_dirty = true;
                if self.rcc.wants_crystal {
                    self.note("hse", || {
                        "the firmware turned on HSE, an external crystal, and this board has \
                         none: it will wait for it for ever — use the internal oscillator \
                         (ch32-hal's SYSCLK_FREQ_48MHZ_HSI)"
                            .to_string()
                    });
                }
            }
            0xE000_0000 => match offset {
                0xF4 => {
                    if value != 0 {
                        self.sdi(value);
                    }
                }
                0xF8 => self.data1 = value,
                _ => self.shadow_write(addr, value),
            },
            0xE000_E000..=0xE000_EC00 => self.pfic.write(addr - 0xE000_E000, value),
            0xE000_F000 => {
                self.systick.write(offset, value, now, hclk);
                self.timers_dirty = true;
            }
            _ => {
                self.unmodelled(addr);
                self.shadow_write(addr, value);
            }
        }
    }

    fn is_mmio(addr: u32) -> bool {
        matches!(addr, 0x4000_0000..=0x4002_3FFF | 0xE000_0000..=0xE000_FFFF)
    }

    /// Bytes behind an address that is memory: flash at both its addresses,
    /// RAM, and the read-only system area.
    fn memory(&mut self, addr: u32, len: u32) -> Option<&mut [u8]> {
        let (bytes, offset): (&mut [u8], u32) = match addr {
            0x0000_0000..=0x0000_3FFF => (&mut self.flash, addr),
            0x0800_0000..=0x0800_3FFF => (&mut self.flash, addr - 0x0800_0000),
            0x2000_0000..=0x2000_07FF => (&mut self.ram, addr - 0x2000_0000),
            _ => return None,
        };
        let start = offset as usize;
        bytes.get_mut(start..start + len as usize)
    }

    /// The vendor's area: the unique id and capacity, and the option bytes
    /// as an unprotected part leaves the factory.
    fn system(addr: u32) -> Option<u32> {
        match addr {
            // FLACAP: 16 KB.
            0x1FFF_F7E0 => Some(16),
            0x1FFF_F000..=0x1FFF_F7FF => Some(0),
            // RDPR 0xA5 with its complement, USER 0xF7 with its complement.
            0x1FFF_F800 => Some(0x5AA5),
            0x1FFF_F802 => Some(0x08F7),
            0x1FFF_F804..=0x1FFF_F83F => Some(0x00FF),
            _ => None,
        }
    }
}

fn out(drive: Drive) -> Out {
    match drive {
        Drive::Level(level) => Out::Level(level),
        Drive::Pwm { duty, hz } => Out::Pwm { duty, hz },
    }
}

impl Bus for Board {
    fn load(&mut self, addr: u32, size: Size) -> Result<u32, Fault> {
        if let Some(bytes) = self.memory(addr, size.bytes()) {
            let mut value = 0;
            for (i, byte) in bytes.iter().enumerate() {
                value |= u32::from(*byte) << (8 * i);
            }
            return Ok(value);
        }
        if Self::is_mmio(addr) {
            let shift = 8 * (addr & 3);
            let word = self.mmio_read(addr & !3);
            let mask = match size {
                Size::Byte => 0xFF,
                Size::Half => 0xFFFF,
                Size::Word => u32::MAX,
            };
            return Ok((word >> shift) & mask);
        }
        let word = Self::system(addr & !1).ok_or(Fault)?;
        Ok(match size {
            Size::Byte => (word >> (8 * (addr & 1))) & 0xFF,
            Size::Half => word & 0xFFFF,
            Size::Word => word | (Self::system(addr + 2).unwrap_or(0) << 16),
        })
    }

    fn store(&mut self, addr: u32, size: Size, value: u32) -> Result<(), Fault> {
        if (0x2000_0000..=0x2000_07FF).contains(&addr) {
            let bytes = self.memory(addr, size.bytes()).ok_or(Fault)?;
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = (value >> (8 * i)) as u8;
            }
            return Ok(());
        }
        if Self::is_mmio(addr) {
            let word_addr = addr & !3;
            let value = match size {
                Size::Word => value,
                _ => {
                    // A narrow store merges into what is there. Read without
                    // side effects: the shadow, or the register as it stands.
                    let shift = 8 * (addr & 3);
                    let mask = if size == Size::Byte { 0xFF } else { 0xFFFF } << shift;
                    let old = self.peek(word_addr);
                    (old & !mask) | ((value << shift) & mask)
                }
            };
            self.mmio_write(word_addr, value);
            return Ok(());
        }
        // Flash is written through the flash controller's programming
        // sequence, never by a store; the system area not at all.
        Err(Fault)
    }

    fn fetch(&mut self, addr: u32) -> Result<u16, Fault> {
        let bytes = self.memory(addr, 2).ok_or(Fault)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
}

impl Board {
    /// A register's value for merging a narrow store, with no read side
    /// effect (the USART's data register would otherwise lose a byte).
    fn peek(&mut self, addr: u32) -> u32 {
        if addr & !0x3FF == 0x4001_3800 && addr & 0x3FF == 0x04 {
            return 0;
        }
        self.mmio_read(addr)
    }
}

/// A CH32V003, with a firmware in its flash.
pub struct Machine {
    hart: Hart,
    board: Board,
    /// The time, in 48 MHz base ticks since reset.
    now: u64,
    /// `wfi`: nothing runs until an interrupt is pending.
    sleeping: bool,
    /// The interrupts being handled, innermost last.
    active: Vec<u32>,
    /// When a timer next needs looking at.
    next_event: Option<u64>,
    /// What each pin was last said to be doing.
    reported: [Option<Out>; 32],
    events: Vec<Event>,
}

impl Machine {
    /// A machine with the image's segments in its flash, out of reset.
    pub fn new(image: &[u8]) -> Result<Self, String> {
        let mut flash = vec![0xFF; FLASH_SIZE];
        for segment in crate::elf::segments(image)? {
            let at = match segment.addr {
                0x0000_0000..=0x0000_3FFF => segment.addr,
                0x0800_0000..=0x0800_3FFF => segment.addr - 0x0800_0000,
                other => {
                    return Err(format!(
                        "the ELF loads {} bytes at 0x{other:08x}, which is not the CH32V003's \
                         flash — was it linked for another part?",
                        segment.bytes.len(),
                    ));
                }
            } as usize;
            let end = at + segment.bytes.len();
            if end > FLASH_SIZE {
                return Err(format!(
                    "the image needs {end} bytes of flash and the CH32V003 has {FLASH_SIZE}"
                ));
            }
            flash[at..end].copy_from_slice(&segment.bytes);
        }
        Ok(Self {
            hart: Hart::new(),
            board: Board::new(flash),
            now: 0,
            sleeping: false,
            active: Vec::new(),
            next_event: None,
            reported: [None; 32],
            events: Vec::new(),
        })
    }

    /// Microseconds since reset.
    pub fn now_us(&self) -> u64 {
        self.now / (BASE_HZ / 1_000_000)
    }

    /// Run until `deadline_us`, or until something worth saying happens and
    /// `deadline_us` has been reached — every event is stamped with the time
    /// it happened at.
    pub fn run_until(&mut self, deadline_us: u64) {
        let deadline = deadline_us * (BASE_HZ / 1_000_000);
        while self.now < deadline {
            if self.next_event.is_some_and(|at| self.now >= at) {
                self.sync_timers();
            }
            if self.hart.interrupts_enabled()
                && let Some(irq) = self.board.pfic.next()
            {
                self.board.pfic.take(irq);
                self.active.push(irq);
                self.sleeping = false;
                self.board.now = self.now;
                self.hart.interrupt(&mut self.board, irq);
            }
            if self.sleeping {
                if self.board.pfic.wakes() {
                    self.sleeping = false;
                } else {
                    // Nothing to do until the next timer event, or the
                    // deadline when nothing is coming.
                    self.now = self
                        .next_event
                        .unwrap_or(deadline)
                        .clamp(self.now + 1, deadline);
                    continue;
                }
            }
            self.board.now = self.now;
            let step = self.hart.step(&mut self.board);
            self.now += self.board.hclk();
            match step {
                Step::Ran => {}
                Step::Wait => self.sleeping = true,
                Step::Returned => {
                    if let Some(irq) = self.active.pop() {
                        self.board.pfic.finish(irq);
                    }
                }
                Step::Trapped { cause, pc, tval } => self.trapped(cause, pc, tval),
            }
            self.after_step();
        }
    }

    fn trapped(&mut self, code: u32, pc: u32, tval: u32) {
        let what = cpu::cause::name(code);
        let detail = match code {
            cpu::cause::ILLEGAL_INSTRUCTION => format!(" 0x{tval:08x}"),
            cpu::cause::LOAD_FAULT
            | cpu::cause::STORE_FAULT
            | cpu::cause::LOAD_MISALIGNED
            | cpu::cause::STORE_MISALIGNED => format!(" at 0x{tval:08x}"),
            _ => String::new(),
        };
        let key = format!("trap {code} {pc:08x}");
        self.board.note(&key, || {
            format!(
                "[rusty:cpu] {what}{detail} at pc 0x{pc:08x}; the firmware's exception handler runs"
            )
        });
    }

    /// The bookkeeping between instructions: a reset asked for, the pins'
    /// reports, the interrupt lines, the notes.
    fn after_step(&mut self) {
        // Most instructions touch nothing but registers and RAM.
        if !self.board.touched && self.hart.unknown_csr.is_none() {
            return;
        }
        self.board.touched = false;
        if self.board.pfic.reset_requested {
            self.reset();
            return;
        }
        if let Some(csr) = self.hart.unknown_csr.take() {
            self.board.note(&format!("csr {csr:x}"), || {
                format!(
                    "[rusty:cpu] CSR 0x{csr:03x} is not modelled; it reads back what was written"
                )
            });
        }
        if self.board.timers_dirty {
            self.board.timers_dirty = false;
            self.reschedule();
        }
        self.raise_lines();
        if self.board.pins_dirty {
            self.board.pins_dirty = false;
            self.report_pins();
        }
        let at_us = self.now_us();
        for line in self.board.lines.drain(..) {
            self.events.push(Event {
                at_us,
                kind: EventKind::Console(line),
            });
        }
        for note in self.board.notes.drain(..) {
            self.events.push(Event {
                at_us,
                kind: EventKind::Note(note),
            });
        }
    }

    fn sync_timers(&mut self) {
        let (now, hclk) = (self.now, self.board.hclk());
        self.board.tim1.sync(now, hclk);
        self.board.tim2.sync(now, hclk);
        if self.board.tim1.take_changed() | self.board.tim2.take_changed() {
            self.board.pins_dirty = true;
        }
        self.board.systick.sync(now, hclk);
        self.reschedule();
        self.raise_lines();
        if self.board.pins_dirty {
            self.board.pins_dirty = false;
            self.report_pins();
        }
    }

    fn reschedule(&mut self) {
        let hclk = self.board.hclk();
        self.next_event = [
            self.board.tim1.next_event(hclk),
            self.board.tim2.next_event(hclk),
            self.board.systick.next_event(hclk),
        ]
        .into_iter()
        .flatten()
        .min();
    }

    fn raise_lines(&mut self) {
        let b = &self.board;
        let mut lines = 0u64;
        let mut line = |irq: u32, high: bool| {
            if high {
                lines |= 1 << irq;
            }
        };
        line(SYSTICK_IRQ, b.systick.line());
        line(SOFTWARE_IRQ, b.systick.software());
        line(EXTI7_0_IRQ, b.exti.line());
        line(USART1_IRQ, b.usart.line());
        line(TIM1_UP_IRQ, b.tim1.update_line());
        line(TIM1_CC_IRQ, b.tim1.compare_line());
        line(TIM2_IRQ, b.tim2.any_line());
        self.board.pfic.raise(lines);
    }

    fn report_pins(&mut self) {
        let at_us = self.now_us();
        for pin in 0..32u8 {
            let Some(now) = self.board.drive_of(pin) else {
                continue;
            };
            if self.reported[usize::from(pin)] == Some(now) {
                continue;
            }
            self.reported[usize::from(pin)] = Some(now);
            let kind = match now {
                Out::Level(high) => EventKind::Level { pin, high },
                Out::Pwm { duty, hz } => EventKind::Pwm { pin, duty, hz },
            };
            self.events.push(Event { at_us, kind });
        }
    }

    /// What the part does on a system reset: the hart and every peripheral
    /// back to their reset state; the flash, the RAM's contents and the
    /// outside world's levels stay.
    fn reset(&mut self) {
        let flash = std::mem::take(&mut self.board.flash);
        let ram = std::mem::take(&mut self.board.ram);
        let external = self.board.ports.each_ref().map(|port| port.external);
        let said = std::mem::take(&mut self.board.said);
        self.board = Board::new(flash);
        self.board.ram = ram;
        self.board.said = said;
        for (port, levels) in self.board.ports.iter_mut().zip(external) {
            port.external = levels;
        }
        self.hart = Hart::new();
        self.active.clear();
        self.sleeping = false;
        self.next_event = None;
        self.events.push(Event {
            at_us: self.now_us(),
            kind: EventKind::Note("the firmware reset the part".to_string()),
        });
    }

    /// Drive a pin from outside — a button, a wire to another board — or
    /// stop driving it (`None`). An edge reaches EXTI as on the part.
    pub fn drive(&mut self, pin: u8, level: Option<bool>) {
        if !gpio::exists(pin) {
            return;
        }
        let (port, n) = (usize::from(pin / 8), pin % 8);
        let before = self.board.ports[port].level(n, self.board.driven_level(pin));
        self.board.ports[port].external[usize::from(n)] = level;
        let after = self.board.ports[port].level(n, self.board.driven_level(pin));
        if before != after && usize::from(self.board.afio.exti_port(n)) == port {
            self.board.exti.edge(n, after);
            self.raise_lines();
        }
    }

    /// Bytes typed at the firmware, into USART1's receiver.
    pub fn receive(&mut self, bytes: &[u8]) {
        self.board.usart.receive(bytes);
        self.raise_lines();
    }

    /// What has happened since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests;
