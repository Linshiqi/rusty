//! Vim's block cursor, drawn, since a read-only field has no caret.

use super::*;

/// The block where Vim's next key starts, while the textarea has focus.
#[component]
pub(super) fn VimCursor(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        area,
        selection_moves,
        zoom,
        ..
    } = pane;
    view! {
        {move || {
            let modal = state.editor.vim_on.get()
                && state.editor.vim.with(|vim| vim.mode != crate::vim::Mode::Insert);
            if !modal {
                return ().into_any();
            }
            let Some(element) = area.get() else {
                return ().into_any();
            };
            let moves = selection_moves.get();
            let z = zoom.get();
            let path = state.active_path_now();
            let (line, x, width) = state.editor.draft.with(|text| {
                let cursor = vim_cursor(state, &element, path.as_deref(), text);
                let (line, col, under) = cursor_cell(text, cursor);
                let content = text.split('\n').nth(line as usize).unwrap_or_default();
                let hints = path
                    .as_deref()
                    .map(|path| hints_on(state, path, line))
                    .unwrap_or_default();
                // As wide as what it covers — a tab up to its
                // stop — and a space wide on a line break or at
                // the end of the text, where there is nothing.
                match under {
                    Some(_) => {
                        let x = char_left(content, &hints, col, z);
                        (line, x, edge_left(content, &hints, col + 1, z) - x)
                    }
                    None => (
                        line,
                        caret_left(content, &hints, col, z),
                        line_px(" ") * z,
                    ),
                }
            });
            let y = row_top(state, line, z);
            let blink = if moves.is_multiple_of(2) { "vim-cursor-a" } else { "vim-cursor-b" };
            view! {
                <div
                    class="vim-cursor"
                    style=format!(
                        "left: {x}px; top: {y}px; width: {width}px; height: {h}px; \
                         animation-name: {blink}",
                        h = row_height(z),
                    )
                />
            }
            .into_any()
        }}
    }
}
