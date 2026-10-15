//! One open file: its text, its cursors, its history, how it is painted,
//! and what it was on disk.
//!
//! The text is UTF-8 with `\n` between lines whatever the file is
//! (`rusty_edit::text_file` turns it back on save), and every position is a
//! byte into it on a character boundary. Every edit replaces the text whole
//! and says which lines it touched ([`Change`]), which is what the rows,
//! the painting and the language server each need to know.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
    time::Instant,
};

use rusty_edit::{
    Folded, Line, Region,
    text_file::{Charset, LineEnding},
};

use super::{
    cursors::Cursor,
    history::{History, Kind},
    lines::Lines,
};

/// The lines an edit touched: from `first`, `removed` lines became `added`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub first: usize,
    pub removed: usize,
    pub added: usize,
}

pub struct Document {
    pub path: String,
    pub text: String,
    pub lines: Lines,
    /// Never empty; the first is the primary one, where keys and the input
    /// method land.
    pub cursors: Vec<Cursor>,
    pub history: History,
    pub charset: Charset,
    pub ending: LineEnding,
    /// Counts every change to the text, so an answer about one text is not
    /// taken for an answer about another.
    pub version: u64,
    /// The column a run of Up and Down keeps returning to, in pixels.
    pub want_x: Option<f32>,
    /// The painted lines, kept in step with the text: an edit puts plain
    /// lines where it changed, and a repaint replaces them.
    pub painted: Vec<Line>,
    /// The painting `rusty_edit::Files` holds for this file, by number.
    pub paint: Option<u32>,
    /// Lines edited since the last repaint was taken in, which the next one
    /// must paint whether or not its own diff finds them.
    pub unpainted: Option<Range<usize>>,
    /// Outside the project — a dependency's source, a toolchain's header —
    /// and opened to be read: every edit is refused here, the one place
    /// every edit passes through.
    pub read_only: bool,
    /// The regions collapsed on screen. Session state, like the cursors:
    /// yesterday's folds put back over a file somebody has since edited
    /// would hide the wrong lines.
    pub folds: Folded,
    /// The last edit moved or opened folds, so the rows on screen are not
    /// the lines it named and are all drawn again.
    pub folds_moved: bool,
    saved: u64,
}

fn fingerprint(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl Document {
    pub fn new(path: String, text: String, charset: Charset, ending: LineEnding) -> Document {
        let lines = Lines::of(&text);
        Document {
            saved: fingerprint(&text),
            painted: vec![Line::default(); lines.count()],
            path,
            lines,
            text,
            cursors: vec![Cursor::caret(0)],
            history: History::default(),
            charset,
            ending,
            version: 0,
            want_x: None,
            paint: None,
            unpainted: None,
            read_only: false,
            folds: Folded::default(),
            folds_moved: false,
        }
    }

    /// How many rows the document takes on screen.
    pub fn rows(&self) -> usize {
        self.folds.rows(self.lines.count() as u32) as usize
    }

    /// The row a line is drawn on — its fold's header's, when it is hidden.
    pub fn row_of(&self, line: usize) -> usize {
        self.folds.row_for(line as u32) as usize
    }

    /// The line drawn on a row.
    pub fn line_at_row(&self, row: usize) -> usize {
        (self.folds.doc_of_view(row as u32) as usize).min(self.lines.count() - 1)
    }

    /// Open every fold that hides one of `lines`; whether any did.
    pub fn unfold_over(&mut self, lines: impl IntoIterator<Item = usize>) -> bool {
        let mut opened = false;
        for line in lines {
            while let Some(header) = self
                .folds
                .regions()
                .iter()
                .find(|r| line as u32 > r.header && line as u32 <= r.last)
                .map(|r| r.header)
            {
                self.folds.unfold(header);
                opened = true;
            }
        }
        opened
    }

    pub fn dirty(&self) -> bool {
        fingerprint(&self.text) != self.saved
    }

    /// The disk now holds `text`.
    pub fn mark_saved(&mut self, text: &str) {
        self.saved = fingerprint(text);
        self.history.seal();
    }

    pub fn primary(&self) -> Cursor {
        self.cursors[0]
    }

    /// An edit: the new text and cursors, remembered for undo as `kind`.
    pub fn edit(&mut self, text: String, cursors: Vec<Cursor>, kind: Kind) -> Option<Change> {
        if text == self.text || self.read_only {
            self.set_cursors(cursors);
            return None;
        }
        self.history
            .record(&self.text, &self.cursors, kind, Instant::now());
        Some(self.replace(text, cursors))
    }

    pub fn undo(&mut self) -> Option<Change> {
        let (text, cursors) = self.history.undo(&self.text, &self.cursors)?;
        Some(self.replace(text, cursors))
    }

    pub fn redo(&mut self) -> Option<Change> {
        let (text, cursors) = self.history.redo(&self.text, &self.cursors)?;
        Some(self.replace(text, cursors))
    }

    /// Move the cursors without changing the text. The next keystroke
    /// starts an undo step of its own, and Up and Down forget their column.
    pub fn set_cursors(&mut self, cursors: Vec<Cursor>) {
        if cursors.is_empty() {
            return;
        }
        let clamp = |at: usize| boundary(&self.text, at.min(self.text.len()));
        self.cursors = cursors
            .into_iter()
            .map(|c| Cursor {
                anchor: clamp(c.anchor),
                head: clamp(c.head),
            })
            .collect();
        self.history.seal();
    }

    fn replace(&mut self, text: String, cursors: Vec<Cursor>) -> Change {
        let change = changed_lines(&self.text, &self.lines, &text);
        self.text = text;
        self.lines = Lines::of(&self.text);
        self.version += 1;
        self.want_x = None;
        let tail = change.first + change.removed;
        self.painted.splice(
            change.first..tail.min(self.painted.len()),
            std::iter::repeat_n(Line::default(), change.added),
        );
        self.painted.resize(self.lines.count(), Line::default());
        self.unpainted = Some(follow(self.unpainted.clone(), change));
        if !self.folds.is_empty() {
            carry(&mut self.folds, change, &self.text);
            self.folds_moved = true;
        }
        let clamp = |at: usize| boundary(&self.text, at.min(self.text.len()));
        self.cursors = cursors
            .into_iter()
            .map(|c| Cursor {
                anchor: clamp(c.anchor),
                head: clamp(c.head),
            })
            .collect();
        if self.cursors.is_empty() {
            self.cursors.push(Cursor::caret(0));
        }
        change
    }

    /// A repaint taken in: the painting `rusty_edit::Files` now holds, and
    /// its lines from `from` — for the text at `version`. An answer about a
    /// text since edited is not placed (its lines are where they were); the
    /// caller asks again.
    pub fn painted_from(
        &mut self,
        version: u64,
        paint: u32,
        from: usize,
        lines: Vec<Line>,
    ) -> bool {
        self.paint = Some(paint);
        if version != self.version {
            return false;
        }
        for (at, line) in lines.into_iter().enumerate() {
            if let Some(slot) = self.painted.get_mut(from + at) {
                *slot = line;
            }
        }
        self.unpainted = None;
        true
    }
}

/// The lines between the first and the last byte two texts differ in.
fn changed_lines(old: &str, old_lines: &Lines, new: &str) -> Change {
    let (a, b) = (old.as_bytes(), new.as_bytes());
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    let first = old_lines.line_of(prefix);
    let old_last = old_lines.line_of(a.len() - suffix);
    let new_last = first
        + new[old_lines.start(first)..b.len() - suffix]
            .matches('\n')
            .count();
    Change {
        first,
        removed: old_last - first + 1,
        added: new_last - first + 1,
    }
}

/// The folds after `change`: one whose hidden lines it reached opens, since
/// what it hid is not what it hides now; one below moves with its lines;
/// typing on a folded header keeps it, as VS Code does. What is left takes
/// the end the new text gives it, and goes where its line no longer opens a
/// block.
fn carry(folds: &mut Folded, change: Change, text: &str) {
    let on_header =
        change.removed == 1 && change.added == 1 && folds.is_folded(change.first as u32);
    if !on_header {
        let delta = change.added as i64 - change.removed as i64;
        folds.follow(
            change.first as u32,
            (change.first + change.removed) as u32,
            delta,
        );
    }
    if folds.is_empty() {
        return;
    }
    let now = rusty_edit::fold::regions(text);
    let kept: Vec<Region> = folds
        .regions()
        .iter()
        .filter_map(|r| now.iter().find(|n| n.header == r.header).copied())
        .collect();
    folds.clear();
    for region in kept {
        folds.fold(region);
    }
}

/// The lines still to paint after `change`, in the new text's numbering.
fn follow(unpainted: Option<Range<usize>>, change: Change) -> Range<usize> {
    let touched = change.first..change.first + change.added;
    let Some(old) = unpainted else {
        return touched;
    };
    let moved = |line: usize| {
        if line >= change.first + change.removed {
            line + change.added - change.removed
        } else {
            line.min(change.first + change.added)
        }
    };
    old.start.min(touched.start)..moved(old.end).max(touched.end)
}

fn boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::new("a.rs".into(), text.into(), Charset::Utf8, LineEnding::Lf)
    }

    #[test]
    fn an_edit_names_the_lines_it_touched() {
        let mut d = doc("a\nb\nc");
        let change = d
            .edit("a\nbX\nc".into(), vec![Cursor::caret(4)], Kind::Typing)
            .unwrap();
        assert_eq!(
            change,
            Change {
                first: 1,
                removed: 1,
                added: 1
            }
        );
        // A line opened between two: the diff names it from the first byte
        // that differs, so the `c` line became the empty one and `c`.
        let change = d
            .edit("a\nbX\n\nc".into(), vec![Cursor::caret(5)], Kind::Typing)
            .unwrap();
        assert_eq!(
            change,
            Change {
                first: 2,
                removed: 1,
                added: 2
            }
        );
        assert_eq!(d.painted.len(), d.lines.count());
        let change = d
            .edit("c".into(), vec![Cursor::caret(0)], Kind::Other)
            .unwrap();
        assert_eq!((change.first, change.added), (0, 1));
    }

    #[test]
    fn dirty_follows_the_text_back_to_what_was_saved() {
        let mut d = doc("abc");
        assert!(!d.dirty());
        d.edit("abcd".into(), vec![Cursor::caret(4)], Kind::Typing);
        assert!(d.dirty());
        d.undo();
        assert!(!d.dirty(), "undone to what is on disk");
    }

    /// A repaint about a text since edited is not placed: its lines would
    /// land on the wrong rows.
    #[test]
    fn a_repaint_for_an_older_text_is_not_placed() {
        let mut d = doc("a\nb");
        let version = d.version;
        d.edit("a\nb\nc".into(), vec![Cursor::caret(5)], Kind::Typing);
        assert!(!d.painted_from(version, 1, 0, vec![Line::default(); 2]));
        assert!(d.unpainted.is_some(), "still to paint");
        assert!(d.painted_from(d.version, 2, 0, vec![Line::default(); 3]));
        assert!(d.unpainted.is_none());
    }

    fn folded(text: &str, headers: &[u32]) -> Document {
        let mut d = doc(text);
        for region in rusty_edit::fold::regions(text) {
            if headers.contains(&region.header) {
                d.folds.fold(region);
            }
        }
        d
    }

    const TWO: &str = "fn a() {
    1
}
fn b() {
    2
}
";

    /// A fold below an edit moves with its lines; typing on its header
    /// keeps it; an edit that reaches its hidden lines opens it.
    #[test]
    fn folds_follow_the_edits_around_them() {
        let mut d = folded(TWO, &[3]);
        assert_eq!(d.rows(), 6);
        d.edit(
            format!(
                "// top
{TWO}"
            ),
            vec![Cursor::caret(0)],
            Kind::Other,
        );
        assert!(d.folds.is_folded(4), "moved down a line");
        assert_eq!(d.line_at_row(5), 6, "line 5 hidden under it");
        let header = d.lines.start(4) + 3;
        let text = format!("{}x{}", &d.text[..header], &d.text[header..]);
        d.edit(text, vec![Cursor::caret(header + 1)], Kind::Typing);
        assert!(d.folds.is_folded(4), "typed on its header");
        let inside = d.lines.start(5) + 4;
        let text = format!("{}3{}", &d.text[..inside], &d.text[inside..]);
        d.edit(text, vec![Cursor::caret(inside)], Kind::Other);
        assert!(d.folds.is_empty(), "an edit inside opens it");
    }

    /// A fold whose line no longer opens a block goes.
    #[test]
    fn a_fold_whose_block_is_gone_goes() {
        let mut d = folded(TWO, &[0]);
        let text = TWO.replace(
            "fn a() {
    1
}
",
            "fn a() {}
",
        );
        d.edit(text, vec![Cursor::caret(0)], Kind::Other);
        assert!(d.folds.is_empty());
    }

    #[test]
    fn a_hidden_line_is_drawn_on_its_header_s_row() {
        // The body is hidden; the closing brace stays, as indentation
        // folding leaves it.
        let mut d = folded(TWO, &[0]);
        assert_eq!(d.row_of(1), 0);
        assert_eq!(d.row_of(3), 2);
        assert_eq!(d.line_at_row(1), 2);
        assert!(d.unfold_over([1]));
        assert_eq!(d.row_of(3), 3);
    }

    #[test]
    fn what_is_left_to_paint_moves_with_the_lines() {
        let first = follow(
            None,
            Change {
                first: 5,
                removed: 1,
                added: 1,
            },
        );
        assert_eq!(first, 5..6);
        // Two lines opened above it push it down.
        let then = follow(
            Some(first),
            Change {
                first: 1,
                removed: 1,
                added: 3,
            },
        );
        assert_eq!(then, 1..8);
    }
}
