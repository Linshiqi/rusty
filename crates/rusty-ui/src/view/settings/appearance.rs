//! Theme.

use leptos::prelude::*;

use crate::{
    controller,
    state::AppState,
    theme::{self, Theme},
};

use super::*;

#[component]
pub(super) fn Appearance() -> impl IntoView {
    let state = AppState::expect();
    let theme = RwSignal::new(theme::stored());

    view! {
        <Field
            label="Interface scale"
            help="Browser-style zoom over the whole window, 70% to 160%. Applies when you \
                  release the slider; the editor's own Ctrl+wheel text zoom is separate."
        >
            <div class="flex items-center gap-3">
                // The preview during the drag is the label, never the zoom:
                // zooming the window mid-drag rescales the slider under the
                // pointer, which feeds back into the value — the whole
                // interface shook until the pointer escaped.
                {
                    let preview = RwSignal::new(None::<f64>);
                    view! {
                        <input
                            type="range"
                            min="70"
                            max="160"
                            step="5"
                            prop:value=move || {
                                let factor = preview
                                    .get()
                                    .unwrap_or_else(|| state.layout.zoom.get());
                                format!("{:.0}", factor * 100.0)
                            }
                            on:input=move |event| {
                                if let Ok(percent) =
                                    event_target_value(&event).parse::<f64>()
                                {
                                    preview.set(Some((percent / 100.0).clamp(0.7, 1.6)));
                                }
                            }
                            on:change=move |event| {
                                if let Ok(percent) =
                                    event_target_value(&event).parse::<f64>()
                                {
                                    let factor = (percent / 100.0).clamp(0.7, 1.6);
                                    preview.set(None);
                                    state.layout.zoom.set(factor);
                                    crate::state::remember_ui_zoom(factor);
                                    controller::apply_ui_zoom(state);
                                }
                            }
                            class="w-56 accent-rust"
                        />
                        <span class="tnum w-[5ch] font-mono text-callout text-label-2">
                            {move || {
                                let factor = preview
                                    .get()
                                    .unwrap_or_else(|| state.layout.zoom.get());
                                format!("{:.0}%", factor * 100.0)
                            }}
                        </span>
                    }
                }
            </div>
        </Field>
        <Field
            label="Theme"
            help="System follows your desktop, including when you change it while rusty is open."
        >
            <div class="inline-flex rounded-[7px] bg-sunken p-0.5">
                {Theme::ALL
                    .into_iter()
                    .map(|option| {
                        let is_selected = Signal::derive(move || theme.get() == option);
                        view! {
                            <button
                                type="button"
                                on:click=move |_| {
                                    theme::set(option);
                                    theme.set(option);
                                }
                                class=move || {
                                    // A segmented control, as macOS uses for a
                                    // small set of exclusive choices.
                                    let base = "h-[24px] rounded-[5px] px-3 text-callout \
                                                transition-colors";
                                    if is_selected.get() {
                                        format!("{base} bg-content font-medium text-label shadow-sm")
                                    } else {
                                        format!("{base} text-label-2 hover:text-label")
                                    }
                                }
                            >
                                {option.label()}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
        </Field>
    }
}
