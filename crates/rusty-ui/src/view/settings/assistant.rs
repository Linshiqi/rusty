//! The model and the credentials for it.
//!
//! The largest category by some way, and the only one that can fail: a key is
//! stored in the OS credential store rather than here, a base URL can be
//! unreachable, and a model list has to be fetched. Everything else in
//! settings applies the instant it is touched; this page has a Save, because
//! the endpoint is edited as a draft and half a base URL must never be live.

use leptos::prelude::*;

use rusty_ai::{NotCheckedReason, ProviderCheck, ProviderConfig, ProviderKind};

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
    // the screen can know. Through a memo of the profile alone — reading the
    // whole draft here re-ran the check, a backend round trip, on every
    // keystroke in the base URL and model fields.
    let profile = Memo::new(move |_| draft.with(|d| d.profile.clone()));
    Effect::new(move |_| {
        crate::controller::refresh_key_state(state, profile.get());
    });
    let verdict = RwSignal::new(None::<rusty_ai::ProviderCheck>);
    let models = RwSignal::new(Vec::<String>::new());

    Effect::new(move |first: Option<()>| {
        if first.is_none() && state.ai.presets.with(Vec::is_empty) {
            controller::load_assistant(state);
        }
    });

    let field = |read: fn(&ProviderConfig) -> &String, write: fn(&mut ProviderConfig, String)| {
        (
            Signal::derive(move || draft.with(|d| read(d).clone())),
            Callback::new(move |value: String| draft.update(|d| write(d, value))),
        )
    };
    let (base_url, set_base_url) = field(|d| &d.base_url, |d, v| d.base_url = v);
    let (model, set_model) = field(|d| &d.model, |d, v| d.model = v);
    let (profile_name, set_profile) = field(|d| &d.profile, |d, v| d.profile = v);

    view! {
        <Group title=t!("settings.assistant.model")>
            <Row label=t!("settings.assistant.preset")>
                // A menu, as macOS offers a fixed handful of choices: picking
                // one only fills the fields below, and nothing is sent.
                <select
                    class="h-[26px] max-w-[300px] rounded-[6px] bg-sunken px-2 text-callout text-label outline-none ring-1 ring-line focus:ring-rust"
                    on:change=move |event| {
                        let label = event_target_value(&event);
                        let preset = state
                            .ai
                            .presets
                            .with_untracked(|presets| presets.iter().find(|p| p.label == label).cloned());
                        if let Some(preset) = preset {
                            draft.update(|d| {
                                d.kind = preset.kind;
                                d.base_url = preset.base_url.clone();
                                d.model = preset.suggested_model.clone();
                                d.profile = preset.label.to_lowercase();
                            });
                            verdict.set(None);
                            models.set(Vec::new());
                        }
                    }
                >
                    <option value="" selected=true disabled=true>
                        {t!("settings.assistant.preset-pick")}
                    </option>
                    {move || {
                        state
                            .ai
                            .presets
                            .get()
                            .into_iter()
                            .map(|preset| {
                                let text = if preset.local {
                                    format!("{} · {}", preset.label, t!("settings.assistant.local"))
                                } else {
                                    preset.label.clone()
                                };
                                view! { <option value=preset.label.clone()>{text}</option> }
                            })
                            .collect_view()
                    }}
                </select>
            </Row>
            <Row label=t!("settings.assistant.base-url")>
                <TextField value=base_url on_input=set_base_url placeholder="https://" />
            </Row>
            <Row label=t!("settings.assistant.model-name")>
                <TextField value=model on_input=set_model width="w-[220px]" />
                <Button
                    label=t!("settings.assistant.list-models")
                    kind=ButtonKind::Quiet
                    on_click=Callback::new(move |_| {
                        controller::list_models(state, draft.get_untracked(), models)
                    })
                />
            </Row>
            {move || {
                let found = models.get();
                (!found.is_empty())
                    .then(|| {
                        view! {
                            <Row label=t!("settings.assistant.models-found") stacked=true>
                                <div class="flex max-h-[120px] flex-wrap gap-1.5 overflow-y-auto">
                                    {found
                                        .into_iter()
                                        .map(|name| {
                                            let pick = name.clone();
                                            view! {
                                                <button
                                                    type="button"
                                                    on:click=move |_| draft.update(|d| d.model = pick.clone())
                                                    class="rounded-full bg-sunken px-2 py-0.5 font-mono text-footnote text-label-2 hover:text-label"
                                                >
                                                    {name}
                                                </button>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            </Row>
                        }
                    })
            }}
            <Row label=t!("settings.assistant.profile")>
                <TextField value=profile_name on_input=set_profile width="w-[220px]" />
            </Row>
        </Group>

        <Group title=t!("settings.assistant.key-group") footer=t!("settings.assistant.key-note")>
            <Row label=t!("settings.assistant.key")>
                <TextField
                    value=Signal::derive(move || api_key.get())
                    on_input=Callback::new(move |value: String| api_key.set(value))
                    placeholder=t!("settings.assistant.key-placeholder")
                    width="w-[240px]"
                    kind="password"
                />
                <Button
                    label=t!("settings.assistant.save-key")
                    disabled=Signal::derive(move || api_key.with(|k| k.trim().is_empty()))
                    on_click=Callback::new(move |_| {
                        // `store_key` refreshes the stored flag when the write
                        // has landed; asking here as well raced it, and the
                        // answer that arrived first was "not saved".
                        controller::store_key(
                            state,
                            draft.get_untracked().profile,
                            api_key.get_untracked(),
                        );
                        api_key.set(String::new());
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
                                    kind=ButtonKind::Quiet
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
            </Row>
        </Group>

        <Group title=t!("settings.assistant.enable")>
            <Row
                label=t!("settings.assistant.current")
                detail=Signal::derive(move || {
                    state
                        .ai
                        .config
                        .get()
                        .map(|c| t!("settings.assistant.in-use", model = c.model, url = c.base_url))
                        .unwrap_or_else(|| t!("settings.assistant.none-configured"))
                })
            >
                <Button
                    label=t!("settings.assistant.test")
                    on_click=Callback::new(move |_| {
                        controller::check_provider(state, draft.get_untracked(), verdict)
                    })
                />
                <Button
                    label=t!("settings.assistant.save")
                    kind=ButtonKind::Primary
                    on_click=Callback::new(move |_| {
                        controller::set_provider(state, draft.get_untracked())
                    })
                />
            </Row>
            {move || {
                verdict
                    .get()
                    .map(|check| {
                        let (tone, text) = describe_check(&check);
                        view! {
                            <div class="flex items-center gap-2 px-3.5 py-2 text-callout text-label-2">
                                <Dot tone=tone />
                                <span class="select-text">{text}</span>
                            </div>
                        }
                    })
            }}
        </Group>

        <Group title=t!("settings.assistant.tools") footer=t!("settings.assistant.tools-note")>
            <div class="flex flex-wrap gap-1.5 px-3.5 py-2.5">
                {move || {
                    state
                        .ai
                        .tools
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
        </Group>
    }
}

/// The connectivity check's facts as a sentence, and the colour they earn.
///
/// Worded here rather than on the backend so the sentence is in the window's
/// language — and so a check that stopped short is amber, not the green a
/// single "Reachable" string used to paint over every outcome.
fn describe_check(check: &ProviderCheck) -> (Tone, String) {
    match check {
        ProviderCheck::Reachable {
            model,
            model_listed: Some(true),
            ..
        } => (
            Tone::Patina,
            t!(
                "settings.assistant.check-model-listed",
                model = model.clone()
            ),
        ),
        ProviderCheck::Reachable {
            model,
            models_listed,
            model_listed: Some(false),
        } => (
            Tone::Amber,
            t!(
                "settings.assistant.check-model-missing",
                model = model.clone(),
                count = models_listed.to_string()
            ),
        ),
        ProviderCheck::Reachable { model, .. } => (
            Tone::Patina,
            t!("settings.assistant.check-reachable", model = model.clone()),
        ),
        ProviderCheck::NotChecked {
            why: NotCheckedReason::NoModelListing { status },
            ..
        } => (
            Tone::Amber,
            t!(
                "settings.assistant.check-no-listing",
                status = status.to_string()
            ),
        ),
    }
}
