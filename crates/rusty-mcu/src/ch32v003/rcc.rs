//! The clock tree, as far as anything else here can see it: which oscillator
//! the system runs from and how far the AHB divides it. Every timer, SysTick
//! and the hart itself count in HCLK, so HCLK is the one number this exists
//! to answer — in ticks of the machine's 48 MHz time base.

pub use crate::BASE_HZ;

const HSION: u32 = 1 << 0;
const HSIRDY: u32 = 1 << 1;
const HSEON: u32 = 1 << 16;
const HSERDY: u32 = 1 << 17;
const PLLON: u32 = 1 << 24;
const PLLRDY: u32 = 1 << 25;

pub struct Rcc {
    ctlr: u32,
    cfgr0: u32,
    intr: u32,
    apb2prstr: u32,
    apb1prstr: u32,
    ahbpcenr: u32,
    apb2pcenr: u32,
    apb1pcenr: u32,
    rstsckr: u32,
    /// Set the first time the firmware turns on HSE, which this board does
    /// not have; the machine says so once.
    pub wants_crystal: bool,
}

impl Default for Rcc {
    fn default() -> Self {
        Self {
            // HSI on and ready, trimmed to the middle.
            ctlr: HSION | HSIRDY | (16 << 3),
            // HPRE = DIV3: the part comes out of reset at 24 / 3 = 8 MHz,
            // which ch32-hal's default configuration keeps.
            cfgr0: 0b0010 << 4,
            intr: 0,
            apb2prstr: 0,
            apb1prstr: 0,
            ahbpcenr: 0x14,
            apb2pcenr: 0,
            apb1pcenr: 0,
            // Power-on reset flag.
            rstsckr: 1 << 27,
            wants_crystal: false,
        }
    }
}

impl Rcc {
    /// Ready bits follow their enables at once — except HSE's, because the
    /// board has no crystal. A firmware that waits for one waits for ever,
    /// as it would on a J4M6 board without one, and the machine names why.
    fn ctlr(&self) -> u32 {
        let mut value = self.ctlr & !(HSIRDY | HSERDY | PLLRDY);
        if self.ctlr & HSION != 0 {
            value |= HSIRDY;
        }
        if self.ctlr & PLLON != 0 && self.pll_source_ready() {
            value |= PLLRDY;
        }
        value
    }

    fn pll_source_ready(&self) -> bool {
        // PLLSRC 0 is HSI; anything else is HSE, which never comes up.
        (self.cfgr0 >> 16) & 0b11 == 0
    }

    /// The switch the firmware asked for, as far as it can happen.
    fn sws(&self) -> u32 {
        match self.cfgr0 & 0b11 {
            0b10 if self.ctlr() & PLLRDY != 0 => 0b10,
            _ => 0b00,
        }
    }

    pub fn read(&self, offset: u32) -> u32 {
        match offset {
            0x00 => self.ctlr(),
            0x04 => (self.cfgr0 & !0b1100) | (self.sws() << 2),
            0x08 => self.intr,
            0x0C => self.apb2prstr,
            0x10 => self.apb1prstr,
            0x14 => self.ahbpcenr,
            0x18 => self.apb2pcenr,
            0x1C => self.apb1pcenr,
            0x24 => self.rstsckr,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x00 => {
                if value & HSEON != 0 {
                    self.wants_crystal = true;
                }
                self.ctlr = value;
            }
            0x04 => self.cfgr0 = value,
            // Flags clear by writing their clear bits; nothing here raises
            // one, so only the enables are kept.
            0x08 => self.intr = value & 0x0000_1F00,
            0x0C => self.apb2prstr = value,
            0x10 => self.apb1prstr = value,
            0x14 => self.ahbpcenr = value,
            0x18 => self.apb2pcenr = value,
            0x1C => self.apb1pcenr = value,
            // RMVF clears the reset flags.
            0x24 => {
                self.rstsckr = if value & (1 << 24) != 0 {
                    value & 0xFF
                } else {
                    (self.rstsckr & 0xFF00_0000) | (value & 0xFF)
                }
            }
            _ => {}
        }
    }

    /// SYSCLK in MHz: 24 from HSI, 48 from the PLL doubling it.
    fn sysclk_mhz(&self) -> u64 {
        if self.sws() == 0b10 { 48 } else { 24 }
    }

    /// One HCLK period in base ticks.
    pub fn hclk_period(&self) -> u64 {
        const DIV: [u64; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 2, 4, 8, 16, 32, 64, 128, 256];
        let div = DIV[((self.cfgr0 >> 4) & 0xF) as usize];
        (48 / self.sysclk_mhz()) * div
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_reset_the_part_runs_at_eight_megahertz() {
        let rcc = Rcc::default();
        assert_eq!(BASE_HZ / rcc.hclk_period(), 8_000_000);
    }

    #[test]
    fn the_pll_from_hsi_with_no_divider_is_forty_eight() {
        let mut rcc = Rcc::default();
        rcc.write(0x04, 0); // HPRE DIV1, PLLSRC HSI, SW HSI
        rcc.write(0x00, rcc.read(0x00) | PLLON);
        assert_ne!(rcc.read(0x00) & PLLRDY, 0);
        rcc.write(0x04, 0b10);
        assert_eq!((rcc.read(0x04) >> 2) & 0b11, 0b10, "SWS follows");
        assert_eq!(BASE_HZ / rcc.hclk_period(), 48_000_000);
    }

    #[test]
    fn hse_never_comes_ready_and_says_it_was_wanted() {
        let mut rcc = Rcc::default();
        rcc.write(0x00, rcc.read(0x00) | HSEON);
        assert_eq!(rcc.read(0x00) & HSERDY, 0);
        assert!(rcc.wants_crystal);
        rcc.write(0x04, 0b01);
        assert_eq!((rcc.read(0x04) >> 2) & 0b11, 0, "still on HSI");
    }
}
