//! WCH's CH32 parts as one machine: the CH32V003 (16 KB of flash, 2 KB of
//! RAM, a QingKe V2A hart) and the CH32X035 (62 KB, 20 KB, a V4C), each with
//! the peripherals a firmware built on ch32-hal reaches for first — the
//! clock tree, the pins, the timers, SysTick, the interrupt controller, the
//! USARTs — and the debug data registers SDI print writes through. What
//! differs between them is data, in [`part`]; the machine is the same.
//!
//! **What it does not model it names**, once, on the first access: ADC,
//! I2C, SPI, DMA, the watchdogs, and on the X035 USB, USB PD, the op-amps.
//! A peripheral nothing models gets its registers back as it wrote them,
//! which keeps a driver's read-modify-write honest, and nothing it waits
//! for ever happens — so the line saying which peripheral it was is the
//! difference between "my firmware hangs" and "the emulator has no ADC".
//!
//! Time is a 48 MHz count of base ticks; each instruction is one HCLK
//! period of them. A QingKe hart takes about one clock for most
//! instructions and more for a taken branch or a load from flash, so a loop
//! timed by counting instructions runs somewhat fast here, while anything
//! timed by SysTick or a timer — every `Delay` and every PWM — keeps the
//! part's time exactly.

mod gpio;
pub mod part;
mod pfic;
mod rcc;
mod systick;
mod timer;
mod usart;
mod x035_pins;

use std::collections::HashSet;

use crate::cpu::{self, Bus, Fault, Hart, Size, Step};
use gpio::{Afio, Exti, Port, Role};
pub use part::{CH32V003, CH32X035, Part};
use pfic::Pfic;
use rcc::{BASE_HZ, Rcc};
use systick::SysTick;
use timer::{Drive, Timer};
use usart::Usart;

const SYSTICK_IRQ: u32 = 12;
const SOFTWARE_IRQ: u32 = 14;

/// The address of the first port's block; each port is the next kilobyte.
const GPIO_BASE: u32 = 0x4001_0800;

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
    /// A line of text the firmware printed, by SDI print or a USART.
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

struct Board {
    part: &'static Part,
    flash: Vec<u8>,
    ram: Vec<u8>,
    rcc: Rcc,
    afio: Afio,
    exti: Exti,
    /// Ports A to D at their index; a port the part does not have is never
    /// reached, because its block is not decoded.
    ports: [Port; 4],
    /// The part's timers, in [`Part::timers`]' order.
    timers: Vec<Timer>,
    systick: SysTick,
    pfic: Pfic,
    /// The part's USARTs, in [`Part::usarts`]' order, each with the line it
    /// is printing.
    usarts: Vec<(Usart, Vec<u8>)>,
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
    lines: Vec<String>,
    notes: Vec<String>,
    said: HashSet<String>,
}

impl Board {
    fn new(part: &'static Part, flash: Vec<u8>) -> Self {
        Self {
            part,
            flash,
            ram: vec![0; part.ram],
            rcc: Rcc::new(part.family),
            afio: Afio::new(part),
            exti: Exti::new(part),
            ports: std::array::from_fn(|_| Port::new(part.width)),
            timers: part.timers.iter().map(|t| Timer::new(t.advanced)).collect(),
            systick: SysTick::new(part.core.registers == 32),
            pfic: Pfic::default(),
            usarts: part
                .usarts
                .iter()
                .map(|_| (Usart::default(), Vec::new()))
                .collect(),
            data1: 0,
            shadow: Vec::new(),
            now: 0,
            pins_dirty: true,
            timers_dirty: true,
            touched: true,
            sdi_line: Vec::new(),
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

    fn block_name(&self, addr: u32) -> Option<&'static str> {
        self.part
            .blocks
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
        let part = self.part;
        let name = self.block_name(addr);
        match name.and_then(|name| part.unmodelled.iter().find(|(n, _)| *n == name)) {
            Some((name, missing)) => {
                self.note(name, || {
                    format!("{name} is not modelled: its registers read back what was written, and {missing}")
                });
            }
            None if name.is_none() => {
                self.note(&format!("{:08x}", addr & !0x3FF), || {
                    format!("nothing is mapped at 0x{addr:08x} on a {}", part.name)
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
        let width = self.part.width;
        (0..width)
            .filter(|&n| {
                let pin = port as u8 * width + n;
                self.part.exists(pin) && self.ports[port].level(n, self.driven_level(pin))
            })
            .fold(0, |bits, n| bits | (1 << n))
    }

    /// What the part drives on `pin`, or `None` while it drives nothing.
    fn drive_of(&self, pin: u8) -> Option<Out> {
        if !self.part.exists(pin) {
            return None;
        }
        let width = self.part.width;
        let port = &self.ports[usize::from(pin / width)];
        match port.role(pin % width) {
            Role::Input { .. } => None,
            Role::Output(level) => Some(Out::Level(level)),
            Role::Alternate => self.alternate(pin),
        }
    }

    /// Which peripheral reaches an alternate-function pin, and what it puts
    /// there: the timers in order, main outputs before complements, then the
    /// USARTs' transmitters — as the part resolves two peripherals mapped to
    /// one pin, by which is enabled: here, by which is driving anything.
    fn alternate(&self, pin: u8) -> Option<Out> {
        let hclk = self.hclk();
        for (spec, timer) in self.part.timers.iter().zip(&self.timers) {
            let (main, comp) = self.afio.timer_pins(spec.number);
            for (channel, at) in main.into_iter().enumerate() {
                if at == Some(pin)
                    && let (Some(drive), _) = timer.outputs(channel, hclk)
                {
                    return Some(out(drive));
                }
            }
            for (channel, at) in comp.into_iter().enumerate() {
                if at == Some(pin)
                    && let (_, Some(drive)) = timer.outputs(channel, hclk)
                {
                    return Some(out(drive));
                }
            }
        }
        // A transmitter idles high.
        self.part
            .usarts
            .iter()
            .any(|spec| self.afio.usart_tx(spec.number) == Some(pin))
            .then_some(Out::Level(true))
    }

    fn timer_at(&self, block: u32) -> Option<usize> {
        self.part.timers.iter().position(|t| t.base == block)
    }

    fn usart_at(&self, block: u32) -> Option<usize> {
        self.part.usarts.iter().position(|u| u.base == block)
    }

    /// The port a GPIO block is, if the part has it.
    fn port_at(&self, block: u32) -> Option<usize> {
        let index = block.checked_sub(GPIO_BASE)? >> 10;
        self.part
            .die
            .iter()
            .any(|&(port, _)| u32::from(port) == index)
            .then_some(index as usize)
    }

    /// The debug data registers' offsets within their block: DATA0, DATA1.
    fn debug_data(&self) -> (u32, u32) {
        let data0 = self.part.core.debug_data & 0x3FF;
        (data0, data0 + 4)
    }

    fn mmio_read(&mut self, addr: u32) -> u32 {
        // A read can clear a flag (USART's data register) as well as a
        // write can.
        self.touched = true;
        let (now, hclk) = (self.now, self.hclk());
        let offset = addr & 0x3FF;
        let block = addr & !0x3FF;
        if let Some(i) = self.timer_at(block) {
            let value = self.timers[i].read(offset, now, hclk);
            self.pins_dirty |= self.timers[i].take_changed();
            return value;
        }
        if let Some(i) = self.usart_at(block) {
            return self.usarts[i].0.read(offset);
        }
        if let Some(port) = self.port_at(block) {
            let indr = self.indr(port);
            return self.ports[port].read(offset, || indr);
        }
        let (data0, data1) = self.debug_data();
        match block {
            0x4001_0000 => self.afio.read(offset),
            0x4001_0400 => self.exti.read(offset),
            0x4002_1000 => self.rcc.read(offset),
            // DATA0: zero once the debugger has taken the bytes, which here
            // is at once.
            0xE000_0000 if offset == data0 => 0,
            0xE000_0000 if offset == data1 => self.data1,
            0xE000_E000..=0xE000_EC00 => self.pfic.read(addr - 0xE000_E000),
            0xE000_F000 => self.systick.read(offset, now, hclk),
            0xE000_0000 => self.shadow_read(addr),
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
        let block = addr & !0x3FF;
        if let Some(i) = self.timer_at(block) {
            let timer = &mut self.timers[i];
            self.pins_dirty |= timer.write(offset, value, now, hclk);
            self.pins_dirty |= timer.take_changed();
            self.timers_dirty = true;
            if timer.counts_unusually() {
                let name = self.part.timers[i].name;
                self.note(&format!("{name}-direction"), || {
                    format!(
                        "{name} counts down or centre-aligned: its pins' duty and frequency are \
                         right, the counter read back is approximate"
                    )
                });
            }
            return;
        }
        if let Some(i) = self.usart_at(block) {
            let (usart, line) = &mut self.usarts[i];
            if let Some(byte) = usart.write(offset, value) {
                Self::console(line, &mut self.lines, byte);
            }
            return;
        }
        if let Some(port) = self.port_at(block) {
            self.ports[port].write(offset, value);
            self.pins_dirty = true;
            return;
        }
        let (data0, data1) = self.debug_data();
        match block {
            0x4001_0000 => {
                self.afio.write(offset, value);
                self.pins_dirty = true;
            }
            0x4001_0400 => self.exti.write(offset, value),
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
            0xE000_0000 if offset == data0 => {
                if value != 0 {
                    self.sdi(value);
                }
            }
            0xE000_0000 if offset == data1 => self.data1 = value,
            0xE000_0000 => self.shadow_write(addr, value),
            0xE000_E000..=0xE000_EC00 => self.pfic.write(addr - 0xE000_E000, value),
            0xE000_F000 => {
                self.systick.write(offset, value, now, hclk);
                self.timers_dirty = true;
                if self.systick.counts_down() {
                    self.note("systick-down", || {
                        "SysTick counts down: this model counts it up, so a delay timed by it \
                         is the right length only from zero"
                            .to_string()
                    });
                }
            }
            _ => {
                self.unmodelled(addr);
                self.shadow_write(addr, value);
            }
        }
    }

    fn is_mmio(&self, addr: u32) -> bool {
        (0x4000_0000..=self.part.mmio_end).contains(&addr)
            || (0xE000_0000..=0xE000_FFFF).contains(&addr)
    }

    /// Bytes behind an address that is memory: flash at both its addresses,
    /// and RAM.
    fn memory(&mut self, addr: u32, len: u32) -> Option<&mut [u8]> {
        let (flash, ram) = (self.flash.len() as u32, self.ram.len() as u32);
        let (bytes, offset): (&mut [u8], u32) = match addr {
            _ if addr < flash => (&mut self.flash, addr),
            0x0800_0000.. if addr - 0x0800_0000 < flash => (&mut self.flash, addr - 0x0800_0000),
            0x2000_0000.. if addr - 0x2000_0000 < ram => (&mut self.ram, addr - 0x2000_0000),
            _ => return None,
        };
        let start = offset as usize;
        bytes.get_mut(start..start + len as usize)
    }

    fn is_ram(&self, addr: u32) -> bool {
        addr.checked_sub(0x2000_0000)
            .is_some_and(|offset| (offset as usize) < self.ram.len())
    }

    /// The vendor's area: the unique id and capacity, and the option bytes
    /// as an unprotected part leaves the factory.
    fn system(&self, addr: u32) -> Option<u32> {
        match addr {
            // FLACAP: the flash's size in kilobytes.
            0x1FFF_F7E0 => Some((self.part.flash / 1024) as u32),
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
        if self.is_mmio(addr) {
            let shift = 8 * (addr & 3);
            let word = self.mmio_read(addr & !3);
            let mask = match size {
                Size::Byte => 0xFF,
                Size::Half => 0xFFFF,
                Size::Word => u32::MAX,
            };
            return Ok((word >> shift) & mask);
        }
        let word = self.system(addr & !1).ok_or(Fault)?;
        Ok(match size {
            Size::Byte => (word >> (8 * (addr & 1))) & 0xFF,
            Size::Half => word & 0xFFFF,
            Size::Word => word | (self.system(addr + 2).unwrap_or(0) << 16),
        })
    }

    fn store(&mut self, addr: u32, size: Size, value: u32) -> Result<(), Fault> {
        if self.is_ram(addr) {
            let bytes = self.memory(addr, size.bytes()).ok_or(Fault)?;
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = (value >> (8 * i)) as u8;
            }
            return Ok(());
        }
        if self.is_mmio(addr) {
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
    /// effect (a USART's data register would otherwise lose a byte).
    fn peek(&mut self, addr: u32) -> u32 {
        if self.usart_at(addr & !0x3FF).is_some() && addr & 0x3FF == 0x04 {
            return 0;
        }
        self.mmio_read(addr)
    }
}

/// A CH32 part, with a firmware in its flash.
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
    reported: Vec<Option<Out>>,
    events: Vec<Event>,
}

impl Machine {
    /// A `part` with the image's segments in its flash, out of reset.
    pub fn new(part: &'static Part, image: &[u8]) -> Result<Self, String> {
        let size = part.flash;
        let mut flash = vec![0xFF; size];
        for segment in crate::elf::segments(image)? {
            let at = match segment.addr {
                addr if (addr as usize) < size => addr,
                addr if addr >= 0x0800_0000 && ((addr - 0x0800_0000) as usize) < size => {
                    addr - 0x0800_0000
                }
                other => {
                    return Err(format!(
                        "the ELF loads {} bytes at 0x{other:08x}, which is not the {}'s \
                         flash — was it linked for another part?",
                        segment.bytes.len(),
                        part.name,
                    ));
                }
            } as usize;
            let end = at + segment.bytes.len();
            if end > size {
                return Err(format!(
                    "the image needs {end} bytes of flash and the {} has {size}",
                    part.name
                ));
            }
            flash[at..end].copy_from_slice(&segment.bytes);
        }
        Ok(Self {
            hart: Hart::with(part.core),
            board: Board::new(part, flash),
            now: 0,
            sleeping: false,
            active: Vec::new(),
            next_event: None,
            reported: vec![None; part.pin_count()],
            events: Vec::new(),
        })
    }

    /// The part this machine is.
    pub fn part(&self) -> &'static Part {
        self.board.part
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
        let mut changed = false;
        for timer in &mut self.board.timers {
            timer.sync(now, hclk);
            changed |= timer.take_changed();
        }
        if changed {
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
        self.next_event = self
            .board
            .timers
            .iter()
            .map(|timer| timer.next_event(hclk))
            .chain([self.board.systick.next_event(hclk)])
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
        for &(irq, first, last) in b.part.exti_irqs {
            line(irq, b.exti.pending(first, last));
        }
        for (spec, (usart, _)) in b.part.usarts.iter().zip(&b.usarts) {
            line(spec.irq, usart.line());
        }
        for (spec, timer) in b.part.timers.iter().zip(&b.timers) {
            if spec.update_irq == spec.compare_irq {
                line(spec.update_irq, timer.any_line());
            } else {
                line(spec.update_irq, timer.update_line());
                line(spec.compare_irq, timer.compare_line());
            }
        }
        self.board.pfic.raise(lines);
    }

    fn report_pins(&mut self) {
        let at_us = self.now_us();
        let part = self.board.part;
        for pin in part.pins() {
            let Some(now) = self.board.drive_of(pin) else {
                continue;
            };
            let slot = &mut self.reported[usize::from(pin)];
            if *slot == Some(now) {
                continue;
            }
            *slot = Some(now);
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
        let part = self.board.part;
        let flash = std::mem::take(&mut self.board.flash);
        let ram = std::mem::take(&mut self.board.ram);
        let external = self.board.ports.each_ref().map(|port| port.external);
        let said = std::mem::take(&mut self.board.said);
        self.board = Board::new(part, flash);
        self.board.ram = ram;
        self.board.said = said;
        for (port, levels) in self.board.ports.iter_mut().zip(external) {
            port.external = levels;
        }
        self.hart = Hart::with(part.core);
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
        let part = self.board.part;
        if !part.exists(pin) {
            return;
        }
        let (port, n) = (usize::from(pin / part.width), pin % part.width);
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
        if let Some((usart, _)) = self.board.usarts.first_mut() {
            usart.receive(bytes);
        }
        self.raise_lines();
    }

    /// What has happened since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests;
