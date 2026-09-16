//! Feeding keys to the modal state machine, and carrying out what it says.
//!
//! The machine itself is `crate::vim` — pure, DOM-free, and tested there.
//! This is the three things a browser forces on the editor: set `value`, set
//! the selection, `preventDefault`.

use leptos::{ev, html, prelude::*};

use rusty_i18n::t;

use super::*;
use crate::{
    controller,
    state::{AppState, VimCaret},
};

/// Where Vim's cursor is, when the textarea cannot say — `None` means "read
/// the textarea".
///
/// Every key starts from the cursor, and the textarea only holds a
/// selection. In normal mode that is the character under the cursor, so its
/// start *is* the cursor. In visual mode it runs from the anchor through the
/// cursor, so its start is the anchor whenever the cursor is to the right of
/// it: every `l` began again from the anchor, and a selection could not grow
/// past two characters. Visual line mode covers whole lines and keeps no
/// column at all, so `Vjj` stopped at two lines the same way.
///
/// So the cursor Vim last set is trusted for as long as the textarea still
/// shows exactly the selection Vim set, in the same file. Anything else moved
/// it — a click, a find, a paste, undo — and then the textarea's start is
/// where the cursor now is, which is what it always was outside visual mode.
pub(super) fn remembered_cursor(
    last: Option<&VimCaret>,
    path: Option<&str>,
    start: u32,
    end: u32,
    len: usize,
) -> Option<usize> {
    let last = last?;
    (last.path.as_deref() == path && last.start == start && last.end == end)
        .then(|| last.cursor.min(len))
}

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
    let start = area.selection_start().ok().flatten().unwrap_or(0);
    let end = area.selection_end().ok().flatten().unwrap_or(start);
    let path = state.active_path_now();
    let cursor = state
        .editor
        .vim_caret
        .with_value(|last| {
            remembered_cursor(
                last.as_ref(),
                path.as_deref(),
                start,
                end,
                text.chars().count(),
            )
        })
        .unwrap_or_else(|| scalar_of_units(&text, start as usize));

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
    let (start, end) = match step.selection {
        Some((from, to)) => (units_of_scalar(&after, from), units_of_scalar(&after, to)),
        None => {
            let at = units_of_scalar(&after, step.cursor);
            (at, at)
        }
    };
    let _ = area.set_selection_start(Some(start));
    let _ = area.set_selection_end(Some(end));
    // What was set, and the cursor it was set for — the next key's starting
    // point while nothing else moves the selection.
    state.editor.vim_caret.set_value(Some(VimCaret {
        path,
        start,
        end,
        cursor: step.cursor,
    }));

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
                if let Some(path) = state.active_path_now() {
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
                        .update(|vim| vim.rejected = Some(t!("misc.vim-no-back"))),
                    (false, false) => state
                        .editor
                        .vim
                        .update(|vim| vim.rejected = Some(t!("misc.vim-no-forward"))),
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vim::{Key, Mode, Vim};

    /// The textarea, as far as a key sees it: the selection the last step
    /// set, in scalars — these texts are ASCII, so scalars are the units.
    struct Textarea {
        start: u32,
        end: u32,
        last: Option<VimCaret>,
    }

    impl Textarea {
        fn press(&mut self, vim: &mut Vim, key: &str, text: &str) {
            let from_dom = self.start as usize;
            let cursor = remembered_cursor(
                self.last.as_ref(),
                Some("src/lib.rs"),
                self.start,
                self.end,
                text.chars().count(),
            )
            .unwrap_or(from_dom);
            let step = vim.feed(&Key::new(key), text, cursor);
            let (start, end) = step.selection.unwrap_or((step.cursor, step.cursor));
            self.start = start as u32;
            self.end = end as u32;
            self.last = Some(VimCaret {
                path: Some("src/lib.rs".into()),
                start: self.start,
                end: self.end,
                cursor: step.cursor,
            });
        }
    }

    /// The report: `v` then `l` over and over grows the selection one
    /// character a press, through the one under the cursor, as Vim's does —
    /// where reading the textarea's start stopped it at two.
    #[test]
    fn a_visual_selection_grows_rightwards_one_press_at_a_time() {
        let text = "abcdef";
        let mut vim = Vim::default();
        let mut area = Textarea {
            start: 0,
            end: 1,
            last: None,
        };
        area.press(&mut vim, "v", text);
        assert_eq!(vim.mode, Mode::Visual);
        for (presses, end) in [(1, 2), (2, 3), (3, 4)] {
            area.press(&mut vim, "l", text);
            assert_eq!(
                (area.start, area.end),
                (0, end),
                "after {presses} l the selection runs from the anchor through the cursor",
            );
        }
        area.press(&mut vim, "h", text);
        assert_eq!((area.start, area.end), (0, 3), "and shrinks back");
    }

    /// `V` then `j` twice is three lines, not two.
    #[test]
    fn a_visual_line_selection_grows_downwards() {
        let text = "one\ntwo\nthree\nfour\n";
        let mut vim = Vim::default();
        let mut area = Textarea {
            start: 0,
            end: 1,
            last: None,
        };
        area.press(&mut vim, "V", text);
        area.press(&mut vim, "j", text);
        area.press(&mut vim, "j", text);
        assert_eq!(vim.mode, Mode::VisualLine);
        assert_eq!(
            &text[area.start as usize..area.end as usize],
            "one\ntwo\nthree\n"
        );
    }

    /// Anything that moves the selection other than Vim — a click, a find, a
    /// paste — is where the cursor now is; and another file's cursor is never
    /// borrowed, whatever its selection looks like.
    #[test]
    fn a_selection_vim_did_not_set_is_read_from_the_textarea() {
        let last = VimCaret {
            path: Some("src/lib.rs".into()),
            start: 0,
            end: 4,
            cursor: 3,
        };
        assert_eq!(
            remembered_cursor(Some(&last), Some("src/lib.rs"), 0, 4, 10),
            Some(3)
        );
        assert_eq!(
            remembered_cursor(Some(&last), Some("src/lib.rs"), 7, 8, 10),
            None,
            "a click moved it"
        );
        assert_eq!(
            remembered_cursor(Some(&last), Some("src/main.rs"), 0, 4, 10),
            None,
            "same selection, another file"
        );
        assert_eq!(remembered_cursor(None, Some("src/lib.rs"), 0, 4, 10), None);
        assert_eq!(
            remembered_cursor(Some(&last), Some("src/lib.rs"), 0, 4, 2),
            Some(2),
            "a buffer that shrank under it clamps the cursor"
        );
    }
}
