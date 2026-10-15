//! What the server said about one text, carried to the next.
//!
//! The server's word on a file — a diagnostic, an inlay hint — is a line and
//! a column in the text it was asked about, and the text moves on under it
//! with every keystroke. Whoever holds one until the server speaks again
//! carries it across each edit by one rule, VS Code's for its decorations: an
//! edit before a position carries it along, an edit after it leaves it be,
//! and an edit that takes away the text a thing was about takes the thing
//! away. The client follows the diagnostics it holds (`diagnostics`, below)
//! and the editor its inlay hints, so the two cannot disagree about where a
//! line went.

use crate::model::{FileDiagnostic, InlayHint, SemanticSpan};

/// The one span two texts differ in: `start..old_end` of the old text became
/// `start..new_end` of the new, all three on character boundaries. Several
/// edits at once are taken as the one span that covers them all, which
/// drops what stood between them rather than guessing where it went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

impl Edit {
    pub fn between(old: &str, new: &str) -> Self {
        let (a, b) = (old.as_bytes(), new.as_bytes());
        let mut prefix = 0;
        while prefix < a.len().min(b.len()) && a[prefix] == b[prefix] {
            prefix += 1;
        }
        while prefix > 0 && !(old.is_char_boundary(prefix) && new.is_char_boundary(prefix)) {
            prefix -= 1;
        }
        let mut suffix = 0;
        while suffix < (a.len() - prefix).min(b.len() - prefix)
            && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
        {
            suffix += 1;
        }
        while suffix > 0
            && !(old.is_char_boundary(a.len() - suffix) && new.is_char_boundary(b.len() - suffix))
        {
            suffix -= 1;
        }
        Self {
            start: prefix,
            old_end: a.len() - suffix,
            new_end: b.len() - suffix,
        }
    }

    /// Where a byte past the edit lands.
    fn shifted(self, at: usize) -> usize {
        at - self.old_end + self.new_end
    }

    /// Where a position goes. One the edit left alone is where it was, or
    /// as far along as the edit grew or shrank the text before it; one
    /// strictly inside what was replaced has nowhere to be. Text typed
    /// exactly at it goes before it when `follows_what_is_typed` — a type
    /// hint after `total`, typed on to `totals` — and after it otherwise, as
    /// a parameter's name belongs to the argument after it.
    pub fn moved(self, at: usize, follows_what_is_typed: bool) -> Option<usize> {
        if at < self.start {
            Some(at)
        } else if at > self.old_end {
            Some(self.shifted(at))
        } else if at == self.start && at == self.old_end {
            Some(if follows_what_is_typed {
                self.new_end
            } else {
                at
            })
        } else if at == self.start {
            Some(at)
        } else if at == self.old_end {
            Some(self.new_end)
        } else {
            None
        }
    }

    /// Where a span the server said something about goes, or `None` once
    /// the edit has touched its text and what was said is about text that is
    /// not there any more. Typing at either edge of a span does not grow it
    /// — VS Code's `NeverGrowsWhenTypingAtEdges` — so a warning over `foo`
    /// stays over `foo` while `foo.` is typed after it. A point — `expected
    /// SEMICOLON`, no columns wide — is about the text on both sides of it,
    /// so any edit at it takes it.
    pub fn span(self, from: usize, to: usize) -> Option<(usize, usize)> {
        let ahead = if from == to {
            to < self.start
        } else {
            to <= self.start
        };
        let behind = if from == to {
            from > self.old_end
        } else {
            from >= self.old_end
        };
        if ahead {
            Some((from, to))
        } else if behind {
            Some((self.shifted(from), self.shifted(to)))
        } else {
            None
        }
    }
}

/// A text's lines, for turning a line and a scalar column into a byte and
/// back.
pub struct Lines<'t> {
    text: &'t str,
    starts: Vec<usize>,
}

impl<'t> Lines<'t> {
    pub fn new(text: &'t str) -> Self {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        Self { text, starts }
    }

    /// The byte a line and a scalar column name, clamped to the line's end
    /// — and to the text's, for a line it does not have.
    pub fn byte(&self, line: u32, col: u32) -> usize {
        let Some(&start) = self.starts.get(line as usize) else {
            return self.text.len();
        };
        let end = self
            .starts
            .get(line as usize + 1)
            .map_or(self.text.len(), |next| next - 1);
        let content = &self.text[start..end];
        start
            + content
                .char_indices()
                .nth(col as usize)
                .map_or(content.len(), |(at, _)| at)
    }

    /// The line and scalar column of a byte.
    pub fn line_col(&self, byte: usize) -> (u32, u32) {
        let line = self
            .starts
            .partition_point(|&start| start <= byte)
            .saturating_sub(1);
        let col = self.text[self.starts[line]..byte].chars().count();
        (line as u32, col as u32)
    }
}

/// Carry `items` from `old` to `new`, one edit apart: each moved with the
/// text before it, and gone where the edit touched the text it was about.
/// Whether anything moved or went, so a holder says so only when it did.
///
/// The check's diagnostics are the reason. rust-analyzer runs `cargo check`
/// on a save and at no other time, so what it found stays until the next
/// one — and a `;` typed for a `,`, saved, and mended kept `struct fields
/// are separated by ,` under the `,` that mended it, in the editor and in
/// Problems, until somebody saved again.
pub fn diagnostics(items: &mut Vec<FileDiagnostic>, old: &str, new: &str) -> bool {
    if items.is_empty() || old == new {
        return false;
    }
    let edit = Edit::between(old, new);
    let (before, after) = (Lines::new(old), Lines::new(new));
    let mut changed = false;
    items.retain_mut(|item| {
        let from = before.byte(item.start_line, item.start_col);
        let to = before.byte(item.end_line, item.end_col).max(from);
        match edit.span(from, to) {
            None => {
                changed = true;
                false
            }
            // Before the edit: the same lines and columns, clamped or not.
            Some(span) if span == (from, to) => true,
            Some((from, to)) => {
                (item.start_line, item.start_col) = after.line_col(from);
                (item.end_line, item.end_col) = after.line_col(to);
                changed = true;
                true
            }
        }
    });
    changed
}

/// Carry inlay hints from `old` to `new`, one edit apart — or several,
/// taken as the one span that covers them all, which drops the hints
/// between them until the server answers again rather than guessing where
/// they went. A hint is drawn inside its line, so one left where it was
/// while the line changes lands in the middle of somebody's word, and one
/// dropped until the next answer takes its width with it, sliding the rest
/// of the line under the caret and back. So hints move with the text, as VS
/// Code's do: an edit before a hint on its line carries it along, one after
/// it leaves it be, and only one that takes away the text a hint stood in
/// takes the hint too. Text typed exactly at a hint goes on the side it is
/// not about: after a type, which belongs to the name before it; before a
/// parameter's name, which belongs to the argument after it.
pub fn hints(hints: &mut Vec<InlayHint>, old: &str, new: &str) {
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

/// Carry semantic spans from `old` to `new`, one edit apart: each moved
/// with the text before it, and gone where the edit touched the text it
/// coloured — the painter's colour shows there until the server answers.
pub fn semantic(spans: &mut Vec<SemanticSpan>, old: &str, new: &str) {
    if spans.is_empty() || old == new {
        return;
    }
    let edit = Edit::between(old, new);
    let (before, after) = (Lines::new(old), Lines::new(new));
    spans.retain_mut(|span| {
        let from = before.byte(span.line, span.start_col);
        let to = before.byte(span.line, span.start_col + span.length);
        match edit.span(from, to) {
            None => false,
            Some(same) if same == (from, to) => true,
            Some((from, to)) => {
                let (line, col) = after.line_col(from);
                let (end_line, end) = after.line_col(to);
                (span.line, span.start_col, span.length) = (line, col, end - col);
                line == end_line
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DiagSeverity;

    fn diagnostic(start: (u32, u32), end: (u32, u32)) -> FileDiagnostic {
        FileDiagnostic {
            severity: DiagSeverity::Error,
            message: "struct fields are separated by `,`".into(),
            source: Some("rustc".into()),
            code: None,
            start_line: start.0,
            start_col: start.1,
            end_line: end.0,
            end_col: end.1,
        }
    }

    fn after(items: &[FileDiagnostic], old: &str, new: &str) -> Vec<((u32, u32), (u32, u32))> {
        let mut items = items.to_vec();
        diagnostics(&mut items, old, new);
        items
            .iter()
            .map(|d| ((d.start_line, d.start_col), (d.end_line, d.end_col)))
            .collect()
    }

    const BROKEN: &str = "pub struct C {\n    pub acc: f32;\n    pub level: f32,\n}\n";
    const MENDED: &str = "pub struct C {\n    pub acc: f32,\n    pub level: f32,\n}\n";

    /// The report: the check's error over the `;`, and the `;` mended to a
    /// `,` without a save. The error was about the `;`, and goes with it —
    /// typed over in one edit or deleted and retyped in two.
    #[test]
    fn the_text_an_error_was_about_mended_takes_the_error() {
        let semicolon = diagnostic((1, 16), (1, 17));
        assert!(after(std::slice::from_ref(&semicolon), BROKEN, MENDED).is_empty());
        let deleted = "pub struct C {\n    pub acc: f32\n    pub level: f32,\n}\n";
        assert!(after(std::slice::from_ref(&semicolon), BROKEN, deleted).is_empty());
        // rust-analyzer's own `expected COMMA` is a point before the `;`.
        let point = diagnostic((1, 16), (1, 16));
        assert!(after(&[point], BROKEN, MENDED).is_empty());
    }

    /// Nothing else it says goes with it: what is before the edit stays as
    /// it was, what is after moves with the text.
    #[test]
    fn what_the_edit_did_not_touch_stays_and_moves_with_its_text() {
        let header = diagnostic((0, 11), (0, 12));
        let below = diagnostic((2, 8), (2, 13));
        let old = "pub struct C {\n    pub acc: f32,\n    pub level: f32,\n}\n";
        let longer = "pub struct C {\n    pub acceleration: f32,\n    pub level: f32,\n}\n";
        assert_eq!(
            after(&[header.clone(), below.clone()], old, longer),
            [((0, 11), (0, 12)), ((2, 8), (2, 13))]
        );
        let opened = "pub struct C {\n    pub acc: f32,\n\n    pub level: f32,\n}\n";
        assert_eq!(
            after(std::slice::from_ref(&below), old, opened),
            [((3, 8), (3, 13))]
        );
        // On its own line, typed before it: carried along.
        let typed = "pub struct C {\n    pub acc: f32,\n    pub  level: f32,\n}\n";
        assert_eq!(after(&[below], old, typed), [((2, 9), (2, 14))]);
    }

    /// Typing at a span's edge does not grow it into what was typed, nor
    /// take it away: `foo` unused stays `foo` while `foo.` is written.
    #[test]
    fn typing_at_an_edge_leaves_a_span_as_it_was() {
        let old = "let foo = 1;";
        let name = diagnostic((0, 4), (0, 7));
        assert_eq!(
            after(std::slice::from_ref(&name), old, "let foox = 1;"),
            [((0, 4), (0, 7))]
        );
        assert_eq!(after(&[name], old, "let xfoo = 1;"), [((0, 5), (0, 8))]);
    }

    /// A point is about the text on both sides of it, so text typed at it
    /// takes it — `expected SEMICOLON` with the `;` typed.
    #[test]
    fn text_typed_at_a_point_takes_the_point() {
        let old = "let a = f()\nlet b = 2;";
        let missing = diagnostic((0, 11), (0, 11));
        assert!(after(&[missing], old, "let a = f();\nlet b = 2;").is_empty());
    }

    /// Lines and columns are characters, so a `中` is one column however
    /// many bytes it takes — before the edit and after it.
    #[test]
    fn columns_are_characters() {
        let old = "// 中\nlet 中 = x;";
        let new = "// 中文\nlet 中 = x;";
        let x = diagnostic((1, 8), (1, 9));
        assert_eq!(after(&[x], old, new), [((1, 8), (1, 9))]);
        let typed = "// 中\nlet 中文 = x;";
        assert_eq!(
            after(&[diagnostic((1, 8), (1, 9))], old, typed),
            [((1, 9), (1, 10))]
        );
    }

    #[test]
    fn nothing_changed_says_so() {
        let mut items = vec![diagnostic((0, 1), (0, 2))];
        assert!(!diagnostics(&mut items, "abc", "abc"));
        assert!(!diagnostics(&mut items, "abc\nx", "abc\ny"));
        assert!(diagnostics(&mut items, "abc", "xabc"));
    }
}

#[cfg(test)]
mod hint_tests {
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
        super::hints(&mut hints, old, new);
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

#[cfg(test)]
mod semantic_tests {
    use super::*;

    fn span(line: u32, start_col: u32, length: u32) -> SemanticSpan {
        SemanticSpan {
            line,
            start_col,
            length,
            kind: "variable".into(),
        }
    }

    /// A span after an edit on its line moves with it, one on a line below
    /// moves down, and one whose text the edit touched goes.
    #[test]
    fn spans_move_with_the_text_and_go_with_what_they_coloured() {
        let old = "let a = b;\nlet c = d;";
        let mut spans = vec![span(0, 4, 1), span(0, 8, 1), span(1, 4, 1)];
        semantic(&mut spans, old, "let a = zz;\n\nlet c = d;");
        let at: Vec<(u32, u32, u32)> = spans
            .iter()
            .map(|s| (s.line, s.start_col, s.length))
            .collect();
        assert_eq!(at, [(0, 4, 1), (2, 4, 1)], "`b` was replaced, and goes");
    }
}
