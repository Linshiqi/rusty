//! QingKe's SysTick: a counter at HCLK or HCLK/8 — 32 bits on the V2, 64
//! on the V4, whose `CNTH` and `CMPH` hold the top halves — that raises
//! `CNTIF` when it reaches `CMP`, and starts again from zero there when
//! `STRE` says to. ch32-hal's blocking `Delay` is exactly that — `CMP` set to
//! the ticks wanted, the counter cleared, `CNTIF` polled — so this is what
//! every `delay_ms` in a firmware waits on.
//!
//! The counter is not stepped. It is a value and the time it had it, and
//! every read works out where it has got to since; nothing here costs
//! anything per instruction.

const STE: u32 = 1 << 0;
const STIE: u32 = 1 << 1;
const STCLK: u32 = 1 << 2;
const STRE: u32 = 1 << 3;
/// The V4's: count down rather than up.
const MODE: u32 = 1 << 4;
/// The V4's: write one to start the counter again (at zero, counting up).
const INIT: u32 = 1 << 5;
const SWIE: u32 = 1 << 31;

pub struct SysTick {
    /// 64 bits rather than 32.
    wide: bool,
    ctlr: u32,
    cntif: bool,
    cmp: u64,
    /// The counter's value at `at`, in base ticks.
    count: u64,
    at: u64,
}

impl SysTick {
    pub fn new(wide: bool) -> Self {
        Self {
            wide,
            ctlr: 0,
            cntif: false,
            cmp: 0,
            count: 0,
            at: 0,
        }
    }

    fn mask(&self) -> u64 {
        if self.wide {
            u64::MAX
        } else {
            u64::from(u32::MAX)
        }
    }

    /// Whether the firmware asked it to count down, which this does not
    /// model: the machine says so once.
    pub fn counts_down(&self) -> bool {
        self.wide && self.ctlr & MODE != 0
    }

    /// One count in base ticks.
    fn tick(&self, hclk: u64) -> u64 {
        if self.ctlr & STCLK != 0 {
            hclk
        } else {
            hclk * 8
        }
    }

    /// Bring the counter and the flag up to `now`.
    pub fn sync(&mut self, now: u64, hclk: u64) {
        if self.ctlr & STE == 0 {
            self.at = now;
            return;
        }
        let tick = self.tick(hclk);
        let elapsed = now.saturating_sub(self.at) / tick;
        if elapsed == 0 {
            return;
        }
        self.at += elapsed * tick;
        let to_compare = self.cmp.wrapping_sub(self.count) & self.mask();
        // Reaching the compare value from below, including exactly now.
        let reached = to_compare != 0 && elapsed >= to_compare;
        if reached {
            self.cntif = true;
        }
        if reached && self.ctlr & STRE != 0 && self.cmp != 0 {
            // From zero again at the match: every `cmp` counts after it.
            self.count = (elapsed - to_compare) % self.cmp;
        } else {
            self.count = self.count.wrapping_add(elapsed) & self.mask();
        }
    }

    /// When the counter next reaches its compare value, if it is running.
    pub fn next_event(&self, hclk: u64) -> Option<u64> {
        if self.ctlr & STE == 0 {
            return None;
        }
        let to_compare = self.cmp.wrapping_sub(self.count) & self.mask();
        (to_compare != 0).then(|| {
            self.at
                .saturating_add(to_compare.saturating_mul(self.tick(hclk)))
        })
    }

    /// The SysTick interrupt's line (core interrupt 12).
    pub fn line(&self) -> bool {
        self.cntif && self.ctlr & STIE != 0
    }

    /// The software interrupt's line (core interrupt 14).
    pub fn software(&self) -> bool {
        self.ctlr & SWIE != 0
    }

    pub fn read(&mut self, offset: u32, now: u64, hclk: u64) -> u32 {
        self.sync(now, hclk);
        match offset {
            0x00 => self.ctlr,
            0x04 => u32::from(self.cntif),
            0x08 => self.count as u32,
            0x0C if self.wide => (self.count >> 32) as u32,
            0x10 => self.cmp as u32,
            0x14 if self.wide => (self.cmp >> 32) as u32,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32, now: u64, hclk: u64) {
        self.sync(now, hclk);
        match offset {
            0x00 => {
                // From here the counter runs at the new rate (or stops).
                self.at = now;
                if self.wide && value & INIT != 0 {
                    self.count = 0;
                }
                self.ctlr = if self.wide { value & !INIT } else { value };
            }
            // Write zero to clear.
            0x04 => self.cntif &= value & 1 != 0,
            0x08 => {
                self.count = (self.count & !u64::from(u32::MAX)) | u64::from(value);
                self.at = now;
            }
            0x0C if self.wide => {
                self.count = (self.count & u64::from(u32::MAX)) | (u64::from(value) << 32);
                self.at = now;
            }
            0x10 => self.cmp = (self.cmp & !u64::from(u32::MAX)) | u64::from(value),
            0x14 if self.wide => {
                self.cmp = (self.cmp & u64::from(u32::MAX)) | (u64::from(value) << 32);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ch32-hal's `Delay::delay_us`, step for step.
    #[test]
    fn a_delay_sets_its_flag_after_exactly_the_ticks_it_asked_for() {
        let hclk = 6; // 8 MHz in 48 MHz ticks
        let mut tick = SysTick::new(false);
        tick.write(0x04, 0, 0, hclk);
        tick.write(0x10, 8_000, 0, hclk); // 1 ms of HCLK
        tick.write(0x08, 0, 0, hclk);
        tick.write(0x00, STE | STCLK, 0, hclk);
        assert_eq!(tick.read(0x04, 8_000 * hclk - 1, hclk), 0);
        assert_eq!(tick.read(0x04, 8_000 * hclk, hclk), 1);
        assert_eq!(tick.next_event(hclk), None, "at the compare value now");
    }

    #[test]
    fn auto_reload_wraps_to_zero_at_the_match() {
        let hclk = 1;
        let mut tick = SysTick::new(false);
        tick.write(0x10, 100, 0, hclk);
        tick.write(0x00, STE | STCLK | STRE | STIE, 0, hclk);
        assert_eq!(tick.next_event(hclk), Some(100));
        assert_eq!(tick.read(0x08, 130, hclk), 30);
        assert!(tick.line());
        tick.write(0x04, 0, 130, hclk);
        assert!(!tick.line());
        assert_eq!(tick.next_event(hclk), Some(200));
    }

    /// ch32-hal's `Delay` writes only the low halves; on the V4 the high
    /// halves read back zero and the count is the same.
    #[test]
    fn the_v4s_counter_is_sixty_four_bits_and_init_starts_it_over() {
        let hclk = 1;
        let mut tick = SysTick::new(true);
        tick.write(0x10, 0, 0, hclk);
        tick.write(0x14, 1, 0, hclk); // compare at 2^32
        tick.write(0x00, STE | STCLK, 0, hclk);
        let past = (1u64 << 32) + 5;
        assert_eq!(tick.read(0x0C, past, hclk), 1);
        assert_eq!(tick.read(0x08, past, hclk), 5);
        assert_eq!(tick.read(0x04, past, hclk), 1, "passed the compare value");
        tick.write(0x00, STE | STCLK | INIT, past, hclk);
        assert_eq!(tick.read(0x08, past, hclk), 0);
        assert_eq!(
            tick.read(0x00, past, hclk) & INIT,
            0,
            "INIT reads back clear"
        );
    }

    #[test]
    fn hclk_over_eight_counts_eight_times_slower() {
        let hclk = 1;
        let mut tick = SysTick::new(false);
        tick.write(0x00, STE, 0, hclk);
        assert_eq!(tick.read(0x08, 80, hclk), 10);
    }
}
