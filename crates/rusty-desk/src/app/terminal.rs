//! The Terminal: a real shell in a pty (`rusty_term`, portable-pty and
//! vt100), drawn as the runs of same-styled text the emulator hands back,
//! typed into with the keys a terminal expects.
//!
//! One shell, started in the project the first time the panel is shown and
//! again on Restart once it has exited. A thread sits on the emulator's
//! updates — coalesced, so a build scrolling at full speed is a frame per
//! wake and not a frame per write — and hands each screen to the window.

use std::{rc::Rc, sync::Arc};

use rusty_term::{Colour, Screen, Style, Terminal};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, platform::Key};

use super::App;
use crate::{Shell, TermRow, TermSpan, Theme, work};

/// The sixteen base colours, light and dark: a program asking for red means
/// the theme's red, and a palette baked for a dark ground is unreadable on
/// a light one.
const LIGHT: [u32; 16] = [
    0x3b3733, 0xb3341f, 0x3f7a34, 0x8a6510, 0x2f5f96, 0x8a4a8f, 0x217a70, 0x5a544d, 0x7d766d,
    0xd3512f, 0x4f9a41, 0xb0821a, 0x3f7ac0, 0xa95fae, 0x2c9a8d, 0x1c1a17,
];
const DARK: [u32; 16] = [
    0x2a2724, 0xef6b56, 0x92cf7b, 0xe2bb52, 0x7cb2e3, 0xd29ade, 0x5accbd, 0xb8b0a6, 0x6d665e,
    0xff8b74, 0xa2dd89, 0xf5cd6e, 0x9bc9f2, 0xdda6e2, 0x74d8cb, 0xf5f1eb,
];

/// The terminal's text as designed, before Ctrl and the wheel scale it.
pub(super) const FONT: f32 = 13.0;

/// What the panel holds.
#[derive(Default)]
pub(super) struct Term {
    terminal: Option<Arc<Terminal>>,
    /// Counts shells, so an update from one already replaced is dropped.
    generation: u64,
    size: (u16, u16),
    screen: Screen,
    /// What is selected: the cell pressed and the cell dragged to, as a row
    /// and a column of the screen drawn.
    selection: Option<((u16, u16), (u16, u16))>,
}

/// A selection's two ends, the earlier first.
fn ordered(ends: ((u16, u16), (u16, u16))) -> ((u16, u16), (u16, u16)) {
    if ends.0 <= ends.1 {
        ends
    } else {
        (ends.1, ends.0)
    }
}

/// What a selection from `from` up to the column before `to` holds: each
/// row's characters whose first cell is in it — a wide one is two cells,
/// as it is drawn — with the blanks at a line's end left out and the rows
/// joined by line breaks.
fn selected_text(rows: &[rusty_term::Row], from: (u16, u16), to: (u16, u16)) -> String {
    use unicode_width::UnicodeWidthChar;
    let mut lines = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let index = index as u16;
        if index < from.0 || index > to.0 {
            continue;
        }
        let first = if index == from.0 { from.1 } else { 0 };
        let last = if index == to.0 { to.1 } else { u16::MAX };
        let mut line = String::new();
        let mut cell = 0u16;
        for c in row.spans.iter().flat_map(|span| span.text.chars()) {
            if cell >= first && cell < last {
                line.push(c);
            }
            cell = cell.saturating_add(c.width().unwrap_or(0) as u16);
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

fn rgb(value: u32) -> slint::Color {
    slint::Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// One terminal colour: the theme's sixteen, then the xterm cube and greys,
/// which are fixed by the specification. `None` is the theme's own.
fn colour(colour: Colour, dark: bool) -> Option<slint::Color> {
    match colour {
        Colour::Default => None,
        Colour::Indexed { index } if index < 16 => Some(rgb(if dark {
            DARK[usize::from(index)]
        } else {
            LIGHT[usize::from(index)]
        })),
        Colour::Indexed { index } if index < 232 => {
            // The 6×6×6 cube: 0, then 95 and 40 apart.
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Some(slint::Color::from_rgb_u8(
                level(i / 36),
                level((i % 36) / 6),
                level(i % 6),
            ))
        }
        Colour::Indexed { index } => {
            let grey = 8 + (index - 232) * 10;
            Some(slint::Color::from_rgb_u8(grey, grey, grey))
        }
        Colour::Rgb { r, g, b } => Some(slint::Color::from_rgb_u8(r, g, b)),
    }
}

/// A key as the bytes a terminal expects; `None` for keys with no terminal
/// meaning, which the window then gets.
fn encode(text: &str, control: bool, alt: bool) -> Option<Vec<u8>> {
    let is = |key: Key| text == SharedString::from(key).as_str();
    // Control codes first: Ctrl+C is 0x03, and must reach the shell rather
    // than being read as the letter C. Ctrl+Alt is AltGr on many layouts.
    if control && !alt {
        let mut chars = text.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_alphabetic() {
                return Some(vec![(c.to_ascii_uppercase() as u8) & 0x1f]);
            }
            match c {
                '[' => return Some(vec![0x1b]),
                '\\' => return Some(vec![0x1c]),
                ']' => return Some(vec![0x1d]),
                _ => {}
            }
        }
    }
    let bytes: &[u8] = if is(Key::Return) {
        b"\r"
    } else if is(Key::Backspace) {
        // DEL, not BS: every modern line editor expects 0x7f.
        b"\x7f"
    } else if is(Key::Tab) {
        b"\t"
    } else if is(Key::Escape) {
        b"\x1b"
    } else if is(Key::UpArrow) {
        b"\x1b[A"
    } else if is(Key::DownArrow) {
        b"\x1b[B"
    } else if is(Key::RightArrow) {
        b"\x1b[C"
    } else if is(Key::LeftArrow) {
        b"\x1b[D"
    } else if is(Key::Home) {
        b"\x1b[H"
    } else if is(Key::End) {
        b"\x1b[F"
    } else if is(Key::Delete) {
        b"\x1b[3~"
    } else if is(Key::PageUp) {
        b"\x1b[5~"
    } else if is(Key::PageDown) {
        b"\x1b[6~"
    } else {
        // A character is text; anything in the private-use plane is a key
        // with nothing to send.
        let mut chars = text.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) if !('\u{f700}'..='\u{f8ff}').contains(&c) && !c.is_control() => {
                Some(c.to_string().into_bytes())
            }
            _ => None,
        };
    };
    Some(bytes.to_vec())
}

/// A run cut where its cells stop being one wide: narrow characters stay
/// together, and each wide one is a piece of its own, so it is drawn in its
/// own two cells whatever width the font's fallback gives it — or every
/// cell after a line of Chinese would sit beside where the cursor says.
fn pieces(text: &str) -> Vec<(&str, usize)> {
    use unicode_width::UnicodeWidthChar;
    let mut out = Vec::new();
    let mut start = 0;
    let mut cells = 0;
    for (at, c) in text.char_indices() {
        let width = c.width().unwrap_or(0);
        if width == 2 {
            if at > start {
                out.push((&text[start..at], cells));
            }
            let end = at + c.len_utf8();
            out.push((&text[at..end], 2));
            start = end;
            cells = 0;
        } else {
            cells += width;
        }
    }
    if start < text.len() {
        out.push((&text[start..], cells));
    }
    out
}

impl App {
    /// The panel shown: a shell, if there is none.
    pub(super) fn term_ensure(self: &Rc<Self>) {
        let needed = {
            let state = self.state.borrow();
            state.term.terminal.is_none()
        };
        if needed {
            self.term_start();
        }
    }

    /// Start a shell in the project, replacing any there was.
    pub(super) fn term_start(self: &Rc<Self>) {
        let root = self.root();
        let (generation, (cols, rows)) = {
            let mut state = self.state.borrow_mut();
            if let Some(old) = state.term.terminal.take() {
                old.kill();
            }
            state.term.generation += 1;
            state.term.selection = None;
            let size = if state.term.size.0 == 0 {
                (100, 24)
            } else {
                state.term.size
            };
            (state.term.generation, size)
        };
        work::spawn(
            move || {
                let shell = rusty_term::default_shell();
                Terminal::spawn(root.as_deref(), cols, rows, Some(&[shell]))
                    .map_err(|e| e.to_string())
            },
            move |app, spawned| match spawned {
                Ok((terminal, updates)) => {
                    let terminal = Arc::new(terminal);
                    {
                        let mut state = app.state.borrow_mut();
                        if state.term.generation != generation {
                            terminal.kill();
                            return;
                        }
                        state.term.terminal = Some(terminal.clone());
                        // The panel measured itself while the shell was
                        // starting, with nothing yet to tell.
                        let size = state.term.size;
                        if size.0 != 0 && size != (cols, rows) {
                            let _ = terminal.resize(size.0, size.1);
                        }
                    }
                    std::thread::spawn(move || {
                        loop {
                            let screen = terminal.screen();
                            let over = screen.exited.is_some();
                            work::deliver(screen, move |app, screen| {
                                app.term_screen(generation, screen)
                            });
                            if over || !updates.wait() {
                                break;
                            }
                        }
                    });
                }
                Err(reason) => app.note(1, reason),
            },
        );
    }

    fn term_screen(&self, generation: u64, screen: Screen) {
        {
            let mut state = self.state.borrow_mut();
            if state.term.generation != generation {
                return;
            }
            state.term.screen = screen;
        }
        self.sync_term();
    }

    /// A key while the terminal has focus. `true` when the shell took it.
    pub(super) fn term_key(&self, text: &str, control: bool, alt: bool, shift: bool) -> bool {
        let Some(terminal) = self.state.borrow().term.terminal.clone() else {
            return false;
        };
        // Ctrl+Shift+V pastes and Ctrl+Shift+C copies; Ctrl+C copies too
        // while something is selected, as VS Code's terminal does on
        // Windows, and is the shell's interrupt otherwise.
        if control && shift && text.eq_ignore_ascii_case("v") {
            self.term_paste();
            return true;
        }
        if control && !alt && text.eq_ignore_ascii_case("c") && (shift || self.term_has_selection())
        {
            self.term_copy();
            self.term_unselect();
            return true;
        }
        let Some(bytes) = encode(text, control, alt) else {
            return false;
        };
        self.term_unselect();
        let _ = terminal.write(&bytes);
        true
    }

    /// The clipboard typed into the shell, its line breaks as Enter.
    pub(super) fn term_paste(&self) {
        let Some(terminal) = self.state.borrow().term.terminal.clone() else {
            return;
        };
        if let Ok(mut clipboard) = arboard::Clipboard::new()
            && let Ok(pasted) = clipboard.get_text()
        {
            let _ = terminal.write(pasted.replace("\r\n", "\r").replace('\n', "\r").as_bytes());
        }
    }

    fn term_has_selection(&self) -> bool {
        self.state
            .borrow()
            .term
            .selection
            .is_some_and(|(from, to)| from != to)
    }

    /// What is selected, as text; empty when nothing is.
    pub(super) fn term_selected(&self) -> String {
        let state = self.state.borrow();
        let term = &state.term;
        match term.selection.map(ordered) {
            Some((from, to)) if from != to => selected_text(&term.screen.rows, from, to),
            _ => String::new(),
        }
    }

    /// The selection onto the clipboard.
    pub(super) fn term_copy(&self) {
        let text = self.term_selected();
        if !text.is_empty() {
            super::git::copy_text(&text);
        }
    }

    /// A press (0) at a cell starts a selection there; a drag (1) carries
    /// its other end.
    pub(super) fn term_select(&self, kind: i32, col: i32, row: i32) {
        let cell = (row.max(0) as u16, col.max(0) as u16);
        {
            let mut state = self.state.borrow_mut();
            let selection = &mut state.term.selection;
            *selection = match (kind, *selection) {
                (1, Some((from, _))) => Some((from, cell)),
                _ => Some((cell, cell)),
            };
        }
        self.sync_term_selection();
    }

    /// Every cell of the screen selected.
    pub(super) fn term_select_all(&self) {
        {
            let mut state = self.state.borrow_mut();
            let term = &mut state.term;
            let rows = term.screen.rows.len().max(1) as u16;
            term.selection = Some(((0, 0), (rows - 1, term.screen.cols.max(1))));
        }
        self.sync_term_selection();
    }

    fn term_unselect(&self) {
        if self.state.borrow_mut().term.selection.take().is_some() {
            self.sync_term_selection();
        }
    }

    /// The screen and its history blanked, and the shell asked for its
    /// prompt again (`Terminal::clear`).
    pub(super) fn term_clear(&self) {
        let Some(terminal) = self.state.borrow().term.terminal.clone() else {
            return;
        };
        let _ = terminal.clear();
        {
            let mut state = self.state.borrow_mut();
            state.term.selection = None;
            state.term.screen = terminal.screen();
        }
        self.sync_term();
    }

    /// The shell ended, where it stands: the panel then says it exited and
    /// offers another.
    pub(super) fn term_kill(&self) {
        if let Some(terminal) = self.state.borrow().term.terminal.clone() {
            std::thread::spawn(move || terminal.kill());
        }
    }

    /// Whether a shell is running, which a menu offers to end.
    pub(super) fn term_alive(&self) -> bool {
        let state = self.state.borrow();
        state.term.terminal.is_some() && state.term.screen.exited.is_none()
    }

    fn sync_term_selection(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let selection = self.state.borrow().term.selection.map(ordered);
        match selection {
            Some((from, to)) if from != to => {
                shell.set_term_selected(true);
                shell.set_term_sel_row(i32::from(from.0));
                shell.set_term_sel_col(i32::from(from.1));
                shell.set_term_sel_end_row(i32::from(to.0));
                shell.set_term_sel_end_col(i32::from(to.1));
            }
            _ => shell.set_term_selected(false),
        }
    }

    /// The panel's size in cells.
    pub(super) fn term_resized(&self, cols: i32, rows: i32) {
        let size = (cols.clamp(10, 1000) as u16, rows.clamp(2, 500) as u16);
        let terminal = {
            let mut state = self.state.borrow_mut();
            if state.term.size == size {
                return;
            }
            state.term.size = size;
            state.term.terminal.clone()
        };
        // The cells a selection named have moved.
        self.term_unselect();
        if let Some(terminal) = terminal {
            let _ = terminal.resize(size.0, size.1);
        }
    }

    pub(super) fn term_scroll(&self, lines: i32) {
        let terminal = self.state.borrow().term.terminal.clone();
        if let Some(terminal) = terminal {
            self.term_unselect();
            terminal.scroll(lines);
        }
    }

    pub(super) fn sync_term(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let dark = window.global::<Theme>().get_dark();
        let state = self.state.borrow();
        let screen = &state.term.screen;
        let span_of = |text: &str, col: usize, cells: usize, style: &Style| {
            let (fg, bg) = if style.inverse {
                (style.bg, style.fg)
            } else {
                (style.fg, style.bg)
            };
            let fg = colour(fg, dark);
            let bg = colour(bg, dark);
            TermSpan {
                text: text.into(),
                col: col as i32,
                cells: cells as i32,
                fg: fg.unwrap_or_default(),
                own_fg: fg.is_some(),
                bg: bg.unwrap_or_default(),
                own_bg: bg.is_some() || style.inverse,
                bold: style.bold,
                italic: style.italic,
                underline: style.underline,
            }
        };
        let rows: Vec<TermRow> = screen
            .rows
            .iter()
            .map(|row| {
                let mut spans = Vec::new();
                let mut col = 0;
                for span in &row.spans {
                    for (piece, cells) in pieces(&span.text) {
                        spans.push(span_of(piece, col, cells, &span.style));
                        col += cells;
                    }
                }
                TermRow {
                    spans: ModelRc::new(VecModel::from(spans)),
                }
            })
            .collect();
        shell.set_term_rows(ModelRc::new(VecModel::from(rows)));
        match screen.cursor {
            Some(cursor) => {
                shell.set_term_cursor_shown(true);
                shell.set_term_cursor_row(i32::from(cursor.row));
                shell.set_term_cursor_col(i32::from(cursor.col));
            }
            None => shell.set_term_cursor_shown(false),
        }
        shell.set_term_exited(SharedString::from(match screen.exited {
            Some(code) => rusty_i18n::t!("terminal.exited", code = code),
            None => String::new(),
        }));
        drop(state);
        self.sync_term_selection();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ctrl+letter is the control code, so Ctrl+C interrupts rather than
    /// typing a `c`; Ctrl+Alt is AltGr and types its character.
    #[test]
    fn ctrl_letters_become_control_codes() {
        assert_eq!(encode("c", true, false), Some(vec![0x03]));
        assert_eq!(encode("C", true, false), Some(vec![0x03]));
        assert_eq!(encode("[", true, false), Some(vec![0x1b]));
        assert_eq!(encode("@", true, true), Some(b"@".to_vec()));
    }

    /// The keys a line editor reads arrive as the sequences it expects, and
    /// a key with nothing to send is left to the window.
    #[test]
    fn keys_become_their_sequences() {
        let key = |k: Key| SharedString::from(k).to_string();
        assert_eq!(
            encode(&key(Key::Return), false, false),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            encode(&key(Key::Backspace), false, false),
            Some(b"\x7f".to_vec())
        );
        assert_eq!(
            encode(&key(Key::UpArrow), false, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(encode("中", false, false), Some("中".as_bytes().to_vec()));
        assert_eq!(encode(&key(Key::F5), false, false), None);
    }

    /// A wide character stands alone in two cells; the runs around it keep
    /// their own count.
    #[test]
    fn a_wide_character_is_a_piece_of_its_own() {
        assert_eq!(
            pieces("ab中c文"),
            vec![("ab", 2), ("中", 2), ("c", 1), ("文", 2)]
        );
        assert_eq!(pieces("plain"), vec![("plain", 5)]);
    }

    /// A selection copies the cells it covers: part of its first and last
    /// rows, all of those between, a wide character by the cell it starts
    /// in, and no line's trailing blanks.
    #[test]
    fn a_selection_copies_the_cells_it_covers() {
        let row = |text: &str| rusty_term::Row {
            spans: vec![rusty_term::Span {
                text: text.to_string(),
                style: Style::default(),
            }],
        };
        let rows = vec![
            row("PS E:\\> echo 中文 ok"),
            row("中文 ok   "),
            row("PS E:\\> "),
        ];
        // From the `echo` to the end of the second row; `中` is two cells.
        assert_eq!(
            selected_text(&rows, (0, 8), (1, 20)),
            "echo 中文 ok\n中文 ok"
        );
        // Cells 2..4 of `中文 ok` are `文`.
        assert_eq!(selected_text(&rows, (1, 2), (1, 4)), "文");
        // Whichever way it was dragged, it is the same selection.
        assert_eq!(ordered(((1, 4), (1, 2))), ((1, 2), (1, 4)));
        assert_eq!(selected_text(&rows, (2, 0), (2, 80)), "PS E:\\>");
    }

    /// The cube's first step is 0 to 95, the rest 40 apart.
    #[test]
    fn the_xterm_cube_is_not_evenly_spaced() {
        assert_eq!(
            colour(Colour::Indexed { index: 17 }, true),
            Some(slint::Color::from_rgb_u8(0, 0, 95))
        );
        assert_eq!(
            colour(Colour::Indexed { index: 231 }, true),
            Some(slint::Color::from_rgb_u8(255, 255, 255))
        );
    }
}
