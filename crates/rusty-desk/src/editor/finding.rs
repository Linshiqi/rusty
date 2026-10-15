//! The find bar on the surface: what it searches for, what it found, and
//! what Enter, a toggle, Replace and Escape do to the document. The
//! arithmetic is `find.rs`; this applies it and keeps the bar and the rows
//! in step with it.

use std::rc::Rc;

use rusty_i18n::t;
use slint::{ComponentHandle, SharedString};

use super::{
    Change, Cursor, Kind, Surface,
    find::{self, Marks, Query},
};
use crate::Find;

/// Which rows new marks have to be drawn on again.
#[derive(Clone, Copy, Debug)]
enum Redraw {
    /// The query or the scope changed: every row with a mark, before or
    /// after.
    All,
    /// The text changed: rows whose marks came or went, measured across
    /// the edit — the edited rows themselves the edit has redrawn.
    Edit(Change),
    /// Only which match the selection is.
    Current,
}

/// The bar's state.
#[derive(Default)]
pub struct Finding {
    pub open: bool,
    pub query: Query,
    pub replacement: String,
    pub scope: Option<(usize, usize)>,
    /// Where typing in the field searches from: the selection when the bar
    /// opened, then each match stepped to — as VS Code's search moves on
    /// from where it began rather than from wherever the last letter left
    /// the selection.
    pub start: usize,
    /// What a regular expression that does not read says about itself.
    pub error: Option<String>,
    pub capped: bool,
    /// The length of the text last searched, which an edit's change of
    /// length is measured against.
    searched: usize,
    /// The lines marks are drawn on, and the current match's line, as they
    /// were when drawn — in that text's lines, which an edit has since moved.
    drawn: Vec<usize>,
    drawn_current: Option<usize>,
}

impl Surface {
    pub(super) fn wire_find(self: &Rc<Self>, find: &Find) {
        macro_rules! on {
            ($method:ident, |$this:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(self);
                find.$method(move |$($arg),*| {
                    if let Some($this) = weak.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(on_edited, |this, text| this.find_typed(text.into()));
        on!(on_replacement_edited, |this, text| this
            .finding
            .borrow_mut()
            .replacement =
            text.into());
        on!(on_toggle, |this, which| this.find_toggle(&which));
        on!(on_step, |this, forward| this.find_step(forward));
        on!(on_replace, |this, all| this.find_replace(all));
        on!(on_select_all, |this| this.find_select_all());
        on!(on_close, |this| this.find_close());
    }

    /// Ctrl+F, or Ctrl+H with `replacing`: the bar open with the selection
    /// or the word at the caret in it, and the keys in its field.
    pub fn find_open(&self, replacing: bool) {
        let Some(doc) = self.doc() else { return };
        {
            let d = doc.borrow();
            let primary = d.primary();
            let mut f = self.finding.borrow_mut();
            if let Some(seed) = find::seed(&d.text, (primary.start(), primary.end())) {
                f.query.text = seed;
            }
            f.open = true;
            f.start = primary.start();
        }
        if let Some(window) = self.window.upgrade() {
            let bar = window.global::<Find>();
            bar.set_query(SharedString::from(&self.finding.borrow().query.text));
            if replacing {
                bar.set_replacing(true);
            }
            bar.set_open(true);
            window.invoke_focus_find(replacing);
        }
        self.research(None);
    }

    /// Vim's `*` and `#`: the word under the cursor as the query, whole,
    /// and the next or the previous one selected — the keys stay in the
    /// code, where `n` goes on from it.
    pub(super) fn find_word(&self, word: &str, forward: bool) {
        let Some(doc) = self.doc() else { return };
        {
            let mut f = self.finding.borrow_mut();
            f.query.text = word.to_string();
            f.query.word = true;
            f.open = true;
            f.start = doc.borrow().primary().start();
        }
        if let Some(window) = self.window.upgrade() {
            let bar = window.global::<Find>();
            bar.set_query(SharedString::from(word));
            bar.set_open(true);
        }
        self.research(None);
        self.find_step(forward);
    }

    /// F3 and Shift+F3 with the bar closed open it first, as the next step
    /// needs a query.
    pub fn find_next(&self, forward: bool) {
        if !self.finding.borrow().open {
            self.find_open(false);
        }
        self.find_step(forward);
    }

    fn find_typed(&self, text: String) {
        self.finding.borrow_mut().query.text = text;
        self.research(None);
        self.select_from_start();
    }

    fn find_toggle(&self, which: &str) {
        {
            let mut f = self.finding.borrow_mut();
            match which {
                "case" => f.query.case = !f.query.case,
                "word" => f.query.word = !f.query.word,
                "regex" => f.query.regex = !f.query.regex,
                "scope" => {
                    f.scope = match f.scope {
                        Some(_) => None,
                        None => self.doc().map(|doc| {
                            let d = doc.borrow();
                            let primary = d.primary();
                            // Nothing selected is the caret's line.
                            if primary.is_caret() {
                                let line = d.lines.line_of(primary.head);
                                (d.lines.start(line), d.lines.end(line))
                            } else {
                                (primary.start(), primary.end())
                            }
                        }),
                    }
                }
                _ => {}
            }
        }
        self.research(None);
        if which != "scope" {
            self.select_from_start();
        }
    }

    /// Enter and Shift+Enter: the match after the selection, or before it,
    /// selected and on screen, wrapping at the ends.
    fn find_step(&self, forward: bool) {
        let Some(doc) = self.doc() else { return };
        let primary = doc.borrow().primary();
        let target = {
            let marks = self.found.borrow();
            let matches = &marks.matches;
            let n = matches.len();
            match (
                find::current(matches, (primary.start(), primary.end())),
                forward,
            ) {
                (Some(at), true) => (n > 0).then(|| (at + 1) % n),
                (Some(at), false) => (n > 0).then(|| (at + n - 1) % n),
                (None, true) => find::after(matches, primary.end()),
                (None, false) => find::before(matches, primary.start()),
            }
            .map(|at| matches[at])
        };
        if let Some(found) = target {
            self.select_match(found);
        }
    }

    /// Replace the match the selection is and go on to the next; with
    /// `all`, every match in the scope, as one step of undo.
    fn find_replace(&self, all: bool) {
        let Some(doc) = self.doc() else { return };
        let (text, primary) = {
            let d = doc.borrow();
            (d.text.clone(), d.primary())
        };
        let (query, scope, with) = {
            let f = self.finding.borrow();
            (f.query.clone(), f.scope, f.replacement.clone())
        };
        if all {
            let Some((replaced, landed)) = query.replace_all(&text, scope, &with) else {
                return;
            };
            let matches = self.found.borrow().matches.clone();
            let at = find::carried(primary.head, &matches, &landed);
            let change = doc
                .borrow_mut()
                .edit(replaced, vec![Cursor::caret(at)], Kind::Other);
            if let Some(change) = change {
                self.after_edit(&doc, change);
            }
            return;
        }
        let current = {
            let marks = self.found.borrow();
            find::current(&marks.matches, (primary.start(), primary.end()))
                .map(|at| marks.matches[at])
        };
        let Some((from, to)) = current else {
            self.find_step(true);
            return;
        };
        let written = query.replacement(&text, (from, to), &with);
        let mut replaced = String::with_capacity(text.len() + written.len());
        replaced.push_str(&text[..from]);
        replaced.push_str(&written);
        replaced.push_str(&text[to..]);
        let past = from + written.len();
        let change = doc
            .borrow_mut()
            .edit(replaced, vec![Cursor::caret(past)], Kind::Other);
        if let Some(change) = change {
            self.after_edit(&doc, change);
        }
        let next = {
            let marks = self.found.borrow();
            find::after(&marks.matches, past).map(|at| marks.matches[at])
        };
        if let Some(found) = next {
            self.select_match(found);
        }
    }

    /// Alt+Enter: a cursor on every match, and the keys back in the code.
    fn find_select_all(&self) {
        let matches = self.found.borrow().matches.clone();
        if matches.is_empty() {
            return;
        }
        let cursors = matches
            .iter()
            .map(|&(anchor, head)| Cursor { anchor, head })
            .collect();
        self.set_cursors(cursors);
        if let Some(window) = self.window.upgrade() {
            window.invoke_focus_code();
        }
    }

    /// Escape: the bar and its marks gone, the keys back in the code with
    /// the match that was found still selected.
    pub fn find_close(&self) {
        self.finding.borrow_mut().open = false;
        self.set_marks(Marks::default(), Redraw::All);
        if let Some(window) = self.window.upgrade() {
            window.global::<Find>().set_open(false);
            window.invoke_focus_code();
        }
    }

    pub(super) fn finding_open(&self) -> bool {
        self.finding.borrow().open
    }

    fn select_match(&self, (from, to): (usize, usize)) {
        self.finding.borrow_mut().start = from;
        self.set_cursors(vec![Cursor {
            anchor: from,
            head: to,
        }]);
    }

    /// While typing in the field: the first match from where the search
    /// began, when there is one.
    fn select_from_start(&self) {
        let start = self.finding.borrow().start;
        let target = {
            let marks = self.found.borrow();
            find::after(&marks.matches, start).map(|at| marks.matches[at])
        };
        if let Some((from, to)) = target {
            self.set_cursors(vec![Cursor {
                anchor: from,
                head: to,
            }]);
        }
    }

    /// Search the document on screen again: the query changed, or the
    /// text did (`change`, which also carries the scope across the edit).
    pub(super) fn research(&self, change: Option<Change>) {
        let Some(doc) = self.doc() else { return };
        if !self.finding.borrow().open {
            return;
        }
        let marks = {
            let d = doc.borrow();
            let mut f = self.finding.borrow_mut();
            if let (Some(change), Some(scope)) = (change, f.scope) {
                let from = d.lines.start(change.first);
                let to = if change.first + change.added < d.lines.count() {
                    d.lines.start(change.first + change.added)
                } else {
                    d.text.len()
                };
                let delta = d.text.len() as isize - f.searched as isize;
                f.scope = Some(find::follow(scope, from, to, delta));
            }
            f.searched = d.text.len();
            let primary = d.primary();
            match f.query.search(&d.text, f.scope) {
                Ok(found) => {
                    f.error = None;
                    f.capped = found.capped;
                    Marks {
                        current: find::current(&found.matches, (primary.start(), primary.end())),
                        matches: found.matches,
                        scope: f.scope,
                    }
                }
                Err(reason) => {
                    f.error = Some(reason);
                    f.capped = false;
                    Marks {
                        scope: f.scope,
                        ..Marks::default()
                    }
                }
            }
        };
        self.set_marks(marks, change.map_or(Redraw::All, Redraw::Edit));
    }

    /// The selection moved: which match it is, if any, is drawn and counted.
    pub(super) fn find_follow_selection(&self) {
        if !self.finding.borrow().open {
            return;
        }
        let Some(doc) = self.doc() else { return };
        let primary = doc.borrow().primary();
        let mut marks = self.found.borrow().clone();
        let current = find::current(&marks.matches, (primary.start(), primary.end()));
        if current != marks.current {
            marks.current = current;
            self.set_marks(marks, Redraw::Current);
        }
    }

    /// Put `marks` on the rows, asking again for the rows `redraw` says, and
    /// say the count.
    fn set_marks(&self, marks: Marks, redraw: Redraw) {
        let Some(doc) = self.doc() else {
            *self.found.borrow_mut() = marks;
            return;
        };
        let changed: Vec<usize> = {
            let d = doc.borrow();
            // Where a row drawn before an edit is now: below what it
            // replaced it moved with the lines added or taken away, and
            // inside it the edit redrew it already.
            let moved = |line: usize| match redraw {
                Redraw::Edit(c) if line >= c.first + c.removed => Some(line + c.added - c.removed),
                Redraw::Edit(c) if line >= c.first => None,
                _ => Some(line),
            };
            let mut f = self.finding.borrow_mut();
            let after_current = marks.current.map(|at| d.lines.line_of(marks.matches[at].0));
            let mut changed: Vec<usize> = Vec::new();
            changed.extend(f.drawn_current.and_then(moved));
            changed.extend(after_current);
            match redraw {
                Redraw::Current => {}
                Redraw::All | Redraw::Edit(_) => {
                    let before: Vec<usize> = f.drawn.iter().copied().filter_map(moved).collect();
                    let after = marks.lines(&d.lines);
                    if let Redraw::All = redraw {
                        changed.extend(&before);
                        changed.extend(&after);
                    } else {
                        changed.extend(before.iter().filter(|l| after.binary_search(l).is_err()));
                        changed.extend(after.iter().filter(|l| before.binary_search(l).is_err()));
                    }
                    f.drawn = after;
                }
            }
            f.drawn_current = after_current;
            changed.sort_unstable();
            changed.dedup();
            changed
        };
        *self.found.borrow_mut() = marks;
        if changed.len() > 400 {
            self.rows.reset();
        } else {
            for line in changed {
                self.rows.changed(line, line + 1);
            }
        }
        self.say_count();
    }

    fn say_count(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let f = self.finding.borrow();
        let marks = self.found.borrow();
        let (count, trouble) = if let Some(reason) = &f.error {
            (t!("desk.find-invalid", reason = reason.clone()), true)
        } else if f.query.text.is_empty() {
            (String::new(), false)
        } else if marks.matches.is_empty() {
            (t!("desk.find-none"), true)
        } else {
            let at = marks
                .current
                .map_or_else(|| "?".to_string(), |at| (at + 1).to_string());
            let total = if f.capped {
                format!("{}+", marks.matches.len())
            } else {
                marks.matches.len().to_string()
            };
            (t!("desk.find-count", at = at, count = total), false)
        };
        let bar = window.global::<Find>();
        bar.set_count(count.into());
        bar.set_trouble(trouble);
        bar.set_case(f.query.case);
        bar.set_word(f.query.word);
        bar.set_regex(f.query.regex);
        bar.set_scoped(f.scope.is_some());
    }
}
