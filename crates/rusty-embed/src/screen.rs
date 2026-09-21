//! What a monochrome OLED was told to draw.
//!
//! The emulator reports the bytes that cross the I2C bus, and for a display
//! that is the whole of the picture: an SSD1306 has no state a driver reads
//! back, so everything on the glass arrived as a write. This module is the
//! reading of that stream — the command set a driver actually uses, the
//! addressing window it sets, and the RAM the data bytes land in — so the
//! board sheet can show the screen rather than a count of transactions.
//!
//! It is the firmware's own picture, drawn by the firmware's own driver.
//! Nothing here is a claim about what a part *would* show: `ssd1306`,
//! `embedded-graphics` and a hand-written init sequence all reach the same
//! RAM, and a driver that sends something this does not recognise leaves the
//! screen as it stood rather than drawing something invented.
//!
//! **The panel's remap is not applied**, and that is a decision rather than
//! an omission. `SEGREMAP` and `COMSCANDEC` are about how a particular
//! module's glass is wired to the controller, not about what the firmware
//! drew: the usual 128×64 board needs both set to come out the right way up,
//! and a firmware that leaves them at their defaults shows a mirrored image
//! on that board and an upright one on another. What rusty can say without
//! knowing which module is on the desk is what the firmware put in the RAM,
//! so that is what it draws.
//!
//! What is deliberately not modelled: the charge pump, the contrast, the
//! pre-charge and VCOMH levels, and the hardware scroll.
//! The first four are analog facts about a panel nobody is looking at
//! through a simulator, and the arguments are consumed so they cannot be
//! read as commands. Scrolling is consumed the same way — the command sets
//! it up and the hardware animates it on a clock this has none of.

/// The controller behind the glass.
///
/// Two, because they are the two a hobby board carries and they differ in
/// exactly one thing that matters here: the SH1106 has 132 columns of RAM
/// behind a 128-column window, so its drivers write from column 2 and an
/// image decoded without that offset sits two pixels to the left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    /// Solomon Systech's SSD1306: the 0.96" module on every starter kit.
    Ssd1306,
    /// Sino Wealth's SH1106: the 1.3" module, and the same command set bar
    /// the addressing modes.
    Sh1106,
    /// Hitachi's HD44780 behind a PCF8574 expander — the two-line character
    /// module in every kit. Nothing about its traffic resembles the other
    /// two's: the bytes are an expander's port states and the picture comes
    /// off their edges ([`crate::lcd`]). It is a panel here anyway because
    /// what it ends as is the same thing — lit dots on glass at an address —
    /// and a second kind of display beside this one would be a second copy
    /// of every place a screen is declared, kept, fed and drawn.
    Hd44780,
}

impl Panel {
    /// Every panel, for a menu to iterate rather than spell.
    pub const ALL: [Panel; 3] = [Panel::Ssd1306, Panel::Sh1106, Panel::Hd44780];

    /// The id the sheet stores in a part's `panel` prop.
    pub fn id(self) -> &'static str {
        match self {
            Panel::Ssd1306 => "ssd1306",
            Panel::Sh1106 => "sh1106",
            Panel::Hd44780 => "hd44780",
        }
    }

    /// What a person calls it.
    pub fn name(self) -> &'static str {
        match self {
            Panel::Ssd1306 => "SSD1306",
            Panel::Sh1106 => "SH1106",
            Panel::Hd44780 => "HD44780",
        }
    }

    /// The panel an id names, case folded, or nothing.
    pub fn from_id(id: &str) -> Option<Panel> {
        let id = id.trim();
        Panel::ALL
            .into_iter()
            .find(|panel| panel.id().eq_ignore_ascii_case(id))
    }

    /// How many columns of RAM the controller has, which is not always how
    /// many the glass shows.
    fn columns(self) -> usize {
        match self {
            Panel::Ssd1306 => 128,
            Panel::Sh1106 => 132,
            // The widest character module sold, in dots.
            Panel::Hd44780 => 20 * crate::lcd::CELL_W,
        }
    }

    /// Whether the panel's bytes are a framebuffer's or an expander's.
    pub fn is_character(self) -> bool {
        matches!(self, Panel::Hd44780)
    }

    /// What `Screen::of` makes: the module this panel usually comes as.
    fn usual(self) -> (usize, usize) {
        match self {
            Panel::Hd44780 => (16 * crate::lcd::CELL_W, 2 * crate::lcd::CELL_H),
            _ => (128, 64),
        }
    }

    /// Which RAM column the leftmost pixel of the glass is.
    fn offset(self) -> usize {
        match self {
            Panel::Sh1106 => 2,
            _ => 0,
        }
    }

    /// What the controller does with the pointer after a data byte, before
    /// a driver says otherwise. The SSD1306 wakes in page addressing and so
    /// does the SH1106, which has no other.
    fn wakes_in(self) -> Mode {
        Mode::Page
    }
}

/// How many pages of memory the controller has. Eight, whatever the glass
/// in front of it turns out to be.
const PAGES: usize = 8;

/// Where the pointer goes after a byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Along the row, then down to the next page — what a driver sending a
    /// whole framebuffer in one transaction uses.
    Horizontal,
    /// Down the page, then along — the same window walked the other way.
    Vertical,
    /// Along the row and round again within the same page: the SH1106's only
    /// mode, and the SSD1306's at power-on.
    Page,
}

/// A command that has arrived and is waiting for its arguments.
///
/// Spelled out rather than counted, because the three that matter *are* the
/// state: a window set across two transactions is an ordinary thing for a
/// driver to do, and a decoder that lost track of it would write the next
/// framebuffer into the wrong corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wait {
    None,
    /// `0x20`: the addressing mode.
    Mode,
    /// `0x21`: the column window, first edge then second.
    ColumnStart,
    ColumnEnd,
    /// `0x22`: the page window.
    PageStart,
    PageEnd,
    /// `0xA8`: how many rows the glass has, which the driver knows and
    /// this cannot.
    Multiplex,
    /// A command whose arguments change nothing on the glass. Counted so
    /// they are consumed rather than read as commands of their own — a
    /// contrast of `0xAF` would otherwise switch the display on.
    Ignore(u8),
}

/// What the next byte of a message is, which is state because a message
/// can arrive in pieces.
///
/// The emulator reports a transaction in the steps the driver moved it in —
/// a framebuffer crosses a thirty-two byte FIFO — so the control byte that
/// says what everything after it means can be several reports back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    /// A control byte.
    Control,
    /// One byte of this kind, then a control byte again: the continuation
    /// bit set.
    One(bool),
    /// Everything left in this message is of this kind.
    Rest(bool),
}

/// A screen, as the bytes a driver has sent it leave it.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    panel: Panel,
    width: usize,
    height: usize,
    /// One byte per column per page, the controller's own layout: bit *n* of
    /// page *p* is row `p * 8 + n`.
    ram: Vec<u8>,
    mode: Mode,
    column_start: usize,
    column_end: usize,
    page_start: usize,
    page_end: usize,
    column: usize,
    page: usize,
    waiting: Wait,
    next: Next,
    on: bool,
    inverse: bool,
    all_on: bool,
    start_line: usize,
    /// Whether any data byte has ever arrived. An init sequence with no
    /// framebuffer after it is a driver that has started and drawn nothing,
    /// which is worth telling from a screen nobody has addressed.
    written: bool,
    /// Present exactly for a character panel, and then it is where
    /// everything above is kept instead.
    chars: Option<crate::lcd::Chars>,
}

impl Screen {
    /// A screen of the size the module usually is: 128×64 dots, or the
    /// sixteen by two characters a kit's LCD comes as.
    pub fn of(panel: Panel) -> Screen {
        let (width, height) = panel.usual();
        Screen::new(panel, width, height)
    }

    /// A character module of this many characters across and down.
    pub fn of_characters(cols: usize, rows: usize) -> Screen {
        Screen::new(
            Panel::Hd44780,
            cols * crate::lcd::CELL_W,
            rows * crate::lcd::CELL_H,
        )
    }

    /// A screen of a stated size.
    ///
    /// **The memory is the controller's and the glass is the module's.**
    /// An SSD1306 has eight pages of RAM whatever is in front of it, so
    /// that is what is allocated; the height only says how much of it is
    /// lit, and the driver corrects it with the multiplex ratio it sets —
    /// a 128×32 module is `0xA8 0x1F`, which this reads rather than being
    /// told. A height guessed at from the part number would draw the
    /// common module's aspect for every panel.
    pub fn new(panel: Panel, width: usize, height: usize) -> Screen {
        let width = width.clamp(8, panel.columns());
        let height = height.clamp(8, 64);
        Screen {
            panel,
            width,
            height,
            ram: vec![0; PAGES * panel.columns()],
            mode: panel.wakes_in(),
            column_start: 0,
            column_end: panel.columns() - 1,
            page_start: 0,
            page_end: PAGES - 1,
            column: 0,
            page: 0,
            waiting: Wait::None,
            next: Next::Control,
            // Off, as the controller wakes: a driver switches it on, and a
            // firmware that never does has a dark screen on the desk too.
            on: false,
            inverse: false,
            all_on: false,
            start_line: 0,
            written: false,
            chars: panel.is_character().then(|| {
                crate::lcd::Chars::new(width / crate::lcd::CELL_W, height / crate::lcd::CELL_H)
            }),
        }
    }

    pub fn panel(&self) -> Panel {
        self.panel
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Whether the driver has switched the panel on.
    pub fn is_on(&self) -> bool {
        self.chars.as_ref().map_or(self.on, |lcd| lcd.is_on())
    }

    /// Whether anything has ever been drawn.
    pub fn written(&self) -> bool {
        self.chars
            .as_ref()
            .map_or(self.written, |lcd| lcd.written())
    }

    /// The character module behind the glass, for anything that wants the
    /// words rather than the dots.
    pub fn characters(&self) -> Option<&crate::lcd::Chars> {
        self.chars.as_ref()
    }

    /// A transaction's payload, control byte and all.
    ///
    /// The control byte is the protocol's own: bit 6 says whether what
    /// follows is data or commands, and bit 7 says whether it is one byte or
    /// the rest of the transaction. Both spellings are in the wild —
    /// `ssd1306` sends `0x00` then a run of commands and `0x40` then
    /// sixteen bytes of framebuffer, while ESP-IDF's driver sends `0x80`
    /// before every single command — so both are read rather than one being
    /// assumed.
    pub fn i2c(&mut self, bytes: &[u8]) {
        self.next = Next::Control;
        self.feed(bytes);
    }

    /// More of the transaction before it — the emulator's `w+`.
    ///
    /// A driver sending a whole framebuffer at once writes a thousand bytes
    /// through a thirty-two byte FIFO, and the emulator reports each step as
    /// it goes. Read as messages of their own, every step after the first
    /// would have its first byte taken for a control byte: a screen of
    /// nonsense from a driver that did nothing unusual.
    pub fn i2c_more(&mut self, bytes: &[u8]) {
        self.feed(bytes);
    }

    fn feed(&mut self, bytes: &[u8]) {
        // A character panel's bytes carry no control byte: each one is a
        // state of the expander's eight pins, and the controller reads its
        // bus on the enable's fall.
        if let Some(lcd) = self.chars.as_mut() {
            for byte in bytes {
                lcd.port(*byte);
            }
            return;
        }
        let mut at = 0;
        while at < bytes.len() {
            match self.next {
                Next::Control => {
                    let control = bytes[at];
                    at += 1;
                    let data = control & 0x40 != 0;
                    self.next = if control & 0x80 != 0 {
                        Next::One(data)
                    } else {
                        Next::Rest(data)
                    };
                }
                Next::One(data) => {
                    self.byte(bytes[at], data);
                    at += 1;
                    self.next = Next::Control;
                }
                Next::Rest(data) => {
                    self.byte(bytes[at], data);
                    at += 1;
                }
            }
        }
    }

    fn byte(&mut self, byte: u8, data: bool) {
        if data {
            self.data(byte);
        } else {
            self.command(byte);
        }
    }

    /// How many pages of RAM this screen has. Always [`PAGES`] — the
    /// method stays because every walk of the memory reads it, and a
    /// controller with another shape would change it here.
    fn pages(&self) -> usize {
        PAGES
    }

    fn command(&mut self, byte: u8) {
        let columns = self.panel.columns();
        let pages = self.pages();

        match self.waiting {
            Wait::None => {}
            Wait::Mode => {
                self.mode = match byte & 0x03 {
                    0 => Mode::Horizontal,
                    1 => Mode::Vertical,
                    _ => Mode::Page,
                };
                self.waiting = Wait::None;
                return;
            }
            Wait::ColumnStart => {
                self.column_start = (byte as usize).min(columns - 1);
                self.column = self.column_start;
                self.waiting = Wait::ColumnEnd;
                return;
            }
            Wait::ColumnEnd => {
                self.column_end = (byte as usize).min(columns - 1);
                self.waiting = Wait::None;
                return;
            }
            Wait::PageStart => {
                self.page_start = (byte as usize).min(pages - 1);
                self.page = self.page_start;
                self.waiting = Wait::PageEnd;
                return;
            }
            Wait::PageEnd => {
                self.page_end = (byte as usize).min(pages - 1);
                self.waiting = Wait::None;
                return;
            }
            Wait::Multiplex => {
                // The glass, not the memory: a 128×32 module drives 32 COM
                // lines and its RAM is still eight pages deep.
                self.height = ((byte as usize & 0x3f) + 1).clamp(8, 64);
                self.waiting = Wait::None;
                return;
            }
            Wait::Ignore(left) => {
                self.waiting = if left > 1 {
                    Wait::Ignore(left - 1)
                } else {
                    Wait::None
                };
                return;
            }
        }

        match byte {
            // The column, low nibble then high, which is how page
            // addressing says where the next run of bytes goes.
            0x00..=0x0f => self.column = (self.column & 0xf0) | (byte as usize & 0x0f),
            0x10..=0x1f => self.column = (self.column & 0x0f) | ((byte as usize & 0x0f) << 4),
            0x20 => self.waiting = Wait::Mode,
            0x21 => self.waiting = Wait::ColumnStart,
            0x22 => self.waiting = Wait::PageStart,
            // Scrolling: set up and consumed, never animated.
            0x26 | 0x27 => self.waiting = Wait::Ignore(6),
            0x29 | 0x2a => self.waiting = Wait::Ignore(5),
            0x2e | 0x2f => {}
            // Which RAM row the top of the glass shows.
            0x40..=0x7f => self.start_line = byte as usize & 0x3f,
            // Contrast, and the charge pump: an argument each, and no
            // difference to what is drawn.
            0x81 | 0x8d => self.waiting = Wait::Ignore(1),
            0xa0 | 0xa1 => {}
            0xa4 => self.all_on = false,
            0xa5 => self.all_on = true,
            0xa6 => self.inverse = false,
            0xa7 => self.inverse = true,
            0xa8 => self.waiting = Wait::Multiplex,
            // Display offset, clock, pre-charge, COM pins, VCOMH: an
            // argument each, none of them visible here.
            0xd3 | 0xd5 | 0xd9 | 0xda | 0xdb => self.waiting = Wait::Ignore(1),
            0xae => self.on = false,
            0xaf => self.on = true,
            0xb0..=0xb7 => self.page = (byte as usize & 0x07).min(pages.saturating_sub(1)),
            0xc0 | 0xc8 => {}
            // Anything else is a command this does not know. Nothing is
            // drawn for it and nothing is guessed: an unread command is a
            // screen that stands still, which is visible, where an invented
            // one is a picture that is quietly wrong.
            _ => {}
        }
    }

    fn data(&mut self, byte: u8) {
        let columns = self.panel.columns();
        let pages = self.pages();

        if self.page < pages && self.column < columns {
            self.ram[self.page * columns + self.column] = byte;
            self.written = true;
        }
        match self.mode {
            Mode::Horizontal => {
                self.column += 1;
                if self.column > self.column_end || self.column >= columns {
                    self.column = self.column_start;
                    self.page += 1;
                    if self.page > self.page_end || self.page >= pages {
                        self.page = self.page_start;
                    }
                }
            }
            Mode::Vertical => {
                self.page += 1;
                if self.page > self.page_end || self.page >= pages {
                    self.page = self.page_start;
                    self.column += 1;
                    if self.column > self.column_end || self.column >= columns {
                        self.column = self.column_start;
                    }
                }
            }
            // Round again within the page, as the controller does: a driver
            // that writes 132 bytes to a 128-column page overwrites its own
            // first ones rather than spilling into the page below.
            Mode::Page => {
                self.column += 1;
                if self.column >= columns {
                    self.column = 0;
                }
            }
        }
    }

    /// Whether the pixel at `(x, y)` — the top left of the glass being
    /// `(0, 0)` — is lit.
    pub fn lit(&self, x: usize, y: usize) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        if let Some(lcd) = self.chars.as_ref() {
            return lcd.lit(x, y);
        }
        if self.all_on {
            return true;
        }
        let rows = self.pages() * 8;
        let row = (y + self.start_line) % rows;
        let column = x + self.panel.offset();
        let byte = self.ram[(row / 8) * self.panel.columns() + column];
        let set = byte & (1 << (row % 8)) != 0;
        set != self.inverse
    }

    /// The lit pixels as horizontal runs: `(x, y, length)`, row by row.
    ///
    /// Runs rather than pixels because that is what draws: a 128×64 screen
    /// is eight thousand pixels and a page of text is a few hundred runs, so
    /// the view puts the whole screen in one path instead of a rectangle per
    /// dot.
    pub fn runs(&self) -> Vec<(usize, usize, usize)> {
        let mut runs = Vec::new();
        for y in 0..self.height {
            let mut x = 0;
            while x < self.width {
                if !self.lit(x, y) {
                    x += 1;
                    continue;
                }
                let from = x;
                while x < self.width && self.lit(x, y) {
                    x += 1;
                }
                runs.push((from, y, x - from));
            }
        }
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The init sequence the `ssd1306` crate sends, in the one transaction
    /// it sends it in: a control byte of zero and the commands after it.
    const INIT: &[u8] = &[
        0x00, 0xae, 0xd5, 0x80, 0xa8, 0x3f, 0xd3, 0x00, 0x40, 0x8d, 0x14, 0x20, 0x00, 0xa1, 0xc8,
        0xda, 0x12, 0x81, 0xcf, 0xd9, 0xf1, 0xdb, 0x40, 0xa4, 0xa6, 0xaf,
    ];

    #[test]
    fn an_init_sequence_switches_the_panel_on_and_draws_nothing() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(INIT);
        assert!(screen.is_on());
        assert!(!screen.written(), "no data byte has arrived");
        assert!(screen.runs().is_empty());
    }

    /// The arguments of the init commands must not be read as commands.
    /// `0x81 0xcf` is a contrast; read as two commands the `0xcf` is
    /// nothing, but `0xa8 0x3f` would set the column high nibble and
    /// `0xd3 0x00` the low one, so the next framebuffer would land
    /// somewhere else entirely.
    #[test]
    fn an_arguments_byte_is_never_read_as_a_command() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(INIT);
        // The window is still the whole screen, and the pointer at its
        // start — which is what the init's `0x20 0x00` asked for.
        screen.i2c(&[0x40, 0xff]);
        assert!(screen.lit(0, 0));
        assert!(screen.lit(0, 7));
        assert!(!screen.lit(1, 0));
    }

    #[test]
    fn a_data_byte_is_a_column_of_eight_pixels() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(INIT);
        // 0b0000_0101: rows 0 and 2 of page 0.
        screen.i2c(&[0x40, 0x05]);
        assert!(screen.lit(0, 0));
        assert!(!screen.lit(0, 1));
        assert!(screen.lit(0, 2));
        assert_eq!(screen.runs(), vec![(0, 0, 1), (0, 2, 1)]);
    }

    #[test]
    fn horizontal_addressing_walks_the_row_then_the_page() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(INIT);
        // A window two columns wide and two pages tall, then four bytes.
        screen.i2c(&[0x00, 0x21, 0x00, 0x01, 0x22, 0x00, 0x01]);
        screen.i2c(&[0x40, 0x01, 0x02, 0x04, 0x08]);
        assert!(screen.lit(0, 0), "first byte, page 0 column 0");
        assert!(screen.lit(1, 1), "second byte, page 0 column 1");
        assert!(screen.lit(0, 10), "third byte wrapped to page 1");
        assert!(screen.lit(1, 11), "fourth byte, page 1 column 1");
    }

    #[test]
    fn vertical_addressing_walks_the_page_then_the_row() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(INIT);
        screen.i2c(&[0x00, 0x20, 0x01, 0x21, 0x00, 0x01, 0x22, 0x00, 0x01]);
        screen.i2c(&[0x40, 0x01, 0x02, 0x04, 0x08]);
        assert!(screen.lit(0, 0), "first byte, page 0");
        assert!(screen.lit(0, 9), "second byte went down a page");
        assert!(screen.lit(1, 2), "third byte moved along a column");
        assert!(screen.lit(1, 11), "fourth byte, page 1 column 1");
    }

    /// Page addressing is set a column at a time, in two nibbles — and the
    /// two arrive as separate commands, so a decoder that took either for
    /// the whole column would put every page-mode driver's bytes in the
    /// wrong half of the screen.
    #[test]
    fn page_addressing_takes_its_column_in_two_nibbles() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(INIT);
        screen.i2c(&[0x00, 0x20, 0x02, 0xb2, 0x03, 0x14]);
        screen.i2c(&[0x40, 0x80]);
        assert!(screen.lit(0x43, 23), "page 2, column 0x43, top bit");
    }

    /// The SH1106's window sits two columns into its RAM, and its drivers
    /// write from there. Decoded without the offset the whole image is two
    /// pixels left, which is exactly the kind of quietly wrong picture a
    /// simulator must not draw.
    #[test]
    fn the_sh1106s_two_columns_of_margin_are_taken_off() {
        let mut screen = Screen::new(Panel::Sh1106, 128, 64);
        screen.i2c(INIT);
        screen.i2c(&[0x00, 0xb0, 0x02, 0x10]);
        screen.i2c(&[0x40, 0x01]);
        assert!(
            screen.lit(0, 0),
            "RAM column 2 is the left edge of the glass"
        );
    }

    /// A shorter module says so in its init, and the driver is the only
    /// thing that knows: `0xA8 0x1F` is 32 rows. Drawn at 64 the picture
    /// is half a screen with an empty half below it, which is the aspect
    /// of a module nobody fitted.
    #[test]
    fn the_glass_is_as_tall_as_the_driver_says_and_the_memory_is_not() {
        let mut screen = Screen::of(Panel::Ssd1306);
        assert_eq!(screen.height(), 64);
        screen.i2c(&[
            0x00, 0xa8, 0x1f, 0x20, 0x00, 0x21, 0x00, 0x7f, 0x22, 0x00, 0x07, 0xaf,
        ]);
        assert_eq!(screen.height(), 32);
        // The memory is still eight pages: a driver may address page 7 of a
        // 128x32 module, and writing past the end would panic.
        screen.i2c(&[0x40, 0x01]);
        assert!(screen.lit(0, 0));
        assert!(!screen.lit(0, 40), "past the glass");
        for _ in 0..7 {
            screen.i2c(
                &[0x40]
                    .iter()
                    .chain([0x00u8; 128].iter())
                    .copied()
                    .collect::<Vec<u8>>(),
            );
        }
        assert_eq!(screen.runs().len(), 1, "and nothing was drawn outside it");
    }

    #[test]
    fn inverse_turns_every_pixel_round() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(INIT);
        screen.i2c(&[0x40, 0x01]);
        assert!(screen.lit(0, 0));
        assert!(!screen.lit(5, 5));
        screen.i2c(&[0x00, 0xa7]);
        assert!(!screen.lit(0, 0));
        assert!(screen.lit(5, 5));
    }

    /// ESP-IDF's driver puts a control byte before every command. Both
    /// spellings reach the same place or one of the two families of driver
    /// draws nothing at all.
    #[test]
    fn a_control_byte_before_every_command_reads_the_same() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(&[0x80, 0xae, 0x80, 0xa6, 0x80, 0xaf]);
        assert!(screen.is_on());
        assert!(!screen.inverse);
        // And one data byte at a time, with the continuation bit set.
        screen.i2c(&[0xc0, 0x03, 0xc0, 0x03]);
        assert!(screen.lit(0, 0) && screen.lit(1, 1));
    }

    #[test]
    fn the_start_line_scrolls_the_glass_over_the_ram() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(INIT);
        screen.i2c(&[0x40, 0x01]);
        assert!(screen.lit(0, 0));
        screen.i2c(&[0x00, 0x40 | 1]);
        assert!(
            screen.lit(0, 63),
            "the row that was at the top is now at the bottom"
        );
    }

    #[test]
    fn a_command_it_does_not_know_leaves_the_screen_where_it_was() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(INIT);
        screen.i2c(&[0x40, 0xff]);
        let before = screen.runs();
        screen.i2c(&[0x00, 0xe3, 0xfd]);
        assert_eq!(screen.runs(), before);
    }

    #[test]
    fn a_window_set_across_two_transactions_still_holds() {
        let mut screen = Screen::of(Panel::Ssd1306);
        screen.i2c(INIT);
        screen.i2c(&[0x00, 0x21, 0x04]);
        screen.i2c(&[0x00, 0x08, 0x22, 0x01]);
        screen.i2c(&[0x00, 0x01]);
        screen.i2c(&[0x40, 0x01]);
        assert!(screen.lit(4, 8), "column 4 of page 1");
    }

    #[test]
    fn a_panel_is_named_by_its_id() {
        assert_eq!(Panel::from_id("SSD1306"), Some(Panel::Ssd1306));
        assert_eq!(Panel::from_id(" sh1106 "), Some(Panel::Sh1106));
        assert_eq!(Panel::from_id("ssd1309"), None);
    }
}
