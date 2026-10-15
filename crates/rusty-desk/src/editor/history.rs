//! Undo and redo: the text and the cursors as they were before each step.
//!
//! A step is a snapshot of the whole text — simple to get right, and cheap
//! next to everything else an edit costs.
//! Keystrokes of one kind close together are one step (typing a word is
//! one undo, not five), anything else is a step of its own, and the
//! cursors come back with the text, selections included. Two hundred
//! steps or 48 MB of them, whichever comes first, the oldest going first.

use std::time::{Duration, Instant};

use super::cursors::Cursor;

/// What an edit was, for deciding whether it joins the step before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Typing,
    Deleting,
    /// Anything else: a paste, a line moved, a comment toggled.
    Other,
}

#[derive(Clone, Debug)]
struct Step {
    text: String,
    cursors: Vec<Cursor>,
}

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    last: Option<(Kind, Instant)>,
    /// Every edit joins the step before it until the next seal: Vim's
    /// change, from its command through what is typed to Escape.
    held: bool,
}

/// How close two keystrokes are to be one step.
const JOIN: Duration = Duration::from_millis(600);
const MOST_STEPS: usize = 200;
const MOST_BYTES: usize = 48 * 1024 * 1024;

impl History {
    /// Remember `text` and `cursors` as they were before an edit of `kind`.
    pub fn record(&mut self, text: &str, cursors: &[Cursor], kind: Kind, now: Instant) {
        self.redo.clear();
        let joins = match self.last {
            Some(_) if self.held => true,
            Some((last, at)) => {
                last == kind && kind != Kind::Other && now.duration_since(at) < JOIN
            }
            None => false,
        };
        self.last = Some((kind, now));
        if joins {
            return;
        }
        self.undo.push(Step {
            text: text.to_string(),
            cursors: cursors.to_vec(),
        });
        self.trim();
    }

    /// The next keystroke starts a step of its own: the caret moved, a save.
    pub fn seal(&mut self) {
        self.last = None;
        self.held = false;
    }

    /// Every edit from now to the next seal is one step with the last.
    pub fn hold(&mut self) {
        self.held = true;
    }

    /// Step back from `text`: what to put on screen instead.
    pub fn undo(&mut self, text: &str, cursors: &[Cursor]) -> Option<(String, Vec<Cursor>)> {
        let step = self.undo.pop()?;
        self.redo.push(Step {
            text: text.to_string(),
            cursors: cursors.to_vec(),
        });
        self.seal();
        Some((step.text, step.cursors))
    }

    pub fn redo(&mut self, text: &str, cursors: &[Cursor]) -> Option<(String, Vec<Cursor>)> {
        let step = self.redo.pop()?;
        self.undo.push(Step {
            text: text.to_string(),
            cursors: cursors.to_vec(),
        });
        self.last = None;
        Some((step.text, step.cursors))
    }

    fn trim(&mut self) {
        while self.undo.len() > MOST_STEPS {
            self.undo.remove(0);
        }
        let mut bytes: usize = self.undo.iter().map(|step| step.text.len()).sum();
        while bytes > MOST_BYTES && self.undo.len() > 1 {
            bytes -= self.undo.remove(0).text.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caret(at: usize) -> Vec<Cursor> {
        vec![Cursor::caret(at)]
    }

    #[test]
    fn a_word_typed_is_one_step_and_its_cursor_comes_back() {
        let mut history = History::default();
        let start = Instant::now();
        history.record("", &caret(0), Kind::Typing, start);
        history.record("a", &caret(1), Kind::Typing, start + JOIN / 2);
        history.record("ab", &caret(2), Kind::Typing, start + JOIN);
        let (text, cursors) = history.undo("abc", &caret(3)).unwrap();
        assert_eq!((text.as_str(), cursors), ("", caret(0)));
        let (text, _) = history.redo("", &caret(0)).unwrap();
        assert_eq!(text, "abc");
    }

    #[test]
    fn a_pause_a_change_of_kind_or_a_seal_starts_a_step() {
        let mut history = History::default();
        let start = Instant::now();
        history.record("", &caret(0), Kind::Typing, start);
        history.record("a", &caret(1), Kind::Typing, start + JOIN * 2);
        history.record("ab", &caret(2), Kind::Deleting, start + JOIN * 2);
        history.seal();
        history.record("a", &caret(1), Kind::Deleting, start + JOIN * 2);
        let mut texts = Vec::new();
        let (mut text, mut cursors) = ("x".to_string(), caret(0));
        while let Some((t, c)) = history.undo(&text, &cursors) {
            texts.push(t.clone());
            (text, cursors) = (t, c);
        }
        assert_eq!(texts, ["a", "ab", "a", ""]);
    }

    /// Something new after an undo forgets what could have been redone.
    #[test]
    fn an_edit_after_an_undo_clears_the_redo() {
        let mut history = History::default();
        history.record("", &caret(0), Kind::Other, Instant::now());
        history.undo("a", &caret(1));
        history.record("", &caret(0), Kind::Other, Instant::now());
        assert!(history.redo("b", &caret(1)).is_none());
    }
}
