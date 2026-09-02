//! Renaming a symbol everywhere it is used.

use rusty_i18n::t;

use leptos::{ev, html, prelude::*};

use crate::{controller, state::AppState};

/// Where a new name is typed. Appears only while a rename is pending.
///
/// A strip rather than a modal, matching the find bar: a rename is a small
/// question about the code you are looking at, and covering the code to ask
/// it takes away the one thing that answers it.
#[component]
pub(super) fn RenameBar() -> impl IntoView {
    let state = AppState::expect();
    let box_ref: NodeRef<html::Input> = NodeRef::new();

    // Focused and pre-selected, so the old name can be typed straight over.
    Effect::new(move |_| {
        if state.editor.rename.with(Option::is_some)
            && let Some(element) = box_ref.get()
        {
            let _ = element.focus();
            element.select();
        }
    });

    move || {
        let (path, line, col, word) = state.editor.rename.get()?;
        Some(view! {
            <div class="flex flex-none items-center gap-2 border-b border-line bg-raised px-3 py-1.5">
                <span class="text-caption text-label-3">"Rename"</span>
                <span class="font-mono text-caption text-label-2">{word.clone()}</span>
                <span class="text-caption text-label-4">"to"</span>
                <input
                    node_ref=box_ref
                    prop:value=word
                    class="w-48 rounded-[5px] bg-sunken px-1.5 py-0.5 font-mono text-caption text-label outline-none ring-1 ring-line focus:ring-rust"
                    on:keydown=move |event: ev::KeyboardEvent| {
                        match event.key().as_str() {
                            "Enter" => {
                                event.prevent_default();
                                let name = event_target_value(&event);
                                let name = name.trim().to_string();
                                state.editor.rename.set(None);
                                if !name.is_empty() {
                                    controller::rename_symbol(
                                        state,
                                        path.clone(),
                                        line,
                                        col,
                                        name,
                                    );
                                }
                            }
                            "Escape" => {
                                event.prevent_default();
                                event.stop_propagation();
                                state.editor.rename.set(None);
                            }
                            _ => {}
                        }
                    }
                />
                <span class="text-caption text-label-4">
                    {t!("misc.rename-scope")}
                </span>
            </div>
        })
    }
}
