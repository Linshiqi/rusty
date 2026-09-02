//! The model and the credentials for it.
//!
//! The largest category by some way, and the only one that can fail: a key is
//! stored in the OS credential store rather than here, a base URL can be
//! unreachable, and a model list has to be fetched. Everything else on this
//! screen applies the instant it is touched.

use leptos::prelude::*;

use rusty_ai::{ProviderConfig, ProviderKind};

use rusty_i18n::t;

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
            label=t!("settings.assistant.start-from")
            help=t!("settings.assistant.start-from-help")
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

        <Field label=t!("settings.assistant.endpoint")>
            <div class="flex flex-col gap-2">
                <TextRow
                    label=t!("settings.assistant.base-url")
                    value=Signal::derive(move || draft.with(|d| d.base_url.clone()))
                    on_input=Callback::new(move |v: String| draft.update(|d| d.base_url = v))
                />
                <TextRow
                    label=t!("settings.assistant.model")
                    value=Signal::derive(move || draft.with(|d| d.model.clone()))
                    on_input=Callback::new(move |v: String| draft.update(|d| d.model = v))
                />
                <TextRow
                    label=t!("settings.assistant.profile")
                    value=Signal::derive(move || draft.with(|d| d.profile.clone()))
                    on_input=Callback::new(move |v: String| draft.update(|d| d.profile = v))
                />
            </div>

            <div class="mt-2 flex items-center gap-2">
                <Button
                    label=t!("settings.assistant.list-models")
                    on_click=Callback::new(move |_| {
                        controller::list_models(state, draft.get_untracked(), models)
                    })
                />
                <span class="text-footnote text-label-3">
                    {t!("settings.assistant.list-models-note")}
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
            label=t!("settings.assistant.api-key")
            help=t!("settings.assistant.api-key-help")
        >
            <div class="flex items-center gap-2">
                <input
                    type="password"
                    placeholder=t!("settings.assistant.api-key-placeholder")
                    class="h-[28px] w-[320px] rounded-[6px] bg-sunken px-2.5 font-mono text-footnote outline-none ring-1 ring-line focus:ring-rust"
                    on:input=move |event| api_key.set(event_target_value(&event))
                />
                <Button
                    label=t!("settings.assistant.save-key")
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
                                <Pill label=t!("settings.assistant.stored") tone=Tone::Patina />
                                <Button
                                    label=t!("settings.assistant.remove")
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
                        view! { <Pill label=t!("settings.assistant.none-saved") tone=Tone::Neutral /> }.into_any()
                    }
                }}
            </div>
        </Field>

        <Field label=t!("settings.assistant.use-it")>
            <div class="flex items-center gap-2">
                <Button
                    label=t!("settings.assistant.save")
                    kind=ButtonKind::Primary
                    on_click=Callback::new(move |_| {
                        controller::set_provider(state, draft.get_untracked())
                    })
                />
                <Button
                    label=t!("settings.assistant.test")
                    title=t!("settings.assistant.test-help")
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
                            .map(|c| t!(
                                "settings.assistant.in-use",
                                model = c.model,
                                url = c.base_url,
                            ))
                            .unwrap_or_else(|| t!("settings.assistant.none-configured"))
                    }}
                </span>
            </div>
        </Field>

        <Field
            label=t!("settings.assistant.tools")
            help=t!("settings.assistant.tools-help")
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
