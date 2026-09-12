//! Where rusty keeps its data, and moving it somewhere else.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    state::AppState,
    view::components::{Button, Pill, Tone},
};

use super::*;

/// Where the data directory is, and moving it.
///
/// This screen exists so nobody deletes a folder they never knew they had —
/// and so pointing the data at a synced folder is a button, not a wiki page.
#[component]
pub(super) fn StorageSettings() -> impl IntoView {
    let state = AppState::expect();
    let location = RwSignal::new(None::<rusty_embed::StorageLocation>);
    let note = RwSignal::new(None::<String>);
    // A relocation refused because the target already holds rusty data; kept
    // so the adopt choice is offered as its own deliberate step.
    let blocked = RwSignal::new(None::<String>);

    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            crate::controller::load_storage_location(location);
        }
    });

    view! {
        <Group footer=t!("settings.storage.note")>
            {move || {
                let Some(here) = location.get() else {
                    return view! { <NoteRow text="…" /> }.into_any();
                };
                let (badge, tone) = if here.env_override {
                    // The variable's own name: it is what to grep for.
                    ("RUSTY_CONFIG_DIR".to_string(), Tone::Amber)
                } else if here.is_default {
                    (t!("settings.storage.default"), Tone::Neutral)
                } else {
                    (t!("settings.storage.custom"), Tone::Patina)
                };
                let env_override = here.env_override;
                view! {
                    <Row label=t!("settings.storage.data-directory") stacked=true>
                        <code class="rounded-[5px] bg-sunken px-2 py-1 font-mono text-footnote text-label select-text">
                            {here.path.clone()}
                        </code>
                        <Pill label=badge tone=tone />
                    </Row>
                    {env_override
                        .then(|| view! { <NoteRow text=t!("settings.storage.env-override") warn=true /> })}
                    <Row label=t!("settings.storage.move")>
                        <Button
                            label=t!("settings.storage.choose")
                            disabled=Signal::derive(move || env_override)
                            on_click=Callback::new(move |_| {
                                crate::controller::pick_storage_folder(Callback::new(move |picked| {
                                    if let Some(target) = picked {
                                        crate::controller::relocate_storage(
                                            state, target, false, note, blocked, location,
                                        );
                                    }
                                }));
                            })
                        />
                    </Row>
                }
                    .into_any()
            }}
            {move || {
                blocked
                    .get()
                    .map(|target| {
                        let adopt = target.clone();
                        view! {
                            <Row label=t!("settings.storage.already-has-data") detail=target.clone()>
                                <Button
                                    label=t!("settings.storage.use-existing")
                                    on_click=Callback::new(move |_| {
                                        crate::controller::relocate_storage(
                                            state,
                                            adopt.clone(),
                                            true,
                                            note,
                                            blocked,
                                            location,
                                        )
                                    })
                                />
                            </Row>
                        }
                    })
            }}
            {move || note.get().map(|text| view! { <NoteRow text=text /> })}
        </Group>
    }
}
