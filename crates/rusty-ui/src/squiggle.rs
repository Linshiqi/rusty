//! Where a diagnostic's squiggle is drawn: the columns of a line it covers.
//!
//! One rule for the echo, which draws the red line, and for the hover card,
//! which has to find a problem where its red line is. Two readings of where
//! a squiggle stands would be a card over nothing, or a red line nobody can
//! ask about.
//!
//! Most diagnostics cover what they are about and are drawn over it. The
//! exception is the one people meet first: a problem at a *point* —
//! rust-analyzer's `expected SEMICOLON`, rustc's ``expected `;` `` — is a
//! range of no width, at the end of the line the semicolon is missing from.
//! Drawn over the characters it covers, it was drawn over none, so the
//! Problems panel listed two errors on a line the editor showed clean.
//! VS Code's rule (`_createDecorationRange`) is the one here: a point past
//! the last thing written on its line is drawn one cell wide there, and a
//! point anywhere else widens to the word it touches.

use rusty_lsp::FileDiagnostic;

/// The columns of line `index`, whose text is `text`, that `d`'s squiggle
/// covers: half-open, in Unicode scalars, `None` where it draws nothing on
/// this line. `to` is one past the line's end for a point at its end, which
/// the echo draws as a cell after everything else on the line.
pub fn drawn_on(d: &FileDiagnostic, index: u32, text: &str) -> Option<(u32, u32)> {
    if index < d.start_line || index > d.end_line {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let length = chars.len() as u32;
    let from = if index == d.start_line {
        d.start_col.min(length)
    } else {
        0
    };
    let to = if index == d.end_line {
        d.end_col.min(length)
    } else {
        length
    };
    if from < to {
        return Some((from, to));
    }
    // Empty on this line. A problem that runs on to another line is drawn
    // there, except one that holds nothing but this line's break, which is
    // a point at the end of this line in all but name.
    let point = d.start_line == d.end_line
        || (index == d.start_line && d.end_line == d.start_line + 1 && d.end_col == 0);
    if !point {
        return None;
    }
    Some(widened(&chars, from))
}

/// A point, made something that can be seen: one cell where nothing but
/// whitespace follows it, the word it touches otherwise, and one cell when
/// it touches none.
fn widened(chars: &[char], at: u32) -> (u32, u32) {
    let length = chars.len() as u32;
    let written = chars
        .iter()
        .rposition(|ch| !ch.is_whitespace())
        .map_or(length, |last| last as u32 + 1);
    if at >= written {
        return (at, at + 1);
    }
    let word = |ch: char| ch.is_alphanumeric() || ch == '_';
    let start = chars[..at as usize]
        .iter()
        .rposition(|&ch| !word(ch))
        .map_or(0, |before| before as u32 + 1);
    let end = chars[at as usize..]
        .iter()
        .position(|&ch| !word(ch))
        .map_or(length, |after| at + after as u32);
    if start < end {
        (start, end)
    } else {
        (at, at + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_lsp::DiagSeverity;

    fn diag(start: (u32, u32), end: (u32, u32)) -> FileDiagnostic {
        FileDiagnostic {
            severity: DiagSeverity::Error,
            message: "no".to_string(),
            source: None,
            code: None,
            start_line: start.0,
            start_col: start.1,
            end_line: end.0,
            end_col: end.1,
        }
    }

    const UP: &str = "    pub const UP: Vector = Vector::new(0.0, 0.0, 1.0)";

    /// The report: `expected SEMICOLON` at the end of a line with no `;` is
    /// drawn one cell past the `)`, not over nothing.
    #[test]
    fn a_point_at_the_end_of_a_line_is_drawn_past_its_last_character() {
        let end = UP.chars().count() as u32;
        let missing = diag((15, end), (15, end));
        assert_eq!(drawn_on(&missing, 15, UP), Some((end, end + 1)));
        // Trailing whitespace is not something written: the cell is where
        // the point is, over the first of the spaces.
        let spaced = format!("{UP}   ");
        assert_eq!(drawn_on(&missing, 15, &spaced), Some((end, end + 1)));
        // A line shortened since the diagnostic was sent: the point is
        // clamped to the end rather than drawn in the void past it.
        let beyond = diag((15, end + 9), (15, end + 9));
        assert_eq!(drawn_on(&beyond, 15, UP), Some((end, end + 1)));
    }

    /// rustc's ``expected `;` `` is the same point spelled as a range over
    /// the line's break alone.
    #[test]
    fn a_range_over_nothing_but_the_line_break_is_a_point() {
        let end = UP.chars().count() as u32;
        let over_break = diag((15, end), (16, 0));
        assert_eq!(drawn_on(&over_break, 15, UP), Some((end, end + 1)));
        assert_eq!(drawn_on(&over_break, 16, ""), None);
    }

    /// An empty line has nothing to underline; the cell is drawn anyway,
    /// which is where "this file contains an unclosed delimiter" lands.
    #[test]
    fn a_point_on_an_empty_line_is_one_cell() {
        let eof = diag((40, 0), (40, 0));
        assert_eq!(drawn_on(&eof, 40, ""), Some((0, 1)));
        assert_eq!(drawn_on(&eof, 40, "    "), Some((0, 1)));
    }

    /// Inside a line a point widens to the word it touches — before it,
    /// after it or through it — and to one cell when it touches none.
    #[test]
    fn a_point_inside_a_line_widens_to_the_word_it_touches() {
        let line = "let total = sum(a, b);";
        let at = |col| drawn_on(&diag((0, col), (0, col)), 0, line);
        assert_eq!(at(4), Some((4, 9)), "at the start of `total`");
        assert_eq!(at(6), Some((4, 9)), "inside it");
        assert_eq!(at(9), Some((4, 9)), "just after it");
        assert_eq!(at(11), Some((11, 12)), "between `=` and a space: one cell");
        assert_eq!(at(16), Some((16, 17)), "`a`");
        // A word is what a Rust name is made of, CJK included.
        let cjk = "let 名前 = 1;";
        assert_eq!(drawn_on(&diag((0, 5), (0, 5)), 0, cjk), Some((4, 6)));
    }

    /// Everything else is drawn over exactly what it covers, a line at a
    /// time; lines it does not reach draw nothing, and an empty piece of a
    /// problem that goes on elsewhere is drawn where it goes on.
    #[test]
    fn a_range_is_drawn_over_what_it_covers() {
        let line = "fn main() {";
        assert_eq!(
            drawn_on(&diag((2, 3), (2, 7)), 2, line),
            Some((3, 7)),
            "one line"
        );
        assert_eq!(drawn_on(&diag((2, 3), (2, 7)), 3, line), None, "not here");
        let long = diag((2, 3), (5, 4));
        assert_eq!(drawn_on(&long, 2, line), Some((3, 11)), "to its end");
        assert_eq!(drawn_on(&long, 3, line), Some((0, 11)), "all of it");
        assert_eq!(drawn_on(&long, 4, ""), None, "an empty line between");
        assert_eq!(drawn_on(&long, 5, line), Some((0, 4)), "up to its end");
        let from_the_end = diag((2, 11), (4, 2));
        assert_eq!(drawn_on(&from_the_end, 2, line), None);
        assert_eq!(drawn_on(&from_the_end, 4, "pub fn"), Some((0, 2)));
    }
}
