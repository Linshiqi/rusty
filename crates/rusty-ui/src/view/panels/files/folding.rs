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
    state.editor.draft.set(text.to_string());
    area.set_value(text);
}
