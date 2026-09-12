//! Theme and interface scale.

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
    // The preview during the drag is the label, never the zoom: zooming the
    // window mid-drag rescales the slider under the pointer, which feeds
    // back into the value — the whole interface shook until the pointer
    // escaped.
    let preview = RwSignal::new(None::<f64>);
    let percent = move || {
        let factor = preview.get().unwrap_or_else(|| state.layout.zoom.get());
        format!("{:.0}", factor * 100.0)
    };

    view! {
        <Group footer=t!("settings.appearance.scale-note")>
            <Row label=t!("settings.appearance.theme")>
                <Segmented>
                    {Theme::ALL
                        .into_iter()
                        .map(|option| {
                            view! {
                                <Segment
                                    label=option.label()
                                    selected=Signal::derive(move || theme.get() == option)
                                    on_click=Callback::new(move |_| {
                                        theme::set(option);
                                        theme.set(option);
                                    })
                                />
                            }
                        })
                        .collect_view()}
                </Segmented>
            </Row>
            <Row label=t!("settings.appearance.scale")>
                <input
                    type="range"
                    min="70"
                    max="160"
                    step="5"
                    prop:value=percent
                    on:input=move |event| {
                        if let Ok(value) = event_target_value(&event).parse::<f64>() {
                            let (min, max) = crate::state::UI_ZOOM_RANGE;
                            preview.set(Some((value / 100.0).clamp(min, max)));
                        }
                    }
                    on:change=move |event| {
                        if let Ok(value) = event_target_value(&event).parse::<f64>() {
                            let (min, max) = crate::state::UI_ZOOM_RANGE;
                            let factor = (value / 100.0).clamp(min, max);
                            preview.set(None);
                            state.layout.zoom.set(factor);
                            crate::state::remember_ui_zoom(factor);
                            controller::apply_ui_zoom(state);
                        }
                    }
                    class="w-44 accent-rust"
                />
                <span class="tnum w-[4ch] text-right font-mono text-callout text-label-2">
                    {move || format!("{}%", percent())}
                </span>
            </Row>
        </Group>
    }
}
