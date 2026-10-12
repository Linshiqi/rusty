//! What a character LCD was told to show.
//!
//! An HD44780 is the two-line module on every starter kit, and it is nearly
//! always reached through a PCF8574 expander: the firmware writes one byte
//! to an I2C address, that byte *is* the module's eight pins, and the
//! controller latches half a command every time the enable pin falls. So
//! the bytes the emulator reports are not display data at all — they are
//! port states, and the picture has to be recovered from their edges.
//!
//! That is what this module does, and it is why it sits beside
//! [`crate::screen`] rather than inside it: the same glass, a completely
//! different thing arriving at it.
//!
//! **The character generator here is ASCII, and a real module's may not
//! be.** The HD44780's ROM comes in two flavours: A00 shows `¥` for `0x5c`
//! and `→` for `0x7e`, A02 is Latin and shows what you typed. Nothing in
//! the traffic says which is behind the glass — the firmware writes the
//! same byte either way — so this draws the byte as ASCII and says here
//! that it is doing so, exactly as the OLED's decoder declines to apply a
//! remap it cannot know. The eight characters a firmware *defines*
//! (`CGRAM`, `0x00`–`0x07`) need no such guess and are drawn from the dots
//! the firmware itself sent.
//!
//! What is not modelled: the busy flag and every timing with it (a
//! measurement is always ready, as everywhere else here), reading back from
//! the controller, the cursor and its blink, and display shift. The first
//! is a delay nobody watching a simulator is waiting for; the rest are
//! consumed so they cannot be read as something else.

/// How a character cell is laid out on the glass: five dots across and
/// eight down, with one dark dot between cells and between rows. The
/// eighth row is the cursor's on a real module and stays dark here.
pub const CELL_W: usize = 6;
pub const CELL_H: usize = 9;

/// The dots one character occupies, which is what the pitch is minus the
/// gap.
const GLYPH_W: usize = 5;
const GLYPH_H: usize = 8;

/// A character LCD, as the port bytes a driver has sent it leave it.
#[derive(Debug, Clone, PartialEq)]
pub struct Chars {
    cols: usize,
    rows: usize,
    /// The controller's own eighty bytes, whatever the glass shows of them.
    ddram: [u8; 80],
    /// Eight characters of five-bit rows, as the firmware defined them.
    cgram: [u8; 64],
    /// The address counter, and which memory it points into.
    address: u8,
    in_cgram: bool,
    /// Entry mode: whether the counter moves forward after a byte.
    forward: bool,
    on: bool,
    written: bool,
    /// The half-assembled state of the four-bit bus.
    high: Option<u8>,
    /// The enable pin as it last stood, so a *fall* can be told from a level.
    enable: bool,
}

impl Chars {
    /// A module of this many characters across and down. Sixteen by two is
    /// the one in every kit; twenty by four is the other one sold.
    pub fn new(cols: usize, rows: usize) -> Chars {
        Chars {
            cols: cols.clamp(8, 20),
            rows: rows.clamp(1, 4),
            ddram: [b' '; 80],
            cgram: [0; 64],
            address: 0,
            in_cgram: false,
            forward: true,
            // Off, as the controller wakes: a driver switches it on, and a
            // firmware that never does has a dark module on the desk too.
            on: false,
            written: false,
            high: None,
            enable: false,
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn written(&self) -> bool {
        self.written
    }

    /// The dots of the glass: `width` across, `height` down.
    pub fn width(&self) -> usize {
        self.cols * CELL_W
    }

    pub fn height(&self) -> usize {
        self.rows * CELL_H
    }

    /// One byte written to the expander, which is one state of the module's
    /// eight pins.
    ///
    /// The wiring is the one every one of these boards uses: `P0` is the
    /// register select, `P1` the read/write line, `P2` the enable, `P3` the
    /// backlight, and `P4`–`P7` the high four data lines. A nibble is taken
    /// on the enable's **falling** edge, which is the only moment the
    /// controller reads its bus — a decoder that acted on the level would
    /// take the same nibble twice, since a driver writes the byte with
    /// enable high and again with it low.
    pub fn port(&mut self, byte: u8) {
        let enable = byte & 0x04 != 0;
        let fell = self.enable && !enable;
        self.enable = enable;
        if !fell {
            return;
        }
        // A write only. A read would put the module's own bits on the bus,
        // and nothing here can answer with them.
        if byte & 0x02 != 0 {
            return;
        }
        let nibble = byte >> 4;
        let rs = byte & 0x01 != 0;
        self.nibble(nibble, rs);
    }

    /// Half a byte, from whichever transport carried it.
    ///
    /// Four-bit mode is the only one this reads, because the expander has
    /// four data lines and no other. The initialisation sequence every
    /// driver sends — `0x3` three times, then `0x2` — pairs up into two
    /// function-set commands, which is harmless: it is an even number of
    /// nibbles, so everything after it is still aligned, and a function set
    /// changes nothing this draws.
    pub fn nibble(&mut self, nibble: u8, rs: bool) {
        match self.high.take() {
            None => self.high = Some(nibble & 0x0f),
            Some(high) => {
                let byte = (high << 4) | (nibble & 0x0f);
                if rs {
                    self.data(byte);
                } else {
                    self.command(byte);
                }
            }
        }
    }

    fn command(&mut self, byte: u8) {
        match byte {
            // Clear: the whole of the display memory becomes spaces and the
            // counter goes home. A driver clears before it draws, so a
            // decoder that ignored this would show the last two pictures at
            // once.
            0x01 => {
                self.ddram = [b' '; 80];
                self.address = 0;
                self.in_cgram = false;
                self.forward = true;
            }
            // Home: the counter only.
            b if b & 0xfe == 0x02 => {
                self.address = 0;
                self.in_cgram = false;
            }
            // Entry mode: which way the counter moves. The shift bit is
            // consumed — see the header.
            b if b & 0xfc == 0x04 => self.forward = b & 0x02 != 0,
            // Display on/off. The cursor and its blink are consumed.
            b if b & 0xf8 == 0x08 => self.on = b & 0x04 != 0,
            // Cursor or display shift, and function set: nothing here draws
            // differently for either, and both are consumed so their
            // arguments cannot be read as an address.
            b if b & 0xf0 == 0x10 || b & 0xe0 == 0x20 => {}
            b if b & 0xc0 == 0x40 => {
                self.address = b & 0x3f;
                self.in_cgram = true;
            }
            b if b & 0x80 == 0x80 => {
                self.address = b & 0x7f;
                self.in_cgram = false;
            }
            _ => {}
        }
    }

    fn data(&mut self, byte: u8) {
        self.written = true;
        if self.in_cgram {
            let at = usize::from(self.address) % self.cgram.len();
            self.cgram[at] = byte & 0x1f;
            self.address = if self.forward {
                (self.address + 1) & 0x3f
            } else {
                self.address.wrapping_sub(1) & 0x3f
            };
            return;
        }
        // An address the controller has no memory at is written nowhere and
        // shows nothing, rather than landing somewhere invented: where the
        // counter goes past the end of a line is a thing the datasheet
        // leaves to the part and observed behaviour differs, so nothing is
        // claimed about it. The counter still moves, so a driver that sets
        // an address afterwards is where it thinks it is.
        if let Some(at) = spot(self.address) {
            self.ddram[at] = byte;
        }
        self.address = if self.forward {
            (self.address + 1) & 0x7f
        } else {
            self.address.wrapping_sub(1) & 0x7f
        };
    }

    /// Where a row of the glass starts in the controller's memory.
    ///
    /// Two lines of memory, folded: the third row of a four-line module is
    /// the first row's continuation and the fourth is the second's, which is
    /// why a driver that writes twenty-one characters to row 0 sees the
    /// twenty-first appear on row 2.
    fn base(&self, row: usize) -> usize {
        // Written as the two line bases plus a column offset, because that
        // is what the folding *is*: row 2 continues row 0's memory.
        match row {
            0 => 0,
            1 => 0x40,
            2 => self.cols,
            _ => 0x40 + self.cols,
        }
    }

    /// The character at a cell, as the controller holds it.
    pub fn at(&self, col: usize, row: usize) -> u8 {
        if col >= self.cols || row >= self.rows {
            return b' ';
        }
        let address = u8::try_from(self.base(row) + col).unwrap_or(0xff);
        spot(address).map_or(b' ', |at| self.ddram[at])
    }

    /// Every row as text, for anything that wants the words rather than the
    /// dots — a headless run's log, and a test that can then be read.
    pub fn lines(&self) -> Vec<String> {
        (0..self.rows)
            .map(|row| {
                (0..self.cols)
                    .map(|col| match self.at(col, row) {
                        byte @ 0x20..=0x7e => char::from(byte),
                        // A defined character has no letter to print, and a
                        // byte outside the ASCII the font covers is not
                        // claimed about — see the header.
                        _ => '\u{fffd}',
                    })
                    .collect()
            })
            .collect()
    }

    /// Whether the dot at `(x, y)` — the top left of the glass being
    /// `(0, 0)` — is lit.
    pub fn lit(&self, x: usize, y: usize) -> bool {
        if !self.on {
            return false;
        }
        let (col, dot_x) = (x / CELL_W, x % CELL_W);
        let (row, dot_y) = (y / CELL_H, y % CELL_H);
        if dot_x >= GLYPH_W || dot_y >= GLYPH_H || col >= self.cols || row >= self.rows {
            return false;
        }
        let byte = self.at(col, row);
        if byte < 8 {
            // A character the firmware defined: eight rows of five bits,
            // the leftmost dot in bit 4.
            let at = usize::from(byte) * 8 + dot_y;
            return self.cgram[at] & (0x10 >> dot_x) != 0;
        }
        glyph(byte).is_some_and(|columns| columns[dot_x] & (1 << dot_y) != 0)
    }

    /// The lit dots as horizontal runs: `(x, y, length)`, row by row — the
    /// same shape the OLED answers with, so one path draws either.
    pub fn runs(&self) -> Vec<(usize, usize, usize)> {
        let (width, height) = (self.width(), self.height());
        let mut runs = Vec::new();
        for y in 0..height {
            let mut x = 0;
            while x < width {
                if !self.lit(x, y) {
                    x += 1;
                    continue;
                }
                let from = x;
                while x < width && self.lit(x, y) {
                    x += 1;
                }
                runs.push((from, y, x - from));
            }
        }
        runs
    }
}

/// Where an address lives in the controller's eighty bytes — or nowhere.
///
/// The memory is not flat: a line of forty at `0x00`, a second at `0x40`,
/// and nothing between or after them. Indexed by the raw address instead,
/// row 1's last columns read row 0's first, which is how a four-line
/// module's second row came out holding the first row's overflow.
fn spot(address: u8) -> Option<usize> {
    match address {
        0x00..=0x27 => Some(usize::from(address)),
        0x40..=0x67 => Some(40 + usize::from(address - 0x40)),
        _ => None,
    }
}

/// One character's five columns, bit 0 the top row — or nothing for a byte
/// the font does not cover, which draws as blank rather than as some other
/// letter.
fn glyph(byte: u8) -> Option<[u8; GLYPH_W]> {
    let at = usize::from(byte).checked_sub(0x20)?;
    let at = at.checked_mul(GLYPH_W)?;
    let columns = FONT.get(at..at + GLYPH_W)?;
    Some([columns[0], columns[1], columns[2], columns[3], columns[4]])
}

/// The 5×7 font, `0x20` through `0x7e`, five columns a character with bit 0
/// the top row.
///
/// Read it with the dump test below rather than by eye: `RUSTY_LCD_FONT`
/// names a file and every glyph is written into it as dots, which is the
/// only way to check a table of hex for the one letter that is wrong.
#[rustfmt::skip]
const FONT: &[u8] = &[
    0x00, 0x00, 0x00, 0x00, 0x00, // space
    0x00, 0x00, 0x5f, 0x00, 0x00, // !
    0x00, 0x07, 0x00, 0x07, 0x00, // "
    0x14, 0x7f, 0x14, 0x7f, 0x14, // #
    0x24, 0x2a, 0x7f, 0x2a, 0x12, // $
    0x23, 0x13, 0x08, 0x64, 0x62, // %
    0x36, 0x49, 0x55, 0x22, 0x50, // &
    0x00, 0x05, 0x03, 0x00, 0x00, // '
    0x00, 0x1c, 0x22, 0x41, 0x00, // (
    0x00, 0x41, 0x22, 0x1c, 0x00, // )
    0x14, 0x08, 0x3e, 0x08, 0x14, // *
    0x08, 0x08, 0x3e, 0x08, 0x08, // +
    0x00, 0x50, 0x30, 0x00, 0x00, // ,
    0x08, 0x08, 0x08, 0x08, 0x08, // -
    0x00, 0x60, 0x60, 0x00, 0x00, // .
    0x20, 0x10, 0x08, 0x04, 0x02, // /
    0x3e, 0x51, 0x49, 0x45, 0x3e, // 0
    0x00, 0x42, 0x7f, 0x40, 0x00, // 1
    0x42, 0x61, 0x51, 0x49, 0x46, // 2
    0x21, 0x41, 0x45, 0x4b, 0x31, // 3
    0x18, 0x14, 0x12, 0x7f, 0x10, // 4
    0x27, 0x45, 0x45, 0x45, 0x39, // 5
    0x3c, 0x4a, 0x49, 0x49, 0x30, // 6
    0x01, 0x71, 0x09, 0x05, 0x03, // 7
    0x36, 0x49, 0x49, 0x49, 0x36, // 8
    0x06, 0x49, 0x49, 0x29, 0x1e, // 9
    0x00, 0x36, 0x36, 0x00, 0x00, // :
    0x00, 0x56, 0x36, 0x00, 0x00, // ;
    0x08, 0x14, 0x22, 0x41, 0x00, // <
    0x14, 0x14, 0x14, 0x14, 0x14, // =
    0x00, 0x41, 0x22, 0x14, 0x08, // >
    0x02, 0x01, 0x51, 0x09, 0x06, // ?
    0x32, 0x49, 0x79, 0x41, 0x3e, // @
    0x7e, 0x11, 0x11, 0x11, 0x7e, // A
    0x7f, 0x49, 0x49, 0x49, 0x36, // B
    0x3e, 0x41, 0x41, 0x41, 0x22, // C
    0x7f, 0x41, 0x41, 0x22, 0x1c, // D
    0x7f, 0x49, 0x49, 0x49, 0x41, // E
    0x7f, 0x09, 0x09, 0x09, 0x01, // F
    0x3e, 0x41, 0x49, 0x49, 0x7a, // G
    0x7f, 0x08, 0x08, 0x08, 0x7f, // H
    0x00, 0x41, 0x7f, 0x41, 0x00, // I
    0x20, 0x40, 0x41, 0x3f, 0x01, // J
    0x7f, 0x08, 0x14, 0x22, 0x41, // K
    0x7f, 0x40, 0x40, 0x40, 0x40, // L
    0x7f, 0x02, 0x0c, 0x02, 0x7f, // M
    0x7f, 0x04, 0x08, 0x10, 0x7f, // N
    0x3e, 0x41, 0x41, 0x41, 0x3e, // O
    0x7f, 0x09, 0x09, 0x09, 0x06, // P
    0x3e, 0x41, 0x51, 0x21, 0x5e, // Q
    0x7f, 0x09, 0x19, 0x29, 0x46, // R
    0x46, 0x49, 0x49, 0x49, 0x31, // S
    0x01, 0x01, 0x7f, 0x01, 0x01, // T
    0x3f, 0x40, 0x40, 0x40, 0x3f, // U
    0x1f, 0x20, 0x40, 0x20, 0x1f, // V
    0x3f, 0x40, 0x38, 0x40, 0x3f, // W
    0x63, 0x14, 0x08, 0x14, 0x63, // X
    0x07, 0x08, 0x70, 0x08, 0x07, // Y
    0x61, 0x51, 0x49, 0x45, 0x43, // Z
    0x00, 0x7f, 0x41, 0x41, 0x00, // [
    0x02, 0x04, 0x08, 0x10, 0x20, // backslash
    0x00, 0x41, 0x41, 0x7f, 0x00, // ]
    0x04, 0x02, 0x01, 0x02, 0x04, // ^
    0x40, 0x40, 0x40, 0x40, 0x40, // _
    0x00, 0x01, 0x02, 0x04, 0x00, // `
    0x20, 0x54, 0x54, 0x54, 0x78, // a
    0x7f, 0x48, 0x44, 0x44, 0x38, // b
    0x38, 0x44, 0x44, 0x44, 0x20, // c
    0x38, 0x44, 0x44, 0x48, 0x7f, // d
    0x38, 0x54, 0x54, 0x54, 0x18, // e
    0x08, 0x7e, 0x09, 0x01, 0x02, // f
    0x0c, 0x52, 0x52, 0x52, 0x3e, // g
    0x7f, 0x08, 0x04, 0x04, 0x78, // h
    0x00, 0x44, 0x7d, 0x40, 0x00, // i
    0x20, 0x40, 0x44, 0x3d, 0x00, // j
    0x7f, 0x10, 0x28, 0x44, 0x00, // k
    0x00, 0x41, 0x7f, 0x40, 0x00, // l
    0x7c, 0x04, 0x18, 0x04, 0x78, // m
    0x7c, 0x08, 0x04, 0x04, 0x78, // n
    0x38, 0x44, 0x44, 0x44, 0x38, // o
    0x7c, 0x14, 0x14, 0x14, 0x08, // p
    0x08, 0x14, 0x14, 0x18, 0x7c, // q
    0x7c, 0x08, 0x04, 0x04, 0x08, // r
    0x48, 0x54, 0x54, 0x54, 0x20, // s
    0x04, 0x3f, 0x44, 0x40, 0x20, // t
    0x3c, 0x40, 0x40, 0x20, 0x7c, // u
    0x1c, 0x20, 0x40, 0x20, 0x1c, // v
    0x3c, 0x40, 0x30, 0x40, 0x3c, // w
    0x44, 0x28, 0x10, 0x28, 0x44, // x
    0x0c, 0x50, 0x50, 0x50, 0x3c, // y
    0x44, 0x64, 0x54, 0x4c, 0x44, // z
    0x00, 0x08, 0x36, 0x41, 0x00, // {
    0x00, 0x00, 0x7f, 0x00, 0x00, // |
    0x00, 0x41, 0x36, 0x08, 0x00, // }
    0x08, 0x04, 0x08, 0x10, 0x08, // ~
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The port bytes a driver sends for one byte in four-bit mode: the
    /// high nibble with enable up and down, then the low one. The backlight
    /// bit is set because every driver sets it, and it must change nothing.
    fn send(lcd: &mut Chars, byte: u8, rs: bool) {
        let rs = u8::from(rs);
        for nibble in [byte & 0xf0, (byte << 4) & 0xf0] {
            lcd.port(nibble | rs | 0x08);
            lcd.port(nibble | rs | 0x08 | 0x04);
            lcd.port(nibble | rs | 0x08);
        }
    }

    /// The initialisation every `LiquidCrystal_I2C` driver sends, and what
    /// a driver does next: switch the display on and clear it.
    fn started() -> Chars {
        let mut lcd = Chars::new(16, 2);
        for nibble in [0x30, 0x30, 0x30, 0x20] {
            lcd.port(nibble | 0x08);
            lcd.port(nibble | 0x08 | 0x04);
            lcd.port(nibble | 0x08);
        }
        send(&mut lcd, 0x28, false); // function set: 4-bit, two lines
        send(&mut lcd, 0x0c, false); // display on, no cursor
        send(&mut lcd, 0x06, false); // entry mode: forward
        send(&mut lcd, 0x01, false); // clear
        lcd
    }

    fn print(lcd: &mut Chars, text: &str) {
        for byte in text.bytes() {
            send(lcd, byte, true);
        }
    }

    /// The whole point: the words a driver printed come back as words.
    #[test]
    fn what_the_firmware_printed_is_what_the_glass_shows() {
        let mut lcd = started();
        assert!(lcd.is_on(), "the driver switched it on");
        assert_eq!(lcd.lines(), ["                ", "                "]);

        print(&mut lcd, "hello");
        send(&mut lcd, 0x80 | 0x40, false); // second row, first column
        print(&mut lcd, "rusty");
        assert_eq!(lcd.lines(), ["hello           ", "rusty           "]);

        // And a clear is a clear, not a second picture over the first.
        send(&mut lcd, 0x01, false);
        print(&mut lcd, "ok");
        assert_eq!(lcd.lines(), ["ok              ", "                "]);
    }

    /// The enable pin is read on its *fall*. A decoder that acted on the
    /// level would take every nibble twice, which spells every word with
    /// its letters doubled and then some.
    #[test]
    fn a_nibble_is_taken_once_however_long_enable_is_held() {
        let mut lcd = started();
        // 'A' with enable held up for three port writes in each half.
        for nibble in [0x40, 0x10] {
            lcd.port(nibble | 0x09);
            lcd.port(nibble | 0x09 | 0x04);
            lcd.port(nibble | 0x09 | 0x04);
            lcd.port(nibble | 0x09 | 0x04);
            lcd.port(nibble | 0x09);
        }
        assert_eq!(&lcd.lines()[0][..2], "A ");
    }

    /// A read would put the module's own bits on the bus, and nothing here
    /// can answer with them — so it is not mistaken for a write.
    #[test]
    fn a_read_writes_nothing() {
        let mut lcd = started();
        print(&mut lcd, "x");
        for nibble in [0x40, 0x10] {
            lcd.port(nibble | 0x0a);
            lcd.port(nibble | 0x0a | 0x04);
            lcd.port(nibble | 0x0a);
        }
        assert_eq!(&lcd.lines()[0][..2], "x ", "the read added nothing");
    }

    /// The third row of a four-line module is the first row's own memory
    /// continued, which is why a driver writing past column 19 sees it
    /// appear two rows down.
    #[test]
    fn a_four_line_module_folds_its_memory_the_way_the_controller_does() {
        let mut lcd = Chars::new(20, 4);
        send(&mut lcd, 0x0c, false);
        send(&mut lcd, 0x01, false);
        send(&mut lcd, 0x80, false);
        print(&mut lcd, "0123456789012345678901234");
        let lines = lcd.lines();
        assert_eq!(lines[0], "01234567890123456789");
        assert_eq!(&lines[2][..5], "01234", "the overflow lands on row 2");
        assert_eq!(lines[1], " ".repeat(20));
    }

    /// A character the firmware defined is drawn from the dots the firmware
    /// sent, which needs no guess about anybody's character ROM.
    #[test]
    fn a_defined_character_draws_the_dots_it_was_given() {
        let mut lcd = started();
        // A solid block in slot 0: eight rows of five dots.
        send(&mut lcd, 0x40, false);
        for _ in 0..8 {
            send(&mut lcd, 0x1f, true);
        }
        send(&mut lcd, 0x80, false);
        send(&mut lcd, 0x00, true);
        for y in 0..GLYPH_H {
            for x in 0..GLYPH_W {
                assert!(lcd.lit(x, y), "({x}, {y}) of a solid block");
            }
        }
        // And the gap column and row between cells stay dark, or two
        // neighbouring blocks would read as one.
        assert!(!lcd.lit(GLYPH_W, 0));
        assert!(!lcd.lit(0, GLYPH_H));
    }

    /// A display nobody switched on is dark, as it is on the desk. Half of
    /// the reports about these modules are a missing `backlight()` or a
    /// missing `display()`, and showing text anyway would hide it.
    #[test]
    fn a_display_nobody_switched_on_is_dark() {
        let mut lcd = Chars::new(16, 2);
        send(&mut lcd, 0x01, false);
        print(&mut lcd, "hello");
        assert_eq!(&lcd.lines()[0][..5], "hello", "the memory holds it");
        assert!(lcd.runs().is_empty(), "and nothing is lit");
        send(&mut lcd, 0x0c, false);
        assert!(!lcd.runs().is_empty());
    }

    /// The font is a table of hex, which is unreadable, so it is written
    /// out as dots and looked at. `RUSTY_LCD_FONT=<file>` writes every
    /// glyph; without it this asserts the shapes of three letters whose
    /// dots can be read in the source.
    #[test]
    fn every_glyph_can_be_looked_at() {
        let rows = |byte: u8| {
            let columns = glyph(byte).expect("in the font");
            (0..7)
                .map(|y| {
                    (0..GLYPH_W)
                        .map(|x| if columns[x] & (1 << y) != 0 { '#' } else { '.' })
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(b'A'),
            [
                ".###.", "#...#", "#...#", "#...#", "#####", "#...#", "#...#"
            ]
        );
        assert_eq!(
            rows(b'L'),
            [
                "#....", "#....", "#....", "#....", "#....", "#....", "#####"
            ]
        );
        assert_eq!(
            rows(b'1'),
            [
                "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."
            ]
        );

        if let Ok(path) = std::env::var("RUSTY_LCD_FONT") {
            let mut out = String::new();
            for byte in 0x20..=0x7eu8 {
                out.push_str(&format!("{byte:#04x} {}\n", char::from(byte)));
                for line in rows(byte) {
                    out.push_str(&format!("  {line}\n"));
                }
            }
            std::fs::write(&path, out).expect("write the font dump");
            eprintln!("wrote {path}");
        }
    }
}
