//! Modal editing and text size.

use leptos::prelude::*;

use crate::{controller, state::AppState};

use super::*;

/// The editor itself, rather than how it looks.
///
/// Modal editing was reachable only from a menu at first, which is where a
/// setting is *looked* for last. Anything that changes how the editor
/// behaves belongs here, where somebody goes to ask "can it do X".
#[component]
pub(super) fn EditorSettings() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <Field
            label="Vim keys"
            help="Modal editing in the editor: normal, insert and visual modes, with the \
                  mode shown in the status bar. Remembered across launches and shared by \
                  every window. Chords stay with the editor — Ctrl+S saves, Ctrl+A selects \
                  all, Ctrl+C copies — and insert mode behaves exactly as it does with this \
                  off, so nothing you already know stops working."
        >
            <div class="inline-flex self-start rounded-[7px] bg-sunken p-0.5">
                {[("Off", false), ("On", true)]
                    .into_iter()
                    .map(|(label, wanted)| {
                        let class = move || {
                            let on = state.editor.vim_on.get() == wanted;
                            let base = "rounded-[6px] px-3 py-1 text-callout transition-colors";
                            if on {
                                format!("{base} bg-raised text-label shadow-sm")
                            } else {
                                format!("{base} text-label-3 hover:text-label")
                            }
                        };
                        view! {
                            <button
                                type="button"
                                class=class
                                on:click=move |_| controller::set_vim(state, wanted)
                            >
                                {label}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
        </Field>

        <Field
            label="Text size"
            help="The editor's own zoom, separate from the interface scale in Appearance. \
                  Ctrl+= and Ctrl+- change it from the editor too."
        >
            <div class="flex items-center gap-3">
                {[("A-", -1.0), ("Reset", 0.0), ("A+", 1.0)]
                    .into_iter()
                    .map(|(label, step)| {
                        view! {
                            <button
                                type="button"
                                class="rounded-[6px] bg-sunken px-3 py-1 text-callout text-label-2 transition-colors hover:bg-raised hover:text-label"
                                on:click=move |_| {
                                    let next = if step == 0.0 {
                                        1.0
                                    } else {
                                        (state.editor.zoom.get_untracked() + step * 0.1)
                                            .clamp(0.6, 2.4)
                                    };
                                    state.editor.zoom.set(next);
                                    crate::state::remember_zoom(next);
                                }
                            >
                                {label}
                            </button>
                        }
                    })
                    .collect_view()}
                <span class="tnum font-mono text-caption text-label-3">
                    {move || format!("{:.0}%", state.editor.zoom.get() * 100.0)}
                </span>
            </div>
        </Field>
    }
}
