//! Inlay hints through an edit: where each one stands once the text has
//! changed under it, until the server has said again.
//!
//! A hint is drawn inside its line, so a hint left where it was while the
//! line changes lands in the middle of somebody's word — and one dropped
//! until the next answer takes its width with it, sliding the rest of the
//! line left and back under the caret. So hints move with the text, as VS
//! Code's do: an edit before a hint on its line carries it along, an edit
//! after it leaves it be, and only an edit that takes away the text a hint
//! stood in takes the hint away too.

use rusty_lsp::{
    InlayHint,
    follow::{Edit, Lines},
};

/// Carry `hints` from `old` to `new`, one edit apart — or several, taken as
/// the one span that covers them all, which drops the hints between them
/// until the server answers again rather than guessing where they went.
/// The arithmetic is `rusty_lsp::follow`'s, which the client carries its
/// diagnostics through as well.
pub fn follow(hints: &mut Vec<InlayHint>, old: &str, new: &str) {
    if hints.is_empty() || old == new {
        return;
    }
    let edit = Edit::between(old, new);
    let (before, after) = (Lines::new(old), Lines::new(new));
    hints.retain_mut(|hint| {
        let at = before.byte(hint.line, hint.col);
        let Some(to) = edit.moved(at, !hint.parameter) else {
            return false;
        };
        if to == at && at < edit.start {
            // Before the edit: the same line and column as before.
            return true;
        }
        (hint.line, hint.col) = after.line_col(to);
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(line: u32, col: u32, parameter: bool) -> InlayHint {
        InlayHint {
            line,
            col,
            label: if parameter { "x:" } else { ": f32" }.to_string(),
            parameter,
            pad_left: false,
            pad_right: parameter,
        }
    }

    fn after(hints: &[InlayHint], old: &str, new: &str) -> Vec<(u32, u32)> {
        let mut hints = hints.to_vec();
        follow(&mut hints, old, new);
        hints.iter().map(|h| (h.line, h.col)).collect()
    }

    /// Typing before a hint on its line carries it along; typing after it,
    /// or on another line, leaves it be.
    #[test]
    fn a_hint_moves_with_the_text_before_it_on_its_line() {
        let old = "let total = 1;\nlet x = 2;\n";
        let type_of_total = hint(0, 9, false);
        assert_eq!(
            after(
                std::slice::from_ref(&type_of_total),
                old,
                "let  total = 1;\nlet x = 2;\n"
            ),
            [(0, 10)]
        );
        assert_eq!(
            after(
                std::slice::from_ref(&type_of_total),
                old,
                "let total = 12;\nlet x = 2;\n"
            ),
            [(0, 9)]
        );
        assert_eq!(
            after(&[type_of_total], old, "let total = 1;\nlet xy = 2;\n"),
            [(0, 9)]
        );
    }

    /// Text typed exactly where a type sits goes before the type, which
    /// belongs to the name it follows; typed where a parameter's name sits,
    /// it goes after the name, which belongs to the argument.
    #[test]
    fn what_is_typed_at_a_hint_goes_on_the_side_it_belongs_to() {
        let old = "let total = f(1);";
        let typed = "let totals = f(1);";
        assert_eq!(after(&[hint(0, 9, false)], old, typed), [(0, 10)]);
        let argument = "let total = f(21);";
        assert_eq!(after(&[hint(0, 14, true)], old, argument), [(0, 14)]);
    }

    #[test]
    fn a_line_opened_above_moves_the_hints_below_it_down() {
        let old = "fn a() {\n    let t = 1;\n}\n";
        let new = "fn a() {\n\n    let t = 1;\n}\n";
        assert_eq!(after(&[hint(1, 9, false)], old, new), [(2, 9)]);
        let joined = "fn a() {    let t = 1;\n}\n";
        assert_eq!(after(&[hint(1, 9, false)], old, joined), [(0, 17)]);
    }

    /// The text a hint stood in taken away takes the hint with it — and
    /// only that hint.
    #[test]
    fn a_hint_inside_what_was_replaced_goes() {
        let old = "let (a, b) = pair();";
        let new = "let c = pair();";
        let hints = [hint(0, 6, false), hint(0, 9, false), hint(0, 19, false)];
        assert_eq!(after(&hints, old, new), [(0, 14)]);
    }

    /// Columns are characters: a `中` before a hint is one column however
    /// many bytes it is.
    #[test]
    fn columns_are_counted_in_characters() {
        let old = "let 中 = 1;";
        let new = "let 中文 = 1;";
        assert_eq!(after(&[hint(0, 5, false)], old, new), [(0, 6)]);
        assert_eq!(
            after(&[hint(0, 5, false)], old, "// 中\nlet 中 = 1;"),
            [(1, 5)]
        );
    }
}
