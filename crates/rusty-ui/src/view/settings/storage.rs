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
        <Field
            label=t!("settings.storage.data-directory")
            help=t!("settings.storage.data-directory-help")
        >
            {move || {
                let Some(here) = location.get() else {
                    return view! { <p class="text-callout text-label-2">"…"</p> }.into_any();
                };
                let (badge, tone) = if here.env_override {
                    // The variable's own name: it is what to grep for.
                    ("RUSTY_CONFIG_DIR".to_string(), Tone::Amber)
                } else if here.is_default {
                    (t!("settings.storage.default"), Tone::Neutral)
                } else {
                    (t!("settings.storage.custom"), Tone::Patina)
                };
                let env_note = here.env_override;
                view! {
                    <div class="flex flex-wrap items-center gap-2">
                        <code class="rounded-[6px] bg-sunken px-2 py-1 font-mono text-footnote select-text">
                            {here.path.clone()}
                        </code>
                        <Pill label=badge tone=tone />
                    </div>
                    {env_note
                        .then(|| {
                            view! {
                                <p class="mt-2 text-footnote text-amber">
                                    {t!("settings.storage.env-override")}
                                </p>
                            }
                        })}
                }
                .into_any()
            }}
        </Field>

        <Field
            label=t!("settings.storage.move-it")
            help=t!("settings.storage.move-it-help")
        >
            <Button
                label=t!("settings.storage.choose")
                disabled=Signal::derive(move || {
                    location.get().is_some_and(|here| here.env_override)
                })
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
            {move || {
                blocked
                    .get()
                    .map(|target| {
                        let adopt = target.clone();
                        view! {
                            <div class="mt-2 max-w-[62ch] rounded-[8px] bg-amber-fill px-3 py-2">
                                <p class="text-callout leading-relaxed">
                                    {t!("settings.storage.already-has-data")}
                                </p>
                                <div class="mt-1.5">
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
                                </div>
                            </div>
                        }
                    })
            }}
            {move || {
                note.get()
                    .map(|text| {
                        view! {
                            <p class="mt-2 max-w-[62ch] text-footnote leading-relaxed text-label-2 select-text">
                                {text}
                            </p>
                        }
                    })
            }}
        </Field>
    }
}
