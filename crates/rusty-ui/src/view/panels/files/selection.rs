//! The caret and the selection, drawn by the editor rather than by the
//! textarea.
//!
//! The textarea still holds both — every key, every read of where the caret
//! is, every edit goes through its selection as before — but its own
//! drawing of them is hidden (`caret-transparent` and `::selection` in
//! `input.css`): it lays out the file without the inlay hints the echo draws
//! inside a line, and would stand a hint's width left of the text after one.
//! These are drawn where `hints.rs` says the text is: the caret two pixels
//! wide, as VS Code's, blinking and restarted by every move, shown while the
//! editor has focus; the selection washed behind the text, shown whether it
//! has focus or not — the find bar's match stays marked while the find bar
//! is typed in. Every cursor of several is drawn the same way (`multi.rs`).

use std::ops::Range;

use leptos::{html, prelude::*};

use super::*;
use crate::{cursors::Cursor, state::AppState};

/// Every cursor to draw: the textarea's selection first, then the others —
/// none of the textarea's while it lags an edit made in the other view of
/// its file, since what it holds is not the text on screen.
fn drawn_cursors(state: AppState, area: NodeRef<html::Textarea>) -> Vec<Cursor> {
    let mut all = Vec::new();
    if let Some(element) = area.get()
        && !state.editor.lagging.get()
    {
        all.push(textarea_cursor(state, &element));
    }
    all.extend(state.editor.cursors.get());
    all
}

/// The washes under every selection, behind the text.
pub(super) fn selections(
    state: AppState,
    path: String,
    area: NodeRef<html::Textarea>,
    window: Memo<Range<u32>>,
    moves: RwSignal<u32>,
) -> impl IntoView {
    let zoom = state.editor.zoom;
    move || {
        moves.track();
        let cursors: Vec<Cursor> = drawn_cursors(state, area)
            .into_iter()
            .filter(|cursor| !cursor.is_caret())
            .collect();
        if cursors.is_empty() {
            return ().into_any();
        }
        let z = zoom.get();
        let height = row_height(z);
        let rows = window.get();
        let space = line_px(" ") * z;
        let mut washes = Vec::new();
        state.editor.draft.with(|draft| {
            let lines: Vec<&str> = draft.split('\n').collect();
            for cursor in &cursors {
                let (first, from) = line_col_of_byte(draft, cursor.start());
                let (last, to) = line_col_of_byte(draft, cursor.end());
                for line in first..=last {
                    let row = row_for(state, line);
                    if !rows.contains(&row) || line_of_row(state, row) != line {
                        continue;
                    }
                    let text = lines.get(line as usize).copied().unwrap_or_default();
                    let hints = hints_on(state, &path, line);
                    let x0 = if line == first {
                        char_left(text, &hints, from, z)
                    } else {
                        PAD_PX
                    };
                    // A line the selection runs on past carries its break,
                    // drawn as a space's width after everything on it.
                    let x1 = if line == last {
                        edge_left(text, &hints, to, z)
                    } else {
                        line_right(text, &hints, z) + space
                    };
                    let top = row_top(state, line, z);
                    washes.push(view! {
                        <div
                            class="pointer-events-none absolute bg-text-selection"
                            style=format!(
                                "left: {x0}px; top: {top}px; width: {}px; height: {height}px",
                                (x1 - x0).max(2.0),
                            )
                        />
                    });
                }
            }
        });
        washes.collect_view().into_any()
    }
}

/// A caret at every cursor, over the text. Placed after the textarea, so
/// it shows only while the textarea has focus, as a caret does
/// (`input.css`); not in Vim's normal and visual modes, which draw their
/// block instead.
pub(super) fn carets(
    state: AppState,
    path: String,
    area: NodeRef<html::Textarea>,
    window: Memo<Range<u32>>,
    moves: RwSignal<u32>,
) -> impl IntoView {
    let zoom = state.editor.zoom;
    move || {
        let turn = moves.get();
        let modal = state.editor.vim_on.get()
            && state
                .editor
                .vim
                .with(|vim| vim.mode != crate::vim::Mode::Insert);
        if modal {
            return ().into_any();
        }
        let cursors = drawn_cursors(state, area);
        let z = zoom.get();
        let height = row_height(z);
        let rows = window.get();
        // Two identical animations, swapped on every move: changing the
        // name is what restarts one, so a caret that moved shows at once.
        let blink = if turn.is_multiple_of(2) {
            "caret-blink-a"
        } else {
            "caret-blink-b"
        };
        let marks = state.editor.draft.with(|draft| {
            cursors
                .iter()
                .filter_map(|cursor| {
                    let (line, col) = line_col_of_byte(draft, cursor.head);
                    let row = row_for(state, line);
                    if !rows.contains(&row) || line_of_row(state, row) != line {
                        return None;
                    }
                    let text = draft.split('\n').nth(line as usize).unwrap_or_default();
                    let hints = hints_on(state, &path, line);
                    let x = caret_left(text, &hints, col, z);
                    let top = row_top(state, line, z);
                    Some(view! {
                        <div
                            class="editor-caret"
                            style=format!(
                                "left: {x}px; top: {top}px; height: {height}px; \
                                 animation-name: {blink}",
                            )
                        />
                    })
                })
                .collect_view()
        });
        view! { <div class="editor-carets">{marks}</div> }.into_any()
    }
}

/// Keep the drawn caret in view horizontally. The browser scrolls to the
/// textarea's own caret, which a hint before it on its line leaves short of
/// the drawn one: past the right edge by up to the hints' width. Only that
/// way — to the left, the textarea's caret is the further of the two, and
/// the browser's own scroll already brings both.
pub(super) fn follow_caret(
    state: AppState,
    area: NodeRef<html::Textarea>,
    scroller: NodeRef<html::Div>,
    moves: RwSignal<u32>,
) {
    Effect::new(move |_| {
        moves.track();
        state.editor.draft.track();
        let (Some(element), Some(outer)) = (area.get_untracked(), scroller.get_untracked()) else {
            return;
        };
        let Some(column) = element.parent_element() else {
            return;
        };
        let column: &web_sys::HtmlElement = wasm_bindgen::JsCast::unchecked_ref(&column);
        let head = textarea_cursor(state, &element).head;
        let zoom = state.editor.zoom.get_untracked();
        let x = state.editor.draft.with_untracked(|draft| {
            let (line, col) = line_col_of_byte(draft, head);
            let text = draft.split('\n').nth(line as usize).unwrap_or_default();
            caret_left(text, &hints_on_now(state, line), col, zoom)
        });
        let at = f64::from(column.offset_left()) + x;
        let (left, width) = (
            f64::from(outer.scroll_left()),
            f64::from(outer.client_width()),
        );
        if at > left + width - 16.0 {
            outer.set_scroll_left((at - width + 48.0) as i32);
        }
    });
}

/// How far the hints before the textarea's caret on its line push the drawn
/// caret along, in pixels — what the textarea is shifted by while an input
/// method composes, since the input method puts its window at the
/// textarea's own caret.
pub(super) fn caret_shift(state: AppState, area: &web_sys::HtmlTextAreaElement) -> f64 {
    let head = textarea_cursor(state, area).head;
    let zoom = state.editor.zoom.get_untracked();
    state.editor.draft.with_untracked(|draft| {
        let (line, col) = line_col_of_byte(draft, head);
        let text = draft.split('\n').nth(line as usize).unwrap_or_default();
        let hints = hints_on_now(state, line);
        HintedLine {
            text,
            hints: &hints,
        }
        .caret_shift_px(col, &advance_of)
            * zoom
    })
}

/// The textarea's selection as a cursor over the document: its head where
/// the selection's moving end is.
pub(super) fn textarea_cursor(state: AppState, area: &web_sys::HtmlTextAreaElement) -> Cursor {
    let (from, to) = doc_selection(area, state);
    let backward = area.selection_direction().ok().flatten().as_deref() == Some("backward");
    if backward {
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
