//! TIM1 (advanced) and TIM2 (general-purpose): a prescaled counter running
//! to its reload value, four compare channels, and what those channels put
//! on their pins.
//!
//! **A PWM output is reported as what it is — a duty and a frequency — not
//! as edges.** At 1 kHz an edge-by-edge account is two thousand lines a
//! second for a lamp nobody can see flicker; the board reads a duty and
//! draws the lamp as bright as its average current, which is what the eye
//! does with the real one.
//!
//! The counter is a value and the time it had it, like SysTick's: nothing
//! is stepped per instruction. Edge-aligned up-counting is exact; down and
//! centre-aligned counting give the right duty and frequency on the pins
//! and an approximate counter, said once by the machine.

/// What a timer channel does to its pin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drive {
    /// A fixed level: a forced output, or PWM with the counter stopped.
    Level(bool),
    /// High for `duty` of every period, `hz` periods a second.
    Pwm { duty: f64, hz: f64 },
}

impl Drive {
    fn inverted(self) -> Self {
        match self {
            Drive::Level(level) => Drive::Level(!level),
            Drive::Pwm { duty, hz } => Drive::Pwm {
                duty: 1.0 - duty,
                hz,
            },
        }
    }
}

const CEN: u32 = 1 << 0;
const UDIS: u32 = 1 << 1;
const URS: u32 = 1 << 2;
const OPM: u32 = 1 << 3;
const DIR: u32 = 1 << 4;
const CMS: u32 = 0b11 << 5;
const ARPE: u32 = 1 << 7;
const UIF: u32 = 1 << 0;
const MOE: u32 = 1 << 15;

#[derive(Default)]
pub struct Timer {
    /// TIM1: break, complementary outputs and the main output enable.
    advanced: bool,
    ctlr1: u32,
    ctlr2: u32,
    smcfgr: u32,
    dmaintenr: u32,
    intfr: u32,
    chctlr: [u32; 2],
    ccer: u32,
    psc: u32,
    arr: u32,
    rptcr: u32,
    ccr: [u32; 4],
    bdtr: u32,
    dmacfgr: u32,
    dmaadr: u32,
    /// The values in force, which the preloaded registers reach only at an
    /// update event.
    live_psc: u32,
    live_arr: u32,
    live_ccr: [u32; 4],
    /// The counter at `at`.
    count: u32,
    at: u64,
    /// An update event loaded values that change what the pins show, and
    /// nobody has asked yet. Kept here rather than only returned, because a
    /// register *read* brings the counter up to date too, and the update it
    /// finds is a change of duty all the same.
    changed: bool,
}

impl Timer {
    pub fn new(advanced: bool) -> Self {
        Self {
            advanced,
            arr: 0xFFFF,
            live_arr: 0xFFFF,
            ..Self::default()
        }
    }

    /// Counting some way other than up from zero, which is where the counter
    /// read back is approximate.
    pub fn counts_unusually(&self) -> bool {
        self.ctlr1 & (DIR | CMS) != 0
    }

    fn running(&self) -> bool {
        self.ctlr1 & CEN != 0
    }

    fn tick(&self, hclk: u64) -> u64 {
        hclk * (u64::from(self.live_psc) + 1)
    }

    fn period(&self) -> u64 {
        u64::from(self.live_arr) + 1
    }

    /// Load the preloaded registers, as an update event does.
    fn update_event(&mut self) {
        self.live_psc = self.psc;
        self.live_arr = self.arr;
        for channel in 0..4 {
            self.live_ccr[channel] = self.ccr[channel];
        }
    }

    fn preloads_compare(&self, channel: usize) -> bool {
        self.chctlr[channel / 2] >> (3 + 8 * (channel % 2)) & 1 != 0
    }

    fn compare_output(&self, channel: usize) -> bool {
        self.chctlr[channel / 2] >> (8 * (channel % 2)) & 0b11 == 0
    }

    /// Bring the counter and the flags up to `now`. True when an update
    /// event loaded new values — the pins may then read differently.
    pub fn sync(&mut self, now: u64, hclk: u64) -> bool {
        if !self.running() {
            self.at = now;
            return false;
        }
        let tick = self.tick(hclk);
        let elapsed = now.saturating_sub(self.at) / tick;
        if elapsed == 0 {
            return false;
        }
        self.at += elapsed * tick;
        let period = self.period();
        let from = u64::from(self.count);
        let to = from + elapsed;
        for channel in 0..4 {
            let target = u64::from(self.live_ccr[channel]);
            if self.compare_output(channel) && target < period && passes(from, to, target, period) {
                self.intfr |= 1 << (1 + channel);
            }
        }
        if to >= period {
            if self.ctlr1 & UDIS == 0 {
                self.intfr |= UIF;
                let before = (self.live_psc, self.live_arr, self.live_ccr);
                self.update_event();
                if self.ctlr1 & OPM != 0 {
                    self.ctlr1 &= !CEN;
                    self.count = 0;
                    self.changed = true;
                    return true;
                }
                self.count = (to % period) as u32;
                let changed = before != (self.live_psc, self.live_arr, self.live_ccr);
                self.changed |= changed;
                return changed;
            }
            self.count = (to % period) as u32;
        } else {
            self.count = to as u32;
        }
        false
    }

    /// Whether an update event changed what the pins show since this was
    /// last asked — whichever access brought the counter up to date.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// When the counter next overflows or meets a compare value.
    pub fn next_event(&self, hclk: u64) -> Option<u64> {
        if !self.running() {
            return None;
        }
        let period = self.period();
        let count = u64::from(self.count);
        let mut ticks = period - count.min(period - 1);
        for channel in 0..4 {
            let target = u64::from(self.live_ccr[channel]);
            if self.compare_output(channel) && target < period && target > count {
                ticks = ticks.min(target - count);
            }
        }
        Some(self.at + ticks * self.tick(hclk))
    }

    /// The update interrupt's line.
    pub fn update_line(&self) -> bool {
        self.intfr & self.dmaintenr & UIF != 0
    }

    /// The capture/compare interrupt's line.
    pub fn compare_line(&self) -> bool {
        self.intfr & self.dmaintenr & 0b1_1110 != 0
    }

    /// Everything TIM2's one interrupt carries: update, compare, trigger.
    pub fn any_line(&self) -> bool {
        self.intfr & self.dmaintenr & 0b101_1111 != 0
    }

    pub fn read(&mut self, offset: u32, now: u64, hclk: u64) -> u32 {
        self.sync(now, hclk);
        match offset {
            0x00 => self.ctlr1,
            0x04 => self.ctlr2,
            0x08 => self.smcfgr,
            0x0C => self.dmaintenr,
            0x10 => self.intfr,
            0x18 => self.chctlr[0],
            0x1C => self.chctlr[1],
            0x20 => self.ccer,
            0x24 => self.count,
            0x28 => self.psc,
            0x2C => self.arr,
            0x30 if self.advanced => self.rptcr,
            0x34 => self.ccr[0],
            0x38 => self.ccr[1],
            0x3C => self.ccr[2],
            0x40 => self.ccr[3],
            0x44 if self.advanced => self.bdtr,
            0x48 => self.dmacfgr,
            0x4C => self.dmaadr,
            _ => 0,
        }
    }

    /// A register write, at `now`. True when what the pins show may have
    /// changed.
    pub fn write(&mut self, offset: u32, value: u32, now: u64, hclk: u64) -> bool {
        self.sync(now, hclk);
        match offset {
            0x00 => {
                if value & CEN != 0 && !self.running() {
                    self.at = now;
                }
                self.ctlr1 = value;
                if value & ARPE == 0 {
                    self.live_arr = self.arr;
                }
            }
            0x04 => self.ctlr2 = value,
            0x08 => self.smcfgr = value,
            0x0C => self.dmaintenr = value,
            // Flags clear by writing zero to them.
            0x10 => self.intfr &= value,
            0x14 => {
                if value & 1 != 0 {
                    self.count = 0;
                    self.at = now;
                    self.update_event();
                    if self.ctlr1 & URS == 0 {
                        self.intfr |= UIF;
                    }
                }
                self.intfr |= value & 0b1_1110;
            }
            0x18 => self.chctlr[0] = value,
            0x1C => self.chctlr[1] = value,
            0x20 => self.ccer = value,
            0x24 => {
                self.count = value & 0xFFFF;
                self.at = now;
            }
            0x28 => self.psc = value & 0xFFFF,
            0x2C => {
                self.arr = value & 0xFFFF;
                if self.ctlr1 & ARPE == 0 {
                    self.live_arr = self.arr;
                }
            }
            0x30 if self.advanced => self.rptcr = value,
            0x34..=0x40 => {
                let channel = ((offset - 0x34) / 4) as usize;
                self.ccr[channel] = value & 0xFFFF;
                if !self.preloads_compare(channel) {
                    self.live_ccr[channel] = self.ccr[channel];
                }
            }
            0x44 if self.advanced => self.bdtr = value,
            0x48 => self.dmacfgr = value,
            0x4C => self.dmaadr = value,
            _ => return false,
        }
        true
    }

    /// What channel `channel` (0..4) drives on its pin and, for TIM1's first
    /// three, on its complementary pin. `None` is an output that is off.
    pub fn outputs(&self, channel: usize, hclk: u64) -> (Option<Drive>, Option<Drive>) {
        let mode_bits = self.chctlr[channel / 2] >> (8 * (channel % 2));
        if mode_bits & 0b11 != 0 {
            // A capture input drives nothing.
            return (None, None);
        }
        if self.advanced && self.bdtr & MOE == 0 {
            return (None, None);
        }
        let reference = self.reference(channel, (mode_bits >> 4) & 0b111, hclk);
        let shift = 4 * channel;
        let main = (self.ccer >> shift & 1 != 0).then(|| {
            if self.ccer >> (shift + 1) & 1 != 0 {
                reference.inverted()
            } else {
                reference
            }
        });
        let complement =
            (self.advanced && channel < 3 && self.ccer >> (shift + 2) & 1 != 0).then(|| {
                let inverse = reference.inverted();
                if self.ccer >> (shift + 3) & 1 != 0 {
                    inverse.inverted()
                } else {
                    inverse
                }
            });
        (main, complement)
    }

    /// OCxREF, before polarity.
    fn reference(&self, channel: usize, mode: u32, hclk: u64) -> Drive {
        let period = self.period();
        let compare = u64::from(self.live_ccr[channel]);
        let below = u64::from(self.count) < compare;
        let pwm = |high_below: bool| {
            if !self.running() {
                return Drive::Level(below == high_below);
            }
            let share = compare.min(period) as f64 / period as f64;
            let mut hz = (crate::BASE_HZ as f64) / (self.tick(hclk) as f64 * period as f64);
            if self.ctlr1 & CMS != 0 {
                // Up and down again: twice the counts per period.
                hz /= 2.0;
            }
            Drive::Pwm {
                duty: if high_below { share } else { 1.0 - share },
                hz,
            }
        };
        match mode {
            0b110 => pwm(true),
            0b111 => pwm(false),
            0b100 => Drive::Level(false),
            0b101 => Drive::Level(true),
            // Toggle on match: a square wave at half the counter's rate.
            0b011 if self.running() => Drive::Pwm {
                duty: 0.5,
                hz: (crate::BASE_HZ as f64) / (self.tick(hclk) as f64 * period as f64) / 2.0,
            },
            // Active on match, and the match has happened or will.
            0b001 => Drive::Level(true),
            _ => Drive::Level(false),
        }
    }
}

/// Whether a counter running from `from` to `to` (exclusive of `from`,
/// wrapping at `period`) takes the value `target` on the way.
fn passes(from: u64, to: u64, target: u64, period: u64) -> bool {
    if to - from >= period {
        return true;
    }
    let first = from + 1;
    // The first value at or after `first` that is `target` modulo `period`.
    let base = first - first % period + target;
    let hit = if base >= first { base } else { base + period };
    hit <= to
}

#[cfg(test)]
mod tests {
    use super::*;

    const HCLK: u64 = 1;

    /// ch32-hal's SimplePwm at 1 kHz on a 48 MHz clock: PSC 0, ARR 47999,
    /// PWM mode 1 on channel 4, preloaded, MOE on.
    fn pwm_on_channel_four() -> Timer {
        let mut tim = Timer::new(true);
        tim.write(0x28, 0, 0, HCLK);
        tim.write(0x2C, 47_999, 0, HCLK);
        tim.write(0x1C, (0b110 << 12) | (1 << 11), 0, HCLK);
        tim.write(0x20, 1 << 12, 0, HCLK);
        tim.write(0x44, MOE, 0, HCLK);
        tim.write(0x14, 1, 0, HCLK);
        tim.write(0x00, CEN | ARPE, 0, HCLK);
        tim
    }

    #[test]
    fn pwm_mode_one_is_high_for_compare_over_period() {
        let mut tim = pwm_on_channel_four();
        tim.write(0x40, 12_000, 0, HCLK);
        // Preloaded: the old compare value stands until the update event.
        assert_eq!(
            tim.outputs(3, HCLK).0,
            Some(Drive::Pwm {
                duty: 0.0,
                hz: 1000.0
            })
        );
        assert!(tim.sync(48_000, HCLK), "the update event changed the duty");
        assert_eq!(
            tim.outputs(3, HCLK).0,
            Some(Drive::Pwm {
                duty: 0.25,
                hz: 1000.0
            })
        );
        assert_eq!(tim.read(0x10, 48_000, HCLK) & UIF, UIF);
    }

    #[test]
    fn polarity_and_mode_two_both_invert() {
        let mut tim = pwm_on_channel_four();
        tim.write(0x1C, 0b111 << 12, 0, HCLK);
        tim.write(0x40, 12_000, 0, HCLK);
        assert_eq!(
            tim.outputs(3, HCLK).0,
            Some(Drive::Pwm {
                duty: 0.75,
                hz: 1000.0
            })
        );
        tim.write(0x20, (1 << 12) | (1 << 13), 0, HCLK);
        assert_eq!(
            tim.outputs(3, HCLK).0,
            Some(Drive::Pwm {
                duty: 0.25,
                hz: 1000.0
            })
        );
    }

    #[test]
    fn tim1_drives_nothing_until_its_main_output_is_enabled() {
        let mut tim = pwm_on_channel_four();
        tim.write(0x44, 0, 0, HCLK);
        assert_eq!(tim.outputs(3, HCLK), (None, None));
    }

    #[test]
    fn a_compare_value_past_the_period_is_always_high() {
        let mut tim = pwm_on_channel_four();
        tim.write(0x1C, 0b110 << 12, 0, HCLK);
        tim.write(0x40, 60_000, 0, HCLK);
        assert_eq!(
            tim.outputs(3, HCLK).0,
            Some(Drive::Pwm {
                duty: 1.0,
                hz: 1000.0
            })
        );
    }

    #[test]
    fn the_counter_wraps_and_counts_its_overflows_into_one_flag() {
        let mut tim = Timer::new(false);
        tim.write(0x2C, 99, 0, HCLK);
        tim.write(0x0C, 1, 0, HCLK);
        tim.write(0x00, CEN, 0, HCLK);
        assert_eq!(tim.next_event(HCLK), Some(100));
        assert_eq!(tim.read(0x24, 250, HCLK), 50);
        assert!(tim.update_line());
        tim.write(0x10, !UIF, 250, HCLK);
        assert!(!tim.update_line());
    }

    #[test]
    fn a_compare_match_raises_its_flag_once_passed() {
        assert!(passes(0, 10, 5, 100));
        assert!(!passes(5, 10, 5, 100), "starting on it is not passing it");
        assert!(passes(95, 105, 2, 100), "across the wrap");
        assert!(!passes(95, 101, 2, 100));
        assert!(passes(3, 103, 1, 100), "a whole period passes everything");
    }
}
