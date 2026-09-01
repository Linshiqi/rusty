//! The model and the credentials for it.
//!
//! The largest category by some way, and the only one that can fail: a key is
//! stored in the OS credential store rather than here, a base URL can be
//! unreachable, and a model list has to be fetched. Everything else on this
//! screen applies the instant it is touched.

use leptos::prelude::*;

use rusty_ai::{ProviderConfig, ProviderKind};

use crate::{
    controller,
    state::AppState,
    view::components::{Button, ButtonKind, Dot, Pill, Tone},
};

use super::*;

/// Bring-your-own-model configuration.
///
/// The key field is write-only by construction: it is sent to the OS credential
/// store and never read back, so this screen has no way to display a secret and
/// nothing to leak if the WebView is inspected. `ai_key_configured` answers
/// "is one on file" with a boolean, which is all the UI needs to know.
#[component]
pub(super) fn Assistant() -> impl IntoView {
    let state = AppState::expect();

    // Local to the form: edits are not applied until Save, so abandoning a
    // half-typed base URL cannot leave the assistant pointed at nothing.
    let draft = RwSignal::new(
        state
            .ai
            .config
            .get_untracked()
            .unwrap_or_else(|| ProviderConfig {
                profile: "default".to_string(),
                kind: ProviderKind::OpenAiCompatible,
                base_url: String::new(),
                model: String::new(),
                max_tokens: 4096,
                temperature: None,
                supports_tools: true,
            }),
    );
    let api_key = RwSignal::new(String::new());

    // The stored-key check, refreshed whenever the profile changes: the whole
    // point of the write-only design is that this boolean is the only thing
    // the screen can know.
    Effect::new(move |_| {
        let profile = draft.with(|d| d.profile.clone());
        crate::controller::refresh_key_state(state, profile);
    });
    let verdict = RwSignal::new(None::<String>);
    let models = RwSignal::new(Vec::<String>::new());

    Effect::new(move |first: Option<()>| {
        if first.is_none() && state.ai.presets.with(Vec::is_empty) {
            controller::load_assistant(state);
        }
    });

    view! {
        <Field
            label="Start from"
            help="Local runtimes are first-class: nothing leaves the machine and no key is \
                  needed. Picking one only fills the fields below — nothing is sent."
        >
            <div class="flex flex-wrap gap-1.5">
                {move || {
                    state
                        .ai.presets
                        .get()
                        .into_iter()
                        .map(|preset| {
                            let label = preset.label.clone();
                            view! {
                                <button
                                    type="button"
                                    on:click=move |_| {
                                        draft
                                            .update(|d| {
                                                d.kind = preset.kind;
                                                d.base_url = preset.base_url.clone();
                                                d.model = preset.suggested_model.clone();
                                                d.profile = preset.label.to_lowercase();
                                            });
                                        verdict.set(None);
                                        models.set(Vec::new());
                                    }
                                    class="rounded-[6px] px-2 py-1 text-callout text-label-2 ring-1 ring-line transition-colors hover:bg-sunken hover:text-label"
                                >
                                    {label}
                                    {preset
                                        .local
                                        .then(|| {
                                            view! {
                                                <span class="ml-1.5 text-footnote text-patina">
                                                    "local"
                                                </span>
                                            }
                                        })}
                                </button>
                            }
                        })
                        .collect_view()
                }}
            </div>
        </Field>

        <Field label="Endpoint">
            <div class="flex flex-col gap-2">
                <TextRow
                    label="Base URL"
                    value=Signal::derive(move || draft.with(|d| d.base_url.clone()))
                    on_input=Callback::new(move |v: String| draft.update(|d| d.base_url = v))
                />
                <TextRow
                    label="Model"
                    value=Signal::derive(move || draft.with(|d| d.model.clone()))
                    on_input=Callback::new(move |v: String| draft.update(|d| d.model = v))
                />
                <TextRow
                    label="Profile"
                    value=Signal::derive(move || draft.with(|d| d.profile.clone()))
                    on_input=Callback::new(move |v: String| draft.update(|d| d.profile = v))
                />
            </div>

            <div class="mt-2 flex items-center gap-2">
                <Button
                    label="List models"
                    on_click=Callback::new(move |_| {
                        controller::list_models(state, draft.get_untracked(), models)
                    })
                />
                <span class="text-footnote text-label-3">
                    "Names drift faster than any list rusty could ship."
                </span>
            </div>

            {move || {
                let found = models.get();
                (!found.is_empty())
                    .then(|| {
                        view! {
                            <div class="mt-2 flex max-h-[132px] flex-wrap gap-1.5 overflow-y-auto">
                                {found
                                    .into_iter()
                                    .map(|name| {
                                        let pick = name.clone();
                                        view! {
                                            <button
                                                type="button"
                                                on:click=move |_| {
                                                    draft.update(|d| d.model = pick.clone())
                                                }
                                                class="rounded-full bg-sunken px-2 py-0.5 font-mono text-footnote text-label-2 hover:text-label"
                                            >
                                                {name}
                                            </button>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })
            }}
        </Field>

        <Field
            label="API key"
            help="Stored in the operating system's credential store and read only by the Rust \
                  side at the moment of a request. It is never sent back to this window, which is \
                  why there is nothing here to show you once it is saved."
        >
            <div class="flex items-center gap-2">
                <input
                    type="password"
                    placeholder="paste, save, and it leaves this window"
                    class="h-[28px] w-[320px] rounded-[6px] bg-sunken px-2.5 font-mono text-footnote outline-none ring-1 ring-line focus:ring-rust"
                    on:input=move |event| api_key.set(event_target_value(&event))
                />
                <Button
                    label="Save key"
                    disabled=Signal::derive(move || api_key.with(|k| k.trim().is_empty()))
                    on_click=Callback::new(move |_| {
                        controller::store_key(
                            state,
                            draft.get_untracked().profile,
                            api_key.get_untracked(),
                        );
                        api_key.set(String::new());
                        crate::controller::refresh_key_state(
                            state,
                            draft.get_untracked().profile,
                        );
                    })
                />
                // The one thing this screen is allowed to know about the key.
                {move || {
                    if state.ai.key_stored.get() {
                        view! {
                            <>
                                <Pill label="stored" tone=Tone::Patina />
                                <Button
                                    label="Remove"
                                    on_click=Callback::new(move |_| {
                                        crate::controller::delete_key(
                                            state,
                                            draft.get_untracked().profile,
                                        )
                                    })
                                />
                            </>
                        }
                            .into_any()
                    } else {
                        view! { <Pill label="none saved" tone=Tone::Neutral /> }.into_any()
                    }
                }}
            </div>
        </Field>

        <Field label="Use it">
            <div class="flex items-center gap-2">
                <Button
                    label="Save"
                    kind=ButtonKind::Primary
                    on_click=Callback::new(move |_| {
                        controller::set_provider(state, draft.get_untracked())
                    })
                />
                <Button
                    label="Test"
                    title="Check the endpoint, the key and the model name before a conversation \
                           depends on them"
                    on_click=Callback::new(move |_| {
                        controller::check_provider(state, draft.get_untracked(), verdict)
                    })
                />
                {move || {
                    verdict
                        .get()
                        .map(|text| {
                            view! {
                                <span class="flex items-center gap-2 text-callout text-label-2">
                                    <Dot tone=Tone::Patina />
                                    {text}
                                </span>
                            }
                        })
                }}
            </div>

            <div class="mt-2 flex items-center gap-2">
                <Dot tone=Signal::derive(move || {
                    if state.ai.config.with(Option::is_some) { Tone::Patina } else { Tone::Neutral }
                })
                    .get() />
                <span class="text-callout text-label-2">
                    {move || {
                        state
                            .ai.config
                            .get()
                            .map(|c| format!("in use: {} at {}", c.model, c.base_url))
                            .unwrap_or_else(|| "None configured".to_string())
                    }}
                </span>
            </div>
        </Field>

        <Field
            label="What it can call"
            help="The assistant does not read your configuration files and theorise; it calls \
                  these and gets the actual answer. Each one reports what it is missing rather \
                  than guessing, so a wrong answer is a bug and not a coin flip."
        >
            <div class="flex flex-wrap gap-1.5">
                {move || {
                    state
                        .ai.tools
                        .get()
                        .into_iter()
                        .map(|tool| {
                            view! {
                                <span
                                    title=tool.description.clone()
                                    class="rounded-full bg-sunken px-2 py-0.5 font-mono text-footnote text-label-2"
                                >
                                    {tool.name}
                                </span>
                            }
                        })
                        .collect_view()
                }}
            </div>
        </Field>
    }
}
