//! Inlay hints, drawn in the line as VS Code draws them: `let total: f32 =
//! both();`, `sample(sensor: &gyro)`.
//!
//! A hint takes room in its line, so from a line's first hint on, where a
//! character is drawn is not where the text alone would put it. Everything
//! that turns a column into an x or an x into a column — the pieces of a
//! row, the caret, a selection, a squiggle, a click, the popups — goes
//! through this one walk of a line's characters and its hints.
//!
//! **A hint belongs to one side of its column.** A type after a name, a
//! chain's type, a closing brace's block belong to the code before them; a
//! parameter's name to the argument after it. The caret at that column
//! stands on the side of the code the hint is not about — before `: f32`,
//! after `sensor: ` — so what is typed there lands beside what the hint
//! describes, and `rusty_lsp::follow::hints` moves hints through every edit
//! by the same rule.

use rusty_lsp::InlayHint;

use super::{Document, Metrics};

/// The space inside a hint's pill on either side of its label.
pub const INSET: f32 = 3.0;

/// The hints on `line` of `d`, in column order.
pub fn on(d: &Document, line: usize) -> &[InlayHint] {
    let line = line as u32;
    let start = d.hints.partition_point(|h| h.line < line);
    let end = start + d.hints[start..].partition_point(|h| h.line <= line);
    &d.hints[start..end]
}

/// The room a padded side of a hint takes: half a character.
pub fn pad(m: &Metrics, on: bool) -> f32 {
    if on { m.ascii() / 2.0 } else { 0.0 }
}

/// The room a hint takes in its line: its label inside its pill, and its
/// padding.
pub fn width(m: &Metrics, hint: &InlayHint) -> f32 {
    m.x_of(&hint.label, usize::MAX) + 2.0 * INSET + pad(m, hint.pad_left) + pad(m, hint.pad_right)
}

/// One column of a line as drawn: where the code before its hints ends,
/// where the caret stands, and where the character at it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stop {
    before: f32,
    caret: f32,
    start: f32,
}

/// Walk `text` with its `hints`, calling `at` for every column up to and
/// including the end until it answers `true`; the x after everything.
fn walk(
    m: &Metrics,
    text: &str,
    hints: &[InlayHint],
    mut at: impl FnMut(usize, Stop, f32) -> bool,
) -> f32 {
    let mut x = 0.0;
    let mut next = 0;
    let mut chars = text.chars();
    let mut col = 0usize;
    loop {
        let before = x;
        let mut parameter = false;
        while let Some(hint) = hints.get(next).filter(|h| h.col as usize <= col) {
            x += width(m, hint);
            parameter |= hint.parameter;
            next += 1;
        }
        let ch = chars.next();
        let advance = ch.map_or(0.0, |ch| m.advance(ch, x));
        let stop = Stop {
            before,
            caret: if parameter { x } else { before },
            start: x,
        };
        if at(col, stop, advance) {
            return x;
        }
        let Some(_) = ch else {
            return x;
        };
        x += advance;
        col += 1;
    }
}

/// Where the caret at `col` stands.
pub fn caret_x(m: &Metrics, text: &str, hints: &[InlayHint], col: usize) -> f32 {
    let mut found = 0.0;
    walk(m, text, hints, |c, stop, _| {
        found = stop.caret;
        c >= col
    });
    found
}

/// Where the character at `col` starts — after any hints at its column.
pub fn char_x(m: &Metrics, text: &str, hints: &[InlayHint], col: usize) -> f32 {
    let mut found = 0.0;
    walk(m, text, hints, |c, stop, _| {
        found = stop.start;
        c >= col
    });
    found
}

/// The column a point `x` along the line is nearest, as a caret goes: a
/// point on a hint is the column the hint stands at.
pub fn column_at(m: &Metrics, text: &str, hints: &[InlayHint], x: f32) -> usize {
    let mut found = 0;
    walk(m, text, hints, |c, stop, advance| {
        found = c;
        x < stop.start + advance / 2.0
    });
    found
}

/// Where everything on the line ends, its last hints included.
pub fn end_x(m: &Metrics, text: &str, hints: &[InlayHint]) -> f32 {
    walk(m, text, hints, |_, _, _| false)
}

/// The same four for a line of a document.
pub fn caret_on(m: &Metrics, d: &Document, line: usize, col: usize) -> f32 {
    caret_x(m, d.lines.text(&d.text, line), on(d, line), col)
}

pub fn char_on(m: &Metrics, d: &Document, line: usize, col: usize) -> f32 {
    char_x(m, d.lines.text(&d.text, line), on(d, line), col)
}

pub fn column_on(m: &Metrics, d: &Document, line: usize, x: f32) -> usize {
    column_at(m, d.lines.text(&d.text, line), on(d, line), x)
}

pub fn end_on(m: &Metrics, d: &Document, line: usize) -> f32 {
    end_x(m, d.lines.text(&d.text, line), on(d, line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(col: u32, label: &str, parameter: bool) -> InlayHint {
        InlayHint {
            line: 0,
            col,
            label: label.into(),
            parameter,
            pad_left: false,
            pad_right: false,
        }
    }

    /// Characters ten pixels wide; a hint of five characters is 56 wide.
    fn m() -> Metrics {
        Metrics::fixed(10.0)
    }

    /// `let total: f32 = 1;` with `: f32` after `total`: the caret at its
    /// column stands before it, and the characters after it are pushed
    /// along by its width.
    #[test]
    fn a_type_pushes_the_line_along_and_the_caret_stands_before_it() {
        let text = "let total = 1;";
        let hints = [hint(9, ": f32", false)];
        let w = width(&m(), &hints[0]);
        assert_eq!(w, 50.0 + 2.0 * INSET);
        assert_eq!(caret_x(&m(), text, &hints, 9), 90.0);
        assert_eq!(char_x(&m(), text, &hints, 9), 90.0 + w);
        assert_eq!(caret_x(&m(), text, &hints, 10), 100.0 + w);
        assert_eq!(caret_x(&m(), text, &hints, 3), 30.0, "before it, untouched");
        assert_eq!(end_x(&m(), text, &hints), 140.0 + w);
    }

    /// `f(sensor: x)`: the caret at the argument's column stands after the
    /// parameter's name, before the argument it belongs to.
    #[test]
    fn a_parameters_name_has_the_caret_after_it() {
        let text = "f(x)";
        let hints = [hint(2, "sensor:", true)];
        let w = width(&m(), &hints[0]);
        assert_eq!(caret_x(&m(), text, &hints, 2), 20.0 + w);
        assert_eq!(char_x(&m(), text, &hints, 2), 20.0 + w);
    }

    /// A click on a hint is the column it stands at; a click on a character
    /// past it is that character's column, not one a hint's width further.
    #[test]
    fn a_click_finds_the_column_through_the_hints() {
        let text = "let total = 1;";
        let hints = [hint(9, ": f32", false)];
        let w = width(&m(), &hints[0]);
        assert_eq!(column_at(&m(), text, &hints, 95.0), 9, "on the hint");
        assert_eq!(column_at(&m(), text, &hints, 90.0 + w + 12.0), 10);
        assert_eq!(column_at(&m(), text, &hints, 32.0), 3);
        assert_eq!(column_at(&m(), text, &hints, 10_000.0), 14);
    }

    /// With no hints the walk is the text's own arithmetic.
    #[test]
    fn without_hints_it_is_the_text() {
        let text = "a\tb";
        for col in 0..=3 {
            assert_eq!(caret_x(&m(), text, &[], col), m().x_of(text, col));
        }
        assert_eq!(column_at(&m(), text, &[], 41.0), m().column_at(text, 41.0));
    }
}
