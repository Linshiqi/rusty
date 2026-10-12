//! The programmable fast interrupt controller: which of the 39 interrupt
//! numbers are enabled, pending and active, and in what priority.
//!
//! Sources are levels here, as they are on the part: a timer's line is high
//! while its flag and its enable both are, and pends the interrupt; the
//! handler clears the flag, and taking the interrupt clears the pending bit,
//! so a line still high after `mret` pends it again.
//!
//! Not modelled: preemption by priority (the hart masks interrupts in a
//! handler, so a second waits for the first to return), the threshold, and
//! the four fast vector slots (VTF), which go unused by qingke-rt.

pub struct Pfic {
    enabled: u64,
    pending: u64,
    active: u64,
    priority: [u8; 64],
    threshold: u32,
    sctlr: u32,
    /// Set when the firmware asks for a system reset; the machine performs it.
    pub reset_requested: bool,
}

impl Default for Pfic {
    fn default() -> Self {
        Self {
            enabled: 0,
            pending: 0,
            active: 0,
            priority: [0; 64],
            threshold: 0,
            sctlr: 0,
            reset_requested: false,
        }
    }
}

impl Pfic {
    /// Pend every interrupt whose line is high.
    pub fn raise(&mut self, lines: u64) {
        self.pending |= lines;
    }

    /// The interrupt that should be taken now, if any: enabled, pending,
    /// lowest priority value first, then lowest number.
    pub fn next(&self) -> Option<u32> {
        let ready = self.enabled & self.pending;
        if ready == 0 {
            return None;
        }
        (0..64u32)
            .filter(|n| ready & (1 << n) != 0)
            .min_by_key(|&n| (self.priority[n as usize], n))
    }

    /// Whether anything enabled is pending — what wakes a `wfi`.
    pub fn wakes(&self) -> bool {
        self.enabled & self.pending != 0
    }

    pub fn take(&mut self, irq: u32) {
        self.pending &= !(1 << irq);
        self.active |= 1 << irq;
    }

    pub fn finish(&mut self, irq: u32) {
        self.active &= !(1 << irq);
    }

    fn half(bits: u64, offset: u32) -> u32 {
        (bits >> (32 * ((offset >> 2) & 1))) as u32
    }

    fn with(bits: &mut u64, offset: u32, value: u32, set: bool) {
        let mask = u64::from(value) << (32 * ((offset >> 2) & 1));
        if set {
            *bits |= mask;
        } else {
            *bits &= !mask;
        }
    }

    pub fn read(&self, offset: u32) -> u32 {
        match offset {
            0x000..=0x00C => Self::half(self.enabled, offset),
            0x020..=0x02C => Self::half(self.pending, offset),
            0x040 => self.threshold,
            0x300..=0x30C => Self::half(self.active, offset),
            0x400..=0x43F => {
                let base = (offset - 0x400) as usize;
                u32::from_le_bytes([
                    self.priority[base],
                    self.priority.get(base + 1).copied().unwrap_or(0),
                    self.priority.get(base + 2).copied().unwrap_or(0),
                    self.priority.get(base + 3).copied().unwrap_or(0),
                ])
            }
            0xD10 => self.sctlr,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x040 => self.threshold = value,
            // CFGR: RESETSYS (bit 7) under its key.
            0x048 => {
                if value & (1 << 7) != 0 && value >> 16 == 0xBEEF {
                    self.reset_requested = true;
                }
            }
            0x100..=0x10C => Self::with(&mut self.enabled, offset, value, true),
            0x180..=0x18C => Self::with(&mut self.enabled, offset, value, false),
            0x200..=0x20C => Self::with(&mut self.pending, offset, value, true),
            0x280..=0x28C => Self::with(&mut self.pending, offset, value, false),
            0x400..=0x43F => {
                let base = (offset - 0x400) as usize;
                for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
                    if let Some(slot) = self.priority.get_mut(base + i) {
                        *slot = byte;
                    }
                }
            }
            0xD10 => {
                if value & (1 << 31) != 0 {
                    self.reset_requested = true;
                }
                self.sctlr = value & !(1 << 31);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_enabled_pending_interrupt_is_taken_by_priority_then_number() {
        let mut pfic = Pfic::default();
        pfic.write(0x104, (1 << (38 - 32)) | (1 << (35 - 32)));
        pfic.raise((1 << 38) | (1 << 35) | (1 << 20));
        assert_eq!(pfic.next(), Some(35), "20 is not enabled; 35 before 38");
        // Interrupt 35's priority byte is the top byte of the word at 32.
        pfic.write(0x400 + 32, 0x80 << 24);
        assert_eq!(pfic.next(), Some(38), "a lower value wins");
        pfic.take(38);
        assert_eq!(pfic.next(), Some(35));
        assert_eq!(pfic.read(0x304), 1 << (38 - 32), "38 is active");
    }

    #[test]
    fn a_disable_write_clears_only_the_bits_it_names() {
        let mut pfic = Pfic::default();
        pfic.write(0x100, 0b1100);
        pfic.write(0x180, 0b0100);
        assert_eq!(pfic.read(0x000), 0b1000);
    }
}
