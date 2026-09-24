//! The editor's own right-click menu: the clipboard three every text box
//! has, and what only this editor knows.

use super::*;

/// The menu the last right-click opened, where it opened.
#[component]
pub(super) fn EditorMenu(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        area,
        read_only,
        is_rust,
        editor_menu,
        ..
    } = pane;
    let path = pane.path.get_value();
    view! {
        {
            let path = path.clone();
            move || {
                let (x, y) = editor_menu.get()?;
                let close = Callback::new(move |_| editor_menu.set(None));
                let path = path.clone();
                let has_selection = area
                    .get_untracked()
                    .and_then(|element| selection_of(&element, state))
                    .is_some();
                let (goto_path, fix_path) = (path.clone(), path.clone());
                Some(
                    view! {
                        <ContextMenu x=x y=y on_close=close>
                            <MenuItem
                                label=t!("context.editor-cut")
                                shortcut="Ctrl+X"
                                disabled=read_only
                                on_select=Callback::new(move |_| {
                                    // With nothing selected, the line — the
                                    // same rule as the key.
                                    if let Some(element) = area.get_untracked()
                                        && !has_selection
                                    {
                                        clipboard_key(state, &element, true, read_only);
                                    } else if let Some(element) = area.get_untracked() {
                                        let text = state.editor.draft.get_untracked();
                                        if let Some((from, to, picked)) =
                                            selection_of(&element, state)
                                        {
                                            copy_to_clipboard(&picked);
                                            record_edit(state);
                                            let mut next = text.clone();
                                            next.replace_range(from..to, "");
                                            echo_edit(state, &next);
                                            set_buffer(state, &element, &next);
                                            let caret = utf16_len(&next[..from]);
                                            let _ = element.set_selection_start(Some(caret));
                                            let _ = element.set_selection_end(Some(caret));
                                            controller::schedule_pulse(state);
                                        }
                                    }
                                    editor_menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.editor-copy")
                                shortcut="Ctrl+C"
                                on_select=Callback::new(move |_| {
                                    if let Some(element) = area.get_untracked()
                                        && !has_selection
                                    {
                                        clipboard_key(state, &element, false, read_only);
                                    } else if let Some((_, _, picked)) = area
                                        .get_untracked()
                                        .and_then(|element| selection_of(&element, state))
                                    {
                                        copy_to_clipboard(&picked);
                                    }
                                    editor_menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.editor-paste")
                                shortcut="Ctrl+V"
                                disabled=read_only
                                on_select=Callback::new(move |_| {
                                    paste_at_caret(state, area);
                                    editor_menu.set(None);
                                })
                            />
                            <MenuSeparator />
                            <MenuItem
                                label=t!("context.editor-definition")
                                shortcut="Ctrl+Click"
                                disabled=!is_rust
                                on_select=Callback::new(move |_| {
                                    if let Some(element) = area.get_untracked()
                                        && let Some((row, col)) =
                                            caret_line_col(&element, &screen(state))
                                    {
                                        controller::goto_definition(
                                            state,
                                            goto_path.clone(),
                                            line_of_row(state, row),
                                            col,
                                        );
                                    }
                                    editor_menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("menu.view.references")
                                shortcut="Shift+F12"
                                disabled=!is_rust
                                on_select=Callback::new(move |_| {
                                    editor_menu.set(None);
                                    controller::find_places(state, controller::PlaceQuery::References);
                                })
                            />
                            <MenuItem
                                label=t!("menu.view.implementations")
                                shortcut="Ctrl+F12"
                                disabled=!is_rust
                                on_select=Callback::new(move |_| {
                                    editor_menu.set(None);
                                    controller::find_places(
                                        state,
                                        controller::PlaceQuery::Implementations,
                                    );
                                })
                            />
                            <MenuItem
                                label=t!("menu.view.type-definition")
                                disabled=!is_rust
                                on_select=Callback::new(move |_| {
                                    editor_menu.set(None);
                                    controller::find_places(
                                        state,
                                        controller::PlaceQuery::TypeDefinition,
                                    );
                                })
                            />
                            <MenuItem
                                label=t!("menu.view.call-hierarchy")
                                disabled=!is_rust
                                on_select=Callback::new(move |_| {
                                    editor_menu.set(None);
                                    controller::show_call_hierarchy(state);
                                })
                            />
                            <MenuItem
                                label=t!("menu.view.expand-macro")
                                disabled=!is_rust
                                on_select=Callback::new(move |_| {
                                    editor_menu.set(None);
                                    controller::expand_macro(state);
                                })
                            />
                            <MenuItem
                                label=t!("context.editor-quick-fix")
                                shortcut="Ctrl+."
                                disabled=!is_rust
                                on_select=Callback::new(move |_| {
                                    if let Some(element) = area.get_untracked()
                                        && let Some((row, col)) =
                                            caret_line_col(&element, &screen(state))
                                    {
                                        controller::request_actions(
                                            state,
                                            fix_path.clone(),
                                            line_of_row(state, row),
                                            col,
                                        );
                                    }
                                    editor_menu.set(None);
                                })
                            />
                            <MenuSeparator />
                            <MenuItem
                                label=t!("context.editor-fold-all")
                                on_select=Callback::new(move |_| {
                                    fold_all(state);
                                    editor_menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.editor-unfold-all")
                                on_select=Callback::new(move |_| {
                                    unfold_all(state);
                                    editor_menu.set(None);
                                })
                            />
                            <MenuSeparator />
                            <MenuItem
                                label=t!("context.editor-save")
                                shortcut="Ctrl+S"
                                disabled=read_only
                                on_select=Callback::new(move |_| {
                                    format_and_save(state, area);
                                    editor_menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.editor-find")
                                shortcut="Ctrl+F"
                                on_select=Callback::new(move |_| {
                                    state.find.open.set(true);
                                    editor_menu.set(None);
                                })
                            />
                        </ContextMenu>
                    },
                )
            }
        }
    }
}
