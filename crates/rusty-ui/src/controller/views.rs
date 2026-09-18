//! One file open in both groups: two views of one document.
//!
//! Each group keeps its own copy of what it shows — the draft, its painting,
//! the disk's text — because every component reads its own group's signals,
//! and a group can be showing another file a moment later. So the two copies
//! of one file are kept the same instead: whatever changes one is carried to
//! the other at once, by the group it happened in. The undo history is not
//! copied but shared (`Editor::histories`): it belongs to the document, and a
//! copy per group would let an undo in one view put back a text from before
//! the other view's edits.
//!
//! What stays each view's own: the caret and the selection, the folds, where
//! it is scrolled, Vim's mode, the popups. An edit carried across moves the
//! other view's folds the way the text under them moved at once, and its
//! caret when that view is next used: its textarea is written then and not
//! per keystroke (`Editor::lagging`, [`catch_up`]).

use leptos::prelude::*;

use rusty_edit::Line;

use super::*;
use crate::{
    paint,
    state::{AppState, PaintState, ParkedEditor},
};

/// Patch the painted lines for an edit, without waiting for the re-highlight.
///
/// A line diff against what the paint currently shows: unchanged lines keep
/// their colours, edited ones are swapped for plain text immediately. The
/// debounced pulse recolours them a beat later — the same catch-up every
/// editor's highlighting does, built from a splice instead of a parser. The
/// lines written plain are marked (`crate::paint`), so the repaint brings
/// them back even when the text they hold is what it was.
pub fn echo_edit(state: AppState, new: &str) {
    let old = state.editor.echo_text.get_untracked();
    if old == new {
        return;
    }
    let mut edit = None;
    state
        .editor
        .highlighted
        .update(|lines| edit = Some(paint::echo(lines, &old, new)));
    let Some(edit) = edit else {
        return;
    };
    state
        .editor
        .paint
        .update_value(|paint| paint.stale = paint::stale_after(paint.stale, edit));
    state.editor.painting.update_value(|ask| {
        if let Some(ask) = ask {
            ask.since = paint::stale_after(ask.since, edit);
        }
    });
    state.editor.echo_text.set(new.to_string());
}

/// Whether the other group holds `path`, on screen or parked.
pub fn other_holds(state: AppState, path: &str) -> bool {
    state
        .other()
        .editor
        .tabs
        .with_untracked(|tabs| tabs.iter().any(|tab| tab == path))
}

/// Carry an edit made in this group to the other group's copy of the file:
/// the text, its painting, and where that view's caret and folds now stand.
///
/// Called from `schedule_pulse`, the one hook every edit path goes through,
/// with the draft already the text after the edit.
pub fn share_edit(state: AppState) {
    let Some(path) = state.active_path_now() else {
        return;
    };
    if !other_holds(state, &path) {
        return;
    }
    let other = state.other();
    let new = state.editor.draft.get_untracked();
    if other.active_path_now().as_deref() == Some(path.as_str()) {
        let old = other.editor.draft.get_untracked();
        if old == new {
            return;
        }
        start_lagging(other);
        follow_folds_of(other, &old, &new);
        echo_edit(other, &new);
        other.editor.draft.set(new);
        // What hangs off a line in the other view is about a line that may
        // just have moved under it.
        dismiss_completion(other);
        other.editor.signature.set(None);
        other.editor.hover.set(None);
        other.editor.actions.set(None);
        other.editor.occurrences.set(None);
        caught_up_if_focused(other);
        return;
    }
    other.editor.parked.update(|parked| {
        let Some(entry) = parked.iter_mut().find(|e| e.document.path == path) else {
            return;
        };
        if entry.draft == new {
            return;
        }
        let edit = paint::echo(&mut entry.highlighted, &entry.draft, &new);
        entry.paint.stale = paint::stale_after(entry.paint.stale, edit);
        entry.caret = entry.caret.map(|(line, col)| {
            let (line, col) = paint::follow(&entry.draft, &new, (line as usize, col));
            (line as u32, col)
        });
        follow_folds(&mut entry.folds, edit);
        entry.draft = new;
    });
}

/// Carry a repaint that landed here — `painted` from line `from` of `sent` —
/// to the other group's copy of the file, which has had the same edits and
/// so stands where this one does.
pub fn share_repaint(
    state: AppState,
    path: &str,
    sent: &str,
    from: usize,
    painted: Vec<Line>,
    paint: PaintState,
) {
    if !other_holds(state, path) {
        return;
    }
    let other = state.other();
    if other.active_path_now().as_deref() == Some(path) {
        let now = other.editor.echo_text.get_untracked();
        let mut placed = false;
        other
            .editor
            .highlighted
            .update(|lines| placed = paint::place(lines, sent, &now, from, painted));
        if placed {
            other.editor.paint.set_value(paint);
        }
        return;
    }
    other.editor.parked.update(|parked| {
        if let Some(entry) = parked.iter_mut().find(|e| e.document.path == path)
            && paint::place(&mut entry.highlighted, sent, &entry.draft, from, painted)
        {
            entry.paint = paint;
        }
    });
}

/// Carry a whole new state of the file to the other group's copy: the disk's
/// text after a save or a reload, and the painting that came with it.
pub fn share_document(state: AppState) {
    let Some(path) = state.active_path_now() else {
        return;
    };
    if !other_holds(state, &path) {
        return;
    }
    let Some(document) = state.editor.document.get_untracked() else {
        return;
    };
    let other = state.other();
    let draft = state.editor.draft.get_untracked();
    let highlighted = state.editor.highlighted.get_untracked();
    let paint = state.editor.paint.get_value();
    if other.active_path_now().as_deref() == Some(path.as_str()) {
        let old = other.editor.draft.get_untracked();
        if old != draft {
            start_lagging(other);
            follow_folds_of(other, &old, &draft);
            other.editor.draft.set(draft.clone());
        }
        other.editor.echo_text.set(draft);
        other.editor.highlighted.set(highlighted);
        painted_whole(other, paint.version, paint.stale);
        other.editor.document.set(Some(document));
        caught_up_if_focused(other);
        return;
    }
    other.editor.parked.update(|parked| {
        let Some(entry) = parked.iter_mut().find(|e| e.document.path == path) else {
            return;
        };
        if entry.draft != draft {
            let edit = paint::line_edit(&entry.draft, &draft);
            entry.caret = entry.caret.map(|(line, col)| {
                let (line, col) = paint::follow(&entry.draft, &draft, (line as usize, col));
                (line as u32, col)
            });
            follow_folds(&mut entry.folds, edit);
            entry.draft = draft;
        }
        entry.highlighted = highlighted;
        entry.paint = paint;
        entry.document = document;
    });
}

/// Open, in this group, a file the other group already holds — a second view
/// of the same document rather than a second read of the disk, which would
/// put the disk's text beside the other view's unsaved draft. It opens where
/// the other view is: the same caret, folds and scroll, as VS Code's split
/// does. False when the other group lists the file without having read it (a
/// tab restored and never clicked), for the caller to read it from the disk.
pub fn open_view(state: AppState, path: &str) -> bool {
    let other = state.other();
    let entry = if other.active_path_now().as_deref() == Some(path) {
        let Some(document) = other.editor.document.get_untracked() else {
            return false;
        };
        ParkedEditor {
            document,
            draft: other.editor.draft.get_untracked(),
            highlighted: other.editor.highlighted.get_untracked(),
            paint: other.editor.paint.get_value(),
            caret: caret_position(other),
            folds: other.editor.folds.get_untracked(),
            viewport: viewport_position(other),
        }
    } else {
        let parked = other
            .editor
            .parked
            .with_untracked(|parked| parked.iter().find(|e| e.document.path == path).cloned());
        let Some(entry) = parked else {
            return false;
        };
        entry
    };
    if state.active_path_now().is_some() {
        park_active(state);
    }
    state.editor.tabs.update(|tabs| {
        if !tabs.iter().any(|tab| tab == path) {
            tabs.push(path.to_string());
        }
    });
    state.editor.parked.update(|parked| {
        parked.retain(|e| e.document.path != path);
        parked.push(entry);
    });
    front_parked(state, path)
}

/// The folds of the other view, carried across an edit.
fn follow_folds(folds: &mut rusty_edit::Folded, edit: paint::LineEdit) {
    folds.follow(
        edit.prefix as u32,
        (edit.old - edit.suffix) as u32,
        edit.new as i64 - edit.old as i64,
    );
}

/// Leave a view's textarea behind its draft (`Editor::lagging`). Set once:
/// setting it again would run its binding again, which reads the whole
/// textarea back to leave it as it is.
fn start_lagging(view: AppState) {
    if !view.editor.lagging.get_untracked() {
        view.editor.lagging.set(true);
    }
}

/// Carry a view's folds across an edit made in the other view, when it has
/// any: setting them anyway would redraw its echo for nothing.
fn follow_folds_of(view: AppState, old: &str, new: &str) {
    if view
        .editor
        .folds
        .with_untracked(|folds| folds.regions().is_empty())
    {
        return;
    }
    let edit = paint::line_edit(old, new);
    view.editor.folds.update(|folds| follow_folds(folds, edit));
}

/// The text a view's textarea should hold: its draft, less what is folded.
fn screen_of(view: AppState) -> String {
    view.editor.folds.with_untracked(|folds| {
        view.editor
            .draft
            .with_untracked(|draft| folds.view_text(draft))
    })
}

/// Where UTF-16 offset `units` of `old` is in `new`.
fn follow_units(old: &str, new: &str, units: u32) -> u32 {
    use rusty_lsp::positions::{self, Encoding::Utf16};

    let byte = positions::byte_of_character(old, units as usize, Utf16);
    let moved = paint::follow_byte(old, new, byte);
    new[..moved].encode_utf16().count() as u32
}

/// A view's selection in the text its textarea should hold — mapped across
/// what the other view changed while it lagged, and as the textarea has it
/// otherwise. For reading where a caret is without writing the textarea.
pub(super) fn selection_now(
    view: AppState,
    area: &web_sys::HtmlTextAreaElement,
) -> Option<(u32, u32)> {
    let start = area.selection_start().ok().flatten()?;
    let end = area.selection_end().ok().flatten()?;
    if !view.editor.lagging.get_untracked() {
        return Some((start, end));
    }
    let (old, new) = (area.value(), screen_of(view));
    Some((
        follow_units(&old, &new, start),
        follow_units(&old, &new, end),
    ))
}

/// Write a lagging view's textarea (`Editor::lagging`): the text it should
/// hold, and its selection moved the way the text under it moved — mapped
/// from what the textarea holds, whatever that is, so a second call, or a
/// write that already caught it up, changes nothing. Before anything uses
/// the textarea: the view taking focus or a press anywhere in it (both,
/// in `EditorGroup`) — which a jump into the view does too, before it sets
/// its selection.
pub fn catch_up(view: AppState) {
    if !view.editor.lagging.get_untracked() {
        return;
    }
    if let Some(area) = editor_area(view.group) {
        let (old, new) = (area.value(), screen_of(view));
        if old != new {
            let selection = (area.selection_start(), area.selection_end());
            area.set_value(&new);
            if let (Ok(Some(start)), Ok(Some(end))) = selection {
                let _ = area.set_selection_range(
                    follow_units(&old, &new, start),
                    follow_units(&old, &new, end),
                );
            }
        }
    }
    view.editor.lagging.set(false);
}

/// A view whose textarea has the keyboard is written at once: the next key
/// lands in that textarea, and must land in the text the draft now holds.
/// Only a change that did not come from typing there reaches it — a reload
/// from the disk, a save's re-read.
fn caught_up_if_focused(view: AppState) {
    let focused = editor_area(view.group).is_some_and(|area| {
        web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
            .is_some_and(|active| active == **area)
    });
    if focused {
        catch_up(view);
    }
}
