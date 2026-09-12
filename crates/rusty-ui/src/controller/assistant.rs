//! The assistant: provider configuration, keys, and the agent loop's events.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_ai::{
    AgentEvent, ChatEvent, Content, Message, Preset, ProviderCheck, ProviderConfig, StopReason,
    ToolDef,
};
use rusty_i18n::t;

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

/// File an API key in the OS credential store, and re-read whether one is
/// on file once the write has landed.
///
/// The re-read is here and not at the call site: the settings page used to
/// ask right after calling this, the two round trips raced, and "not saved"
/// arrived first and stayed — over a key the next request used perfectly
/// well. A flag that says the opposite of the store is the confident wrong
/// answer this project exists to avoid.
pub fn store_key(state: AppState, profile: String, api_key: String) {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        profile: String,
        api_key: String,
    }

    let args = Args {
        profile: profile.clone(),
        api_key,
    };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::ai::STORE_KEY, &args).await },
        move |()| refresh_key_state(state, profile),
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
///
/// The answer is facts — reached or not, the model listed or not — and the
/// settings screen words them. A failed request is an error and lands on the
/// banner like any other; it used to be read as an empty model list and the
/// empty list as success.
pub fn check_provider(
    state: AppState,
    config: ProviderConfig,
    into: RwSignal<Option<ProviderCheck>>,
) {
    #[derive(serde::Serialize)]
    struct Args {
        config: ProviderConfig,
    }

    let args = Args { config };
    into.set(None);
    track(
        state,
        async move { ipc::call::<_, ProviderCheck>(cmd::ai::CHECK_PROVIDER, &args).await },
        move |verdict| into.set(Some(verdict)),
    );
}

/// Stop the question in flight. The backend cancels the agent loop and
/// `ask` resolves; whatever text had streamed stays on screen.
pub fn cancel_ask(state: AppState) {
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::ai::CANCEL, &()).await },
        |()| {},
    );
}

/// Ask a question, streaming the answer. `context` is the file the user had
/// open, when they chose to send it along: its path and its text.
pub fn ask(state: AppState, question: String, context: Option<(String, String)>) {
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

    let mut content = vec![Content::Text { text: question }];
    if let Some((path, text)) = context {
        content.push(Content::Attachment { path, text });
    }
    state.ai.conversation.update(|c| {
        c.push(Message {
            role: rusty_ai::Role::User,
            content,
        })
    });
    state.ai.pending.set(String::new());
    state.ai.thinking.set(String::new());
    state.ai.cut_short.set(false);
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
            let outcome =
                ipc::call_streaming::<_, Vec<Message>>(cmd::ai::ASK, &args, "onEvent", &channel)
                    .await;
            let Err(error) = outcome else {
                return outcome;
            };
            // Any failure ends the streaming state, or the panel says
            // "thinking" over a question the backend has already given up.
            state.ai.streaming.set(false);
            // Stopped on purpose is not a fault: whatever had streamed stays
            // in the transcript, marked, and no banner is raised. The
            // backend names a stop with exactly this message.
            if error.message == cmd::ai::STOPPED {
                let mut history = state.ai.conversation.get_untracked();
                let thought = state.ai.thinking.get_untracked();
                let partial = state.ai.pending.get_untracked();
                let mut content = Vec::new();
                if !thought.is_empty() {
                    content.push(Content::Thinking { text: thought });
                }
                if !partial.is_empty() {
                    content.push(Content::Text {
                        text: format!("{partial}\n\n*{}*", t!("assistant.stopped")),
                    });
                }
                if !content.is_empty() {
                    history.push(Message::assistant(content));
                }
                return Ok(history);
            }
            Err(error)
        },
        move |history| {
            // The backend's history is authoritative — it contains the tool
            // calls and their results, which the stream only summarised. Keeping
            // the locally-accumulated text instead would send a transcript back
            // next turn that the model never actually produced.
            state.ai.conversation.set(history);
            state.ai.pending.set(String::new());
            state.ai.thinking.set(String::new());
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
        AgentEvent::Chat(ChatEvent::ThinkingDelta { text }) => {
            state
                .ai
                .thinking
                .update(|thinking| thinking.push_str(&text));
        }
        // The answer hit the output cap. The provider says so once, at the
        // end of the stream, and the history that comes back cannot carry it
        // — a model that spent its budget thinking produced no text to mark
        // — so the window keeps the fact and says it under the answer.
        AgentEvent::Chat(ChatEvent::Done {
            stop: StopReason::MaxTokens,
        }) => state.ai.cut_short.set(true),
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
    state.ai.thinking.set(String::new());
    state.ai.cut_short.set(false);
    state.ai.activity.set(Vec::new());
    state.ai.usage.set(None);
}

/// The file in front of the user, as the context sent with a question: its
/// path and its *draft* — what is on screen, unsaved edits included — cut
/// to a size a model can take. `None` when nothing is open, or what is open
/// is not text.
pub fn open_file_context(state: AppState) -> Option<(String, String)> {
    let group = state.focused();
    let document = group.editor.document.get_untracked()?;
    if document.binary {
        return None;
    }
    let text = group.editor.draft.get_untracked();
    Some((document.path, attachment_text(&text)))
}

/// The most of a file that goes along with a question. Sixty kilobytes is
/// about fifteen thousand tokens: a whole chapter or a long source file, and
/// short of the point where the file crowds out the question.
const ATTACHMENT_CAP: usize = 60_000;

/// A file's text as sent: whole when it fits, otherwise the first
/// `ATTACHMENT_CAP` bytes cut at a character boundary and marked as cut, so
/// the model knows it is reading the start of something and not all of it.
pub fn attachment_text(text: &str) -> String {
    if text.len() <= ATTACHMENT_CAP {
        return text.to_string();
    }
    let mut end = ATTACHMENT_CAP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let omitted = text.len() - end;
    format!("{}\n… [{omitted} bytes omitted]", &text[..end])
}

#[cfg(test)]
mod attachment_tests {
    use super::{ATTACHMENT_CAP, attachment_text};

    /// A short file goes whole; a long one is cut at a character boundary —
    /// the CJK case is the one that panics when it is not — and says so.
    #[test]
    fn a_long_file_is_cut_at_a_character_and_marked() {
        assert_eq!(attachment_text("fn main() {}"), "fn main() {}");
        let long: String = std::iter::repeat_n('中', ATTACHMENT_CAP).collect();
        let sent = attachment_text(&long);
        assert!(sent.len() < long.len());
        assert!(
            sent.contains("bytes omitted]"),
            "{}",
            &sent[sent.len() - 40..]
        );
        assert!(sent.starts_with("中中中"));
    }
}
