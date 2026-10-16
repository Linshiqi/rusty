//! Selecting in a diff: each side of it read as a document of its own, and
//! a selection a range in one of them.
//!
//! A patch is drawn as rows, and a row is not always a line of anything: a
//! hunk's header, a side left blank across from a run the other side has
//! longer, git's `\ No newline at end of file`. What is selected and copied
//! is the text of one side — side by side the old file's lines or the new
//! file's, in one column the lines in git's order — without the numbers,
//! the `+` and `-`, the headers and the notes, as VS Code's read-only diff
//! editor copies it. So each side is the lines that side shows, in order,
//! and a selection is two places in them, a line and a character, the way
//! an editor's is. A selection stays on the side it was started on, which
//! is how a drag down the left column never picks up the right one's text.
//! Pure, so what a drag across a header or a blank selects is a test rather
//! than something found by dragging (`app/git_select.rs` is the window's
//! half).

/// A line as the view draws it: a tab is four spaces, which is what the
/// columns of a place count.
pub fn drawn(text: &str) -> String {
    text.replace('\t', "    ")
}

/// How many columns a character of the patch takes as drawn.
fn columns(ch: char) -> usize {
    if ch == '\t' { 4 } else { 1 }
}

/// The byte of `text` a column of its drawn line falls at: a column inside
/// a tab's four goes to whichever end of the tab is nearer.
fn byte_of(text: &str, col: usize) -> usize {
    let mut at = 0;
    for (byte, ch) in text.char_indices() {
        let width = columns(ch);
        if col < at + width {
            return if col - at > width / 2 {
                byte + ch.len_utf8()
            } else {
                byte
            };
        }
        at += width;
    }
    text.len()
}

/// A place in one side's lines: a line, and a character of it as drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    pub line: usize,
    pub col: usize,
}

/// The lines one side of a diff shows, in order, and the rows they are
/// drawn on.
#[derive(Clone, Debug, Default)]
pub struct Side {
    /// Each line as the patch has it, tabs and all — what a copy copies.
    lines: Vec<String>,
    /// Each line as it is drawn — what a column counts.
    drawn: Vec<String>,
    /// The row each line is drawn on.
    rows: Vec<usize>,
    /// For every row, the first line drawn on it or below it.
    next: Vec<usize>,
}

impl Side {
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// A line as the view draws it.
    pub fn drawn(&self, line: usize) -> &str {
        self.drawn.get(line).map_or("", String::as_str)
    }

    /// The row a line is drawn on.
    pub fn row_of(&self, line: usize) -> usize {
        self.rows.get(line).copied().unwrap_or(0)
    }

    /// How many characters a line is as drawn.
    fn width(&self, line: usize) -> usize {
        self.drawn(line).chars().count()
    }

    /// The end of the last line.
    pub fn end(&self) -> Place {
        match self.len() {
            0 => Place::default(),
            n => Place {
                line: n - 1,
                col: self.width(n - 1),
            },
        }
    }

    /// The place a pointer on `row` names, `col_of` turning a line as
    /// drawn and the pointer's distance into it into a column. On a row of
    /// this side's, that line; on one that is not — a header, a blank
    /// across from the other side's run, a note — the start of the next
    /// line, so a drag that ends on a header takes the line above it whole;
    /// above the first row the start, past the last the end. Nothing for a
    /// side with no lines.
    pub fn place_at(&self, row: i64, col_of: impl FnOnce(&str) -> usize) -> Option<Place> {
        if self.is_empty() {
            return None;
        }
        if row < 0 {
            return Some(Place::default());
        }
        let Some(&line) = self.next.get(row as usize) else {
            return Some(self.end());
        };
        if line >= self.len() {
            return Some(self.end());
        }
        if self.rows[line] != row as usize {
            return Some(Place { line, col: 0 });
        }
        let col = col_of(self.drawn(line)).min(self.width(line));
        Some(Place { line, col })
    }

    /// The text between two places, in either order: whole lines between
    /// them, a line break between lines, and a tab copied as the tab it is.
    pub fn text(&self, a: Place, b: Place) -> String {
        let (from, to) = if a <= b { (a, b) } else { (b, a) };
        if self.is_empty() || from == to {
            return String::new();
        }
        let last = self.len() - 1;
        let (from_line, to_line) = (from.line.min(last), to.line.min(last));
        let mut out = String::new();
        for line in from_line..=to_line {
            let text = &self.lines[line];
            let start = if line == from_line {
                byte_of(text, from.col)
            } else {
                0
            };
            let end = if line == to_line {
                byte_of(text, to.col)
            } else {
                text.len()
            };
            if line > from_line {
                out.push('\n');
            }
            out.push_str(&text[start..end.max(start)]);
        }
        out
    }

    /// The word a place is in, as a double-click picks it: a run of
    /// letters, digits and `_`, a run of spaces, or one other character.
    /// At a line's end, the word before it.
    pub fn word(&self, at: Place) -> (Place, Place) {
        let chars: Vec<char> = self.drawn(at.line).chars().collect();
        if chars.is_empty() {
            return (at, at);
        }
        let kind = |ch: char| {
            if ch.is_whitespace() {
                0
            } else if ch.is_alphanumeric() || ch == '_' {
                1
            } else {
                2
            }
        };
        let col = at.col.min(chars.len() - 1);
        let class = kind(chars[col]);
        if class == 2 {
            return (Place { col, ..at }, Place { col: col + 1, ..at });
        }
        let mut start = col;
        while start > 0 && kind(chars[start - 1]) == class {
            start -= 1;
        }
        let mut end = col + 1;
        while end < chars.len() && kind(chars[end]) == class {
            end += 1;
        }
        (Place { col: start, ..at }, Place { col: end, ..at })
    }
}

/// Both sides of a diff as drawn: the left column's lines (or the one
/// column's) and the right column's.
#[derive(Clone, Debug, Default)]
pub struct DiffText {
    sides: [Side; 2],
}

impl DiffText {
    /// One entry per row drawn: the row's text on the left (or in the one
    /// column) and on the right, `None` where the row is no line of that
    /// side.
    pub fn new(rows: &[[Option<&str>; 2]]) -> DiffText {
        let mut sides: [Side; 2] = Default::default();
        for (index, side) in sides.iter_mut().enumerate() {
            for (row, cells) in rows.iter().enumerate() {
                if let Some(text) = cells[index] {
                    side.lines.push(text.to_string());
                    side.drawn.push(drawn(text));
                    side.rows.push(row);
                }
            }
            let mut next = vec![side.len(); rows.len()];
            let mut line = side.len();
            for row in (0..rows.len()).rev() {
                if line > 0 && side.rows[line - 1] >= row {
                    line -= 1;
                }
                next[row] = line;
            }
            side.next = next;
        }
        DiffText { sides }
    }

    /// One side: 0 the left column or the one column, 1 the right.
    pub fn side(&self, side: usize) -> &Side {
        &self.sides[side.min(1)]
    }
}

/// A selection: the side it was started on, where it was started and where
/// it has been dragged to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub side: usize,
    pub anchor: Place,
    pub head: Place,
}

impl Selection {
    /// The selection's two ends, the earlier first.
    pub fn ordered(&self) -> (Place, Place) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The whole of one side.
    pub fn all(text: &DiffText, side: usize) -> Option<Selection> {
        let lines = text.side(side);
        (!lines.is_empty()).then(|| Selection {
            side: side.min(1),
            anchor: Place::default(),
            head: lines.end(),
        })
    }

    /// What it holds, as text.
    pub fn text(&self, text: &DiffText) -> String {
        text.side(self.side).text(self.anchor, self.head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A side-by-side hunk as `git_view` draws it: a header, a kept line,
    /// a line replaced, a line only the new side has, a note on the old
    /// side, and a second hunk's header and line.
    fn hunks() -> DiffText {
        DiffText::new(&[
            [None, None],
            [Some("keep"), Some("keep")],
            [Some("old line"), Some("new line")],
            [None, Some("added")],
            [None, None],
            [Some("tail"), Some("tail")],
        ])
    }

    /// One column a character wide, for a pointer `x` columns in.
    fn at(col: usize) -> impl FnOnce(&str) -> usize {
        move |_| col
    }

    /// Each side is the lines it shows, on the rows they are drawn on: no
    /// header, no blank, no note.
    #[test]
    fn a_side_is_the_lines_it_shows() {
        let text = hunks();
        let left = text.side(0);
        let right = text.side(1);
        assert_eq!(left.len(), 3);
        assert_eq!(right.len(), 4);
        assert_eq!(
            (left.row_of(1), left.row_of(2)),
            (2, 5),
            "the old side's lines skip the new side's addition"
        );
        assert_eq!(right.row_of(2), 3);
    }

    /// A drag down the right column copies the right column's text and none
    /// of the left's, whatever rows it crosses.
    #[test]
    fn a_selection_copies_its_own_side_only() {
        let text = hunks();
        let right = text.side(1);
        let from = right.place_at(2, at(4)).unwrap();
        let to = right.place_at(5, at(2)).unwrap();
        let selection = Selection {
            side: 1,
            anchor: from,
            head: to,
        };
        assert_eq!(selection.text(&text), "line\nadded\nta");
        // The same drag down the left column: the old side's text, with no
        // line where the new side added one.
        let left = Selection {
            side: 0,
            anchor: text.side(0).place_at(2, at(4)).unwrap(),
            head: text.side(0).place_at(5, at(2)).unwrap(),
        };
        assert_eq!(left.text(&text), "line\nta");
    }

    /// Dragged backwards, it is the same selection.
    #[test]
    fn a_backward_drag_is_the_same_selection() {
        let text = hunks();
        let right = text.side(1);
        let a = right.place_at(1, at(1)).unwrap();
        let b = right.place_at(3, at(3)).unwrap();
        assert_eq!(right.text(a, b), right.text(b, a));
        assert_eq!(right.text(b, a), "eep\nnew line\nadd");
    }

    /// A pointer on a row that is no line of this side — a header, the
    /// blank across from an addition — stands at the start of the next
    /// line; above the rows, the start; past them, the end.
    #[test]
    fn a_row_that_is_no_line_stands_at_the_next() {
        let text = hunks();
        let left = text.side(0);
        assert_eq!(left.place_at(0, at(9)), Some(Place { line: 0, col: 0 }));
        assert_eq!(left.place_at(3, at(9)), Some(Place { line: 2, col: 0 }));
        assert_eq!(left.place_at(4, at(9)), Some(Place { line: 2, col: 0 }));
        assert_eq!(left.place_at(-3, at(9)), Some(Place::default()));
        assert_eq!(left.place_at(40, at(0)), Some(Place { line: 2, col: 4 }));
        assert_eq!(
            left.place_at(1, at(99)),
            Some(Place { line: 0, col: 4 }),
            "past a line's end is its end"
        );
        assert_eq!(DiffText::default().side(0).place_at(0, at(0)), None);
    }

    /// A drag from a line onto the header below it takes the line whole,
    /// its break included.
    #[test]
    fn a_drag_onto_a_header_takes_the_line_above_whole() {
        let text = hunks();
        let right = text.side(1);
        let from = right.place_at(3, at(0)).unwrap();
        let to = right.place_at(4, at(0)).unwrap();
        assert_eq!(right.text(from, to), "added\n");
    }

    /// A tab is drawn as four spaces and copied as the tab it is; a column
    /// inside its four goes to the nearer end.
    #[test]
    fn a_tab_is_copied_as_a_tab() {
        let text = DiffText::new(&[[Some("\tx = 1;"), None]]);
        let side = text.side(0);
        assert_eq!(side.drawn(0), "    x = 1;");
        assert_eq!(side.text(Place::default(), side.end()), "\tx = 1;");
        let one = |col| Place { line: 0, col };
        assert_eq!(
            side.text(one(1), one(5)),
            "\tx",
            "a column near a tab's start takes it"
        );
        assert_eq!(side.text(one(3), one(5)), "x", "near its end leaves it");
    }

    /// Select all is the side whole, every line and the breaks between.
    #[test]
    fn select_all_is_the_side_whole() {
        let text = hunks();
        let all = Selection::all(&text, 1).unwrap();
        assert_eq!(all.text(&text), "keep\nnew line\nadded\ntail");
        assert!(Selection::all(&DiffText::default(), 0).is_none());
    }

    /// A double-click picks a word, a run of spaces, or one mark; at the
    /// line's end the word before it; a run of Chinese is a word.
    #[test]
    fn a_double_click_picks_a_word() {
        let text = DiffText::new(&[[Some("let total = a.b(中文);"), None]]);
        let side = text.side(0);
        let word = |col| {
            let (a, b) = side.word(Place { line: 0, col });
            side.text(a, b)
        };
        assert_eq!(word(5), "total");
        assert_eq!(word(3), " ");
        assert_eq!(word(13), ".");
        assert_eq!(word(17), "中文");
        assert_eq!(word(99), ";");
    }

    /// Characters are counted as characters, so a `中` does not move the
    /// columns after it.
    #[test]
    fn a_wide_character_is_one_column() {
        let text = DiffText::new(&[[Some("中文 ok"), None]]);
        let side = text.side(0);
        let one = |col| Place { line: 0, col };
        assert_eq!(side.text(one(3), one(5)), "ok");
        assert_eq!(side.end(), one(5));
    }
}
