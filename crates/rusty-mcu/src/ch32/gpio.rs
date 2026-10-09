//! The pins: the ports' configuration and output registers, the remap
//! register that decides which pin a peripheral reaches, and the external
//! interrupt lines a pin's edges raise.
//!
//! Pins are numbered flat, as many to a port as the part's GPIO registers
//! are wide ([`Part::width`]): eight on the CH32V003 — PA1 is 1, PC4 is 20,
//! PD6 is 30 — and twenty-four on the CH32X035, whose ports run to PA23 —
//! PB12 is 36, PC19 is 67. That is how they travel on rusty's pin channel,
//! where a pin has always been a number.

use super::part::{Family, Part};
use super::x035_pins as x035;

/// How a pin is configured, from its four bits of `CFGLR`, `CFGHR` or
/// `CFGXR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// An input: `pull` is `Some(up)` with a pull resistor, `None` floating
    /// or analog.
    Input { pull: Option<bool> },
    /// A general-purpose output at the level `OUTDR` gives it.
    Output(bool),
    /// An output a peripheral drives.
    Alternate,
}

/// One port: up to twenty-four pins, eight to each configuration register.
pub struct Port {
    /// `CFGLR`, `CFGHR` and `CFGXR`: pins 0–7, 8–15 and 16–23.
    pub cfg: [u32; 3],
    pub outdr: u32,
    lckr: u32,
    /// What the outside world drives each pin to, when it drives it.
    pub external: [Option<bool>; 24],
    width: u8,
}

impl Port {
    pub fn new(width: u8) -> Self {
        Self {
            // Every pin a floating input out of reset.
            cfg: [0x4444_4444; 3],
            outdr: 0,
            lckr: 0,
            external: [None; 24],
            width,
        }
    }

    fn mask(&self) -> u32 {
        (1u32 << self.width) - 1
    }

    pub fn role(&self, n: u8) -> Role {
        let register = self.cfg[usize::from(n / 8)];
        let nibble = (register >> (4 * u32::from(n % 8))) & 0xF;
        let (mode, cnf) = (nibble & 0b11, nibble >> 2);
        let odr = self.outdr >> n & 1 != 0;
        match (mode, cnf) {
            (0, 0b10) => Role::Input { pull: Some(odr) },
            (0, _) => Role::Input { pull: None },
            (_, 0b00 | 0b01) => Role::Output(odr),
            _ => Role::Alternate,
        }
    }

    /// The level `INDR` reads for one pin: an output reads back what it
    /// drives, an input what the outside drives or its pull, an alternate
    /// output whatever level its peripheral is at (`driven`).
    pub fn level(&self, n: u8, driven: Option<bool>) -> bool {
        match self.role(n) {
            Role::Output(level) => level,
            Role::Input { pull } => self.external[usize::from(n)].or(pull).unwrap_or(false),
            Role::Alternate => driven.unwrap_or(false),
        }
    }

    pub fn read(&self, offset: u32, indr: impl Fn() -> u32) -> u32 {
        match offset {
            0x00 => self.cfg[0],
            0x04 if self.width > 8 => self.cfg[1],
            0x08 => indr(),
            0x0C => self.outdr,
            0x18 => self.lckr,
            0x1C if self.width > 16 => self.cfg[2],
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        let mask = self.mask();
        match offset {
            0x00 => self.cfg[0] = value,
            0x04 if self.width > 8 => self.cfg[1] = value,
            0x0C => self.outdr = value & mask,
            // Set in the low half, reset in the high; a set wins. Pins 0–15
            // only: the rest are `BSXR`'s.
            0x10 => {
                let reset = (value >> 16) & mask;
                let set = value & 0xFFFF & mask;
                self.outdr = (self.outdr & !reset) | set;
            }
            0x14 => self.outdr &= !(value & mask),
            0x18 => self.lckr = value,
            0x1C if self.width > 16 => self.cfg[2] = value,
            // `BSXR`: pins 16–23, set in the low byte and reset in the
            // third.
            0x20 if self.width > 16 => {
                let reset = ((value >> 16) & 0xFF) << 16;
                let set = (value & 0xFF) << 16;
                self.outdr = (self.outdr & !reset) | set;
            }
            _ => {}
        }
    }
}

/// `AFIO`: which pins the timers and the USARTs reach, and which port each
/// external interrupt line listens to.
pub struct Afio {
    family: Family,
    pub pcfr1: u32,
    exticr: [u32; 2],
    ctlr: u32,
}

/// A timer's four channels' pins and its three complements', `None` for one
/// its remap brings out nowhere.
pub type TimerPins = ([Option<u8>; 4], [Option<u8>; 3]);

impl Afio {
    pub fn new(part: &Part) -> Self {
        Self {
            family: part.family,
            pcfr1: 0,
            exticr: [0; 2],
            ctlr: 0,
        }
    }

    pub fn read(&self, offset: u32) -> u32 {
        match (self.family, offset) {
            (_, 0x04) => self.pcfr1,
            (_, 0x08) => self.exticr[0],
            (Family::X035, 0x0C) => self.exticr[1],
            (Family::X035, 0x18) => self.ctlr,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match (self.family, offset) {
            (_, 0x04) => self.pcfr1 = value,
            (_, 0x08) => self.exticr[0] = value,
            (Family::X035, 0x0C) => self.exticr[1] = value,
            (Family::X035, 0x18) => self.ctlr = value,
            _ => {}
        }
    }

    fn field(&self, at: u32, width: u32) -> usize {
        ((self.pcfr1 >> at) & ((1 << width) - 1)) as usize
    }

    /// Timer `number`'s pins (TIM1 is 1), as its remap places them.
    pub fn timer_pins(&self, number: u8) -> TimerPins {
        match self.family {
            Family::V003 => match number {
                1 => {
                    let (main, comp) = self.v003_tim1_pins();
                    (main.map(Some), comp.map(Some))
                }
                2 => (self.v003_tim2_pins().map(Some), [None; 3]),
                _ => ([None; 4], [None; 3]),
            },
            Family::X035 => {
                let (table, remap) = match number {
                    1 => (x035::TIM1, self.field(15, 3)),
                    2 => (x035::TIM2, self.field(18, 3)),
                    3 => (x035::TIM3, self.field(21, 2)),
                    _ => return ([None; 4], [None; 3]),
                };
                let some = |pin: u8| (pin != x035::NONE).then_some(pin);
                table
                    .get(remap)
                    .map(|(main, comp)| (main.map(some), comp.map(some)))
                    .unwrap_or(([None; 4], [None; 3]))
            }
        }
    }

    /// USART `number`'s TX pin (USART1 is 1), as its remap places it.
    pub fn usart_tx(&self, number: u8) -> Option<u8> {
        match self.family {
            Family::V003 => (number == 1).then(|| self.v003_usart1_tx()),
            Family::X035 => {
                let (table, remap) = match number {
                    1 => (x035::USART1_TX, self.field(5, 2)),
                    2 => (x035::USART2_TX, self.field(7, 3)),
                    3 => (x035::USART3_TX, self.field(10, 2)),
                    4 => (x035::USART4_TX, self.field(12, 3)),
                    _ => return None,
                };
                table.get(remap).copied().filter(|&pin| pin != x035::NONE)
            }
        }
    }

    /// TIM1's channels' pins and their complements' (`CH1N` to `CH3N`) on
    /// the CH32V003, as `TIM1_RM` places them — ch32-data's table.
    fn v003_tim1_pins(&self) -> ([u8; 4], [u8; 3]) {
        const PA1: u8 = 1;
        const PA2: u8 = 2;
        let pc = |n: u8| 16 + n;
        let pd = |n: u8| 24 + n;
        match (self.pcfr1 >> 6) & 0b11 {
            0b01 => ([pc(6), pc(7), pc(0), pd(3)], [pc(3), pc(4), pd(1)]),
            0b11 => ([pc(4), pc(7), pc(5), pd(4)], [pc(3), pd(2), pc(6)]),
            // 0b00 and 0b10 differ only in where ETR is.
            _ => ([pd(2), PA1, pc(3), pc(4)], [pd(0), PA2, pd(1)]),
        }
    }

    /// TIM2's four channels' pins on the CH32V003, as `TIM2_RM` places them.
    fn v003_tim2_pins(&self) -> [u8; 4] {
        let pc = |n: u8| 16 + n;
        let pd = |n: u8| 24 + n;
        match (self.pcfr1 >> 8) & 0b11 {
            0b01 => [pc(5), pc(2), pd(2), pc(1)],
            0b10 => [pc(1), pd(3), pc(0), pd(7)],
            0b11 => [pc(1), pc(7), pd(6), pd(5)],
            _ => [pd(4), pd(3), pc(0), pd(7)],
        }
    }

    /// USART1's TX pin on the CH32V003. Its remap is split across two bits:
    /// `USART1_RM` and `USART1_RM1`.
    fn v003_usart1_tx(&self) -> u8 {
        let remap = ((self.pcfr1 >> 2) & 1) | (((self.pcfr1 >> 21) & 1) << 1);
        match remap {
            0b01 => 24,
            0b10 => 24 + 6,
            0b11 => 16,
            _ => 24 + 5,
        }
    }

    /// The port index (0 A, 1 B, 2 C, 3 D) external interrupt line `line`
    /// listens to: two bits a line, sixteen lines to an `EXTICR`.
    pub fn exti_port(&self, line: u8) -> u8 {
        let register = self.exticr[usize::from(line / 16).min(1)];
        ((register >> (2 * u32::from(line % 16))) & 0b11) as u8
    }
}

/// The external interrupt lines, one per pin number within a port: eight on
/// the CH32V003, twenty-four on the CH32X035 (and two more of its own).
pub struct Exti {
    lines: u32,
    intenr: u32,
    evenr: u32,
    rtenr: u32,
    ftenr: u32,
    intfr: u32,
}

impl Exti {
    pub fn new(part: &Part) -> Self {
        Self {
            lines: part.exti_lines,
            intenr: 0,
            evenr: 0,
            rtenr: 0,
            ftenr: 0,
            intfr: 0,
        }
    }

    fn mask(&self) -> u32 {
        if self.lines >= 32 {
            u32::MAX
        } else {
            (1 << self.lines) - 1
        }
    }

    /// A pin's level changed; raise its line's flag if that edge is wanted.
    pub fn edge(&mut self, line: u8, rising: bool) {
        let enabled = if rising { self.rtenr } else { self.ftenr };
        if enabled >> line & 1 != 0 {
            self.intfr |= 1 << line;
        }
    }

    /// Whether any of lines `first..=last` is flagged and enabled: one of
    /// the `EXTIx_y` interrupts.
    pub fn pending(&self, first: u8, last: u8) -> bool {
        let span = ((1u64 << (last + 1)) - (1u64 << first)) as u32;
        self.intfr & self.intenr & span != 0
    }

    pub fn read(&self, offset: u32) -> u32 {
        match offset {
            0x00 => self.intenr,
            0x04 => self.evenr,
            0x08 => self.rtenr,
            0x0C => self.ftenr,
            0x14 => self.intfr,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        let mask = self.mask();
        match offset {
            0x00 => self.intenr = value & mask,
            0x04 => self.evenr = value & mask,
            0x08 => self.rtenr = value & mask,
            0x0C => self.ftenr = value & mask,
            // A software event raises the flag itself.
            0x10 => self.intfr |= value & mask,
            // Write one to clear.
            0x14 => self.intfr &= !value,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::part::{CH32V003, CH32X035};
    use super::*;

    #[test]
    fn a_pin_is_named_by_its_port() {
        assert_eq!(CH32V003.pin_name(20), "PC4");
        assert_eq!(CH32V003.pin_name(1), "PA1");
        assert_eq!(CH32V003.pin_name(30), "PD6");
        assert!(CH32V003.exists(2) && !CH32V003.exists(0) && !CH32V003.exists(8));
        assert_eq!(CH32X035.pin_name(36), "PB12");
        assert_eq!(CH32X035.pin_name(67), "PC19");
        assert!(CH32X035.exists(23) && CH32X035.exists(62) && !CH32X035.exists(56));
    }

    #[test]
    fn cfglr_decides_input_output_and_alternate() {
        let mut port = Port::new(8);
        assert_eq!(port.role(4), Role::Input { pull: None });
        // PC4: push-pull output at 50 MHz, then alternate push-pull.
        port.write(0x00, 0x0003_0000);
        port.write(0x10, 1 << 4);
        assert_eq!(port.role(4), Role::Output(true));
        port.write(0x10, 1 << (16 + 4));
        assert_eq!(port.role(4), Role::Output(false));
        port.write(0x00, 0x000B_0000);
        assert_eq!(port.role(4), Role::Alternate);
        // An input with a pull: OUTDR says which way.
        port.write(0x00, 0x0008_0000);
        port.write(0x0C, 1 << 4);
        assert_eq!(port.role(4), Role::Input { pull: Some(true) });
        assert!(port.level(4, None));
        port.external[4] = Some(false);
        assert!(!port.level(4, None), "the outside world beats the pull");
    }

    /// A 24-pin port: pins 8–15 are `CFGHR`'s, 16–23 `CFGXR`'s, and
    /// `BSXR` sets and resets the top eight where `BSHR` cannot reach.
    #[test]
    fn a_wide_port_configures_and_drives_all_twenty_four_pins() {
        let mut port = Port::new(24);
        // PB12: output, in CFGHR's fifth nibble.
        port.write(0x04, 0x0003_0000 | (0x4444_4444 & !0x000F_0000));
        port.write(0x10, 1 << 12);
        assert_eq!(port.role(12), Role::Output(true));
        // PA19: output, in CFGXR's fourth nibble.
        port.write(0x1C, 0x0000_3000 | (0x4444_4444 & !0x0000_F000));
        port.write(0x20, 1 << 3);
        assert_eq!(port.role(19), Role::Output(true));
        port.write(0x20, 1 << (16 + 3));
        assert_eq!(port.role(19), Role::Output(false));
        port.write(0x10, 1 << (16 + 12));
        assert_eq!(port.outdr, 0);
        assert_eq!(port.read(0x1C, || 0) & 0xF000, 0x3000);
    }

    #[test]
    fn the_v003_remaps_put_channel_four_where_ch32_data_says() {
        let mut afio = Afio::new(&CH32V003);
        assert_eq!(afio.timer_pins(1).0[3], Some(20), "PC4 by default");
        afio.write(0x04, 0b01 << 6);
        assert_eq!(afio.timer_pins(1).0[3], Some(27), "PD3");
        afio.write(0x04, 0b11 << 6);
        assert_eq!(afio.timer_pins(1).0[3], Some(28), "PD4");
        afio.write(0x04, 0b11 << 8);
        assert_eq!(afio.timer_pins(2).0, [17, 23, 30, 29].map(Some));
        afio.write(0x04, 1 << 21);
        assert_eq!(afio.usart_tx(1), Some(30), "TX on PD6 at remap 0b10");
        assert_eq!(afio.usart_tx(2), None, "the V003 has one USART");
    }

    /// The X035's tables are ch32-data's, read through `PCFR1`'s fields at
    /// the offsets ch32-metapac gives them.
    #[test]
    fn the_x035_remaps_read_pcfr1_where_ch32_metapac_puts_each_field() {
        let mut afio = Afio::new(&CH32X035);
        assert_eq!(afio.timer_pins(1).0[3], Some(64), "PC16 by default");
        // TIM1_RM = 2: ch32-hal's PWM example's PB12.
        afio.write(0x04, 2 << 15);
        assert_eq!(afio.timer_pins(1).0[3], Some(36));
        afio.write(0x04, 4 << 18);
        assert_eq!(
            afio.timer_pins(2).0,
            [40, 41, 42, 43].map(Some),
            "PB16..PB19"
        );
        assert_eq!(afio.timer_pins(3).0[2], None, "TIM3 has two channels");
        afio.write(0x04, 3 << 5);
        assert_eq!(afio.usart_tx(1), Some(7), "PA7");
        // Line 20 is EXTICR2's fifth pair.
        afio.write(0x0C, 0b10 << 8);
        assert_eq!(afio.exti_port(20), 2);
        assert_eq!(afio.exti_port(4), 0);
    }

    #[test]
    fn exti_groups_answer_for_their_own_lines() {
        let mut exti = Exti::new(&CH32X035);
        exti.write(0x00, 1 << 17);
        exti.write(0x08, 1 << 17);
        exti.edge(17, true);
        assert!(exti.pending(16, 25));
        assert!(!exti.pending(0, 7) && !exti.pending(8, 15));
        exti.write(0x14, 1 << 17);
        assert!(!exti.pending(16, 25));
    }
}
