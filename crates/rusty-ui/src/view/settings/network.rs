//! How downloads reach the internet.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::view::components::{Pill, Tone};

use super::*;

/// The proxy for tool downloads and crates.io queries.
///
/// Detect is the default and reads the environment, then the OS proxy the
/// browser uses — a Clash on 127.0.0.1:7890 is found without being told.
/// The other two exist for when detection is wrong: force direct, or name
/// the proxy outright.
#[component]
pub(super) fn NetworkSettings() -> impl IntoView {
    let stored = RwSignal::new(None::<String>);
    let detected = RwSignal::new(None::<String>);
    let saved = RwSignal::new(false);

    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            crate::controller::load_proxy_setting(stored, detected);
        }
    });

    let choose = move |value: Option<String>| {
        crate::controller::save_proxy_setting(value, stored, detected, saved);
    };
    let is_auto = Signal::derive(move || stored.with(Option::is_none));
    let is_direct = Signal::derive(move || stored.with(|s| s.as_deref() == Some("none")));
    let is_manual = Signal::derive(move || !is_auto.get() && !is_direct.get());

    view! {
        <Group footer=t!("settings.network.note")>
            <Row label=t!("settings.network.proxy")>
                <Segmented>
                    <Segment
                        label=t!("misc.proxy-detect")
                        selected=is_auto
                        on_click=Callback::new(move |_| choose(None))
                    />
                    <Segment
                        label=t!("misc.proxy-direct")
                        selected=is_direct
                        on_click=Callback::new(move |_| choose(Some("none".to_string())))
                    />
                    <Segment
                        label=t!("misc.proxy-manual")
                        selected=is_manual
                        on_click=Callback::new(move |_| {})
                    />
                </Segmented>
            </Row>
            <Row label=t!("settings.network.address")>
                <input
                    type="text"
                    placeholder="http://127.0.0.1:7890"
                    autocomplete="off"
                    spellcheck="false"
                    prop:value=move || {
                        stored.get().filter(|v| v != "none").unwrap_or_default()
                    }
                    on:change=move |event: leptos::ev::Event| {
                        let value = event_target_value(&event);
                        let value = value.trim();
                        if !value.is_empty() {
                            choose(Some(value.to_string()));
                        }
                    }
                    class="h-[26px] w-[260px] rounded-[6px] bg-sunken px-2.5 font-mono text-footnote text-label outline-none ring-1 ring-line placeholder:text-label-4 focus:ring-rust"
                />
                {move || saved.get().then(|| view! { <Pill label=t!("settings.network.saved") tone=Tone::Patina /> })}
            </Row>
            {move || {
                let line = match (stored.get(), detected.get()) {
                    (None, Some(found)) => t!("settings.network.detected", proxy = found),
                    (None, None) => t!("settings.network.none"),
                    (Some(v), _) if v == "none" => t!("settings.network.forced-direct"),
                    (Some(url), _) => t!("settings.network.using", url = url),
                };
                view! { <NoteRow text=line /> }
            }}
        </Group>
    }
}
