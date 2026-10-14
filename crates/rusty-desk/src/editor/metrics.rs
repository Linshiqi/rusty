//! Where each character of a line sits, in logical pixels.
//!
//! The editor's font is monospaced, so every ASCII character is one width,
//! measured once. Anything else — a `中` in a comment, drawn from a fallback
//! font — is measured the first time it is seen and remembered. A tab goes to
//! the next stop of four columns. Every position the editor names — a piece
//! of a row, the caret, the column under a click — comes from here, so they
//! cannot disagree.

use std::collections::HashMap;

use crate::AppWindow;

pub const TAB_COLUMNS: f32 = 4.0;

pub struct Metrics {
    ascii: f32,
    wide: HashMap<char, f32>,
}

impl Metrics {
    /// The ASCII advance, measured over a run so rounding in one glyph does
    /// not multiply down a long line.
    pub fn measure(window: &AppWindow) -> Metrics {
        const RUN: usize = 64;
        let ascii = width_of(window, &"M".repeat(RUN)) / RUN as f32;
        Metrics {
            ascii: if ascii > 0.0 { ascii } else { 7.5 },
            wide: HashMap::new(),
        }
    }

    /// Measure every character of `text` not yet known — over a run, as the
    /// ASCII width is: a width measured once is rounded, and the rounding of
    /// one glyph was eight pixels by the end of a line of Chinese.
    pub fn learn(&mut self, window: &AppWindow, text: &str) {
        const RUN: usize = 16;
        for ch in text.chars() {
            if !ch.is_ascii() && !self.wide.contains_key(&ch) {
                let width = width_of(window, &ch.to_string().repeat(RUN)) / RUN as f32;
                self.wide
                    .insert(ch, if width > 0.0 { width } else { self.ascii * 2.0 });
            }
        }
    }

    pub fn advance(&self, ch: char, x: f32) -> f32 {
        match ch {
            '\t' => {
                let stop = self.ascii * TAB_COLUMNS;
                (x / stop).floor() * stop + stop - x
            }
            ch if ch.is_ascii() => self.ascii,
            ch => self.wide.get(&ch).copied().unwrap_or(self.ascii * 2.0),
        }
    }

    /// Where the caret at character `col` stands.
    pub fn x_of(&self, text: &str, col: usize) -> f32 {
        let mut x = 0.0;
        for ch in text.chars().take(col) {
            x += self.advance(ch, x);
        }
        x
    }

    /// The character boundary nearest to `x`.
    pub fn column_at(&self, text: &str, x: f32) -> usize {
        let mut at = 0.0;
        for (col, ch) in text.chars().enumerate() {
            let width = self.advance(ch, at);
            if x < at + width / 2.0 {
                return col;
            }
            at += width;
        }
        text.chars().count()
    }

    /// One ASCII character's width.
    pub fn ascii(&self) -> f32 {
        self.ascii
    }

    /// The widest line of a document, for how far the view scrolls sideways.
    pub fn widest(&self, doc: &super::Document) -> f32 {
        (0..doc.lines.count())
            .map(|line| self.x_of(doc.lines.text(&doc.text, line), usize::MAX))
            .fold(0.0, f32::max)
    }
}

fn width_of(window: &AppWindow, text: &str) -> f32 {
    window.set_probe_text(text.into());
    window.get_probe_width()
}
