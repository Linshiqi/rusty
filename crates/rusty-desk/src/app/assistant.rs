//! The assistant: the right dock's conversation and Settings' model.
//!
//! A question runs `rusty_ai`'s agent loop on a thread of its own, with a
//! runtime of its own, and every event it streams comes back to the window
//! as it happens; the history it hands back when it is done is the
//! transcript from then on, because it holds the tool calls and their
//! results that the stream only summarised. One question at a time, and it
//! can be stopped: whatever had streamed stays, marked.
//!
//! The assistant gets the workbench's read-only tools (`ToolRegistry::
//! workbench`) — nobody is asked before one runs, so none of them writes —
//! and the file in front goes with a question unless the chip says not to.
//! The key never enters the window: it goes from the field into the system's
//! credential store, and is read from there when a request is made.

use std::{collections::HashMap, rc::Rc, sync::Arc, time::Duration};

use rusty_ai::{
    AgentEvent, Assistant as Agent, ChatEvent, Content, LazyWorkspace, Message, ProviderCheck,
    ProviderConfig, ProviderKind, Role, StopReason, ToolContext, ToolRegistry, secrets,
};
use rusty_embed::AssistantChoice;
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{App, Tab, markdown_view};
use crate::{AiEntry, Shell, work};

/// Longest one question may run, tool rounds included: generous, and finite.
const BUDGET: Duration = Duration::from_secs(20 * 60);

/// A tool the loop called, as the stream reported it.
#[derive(Clone, Debug)]
struct ToolRun {
    id: String,
    name: String,
    ok: Option<bool>,
}

/// The question in flight.
#[derive(Default)]
struct Streaming {
    text: String,
    thinking: String,
    tools: Vec<ToolRun>,
}

/// The question's cost: every round summed, the cached share beside it.
#[derive(Clone, Copy, Debug, Default)]
struct Spend {
    input: u64,
    cached: u64,
    output: u64,
    rounds: u32,
}

#[derive(Default)]
pub(super) struct Assistant {
    history: Vec<Message>,
    streaming: Option<Streaming>,
    /// Counts questions, so an event from one already stopped is dropped.
    ticket: u64,
    stop: Option<Arc<tokio::sync::Notify>>,
    spend: Option<Spend>,
    cut_short: Option<u32>,
    problem: Option<String>,
    /// The file in front does not go with the next question.
    detached: bool,
    /// Thinking blocks unfolded, by their place in the transcript.
    unfolded: Vec<usize>,
    /// The output cap a provider named when it refused the budget, kept for
    /// the session so it is refused once and not once per question.
    caps: HashMap<String, u32>,
    redraw_due: bool,
    // Settings.
    key_stored: bool,
    models: Vec<String>,
    check: Option<(bool, String)>,
}

/// The choice stored in `workbench.toml`, as a provider configuration.
fn configured() -> Option<ProviderConfig> {
    let choice = rusty_embed::config::workbench().assistant?;
    Some(ProviderConfig {
        profile: choice.profile.clone(),
        kind: match choice.kind.as_str() {
            "anthropic" => ProviderKind::Anthropic,
            _ => ProviderKind::OpenAiCompatible,
        },
        base_url: choice.base_url.clone(),
        model: choice.model.clone(),
        // 4096 was the default for every profile written before the setting
        // existed, and nothing could set it otherwise: it reads as the new one.
        max_tokens: choice
            .max_tokens
            .filter(|&max| max != 4096)
            .unwrap_or(rusty_ai::DEFAULT_MAX_TOKENS),
        temperature: choice.temperature,
        supports_tools: choice.supports_tools.unwrap_or(true),
    })
    .filter(|c| !c.base_url.is_empty() && !c.model.is_empty())
}

fn kind_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::OpenAiCompatible => "openAiCompatible",
    }
}

/// A number of tokens, short: 18.4k.
fn tokens(n: u64) -> String {
    if n >= 10_000 {
        format!("{:.0}k", n as f64 / 1000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

impl App {
    pub(super) fn ai_act(self: &Rc<Self>, verb: &str) {
        match verb {
            "shown" => {
                self.ai_settings_read();
                self.sync_assistant();
            }
            "ask" => self.ai_ask(),
            "stop" => self.ai_stop(),
            "clear" => {
                if self.state.borrow().assistant.streaming.is_some() {
                    return;
                }
                {
                    let mut state = self.state.borrow_mut();
                    let ai = &mut state.assistant;
                    ai.history.clear();
                    ai.spend = None;
                    ai.cut_short = None;
                    ai.problem = None;
                    ai.unfolded.clear();
                }
                self.sync_assistant();
            }
            "attach" => {
                {
                    let mut state = self.state.borrow_mut();
                    state.assistant.detached = !state.assistant.detached;
                }
                self.sync_assistant();
            }
            "settings" => self.open_tab(Tab::Page("settings")),
            "save" => self.ai_save(),
            "list-models" => self.ai_list_models(),
            "check" => self.ai_check(),
            "save-key" => self.ai_save_key(),
            "remove-key" => self.ai_remove_key(),
            _ => {}
        }
    }

    // ---- asking ------------------------------------------------------------

    fn ai_ask(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let question = shell.get_ai_draft().trim().to_string();
        if question.is_empty() || self.state.borrow().assistant.streaming.is_some() {
            return;
        }
        let Some(config) = configured() else {
            self.open_tab(Tab::Page("settings"));
            return;
        };
        let mut content = vec![Content::Text { text: question }];
        if !self.state.borrow().assistant.detached
            && let Some((path, text, focus)) = self.editor.attachment()
        {
            content.push(Content::attach(path, &text, Some(focus)));
        }
        let stop = Arc::new(tokio::sync::Notify::new());
        let (ticket, history, budget) = {
            let mut state = self.state.borrow_mut();
            let ai = &mut state.assistant;
            ai.history.push(Message {
                role: Role::User,
                content,
            });
            ai.ticket += 1;
            ai.streaming = Some(Streaming::default());
            ai.stop = Some(stop.clone());
            ai.spend = None;
            ai.cut_short = None;
            ai.problem = None;
            let key = cap_key(&config);
            let budget = match ai.caps.get(&key) {
                Some(cap) => (*cap).min(config.max_tokens),
                None => config.max_tokens,
            };
            (ai.ticket, ai.history.clone(), budget)
        };
        shell.set_ai_draft(SharedString::new());
        self.sync_assistant();
        let root = self.root();
        let workspace = self.crates_workspace();
        // What a running simulation says, frozen for the answer: a sheet
        // the Math page shows live is worked out with the same numbers.
        let live = Some(self.live_now()).filter(|l| !l.channels.is_empty() || l.truth.is_some());
        std::thread::spawn(move || {
            let question = Question {
                config,
                history,
                budget,
                root,
                workspace,
                live,
            };
            let answer = run_question(question, stop, ticket);
            work::deliver(answer, move |app, answer| app.ai_done(ticket, answer));
        });
    }

    fn ai_stop(self: &Rc<Self>) {
        if let Some(stop) = self.state.borrow().assistant.stop.as_ref() {
            stop.notify_one();
        }
    }

    /// One streamed event, folded into what the drawer shows.
    fn ai_event(self: &Rc<Self>, ticket: u64, event: AgentEvent) {
        {
            let mut state = self.state.borrow_mut();
            let ai = &mut state.assistant;
            if ai.ticket != ticket {
                return;
            }
            let Some(streaming) = ai.streaming.as_mut() else {
                return;
            };
            match event {
                AgentEvent::Chat(ChatEvent::TextDelta { text }) => streaming.text.push_str(&text),
                AgentEvent::Chat(ChatEvent::ThinkingDelta { text }) => {
                    streaming.thinking.push_str(&text)
                }
                AgentEvent::Chat(ChatEvent::Usage {
                    input_tokens,
                    cached_tokens,
                    output_tokens,
                }) => {
                    let spend = ai.spend.get_or_insert_default();
                    spend.input += u64::from(input_tokens);
                    spend.cached += u64::from(cached_tokens);
                    spend.output += u64::from(output_tokens);
                    spend.rounds += 1;
                }
                AgentEvent::Chat(ChatEvent::Done {
                    stop: StopReason::MaxTokens,
                }) => {
                    ai.cut_short = configured().map(|c| c.max_tokens);
                }
                AgentEvent::ToolStarted { id, name, .. } => {
                    // A tool's text so far is a paragraph of the answer, and
                    // the next round's text starts after it.
                    streaming.tools.push(ToolRun { id, name, ok: None });
                }
                AgentEvent::ToolFinished { id, ok, .. } => {
                    if let Some(run) = streaming.tools.iter_mut().find(|r| r.id == id) {
                        run.ok = Some(ok);
                    }
                }
                AgentEvent::Chat(_) => {}
            }
            if ai.redraw_due {
                return;
            }
            ai.redraw_due = true;
        }
        // Many deltas a frame: drawn once a frame.
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::from_millis(40), move || {
            if let Some(app) = weak.upgrade() {
                app.state.borrow_mut().assistant.redraw_due = false;
                app.sync_assistant();
            }
        });
    }

    fn ai_done(self: &Rc<Self>, ticket: u64, answer: Answer) {
        {
            let mut state = self.state.borrow_mut();
            let ai = &mut state.assistant;
            if ai.ticket != ticket {
                return;
            }
            let streaming = ai.streaming.take().unwrap_or_default();
            ai.stop = None;
            if let Some((key, cap)) = answer.cap {
                ai.caps.insert(key, cap);
            }
            match answer.outcome {
                // The loop's history is the transcript: it holds the calls
                // and their results that the stream only summarised.
                Outcome::Done(history) => ai.history = history,
                // Stopped on purpose is not a fault: what had streamed stays,
                // marked, and nothing is raised.
                Outcome::Stopped => {
                    let mut content = Vec::new();
                    if !streaming.thinking.is_empty() {
                        content.push(Content::Thinking {
                            text: streaming.thinking,
                        });
                    }
                    let text = if streaming.text.is_empty() {
                        format!("*{}*", t!("assistant.stopped"))
                    } else {
                        format!("{}\n\n*{}*", streaming.text, t!("assistant.stopped"))
                    };
                    content.push(Content::Text { text });
                    ai.history.push(Message::assistant(content));
                }
                Outcome::Failed(why) => ai.problem = Some(why),
            }
        }
        if let Some(workspace) = answer.workspace {
            self.crates_keep_workspace(workspace);
        }
        self.sync_assistant();
    }

    // ---- settings ------------------------------------------------------------

    /// The stored choice into the fields, and whether its profile has a key.
    fn ai_settings_read(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let presets = rusty_ai::presets();
        // The first row is no preset: a profile of somebody's own is not
        // one of these, and the box must not claim it is.
        shell.set_ai_presets(ModelRc::new(VecModel::from(
            std::iter::once(SharedString::from(t!("settings.assistant.preset-pick")))
                .chain(presets.iter().map(|p| {
                    SharedString::from(if p.local {
                        format!("{} · {}", p.label, t!("settings.assistant.local"))
                    } else {
                        p.label.clone()
                    })
                }))
                .collect::<Vec<_>>(),
        )));
        let choice = rusty_embed::config::workbench().assistant;
        if let Some(choice) = &choice {
            shell.set_ai_profile(choice.profile.as_str().into());
            shell.set_ai_kind(i32::from(choice.kind == "anthropic"));
            shell.set_ai_base_url(choice.base_url.as_str().into());
            shell.set_ai_model(choice.model.as_str().into());
            shell.set_ai_max_tokens(
                choice
                    .max_tokens
                    .filter(|&m| m != 4096)
                    .unwrap_or(rusty_ai::DEFAULT_MAX_TOKENS)
                    .to_string()
                    .into(),
            );
        } else if shell.get_ai_max_tokens().is_empty() {
            shell.set_ai_max_tokens(rusty_ai::DEFAULT_MAX_TOKENS.to_string().into());
        }
        let profile = shell.get_ai_profile().to_string();
        self.ai_key_state(profile);
    }

    /// A preset picked: its endpoint, dialect and suggested model in the
    /// fields, nothing saved until Save.
    pub(super) fn ai_preset(self: &Rc<Self>, index: i32) {
        let Some(preset) = usize::try_from(index - 1)
            .ok()
            .and_then(|at| rusty_ai::presets().into_iter().nth(at))
        else {
            return;
        };
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        shell.set_ai_profile(preset.label.as_str().into());
        shell.set_ai_kind(i32::from(preset.kind == ProviderKind::Anthropic));
        shell.set_ai_base_url(preset.base_url.as_str().into());
        shell.set_ai_model(preset.suggested_model.as_str().into());
        self.state.borrow_mut().assistant.models.clear();
        self.state.borrow_mut().assistant.check = None;
        self.ai_key_state(preset.label);
        self.sync_assistant();
    }

    /// What the fields say, as a configuration.
    fn ai_fields(&self) -> Option<ProviderConfig> {
        let window = self.window()?;
        let shell = window.global::<Shell>();
        let profile = shell.get_ai_profile().trim().to_string();
        Some(ProviderConfig {
            profile: if profile.is_empty() {
                "default".into()
            } else {
                profile
            },
            kind: if shell.get_ai_kind() == 1 {
                ProviderKind::Anthropic
            } else {
                ProviderKind::OpenAiCompatible
            },
            base_url: shell
                .get_ai_base_url()
                .trim()
                .trim_end_matches('/')
                .to_string(),
            model: shell.get_ai_model().trim().to_string(),
            max_tokens: shell
                .get_ai_max_tokens()
                .trim()
                .parse()
                .ok()
                .filter(|&m: &u32| m >= 256)
                .unwrap_or(rusty_ai::DEFAULT_MAX_TOKENS),
            temperature: None,
            supports_tools: true,
        })
    }

    fn ai_save(self: &Rc<Self>) {
        let Some(config) = self.ai_fields() else {
            return;
        };
        let choice = AssistantChoice {
            profile: config.profile.clone(),
            kind: kind_name(config.kind).to_string(),
            base_url: config.base_url.clone(),
            model: config.model.clone(),
            max_tokens: Some(config.max_tokens),
            temperature: config.temperature,
            supports_tools: Some(config.supports_tools),
        };
        let _ = rusty_embed::config::update(move |saved| saved.assistant = Some(choice));
        self.sync_assistant();
    }

    fn ai_key_state(self: &Rc<Self>, profile: String) {
        work::spawn(
            move || secrets::is_configured(&profile),
            |app, stored| {
                app.state.borrow_mut().assistant.key_stored = stored;
                app.sync_assistant();
            },
        );
    }

    fn ai_save_key(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let key = shell.get_ai_key().trim().to_string();
        let Some(config) = self.ai_fields() else {
            return;
        };
        if key.is_empty() {
            return;
        }
        // Out of the window the moment it is saved.
        shell.set_ai_key(SharedString::new());
        let profile = config.profile;
        work::spawn(
            move || {
                secrets::store(&profile, &key).map_err(|e| e.to_string())?;
                Ok::<_, String>(profile)
            },
            |app, stored| match stored {
                // Read back rather than assumed: a flag saying the opposite
                // of the store is the confident wrong answer.
                Ok(profile) => app.ai_key_state(profile),
                Err(why) => app.note(2, why),
            },
        );
    }

    fn ai_remove_key(self: &Rc<Self>) {
        let Some(config) = self.ai_fields() else {
            return;
        };
        let profile = config.profile;
        work::spawn(
            move || {
                let _ = secrets::delete(&profile);
                profile
            },
            |app, profile| app.ai_key_state(profile),
        );
    }

    /// Ask the endpoint which models it serves: names drift faster than a
    /// release, and a self-hosted server's are unknowable in advance.
    fn ai_list_models(self: &Rc<Self>) {
        let Some(config) = self.ai_fields() else {
            return;
        };
        work::spawn(
            move || {
                block_on(async move {
                    let key = secrets::load(&config.profile).map_err(|e| e.to_string())?;
                    rusty_ai::config::list_models(&config, key, &http())
                        .await
                        .map_err(|e| e.to_string())
                })
            },
            |app, models| match models {
                Ok(models) => {
                    app.state.borrow_mut().assistant.models = models;
                    app.sync_assistant();
                }
                Err(why) => app.note(2, why),
            },
        );
    }

    /// One real request, said as what it established.
    fn ai_check(self: &Rc<Self>) {
        let Some(config) = self.ai_fields() else {
            return;
        };
        self.state.borrow_mut().assistant.check = None;
        self.sync_assistant();
        work::spawn(
            move || {
                block_on(async move {
                    let key = secrets::load(&config.profile).map_err(|e| e.to_string())?;
                    rusty_ai::config::check(&config, key, &http())
                        .await
                        .map_err(|e| e.to_string())
                })
            },
            |app, verdict| {
                let check = match verdict {
                    Ok(ProviderCheck::Reachable {
                        model,
                        models_listed,
                        model_listed,
                    }) => match model_listed {
                        Some(true) => (
                            true,
                            t!("settings.assistant.check-model-listed", model = model),
                        ),
                        Some(false) => (
                            false,
                            t!(
                                "settings.assistant.check-model-missing",
                                model = model,
                                count = models_listed
                            ),
                        ),
                        None => (
                            true,
                            t!("settings.assistant.check-reachable", model = model),
                        ),
                    },
                    Ok(ProviderCheck::NotChecked { why, .. }) => {
                        let rusty_ai::NotCheckedReason::NoModelListing { status } = why;
                        (
                            false,
                            t!("settings.assistant.check-no-listing", status = status),
                        )
                    }
                    Err(why) => (false, why),
                };
                app.state.borrow_mut().assistant.check = Some(check);
                app.sync_assistant();
            },
        );
    }

    /// A model picked from the endpoint's list.
    pub(super) fn ai_pick_model(self: &Rc<Self>, model: &str) {
        if let Some(window) = self.window() {
            window.global::<Shell>().set_ai_model(model.into());
        }
    }

    pub(super) fn ai_fold(self: &Rc<Self>, at: i32) {
        let Ok(at) = usize::try_from(at) else { return };
        {
            let mut state = self.state.borrow_mut();
            let unfolded = &mut state.assistant.unfolded;
            match unfolded.iter().position(|&u| u == at) {
                Some(i) => {
                    unfolded.remove(i);
                }
                None => unfolded.push(at),
            }
        }
        self.sync_assistant();
    }

    // ---- drawing -------------------------------------------------------------

    pub(super) fn sync_assistant(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let config = configured();
        let state = self.state.borrow();
        let ai = &state.assistant;
        shell.set_ai_ready(config.is_some());
        shell.set_ai_title(
            config
                .as_ref()
                .map(|c| c.model.clone())
                .unwrap_or_default()
                .into(),
        );
        shell.set_ai_current(
            match &config {
                Some(c) => t!(
                    "settings.assistant.in-use",
                    model = c.model.clone(),
                    url = c.base_url.clone()
                ),
                None => t!("settings.assistant.none-configured"),
            }
            .into(),
        );
        shell.set_ai_key_stored(ai.key_stored);
        shell.set_ai_models(ModelRc::new(VecModel::from(
            ai.models
                .iter()
                .map(|m| SharedString::from(m.as_str()))
                .collect::<Vec<_>>(),
        )));
        let (ok, check) = ai.check.clone().unwrap_or_default();
        shell.set_ai_check(check.into());
        shell.set_ai_check_ok(ok);
        shell.set_ai_running(ai.streaming.is_some());

        // The transcript.
        let mut entries = Vec::new();
        let mut results: HashMap<&str, bool> = HashMap::new();
        for message in &ai.history {
            for content in &message.content {
                if let Content::ToolResult { id, is_error, .. } = content {
                    results.insert(id.as_str(), !is_error);
                }
            }
        }
        let entry = |kind: i32| AiEntry {
            kind,
            index: -1,
            ..AiEntry::default()
        };
        let mut place = 0usize;
        for message in &ai.history {
            match message.role {
                Role::User => {
                    let mut text = String::new();
                    let mut attached = String::new();
                    for content in &message.content {
                        match content {
                            Content::Text { text: t } => text.push_str(t),
                            Content::Attachment { path, lines, .. } => {
                                attached = match lines {
                                    Some(l) => t!(
                                        "assistant.attached-lines",
                                        first = l.first,
                                        last = l.last,
                                        total = l.total,
                                        path = path.clone()
                                    ),
                                    None => t!("assistant.attached", path = path.clone()),
                                };
                            }
                            _ => {}
                        }
                    }
                    entries.push(AiEntry {
                        text: text.into(),
                        attached: attached.into(),
                        ..entry(0)
                    });
                }
                Role::Assistant => {
                    for content in &message.content {
                        match content {
                            Content::Thinking { text } => {
                                let open = ai.unfolded.contains(&place);
                                entries.push(AiEntry {
                                    index: place as i32,
                                    text: if open {
                                        text.as_str().into()
                                    } else {
                                        SharedString::new()
                                    },
                                    open,
                                    ..entry(2)
                                });
                            }
                            Content::Text { text } => entries.push(AiEntry {
                                blocks: markdown_view::blocks(&self.files, text, None),
                                ..entry(1)
                            }),
                            Content::ToolUse { id, name, input } => {
                                let summary: String = input.to_string().chars().take(160).collect();
                                entries.push(AiEntry {
                                    text: name.as_str().into(),
                                    attached: summary.into(),
                                    state: match results.get(id.as_str()) {
                                        Some(true) => 1,
                                        Some(false) => 2,
                                        None => 0,
                                    },
                                    ..entry(3)
                                });
                            }
                            _ => {}
                        }
                        place += 1;
                    }
                }
                Role::Tool => {}
            }
        }
        if let Some(streaming) = &ai.streaming {
            if !streaming.thinking.is_empty() {
                entries.push(AiEntry {
                    text: streaming.thinking.as_str().into(),
                    open: true,
                    index: -1,
                    ..entry(2)
                });
            }
            for run in &streaming.tools {
                entries.push(AiEntry {
                    text: run.name.as_str().into(),
                    state: match run.ok {
                        Some(true) => 1,
                        Some(false) => 2,
                        None => 0,
                    },
                    ..entry(3)
                });
            }
            if !streaming.text.is_empty() {
                entries.push(AiEntry {
                    blocks: markdown_view::blocks(&self.files, &streaming.text, None),
                    ..entry(1)
                });
            }
        }
        if let Some(max) = ai.cut_short {
            entries.push(AiEntry {
                text: t!("assistant.cut-short", max = max).into(),
                ..entry(4)
            });
        }
        if let Some(problem) = &ai.problem {
            entries.push(AiEntry {
                text: problem.as_str().into(),
                state: 2,
                ..entry(4)
            });
        }
        shell.set_ai_entries(ModelRc::new(VecModel::from(entries)));
        shell.set_ai_spend(
            ai.spend
                .map(|s| {
                    if s.cached > 0 {
                        t!(
                            "assistant.tokens-cached",
                            input = tokens(s.input),
                            cached = tokens(s.cached),
                            output = tokens(s.output)
                        )
                    } else {
                        t!(
                            "assistant.tokens",
                            input = tokens(s.input),
                            output = tokens(s.output)
                        )
                    }
                })
                .unwrap_or_default()
                .into(),
        );
        shell.set_ai_spend_tip(
            ai.spend
                .map(|s| t!("assistant.tokens-rounds", rounds = s.rounds))
                .unwrap_or_default()
                .into(),
        );
        // The chip: what goes with the next question.
        let attach = self.editor.current_path();
        shell.set_ai_attach(attach.clone().unwrap_or_default().into());
        shell.set_ai_detached(ai.detached);
        shell.set_ai_tick(shell.get_ai_tick().wrapping_add(1));
    }
}

fn cap_key(config: &ProviderConfig) -> String {
    format!("{}\n{}\n{}", config.profile, config.base_url, config.model)
}

fn http() -> rusty_ai::Http {
    rusty_ai::Http {
        proxy: rusty_embed::net::effective_proxy(),
    }
}

/// A future run to its end on a runtime of its own — the window's thread
/// has none, and a question's thread is its own.
fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime for the assistant")
        .block_on(future)
}

enum Outcome {
    Done(Vec<Message>),
    Stopped,
    Failed(String),
}

/// How a question ended, and what it learned on the way.
struct Answer {
    outcome: Outcome,
    /// The output cap a provider named, under the provider's key.
    cap: Option<(String, u32)>,
    /// The Cargo analysis a tool loaded, to keep for the next question.
    workspace: Option<Arc<rusty_core::Workspace>>,
}

/// One question, start to finish, off the window's thread.
/// What a question goes with: the model, the conversation so far, the
/// output budget, the project and what is known of it, and what a running
/// simulation said.
struct Question {
    config: ProviderConfig,
    history: Vec<Message>,
    budget: u32,
    root: Option<std::path::PathBuf>,
    workspace: Option<Arc<rusty_core::Workspace>>,
    live: Option<rusty_embed::spatial::sheet::Live>,
}

fn run_question(question: Question, stop: Arc<tokio::sync::Notify>, ticket: u64) -> Answer {
    let Question {
        config,
        mut history,
        budget,
        root,
        workspace,
        live,
    } = question;
    let key = cap_key(&config);
    block_on(async move {
        let catalog = rusty_embed::catalog::Catalog::load(root.as_deref());
        let lazy = root.clone().map(|root| match workspace {
            Some(workspace) => LazyWorkspace::holding(root, workspace),
            None => LazyWorkspace::new(root),
        });
        let firmware = root
            .as_deref()
            .and_then(rusty_embed::firmware::newest_in_project)
            .map(|f| std::path::PathBuf::from(f.path));
        let context = ToolContext {
            workspace: None,
            workspace_on_demand: lazy.as_ref(),
            root: root.as_deref(),
            firmware,
            catalog: Some(&catalog),
            live: live.as_ref(),
        };
        let provider = secrets::load(&config.profile)
            .map_err(|e| e.to_string())
            .and_then(|stored| {
                rusty_ai::config::build(&config, stored, &http()).map_err(|e| e.to_string())
            });
        let provider = match provider {
            Ok(provider) => provider,
            Err(why) => {
                return Answer {
                    outcome: Outcome::Failed(why),
                    cap: None,
                    workspace: None,
                };
            }
        };
        let agent = Agent::new(provider)
            .with_tools(ToolRegistry::workbench())
            .with_max_tokens(budget);
        let mut deliver = move |event: AgentEvent| {
            work::deliver(event, move |app, event| app.ai_event(ticket, event));
        };
        let outcome = {
            let question = agent.ask(&context, &mut history, &mut deliver);
            tokio::select! {
                finished = tokio::time::timeout(BUDGET, question) => match finished {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(Outcome::Failed(error.to_string())),
                    Err(_) => Some(Outcome::Failed(t!("desk.ai-too-long", minutes = BUDGET.as_secs() / 60))),
                },
                () = stop.notified() => Some(Outcome::Stopped),
            }
        };
        Answer {
            outcome: outcome.unwrap_or(Outcome::Done(history)),
            cap: agent.learned_cap().map(|cap| (key, cap)),
            workspace: lazy.as_ref().and_then(LazyWorkspace::loaded),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_count_is_short() {
        assert_eq!(tokens(512), "512");
        assert_eq!(tokens(1840), "1.8k");
        assert_eq!(tokens(41_302), "41k");
    }
}
