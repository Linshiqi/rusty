//! Ctrl over a name draws it as a link — VS Code's: underlined, in the
//! link colour, the pointer a hand — where the server knows a definition
//! for it, which is what Ctrl+click would jump to.
//!
//! Asked when the pointer moves with Ctrl held and when Ctrl goes down with
//! the pointer still; the asks are spaced [`THROTTLE`] apart and numbered,
//! so an answer for a word the pointer has left is dropped. The server is
//! asked for the definition itself and the link drawn only where there is
//! one, as VS Code asks. It goes when Ctrl comes up, when the pointer
//! leaves the name, when the editor loses the keys — the window losing
//! focus among them — and at any key.
//!
//! The rows draw it (`rows.rs`): the name's characters in the link colour,
//! not an overlay over them, and a line under them placed by the arithmetic
//! every other mark is (`inlays::ink`), so a hint or a tab before the name
//! moves the link with the text. A Ctrl+click on it goes where the answer
//! said, rather than asking again about the point the click put the caret
//! at: what is underlined is what the click follows.
//!
//! Slint chooses the pointer's shape only as the pointer moves, so a hand
//! that comes or goes with the pointer still — Ctrl pressed and answered,
//! or let go — is made by sending the window the move it last saw again.

use std::{
    cell::{Cell, RefCell},
    time::Duration,
};

use rusty_lsp::Location;
use slint::{
    LogicalPosition, Timer, TimerMode,
    platform::{Key, WindowEvent},
};

use super::{Surface, asks::Asks, cursors};
use crate::Code;

/// How far apart the asks are while the pointer moves with Ctrl held.
const THROTTLE: Duration = Duration::from_millis(60);

/// Where the pointer was last seen over the code: the row on screen and
/// the point in it, and where that is in the window.
#[derive(Clone, Copy, Debug)]
struct Pointer {
    row: usize,
    x: f32,
    y: f32,
    window: (f32, f32),
}

/// A name under the pointer: its document and text, its line, the columns
/// it covers and the column the pointer is on.
#[derive(Clone, Debug)]
struct Name {
    path: String,
    version: u64,
    line: usize,
    from: usize,
    to: usize,
    col: usize,
}

impl Name {
    /// The same name in the same text, wherever along it the pointer is.
    fn is(&self, other: &Name) -> bool {
        (&self.path, self.version, self.line, self.from, self.to)
            == (&other.path, other.version, other.line, other.from, other.to)
    }
}

#[derive(Default)]
pub(super) struct Linking {
    pointer: Cell<Option<Pointer>>,
    ctrl: Cell<bool>,
    timer: Timer,
    asks: Asks,
    /// The name waiting for the throttle or for its answer.
    pending: RefCell<Option<Name>>,
    /// The name asked about that has no definition, not asked about again
    /// while the pointer stays on it.
    refused: RefCell<Option<Name>>,
    /// The name drawn as a link and where it leads.
    shown: RefCell<Option<(Name, Location)>>,
}

/// Whether a key is Ctrl (or the Command key) pressed on its own.
fn is_ctrl(text: &str) -> bool {
    let keys = [Key::Control, Key::ControlR, Key::Meta, Key::MetaR];
    text.chars()
        .next()
        .is_some_and(|ch| keys.iter().any(|key| char::from(*key) == ch))
}

/// Whether a key is a modifier pressed on its own, which is no key to put
/// the hover card or the link away for.
pub(super) fn is_modifier(text: &str) -> bool {
    crate::keys::is_modifier(text)
}

impl Surface {
    /// The pointer moved over row `row` of the code, at `x`, `y` in it and
    /// `window` in the window, with Ctrl held or not.
    pub(super) fn link_over(&self, row: i32, x: f32, y: f32, ctrl: bool, window: (f32, f32)) {
        let Ok(row) = usize::try_from(row) else {
            self.linking.pointer.set(None);
            self.link_away();
            return;
        };
        self.linking
            .pointer
            .set(Some(Pointer { row, x, y, window }));
        self.linking.ctrl.set(ctrl);
        if ctrl {
            self.link_consider();
        } else {
            self.link_away();
        }
    }

    /// The pointer left row `row` — reported after the move onto the next
    /// row may already have been, so only the row it is on counts.
    pub(super) fn link_left(&self, row: i32) {
        let on = self.linking.pointer.get().map(|p| p.row as i32);
        if on == Some(row) {
            self.linking.pointer.set(None);
            self.link_away();
        }
    }

    /// A key went down (`true`) or came up. Ctrl down is a question about
    /// the name under a pointer that has not moved; Ctrl up, and any other
    /// key but a modifier, puts the link away.
    pub(super) fn link_key(&self, text: &str, down: bool) {
        if is_ctrl(text) {
            self.linking.ctrl.set(down);
            if down {
                self.link_consider();
            } else {
                self.link_away();
            }
        } else if down && !is_modifier(text) {
            self.link_away();
        }
    }

    /// The editor lost the keys — to another panel, or the window to
    /// another — and with them any Ctrl it saw go down.
    pub(super) fn link_blurred(&self) {
        self.linking.ctrl.set(false);
        self.link_away();
    }

    /// Where a Ctrl+click on row `row` at `x`, `y` leads when it is on the
    /// name drawn as a link.
    pub(super) fn link_under(&self, row: usize, x: f32, y: f32) -> Option<Location> {
        let doc = self.doc()?;
        let name = self.name_at(&doc, row, x, y)?;
        let shown = self.linking.shown.borrow();
        let (linked, location) = shown.as_ref()?;
        linked.is(&name).then(|| location.clone())
    }

    /// For the probe: the name drawn as a link — its line and the columns
    /// it covers — and where it leads.
    pub fn link_shown(&self) -> Option<(usize, usize, usize, Location)> {
        let shown = self.linking.shown.borrow();
        shown
            .as_ref()
            .map(|(name, location)| (name.line, name.from, name.to, location.clone()))
    }

    /// For the probe: where the pointer was last seen over the code — its
    /// row, its point in the row, and where that is in the window.
    pub fn link_pointer(&self) -> Option<(usize, f32, f32, (f32, f32))> {
        self.linking
            .pointer
            .get()
            .map(|p| (p.row, p.x, p.y, p.window))
    }

    /// The name under the pointer, when there is one.
    fn name_at(
        &self,
        doc: &std::rc::Rc<RefCell<super::Document>>,
        row: usize,
        x: f32,
        y: f32,
    ) -> Option<Name> {
        let (line, at) = self.spot_at(row, x, y)?;
        let d = doc.borrow();
        let (from, to) =
            cursors::word_at(&d.text, at).filter(|&(from, to)| from <= at && at < to)?;
        Some(Name {
            path: d.path.clone(),
            version: d.version,
            line,
            from: d.lines.position(&d.text, from).1,
            to: d.lines.position(&d.text, to).1,
            col: d.lines.position(&d.text, at).1,
        })
    }

    /// Ctrl is held over the code: the name under the pointer is kept when
    /// it is the one drawn or asked about, and asked about otherwise.
    fn link_consider(&self) {
        let Some(pointer) = self.linking.pointer.get() else {
            return;
        };
        let Some(doc) = self.doc() else { return };
        let Some(name) = self.name_at(&doc, pointer.row, pointer.x, pointer.y) else {
            self.link_away();
            return;
        };
        let same =
            |held: &RefCell<Option<Name>>| held.borrow().as_ref().is_some_and(|n| n.is(&name));
        let drawn = self
            .linking
            .shown
            .borrow()
            .as_ref()
            .is_some_and(|(shown, _)| shown.is(&name));
        if drawn || same(&self.linking.pending) || same(&self.linking.refused) {
            return;
        }
        self.link_away();
        *self.linking.pending.borrow_mut() = Some(name);
        if !self.linking.timer.running() {
            let this = self.this.clone();
            self.linking
                .timer
                .start(TimerMode::SingleShot, THROTTLE, move || {
                    if let Some(this) = this.upgrade() {
                        this.link_ask();
                    }
                });
        }
    }

    /// Ask the server where the name waiting leads.
    fn link_ask(&self) {
        let Some(name) = self.linking.pending.borrow().clone() else {
            return;
        };
        let Some(doc) = self.doc() else { return };
        let draft = {
            let d = doc.borrow();
            if d.path != name.path || d.version != name.version {
                return;
            }
            self.servers.draft(&d.text)
        };
        let asked = self.linking.asks.ask();
        let (line, col) = (name.line as u32, name.col as u32);
        // The surface that asked, by number: a window a tab was moved into
        // draws its own links, and one closed since drops the answer.
        let asker = self.id;
        self.servers.ask(
            &name.path.clone(),
            Some(draft),
            move |client, path| client.definition(path, line, col),
            move |app, found| {
                if let Some(this) = app.editor().view(asker) {
                    this.linked(asked, name, found);
                }
            },
        );
    }

    /// The server's answer: the name drawn as a link while it is still the
    /// name asked about, Ctrl is still held and the text is the same.
    fn linked(&self, asked: u64, name: Name, found: Option<Location>) {
        let waiting = self
            .linking
            .pending
            .borrow()
            .as_ref()
            .is_some_and(|n| n.is(&name));
        if !self.linking.asks.answers(asked) || !waiting {
            return;
        }
        *self.linking.pending.borrow_mut() = None;
        let Some(doc) = self.doc() else { return };
        let row = {
            let d = doc.borrow();
            if !self.linking.ctrl.get() || d.path != name.path || d.version != name.version {
                return;
            }
            d.row_of(name.line)
        };
        let Some(location) = found else {
            *self.linking.refused.borrow_mut() = Some(name);
            return;
        };
        self.resting.borrow_mut().link = Some((
            name.path.clone(),
            name.version,
            name.line,
            name.from,
            name.to,
        ));
        self.rows.changed(name.line, name.line + 1);
        *self.linking.shown.borrow_mut() = Some((name, location));
        if let Some(window) = self.window.upgrade() {
            window.global::<Code>().set_link_row(row as i32);
        }
        self.pointer_again();
    }

    /// The link gone, and whatever was waiting to be one forgotten.
    pub(super) fn link_away(&self) {
        if self.unlink() {
            self.pointer_again();
        }
    }

    /// The link's state cleared and its row drawn again; whether one was
    /// drawn.
    pub(super) fn unlink(&self) -> bool {
        self.linking.timer.stop();
        self.linking.asks.forget();
        *self.linking.pending.borrow_mut() = None;
        *self.linking.refused.borrow_mut() = None;
        let shown = self.linking.shown.borrow_mut().take();
        let Some((name, _)) = shown else {
            return false;
        };
        let drawn = self.resting.borrow_mut().link.take();
        if drawn.is_some() {
            let count = self.doc().map_or(0, |doc| doc.borrow().lines.count());
            if name.line < count {
                self.rows.changed(name.line, name.line + 1);
            }
        }
        if let Some(window) = self.window.upgrade() {
            window.global::<Code>().set_link_row(-1);
        }
        true
    }

    /// The window sent the last move it saw again, once this event is
    /// over, so it chooses the pointer's shape for what is under it now.
    fn pointer_again(&self) {
        let Some(pointer) = self.linking.pointer.get() else {
            return;
        };
        let window = self.window.clone();
        Timer::single_shot(Duration::ZERO, move || {
            if let Some(window) = window.upgrade() {
                let (x, y) = pointer.window;
                window.window().dispatch_event(WindowEvent::PointerMoved {
                    position: LogicalPosition::new(x, y),
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: Key) -> String {
        slint::SharedString::from(k).to_string()
    }

    /// Ctrl and the Command key are the link's; a modifier on its own puts
    /// nothing away, and a letter is no modifier.
    #[test]
    fn which_keys_are_ctrl_and_which_are_modifiers() {
        assert!(is_ctrl(&key(Key::Control)) && is_ctrl(&key(Key::ControlR)));
        assert!(!is_ctrl(&key(Key::Shift)) && !is_ctrl("c"));
        for k in [Key::Control, Key::Shift, Key::Alt, Key::AltGr, Key::Meta] {
            assert!(is_modifier(&key(k)));
        }
        assert!(!is_modifier("a") && !is_modifier(&key(Key::LeftArrow)));
        assert!(!is_modifier(""));
    }
}
