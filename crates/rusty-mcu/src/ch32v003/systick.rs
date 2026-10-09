//! QingKe V2's SysTick: a 32-bit counter at HCLK or HCLK/8 that raises
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
const SWIE: u32 = 1 << 31;

#[derive(Default)]
pub struct SysTick {
    ctlr: u32,
    cntif: bool,
    cmp: u32,
    /// The counter's value at `at`, in base ticks.
    count: u32,
    at: u64,
}

impl SysTick {
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
        let to_compare = u64::from(self.cmp.wrapping_sub(self.count));
        // Reaching the compare value from below, including exactly now.
        let reached = to_compare != 0 && elapsed >= to_compare;
        if reached {
            self.cntif = true;
        }
        if reached && self.ctlr & STRE != 0 && self.cmp != 0 {
            // From zero again at the match: every `cmp` counts after it.
            let after = (elapsed - to_compare) % u64::from(self.cmp);
            self.count = after as u32;
        } else {
            self.count = self.count.wrapping_add(elapsed as u32);
        }
    }

    /// When the counter next reaches its compare value, if it is running.
    pub fn next_event(&self, hclk: u64) -> Option<u64> {
        if self.ctlr & STE == 0 {
            return None;
        }
        let to_compare = u64::from(self.cmp.wrapping_sub(self.count));
        (to_compare != 0).then(|| self.at + to_compare * self.tick(hclk))
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
            0x08 => self.count,
            0x10 => self.cmp,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32, now: u64, hclk: u64) {
        self.sync(now, hclk);
        match offset {
            0x00 => {
                // From here the counter runs at the new rate (or stops).
                self.at = now;
                self.ctlr = value;
            }
            // Write zero to clear.
            0x04 => self.cntif &= value & 1 != 0,
            0x08 => {
                self.count = value;
                self.at = now;
            }
            0x10 => self.cmp = value,
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
        let mut tick = SysTick::default();
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
        let mut tick = SysTick::default();
        tick.write(0x10, 100, 0, hclk);
        tick.write(0x00, STE | STCLK | STRE | STIE, 0, hclk);
        assert_eq!(tick.next_event(hclk), Some(100));
        assert_eq!(tick.read(0x08, 130, hclk), 30);
        assert!(tick.line());
        tick.write(0x04, 0, 130, hclk);
        assert!(!tick.line());
        assert_eq!(tick.next_event(hclk), Some(200));
    }

    #[test]
    fn hclk_over_eight_counts_eight_times_slower() {
        let hclk = 1;
        let mut tick = SysTick::default();
        tick.write(0x00, STE, 0, hclk);
        assert_eq!(tick.read(0x08, 80, hclk), 10);
    }
}
