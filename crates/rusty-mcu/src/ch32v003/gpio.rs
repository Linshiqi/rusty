//! The pins: three ports' configuration and output registers, the remap
//! register that decides which pin a peripheral reaches, and the external
//! interrupt lines a pin's edges raise.
//!
//! Pins are numbered flat, eight to a port — PA1 is 1, PC4 is 20, PD6 is 30
//! — which is how they travel on rusty's pin channel, where a pin has always
//! been a number.

/// Port A, C and D's index in the flat numbering; there is no port B.
pub const PORTS: [(char, u8); 3] = [('A', 0), ('C', 2), ('D', 3)];

/// The pins the die has: PA1, PA2, PC0..PC7, PD0..PD7.
pub fn exists(pin: u8) -> bool {
    matches!(pin, 1 | 2 | 16..=31)
}

/// `PC4` for 20.
pub fn name(pin: u8) -> String {
    let port = PORTS
        .iter()
        .find(|(_, index)| *index == pin / 8)
        .map_or('?', |(letter, _)| *letter);
    format!("P{port}{}", pin % 8)
}

/// How a pin is configured, from its four bits of `CFGLR`.
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

#[derive(Default)]
pub struct Port {
    pub cfglr: u32,
    pub outdr: u32,
    lckr: u32,
    /// What the outside world drives each pin to, when it drives it.
    pub external: [Option<bool>; 8],
}

impl Port {
    pub fn new() -> Self {
        Self {
            // Every pin a floating input out of reset.
            cfglr: 0x4444_4444,
            ..Self::default()
        }
    }

    pub fn role(&self, n: u8) -> Role {
        let nibble = (self.cfglr >> (4 * u32::from(n))) & 0xF;
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
            0x00 => self.cfglr,
            0x08 => indr(),
            0x0C => self.outdr,
            0x18 => self.lckr,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x00 => self.cfglr = value,
            0x0C => self.outdr = value & 0xFF,
            // Set in the low half, reset in the high; a set wins.
            0x10 => {
                let reset = (value >> 16) & 0xFF;
                let set = value & 0xFF;
                self.outdr = (self.outdr & !reset) | set;
            }
            0x14 => self.outdr &= !(value & 0xFF),
            0x18 => self.lckr = value,
            _ => {}
        }
    }
}

/// `AFIO`: which pins TIM1, TIM2 and USART1 reach, and which port each
/// external interrupt line listens to.
#[derive(Default)]
pub struct Afio {
    pub pcfr1: u32,
    pub exticr: u32,
}

impl Afio {
    pub fn read(&self, offset: u32) -> u32 {
        match offset {
            0x04 => self.pcfr1,
            0x08 => self.exticr,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x04 => self.pcfr1 = value,
            0x08 => self.exticr = value,
            _ => {}
        }
    }

    /// TIM1's channels' pins and their complements' (`CH1N` to `CH3N`), as
    /// `TIM1_RM` places them — ch32-data's table for the CH32V003.
    pub fn tim1_pins(&self) -> ([u8; 4], [u8; 3]) {
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

    /// TIM2's four channels' pins, as `TIM2_RM` places them.
    pub fn tim2_pins(&self) -> [u8; 4] {
        let pc = |n: u8| 16 + n;
        let pd = |n: u8| 24 + n;
        match (self.pcfr1 >> 8) & 0b11 {
            0b01 => [pc(5), pc(2), pd(2), pc(1)],
            0b10 => [pc(1), pd(3), pc(0), pd(7)],
            0b11 => [pc(1), pc(7), pd(6), pd(5)],
            _ => [pd(4), pd(3), pc(0), pd(7)],
        }
    }

    /// USART1's TX pin. Its remap is split across two bits: `USART1_RM` and
    /// `USART1_RM1`.
    pub fn usart1_tx(&self) -> u8 {
        let remap = ((self.pcfr1 >> 2) & 1) | (((self.pcfr1 >> 21) & 1) << 1);
        match remap {
            0b01 => 24,
            0b10 => 24 + 6,
            0b11 => 16,
            _ => 24 + 5,
        }
    }

    /// The port index (0 A, 2 C, 3 D) external interrupt line `line` listens
    /// to.
    pub fn exti_port(&self, line: u8) -> u8 {
        match (self.exticr >> (2 * u32::from(line))) & 0b11 {
            0b10 => 2,
            0b11 => 3,
            _ => 0,
        }
    }
}

/// The eight external interrupt lines, one per pin number within a port.
#[derive(Default)]
pub struct Exti {
    intenr: u32,
    evenr: u32,
    rtenr: u32,
    ftenr: u32,
    intfr: u32,
}

impl Exti {
    /// A pin's level changed; raise its line's flag if that edge is wanted.
    pub fn edge(&mut self, line: u8, rising: bool) {
        let enabled = if rising { self.rtenr } else { self.ftenr };
        if enabled >> line & 1 != 0 {
            self.intfr |= 1 << line;
        }
    }

    /// `EXTI7_0`'s interrupt line.
    pub fn line(&self) -> bool {
        self.intfr & self.intenr & 0xFF != 0
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
        match offset {
            0x00 => self.intenr = value,
            0x04 => self.evenr = value,
            0x08 => self.rtenr = value,
            0x0C => self.ftenr = value,
            // A software event raises the flag itself.
            0x10 => self.intfr |= value & 0xFF,
            // Write one to clear.
            0x14 => self.intfr &= !value,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_is_named_by_its_port() {
        assert_eq!(name(20), "PC4");
        assert_eq!(name(1), "PA1");
        assert_eq!(name(30), "PD6");
        assert!(exists(2) && !exists(0) && !exists(8));
    }

    #[test]
    fn cfglr_decides_input_output_and_alternate() {
        let mut port = Port::new();
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

    #[test]
    fn the_remaps_put_channel_four_where_ch32_data_says() {
        let mut afio = Afio::default();
        assert_eq!(afio.tim1_pins().0[3], 20, "PC4 by default");
        afio.write(0x04, 0b01 << 6);
        assert_eq!(afio.tim1_pins().0[3], 27, "PD3");
        afio.write(0x04, 0b11 << 6);
        assert_eq!(afio.tim1_pins().0[3], 28, "PD4");
        afio.write(0x04, 0b11 << 8);
        assert_eq!(afio.tim2_pins(), [17, 23, 30, 29]);
        afio.write(0x04, 1 << 21);
        assert_eq!(afio.usart1_tx(), 30, "TX on PD6 at remap 0b10");
    }
}
