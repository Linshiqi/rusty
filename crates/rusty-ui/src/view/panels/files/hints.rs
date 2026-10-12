//! Inlay hints, drawn where they belong: `let total: f32 = both();`.
//!
//! The echo draws a hint inside its line, as VS Code does, and the rest of
//! the line moves over to make room. The textarea above it cannot: it holds
//! the file and lays out nothing but the file, so from a line's first hint
//! on, the two layers no longer agree about where a character is. Everything
//! that used to read the textarea's layout therefore reads this module's
//! instead — the caret and the selection are drawn (`selection.rs`), a press
//! is placed by [`HintedLine::caret_col_at`] rather than by the browser
//! (`pointer.rs`), every overlay measures a column through [`HintedLine`],
//! and while an input method composes the textarea is shifted so its caret,
//! the one the input method reads, stands where the drawn one does.
//!
//! **A hint belongs to one side of its column.** A type after a name, a
//! chain's type, a closing brace's block belong to the code before them; a
//! parameter's name belongs to the argument after it. The caret at a hint's
//! column stands on the far side of it from the code it is not about, so
//! what is typed there lands beside the code the hint describes — `total`
//! typed on to `totals` keeps its type after it — and `crate::inlay` moves
//! the hint the same way when the text changes under it.

use leptos::prelude::*;
use rusty_lsp::InlayHint;

use super::*;
use crate::state::{AppState, HintSet};

/// A hint as its line draws it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Placed {
    /// The column it is drawn at: before the character there, after every
    /// character before it.
    pub col: u32,
    pub label: String,
    pub pad_left: bool,
    pub pad_right: bool,
    /// A parameter's name, which belongs to the argument after it.
    pub parameter: bool,
}

/// The hints on document line `line` of `path`, in the order they are
/// drawn: by column, and at one column the ones about the code before it
/// first, so each stands against the code it describes.
pub(super) fn placed_on(set: Option<&HintSet>, path: &str, line: u32) -> Vec<Placed> {
    let Some(set) = set.filter(|set| set.path == path) else {
        return Vec::new();
    };
    let from = set.hints.partition_point(|hint| hint.line < line);
    let mut placed: Vec<Placed> = set.hints[from..]
        .iter()
        .take_while(|hint| hint.line == line)
        .filter_map(place)
        .collect();
    placed.sort_by_key(|hint| (hint.col, hint.parameter));
    placed
}

/// One hint as drawn, or nothing for a label with nothing to draw.
fn place(hint: &InlayHint) -> Option<Placed> {
    // A tab or a line break inside a hint would be laid out as one by the
    // echo and measured as something else here.
    let label: String = hint
        .label
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if label.trim().is_empty() {
        return None;
    }
    Some(Placed {
        col: hint.col,
        label,
        pad_left: hint.pad_left,
        pad_right: hint.pad_right,
        parameter: hint.parameter,
    })
}

/// The hints of a line of the file on screen, while hints are shown —
/// tracked, for what is drawn.
pub(super) fn hints_on(state: AppState, path: &str, line: u32) -> Vec<Placed> {
    if !state.editor.view.with(|view| view.inlay_hints) {
        return Vec::new();
    }
    state
        .editor
        .hints
        .with(|set| placed_on(set.as_ref(), path, line))
}

/// The same, untracked — for a press, a key, anything that is not drawing.
pub(super) fn hints_on_now(state: AppState, line: u32) -> Vec<Placed> {
    if !state.editor.view.with_untracked(|view| view.inlay_hints) {
        return Vec::new();
    }
    let Some(path) = state.active_path_now() else {
        return Vec::new();
    };
    state
        .editor
        .hints
        .with_untracked(|set| placed_on(set.as_ref(), &path, line))
}

/// What a line is drawn as, piece by piece.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Piece {
    /// A hint, at column `col`.
    Hint {
        col: u32,
    },
    Char {
        col: u32,
    },
}

/// A line of text and the hints drawn in it: the one measure of where
/// anything on a line is, at zoom 1, in pixels from the line's start.
#[derive(Clone, Copy)]
pub(super) struct HintedLine<'a> {
    pub text: &'a str,
    /// In drawing order, as [`placed_on`] returns them.
    pub hints: &'a [Placed],
}

impl HintedLine<'_> {
    /// Walk the line as it is drawn, calling `visit` with each piece and
    /// where it starts and ends until it answers false; the pen's position
    /// at the end.
    ///
    /// A tab goes to its stop from wherever the pen is, hints included —
    /// the echo lays the hint out in the line, so the stop after it is
    /// counted from past it, and [`pen_after`] is the rule both keep.
    fn walk(
        &self,
        advance: &dyn Fn(char) -> f64,
        mut visit: impl FnMut(Piece, f64, f64) -> bool,
    ) -> f64 {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len() as u32;
        let mut x = 0.0;
        let mut next = 0;
        for col in 0..=len {
            // A hint past the end of its line — the line shortened under
            // it before the next answer — is drawn at the end.
            while let Some(hint) = self.hints.get(next).filter(|h| h.col.min(len) == col) {
                let to = x + hint_px(hint, advance);
                if !visit(Piece::Hint { col }, x, to) {
                    return to;
                }
                x = to;
                next += 1;
            }
            if let Some(&ch) = chars.get(col as usize) {
                let to = pen_after(x, ch, advance);
                if !visit(Piece::Char { col }, x, to) {
                    return to;
                }
                x = to;
            }
        }
        x
    }

    /// Where the line's column `col` begins: after everything before it and
    /// before any hint there.
    pub(super) fn edge_px(&self, col: u32, advance: &dyn Fn(char) -> f64) -> f64 {
        let mut at = None;
        let end = self.walk(advance, |piece, from, _| match piece {
            Piece::Hint { col: c } | Piece::Char { col: c } if c >= col => {
                at = Some(from);
                false
            }
            _ => true,
        });
        at.unwrap_or(end)
    }

    /// Where the character at `col` starts: past every hint at its column.
    pub(super) fn char_px(&self, col: u32, advance: &dyn Fn(char) -> f64) -> f64 {
        let mut at = None;
        let end = self.walk(advance, |piece, from, _| match piece {
            Piece::Char { col: c } if c >= col => {
                at = Some(from);
                false
            }
            _ => true,
        });
        at.unwrap_or(end)
    }

    /// Where the caret at `col` stands: before a hint about the code behind
    /// it, after a parameter's name — beside the code either is about.
    pub(super) fn caret_px(&self, col: u32, advance: &dyn Fn(char) -> f64) -> f64 {
        let len = self.text.chars().count() as u32;
        let at = col.min(len);
        let about_before = self
            .hints
            .iter()
            .any(|hint| hint.col.min(len) == at && !hint.parameter);
        if about_before {
            self.edge_px(at, advance)
        } else {
            self.char_px(at, advance)
        }
    }

    /// The whole line as drawn, every hint included.
    pub(super) fn width_px(&self, advance: &dyn Fn(char) -> f64) -> f64 {
        self.walk(advance, |_, _, _| true)
    }

    /// The caret column nearest `x`: the nearer side of the character
    /// under it, and a hint's own column for anywhere on the hint — a hint
    /// is not text, and the one place it stands for is where it is.
    pub(super) fn caret_col_at(&self, x: f64, advance: &dyn Fn(char) -> f64) -> u32 {
        let mut at = None;
        self.walk(advance, |piece, from, to| {
            if x >= to {
                return true;
            }
            at = Some(match piece {
                Piece::Hint { col } => col,
                Piece::Char { col } if x < (from + to) / 2.0 => col,
                Piece::Char { col } => col + 1,
            });
            false
        });
        at.unwrap_or(self.text.chars().count() as u32)
    }

    /// The column of the character under `x`, for hover and Ctrl+click —
    /// the last column past the end of the line, where "what is this?"
    /// still means the token the line ends with, and nothing over a hint,
    /// which is no token.
    pub(super) fn col_under(&self, x: f64, advance: &dyn Fn(char) -> f64) -> Option<u32> {
        let mut at = None;
        let mut over_hint = false;
        self.walk(advance, |piece, _, to| {
            if x >= to {
                return true;
            }
            match piece {
                Piece::Hint { .. } => over_hint = true,
                Piece::Char { col } => at = Some(col),
            }
            false
        });
        if over_hint {
            return None;
        }
        Some(at.unwrap_or(self.text.chars().count() as u32))
    }

    /// How far the hints before the caret at `col` push it along: what an
    /// input method, which reads the textarea's own caret, is told the
    /// caret is off by.
    pub(super) fn caret_shift_px(&self, col: u32, advance: &dyn Fn(char) -> f64) -> f64 {
        let bare = HintedLine {
            text: self.text,
            hints: &[],
        };
        self.caret_px(col, advance) - bare.caret_px(col, advance)
    }
}

/// How wide a hint is drawn: its label, and a space either side it asked for.
fn hint_px(hint: &Placed, advance: &dyn Fn(char) -> f64) -> f64 {
    let pads = f64::from(u8::from(hint.pad_left) + u8::from(hint.pad_right)) * advance(' ');
    pads + hint.label.chars().map(advance).sum::<f64>()
}

// ─── in the editor's pixels ──────────────────────────────────────────────────

/// Where the caret at a column is drawn, in pixels from the text column's
/// left edge at this zoom.
pub(super) fn caret_left(text: &str, hints: &[Placed], col: u32, zoom: f64) -> f64 {
    PAD_PX + HintedLine { text, hints }.caret_px(col, &advance_of) * zoom
}

/// Where the character at a column starts.
pub(super) fn char_left(text: &str, hints: &[Placed], col: u32, zoom: f64) -> f64 {
    PAD_PX + HintedLine { text, hints }.char_px(col, &advance_of) * zoom
}

/// Where a column begins, before any hint at it: the right end of a range
/// that stops there.
pub(super) fn edge_left(text: &str, hints: &[Placed], col: u32, zoom: f64) -> f64 {
    PAD_PX + HintedLine { text, hints }.edge_px(col, &advance_of) * zoom
}

/// Where everything drawn on a line ends.
pub(super) fn line_right(text: &str, hints: &[Placed], zoom: f64) -> f64 {
    PAD_PX + HintedLine { text, hints }.width_px(&advance_of) * zoom
}

/// The row and caret column a point in the text column falls on, for a
/// press: the nearest row, the nearest caret column on it. `x` and `y` are
/// pixels from the text column's corner.
pub(super) fn caret_at_point(state: AppState, x: f64, y: f64) -> (u32, u32) {
    let zoom = state.editor.zoom.get_untracked();
    let rows = state.editor.folds.with_untracked(|folds| {
        folds.rows(state.editor.highlighted.with_untracked(Vec::len).max(1) as u32)
    });
    let row = ((y - PAD_PX) / row_height(zoom))
        .floor()
        .clamp(0.0, f64::from(rows.saturating_sub(1))) as u32;
    let line = line_of_row(state, row);
    let text = state.editor.draft.with_untracked(|draft| {
        draft
            .split('\n')
            .nth(line as usize)
            .unwrap_or_default()
            .to_string()
    });
    let hints = hints_on_now(state, line);
    let col = HintedLine {
        text: &text,
        hints: &hints,
    }
    .caret_col_at((x - PAD_PX) / zoom, &advance_of);
    (row, col)
}

/// The row and the column of the character a point is over, for hover and
/// Ctrl+click: nothing above the text or over a hint.
pub(super) fn cell_at_point(state: AppState, x: f64, y: f64) -> Option<(u32, u32)> {
    let zoom = state.editor.zoom.get_untracked();
    let row = ((y - PAD_PX) / row_height(zoom)).floor();
    if row < 0.0 {
        return None;
    }
    let row = row as u32;
    let line = line_of_row(state, row);
    let text = state
        .editor
        .draft
        .with_untracked(|draft| draft.split('\n').nth(line as usize).map(str::to_string))?;
    // Below the last line is no line at all.
    if state
        .editor
        .folds
        .with_untracked(|folds| folds.view_of_doc(line))
        != Some(row)
    {
        return None;
    }
    let hints = hints_on_now(state, line);
    let col = HintedLine {
        text: &text,
        hints: &hints,
    }
    .col_under(((x - PAD_PX) / zoom).max(0.0), &advance_of)?;
    Some((row, col))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character one cell wide, `中` two — what the editor's
    /// monospace font does.
    fn cell(ch: char) -> f64 {
        if ch == '中' { 2.0 } else { 1.0 }
    }

    fn hint(col: u32, label: &str, parameter: bool) -> Placed {
        Placed {
            col,
            label: label.to_string(),
            pad_left: false,
            pad_right: parameter,
            parameter,
        }
    }

    const LET: &str = "let r = q;";

    /// A type after `r`: the characters from there on move over by its
    /// width, and nothing before it moves.
    #[test]
    fn a_hint_pushes_the_rest_of_its_line_along() {
        let hints = [hint(5, ": Quat", false)];
        let line = HintedLine {
            text: LET,
            hints: &hints,
        };
        assert_eq!(line.char_px(4, &cell), 4.0, "r");
        assert_eq!(line.char_px(5, &cell), 11.0, "the space after the hint");
        assert_eq!(line.char_px(9, &cell), 15.0, "the semicolon");
        assert_eq!(line.width_px(&cell), 16.0);
        assert_eq!(line.edge_px(5, &cell), 5.0, "a range ending at r ends at r");
    }

    /// The caret after `r` stands before the type, which is about `r`, so
    /// what is typed there extends the name; the caret before an argument
    /// stands after the parameter's name, which is about the argument.
    #[test]
    fn the_caret_at_a_hint_stands_beside_the_code_the_hint_is_about() {
        let typed = [hint(5, ": Quat", false)];
        let line = HintedLine {
            text: LET,
            hints: &typed,
        };
        assert_eq!(line.caret_px(5, &cell), 5.0);
        assert_eq!(line.caret_px(6, &cell), 12.0);
        let text = "f(1, 2)";
        let names = [hint(2, "a:", true), hint(5, "b:", true)];
        let call = HintedLine {
            text,
            hints: &names,
        };
        assert_eq!(call.caret_px(2, &cell), 5.0, "after `a: `, before 1");
        assert_eq!(call.caret_px(1, &cell), 1.0, "before the parenthesis");
        assert_eq!(call.char_px(5, &cell), 11.0, "2, after `a: ` and `b: `");
    }

    /// A press goes to the nearer side of a character, and anywhere on a
    /// hint to the hint's own column; past the end, to the end.
    #[test]
    fn a_press_lands_on_the_nearest_caret_column() {
        let hints = [hint(5, ": Quat", false)];
        let line = HintedLine {
            text: LET,
            hints: &hints,
        };
        assert_eq!(line.caret_col_at(4.2, &cell), 4, "left half of r");
        assert_eq!(line.caret_col_at(4.7, &cell), 5, "right half of r");
        assert_eq!(line.caret_col_at(8.0, &cell), 5, "on the hint");
        assert_eq!(line.caret_col_at(11.6, &cell), 6, "right half of the space");
        assert_eq!(line.caret_col_at(40.0, &cell), 10, "past the end");
        assert_eq!(line.caret_col_at(-3.0, &cell), 0);
    }

    /// Hover reads the character under the pointer — the right one after a
    /// hint — and nothing over the hint itself.
    #[test]
    fn hover_finds_the_character_and_not_the_hint() {
        let hints = [hint(5, ": Quat", false)];
        let line = HintedLine {
            text: LET,
            hints: &hints,
        };
        assert_eq!(line.col_under(4.5, &cell), Some(4));
        assert_eq!(line.col_under(7.0, &cell), None);
        assert_eq!(line.col_under(14.5, &cell), Some(8), "q, past the hint");
        assert_eq!(line.col_under(99.0, &cell), Some(10));
    }

    /// A tab goes to its stop counted from past the hints before it, as
    /// the echo lays it out.
    #[test]
    fn a_tab_after_a_hint_goes_to_the_stop_after_the_hint() {
        let hints = [hint(1, ":i", false)];
        let line = HintedLine {
            text: "a\tb",
            hints: &hints,
        };
        // `a:i` is three cells, so the tab runs to four and `b` starts there.
        assert_eq!(line.char_px(2, &cell), 4.0);
        let wide = [hint(1, ":xyz", false)];
        let pushed = HintedLine {
            text: "a\tb",
            hints: &wide,
        };
        // Five cells: the next stop is eight.
        assert_eq!(pushed.char_px(2, &cell), 8.0);
    }

    /// Padding is a space either side; a `中` is two cells before a hint as
    /// anywhere else.
    #[test]
    fn padding_and_wide_characters_count_as_drawn() {
        let chain = Placed {
            col: 3,
            label: "u8".to_string(),
            pad_left: true,
            pad_right: false,
            parameter: false,
        };
        let line = HintedLine {
            text: "中文;",
            hints: std::slice::from_ref(&chain),
        };
        // `中` two cells, `文` and `;` one each here, then ` u8`.
        assert_eq!(line.width_px(&cell), 4.0 + 3.0);
        assert_eq!(line.caret_px(3, &cell), 4.0, "before the chain's type");
        assert_eq!(line.caret_shift_px(3, &cell), 0.0);
        let typed = [hint(1, ": T", false)];
        let before = HintedLine {
            text: "ab",
            hints: &typed,
        };
        assert_eq!(before.caret_shift_px(2, &cell), 3.0);
    }

    /// Hints come in drawing order: by column, and at one column the type
    /// about the code before it ahead of the name of the argument after.
    #[test]
    fn hints_are_placed_in_the_order_they_are_drawn() {
        let lsp = |line, col, label: &str, parameter| InlayHint {
            line,
            col,
            label: label.to_string(),
            parameter,
            pad_left: false,
            pad_right: parameter,
        };
        let set = HintSet {
            path: "src/main.rs".to_string(),
            lines: (0, 10),
            hints: vec![
                lsp(0, 3, ": u8", false),
                lsp(1, 4, "b:", true),
                lsp(1, 4, ": T", false),
                lsp(1, 2, "\t", false),
                lsp(2, 0, "x", false),
            ],
        };
        let placed: Vec<(u32, String)> = placed_on(Some(&set), "src/main.rs", 1)
            .into_iter()
            .map(|p| (p.col, p.label))
            .collect();
        assert_eq!(
            placed,
            [(4, ": T".to_string()), (4, "b:".to_string())],
            "an empty label draws nothing"
        );
        assert!(placed_on(Some(&set), "src/lib.rs", 1).is_empty());
    }
}
