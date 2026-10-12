//! The running version, what is published, and the way from one to the
//! other.
//!
//! The sheet (`view/update.rs`) is where an update is read and taken; this
//! page is the standing record — the version, the last check's answer, and
//! the restart still on offer after the sheet was put away.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::{AppState, UpdateStage},
    view::components::{Button, ButtonKind, Pill, Tone},
};

use super::*;

#[component]
pub(super) fn UpdateSettings() -> impl IntoView {
    let state = AppState::expect();
    // The backend's version, which the release workflow stamps from the
    // tag; the crate's own is the fallback until a check has answered.
    let version = move || {
        state
            .app
            .update
            .get()
            .map(|status| status.current)
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
    };

    view! {
        <Group footer=t!("settings.update.footer")>
            <Row label=t!("settings.update.version")>
                <code class="font-mono text-footnote text-label-2">{version}</code>
                <Button
                    label=t!("settings.update.check")
                    disabled=Signal::derive(move || {
                        state.app.update_stage.get() == UpdateStage::Checking
                    })
                    on_click=Callback::new(move |_| controller::check_update(state, true))
                />
            </Row>
            {move || {
                let status = state.app.update.get()?;
                let stage = state.app.update_stage.get();
                let body = if status.newer {
                    let version = status.latest.clone().unwrap_or_default();
                    let detail = match stage {
                        UpdateStage::Downloading => t!("settings.update.downloading"),
                        UpdateStage::Ready | UpdateStage::Applying => {
                            t!("settings.update.downloaded")
                        }
                        _ if status.skipped => t!("settings.update.skipped"),
                        _ => status.date.clone().unwrap_or_default(),
                    };
                    let url = status.url.clone().unwrap_or_default();
                    let action = if stage == UpdateStage::Ready {
                        view! {
                            <Button
                                label=t!("settings.update.restart")
                                kind=ButtonKind::Primary
                                on_click=Callback::new(move |_| controller::apply_update(state))
                            />
                        }
                            .into_any()
                    } else {
                        view! {
                            <Button
                                label=t!("settings.update.details")
                                kind=ButtonKind::Primary
                                on_click=Callback::new(move |_| state.app.update_open.set(true))
                            />
                        }
                            .into_any()
                    };
                    view! {
                        <Row label=t!("settings.update.available", version = version) detail=detail>
                            <Button
                                label=t!("settings.update.open-release")
                                kind=ButtonKind::Quiet
                                on_click=Callback::new(move |_| controller::open_url(state, url.clone()))
                            />
                            {action}
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
