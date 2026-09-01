//! Feeding keys to the modal state machine, and carrying out what it says.
//!
//! The machine itself is `crate::vim` — pure, DOM-free, and tested there.
//! This is the three things a browser forces on the editor: set `value`, set
//! the selection, `preventDefault`.

use leptos::{ev, html, prelude::*};

use super::*;
use crate::{controller, state::AppState};

/// Feed one key to the modal state machine, and carry out what it says.
///
/// Returns true when Vim took the key, which is the editor's cue to call
/// `preventDefault` *and* `stopPropagation` — the second is what keeps the
/// global bindings from also acting on a key Vim already used. Returning
/// false is the path that leaves Ctrl+S, the palette and the clipboard
/// exactly as they were.
pub(super) fn vim_key(
    state: AppState,
    area: &web_sys::HtmlTextAreaElement,
    scroller: NodeRef<html::Div>,
    event: &ev::KeyboardEvent,
) -> bool {
    use crate::vim::{Ask, Key};

    let text = state.editor.draft.get_untracked();
    let units = area.selection_start().ok().flatten().unwrap_or(0) as usize;
    let cursor = scalar_of_units(&text, units);

    let key = Key {
        key: event.key(),
        ctrl: event.ctrl_key() || event.meta_key(),
        alt: event.alt_key(),
    };
    let step = state
        .editor
        .vim
        .try_update(|vim| vim.feed(&key, &text, cursor));
    let Some(step) = step else { return false };
    if !step.handled {
        return false;
    }

    // The undo unit closes *before* the change, so the snapshot taken is the
    // buffer as it was — Vim undoes a whole command at once, and the editor's
    // own coalescing is by time, which would split `ciwfoo<Esc>` into pieces.
    if step.seal {
        record_edit(state);
        state
            .editor
            .history
            .update(|history| history.last_push = 0.0);
    }

    let after = if let Some(next) = step.text.clone() {
        echo_edit(state, &next);
        set_buffer(state, area, &next);
        controller::schedule_pulse(state);
        next
    } else {
        text
    };

    // Visual mode selects a range; normal mode's cursor is the caret, drawn
    // as a block by `caret-shape`. Both take this one path rather than two
    // that can disagree about where the cursor is.
    match step.selection {
        Some((from, to)) => {
            let _ = area.set_selection_start(Some(units_of_scalar(&after, from)));
            let _ = area.set_selection_end(Some(units_of_scalar(&after, to)));
        }
        None => {
            let at = units_of_scalar(&after, step.cursor);
            let _ = area.set_selection_start(Some(at));
            let _ = area.set_selection_end(Some(at));
        }
    }

    // Follow the caret, always. The typing path has done this from the start;
    // this one only did it for `Ctrl+D`, so every *other* way of leaving the
    // visible region moved the cursor somewhere the reader could not see —
    // `G`, `gg`, `}`, `%`, `n`, `*`, and `j` at the bottom edge. One call
    // covers all of them, which is why it belongs here rather than in each.
    //
    // Before the asks below: `zz` and its friends reposition deliberately,
    // and must have the last word.
    keep_caret_in_view(area, state, scroller);

    if let Some(ask) = step.ask {
        match ask {
            // The editor's own history, not a second undo stack that would
            // disagree with Ctrl+Z about what the last change was.
            Ask::Undo => apply_history(area, state, scroller, true),
            Ask::Redo => apply_history(area, state, scroller, false),
            Ask::Save => controller::save_file(state),
            Ask::Close | Ask::SaveAndClose => {
                if ask == Ask::SaveAndClose {
                    controller::save_file(state);
                }
                if let Some(path) = state
                    .editor
                    .document
                    .with_untracked(|d| d.as_ref().map(|d| d.path.clone()))
                {
                    controller::close_tab(state, path);
                }
            }
            // The find bar that already exists, rather than a second search
            // that would drift from it.
            Ask::Search { .. } => state.find.open.set(true),
            Ask::SearchNext | Ask::SearchPrevious => state.find.open.set(true),
            // `:s/…` opens the same bar with its replace half showing. The
            // pattern is not parsed here: this editor's replace is literal
            // and Vim's is a regex dialect, and quietly accepting `\(` as
            // either one would be a substitution nobody asked for.
            Ask::Replace => {
                state.find.open.set(true);
                state.find.replace_open.set(true);
            }
            // `*` and `#`: the word under the caret, into the find bar.
            Ask::SearchWord { .. } => {
                if let Some(word) = word_at(&after, step.cursor) {
                    state.find.query.set(word);
                    state.find.open.set(true);
                }
            }
            Ask::Centre { at } => centre_view(state, scroller, &after, step.cursor, at),
            // Comment syntax belongs to the language, which the document
            // knows and the state machine deliberately does not.
            Ask::Comment { from, to } => {
                let out = toggle_comments(state, &after, from, to);
                if out != after {
                    echo_edit(state, &out);
                    set_buffer(state, area, &out);
                    let at = units_of_scalar(&out, from.min(out.chars().count()));
                    let _ = area.set_selection_start(Some(at));
                    let _ = area.set_selection_end(Some(at));
                    controller::schedule_pulse(state);
                }
            }
            // Half a screen, which only the editor knows the height of — the
            // reason this is a request rather than a motion. The cursor moves
            // with the view, because Vim's Ctrl+D moves both and a scroll
            // that left the cursor behind would put the next `j` off-screen.
            Ask::Scroll { down } => {
                let lines = scroller
                    .get_untracked()
                    .map(|element| {
                        let height = f64::from(element.client_height());
                        let line = (row_height(state.editor.zoom.get_untracked())).max(1.0);
                        ((height / line) / 2.0).round().max(1.0) as usize
                    })
                    .unwrap_or(10);
                let motion = if down {
                    crate::vim::motion::Motion::Down
                } else {
                    crate::vim::motion::Motion::Up
                };
                if let Some(span) =
                    crate::vim::motion::apply(motion, &after, step.cursor, lines, &None)
                {
                    let at = units_of_scalar(&after, span.cursor);
                    let _ = area.set_selection_start(Some(at));
                    let _ = area.set_selection_end(Some(
                        at + u32::from(after.chars().nth(span.cursor).is_some()),
                    ));
                    keep_caret_in_view(area, state, scroller);
                }
            }
            // The editor's own navigation history, shared with the menu —
            // not a second list that would disagree with it on the first
            // jump. Vim's Ctrl+O and Alt+Left are the same walk.
            Ask::Jump { back } => {
                // A jump key with nowhere to go says so, the same way an
                // unknown command does. Silence here is indistinguishable
                // from a key that is not wired up at all — which is exactly
                // what it was until now.
                let possible = state.editor.nav.with_untracked(|nav| {
                    if back {
                        nav.can_go_back()
                    } else {
                        nav.can_go_forward()
                    }
                });
                match (possible, back) {
                    (true, true) => controller::nav_back(state),
                    (true, false) => controller::nav_forward(state),
                    (false, true) => state
                        .editor
                        .vim
                        .update(|vim| vim.rejected = Some("nowhere further back".into())),
                    (false, false) => state
                        .editor
                        .vim
                        .update(|vim| vim.rejected = Some("nowhere further forward".into())),
                }
            }
        }
    }
    true
}
