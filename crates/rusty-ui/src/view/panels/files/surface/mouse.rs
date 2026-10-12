//! The pointer over the text: a press, a move and leaving. Methods on
//! `Pane`, so the textarea's handlers are one line each.

use super::*;

impl Pane {
    /// A press: another cursor with Alt, a definition with Ctrl, and otherwise
    /// the caret placed by this editor's arithmetic (`pointer.rs`).
    pub(super) fn on_mousedown(self, event: ev::MouseEvent) {
        let Self {
            state,
            area,
            scroller,
            is_rust,
            drag,
            ..
        } = self;
        let path = self.path.get_value();
        // Alt adds a cursor; a plain press is back to
        // one (`multi.rs`).
        if let Some(element) = area.get_untracked()
            && multi_click(state, &element, &event, scroller)
        {
            return;
        }
        // Ctrl+Click asks where this is defined — the
        // gesture every editor has taught.
        if !(event.ctrl_key() || event.meta_key()) || !is_rust {
            controller::dismiss_completion(state);
            state.editor.signature.set(None);
            state.editor.actions.set(None);
            // Where the press lands is this editor's
            // arithmetic, not the textarea's (`pointer.rs`).
            if let Some(element) = area.get_untracked() {
                press(state, &element, drag, &event);
            }
            return;
        }
        event.prevent_default();
        self.unlink();
        // A pixel names a *row*; the server wants a
        // document line. `line_of_row` is the
        // identity while nothing is folded.
        let Some(element) = area.get_untracked() else {
            return;
        };
        let (x, y) = point_in_column(
            &element,
            (f64::from(event.client_x()), f64::from(event.client_y())),
        );
        if let Some((row, col)) = cell_at_point(state, x, y) {
            controller::goto_definition(state, path.clone(), line_of_row(state, row), col);
        }
    }

    /// The pointer over the text: the link under Ctrl, and the hover card asked
    /// for once the pointer has settled.
    pub(super) fn on_mousemove(self, event: ev::MouseEvent) {
        let Self {
            state,
            area,
            is_rust,
            hover_cell,
            hover_gen,
            on_card,
            ..
        } = self;
        let path = self.path.get_value();
        if !is_rust {
            return;
        }
        let Some(element) = area.get_untracked() else {
            return;
        };
        let (x, y) = point_in_column(
            &element,
            (f64::from(event.client_x()), f64::from(event.client_y())),
        );
        let cell = cell_at_point(state, x, y).map(|(row, col)| (line_of_row(state, row), col));
        if event.ctrl_key() || event.meta_key() {
            self.probe_link(cell);
        } else {
            self.unlink();
        }
        if hover_cell.get_untracked() == cell {
            return;
        }
        hover_cell.set(cell);

        // Inside the shown token, there is nothing to
        // dismiss and nothing to re-request.
        let inside = state.editor.hover.with_untracked(|h| {
            h.as_ref()
                .is_some_and(|card| cell.is_some_and(|(l, c)| within(&card.range, l, c)))
        });
        if inside {
            return;
        }

        // Outside it: a short grace before the card
        // goes, so the pointer can cross the gap onto
        // the card without killing it en route.
        let generation = hover_gen.get_untracked() + 1;
        hover_gen.set(generation);
        if state.editor.hover.with_untracked(Option::is_some) {
            set_timeout(
                move || {
                    // The surface can be disposed inside the
                    // grace period — the tab closed, the project
                    // switched, the file replaced — and these are
                    // *its* signals, not the app's. Reading a
                    // disposed one panics, and a panic in wasm
                    // aborts the module: the window keeps its
                    // last paint and every handler in it is dead,
                    // right down to the close button. `try_` is
                    // None once the owner is gone.
                    let (Some(current), Some(on_card)) =
                        (hover_gen.try_get_untracked(), on_card.try_get_untracked())
                    else {
                        return;
                    };
                    if current == generation && !on_card {
                        state.editor.hover.set(None);
                    }
                },
                std::time::Duration::from_millis(300),
            );
        }

        let Some((line, col)) = cell else { return };
        let path = path.clone();
        set_timeout(
            move || {
                // Disposed-safe, as above.
                let (Some(current), Some(cell)) = (
                    hover_gen.try_get_untracked(),
                    hover_cell.try_get_untracked(),
                ) else {
                    return;
                };
                if current == generation && cell == Some((line, col)) {
                    controller::request_hover(state, path, line, col);
                }
            },
            std::time::Duration::from_millis(400),
        );
    }

    /// The pointer leaving the text: the link goes, and the card after a grace.
    pub(super) fn on_mouseleave(self) {
        let Self {
            state,
            hover_cell,
            hover_gen,
            on_card,
            ..
        } = self;
        self.unlink();
        hover_cell.set(None);
        let generation = hover_gen.get_untracked() + 1;
        hover_gen.set(generation);
        set_timeout(
            move || {
                // Disposed-safe, as above. Leaving the surface is
                // exactly when it is most likely to go away.
                let (Some(current), Some(on_card)) =
                    (hover_gen.try_get_untracked(), on_card.try_get_untracked())
                else {
                    return;
                };
                if current == generation && !on_card {
                    state.editor.hover.set(None);
                }
            },
            std::time::Duration::from_millis(300),
        );
    }
}
