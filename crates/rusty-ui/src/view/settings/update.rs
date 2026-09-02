//! The running version, and what is published.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::components::{Button, ButtonKind, Pill, Tone},
};

use super::*;

/// What is installed, and what is published.
///
/// Checking is a button rather than a poll at startup: a workbench that
/// phones home on launch is a workbench that hangs on launch behind a bad
/// proxy, and this one is used behind those.
#[component]
pub(super) fn UpdateSettings() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <Field
            label=t!("settings.update.version")
            help=t!("settings.update.version-help")
        >
            <div class="flex flex-col gap-3">
                <div class="flex items-center gap-3">
                    <code class="rounded-[4px] bg-sunken px-1.5 py-0.5 font-mono text-footnote">
                        {env!("CARGO_PKG_VERSION")}
                    </code>
                    <Button
                        label=t!("settings.update.check")
                        kind=ButtonKind::Normal
                        on_click=Callback::new(move |_| controller::check_update(state))
                    />
                </div>
                {move || {
                    let status = state.app.update.get()?;
                    let body = if status.newer {
                        let version = status.latest.clone().unwrap_or_default();
                        let url = status.url.clone().unwrap_or_default();
                        view! {
                            <div class="flex items-center gap-3">
                                <Pill label=t!("settings.update.available", version = version.clone()) tone=Tone::Rust />
                                <Button
                                    label=t!("settings.update.open-release")
                                    kind=ButtonKind::Primary
                                    on_click=Callback::new(move |_| {
                                        controller::open_url(state, url.clone())
                                    })
                                />
                            </div>
                        }
                            .into_any()
                    } else if let Some(note) = status.note.clone() {
                        // A failed check is a note, not an error: no network
                        // is the normal state of a workbench on a bench.
                        view! {
                            <p class="max-w-[70ch] text-callout text-label-3">
                                {t!("settings.update.unreachable")}{note}
                            </p>
                        }
                            .into_any()
                    } else {
                        view! {
                            <p class="text-callout text-label-2">{t!("settings.update.newest")}</p>
                        }
                            .into_any()
                    };
                    Some(body)
                }}
            </div>
        </Field>
    }
}
