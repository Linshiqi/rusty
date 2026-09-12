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
        <Group>
            <Row label=t!("settings.update.version")>
                <code class="font-mono text-footnote text-label-2">{env!("CARGO_PKG_VERSION")}</code>
                <Button
                    label=t!("settings.update.check")
                    on_click=Callback::new(move |_| controller::check_update(state))
                />
            </Row>
            {move || {
                let status = state.app.update.get()?;
                let body = if status.newer {
                    let version = status.latest.clone().unwrap_or_default();
                    let url = status.url.clone().unwrap_or_default();
                    view! {
                        <Row label=t!("settings.update.available", version = version)>
                            <Button
                                label=t!("settings.update.open-release")
                                kind=ButtonKind::Primary
                                on_click=Callback::new(move |_| controller::open_url(state, url.clone()))
                            />
                        </Row>
                    }
                        .into_any()
                } else if let Some(note) = status.note.clone() {
                    // A failed check is a note, not an error: no network is
                    // the normal state of a workbench on a bench.
                    view! { <NoteRow text=format!("{}{note}", t!("settings.update.unreachable")) /> }
                        .into_any()
                } else {
                    view! {
                        <div class="flex items-center gap-2 px-3.5 py-2">
                            <Pill label=t!("settings.update.newest") tone=Tone::Patina />
                        </div>
                    }
                        .into_any()
                };
                Some(body)
            }}
        </Group>
    }
}
