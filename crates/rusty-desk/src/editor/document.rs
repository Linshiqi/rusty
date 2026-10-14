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
    Line,
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
    pub language: Option<String>,
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
            language: None,
            version: 0,
            want_x: None,
            paint: None,
            unpainted: None,
        }
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
        if text == self.text {
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
