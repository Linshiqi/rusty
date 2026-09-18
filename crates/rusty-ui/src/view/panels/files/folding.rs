//! The editor's side of code folding: the two conversions, in one place.
//!
//! [`rusty_edit::fold`] holds the arithmetic and its tests. This is the thin
//! layer that reads them off `AppState`, and it exists so that no view module
//! computes a screen position from a document line by hand.
//!
//! **The invariant that makes this safe to add to a working editor:** with
//! nothing folded, [`screen`] returns the draft unchanged, [`row_for`] is the
//! identity, and every call site behaves exactly as it did before. A site
//! that was missed is therefore only wrong while something is collapsed — a
//! misplaced overlay — and never a wrong write, because the one path that
//! writes goes through [`rusty_edit::fold::splice`].

use leptos::prelude::*;

use rusty_edit::fold::{Folded, Region, regions};

use crate::state::AppState;

/// What the textarea holds: the draft with every folded region removed.
pub(super) fn screen(state: AppState) -> String {
    state
        .editor
        .folds
        .with_untracked(|folds| folds.view_text(&state.editor.draft.get_untracked()))
}

/// The same, tracked — for `prop:value`, which must re-run when either moves.
pub(super) fn screen_tracked(state: AppState) -> String {
    state
        .editor
        .folds
        .with(|folds| folds.view_text(&state.editor.draft.get()))
}

/// The screen row a document line is drawn on, tracked.
///
/// **Every overlay that positions itself by line goes through here.** A
/// squiggle drawn at its document line while the code above it is folded
/// lands on somebody else's line. A line inside a collapsed region answers
/// with the collapsed header — the row that stands for it — so an error in a
/// folded function marks the fold rather than disappearing.
///
/// With nothing folded this is the identity, which is what makes it safe to
/// put in front of every existing call site.
pub(super) fn row_for(state: AppState, line: u32) -> u32 {
    state.editor.folds.with(|f| f.row_for(line))
}

/// The two layers' inner padding, in pixels — `py-2 pl-2` on both. Every
/// overlay anchors from here; a fifth copy of the literal `8.0` was how the
/// cards and the caret could come to disagree about where a line starts.
pub(super) const PAD_PX: f64 = 8.0;

/// The top edge of the screen row a document line is drawn on, in pixels
/// from the top of the text column, at this zoom.
pub(super) fn row_top(state: AppState, line: u32, zoom: f64) -> f64 {
    PAD_PX + f64::from(row_for(state, line)) * super::row_height(zoom)
}

/// The left edge of a scalar column on a line, in pixels, at this zoom.
pub(super) fn col_left(text: &str, line: u32, col: u32, zoom: f64) -> f64 {
    PAD_PX + super::column_px(text, line, col) * zoom
}

/// Where a card anchored to `line` sits: hanging from the row above it when
/// `above`, or starting just under the row otherwise. Returns the `style`
/// fragment, because the flip is a `translateY` and not a different `top`.
/// Four popups each had their own copy of these two formulas.
pub(super) fn card_place(state: AppState, line: u32, zoom: f64, above: bool) -> String {
    if above {
        let y = row_top(state, line, zoom) - 4.0;
        format!("top: {y}px; transform: translateY(-100%)")
    } else {
        let y = row_top(state, line, zoom) + super::row_height(zoom) + 2.0;
        format!("top: {y}px")
    }
}

/// A document byte offset for a UTF-16 offset into the screen text — where a
/// textarea selection is in the draft.
///
/// Rows map through the fold table and columns are the same on both sides,
/// so the conversion is exact; with nothing folded it is the identity.
/// Pure over the three texts it relates, so both directions are tested
/// against real folds rather than read off signals.
pub(super) fn doc_byte_at(screen: &str, draft: &str, folds: &Folded, units: usize) -> usize {
    let byte = super::byte_of_utf16(screen, units);
    let before = &screen[..byte.min(screen.len())];
    let row = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map_or(0, |at| at + 1);
    let col = before[line_start..].chars().count();
    let line = folds.doc_of_view(row) as usize;
    let mut offset = 0;
    for (index, text) in draft.split('\n').enumerate() {
        if index == line {
            return offset + text.chars().take(col).map(char::len_utf8).sum::<usize>();
        }
        offset += text.len() + 1;
    }
    draft.len()
}

/// The other way: where a document byte offset is in the screen text, as
/// the UTF-16 offset a textarea selection takes — or `None` when a collapsed
/// region hides its line, since nothing on screen stands for it.
pub(super) fn screen_units_at(
    screen: &str,
    draft: &str,
    folds: &Folded,
    byte: usize,
) -> Option<u32> {
    let mut byte = byte.min(draft.len());
    while !draft.is_char_boundary(byte) {
        byte -= 1;
    }
    let before = &draft[..byte];
    let line = before.matches('\n').count() as u32;
    let col = before[before.rfind('\n').map_or(0, |at| at + 1)..]
        .chars()
        .count();
    let row = folds.view_of_doc(line)?;
    let mut start = 0;
    for _ in 0..row {
        start += screen[start..].find('\n')? + 1;
    }
    let end = screen[start..]
        .find('\n')
        .map_or(screen.len(), |at| start + at);
    let within: usize = screen[start..end]
        .chars()
        .take(col)
        .map(char::len_utf8)
        .sum();
    Some(super::utf16_len(&screen[..start + within]))
}

/// The document line a screen row shows. The inverse of [`row_for`], for
/// anything that starts from a pixel — a click, a hover.
pub(super) fn line_of_row(state: AppState, row: u32) -> u32 {
    state.editor.folds.with_untracked(|f| f.doc_of_view(row))
}

/// The region that can be collapsed at this document line, if any.
pub(super) fn region_at(state: AppState, line: u32) -> Option<Region> {
    rusty_edit::fold::region_at(&state.editor.draft.get_untracked(), line)
}

/// Fold or unfold the region headed by `line`.
pub(super) fn toggle_fold(state: AppState, line: u32) {
    let Some(region) = region_at(state, line) else {
        return;
    };
    state.editor.folds.update(|folds| folds.toggle(region));
}

/// Collapse every top-level region — the "give me the shape of this file"
/// gesture. Nested regions are absorbed, so unfolding one gives back a whole
/// item rather than a half-collapsed one.
pub(super) fn fold_all(state: AppState) {
    let text = state.editor.draft.get_untracked();
    let all = regions(&text);
    let outermost: Vec<Region> = all
        .iter()
        .copied()
        .filter(|r| {
            !all.iter()
                .any(|other| other.header < r.header && other.last >= r.last)
        })
        .collect();
    state.editor.folds.update(|folds| {
        *folds = Folded::default();
        for region in outermost {
            folds.fold(region);
        }
    });
}

pub(super) fn unfold_all(state: AppState) {
    state.editor.folds.update(Folded::clear);
}

/// Put a whole new document into the buffer and show it.
///
/// **Every path that rewrites the buffer wholesale goes through here** — cut,
/// paste, undo, redo, comment toggle, accepting a completion, replace-all, a
/// Vim operator, a format. It expands every fold first, and that is the
/// design rather than a shortcut: all of those compute a new *document* and
/// hand it straight to `set_value`, while a folded textarea holds the screen
/// text instead. Splicing each of them separately would be a dozen chances to
/// get it wrong, and getting it wrong writes the wrong bytes to disk;
/// unfolding first makes the two texts the same text again.
///
/// Only the keystroke path stays fold-aware, because it is the one that can:
/// an input event carries the screen after the edit, which
/// [`rusty_edit::fold::splice`] can map back exactly.
pub(super) fn set_buffer(state: AppState, area: &web_sys::HtmlTextAreaElement, text: &str) {
    unfold_all(state);
    // A text written wholesale leaves nowhere the other cursors stood.
    if state
        .editor
        .cursors
        .with_untracked(|extra| !extra.is_empty())
    {
        state.editor.cursors.set(Vec::new());
    }
    state.editor.draft.set(text.to_string());
    area.set_value(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A function body collapsed: lines 1 and 2 hidden, the brace and what
    /// follows moved up two rows, and a `中` on the line after so a column
    /// counted in bytes or in UTF-16 cannot pass for one counted right.
    fn folded() -> (String, Folded, String) {
        let draft = "fn a() {\n    one\n    two\n}\n中 next\n".to_string();
        let mut folds = Folded::default();
        folds.fold(Region { header: 0, last: 2 });
        let screen = folds.view_text(&draft);
        assert_eq!(screen, "fn a() {\n}\n中 next\n");
        (draft, folds, screen)
    }

    #[test]
    fn with_nothing_folded_both_directions_are_the_identity() {
        let draft = "// 中文\nfn main() {}\n";
        let folds = Folded::default();
        for byte in [0, 3, 10, draft.len()] {
            let units = screen_units_at(draft, draft, &folds, byte).unwrap();
            assert_eq!(units, super::super::utf16_len(&draft[..byte]));
            assert_eq!(doc_byte_at(draft, draft, &folds, units as usize), byte);
        }
    }

    /// Below a fold, a screen position is further down the document, and
    /// back again — the round trip Vim's cursor makes on every key.
    #[test]
    fn below_a_fold_positions_map_across_the_hidden_lines_both_ways() {
        let (draft, folds, screen) = folded();
        let next_in_draft = draft.find("next").unwrap();
        let next_on_screen = super::super::utf16_len(&screen[..screen.find("next").unwrap()]);
        assert_eq!(
            doc_byte_at(&screen, &draft, &folds, next_on_screen as usize),
            next_in_draft
        );
        assert_eq!(
            screen_units_at(&screen, &draft, &folds, next_in_draft),
            Some(next_on_screen)
        );
        let brace = draft.find('}').unwrap();
        assert_eq!(screen_units_at(&screen, &draft, &folds, brace), Some(9));
    }

    #[test]
    fn a_hidden_line_has_no_place_on_screen() {
        let (draft, folds, screen) = folded();
        let hidden = draft.find("two").unwrap();
        assert_eq!(screen_units_at(&screen, &draft, &folds, hidden), None);
        assert_eq!(
            screen_units_at(&screen, &draft, &folds, 0),
            Some(0),
            "the header shows"
        );
    }
}
