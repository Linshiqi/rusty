//! Keeping the painted lines in step with a backend that repaints only what
//! an edit changed.
//!
//! The backend keeps each file's painting, and answers a repaint with the
//! lines that differ from the painting the editor names
//! (`rusty_edit::Repaint`). That is right only while the editor's lines
//! really are that painting, so two things are tracked beside them, and both
//! are arithmetic on line numbers, here and under tests:
//!
//! - **Which lines are shown plain.** A keystroke puts the lines it touched
//!   on screen as plain text at once, and they stay plain until a repaint
//!   brings their colours ([`stale_after`]). The backend is told which they
//!   are, because a letter typed and deleted leaves the text as it was — no
//!   change for the backend to find — and the line still plain on screen.
//! - **Where an answer goes when typing went on while it was out.** An answer
//!   describes the text that was sent. [`place`] puts each line where that
//!   line of the sent text is now, and leaves out the lines edited since:
//!   they are plain, and marked, and the next repaint brings them.

use rusty_edit::{Line, Span, Token};

/// Lines, as the smallest range covering them: `(first, one past the last)`.
pub type Lines = Option<(usize, usize)>;

/// How one text became another, in lines: the first `prefix` lines and the
/// last `suffix` are the same in both, and the lines between were replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineEdit {
    pub prefix: usize,
    pub suffix: usize,
    /// How many lines each text has, split at `\n` as the textarea counts.
    pub old: usize,
    pub new: usize,
}

impl LineEdit {
    /// Where a line of the old text is in the new one: where it was above the
    /// edit, moved by the lines the edit added below it, and nowhere inside it.
    pub fn moved(self, line: usize) -> Option<usize> {
        if line < self.prefix {
            Some(line)
        } else if line >= self.old - self.suffix && line < self.old {
            Some(line + self.new - self.old)
        } else {
            None
        }
    }

    /// The lines of the new text the edit wrote.
    pub fn written(self) -> Lines {
        let end = self.new - self.suffix;
        (self.prefix < end).then_some((self.prefix, end))
    }
}

/// Compare two texts line by line. A text that did not change is all prefix.
pub fn line_edit(old: &str, new: &str) -> LineEdit {
    let old_lines: Vec<&str> = old.split('\n').collect();
    let new_lines: Vec<&str> = new.split('\n').collect();
    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    LineEdit {
        prefix,
        suffix,
        old: old_lines.len(),
        new: new_lines.len(),
    }
}

/// The lines of `new` an edit replaced, as plain text — what goes on screen
/// the moment a key is pressed.
pub fn plain_lines(new: &str, edit: LineEdit) -> Vec<Line> {
    new.split('\n')
        .skip(edit.prefix)
        .take(edit.new - edit.suffix - edit.prefix)
        .map(|text| Line {
            spans: vec![Span {
                text: text.to_string(),
                token: Token::Plain,
            }],
        })
        .collect()
}

/// Patch painted lines that depict `old` so they depict `new`: the lines the
/// edit wrote go in as plain text, and the rest keep their colours. Clamped,
/// since a splice out of range panics the window. What `echo_edit` does to
/// the lines on screen and a mirrored edit does to a parked tab's.
pub fn echo(lines: &mut Vec<Line>, old: &str, new: &str) -> LineEdit {
    let edit = line_edit(old, new);
    let end = (edit.old - edit.suffix).min(lines.len());
    let start = edit.prefix.min(end);
    lines.splice(start..end, plain_lines(new, edit));
    edit
}

/// Where a position — a line and a scalar column — in `old` is in `new`, for
/// a caret in the other view of a file, which the edit was not made at. By
/// character, not by line, as VS Code's other view moves: in front of what
/// changed it stays, behind it it moves with the text — along its own line
/// too, when the change was earlier on that line — and inside what the edit
/// replaced it goes to where the change starts.
pub fn follow(old: &str, new: &str, (line, col): (usize, u32)) -> (usize, u32) {
    position_at(new, follow_byte(old, new, byte_at(old, line, col)))
}

/// [`follow`] for a byte offset: where byte `at` of `old` is in `new`.
pub fn follow_byte(old: &str, new: &str, at: usize) -> usize {
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let room = old.len().min(new.len()) - prefix;
    let mut suffix = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) {
        suffix -= 1;
    }
    if at <= prefix {
        at
    } else if at >= old.len() - suffix {
        at + new.len() - old.len()
    } else {
        prefix
    }
}

/// The byte a line and a scalar column name, clamped to the line's end.
fn byte_at(text: &str, line: usize, col: u32) -> usize {
    let mut start = 0;
    for (index, text_line) in text.split('\n').enumerate() {
        if index == line {
            return start
                + text_line
                    .char_indices()
                    .nth(col as usize)
                    .map_or(text_line.len(), |(byte, _)| byte);
        }
        start += text_line.len() + 1;
    }
    text.len()
}

/// The line and scalar column of a byte.
fn position_at(text: &str, byte: usize) -> (usize, u32) {
    let before = &text[..byte.min(text.len())];
    let line = before.matches('\n').count();
    let col = before
        .rsplit('\n')
        .next()
        .map_or(0, |tail| tail.chars().count());
    (line, col as u32)
}

/// Every line of `text` as plain text.
pub fn all_plain(text: &str) -> Vec<Line> {
    let whole = LineEdit {
        prefix: 0,
        suffix: 0,
        old: 0,
        new: text.split('\n').count(),
    };
    plain_lines(text, whole)
}

/// The lines shown plain once `edit` has been echoed: the ones that were,
/// wherever the edit moved them, and the ones it wrote.
pub fn stale_after(stale: Lines, edit: LineEdit) -> Lines {
    let tail = edit.old - edit.suffix;
    let moved = stale.and_then(|(from, to)| {
        let from = if from < edit.prefix {
            from
        } else if from >= tail {
            from + edit.new - edit.old
        } else {
            edit.prefix
        };
        let to = if to <= edit.prefix {
            to
        } else if to >= tail {
            to + edit.new - edit.old
        } else {
            edit.new - edit.suffix
        };
        (from < to).then_some((from, to))
    });
    union(moved, edit.written())
}

fn union(a: Lines, b: Lines) -> Lines {
    match (a, b) {
        (Some((a_from, a_to)), Some((b_from, b_to))) => Some((a_from.min(b_from), a_to.max(b_to))),
        (one, None) | (None, one) => one,
    }
}

/// Put a repaint of `sent` — `painted`, from line `from` — into `lines`,
/// which depict `now`.
///
/// Each line goes where that line of `sent` is in `now`, and one edited since
/// has no place and is left as it is. False, with nothing changed, when the
/// answer does not fit: `lines` are not `now` line for line, or the answer
/// runs past the end of what was sent. No edit path should allow either, and
/// the caller answers it by starting again from plain text.
pub fn place(lines: &mut [Line], sent: &str, now: &str, from: usize, painted: Vec<Line>) -> bool {
    let edit = line_edit(sent, now);
    if lines.len() != edit.new || from + painted.len() > edit.old {
        return false;
    }
    for (offset, line) in painted.into_iter().enumerate() {
        if let Some(at) = edit.moved(from + offset) {
            lines[at] = line;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted(text: &str) -> Line {
        Line {
            spans: vec![Span {
                text: text.to_string(),
                token: Token::Keyword,
            }],
        }
    }

    fn plain(text: &str) -> Line {
        Line {
            spans: vec![Span {
                text: text.to_string(),
                token: Token::Plain,
            }],
        }
    }

    fn texts(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.text.as_str()).collect())
            .collect()
    }

    /// Only the edited line is repainted plain; the lines around it keep
    /// their colours. Typing on line two of three touches one row.
    #[test]
    fn an_edit_inside_one_line_replaces_only_that_line() {
        let edit = line_edit("a\nb\nc", "a\nbX\nc");
        assert_eq!((edit.prefix, edit.suffix), (1, 1));
        assert_eq!(texts(&plain_lines("a\nbX\nc", edit)), vec!["bX"]);
    }

    /// Enter in the middle of a file inserts a row rather than repainting
    /// everything below it.
    #[test]
    fn a_new_line_is_an_insertion_not_a_repaint_of_the_rest() {
        let edit = line_edit("a\nb\nc", "a\nb\n\nc");
        assert_eq!((edit.prefix, edit.suffix), (2, 1));
        assert_eq!(texts(&plain_lines("a\nb\n\nc", edit)), vec![""]);
    }

    /// Deleting a line is an empty replacement over one row, and a repeated
    /// line is not mistaken for context: "a\na" minus the second "a" removes
    /// one row rather than claiming both survived.
    #[test]
    fn a_deleted_line_is_an_empty_replacement() {
        let edit = line_edit("a\nb\nc", "a\nc");
        assert_eq!((edit.prefix, edit.suffix), (1, 1));
        assert!(plain_lines("a\nc", edit).is_empty());

        let edit = line_edit("a\na", "a");
        assert_eq!(edit.prefix + edit.suffix, 1);
        assert!(plain_lines("a", edit).is_empty());
    }

    #[test]
    fn a_line_moves_by_what_was_added_above_it_and_is_lost_inside_the_edit() {
        // "a b c d" → "a X Y c d": b replaced by two lines.
        let edit = line_edit("a\nb\nc\nd", "a\nX\nY\nc\nd");
        assert_eq!(
            edit,
            LineEdit {
                prefix: 1,
                suffix: 2,
                old: 4,
                new: 5
            }
        );
        assert_eq!(edit.moved(0), Some(0));
        assert_eq!(edit.moved(1), None, "b is gone");
        assert_eq!(edit.moved(2), Some(3));
        assert_eq!(edit.moved(3), Some(4));
        assert_eq!(edit.written(), Some((1, 3)));
        assert_eq!(plain_lines("a\nX\nY\nc\nd", edit), [plain("X"), plain("Y")]);
    }

    /// A text that did not change is all prefix, so every line stays where
    /// it is — the case of a repaint answered after nothing was typed.
    #[test]
    fn an_unchanged_text_keeps_every_line_in_place() {
        let edit = line_edit("a\nb\n", "a\nb\n");
        assert_eq!(edit.written(), None);
        for line in 0..3 {
            assert_eq!(edit.moved(line), Some(line));
        }
    }

    #[test]
    fn plain_lines_follow_the_edits_that_move_them_and_grow_with_the_ones_that_write() {
        // Line 5 typed on: stale is that line.
        let typed = line_edit("0\n1\n2\n3\n4\n5\n6", "0\n1\n2\n3\n4\n5x\n6");
        let stale = stale_after(None, typed);
        assert_eq!(stale, Some((5, 6)));
        // Two lines added above it: it moves down two, and the new lines join.
        let added = line_edit("0\n1\n2\n3\n4\n5x\n6", "0\nA\nB\n1\n2\n3\n4\n5x\n6");
        assert_eq!(stale_after(stale, added), Some((1, 8)));
        // Deleted outright: nothing left to paint, and nothing written.
        let deleted = line_edit("0\n1\n2\n3\n4\n5x\n6", "0\n1\n2\n3\n4\n6");
        assert_eq!(stale_after(stale, deleted), None);
        // An edit below it leaves it where it was.
        let below = line_edit("0\n1\n2\n3\n4\n5x\n6", "0\n1\n2\n3\n4\n5x\n6!");
        assert_eq!(stale_after(stale, below), Some((5, 7)));
    }

    /// Typing went on while the answer was out: the answer's lines land where
    /// those lines are now, and the line typed since keeps what the keystroke
    /// put there.
    #[test]
    fn an_answer_lands_where_its_lines_went_and_skips_the_ones_edited_since() {
        let sent = "fn a\nfn b\nfn c";
        let answer = || vec![painted("fn a"), painted("fn b"), painted("fn c")];

        let now = "fn a\nfn b\n// new\nfn c";
        let mut lines = vec![plain("fn a"), plain("fn b"), plain("// new"), plain("fn c")];
        assert!(place(&mut lines, sent, now, 0, answer()));
        assert_eq!(
            lines,
            [
                painted("fn a"),
                painted("fn b"),
                plain("// new"),
                painted("fn c")
            ]
        );

        // Two edits apart read as one span from the first to the last, so a
        // line between them waits for the next repaint — as it should, since
        // both edits marked it stale on the way.
        let now = "// new\nfn a\nfn b!\nfn c";
        let mut lines = vec![
            plain("// new"),
            plain("fn a"),
            plain("fn b!"),
            plain("fn c"),
        ];
        assert!(place(&mut lines, sent, now, 0, answer()));
        assert_eq!(
            lines,
            [
                plain("// new"),
                plain("fn a"),
                plain("fn b!"),
                painted("fn c")
            ]
        );
        let echoed = [
            line_edit("fn a\nfn b\nfn c", "// new\nfn a\nfn b\nfn c"),
            line_edit("// new\nfn a\nfn b\nfn c", now),
        ];
        let stale = echoed.into_iter().fold(None, stale_after);
        assert_eq!(stale, Some((0, 3)), "fn a is inside what stays stale");
    }

    /// The other view's caret goes with the text it is in: down past lines
    /// added above it, along its line past what was typed before it on the
    /// same line, not at all for what was typed after it, and to the start of
    /// a change that took the text it stood in — with a `中` on the line, so a
    /// column counted in bytes cannot pass for one counted in scalars.
    #[test]
    fn a_position_follows_its_text_across_an_edit() {
        let old = "a\nb\nc";
        let added = "a\nX\nY\nb\nc";
        assert_eq!(follow(old, added, (2, 1)), (4, 1));
        assert_eq!(follow(old, added, (0, 1)), (0, 1));
        let line = "let 中 = sample();";
        let typed = "let 中 = my_sample();";
        assert_eq!(
            follow(line, typed, (0, 14)),
            (0, 17),
            "behind it on its line"
        );
        assert_eq!(follow(line, typed, (0, 3)), (0, 3), "in front of it");
        let old = "a\nbbb\nc\nd";
        assert_eq!(follow(old, "a\nd", (1, 2)), (1, 0), "inside what went");
        assert_eq!(follow("a\nb", "", (1, 0)), (0, 0));
        assert_eq!(follow("ab", "ab", (0, 9)), (0, 2), "clamped to its line");
    }

    #[test]
    fn echoing_an_edit_keeps_every_line_it_did_not_write() {
        let mut lines = vec![painted("a"), painted("b"), painted("c")];
        let edit = echo(&mut lines, "a\nb\nc", "a\nbX\nY\nc");
        assert_eq!(edit.written(), Some((1, 3)));
        assert_eq!(lines, [painted("a"), plain("bX"), plain("Y"), painted("c")]);
    }

    #[test]
    fn an_answer_that_does_not_fit_changes_nothing() {
        let mut lines = vec![plain("a"), plain("b")];
        assert!(!place(&mut lines, "a\nb", "a\nb\nc", 0, vec![painted("a")]));
        assert!(!place(
            &mut lines,
            "a\nb",
            "a\nb",
            1,
            vec![painted("b"), painted("c")]
        ));
        assert_eq!(lines, [plain("a"), plain("b")]);
    }
}
