//! The assistant: provider configuration, keys, and the agent loop's events.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_ai::{AgentEvent, ChatEvent, Message, Preset, ProviderConfig, ToolDef};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{AppState, ToolRun, carried_provider},
};

/// Presets and the tool list. Both static, so once per session.
pub fn load_assistant(state: AppState) {
    track(
        state,
        ipc::get::<Vec<Preset>>(cmd::ai::PRESETS),
        move |presets| state.ai.presets.set(presets),
    );
    track(
        state,
        ipc::get::<Vec<ToolDef>>(cmd::ai::TOOLS),
        move |tools| state.ai.tools.set(tools),
    );
}

/// Save the provider profile. The key is handled separately and never comes back.
pub fn set_provider(state: AppState, config: ProviderConfig) {
    #[derive(serde::Serialize)]
    struct Args {
        choice: rusty_embed::AssistantChoice,
    }
    let args = Args {
        choice: to_choice(&config),
    };
    state.ai.config.set(Some(config));
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::workbench::SET_ASSISTANT, &args).await;
    });
}

/// The profile last chosen, from the file — falling back to whatever this
/// window still holds from before it was one, which is then written through
/// so the next launch reads it from the file like everything else.
pub fn load_provider(state: AppState) {
    spawn_local(async move {
        let stored =
            ipc::call::<_, Option<rusty_embed::AssistantChoice>>(cmd::workbench::ASSISTANT, &())
                .await
                .ok()
                .flatten();
        match stored.and_then(|choice| from_choice(&choice)) {
            Some(config) => state.ai.config.set(Some(config)),
            None => {
                if let Some(config) = carried_provider() {
                    set_provider(state, config);
                }
            }
        }
    });
}

/// The file's shape and the frontend's are separate types on purpose — the
/// same rule that keeps the chip catalogue's file format out of `model`.
fn to_choice(config: &ProviderConfig) -> rusty_embed::AssistantChoice {
    rusty_embed::AssistantChoice {
        profile: config.profile.clone(),
        kind: serde_json::to_value(config.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        base_url: config.base_url.clone(),
        model: config.model.clone(),
        max_tokens: Some(config.max_tokens),
        temperature: config.temperature,
        supports_tools: Some(config.supports_tools),
    }
}

fn from_choice(choice: &rusty_embed::AssistantChoice) -> Option<ProviderConfig> {
    Some(ProviderConfig {
        profile: choice.profile.clone(),
        kind: serde_json::from_value(serde_json::Value::String(choice.kind.clone())).ok()?,
        base_url: choice.base_url.clone(),
        model: choice.model.clone(),
        max_tokens: choice.max_tokens.unwrap_or(4096),
        temperature: choice.temperature,
        supports_tools: choice.supports_tools.unwrap_or(true),
    })
}

/// File an API key in the OS credential store.
pub fn store_key(state: AppState, profile: String, api_key: String) {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        profile: String,
        api_key: String,
    }

    let args = Args { profile, api_key };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::ai::STORE_KEY, &args).await },
        move |()| {},
    );
}

/// Ask the endpoint which models it serves.
///
/// Discovered rather than hardcoded: model names drift far faster than a
/// release cycle, and a self-hosted server's names are unknowable in advance.
pub fn list_models(state: AppState, config: ProviderConfig, into: RwSignal<Vec<String>>) {
    #[derive(serde::Serialize)]
    struct Args {
        config: ProviderConfig,
    }

    let args = Args { config };
    track(
        state,
        async move { ipc::call::<_, Vec<String>>(cmd::ai::LIST_MODELS, &args).await },
        move |models| into.set(models),
    );
}

/// Check a profile end to end without starting a conversation.
pub fn check_provider(state: AppState, config: ProviderConfig, into: RwSignal<Option<String>>) {
    #[derive(serde::Serialize)]
    struct Args {
        config: ProviderConfig,
    }

    let args = Args { config };
    track(
        state,
        async move { ipc::call::<_, String>(cmd::ai::CHECK_PROVIDER, &args).await },
        move |verdict| into.set(Some(verdict)),
    );
}

/// Ask a question, streaming the answer.
pub fn ask(state: AppState, question: String) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    let Some(config) = state.ai.config.get_untracked() else {
        return;
    };

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        config: ProviderConfig,
        history: Vec<Message>,
    }

    state
        .ai
        .conversation
        .update(|c| c.push(Message::user(question)));
    state.ai.pending.set(String::new());
    state.ai.activity.set(Vec::new());
    state.ai.usage.set(None);
    state.ai.streaming.set(true);

    let channel = ipc::Channel::new();
    let on_event = Closure::wrap(Box::new(move |value: JsValue| {
        if let Ok(event) = serde_wasm_bindgen::from_value::<AgentEvent>(value) {
            apply_event(state, event);
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_event);
    // Held by the backend for the length of the answer, which outlives this
    // call. One per question asked.
    on_event.forget();

    let args = Args {
        config,
        history: state.ai.conversation.get_untracked(),
    };
    track(
        state,
        async move {
            ipc::call_streaming::<_, Vec<Message>>(cmd::ai::ASK, &args, "onEvent", &channel).await
        },
        move |history| {
            // The backend's history is authoritative — it contains the tool
            // calls and their results, which the stream only summarised. Keeping
            // the locally-accumulated text instead would send a transcript back
            // next turn that the model never actually produced.
            state.ai.conversation.set(history);
            state.ai.pending.set(String::new());
            state.ai.streaming.set(false);
        },
    );
}

/// Fold one streamed event into the visible state.
fn apply_event(state: AppState, event: AgentEvent) {
    match event {
        AgentEvent::Chat(ChatEvent::TextDelta { text }) => {
            state.ai.pending.update(|pending| pending.push_str(&text));
        }
        AgentEvent::Chat(ChatEvent::Usage {
            input_tokens,
            output_tokens,
        }) => state.ai.usage.set(Some((input_tokens, output_tokens))),
        AgentEvent::ToolStarted { id, name, .. } => {
            state
                .ai
                .activity
                .update(|runs| runs.push(ToolRun { id, name, ok: None }));
        }
        AgentEvent::ToolFinished { id, ok, .. } => {
            state.ai.activity.update(|runs| {
                if let Some(run) = runs.iter_mut().find(|r| r.id == id) {
                    run.ok = Some(ok);
                }
            });
        }
        // The provider-level tool-call events restate what `ToolStarted` and
        // `ToolFinished` already say, but without the agent loop's knowledge of
        // whether the call succeeded. Rendering both would double every row.
        AgentEvent::Chat(_) => {}
    }
}

/// Throw away the transcript.
pub fn clear_conversation(state: AppState) {
    state.ai.conversation.set(Vec::new());
    state.ai.pending.set(String::new());
    state.ai.activity.set(Vec::new());
    state.ai.usage.set(None);
}
