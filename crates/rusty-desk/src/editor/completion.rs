//! What the server offers while typing: the completion popup under the
//! word being typed, and the signature of the call the caret is in above
//! it. When to ask, what to show and what a snippet inserts are
//! `rusty_lsp::complete`'s rules; this is asking, showing and accepting.
//!
//! Every ask is numbered and anchored to the line and the word it was
//! about, and an answer is shown only while that is still the word being
//! typed — an answer landing after the caret moved on is dropped, rather
//! than opening a popup for a word nobody is typing.

use std::{cell::Cell, cell::RefCell, time::Duration};

use rusty_lsp::{
    ActionEdit, CompletionItem, CompletionList, SignatureInfo,
    complete::{self, Ask, Showing},
};
use slint::{ModelRc, Timer, TimerMode, VecModel};

use super::{Cursor, Kind, Surface};
use crate::{Code, CompletionRow};

/// Rows the popup draws at once; the keys reach the rest.
const ROWS: usize = 10;
/// How many ranked items the keys can reach.
const SHOWN: usize = 50;
/// A word already showing is asked about again once typing pauses this
/// long, while the server says its answer was incomplete.
const AGAIN_AFTER: Duration = Duration::from_millis(40);

/// The popup: the word it is about, what the server offered, and which row
/// the keys are on.
struct Popup {
    line: u32,
    word_start: u32,
    items: Vec<CompletionItem>,
    incomplete: bool,
    reply: u64,
    /// The ranked items shown, by index into `items`.
    shown: Vec<usize>,
    selected: usize,
}

#[derive(Default)]
pub struct Completing {
    popup: RefCell<Option<Popup>>,
    /// An ask on its way: its number, line and word.
    pending: Cell<Option<(u64, u32, u32)>>,
    asked: Cell<u64>,
    again: Timer,
    /// The signature card: its ask's number, and whether it is up.
    signature_asked: Cell<u64>,
    signature_open: Cell<bool>,
}

/// A row's glyph and the token kind it is coloured as, by the item's kind.
fn glyph(kind: Option<&str>) -> (&'static str, i32) {
    match kind.unwrap_or("") {
        "function" | "method" | "constructor" => ("ƒ", 6),
        "struct" | "class" | "interface" | "enum" | "type parameter" => ("T", 5),
        "enum member" | "constant" | "value" | "unit" => ("c", 3),
        "field" | "property" | "variable" => ("v", 9),
        "module" | "folder" | "file" => ("m", 10),
        "keyword" => ("k", 1),
        "snippet" => ("s", 7),
        _ => ("·", 0),
    }
}

impl Surface {
    /// The text before the caret on its line, and where the caret is —
    /// with one cursor; several type what nobody completes.
    fn before_caret(&self) -> Option<(Vec<char>, u32, u32, String)> {
        let doc = self.doc()?;
        let d = doc.borrow();
        if d.cursors.len() != 1 || !d.primary().is_caret() {
            return None;
        }
        let (line, col) = d.lines.position(&d.text, d.primary().head);
        let before: Vec<char> = d.lines.text(&d.text, line).chars().take(col).collect();
        Some((before, line as u32, col as u32, d.path.clone()))
    }

    /// Typing (or deleting, when `typing` is false) changed the text: what
    /// that asks of the popup and of the signature card.
    pub(super) fn after_typing(&self, typing: bool) {
        let Some((before, line, col, _)) = self.before_caret() else {
            self.close_completion();
            return;
        };
        let showing = {
            let popup = self.completing.popup.borrow();
            match (popup.as_ref(), self.completing.pending.get()) {
                (Some(p), _) => Some(Showing {
                    line: p.line,
                    word_start: p.word_start,
                    incomplete: p.incomplete,
                }),
                (None, Some((_, line, word_start))) => Some(Showing {
                    line,
                    word_start,
                    incomplete: true,
                }),
                (None, None) => None,
            }
        };
        match complete::ask_for(&before, line, showing, typing) {
            Ask::Keep => self.narrow(),
            Ask::Close => self.close_completion(),
            Ask::Complete { word_start, now } => {
                self.narrow();
                if now {
                    self.ask_completion(line, word_start, col);
                } else {
                    let weak = self.this.clone();
                    self.completing
                        .again
                        .start(TimerMode::SingleShot, AGAIN_AFTER, move || {
                            if let Some(this) = weak.upgrade()
                                && let Some((_, line, col, _)) = this.before_caret()
                            {
                                this.ask_completion(line, word_start, col);
                            }
                        });
                }
            }
        }
        // The signature: opened by `(` and `,`, and asked again while up.
        let opens = typing && matches!(before.last(), Some('(' | ','));
        if opens || self.completing.signature_open.get() {
            self.ask_signature();
        }
    }

    /// Ctrl+Space: ask about the word at the caret whatever was typed.
    pub(super) fn suggest(&self) {
        let Some((before, line, col, _)) = self.before_caret() else {
            return;
        };
        let word = before
            .iter()
            .rev()
            .take_while(|c| c.is_alphanumeric() || **c == '_')
            .count() as u32;
        self.ask_completion(line, col - word, col);
    }

    fn ask_completion(&self, line: u32, word_start: u32, col: u32) {
        let Some(doc) = self.doc() else { return };
        let (path, draft) = {
            let d = doc.borrow();
            (d.path.clone(), self.servers.draft(&d.text))
        };
        let asked = self.completing.asked.get() + 1;
        self.completing.asked.set(asked);
        self.completing.pending.set(Some((asked, line, word_start)));
        let asker = self.id;
        self.servers.ask(
            &path,
            Some(draft),
            move |client, path| client.completion(path, line, col),
            move |app, list| {
                if let Some(this) = app.editor().view(asker) {
                    this.completed(asked, line, word_start, list);
                }
            },
        );
    }

    /// The server's answer: shown while its word is still the word typed.
    pub fn completed(&self, asked: u64, line: u32, word_start: u32, list: CompletionList) {
        if self.completing.pending.get().map(|(n, ..)| n) != Some(asked) {
            return;
        }
        self.completing.pending.set(None);
        let still = self
            .before_caret()
            .is_some_and(|(before, l, ..)| l == line && before.len() as u32 >= word_start);
        if !still {
            return;
        }
        *self.completing.popup.borrow_mut() = Some(Popup {
            line,
            word_start,
            items: list.items,
            incomplete: list.incomplete,
            reply: list.reply,
            shown: Vec::new(),
            selected: 0,
        });
        self.narrow();
    }

    /// The popup's rows for the word as typed now, the keys back on the
    /// first — kept on a row that is gone, Enter accepted something else.
    fn narrow(&self) {
        let Some(typed) = self.typed_word() else {
            return;
        };
        {
            let mut popup = self.completing.popup.borrow_mut();
            let Some(popup) = popup.as_mut() else { return };
            popup.shown = complete::ranked(&popup.items, &typed)
                .into_iter()
                .take(SHOWN)
                .collect();
            popup.selected = 0;
        }
        self.sync_completion();
    }

    fn typed_word(&self) -> Option<String> {
        let popup = self.completing.popup.borrow();
        let popup = popup.as_ref()?;
        let doc = self.doc()?;
        let d = doc.borrow();
        Some(complete::typed_word(&d.text, popup.line, popup.word_start))
    }

    /// Whether the popup is up with rows in it: only then are its keys its
    /// own — narrowed to nothing it is invisible, and Enter breaks the line.
    fn popup_visible(&self) -> bool {
        self.completing
            .popup
            .borrow()
            .as_ref()
            .is_some_and(|p| !p.shown.is_empty())
    }

    /// The keys the popup and the signature card take while they are up:
    /// Up and Down (wrapping), Page Up and Down, Enter and Tab to accept,
    /// Escape to put them away.
    pub(super) fn completion_key(&self, key: &str, modified: bool) -> bool {
        use slint::platform::Key;
        let is = |k: Key| key.starts_with(char::from(k));
        if self.popup_visible() && !modified {
            let step: Option<isize> = match () {
                _ if is(Key::DownArrow) => Some(1),
                _ if is(Key::UpArrow) => Some(-1),
                _ if is(Key::PageDown) => Some(ROWS as isize),
                _ if is(Key::PageUp) => Some(-(ROWS as isize)),
                _ => None,
            };
            if let Some(step) = step {
                {
                    let mut popup = self.completing.popup.borrow_mut();
                    if let Some(p) = popup.as_mut() {
                        let n = p.shown.len() as isize;
                        p.selected = if step.abs() == 1 {
                            ((p.selected as isize + step).rem_euclid(n)) as usize
                        } else {
                            (p.selected as isize + step).clamp(0, n - 1) as usize
                        };
                    }
                }
                self.sync_completion();
                return true;
            }
            if is(Key::Return) || is(Key::Tab) {
                let selected = self.completing.popup.borrow().as_ref().map(|p| p.selected);
                if let Some(selected) = selected {
                    self.accept(selected);
                }
                return true;
            }
            if is(Key::Escape) {
                self.close_completion();
                return true;
            }
        }
        if self.completing.signature_open.get() && is(Key::Escape) && !modified {
            self.close_signature();
            return true;
        }
        false
    }

    /// A row of the popup clicked.
    pub(super) fn completion_pick(&self, row: usize) {
        let top = self.completion_top();
        self.accept(top + row);
    }

    /// The first row drawn: the window of rows follows the keys.
    fn completion_top(&self) -> usize {
        let popup = self.completing.popup.borrow();
        popup.as_ref().map_or(0, |p| {
            p.selected
                .saturating_sub(ROWS - 1)
                .min(p.shown.len().saturating_sub(ROWS))
        })
    }

    /// Put the `at`th shown item in: over the word typed (or the range the
    /// server named), a snippet expanded with its first placeholder
    /// selected, then the edits the item brings besides — an import — asked
    /// for and put above.
    fn accept(&self, at: usize) {
        let Some(doc) = self.doc() else { return };
        let (item, reply) = {
            let popup = self.completing.popup.borrow();
            let Some(popup) = popup.as_ref() else { return };
            let Some(&index) = popup.shown.get(at) else {
                return;
            };
            (popup.items[index].clone(), popup.reply)
        };
        let word_start = self
            .completing
            .popup
            .borrow()
            .as_ref()
            .map(|p| p.word_start);
        self.close_completion();
        let (text, cursor, path) = {
            let d = doc.borrow();
            let head = d.primary().head;
            let line = d.lines.line_of(head);
            let start = match item.edit {
                Some(range) => {
                    d.lines
                        .byte(&d.text, range.start_line as usize, range.start_col as usize)
                }
                None => d
                    .lines
                    .byte(&d.text, line, word_start.unwrap_or(0) as usize),
            }
            .min(head);
            let end = item
                .edit
                .filter(|range| range.end_line as usize == line)
                .map(|range| d.lines.byte(&d.text, line, range.end_col as usize))
                .unwrap_or(head)
                .max(head);
            let indent: String = d
                .lines
                .text(&d.text, line)
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            let (inserted, select) = if item.snippet {
                let expanded = complete::expand_snippet(&item.insert, &indent);
                (expanded.text, expanded.select)
            } else {
                let length = item.insert.chars().count();
                (item.insert.clone(), (length, length))
            };
            let byte = |chars: usize| {
                start
                    + inserted
                        .char_indices()
                        .nth(chars)
                        .map_or(inserted.len(), |(at, _)| at)
            };
            let cursor = Cursor {
                anchor: byte(select.0),
                head: byte(select.1),
            };
            let mut text = String::with_capacity(d.text.len() + inserted.len());
            text.push_str(&d.text[..start]);
            text.push_str(&inserted);
            text.push_str(&d.text[end..]);
            (text, cursor, d.path.clone())
        };
        let change = doc.borrow_mut().edit(text, vec![cursor], Kind::Other);
        if let Some(change) = change {
            self.after_edit(&doc, change);
        }
        let version = doc.borrow().version;
        let index = item.index;
        let asker = self.id;
        self.servers.ask(
            &path,
            None,
            move |client, path| client.resolve_completion(path, reply, index),
            move |app, edits| {
                if let Some(this) = app.editor().view(asker) {
                    this.apply_resolved(version, edits);
                }
            },
        );
        // Between the parentheses of a call: what it takes.
        if self
            .before_caret()
            .is_some_and(|(before, ..)| before.last() == Some(&'('))
        {
            self.ask_signature();
        }
    }

    /// The edits an accepted item brings besides itself — an import —
    /// applied while the text is still the one it was accepted into, the
    /// caret moved along by what they put above it.
    pub fn apply_resolved(&self, version: u64, edits: Vec<ActionEdit>) {
        let Some(doc) = self.doc() else { return };
        if edits.is_empty() || doc.borrow().version != version {
            return;
        }
        let (text, cursor) = {
            let d = doc.borrow();
            let mut ranges: Vec<(usize, usize, &str)> = edits
                .iter()
                .map(|e| {
                    let r = e.range;
                    (
                        d.lines
                            .byte(&d.text, r.start_line as usize, r.start_col as usize),
                        d.lines
                            .byte(&d.text, r.end_line as usize, r.end_col as usize),
                        e.new_text.as_str(),
                    )
                })
                .collect();
            ranges.sort_by_key(|r| std::cmp::Reverse(r.0));
            let primary = d.primary();
            let (mut anchor, mut head) = (primary.anchor, primary.head);
            let mut text = d.text.clone();
            for (from, to, with) in ranges {
                let delta = with.len() as isize - (to - from) as isize;
                if to <= head {
                    head = (head as isize + delta) as usize;
                }
                if to <= anchor {
                    anchor = (anchor as isize + delta) as usize;
                }
                text.replace_range(from..to, with);
            }
            (text, Cursor { anchor, head })
        };
        let change = doc.borrow_mut().edit(text, vec![cursor], Kind::Other);
        if let Some(change) = change {
            self.after_edit(&doc, change);
        }
    }

    /// The popup away, and any answer on its way dropped.
    pub(super) fn close_completion(&self) {
        self.completing.again.stop();
        self.completing.pending.set(None);
        self.completing.asked.set(self.completing.asked.get() + 1);
        let was_open = self.completing.popup.borrow_mut().take().is_some();
        if was_open && let Some(window) = self.window.upgrade() {
            window.global::<Code>().set_completion_open(false);
        }
    }

    fn sync_completion(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let code = window.global::<Code>();
        let top = self.completion_top();
        let popup = self.completing.popup.borrow();
        let Some(popup) = popup.as_ref().filter(|p| !p.shown.is_empty()) else {
            code.set_completion_open(false);
            return;
        };
        let rows: Vec<CompletionRow> = popup
            .shown
            .iter()
            .skip(top)
            .take(ROWS)
            .map(|&index| {
                let item = &popup.items[index];
                let (glyph, kind) = glyph(item.kind.as_deref());
                CompletionRow {
                    label: item.label.as_str().into(),
                    extra: item.label_detail.clone().unwrap_or_default().into(),
                    detail: item
                        .description
                        .clone()
                        .or_else(|| item.detail.clone())
                        .unwrap_or_default()
                        .into(),
                    glyph: glyph.into(),
                    kind,
                }
            })
            .collect();
        let (x, row) = self.doc().map_or((0.0, popup.line as usize), |doc| {
            let d = doc.borrow();
            let (line, start) = (popup.line as usize, popup.word_start as usize);
            (
                super::inlays::char_on(&self.metrics.borrow(), &d, line, start),
                d.row_of(popup.line as usize),
            )
        });
        code.set_completion_line(row as i32);
        code.set_completion_x(x);
        code.set_completion_rows(ModelRc::new(VecModel::from(rows)));
        code.set_completion_selected((popup.selected - top) as i32);
        code.set_completion_count(popup.shown.len() as i32);
        code.set_completion_open(true);
    }

    // ---- the signature card -----------------------------------------------

    fn ask_signature(&self) {
        let Some(doc) = self.doc() else { return };
        let Some((_, line, col, path)) = self.before_caret() else {
            return;
        };
        let draft = self.servers.draft(&doc.borrow().text);
        let asked = self.completing.signature_asked.get() + 1;
        self.completing.signature_asked.set(asked);
        let asker = self.id;
        self.servers.ask(
            &path,
            Some(draft),
            move |client, path| client.signature_help(path, line, col),
            move |app, info| {
                if let Some(this) = app.editor().view(asker) {
                    this.signature(asked, line, info);
                }
            },
        );
    }

    /// The server's signature for the call the caret is in — or none, the
    /// caret having left it, which puts the card away.
    pub fn signature(&self, asked: u64, line: u32, info: Option<SignatureInfo>) {
        if asked != self.completing.signature_asked.get() {
            return;
        }
        let Some(info) = info else {
            self.close_signature();
            return;
        };
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let code = window.global::<Code>();
        let label = info.label.as_str();
        let (from, to) = match (info.param_start, info.param_end) {
            (Some(from), Some(to))
                if (to as usize) <= label.len()
                    && label.is_char_boundary(from as usize)
                    && label.is_char_boundary(to as usize) =>
            {
                (from as usize, to as usize)
            }
            _ => (label.len(), label.len()),
        };
        let (caret_x, row) = self.doc().map_or((0.0, line as usize), |doc| {
            let d = doc.borrow();
            let head = d.primary().head;
            let (l, col) = d.lines.position(&d.text, head);
            (
                super::inlays::caret_on(&self.metrics.borrow(), &d, l, col),
                d.row_of(line as usize),
            )
        });
        code.set_signature_line(row as i32);
        code.set_signature_x(caret_x);
        code.set_signature_before(label[..from].into());
        code.set_signature_param(label[from..to].into());
        code.set_signature_after(label[to..].into());
        code.set_signature_open(true);
        self.completing.signature_open.set(true);
    }

    pub(super) fn completing_signature(&self) -> bool {
        self.completing.signature_open.get()
    }

    pub(super) fn close_signature(&self) {
        self.completing
            .signature_asked
            .set(self.completing.signature_asked.get() + 1);
        if self.completing.signature_open.replace(false)
            && let Some(window) = self.window.upgrade()
        {
            window.global::<Code>().set_signature_open(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_the_client_names_has_a_glyph_or_the_plain_dot() {
        for kind in [
            "text",
            "method",
            "function",
            "constructor",
            "field",
            "variable",
            "class",
            "interface",
            "module",
            "property",
            "unit",
            "value",
            "enum",
            "keyword",
            "snippet",
            "color",
            "file",
            "reference",
            "folder",
            "enum member",
            "constant",
            "struct",
            "event",
            "operator",
            "type parameter",
            "other",
        ] {
            let (glyph, colour) = glyph(Some(kind));
            assert!(!glyph.is_empty());
            assert!((0..=10).contains(&colour), "{kind}");
        }
        assert_eq!(glyph(Some("method")).0, glyph(Some("function")).0);
    }
}
