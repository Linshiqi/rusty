//! USART1, as far as text goes: what the firmware writes comes out on the
//! console, and what is typed into it arrives in the receive register.
//! Transmission takes no time — `TXE` and `TC` are always set — because the
//! one thing a reader of the console needs is the text, in order.

use std::collections::VecDeque;

const RXNE: u32 = 1 << 5;
const TC: u32 = 1 << 6;
const TXE: u32 = 1 << 7;
const UE: u32 = 1 << 13;
const TE: u32 = 1 << 3;
const RE: u32 = 1 << 2;

#[derive(Default)]
pub struct Usart {
    brr: u32,
    ctlr1: u32,
    ctlr2: u32,
    ctlr3: u32,
    gpr: u32,
    received: VecDeque<u8>,
}

impl Usart {
    fn statr(&self) -> u32 {
        TXE | TC | if self.received.is_empty() { 0 } else { RXNE }
    }

    /// Bytes from outside, for the firmware to read — only while its
    /// receiver is on, as on the part.
    pub fn receive(&mut self, bytes: &[u8]) {
        if self.ctlr1 & (UE | RE) == UE | RE {
            self.received.extend(bytes);
        }
    }

    /// USART1's interrupt line: receive-not-empty, transmit-empty and
    /// transmission-complete, each with its enable.
    pub fn line(&self) -> bool {
        self.statr() & self.ctlr1 & (RXNE | TXE | TC) != 0
    }

    pub fn read(&mut self, offset: u32) -> u32 {
        match offset {
            0x00 => self.statr(),
            0x04 => u32::from(self.received.pop_front().unwrap_or(0)),
            0x08 => self.brr,
            0x0C => self.ctlr1,
            0x10 => self.ctlr2,
            0x14 => self.ctlr3,
            0x18 => self.gpr,
            _ => 0,
        }
    }

    /// A write; a transmitted byte comes back to go on the console.
    pub fn write(&mut self, offset: u32, value: u32) -> Option<u8> {
        match offset {
            0x04 if self.ctlr1 & (UE | TE) == UE | TE => return Some(value as u8),
            0x08 => self.brr = value,
            0x0C => self.ctlr1 = value,
            0x10 => self.ctlr2 = value,
            0x14 => self.ctlr3 = value,
            0x18 => self.gpr = value,
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_goes_out_only_with_the_transmitter_on() {
        let mut usart = Usart::default();
        assert_eq!(usart.write(0x04, u32::from(b'x')), None);
        usart.write(0x0C, UE | TE | RE);
        assert_eq!(usart.write(0x04, u32::from(b'x')), Some(b'x'));
        usart.receive(b"ok");
        assert_ne!(usart.read(0x00) & RXNE, 0);
        assert_eq!(usart.read(0x04), u32::from(b'o'));
        assert_eq!(usart.read(0x04), u32::from(b'k'));
        assert_eq!(usart.read(0x00) & RXNE, 0);
    }
}
