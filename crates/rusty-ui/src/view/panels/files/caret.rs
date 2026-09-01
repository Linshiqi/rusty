//! Where the caret is, in whichever unit the asker counts in.
//!
//! Three of them meet at this boundary: a textarea's selection is a UTF-16
//! index, Rust strings slice by bytes, and everything above — vim, the LSP
//! client, the highlighter — works in Unicode scalars. The arithmetic itself
//! lives in `rusty_lsp::positions`; what is here is the naming, the fixed
//! encoding (the DOM has only one), and the geometry that turns a caret into
//! a pixel or a pixel back into a caret.

use leptos::{html, prelude::*};
use wasm_bindgen::JsCast;

use rusty_lsp::positions::{self, Encoding::Utf16};

use super::*;
use crate::state::AppState;

// ─────────────────────────────────────────────────────────────────────────────
// The DOM boundary
//
// A textarea reports and accepts selections as UTF-16 indexes; everything
// above works in Unicode scalars, and Rust strings slice by bytes. That is the
// same three-way conversion `rusty_lsp::positions` does at the protocol
// boundary, so it is the same code — these are names, not arithmetic. The
// editor carried its own copy of all of it until this, which meant two
// implementations of surrogate-pair clamping and tests on only one.
//
// The encoding is fixed here because the DOM has only one; the protocol side
// negotiates and therefore has to pass it.
// ─────────────────────────────────────────────────────────────────────────────

/// UTF-16 code units in `text` — what a textarea counts selections in.
pub(super) fn utf16_len(text: &str) -> u32 {
    positions::scalar_to_character(text, text.chars().count() as u32, Utf16)
}

/// Read the clipboard and drop it in at the caret. Async because that is the
/// only way a browser hands over the clipboard; a refusal lands nowhere,
/// which is what a blocked paste should do.
pub(super) fn paste_at_caret(state: AppState, area: NodeRef<html::Textarea>) {
    use wasm_bindgen_futures::JsFuture;

    let Some(window) = web_sys::window() else {
        return;
    };
    let promise = window.navigator().clipboard().read_text();
    leptos::task::spawn_local(async move {
        let Ok(value) = JsFuture::from(promise).await else {
            return;
        };
        let Some(text) = value.as_string() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if let Some(element) = area.get_untracked() {
            insert_at_caret(&element, state, &text);
        }
    });
}

/// The editor's selection as (byte range, text), when there is one.
pub(super) fn selection_of(
    area: &web_sys::HtmlTextAreaElement,
    text: &str,
) -> Option<(usize, usize, String)> {
    let from = area.selection_start().ok().flatten()? as usize;
    let to = area.selection_end().ok().flatten()? as usize;
    if to <= from {
        return None;
    }
    let (from, to) = (byte_of_utf16(text, from), byte_of_utf16(text, to));
    Some((from, to, text[from..to].to_string()))
}

/// Where the caret lands after `target` replaces `other` on screen: the end
/// of the region where the two texts differ, in UTF-16 units — which is where
/// the eye is already looking.
pub(super) fn caret_after_restore(target: &str, other: &str) -> u32 {
    let target_bytes = target.as_bytes();
    let other_bytes = other.as_bytes();

    let mut prefix = 0;
    while prefix < target_bytes.len().min(other_bytes.len())
        && target_bytes[prefix] == other_bytes[prefix]
    {
        prefix += 1;
    }
    while prefix > 0 && !target.is_char_boundary(prefix) {
        prefix -= 1;
    }

    let mut suffix = 0;
    while suffix < (target_bytes.len() - prefix).min(other_bytes.len() - prefix)
        && target_bytes[target_bytes.len() - 1 - suffix]
            == other_bytes[other_bytes.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let mut end = target_bytes.len() - suffix;
    while end < target_bytes.len() && !target.is_char_boundary(end) {
        end += 1;
    }
    let end = end.max(prefix);

    utf16_len(&target[..end])
}

/// Scroll the shared scroller so the caret's line is on screen. The textarea
/// cannot do this itself any more — its own scrolling is pinned to zero.
pub(super) fn keep_caret_in_view(
    area: &web_sys::HtmlTextAreaElement,
    state: AppState,
    scroller: NodeRef<html::Div>,
) {
    let Some(outer) = scroller.get_untracked() else {
        return;
    };
    let text = state.editor.draft.get_untracked();
    let Some((line, _)) = caret_line_col(area, &text) else {
        return;
    };
    let lh = LINE_HEIGHT * state.editor.zoom.get_untracked();
    let y = 8.0 + f64::from(line) * lh;
    let view_top = f64::from(outer.scroll_top());
    let view_height = f64::from(outer.client_height());
    if y < view_top + lh {
        outer.set_scroll_top((y - lh * 3.0).max(0.0) as i32);
    } else if y + lh * 2.0 > view_top + view_height {
        outer.set_scroll_top((y + lh * 4.0 - view_height) as i32);
    }
}

/// The caret as (line, scalar column) in `text`.
pub(super) fn caret_line_col(
    area: &web_sys::HtmlTextAreaElement,
    text: &str,
) -> Option<(u32, u32)> {
    let units = area.selection_start().ok().flatten()? as usize;
    let byte = byte_of_utf16(text, units);
    let before = &text[..byte];
    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map(|at| at + 1).unwrap_or(0);
    let col = before[line_start..].chars().count() as u32;
    Some((line, col))
}

/// The caret's byte offset into the draft.
pub(super) fn caret_byte(area: &web_sys::HtmlTextAreaElement, state: AppState) -> usize {
    let units = area.selection_start().ok().flatten().unwrap_or(0) as usize;
    byte_of_utf16(&state.editor.draft.get_untracked(), units)
}

/// selectionStart counts UTF-16 units — it is a JS string index. Treating it
/// as bytes panics on the first CJK comment.
pub(super) fn byte_of_utf16(text: &str, units: usize) -> usize {
    positions::byte_of_character(text, units, Utf16)
}

/// A (line, scalar column) as a UTF-16 offset, for placing the caret.
pub(super) fn utf16_offset_of(text: &str, line: u32, col: u32) -> u32 {
    positions::position_to_character(text, line, col, Utf16)
}

/// Which (line, scalar column) sits under a point in the text column.
///
/// The column is found by measuring, not dividing: a CJK glyph is two cells
/// wide in a monospace font, so `x / ch` drifts one column per ideograph and
/// hover would describe the wrong token on any line with a Chinese comment.
pub(super) fn cell_under(
    text: &str,
    offset_x: f64,
    offset_y: f64,
    zoom: f64,
) -> Option<(u32, u32)> {
    // The 8s are the text column's pl-2 / py-2.
    let line = ((offset_y - 8.0) / (LINE_HEIGHT * zoom)).floor();
    if line < 0.0 {
        return None;
    }
    let line = line as u32;
    let content = text.split('\n').nth(line as usize)?;

    let x = offset_x - 8.0;
    if x < 0.0 {
        return Some((line, 0));
    }
    let mut reached = 0.0;
    for (index, ch) in content.chars().enumerate() {
        let advance = advance_of(ch) * zoom;
        if reached + advance > x {
            return Some((line, index as u32));
        }
        reached += advance;
    }
    // Past the end of the line: the last column, where "what is this?" still
    // usually means the token the line ends with.
    Some((line, content.chars().count() as u32))
}

/// Pixels from the line start to a scalar column, for anchoring the tooltip.
pub(super) fn column_px(text: &str, line: u32, col: u32) -> f64 {
    text.split('\n')
        .nth(line as usize)
        .map(|content| content.chars().take(col as usize).map(advance_of).sum())
        .unwrap_or(0.0)
}

/// One glyph's advance in the editor's font, measured once per character via
/// canvas and cached — measuring is what makes CJK correct, caching is what
/// makes it affordable on every mouse move.
fn advance_of(ch: char) -> f64 {
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static CACHE: RefCell<HashMap<char, f64>> = RefCell::new(HashMap::new());
        static CONTEXT: RefCell<Option<web_sys::CanvasRenderingContext2d>> =
            const { RefCell::new(None) };
    }

    CACHE.with(|cache| {
        if let Some(width) = cache.borrow().get(&ch) {
            return *width;
        }
        let width = CONTEXT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.create_element("canvas").ok())
                    .and_then(|c| c.dyn_into::<web_sys::HtmlCanvasElement>().ok())
                    .and_then(|c| c.get_context("2d").ok().flatten())
                    .and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
                    .inspect(|context| {
                        context.set_font(
                            "12.5px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
                        );
                    });
            }
            slot.as_ref()
                .and_then(|context| context.measure_text(&ch.to_string()).ok())
                .map(|m| m.width())
                // The monospace advance at 12.5px, if the canvas is somehow
                // unavailable; wrong for CJK but never absurd.
                .unwrap_or(7.5)
        });
        cache.borrow_mut().insert(ch, width);
        width
    })
}

/// A scalar index as the UTF-16 unit the textarea counts selections in.
///
/// The whole of `vim` works in Unicode scalars and converts here, exactly as
/// the LSP client converts at its boundary. Skipping this would make every
/// motion past a `中` land in the wrong place.
pub(super) fn units_of_scalar(text: &str, scalars: usize) -> u32 {
    positions::scalar_to_character(text, scalars as u32, Utf16)
}

pub(super) fn scalar_of_units(text: &str, units: usize) -> usize {
    positions::character_to_scalar(text, units as u32, Utf16) as usize
}

/// `zz` `zt` `zb`: put the caret's line where it was asked for.
pub(super) fn centre_view(
    state: AppState,
    scroller: NodeRef<html::Div>,
    text: &str,
    cursor: usize,
    at: crate::vim::View,
) {
    let Some(outer) = scroller.get_untracked() else {
        return;
    };
    let line = text.chars().take(cursor).filter(|c| *c == '\n').count() as f64;
    let lh = LINE_HEIGHT * state.editor.zoom.get_untracked();
    let y = 8.0 + line * lh;
    let height = f64::from(outer.client_height());
    let top = match at {
        crate::vim::View::Middle => y - height / 2.0 + lh / 2.0,
        crate::vim::View::Top => y - lh,
        crate::vim::View::Bottom => y - height + lh * 2.0,
    };
    outer.set_scroll_top(top.max(0.0) as i32);
}

#[cfg(test)]
mod tests {
    use super::utf16_offset_of;

    #[test]
    fn caret_offsets_survive_cjk() {
        let text = "// 中文\nfn main() {}\n";
        // Line 1 col 3: past "fn " — offset counts the CJK line as utf16.
        // "// 中文" = 5 utf16 units, +1 newline.
        assert_eq!(utf16_offset_of(text, 1, 3), 6 + 3);
    }
}
