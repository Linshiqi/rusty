//! Vim's keys in the code. The machine is `rusty_edit::vim` — a pure
//! function of the key, the text and the cursor, under its own tests; this
//! feeds it the keys and carries out what it says: the new text, where the
//! cursor goes, what is selected, and the few things only the editor can do
//! (save, find, scroll, undo).
//!
//! **The precedence is the whole design.** Normal and visual mode own the
//! unmodified keys and five chords; insert mode claims only Escape, so
//! completion, quick fixes and every learned shortcut keep working the
//! moment anybody types. A key the machine does not take goes on to the
//! editor's keys and the window's, as with Vim off.
//!
//! **The selection is not the cursor, so the cursor is remembered.** In
//! visual mode the selection runs from the anchor through the character the
//! cursor is on, and its head is one past it; read off the selection, every
//! `l` would begin again from the wrong end. The cursor the machine last
//! set is trusted while the document still holds exactly the selection it
//! set for it; anything else — a click, a find, an undo — moved it, and the
//! head is where the cursor now is.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use rusty_edit::vim::{self, Ask, Mode, Vim, motion::Motion};

use super::{
    Command, Document, Surface, cursors::Cursor, history::Kind, inlays, language, lineops,
};

use crate::Code;

#[derive(Default)]
pub(super) struct Modal {
    pub on: Cell<bool>,
    /// One machine per document, by path: its mode, what is pending, its
    /// register.
    machines: RefCell<HashMap<String, Vim>>,
    /// The cursor the machine last set, and the selection it set for it.
    placed: RefCell<Option<Placed>>,
}

#[derive(Clone)]
struct Placed {
    path: String,
    selection: Cursor,
    /// A byte into the text, on the character the cursor stands on.
    cursor: usize,
}

impl Surface {
    /// Vim switched on or off — View, the palette, Settings — every
    /// document starting in normal mode.
    pub fn set_vim(&self, on: bool) {
        self.modal.on.set(on);
        self.modal.machines.borrow_mut().clear();
        *self.modal.placed.borrow_mut() = None;
        self.show_caret();
    }

    /// The mode and what is pending or refused, for the status bar, while
    /// Vim is on and a document is on screen.
    pub fn vim_status(&self) -> Option<(String, String)> {
        if !self.modal.on.get() {
            return None;
        }
        let path = self.current_path()?;
        let machines = self.modal.machines.borrow();
        let machine = machines.get(&path);
        let mode = machine.map_or(Mode::Normal, |m| m.mode);
        Some((
            mode.label().to_string(),
            machine.map(Vim::hint).unwrap_or_default(),
        ))
    }

    /// Whether the document on screen is in a mode whose keys are commands,
    /// where nothing typed or composed is text.
    pub(super) fn vim_commands(&self) -> bool {
        self.modal.on.get()
            && self.current_path().is_some_and(|path| {
                self.modal
                    .machines
                    .borrow()
                    .get(&path)
                    .is_none_or(|m| m.mode != Mode::Insert)
            })
    }

    /// Where Vim's cursor is in `d` while its keys are commands: the one it
    /// last set while the selection is still the one it set, the head
    /// otherwise.
    pub(super) fn vim_cursor(&self, d: &Document) -> Option<usize> {
        if !self.vim_commands() {
            return None;
        }
        let placed = self.modal.placed.borrow();
        Some(match placed.as_ref() {
            Some(placed) if placed.path == d.path && d.cursors == [placed.selection] => {
                placed.cursor.min(d.text.len())
            }
            _ => d.primary().head,
        })
    }

    /// A press: a visual selection is left, as a click in Vim leaves it.
    pub(super) fn vim_pressed(&self) {
        let Some(path) = self.current_path() else {
            return;
        };
        if let Some(machine) = self.modal.machines.borrow_mut().get_mut(&path)
            && machine.mode.is_visual()
        {
            machine.mode = Mode::Normal;
        }
    }

    /// A key, fed to the machine of the document on screen: true when Vim
    /// took it, and nothing else is to act on it.
    pub(super) fn vim_key(&self, text: &str, ctrl: bool, alt: bool) -> bool {
        if !self.modal.on.get() {
            return false;
        }
        let Some(doc) = self.doc() else { return false };
        let Some(name) = key_name(text) else {
            return false;
        };
        let key = vim::Key {
            key: name,
            ctrl,
            alt,
        };
        let (path, body, cursor) = {
            let d = doc.borrow();
            let at = self.vim_cursor(&d).unwrap_or(d.primary().head);
            (d.path.clone(), d.text.clone(), d.text[..at].chars().count())
        };
        let (step, inserting) = {
            let mut machines = self.modal.machines.borrow_mut();
            let machine = machines.entry(path.clone()).or_default();
            let step = machine.feed(&key, &body, cursor);
            (step, machine.mode == Mode::Insert)
        };
        if !step.handled {
            return false;
        }
        // The undo unit closes before the change, so a whole command —
        // `ciwfoo<Esc>` — undoes at once.
        if step.seal {
            doc.borrow_mut().history.seal();
        }
        let after = step.text.clone().unwrap_or_else(|| body.clone());
        let byte = |scalar: usize| byte_of(&after, scalar);
        let at = byte(step.cursor);
        let selection = match step.selection {
            Some((from, to)) => {
                let (from, to) = (byte(from), byte(to));
                if at <= from && from != to {
                    Cursor {
                        anchor: to,
                        head: from,
                    }
                } else {
                    Cursor {
                        anchor: from,
                        head: to,
                    }
                }
            }
            None => Cursor::caret(at),
        };
        *self.modal.placed.borrow_mut() = Some(Placed {
            path,
            selection,
            cursor: at,
        });
        match step.text {
            Some(text) => {
                let change = doc.borrow_mut().edit(text, vec![selection], Kind::Other);
                match change {
                    Some(change) => self.after_edit(&doc, change),
                    None => self.cursors_moved(),
                }
            }
            None => {
                doc.borrow_mut().set_cursors(vec![selection]);
                self.cursors_moved();
            }
        }
        // Into insert mode: what is typed until Escape joins the command's
        // change, so `ciwfoo<Esc>` is one undo.
        if inserting {
            doc.borrow_mut().history.hold();
        }
        if let Some(ask) = step.ask {
            self.vim_ask(ask, &after, step.cursor);
        }
        self.tell_caret();
        true
    }

    /// What the machine asks of the editor.
    fn vim_ask(&self, ask: Ask, text: &str, cursor: usize) {
        match ask {
            // The editor's own history, not a second undo stack that would
            // disagree with Ctrl+Z about what the last change was.
            Ask::Undo | Ask::Redo => {
                self.run(if ask == Ask::Undo {
                    Command::Undo
                } else {
                    Command::Redo
                });
                // On the change's first character, nothing selected, as
                // Vim leaves it.
                self.vim_collapse();
            }
            // The window's own: a window of its own saves and closes its file.
            Ask::Save => self.act("file.save"),
            Ask::Close => self.act("tab.close"),
            Ask::SaveAndClose => {
                self.act("file.save");
                self.act("tab.close");
            }
            // The find bar that already exists, rather than a second
            // search that would drift from it.
            Ask::Search { .. } => self.find_open(false),
            Ask::SearchNext => self.vim_search(true),
            Ask::SearchPrevious => self.vim_search(false),
            Ask::Replace => self.find_open(true),
            Ask::SearchWord { backwards } => {
                let at = byte_of(text, cursor);
                if let Some((from, to)) = super::cursors::word_at(text, at) {
                    self.find_word(&text[from..to], !backwards);
                    self.vim_collapse();
                }
            }
            Ask::Centre { at } => self.vim_centre(at),
            // Half a screen, which only the editor knows the height of. The
            // cursor goes with the view, as Vim's Ctrl+D moves both.
            Ask::Scroll { down } => {
                let half = (self.page() / 2).max(1) as usize;
                let motion = if down { Motion::Down } else { Motion::Up };
                if let Some(span) = vim::motion::apply(motion, text, cursor, half, &None) {
                    self.scroll_rows(if down { half as f32 } else { -(half as f32) });
                    let at = Cursor::caret(byte_of(text, span.cursor));
                    if let Some(placed) = self.modal.placed.borrow_mut().as_mut() {
                        placed.selection = at;
                        placed.cursor = at.head;
                    }
                    self.set_cursors(vec![at]);
                }
            }
            Ask::Jump { back } => self.act(if back { "nav.back" } else { "nav.forward" }),
            // Comment syntax is the document's language, which the editor
            // knows and the machine deliberately does not.
            Ask::Comment { from, to } => {
                let Some(doc) = self.doc() else { return };
                let marker = language::of(&doc.borrow().path).and_then(|l| l.comment);
                let span = Cursor {
                    anchor: byte_of(text, from),
                    head: byte_of(text, to),
                };
                let commented = marker.and_then(|marker| lineops::commented(text, &[span], marker));
                if let Some((out, _)) = commented {
                    let caret = Cursor::caret(byte_of(&out, from));
                    let change = doc.borrow_mut().edit(out, vec![caret], Kind::Other);
                    if let Some(change) = change {
                        self.after_edit(&doc, change);
                    }
                }
            }
        }
    }

    /// `n` and `N`: the next match from the cursor, with the cursor on its
    /// first character, as Vim leaves it.
    fn vim_search(&self, forward: bool) {
        if !self.finding_open() {
            self.find_open(false);
            return;
        }
        self.find_next(forward);
        self.vim_collapse();
    }

    /// A match the find bar selected, as Vim's cursor: on its first
    /// character, nothing selected.
    fn vim_collapse(&self) {
        let Some(doc) = self.doc() else { return };
        let start = doc.borrow().primary().start();
        let at = Cursor::caret(start);
        *self.modal.placed.borrow_mut() = self.current_path().map(|path| Placed {
            path,
            selection: at,
            cursor: start,
        });
        self.set_cursors(vec![at]);
    }

    /// `zz`, `zt`, `zb`: the cursor's line in the middle, at the top or at
    /// the bottom of the view.
    fn vim_centre(&self, at: vim::View) {
        let (Some(window), Some(doc)) = (self.window.upgrade(), self.doc()) else {
            return;
        };
        let code = window.global::<Code>();
        let row_height = code.get_row_height();
        let view = code.get_view_height();
        let row = {
            let d = doc.borrow();
            let head = self.vim_cursor(&d).unwrap_or(d.primary().head);
            d.row_of(d.lines.line_of(head)) as f32
        };
        let top = match at {
            vim::View::Top => row * row_height,
            vim::View::Middle => row * row_height - (view - row_height) / 2.0,
            vim::View::Bottom => row * row_height - (view - row_height),
        };
        code.set_scroll_y(-top.max(0.0));
        self.view_changed();
    }

    /// The view scrolled by `rows`, as far as there is text to show — the
    /// last line at the top at most.
    fn scroll_rows(&self, rows: f32) {
        let (Some(window), Some(doc)) = (self.window.upgrade(), self.doc()) else {
            return;
        };
        let code = window.global::<Code>();
        let row_height = code.get_row_height();
        let most = super::reach::deepest(doc.borrow().rows(), row_height, code.get_view_height());
        let top = (-code.get_scroll_y() + rows * row_height).clamp(0.0, most);
        code.set_scroll_y(-top);
        self.view_changed();
    }

    /// The block cursor's width: the character it stands on, or a space's
    /// past the end of a line, as Vim draws it on an empty one.
    pub(super) fn block_width(&self, d: &Document, at: usize) -> f32 {
        let metrics = self.metrics.borrow();
        let (line, col) = d.lines.position(&d.text, at);
        let text = d.lines.text(&d.text, line);
        if col >= text.chars().count() {
            return metrics.ascii();
        }
        inlays::char_on(&metrics, d, line, col + 1) - inlays::char_on(&metrics, d, line, col)
    }
}

/// A byte into `text` for a character index, the end past the last.
fn byte_of(text: &str, scalar: usize) -> usize {
    text.char_indices()
        .nth(scalar)
        .map_or(text.len(), |(at, _)| at)
}

/// A key as the machine names it: a character, or for a key that types none
/// the `KeyboardEvent.key` name it was written against (`ArrowUp`,
/// `Escape`). `None` for a key it has no name for (a modifier alone, a
/// function key), which is left to the editor and the window.
fn key_name(text: &str) -> Option<String> {
    use slint::platform::Key as K;
    let ch = text.chars().next()?;
    let named = [
        (K::Escape, "Escape"),
        (K::Return, "Enter"),
        (K::Backspace, "Backspace"),
        (K::Delete, "Delete"),
        (K::Tab, "Tab"),
        (K::Backtab, "Tab"),
        (K::UpArrow, "ArrowUp"),
        (K::DownArrow, "ArrowDown"),
        (K::LeftArrow, "ArrowLeft"),
        (K::RightArrow, "ArrowRight"),
        (K::Home, "Home"),
        (K::End, "End"),
        (K::PageUp, "PageUp"),
        (K::PageDown, "PageDown"),
    ];
    if let Some((_, name)) = named.iter().find(|(key, _)| char::from(*key) == ch) {
        return Some((*name).to_string());
    }
    // Slint keeps the control range and a private-use block for keys of
    // its own: Shift alone is 0x10, the function keys are past 0xF700.
    if ch.is_control() || ('\u{E000}'..='\u{F8FF}').contains(&ch) {
        return None;
    }
    Some(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_named_as_the_machine_reads_them() {
        let named = |key: slint::platform::Key| key_name(&slint::SharedString::from(key));
        assert_eq!(
            named(slint::platform::Key::Escape).as_deref(),
            Some("Escape")
        );
        assert_eq!(
            named(slint::platform::Key::Return).as_deref(),
            Some("Enter")
        );
        assert_eq!(named(slint::platform::Key::Shift), None, "a modifier alone");
        assert_eq!(named(slint::platform::Key::F5), None, "the window's");
        assert_eq!(key_name("A").as_deref(), Some("A"), "Shift baked in");
        assert_eq!(key_name("中").as_deref(), Some("中"));
    }

    #[test]
    fn a_character_index_is_a_byte_on_its_boundary() {
        assert_eq!(byte_of("a中b", 2), 4);
        assert_eq!(byte_of("a中b", 9), 5);
    }
}
