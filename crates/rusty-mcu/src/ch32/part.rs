//! What differs between the parts this machine is, as data: the core, the
//! memories, the ports and how wide they are, which peripherals sit where
//! and which interrupt each raises. Everything the machine does is the same
//! for every part; where a part's own register layout differs in kind — the
//! clock tree, the remap register — the peripheral asks [`Family`].

use crate::cpu::Core;

/// Which register layouts a part's peripherals follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// The CH32V003: ch32-metapac's `rcc_v003`, `afio_v003`, `gpio_v0`.
    V003,
    /// The CH32X035: `rcc_x0`, `afio_x0`, `gpio_x0` (24 pins a port).
    X035,
}

/// A timer the part has: where it is, which number it answers to in the
/// remap register, and its interrupts.
pub struct TimerSpec {
    pub name: &'static str,
    pub base: u32,
    pub number: u8,
    /// An advanced timer: its outputs wait for `MOE`, and it has
    /// complementary outputs.
    pub advanced: bool,
    /// The update interrupt, and the capture/compare one. A timer with one
    /// interrupt for everything names it twice.
    pub update_irq: u32,
    pub compare_irq: u32,
}

/// A USART the part has.
pub struct UsartSpec {
    pub base: u32,
    pub number: u8,
    pub irq: u32,
}

pub struct Part {
    pub family: Family,
    /// As a reader would write it: `CH32V003`.
    pub name: &'static str,
    pub core: Core,
    pub flash: usize,
    pub ram: usize,
    /// Pins a port in the flat numbering: the width of the part's GPIO
    /// registers.
    pub width: u8,
    /// The pins the die has, by port index (A 0, B 1, C 2, D 3), as a mask
    /// of pin numbers within the port.
    pub die: &'static [(u8, u32)],
    /// How many external interrupt lines there are.
    pub exti_lines: u32,
    /// The `EXTIx_y` interrupts: number, first line, last line.
    pub exti_irqs: &'static [(u32, u8, u8)],
    pub timers: &'static [TimerSpec],
    pub usarts: &'static [UsartSpec],
    /// Every peripheral by its 1 KB block, for naming an access.
    pub blocks: &'static [(u32, &'static str)],
    /// Peripherals whose registers are kept and nothing more, and what the
    /// firmware will find missing.
    pub unmodelled: &'static [(&'static str, &'static str)],
    /// The last address of the peripheral region.
    pub mmio_end: u32,
}

impl Part {
    /// The part a chip id names: `ch32v003j4m6` and its packages are the
    /// CH32V003, `ch32x035f8u6` the CH32X035. Nothing else, rather than
    /// the nearest thing.
    pub fn for_chip(chip: &str) -> Option<&'static Part> {
        if chip.starts_with("ch32v003") {
            Some(&CH32V003)
        } else if chip.starts_with("ch32x035") {
            Some(&CH32X035)
        } else {
            None
        }
    }

    /// Whether the die has pin `pin`.
    pub fn exists(&self, pin: u8) -> bool {
        let (port, n) = (pin / self.width, pin % self.width);
        self.die
            .iter()
            .any(|&(index, mask)| index == port && mask >> n & 1 != 0)
    }

    /// `PC4` for 20 on a CH32V003.
    pub fn pin_name(&self, pin: u8) -> String {
        let port = (b'A' + pin / self.width) as char;
        format!("P{port}{}", pin % self.width)
    }

    /// Every pin number the die has, in order.
    pub fn pins(&self) -> impl Iterator<Item = u8> + '_ {
        (0..=u8::MAX).filter(|&pin| self.exists(pin))
    }

    /// One past the highest pin number.
    pub fn pin_count(&self) -> usize {
        4 * usize::from(self.width)
    }
}

pub static CH32V003: Part = Part {
    family: Family::V003,
    name: "CH32V003",
    core: Core::V2A,
    flash: 16 * 1024,
    ram: 2 * 1024,
    width: 8,
    // PA1 and PA2, PC0..PC7, PD0..PD7; there is no port B.
    die: &[(0, 0b0110), (2, 0xFF), (3, 0xFF)],
    exti_lines: 10,
    exti_irqs: &[(20, 0, 7)],
    timers: &[
        TimerSpec {
            name: "TIM1",
            base: 0x4001_2C00,
            number: 1,
            advanced: true,
            update_irq: 35,
            compare_irq: 37,
        },
        TimerSpec {
            name: "TIM2",
            base: 0x4000_0000,
            number: 2,
            advanced: false,
            update_irq: 38,
            compare_irq: 38,
        },
    ],
    usarts: &[UsartSpec {
        base: 0x4001_3800,
        number: 1,
        irq: 32,
    }],
    blocks: &[
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
    ],
    unmodelled: &[
        WWDG,
        IWDG,
        I2C1,
        PWR,
        ADC1,
        SPI1,
        DMA1,
        (
            "EXTEND",
            "the extended configuration and the op-amp do nothing",
        ),
    ],
    mmio_end: 0x4002_3FFF,
};

pub static CH32X035: Part = Part {
    family: Family::X035,
    name: "CH32X035",
    core: Core::V4C,
    flash: 62 * 1024,
    ram: 20 * 1024,
    width: 24,
    // PA0..PA23, PB0..PB21, PC0..PC7 and PC14..PC19: ch32-data's pins for
    // the die. A package bonds out fewer; the catalogue says which.
    die: &[(0, 0x00FF_FFFF), (1, 0x003F_FFFF), (2, 0x000F_C0FF)],
    exti_lines: 26,
    exti_irqs: &[(20, 0, 7), (40, 8, 15), (41, 16, 25)],
    timers: &[
        TimerSpec {
            name: "TIM1",
            base: 0x4001_2C00,
            number: 1,
            advanced: true,
            update_irq: 35,
            compare_irq: 37,
        },
        TimerSpec {
            name: "TIM2",
            base: 0x4000_0000,
            number: 2,
            // An advanced timer on this part: complementary outputs, a
            // break input and `MOE`.
            advanced: true,
            update_irq: 38,
            compare_irq: 51,
        },
        TimerSpec {
            name: "TIM3",
            base: 0x4000_0400,
            number: 3,
            advanced: false,
            update_irq: 54,
            compare_irq: 54,
        },
    ],
    usarts: &[
        UsartSpec {
            base: 0x4001_3800,
            number: 1,
            irq: 32,
        },
        UsartSpec {
            base: 0x4000_4400,
            number: 2,
            irq: 39,
        },
        UsartSpec {
            base: 0x4000_4800,
            number: 3,
            irq: 42,
        },
        UsartSpec {
            base: 0x4000_4C00,
            number: 4,
            irq: 43,
        },
    ],
    blocks: &[
        (0x4000_0000, "TIM2"),
        (0x4000_0400, "TIM3"),
        (0x4000_2C00, "WWDG"),
        (0x4000_3000, "IWDG"),
        (0x4000_4400, "USART2"),
        (0x4000_4800, "USART3"),
        (0x4000_4C00, "USART4"),
        (0x4000_5400, "I2C1"),
        (0x4000_7000, "PWR"),
        (0x4001_0000, "AFIO"),
        (0x4001_0400, "EXTI"),
        (0x4001_0800, "GPIOA"),
        (0x4001_0C00, "GPIOB"),
        (0x4001_1000, "GPIOC"),
        (0x4001_2400, "ADC1"),
        (0x4001_2C00, "TIM1"),
        (0x4001_3000, "SPI1"),
        (0x4001_3800, "USART1"),
        (0x4002_0000, "DMA1"),
        (0x4002_1000, "RCC"),
        (0x4002_2000, "FLASH"),
        (0x4002_3400, "USBFS"),
        (0x4002_6000, "OPA"),
        (0x4002_6400, "AWU"),
        (0x4002_6C00, "PIOC"),
        (0x4002_7000, "USBPD"),
    ],
    unmodelled: &[
        WWDG,
        IWDG,
        I2C1,
        PWR,
        ADC1,
        SPI1,
        DMA1,
        (
            "USBFS",
            "the USB device never sees a host, so nothing enumerates",
        ),
        (
            "USBPD",
            "no USB PD partner answers, so no contract is reached",
        ),
        (
            "OPA",
            "the op-amps and the comparators do nothing, so an output reads back what was written",
        ),
        ("AWU", "the auto-wakeup never wakes the part"),
        ("PIOC", "the programmable I/O controller runs no program"),
    ],
    mmio_end: 0x4002_73FF,
};

const WWDG: (&str, &str) = ("WWDG", "the window watchdog never resets the part");
const IWDG: (&str, &str) = ("IWDG", "the independent watchdog never resets the part");
const I2C1: (&str, &str) = (
    "I2C1",
    "no transfer completes, so a driver waits on its first start",
);
const PWR: (&str, &str) = ("PWR", "sleep and the voltage detector do nothing");
const ADC1: (&str, &str) = (
    "ADC1",
    "no conversion completes, so a read waits on its end-of-conversion flag",
);
const SPI1: (&str, &str) = (
    "SPI1",
    "no transfer completes, so a driver waits on its first byte",
);
const DMA1: (&str, &str) = ("DMA1", "no transfer happens");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chip_id_names_its_part_and_nothing_else_does() {
        assert_eq!(
            Part::for_chip("ch32v003f4p6").map(|p| p.name),
            Some("CH32V003")
        );
        assert_eq!(
            Part::for_chip("ch32x035f8u6").map(|p| p.name),
            Some("CH32X035")
        );
        assert!(Part::for_chip("ch32v203c8t6").is_none());
        assert!(Part::for_chip("esp32c3").is_none());
    }

    /// Nineteen pins on the F8U6, every one of them on the die.
    #[test]
    fn the_x035_die_has_every_pin_the_f8u6_bonds_out() {
        let f8u6 = [
            0, 1, 2, 3, 4, 5, 6, 7, 24, 25, 27, 35, 36, 62, 63, 64, 65, 66, 67,
        ];
        assert!(f8u6.iter().all(|&pin| CH32X035.exists(pin)));
        assert_eq!(CH32X035.pins().count(), 24 + 22 + 14);
    }
}
