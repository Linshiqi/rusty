//! Your own model, with rusty's analyses as its tools.
//!
//! The reason this is not a chat box bolted onto an IDE: the assistant does not
//! read `.cargo/config.toml` and theorise about what is wrong. It calls
//! `project_status` and gets the actual mismatch, `memory_report` and gets the
//! actual byte counts. A model guessing at embedded configuration is the exact
//! failure this workbench exists to prevent, so the tools are the point and the
//! conversation is the interface to them.
//!
//! Which tools ran is shown, not hidden. An answer derived from a real
//! resolution and an answer invented from training data look identical in
//! prose, and the difference is the whole value.

use leptos::{ev, html, prelude::*};

use rusty_ai::{Content, Message, Role};

use crate::{
    controller,
    state::{AppState, ToolRun},
    view::{
        SettingsOpen,
        components::{Button, ButtonKind, Dot, Empty, Pill, Tone},
    },
};

#[component]
pub fn Assistant() -> impl IntoView {
    let state = AppState::expect();

    Effect::new(move |first: Option<()>| {
        if first.is_none() && state.ai.tools.with(Vec::is_empty) {
            controller::load_assistant(state);
        }
    });

    move || {
        if state.ai.config.with(Option::is_none) {
            return view! { <NotConfigured /> }.into_any();
        }

        view! {
            <div class="flex min-h-0 flex-1 flex-col">
                <Transcript />
                <Composer />
            </div>
        }
        .into_any()
    }
}

#[component]
fn NotConfigured() -> impl IntoView {
    let state = AppState::expect();
    let SettingsOpen(settings_open) = expect_context::<SettingsOpen>();

    view! {
        <Empty
            title="No model configured"
            detail="rusty brings no model of its own and no account. Point it at any endpoint you \
                    already have — a hosted API, or something running on this machine, in which \
                    case nothing leaves it."
        >
            <div class="mt-1 flex items-center gap-2">
                <Button
                    label="Open settings"
                    kind=ButtonKind::Primary
                    on_click=Callback::new(move |_| settings_open.set(true))
                />
            </div>
            <p class="mt-3 max-w-[52ch] text-callout text-label-3">
                {move || {
                    let count = state.ai.tools.with(Vec::len);
                    if count == 0 {
                        String::new()
                    } else {
                        format!(
                            "{count} of rusty's analyses are wired up as tools, so the model \
                             answers from your actual project rather than from memory.",
                        )
                    }
                }}
            </p>
        </Empty>
    }
}

#[component]
fn Transcript() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <div class="min-h-0 flex-1 overflow-y-auto">
            {move || {
                let conversation = state.ai.conversation.get();
                if conversation.is_empty() && !state.ai.streaming.get() {
                    return view! { <Suggestions /> }.into_any();
                }

                view! {
                    <div class="flex flex-col gap-4 px-4 py-4">
                        {conversation
                            .into_iter()
                            // Tool results are fed back to the model, not shown:
                            // they are a `memory_report` in full JSON, and the
                            // useful summary is the answer built from them.
                            .filter(|m| m.role != Role::Tool)
                            .map(|message| view! { <Bubble message=message /> })
                            .collect_view()}

                        <Streaming />
                    </div>
                }
                    .into_any()
            }}
        </div>
    }
}

/// What the model is doing right now.
#[component]
fn Streaming() -> impl IntoView {
    let state = AppState::expect();

    move || {
        if !state.ai.streaming.get() {
            return ().into_any();
        }

        let pending = state.ai.pending.get();
        let activity = state.ai.activity.get();

        view! {
            <div class="flex flex-col gap-2">
                <ToolActivity runs=activity />
                {(!pending.is_empty())
                    .then(|| {
                        view! {
                            <div class="max-w-[76ch] text-body select-text">
                                <crate::view::markdown::Markdown text=pending />
                            </div>
                        }
                    })}
                <div class="flex items-center gap-2 text-callout text-label-3">
                    <Dot tone=Tone::Rust />
                    "thinking"
                </div>
            </div>
        }
        .into_any()
    }
}

#[component]
fn ToolActivity(runs: Vec<ToolRun>) -> impl IntoView {
    if runs.is_empty() {
        return ().into_any();
    }

    view! {
        <div class="flex flex-wrap gap-1.5">
            {runs
                .into_iter()
                .map(|run| {
                    let (tone, suffix) = match run.ok {
                        None => (Tone::Rust, "…"),
                        Some(true) => (Tone::Patina, ""),
                        // A failed tool is not a failed answer — the model is
                        // told and usually recovers — but hiding it would leave
                        // the user unable to explain a thin answer.
                        Some(false) => (Tone::Crimson, " failed"),
                    };
                    view! { <Pill label=format!("{}{suffix}", run.name) tone=tone /> }
                })
                .collect_view()}
        </div>
    }
    .into_any()
}

#[component]
fn Bubble(message: Message) -> impl IntoView {
    let is_user = message.role == Role::User;
    let text = message.text();

    // Tool calls the model made in this message, so a completed answer still
    // shows what it was built from.
    let calls: Vec<String> = message
        .content
        .iter()
        .filter_map(|c| match c {
            Content::ToolUse { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();

    if is_user {
        return view! {
            <div class="flex justify-end">
                <div class="max-w-[76ch] rounded-[10px] bg-selection px-3 py-2 text-body whitespace-pre-wrap select-text">
                    {text}
                </div>
            </div>
        }
        .into_any();
    }

    view! {
        <div class="flex flex-col gap-2">
            {(!calls.is_empty())
                .then(|| {
                    view! {
                        <div class="flex flex-wrap gap-1.5">
                            {calls
                                .into_iter()
                                .map(|name| view! { <Pill label=name tone=Tone::Patina /> })
                                .collect_view()}
                        </div>
                    }
                })}
            {(!text.is_empty())
                .then(|| {
                    view! {
                        <div class="max-w-[76ch] text-body select-text">
                            <crate::view::markdown::Markdown text=text />
                        </div>
                    }
                })}
        </div>
    }
    .into_any()
}

/// Openers, chosen to show what having the tools changes.
///
/// Not decorative: an empty chat box invites the questions a general model
/// answers badly. These are the ones where calling `project_status` or
/// `memory_report` produces an answer nothing else can give.
#[component]
fn Suggestions() -> impl IntoView {
    let state = AppState::expect();

    const OPENERS: [&str; 4] = [
        "Why will this project not build?",
        "What is taking up the most flash, and what can I do about it?",
        "Which of my dependencies pull in duplicate versions, and who asked for them?",
        "Is my toolchain set up correctly for this chip?",
    ];

    view! {
        <div class="flex flex-1 flex-col items-center justify-center gap-4 p-10">
            <p class="max-w-[52ch] text-center text-body text-label-2">
                "The assistant runs rusty's own analyses rather than reading files and guessing. \
                 Ask it something it would otherwise get wrong."
            </p>
            <div class="flex max-w-[62ch] flex-col gap-1.5">
                {OPENERS
                    .into_iter()
                    .map(|opener| {
                        view! {
                            <button
                                type="button"
                                on:click=move |_| controller::ask(state, opener.to_string())
                                class="rounded-[8px] px-3 py-2 text-left text-callout text-label-2 ring-1 ring-line transition-colors hover:bg-sunken hover:text-label"
                            >
                                {opener}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
            {move || {
                let tools = state.ai.tools.get();
                (!tools.is_empty())
                    .then(|| {
                        view! {
                            <div class="mt-2 flex max-w-[62ch] flex-wrap justify-center gap-1.5">
                                {tools
                                    .into_iter()
                                    .map(|tool| {
                                        view! {
                                            <span
                                                title=tool.description.clone()
                                                class="rounded-full bg-sunken px-2 py-0.5 font-mono text-footnote text-label-3"
                                            >
                                                {tool.name}
                                            </span>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })
            }}
        </div>
    }
}

#[component]
fn Composer() -> impl IntoView {
    let state = AppState::expect();
    let draft = RwSignal::new(String::new());
    let input: NodeRef<html::Textarea> = NodeRef::new();

    let send = move || {
        let question = draft.get_untracked().trim().to_string();
        if question.is_empty() || state.ai.streaming.get_untracked() {
            return;
        }
        draft.set(String::new());
        if let Some(element) = input.get_untracked() {
            element.set_value("");
        }
        controller::ask(state, question);
    };

    view! {
        <div class="flex-none border-t border-line px-4 py-3">
            <div class="flex items-end gap-2">
                <textarea
                    node_ref=input
                    rows="1"
                    placeholder="Ask about this project…"
                    class="max-h-[160px] min-h-[34px] flex-1 resize-none rounded-[8px] bg-sunken px-3 py-2 text-body outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
                    on:input=move |event| draft.set(event_target_value(&event))
                    on:keydown=move |event: ev::KeyboardEvent| {
                        // Enter sends, Shift+Enter breaks the line. The reverse
                        // is a constant source of half-sent questions.
                        if event.key() == "Enter" && !event.shift_key() {
                            event.prevent_default();
                            send();
                        }
                    }
                />
                <Button
                    label="Ask"
                    kind=ButtonKind::Primary
                    disabled=Signal::derive(move || {
                        state.ai.streaming.get() || draft.with(|d| d.trim().is_empty())
                    })
                    on_click=Callback::new(move |_| send())
                />
            </div>

            <div class="mt-1.5 flex items-center gap-3 text-footnote text-label-3">
                {move || {
                    state
                        .ai.config
                        .get()
                        .map(|config| {
                            view! { <span class="font-mono">{config.model}</span> }
                        })
                }}
                {move || {
                    state
                        .ai.usage
                        .get()
                        .map(|(input, output)| {
                            view! {
                                <span class="tnum">{format!("{input} in / {output} out")}</span>
                            }
                        })
                }}
                <span class="flex-1" />
                {move || {
                    (!state.ai.conversation.with(Vec::is_empty))
                        .then(|| {
                            view! {
                                <button
                                    type="button"
                                    class="rounded-[5px] px-1.5 py-0.5 text-label-3 hover:text-label"
                                    on:click=move |_| controller::clear_conversation(state)
                                >
                                    "Clear"
                                </button>
                            }
                        })
                }}
            </div>
        </div>
    }
}
