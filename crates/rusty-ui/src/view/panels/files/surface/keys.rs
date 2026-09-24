//! The keyboard, as the textarea takes it. A method on `Pane`, so the
//! textarea's handler is one line.

use super::*;

impl Pane {
    /// A key in the textarea: completion's, the popups', Vim's, the pairs' and
    /// the editor's own, in that order — see the notes on each.
    pub(super) fn on_keydown(self, event: ev::KeyboardEvent) {
        let Self {
            state,
            area,
            scroller,
            read_only,
            is_rust,
            picked,
            picked_action,
            ..
        } = self;
        let path = self.path.get_value();
        // While an IME is composing, Enter confirms the
        // candidate and Tab moves through them. Stealing
        // either would break Chinese input entirely.
        if event.is_composing() {
            return;
        }
        // Several cursors take their keys first, and a
        // character typed at all of them skips the rules
        // written for one (`multi.rs`).
        if let Some(element) = area.get_untracked()
            && multi_key(state, &element, &event, scroller)
        {
            event.stop_propagation();
            return;
        }
        // Modal editing gets the key first, and takes only
        // what it wants. Everything it passes through —
        // every chord but five, and the whole of insert
        // mode — carries on to the handling below and
        // then to the global bindings, which is why
        // Ctrl+S, Ctrl+K and the clipboard are unchanged
        // by turning Vim on.
        //
        // `stop_propagation` on the taken ones is the
        // half that matters: without it a `d` in normal
        // mode would also reach the window listener.
        // The popup's Escape comes before Vim's, as the
        // suggest widget's does in VS Code: the first
        // press closes the list, the second leaves insert
        // mode. Vim taking it first left the popup drawn,
        // with Tab still accepting into it. An ask still
        // on its way is forgotten either way.
        if event.key() == "Escape" {
            let showing = state.editor.completion.with_untracked(|popup| {
                popup.as_ref().is_some_and(|popup| {
                    !visible_items(popup, &state.editor.draft.get_untracked()).is_empty()
                })
            });
            controller::dismiss_completion(state);
            if showing {
                event.prevent_default();
                event.stop_propagation();
                return;
            }
        }
        if state.editor.vim_on.get_untracked()
            && let Some(element) = area.get_untracked()
            && vim_key(state, &element, scroller, &event)
        {
            event.prevent_default();
            event.stop_propagation();
            return;
        }
        // Copy and cut take the selection when there is one
        // and the whole line when there is not, as VS
        // Code's do — in Vim's modes as well, since normal
        // mode's cursor is drawn rather than selected.
        // Paste is the paste event's, below — Vim's
        // read-only textarea only has to be let in first.
        if (event.ctrl_key() || event.meta_key()) && !event.alt_key() && !event.shift_key() {
            match event.key().to_ascii_lowercase().as_str() {
                key @ ("c" | "x") => {
                    if let Some(element) = area.get_untracked()
                        && clipboard_key(state, &element, key == "x", read_only)
                    {
                        event.prevent_default();
                        return;
                    }
                }
                "v" => open_for_paste(state, area, read_only),
                _ => {}
            }
        }
        // The actions popup owns its keys while it is up.
        if state.editor.actions.with_untracked(Option::is_some) {
            match event.key().as_str() {
                "ArrowDown" => {
                    event.prevent_default();
                    picked_action.update(|i| *i += 1);
                    return;
                }
                "ArrowUp" => {
                    event.prevent_default();
                    picked_action.update(|i| *i = i.saturating_sub(1));
                    return;
                }
                "Enter" => {
                    event.prevent_default();
                    if let Some(element) = area.get_untracked() {
                        apply_action(state, &element, picked_action.get_untracked());
                    }
                    return;
                }
                "Escape" => {
                    event.prevent_default();
                    event.stop_propagation();
                    state.editor.actions.set(None);
                    return;
                }
                _ => {}
            }
        }
        // F2 renames the symbol under the caret, the
        // key every editor uses for it. rust-analyzer has
        // always been able to do this; rusty never asked.
        if event.key() == "F2" && !event.ctrl_key() && is_rust {
            event.prevent_default();
            // Taken here, so the window's binding for the
            // same key does not send it back a second time.
            event.stop_propagation();
            if let Some(element) = area.get_untracked() {
                // The word under the caret is read off the
                // screen text, because that is what the
                // selection indexes; the line it is on is
                // then a document line, because that is
                // what the server renames by.
                let text = screen(state);
                let cursor = scalar_of_units(
                    &text,
                    element.selection_start().ok().flatten().unwrap_or(0) as usize,
                );
                if let (Some(word), Some((row, col))) =
                    (word_at(&text, cursor), caret_line_col(&element, &text))
                {
                    let line = line_of_row(state, row);
                    state
                        .editor
                        .rename
                        .set(Some((path.clone(), line, col, word)));
                }
            }
            return;
        }
        // Comment or uncomment, for everyone — Vim's `gc`
        // reaches the same function. This editor had no
        // comment toggle at all before, in any mode, and
        // commenting out a block of pin setup is the most
        // ordinary thing anyone does while bringing a
        // board up.
        if (event.ctrl_key() || event.meta_key()) && event.key() == "/" {
            event.prevent_default();
            // And kept from the window, whose `editor.comment`
            // binding answers Ctrl+/ by sending it to this
            // textarea again: the comment went on and came
            // straight back off, so the key did nothing on the
            // caret's line — and a selection lost the marker
            // from its first line only.
            event.stop_propagation();
            if let Some(element) = area.get_untracked() {
                comment_selection(state, &element);
            }
            return;
        }
        // Ctrl+. asks what the server can fix here.
        if (event.ctrl_key() || event.meta_key()) && event.key() == "." && is_rust {
            event.prevent_default();
            if let Some(element) = area.get_untracked() {
                let text = screen(state);
                if let Some((row, col)) = caret_line_col(&element, &text) {
                    state.editor.completion.set(None);
                    controller::request_actions(state, path.clone(), line_of_row(state, row), col);
                }
            }
            return;
        }
        // The popup owns its keys while it *shows*
        // something. `Some` alone was the test, and a
        // popup narrowed to nothing — `v.xyz` — was
        // invisible yet still ate Enter and Tab.
        let shown = state.editor.completion.with_untracked(|popup| {
            popup.as_ref().map_or(0, |popup| {
                visible_items(popup, &state.editor.draft.get_untracked()).len()
            })
        });
        if shown > 0 {
            match event.key().as_str() {
                // Round the ends, as VS Code's list does.
                "ArrowDown" => {
                    event.prevent_default();
                    picked.update(|i| *i = (*i + 1) % shown);
                    return;
                }
                "ArrowUp" => {
                    event.prevent_default();
                    picked.update(|i| *i = (*i + shown - 1) % shown);
                    return;
                }
                "Enter" | "Tab" => {
                    event.prevent_default();
                    if let Some(element) = area.get_untracked() {
                        accept_completion(state, &element, picked.get_untracked());
                    }
                    return;
                }
                _ => {}
            }
        }
        // Moving the caret leaves the word the popup is
        // about, and an answer still on its way would
        // open it wherever the caret had gone.
        if matches!(
            event.key().as_str(),
            "ArrowLeft"
                | "ArrowRight"
                | "ArrowUp"
                | "ArrowDown"
                | "Home"
                | "End"
                | "PageUp"
                | "PageDown"
        ) {
            controller::dismiss_completion(state);
        }
        if event.key() == "Escape" && state.editor.signature.with_untracked(Option::is_some) {
            event.prevent_default();
            event.stop_propagation();
            state.editor.signature.set(None);
            return;
        }
        if (event.ctrl_key() || event.meta_key())
            && (event.key().eq_ignore_ascii_case("f") || event.key().eq_ignore_ascii_case("h"))
        {
            event.prevent_default();
            // Prefill from the selection, as every editor
            // does — finding the thing under the cursor is
            // the whole gesture.
            if let Some((_, _, picked)) = area
                .get_untracked()
                .and_then(|element| selection_of(&element, state))
                && !picked.contains('\n')
            {
                state.find.query.set(picked);
                state.find.index.set(0);
            }
            state.find.open.set(true);
            if event.key().eq_ignore_ascii_case("h") {
                state.find.replace_open.set(true);
            }
            return;
        }
        if event.key() == "F3" && state.find.open.get_untracked() {
            event.prevent_default();
            find_jump(state, scroller, if event.shift_key() { -1 } else { 1 });
            return;
        }
        if event.key() == "Escape"
            && state.find.open.get_untracked()
            && state.editor.completion.with_untracked(Option::is_none)
            && state.editor.signature.with_untracked(Option::is_none)
        {
            event.prevent_default();
            state.find.open.set(false);
            state.find.replace_open.set(false);
            return;
        }
        if (event.ctrl_key() || event.meta_key())
            && event.key().eq_ignore_ascii_case("z")
            && !event.shift_key()
        {
            event.prevent_default();
            if let Some(element) = area.get_untracked() {
                apply_history(&element, state, scroller, true);
            }
            return;
        }
        if (event.ctrl_key() || event.meta_key())
            && (event.key().eq_ignore_ascii_case("y")
                || (event.key().eq_ignore_ascii_case("z") && event.shift_key()))
        {
            event.prevent_default();
            if let Some(element) = area.get_untracked() {
                apply_history(&element, state, scroller, false);
            }
            return;
        }
        // Ctrl+Space asks without a trigger character.
        if event.ctrl_key() && event.key() == " " {
            event.prevent_default();
            if let Some(element) = area.get_untracked() {
                let text = screen(state);
                if let Some((row, col)) = caret_line_col(&element, &text) {
                    let start = word_start_before(&text, row, col);
                    let line = line_of_row(state, row);
                    controller::request_completion(state, path.clone(), line, col, start, true);
                }
            }
            return;
        }
        if (event.ctrl_key() || event.meta_key()) && event.key().eq_ignore_ascii_case("s") {
            event.prevent_default();
            format_and_save(state, area);
            return;
        }
        if read_only {
            return;
        }
        if event.key() == "Enter" {
            event.prevent_default();
            // A new line is not the word an answer on its
            // way was asked for.
            controller::dismiss_completion(state);
            if let Some(element) = area.get_untracked() {
                let (from, to) = doc_selection(&element, state);
                // A selection is replaced by the newline,
                // as the browser would have done.
                let edit = if from < to {
                    pairs::Edit {
                        range: (from, to),
                        text: "\n".to_string(),
                        caret: from + 1,
                        select: None,
                    }
                } else {
                    pairs::on_enter(&state.editor.draft.get_untracked(), from)
                };
                apply_edit(&element, state, &edit);
                keep_caret_in_view(&element, state, scroller);
            }
            return;
        }
        // Bracket pairs — the four things every editor
        // does that a textarea does not: an opener brings
        // its closer, a closer typed against one steps
        // over it, a closer on a blank line takes its
        // opener's indentation, and Backspace inside an
        // empty pair removes both. The rules are `pairs`,
        // pure and tested; this applies what they return
        // and then asks the server what the input event
        // would have — that event never fires for a key
        // that was prevented, and a `(` that opened a pair
        // without asking for the signature would be a
        // feature taken away by adding one.
        if !event.ctrl_key() && !event.alt_key() && !event.meta_key() {
            let key = event.key();
            let mut chars = key.chars();
            if let (Some(ch), None) = (chars.next(), chars.next())
                && let Some(element) = area.get_untracked()
            {
                let (from, to) = doc_selection(&element, state);
                let draft = state.editor.draft.get_untracked();
                if let Some(edit) = pairs::on_type(&draft, from, to, ch) {
                    event.prevent_default();
                    apply_edit(&element, state, &edit);
                    typed_triggers(state, &path, is_rust, &element, true);
                    return;
                }
            }
            if key == "Backspace"
                && let Some(element) = area.get_untracked()
            {
                let (from, to) = doc_selection(&element, state);
                if from == to
                    && let Some(edit) =
                        pairs::on_backspace(&state.editor.draft.get_untracked(), from)
                {
                    event.prevent_default();
                    apply_edit(&element, state, &edit);
                    typed_triggers(state, &path, is_rust, &element, false);
                    return;
                }
            }
        }
        // A text area would move focus on Tab. In an editor
        // that is never what was meant. A Tab with Ctrl or
        // Alt is somebody else's — Ctrl+Tab switches files,
        // and in a window with no switcher (a detached
        // editor) it must not indent either.
        if event.key() == "Tab" && !event.ctrl_key() && !event.alt_key() && !event.meta_key() {
            event.prevent_default();
            if let Some(element) = area.get_untracked() {
                insert_at_caret(&element, state, "    ");
            }
        }
    }
}
