//! The editor's side of several cursors (`crate::cursors`): the keys that
//! add them and move them, and the edits made at every one of them. They
//! are drawn with the textarea's own (`selection.rs`).
//!
//! The textarea holds one selection, so it is the first cursor and the
//! others are this editor's to keep, draw and edit at. A key that edits is
//! taken before the browser acts on it and applied at every cursor at once;
//! with one cursor nothing here does anything, and the editor is as it was.
//!
//! Folds open when a second cursor arrives: every cursor is a document
//! position, and a textarea holding the folded screen would need each one
//! mapped across the folds for every key.
//!
//! An input method composes in the textarea itself, which cannot be written
//! to mid-composition, so composing drops the other cursors and goes on with
//! the first — the one gap against VS Code, said here rather than found.

use leptos::{ev, html, prelude::*};
use web_sys::HtmlTextAreaElement;

use super::*;
use crate::{
    controller,
    cursors::{self, Cursor, Motion},
    state::AppState,
};

/// Every cursor: the textarea's selection first, then the others.
fn all_cursors(state: AppState, area: &HtmlTextAreaElement) -> Vec<Cursor> {
    let mut all = vec![textarea_cursor(state, area)];
    all.extend(state.editor.cursors.get_untracked());
    all
}

/// UTF-16 units before byte `at` of `text`: where a textarea puts it.
fn units(text: &str, at: usize) -> u32 {
    text[..at.min(text.len())].encode_utf16().count() as u32
}

/// Put the cursors on screen — the first as the textarea's selection, the
/// rest drawn — after writing `text` everywhere the edit is, when there is
/// one.
fn place(
    state: AppState,
    area: &HtmlTextAreaElement,
    all: Vec<Cursor>,
    text: Option<String>,
    scroller: NodeRef<html::Div>,
) {
    if let Some(text) = text {
        record_edit(state);
        echo_edit(state, &text);
        state.editor.draft.set(text.clone());
        area.set_value(&text);
        controller::schedule_pulse(state);
    }
    let Some(first) = all.first().copied() else {
        return;
    };
    let draft = state.editor.draft.get_untracked();
    let (start, end) = (units(&draft, first.start()), units(&draft, first.end()));
    let direction = if first.head < first.anchor {
        "backward"
    } else {
        "forward"
    };
    let _ = area.set_selection_range_with_direction(start, end, direction);
    state.editor.cursors.set(all[1..].to_vec());
    keep_caret_in_view(area, state, scroller);
}

/// Begin, or go on, with several cursors: folds open first, since every
/// cursor is a document position and the textarea must hold the document.
fn start(
    state: AppState,
    area: &HtmlTextAreaElement,
    all: Vec<Cursor>,
    scroller: NodeRef<html::Div>,
) {
    if state
        .editor
        .folds
        .with_untracked(|folds| !folds.regions().is_empty())
    {
        state.editor.folds.set(rusty_edit::Folded::default());
        area.set_value(&state.editor.draft.get_untracked());
    }
    place(state, area, cursors::merged(&all), None, scroller);
}

/// Back to one cursor: the textarea's own.
fn collapse(state: AppState) {
    if state
        .editor
        .cursors
        .with_untracked(|extra| !extra.is_empty())
    {
        state.editor.cursors.set(Vec::new());
    }
}

/// A key, before the editor's own handling of it. True when it was taken
/// here — or is a character to be typed at every cursor, which the rules for
/// one cursor (bracket pairs, completion) must not see either.
pub(super) fn multi_key(
    state: AppState,
    area: &HtmlTextAreaElement,
    event: &ev::KeyboardEvent,
    scroller: NodeRef<html::Div>,
) -> bool {
    // Vim's keys are Vim's; a read-only file takes no edits anywhere.
    if state.editor.vim_on.get_untracked()
        || state
            .editor
            .document
            .with_untracked(|d| d.as_ref().is_none_or(|d| d.read_only))
    {
        return false;
    }
    let key = event.key();
    let ctrl = event.ctrl_key() || event.meta_key();
    let (shift, alt) = (event.shift_key(), event.alt_key());
    let draft = state.editor.draft.get_untracked();

    // The keys that add cursors, VS Code's, from one cursor as from several.
    if ctrl && !alt && !shift && key.eq_ignore_ascii_case("d") {
        event.prevent_default();
        let mut all = all_cursors(state, area);
        let last = all.len() - 1;
        if all[last].is_caret() {
            // The first press takes the word the cursor is in.
            if let Some((start, end)) = cursors::word_at(&draft, all[last].head) {
                all[last] = Cursor {
                    anchor: start,
                    head: end,
                };
            }
        } else if let Some(next) = cursors::next_match(&draft, &all) {
            all.push(next);
        }
        start(state, area, all, scroller);
        return true;
    }
    if ctrl && shift && !alt && key.eq_ignore_ascii_case("l") {
        event.prevent_default();
        let mut first = all_cursors(state, area)[0];
        if first.is_caret()
            && let Some((start, end)) = cursors::word_at(&draft, first.head)
        {
            first = Cursor {
                anchor: start,
                head: end,
            };
        }
        start(state, area, cursors::every_match(&draft, first), scroller);
        return true;
    }
    if ctrl && alt && (key == "ArrowUp" || key == "ArrowDown") {
        event.prevent_default();
        let mut all = all_cursors(state, area);
        if let Some(added) = cursors::beside_vertically(&draft, &all, key == "ArrowUp") {
            all.push(added);
        }
        start(state, area, all, scroller);
        return true;
    }

    if state.editor.cursors.with_untracked(Vec::is_empty) {
        return false;
    }
    let all = all_cursors(state, area);
    let motion = match key.as_str() {
        "ArrowLeft" => Some(if ctrl { Motion::WordLeft } else { Motion::Left }),
        "ArrowRight" => Some(if ctrl {
            Motion::WordRight
        } else {
            Motion::Right
        }),
        "ArrowUp" if !ctrl => Some(Motion::Up),
        "ArrowDown" if !ctrl => Some(Motion::Down),
        "Home" if !ctrl => Some(Motion::Home),
        "End" if !ctrl => Some(Motion::End),
        _ => None,
    };
    if let Some(motion) = motion
        && !alt
    {
        event.prevent_default();
        place(
            state,
            area,
            cursors::moved(&draft, &all, motion, shift),
            None,
            scroller,
        );
        return true;
    }
    let edited = match key.as_str() {
        "Escape" => {
            event.prevent_default();
            collapse(state);
            return true;
        }
        "Enter" if !ctrl && !alt => Some(cursors::broken(&draft, &all)),
        "Tab" if !ctrl && !alt => Some(cursors::typed(&draft, &all, "    ")),
        "Backspace" if !alt => Some(cursors::erased(&draft, &all, false, ctrl)),
        "Delete" if !alt => Some(cursors::erased(&draft, &all, true, ctrl)),
        _ if ctrl && !alt => match key.to_ascii_lowercase().as_str() {
            "c" => {
                event.prevent_default();
                crate::view::components::copy_to_clipboard(&cursors::copied(&draft, &all));
                return true;
            }
            "x" => {
                crate::view::components::copy_to_clipboard(&cursors::copied(&draft, &all));
                Some(cursors::cut(&draft, &all))
            }
            // Undo, redo and select-all act on one cursor, as they do in VS
            // Code after the others collapse.
            "z" | "y" | "a" => {
                collapse(state);
                return false;
            }
            _ => return false,
        },
        // A character, typed at every cursor — not through the textarea,
        // which would put it at the first alone.
        _ if !ctrl && !alt && key.chars().count() == 1 => Some(cursors::typed(&draft, &all, &key)),
        _ => None,
    };
    match edited {
        Some((text, after)) => {
            event.prevent_default();
            place(state, area, after, Some(text), scroller);
            true
        }
        None => false,
    }
}

/// A paste with several cursors: a line each when the clipboard has one per
/// cursor, all of it at each otherwise. True when it was taken here.
pub(super) fn multi_paste(
    state: AppState,
    area: &HtmlTextAreaElement,
    clip: &str,
    scroller: NodeRef<html::Div>,
) -> bool {
    if state.editor.cursors.with_untracked(Vec::is_empty) {
        return false;
    }
    let draft = state.editor.draft.get_untracked();
    let (text, after) = cursors::pasted(&draft, &all_cursors(state, area), clip);
    place(state, area, after, Some(text), scroller);
    true
}

/// A press in the text: Alt adds a cursor where it lands, as in VS Code; a
/// plain press is back to one cursor, the one it puts down. True when it was
/// taken here.
pub(super) fn multi_click(
    state: AppState,
    area: &HtmlTextAreaElement,
    event: &ev::MouseEvent,
    scroller: NodeRef<html::Div>,
) -> bool {
    if !event.alt_key()
        || event.ctrl_key()
        || event.meta_key()
        || state.editor.vim_on.get_untracked()
    {
        collapse(state);
        return false;
    }
    event.prevent_default();
    let (x, y) = point_in_column(
        area,
        (f64::from(event.client_x()), f64::from(event.client_y())),
    );
    let (row, col) = caret_at_point(state, x, y);
    let line = line_of_row(state, row);
    let draft = state.editor.draft.get_untracked();
    let Some(at) = byte_at(&draft, line, col) else {
        return true;
    };
    let _ = area.focus();
    let mut all = all_cursors(state, area);
    all.push(Cursor::caret(at));
    start(state, area, all, scroller);
    true
}

/// The byte a line and a scalar column name, clamped to the line's end.
fn byte_at(text: &str, line: u32, col: u32) -> Option<usize> {
    let start: usize = text
        .split('\n')
        .take(line as usize)
        .map(|line| line.len() + 1)
        .sum();
    let content = text.get(start..)?.split('\n').next()?;
    Some(
        start
            + content
                .char_indices()
                .nth(col as usize)
                .map_or(content.len(), |(i, _)| i),
    )
}

/// Composing drops the other cursors: see the module's head.
pub(super) fn multi_compose(state: AppState) {
    collapse(state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_and_a_column_name_a_byte() {
        let text = "ab\n中x\n";
        assert_eq!(byte_at(text, 0, 1), Some(1));
        assert_eq!(byte_at(text, 1, 1), Some(6), "after the three bytes of 中");
        assert_eq!(byte_at(text, 1, 9), Some(7), "clamped to the line's end");
        assert_eq!(byte_at(text, 2, 0), Some(8));
    }

    #[test]
    fn units_are_counted_the_way_a_textarea_counts() {
        assert_eq!(units("a中b", 4), 2);
        assert_eq!(units("a😀b", 5), 3, "an emoji is two units");
    }
}
