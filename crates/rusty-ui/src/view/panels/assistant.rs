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
//!
//! **The panel is quiet.** An empty transcript is one line and the composer;
//! the paragraph about the tools, the four suggested questions and the row of
//! tool names that used to fill it were read once and then in the way of
//! every conversation after. The tools are listed in Settings.
//!
//! **The open file goes with the question**, as VS Code sends the active
//! editor: a chip above the input names it, its × drops it for this question,
//! and it travels as its own content block so the transcript shows the chip
//! and the model reads the file.

use leptos::{ev, html, prelude::*};

use rusty_ai::{Content, Message, Role};

use rusty_i18n::t;

use crate::{
    controller,
    state::{AppState, ToolRun},
    view::{
        SettingsOpen,
        components::{Button, ButtonKind, Dot, Empty, Pill, Tone},
        icon::{Icon, IconView},
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
    let SettingsOpen(settings_open) = expect_context::<SettingsOpen>();

    view! {
        <Empty
            title=t!("assistant.no-model-title")
            detail=t!("assistant.no-model-detail")
        >
            <Button
                label=t!("assistant.open-settings")
                kind=ButtonKind::Primary
                on_click=Callback::new(move |_| settings_open.set(true))
            />
        </Empty>
    }
}

#[component]
fn Transcript() -> impl IntoView {
    let state = AppState::expect();
    let SettingsOpen(settings_open) = expect_context::<SettingsOpen>();

    view! {
        <div class="min-h-0 flex-1 overflow-y-auto">
            {move || {
                let conversation = state.ai.conversation.get();
                if conversation.is_empty() && !state.ai.streaming.get() {
                    return view! {
                        <div class="flex h-full flex-col items-center justify-center gap-2 p-10 text-center">
                            <span class="text-label-4">
                                <IconView icon=Icon::Assistant size=22 />
                            </span>
                            <p class="max-w-[34ch] text-callout text-label-3">{t!("assistant.empty")}</p>
                        </div>
                    }
                        .into_any();
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

                        // The answer hit the output cap. Said here, under it,
                        // with the number and the way to raise it — a model
                        // that spent the whole budget thinking produced no
                        // text, and the transcript alone read as a question
                        // nobody answered.
                        {move || {
                            state.ai.cut_short.get().then(|| {
                                let max = state
                                    .ai
                                    .config
                                    .with(|config| config.as_ref().map_or(0, |c| c.max_tokens));
                                view! {
                                    <div class="flex max-w-[76ch] flex-wrap items-center gap-x-2 gap-y-1 rounded-[6px] bg-amber-fill px-2.5 py-1.5 text-caption leading-relaxed text-amber">
                                        <span>{t!("assistant.cut-short", max = max)}</span>
                                        <button
                                            type="button"
                                            on:click=move |_| settings_open.set(true)
                                            class="underline underline-offset-2 hover:text-label"
                                        >
                                            {t!("assistant.open-settings")}
                                        </button>
                                    </div>
                                }
                            })
                        }}

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
        let thinking = state.ai.thinking.get();
        let activity = state.ai.activity.get();

        view! {
            <div class="flex flex-col gap-2">
                <ToolActivity runs=activity />
                // Open while the reasoning streams and the answer has not
                // begun: the minute a reasoning model spends before its first
                // word used to be an empty drawer.
                {(!thinking.is_empty())
                    .then(|| {
                        let open = pending.is_empty();
                        view! { <Reasoning text=thinking open=open /> }
                    })}
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
                    {t!("assistant.thinking")}
                    // A way to stop. Without it the only way out of a slow
                    // or looping answer was to wait — up to eight tool calls
                    // — while the meter ran.
                    <button
                        type="button"
                        on:click=move |_| controller::cancel_ask(state)
                        class="rounded-[4px] px-1.5 py-0.5 text-footnote text-label-3 ring-1 ring-line hover:bg-sunken hover:text-label"
                    >
                        {t!("assistant.stop")}
                    </button>
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
                        None => (Tone::Rust, "…".to_string()),
                        Some(true) => (Tone::Patina, String::new()),
                        // A failed tool is not a failed answer — the model is
                        // told and usually recovers — but hiding it would leave
                        // the user unable to explain a thin answer.
                        Some(false) => (Tone::Crimson, t!("assistant.tool-failed")),
                    };
                    view! { <Pill label=format!("{}{suffix}", run.name) tone=tone /> }
                })
                .collect_view()}
        </div>
    }
    .into_any()
}

/// The last segment of a project-relative path: what a chip has room for.
fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
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
    // Files the user sent along, shown as what they are rather than as the
    // pages of text the model received.
    let attachments: Vec<String> = message
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Attachment { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();

    if is_user {
        return view! {
            <div class="flex flex-col items-end gap-1.5">
                <div class="max-w-[76ch] rounded-[10px] bg-selection px-3 py-2 text-body whitespace-pre-wrap select-text">
                    {text}
                </div>
                {(!attachments.is_empty())
                    .then(|| {
                        view! {
                            <div class="flex flex-wrap justify-end gap-1.5">
                                {attachments
                                    .into_iter()
                                    .map(|path| {
                                        view! {
                                            <span
                                                title=t!("assistant.attached", path = path.clone())
                                                class="inline-flex items-center gap-1 rounded-[6px] bg-sunken px-2 py-0.5 font-mono text-footnote text-label-3"
                                            >
                                                <IconView icon=Icon::Files size=11 />
                                                {file_name(&path)}
                                            </span>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })}
            </div>
        }
        .into_any();
    }

    // What the model thought before answering, when it streamed it. Folded:
    // the answer is what was asked for, and the thinking is there for the
    // reader who wants to know how it got there — or, when the budget ran out
    // before the answer began, what the money bought.
    let thinking: String = message
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Thinking { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();

    view! {
        <div class="flex flex-col gap-2">
            {(!thinking.is_empty()).then(|| view! { <Reasoning text=thinking open=false /> })}
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

/// The model's reasoning, folded under one line that names it. Open while it
/// streams — a model that thinks for a minute before its first word is then
/// visibly thinking rather than silent — and folded once the answer is
/// there. Dim and small, because it is not the answer.
#[component]
fn Reasoning(text: String, open: bool) -> impl IntoView {
    view! {
        <details
            open=open
            class="max-w-[76ch] rounded-[6px] bg-sunken px-2.5 py-1.5 text-caption text-label-3"
        >
            <summary class="cursor-default text-footnote text-label-4 select-none">
                {t!("assistant.reasoning")}
            </summary>
            <div class="mt-1 max-h-48 overflow-y-auto leading-relaxed whitespace-pre-wrap select-text">
                {text}
            </div>
        </details>
    }
}

#[component]
fn Composer() -> impl IntoView {
    let state = AppState::expect();
    let draft = RwSignal::new(String::new());
    let input: NodeRef<html::Textarea> = NodeRef::new();
    // Whether the open file rides along with the next question. On by
    // default, as VS Code attaches the active editor; the × on the chip
    // drops it for this question and the + brings it back.
    let attach = RwSignal::new(true);
    // The file in front of the user, in whichever group has the focus. A
    // binary is not context anyone can read.
    let open_file = Signal::derive(move || {
        let group = state.group(state.layout.focus.get());
        group
            .editor
            .document
            .get()
            .filter(|d| !d.binary)
            .map(|d| d.path)
    });

    let send = move || {
        let question = draft.get_untracked().trim().to_string();
        if question.is_empty() || state.ai.streaming.get_untracked() {
            return;
        }
        draft.set(String::new());
        if let Some(element) = input.get_untracked() {
            element.set_value("");
        }
        let context = if attach.get_untracked() {
            controller::open_file_context(state)
        } else {
            None
        };
        controller::ask(state, question, context);
    };

    view! {
        <div class="flex-none border-t border-line px-3 py-3">
            <div class="rounded-[10px] bg-sunken ring-1 ring-line transition-shadow focus-within:ring-rust">
                {move || {
                    open_file
                        .get()
                        .map(|path| {
                            let name = file_name(&path);
                            if attach.get() {
                                view! {
                                    <div class="flex items-center px-2 pt-2">
                                        <span
                                            title=t!("assistant.context-attached", path = path.clone())
                                            class="inline-flex h-[22px] items-center gap-1 rounded-[6px] bg-raised px-2 font-mono text-footnote text-label-2 ring-1 ring-line"
                                        >
                                            <IconView icon=Icon::Files size=11 />
                                            {name}
                                            <button
                                                type="button"
                                                title=t!("assistant.context-remove")
                                                on:click=move |_| attach.set(false)
                                                class="ml-0.5 rounded-[3px] px-0.5 leading-none text-label-3 hover:text-label"
                                            >
                                                "×"
                                            </button>
                                        </span>
                                    </div>
                                }
                                    .into_any()
                            } else {
                                view! {
                                    <div class="flex items-center px-2 pt-2">
                                        <button
                                            type="button"
                                            title=t!("assistant.context-add")
                                            on:click=move |_| attach.set(true)
                                            class="inline-flex h-[22px] items-center gap-1 rounded-[6px] px-2 font-mono text-footnote text-label-3 hover:bg-raised hover:text-label"
                                        >
                                            "+ "
                                            {name}
                                        </button>
                                    </div>
                                }
                                    .into_any()
                            }
                        })
                }}
                <textarea
                    node_ref=input
                    rows="1"
                    placeholder=t!("assistant.ask-placeholder")
                    class="max-h-[160px] min-h-[36px] w-full resize-none bg-transparent px-3 py-2 text-body outline-none placeholder:text-label-3"
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
                <div class="flex items-center gap-2 px-2 pb-2 text-footnote text-label-3">
                    {move || {
                        state
                            .ai.config
                            .get()
                            .map(|config| view! { <span class="font-mono">{config.model}</span> })
                    }}
                    {move || {
                        state
                            .ai.usage
                            .get()
                            .map(|(input, output)| {
                                view! {
                                    <span class="tnum">
                                        {t!("assistant.tokens", input = input.to_string(), output = output.to_string())}
                                    </span>
                                }
                            })
                    }}
                    <span class="flex-1" />
                    {move || {
                        (!state.ai.conversation.with(Vec::is_empty))
                            .then(|| {
                                view! {
                                    <Button
                                        label=t!("assistant.clear")
                                        kind=ButtonKind::Quiet
                                        on_click=Callback::new(move |_| controller::clear_conversation(state))
                                    />
                                }
                            })
                    }}
                    <Button
                        label=t!("assistant.ask")
                        kind=ButtonKind::Primary
                        disabled=Signal::derive(move || {
                            state.ai.streaming.get() || draft.with(|d| d.trim().is_empty())
                        })
                        on_click=Callback::new(move |_| send())
                    />
                </div>
            </div>
        </div>
    }
}
