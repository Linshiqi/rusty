//! How downloads reach the internet.

use leptos::prelude::*;

use rusty_i18n::t;

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

    let choose = move |value: Option<&'static str>| {
        crate::controller::save_proxy_setting(value.map(str::to_string), stored, detected, saved);
    };

    view! {
        <Field
            label=t!("settings.network.proxy")
            help=t!("settings.network.proxy-help")
        >
            <div class="flex flex-col gap-2">
                <div class="flex items-center gap-2">
                    {move || {
                        let current = stored.get();
                        let is_auto = current.is_none();
                        let is_none = current.as_deref() == Some("none");
                        let manual = !is_auto && !is_none;
                        let pick = |on: bool| {
                            if on {
                                "rounded-[6px] bg-selection px-2.5 py-1 text-footnote text-rust"
                            } else {
                                "rounded-[6px] px-2.5 py-1 text-footnote text-label-3 hover:text-label"
                            }
                        };
                        view! {
                            <button
                                type="button"
                                class=pick(is_auto)
                                on:click=move |_| choose(None)
                            >
                                {t!("misc.proxy-detect")}
                            </button>
                            <button
                                type="button"
                                class=pick(is_none)
                                on:click=move |_| choose(Some("none"))
                            >
                                {t!("misc.proxy-direct")}
                            </button>
                            <span class=pick(manual)>{t!("misc.proxy-manual")}</span>
                        }
                    }}
                    <input
                        type="text"
                        placeholder="http://127.0.0.1:7890"
                        autocomplete="off"
                        spellcheck="false"
                        prop:value=move || {
                            stored
                                .get()
                                .filter(|v| v != "none")
                                .unwrap_or_default()
                        }
                        on:change=move |event: leptos::ev::Event| {
                            let value = event_target_value(&event);
                            let value = value.trim();
                            if !value.is_empty() {
                                crate::controller::save_proxy_setting(
                                    Some(value.to_string()),
                                    stored,
                                    detected,
                                    saved,
                                );
                            }
                        }
                        class="w-[26ch] rounded-[6px] bg-sunken px-2.5 py-1 font-mono text-footnote text-label placeholder:text-label-4"
                    />
                </div>
                {move || {
                    let line = match (stored.get(), detected.get()) {
                        (None, Some(found)) => t!("settings.network.detected", proxy = found),
                        (None, None) => t!("settings.network.none"),
                        (Some(v), _) if v == "none" => t!("settings.network.forced-direct"),
                        (Some(url), _) => t!("settings.network.using", url = url),
                    };
                    view! {
                        <p class="text-footnote text-label-3 select-text">{line}</p>
                    }
                }}
                {move || {
                    saved
                        .get()
                        .then(|| {
                            view! {
                                <p class="text-footnote text-patina">{t!("settings.network.saved")}</p>
                            }
                        })
                }}
            </div>
        </Field>
    }
}
