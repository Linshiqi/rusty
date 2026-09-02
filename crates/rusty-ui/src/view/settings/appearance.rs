//! Theme.

use leptos::prelude::*;

use rusty_i18n::t;

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
            label=t!("settings.appearance.scale")
            help=t!("settings.appearance.scale-help")
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
                                    let (min, max) = crate::state::UI_ZOOM_RANGE;
                                    preview.set(Some((percent / 100.0).clamp(min, max)));
                                }
                            }
                            on:change=move |event| {
                                if let Ok(percent) =
                                    event_target_value(&event).parse::<f64>()
                                {
                                    let (min, max) = crate::state::UI_ZOOM_RANGE;
                                    let factor = (percent / 100.0).clamp(min, max);
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
            label=t!("settings.appearance.theme")
            help=t!("settings.appearance.theme-help")
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
