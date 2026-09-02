//! Modal editing and text size.

use leptos::prelude::*;

use rusty_i18n::t;

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
            label=t!("settings.editor.vim")
            help=t!("settings.editor.vim-help")
        >
            <div class="inline-flex self-start rounded-[7px] bg-sunken p-0.5">
                {[(t!("settings.editor.off"), false), (t!("settings.editor.on"), true)]
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
            label=t!("settings.editor.text-size")
            help=t!("settings.editor.text-size-help")
        >
            <div class="flex items-center gap-3">
                {[("A-".to_string(), -1.0), (t!("settings.editor.reset"), 0.0), ("A+".to_string(), 1.0)]
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
                                            .clamp(
                                                crate::state::EDITOR_ZOOM_RANGE.0,
                                                crate::state::EDITOR_ZOOM_RANGE.1,
                                            )
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
