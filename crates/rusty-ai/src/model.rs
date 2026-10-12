//! Wire types for the AI layer.
//!
//! Compiled unconditionally and free of IO, so the Leptos frontend can `use`
//! these directly — same split as `rusty_core::model` and `rusty_embed::model`.
//! Everything that opens a socket or reads a keychain lives behind the
//! `backend` feature.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─────────────────────────────────────────────────────────────────────────────
// Conversation
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: Role,
    pub content: Vec<Content>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: vec![Content::Text { text: text.into() }],
        }
    }

    pub fn assistant(content: Vec<Content>) -> Self {
        Message {
            role: Role::Assistant,
            content,
        }
    }

    /// Results of tool calls, fed back so the model can continue.
    pub fn tool_results(results: Vec<Content>) -> Self {
        Message {
            role: Role::Tool,
            content: results,
        }
    }

    /// All prose in this message, for rendering.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Assistant,
    /// Carries `ToolResult` content. OpenAI models this as separate messages
    /// with a `tool` role; Anthropic as a user message containing tool_result
    /// blocks. Both are produced from this one variant.
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Content {
    /// A named field, not a newtype. With `tag = "type"` serde has nowhere to
    /// put the discriminant inside a bare string and refuses at *runtime* —
    /// which for this type means every assistant answer failing to cross the
    /// IPC boundary, long after the code that looked wrong. It also matches
    /// what both providers already put on the wire: `{"type":"text","text":…}`.
    Text { text: String },
    #[serde(rename_all = "camelCase")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename_all = "camelCase")]
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    /// The file the user had open when they asked, sent with the question so
    /// the model reads what they are looking at rather than guessing at it.
    /// Its own variant rather than text folded into the question: the panel
    /// shows it as a chip and the providers render it as prose, so the
    /// transcript never carries the file twice — once for the model and once
    /// for the eye.
    #[serde(rename_all = "camelCase")]
    Attachment {
        path: String,
        text: String,
        /// Which lines `text` is when it is not the whole file: the part
        /// around the user's cursor or selection, from a file too long to
        /// send whole with every round. `None` is the whole file.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<AttachedLines>,
    },
    /// What the model thought before it answered, from models that stream
    /// their reasoning. Kept in the transcript so the reader can unfold it,
    /// and skipped by both providers when the history goes back: the OpenAI
    /// dialect's reasoning models reject their own reasoning as input, and
    /// Anthropic's would want it signed.
    Thinking { text: String },
}

/// The lines of a file an attachment holds, 1-based and inclusive, and how
/// many the file has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachedLines {
    pub first: u32,
    pub last: u32,
    pub total: u32,
}

impl Content {
    /// The file the user has open, as it goes with a question.
    ///
    /// A short file whole. Of a longer one, the lines the user is looking at
    /// — the selection, or the cursor's line and up to [`ATTACH_AROUND`]
    /// either side — to [`ATTACH_WHOLE`] bytes, with which lines they are, so
    /// the model knows it holds a part and that `read_file` has the rest. The
    /// whole file, up to sixty kilobytes, went with every question once, and
    /// every round of the agent loop sends what the question carries: some
    /// fifteen thousand tokens a round, spent again on a question that was
    /// not about the file at all. VS Code's Copilot sends what is on screen,
    /// not the file.
    ///
    /// `focus` is the selection's first and last line, 0-based; `None`
    /// starts at the top.
    pub fn attach(path: impl Into<String>, text: &str, focus: Option<(usize, usize)>) -> Content {
        let (text, lines) = window(text, focus);
        Content::Attachment {
            path: path.into(),
            text,
            lines,
        }
    }

    /// What a provider sends for this block where it can only send text: the
    /// prose itself, or an attachment framed as the file it is — and as the
    /// part of it that it is, when it is a part, so the model reads the rest
    /// rather than taking a window for the whole. Tool blocks have their own
    /// wire shapes and answer `None`.
    pub fn prose(&self) -> Option<std::borrow::Cow<'_, str>> {
        match self {
            Content::Text { text } => Some(std::borrow::Cow::Borrowed(text)),
            Content::Attachment {
                path,
                text,
                lines: None,
            } => Some(std::borrow::Cow::Owned(format!(
                "The user has this file open in the editor: `{path}`\n\n```\n{text}\n```"
            ))),
            Content::Attachment {
                path,
                text,
                lines: Some(lines),
            } => Some(std::borrow::Cow::Owned(format!(
                "The user has `{path}` open in the editor. Below are lines {}–{} of its {}, \
                 around their cursor; read_file has the rest.\n\n```\n{text}\n```",
                lines.first, lines.last, lines.total
            ))),
            Content::ToolUse { .. } | Content::ToolResult { .. } | Content::Thinking { .. } => None,
        }
    }
}

/// A file this size or smaller goes whole with a question: about two
/// thousand tokens of code.
pub const ATTACH_WHOLE: usize = 8_000;

/// Lines either side of the cursor sent from a longer file.
pub const ATTACH_AROUND: usize = 60;

/// The text [`Content::attach`] sends, and which lines it is when it is not
/// the whole file.
fn window(text: &str, focus: Option<(usize, usize)>) -> (String, Option<AttachedLines>) {
    if text.len() <= ATTACH_WHOLE {
        return (text.to_string(), None);
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let total = lines.len();
    let (a, b) = focus.unwrap_or((0, 0));
    let from = a.min(total - 1);
    let to = b.clamp(from, total - 1);
    let cost = |i: usize| lines[i].len() + 1;

    // The selection first — a larger one than the budget keeps its start —
    // then outwards a line each side at a time while the budget lasts.
    let (mut first, mut last, mut size) = (from, from, cost(from));
    while last < to && size + cost(last + 1) <= ATTACH_WHOLE {
        last += 1;
        size += cost(last);
    }
    let (mut up, mut down) = (0, 0);
    loop {
        let mut grew = false;
        if up < ATTACH_AROUND && first > 0 && size + cost(first - 1) <= ATTACH_WHOLE {
            first -= 1;
            size += cost(first);
            up += 1;
            grew = true;
        }
        if down < ATTACH_AROUND && last + 1 < total && size + cost(last + 1) <= ATTACH_WHOLE {
            last += 1;
            size += cost(last);
            down += 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }

    let mut part = lines[first..=last].join("\n");
    if part.len() > ATTACH_WHOLE {
        // One line longer than the whole budget: cut it at a character.
        let mut end = ATTACH_WHOLE;
        while !part.is_char_boundary(end) {
            end -= 1;
        }
        part.truncate(end);
        part.push_str(" … [cut]");
    }
    let range = AttachedLines {
        first: (first + 1) as u32,
        last: (last + 1) as u32,
        total: total as u32,
    };
    (part, Some(range))
}

/// Normalized stream events.
///
/// This enum is the contract the frontend renders against. Adding a provider
/// must never add a variant here — if it would, the abstraction is wrong.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatEvent {
    /// A chunk of assistant prose.
    TextDelta {
        text: String,
    },
    /// A chunk of the model's reasoning, from models that stream it
    /// (`reasoning_content` in the OpenAI dialect, `thinking_delta` in
    /// Anthropic's). Its own event because it is not the answer: shown dim
    /// and folded, and never sent back. A reasoning model can spend its
    /// whole output budget here — 4096 tokens of thought and no answer, which
    /// with this dropped on the floor looked like a reply that never came.
    ThinkingDelta {
        text: String,
    },
    /// The model has decided to call a tool. Arguments stream in separately
    /// because both providers emit them as partial JSON.
    #[serde(rename_all = "camelCase")]
    ToolCallStart {
        id: String,
        name: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallDelta {
        id: String,
        partial_json: String,
    },
    ToolCallEnd {
        id: String,
    },
    /// Token counts, when the provider reports them. Shown to the user because
    /// with BYO keys, every token is money out of their pocket.
    ///
    /// One per round, just before `Done`, whatever the provider streamed on
    /// the way: some servers repeat a running total on every chunk, and a
    /// window adding up every report would count each round many times.
    /// `input_tokens` is the whole prompt; `cached_tokens` is how much of it
    /// the provider's prompt cache supplied, billed at a fraction.
    #[serde(rename_all = "camelCase")]
    Usage {
        input_tokens: u32,
        cached_tokens: u32,
        output_tokens: u32,
    },
    Done {
        stop: StopReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    /// The model finished its turn.
    EndTurn,
    /// The model wants tool results before continuing.
    ToolUse,
    /// Hit `max_tokens`. Worth surfacing — the answer is truncated.
    MaxTokens,
    Other,
}

/// Events the UI renders. Wraps provider streaming with the agent loop's own
/// tool execution, which providers know nothing about.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum AgentEvent {
    Chat(ChatEvent),
    #[serde(rename_all = "camelCase")]
    ToolStarted {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename_all = "camelCase")]
    ToolFinished {
        id: String,
        name: String,
        ok: bool,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// Tools
// ─────────────────────────────────────────────────────────────────────────────

/// Where a tool came from.
///
/// Built-ins are the only source today. The variant exists now because the
/// permission model, the namespacing rules, and the UI's "who is doing this"
/// affordance all key off provenance — and all three are painful to retrofit
/// once third-party tools are already running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ToolSource {
    Builtin,
    /// An MCP server the user connected. See `docs/extensibility.md`.
    Mcp {
        server: String,
    },
}

/// What a tool is allowed to do.
///
/// Declared rather than inferred: a host that has to guess at a tool's blast
/// radius cannot ever prompt the user accurately. Everything defaults to the
/// least privilege, so a tool that forgets to declare gets the safe treatment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub reads_workspace: bool,
    pub writes_workspace: bool,
    pub network: bool,
    pub runs_commands: bool,
}

impl Capabilities {
    /// The only shape currently in use: looks at the project, touches nothing.
    pub const READ_ONLY: Self = Self {
        reads_workspace: true,
        writes_workspace: false,
        network: false,
        runs_commands: false,
    };

    /// Whether invoking this needs the user to say yes first.
    pub fn needs_approval(&self) -> bool {
        self.writes_workspace || self.runs_commands
    }
}

/// A tool as the model sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDef {
    pub name: String,
    /// The model reads this to decide whether to call the tool, so it states
    /// what question the tool answers rather than what function it wraps.
    pub description: String,
    /// JSON Schema for the arguments.
    pub input_schema: Value,
    pub capabilities: Capabilities,
    #[serde(default = "builtin_source")]
    pub source: ToolSource,
}

fn builtin_source() -> ToolSource {
    ToolSource::Builtin
}

// ─────────────────────────────────────────────────────────────────────────────
// Checking a provider
// ─────────────────────────────────────────────────────────────────────────────

/// What checking a provider profile established, as facts rather than as a
/// sentence.
///
/// The first version of the check returned prose, and the prose said
/// "Reachable" on a path that had made no network request at all: the model
/// list was `unwrap_or_default()`ed, so a 401, a DNS failure and a timeout all
/// read as an empty list, and an empty list read as success. Facts cannot be
/// defaulted into a verdict — a failed request is an error, and the frontend
/// words what did happen, in the user's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "camelCase")]
pub enum ProviderCheck {
    /// The endpoint answered a request that carried the stored key.
    #[serde(rename_all = "camelCase")]
    Reachable {
        /// The model the profile names.
        model: String,
        /// How many models the endpoint listed. Zero is an answer too — it
        /// listed none — and is why `model_listed` is optional.
        models_listed: usize,
        /// Whether `model` was among them; absent when the endpoint listed
        /// nothing, because "not in an empty list" is not evidence of anything.
        model_listed: Option<bool>,
    },
    /// The endpoint was reached but the check could not be carried out, for
    /// the stated reason. Neither the key nor the model has been verified.
    #[serde(rename_all = "camelCase")]
    NotChecked {
        model: String,
        why: NotCheckedReason,
    },
}

/// Why a provider check stopped short. One variant today; a tagged enum so the
/// next reason is an addition the frontend matches on, not a sentence it has
/// to parse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "camelCase")]
pub enum NotCheckedReason {
    /// `/models` answered with a status saying there is no such listing here
    /// (404 or 405). Either the endpoint does not list its models — some
    /// gateways do not — or the base URL is wrong; the check cannot tell which,
    /// and says so rather than picking one.
    NoModelListing { status: u16 },
}

// ─────────────────────────────────────────────────────────────────────────────
// Providers
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    /// The `/chat/completions` dialect. Most of the world speaks it.
    OpenAiCompatible,
    /// Anthropic's Messages API, kept native for correct tool use.
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// User-chosen profile name. Also the key under which the secret is filed,
    /// so a user can keep several accounts for the same vendor.
    pub profile: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Set false for models that cannot call tools. The workbench analyses
    /// become unavailable, so the UI warns rather than degrading silently.
    #[serde(default = "default_true")]
    pub supports_tools: bool,
}

fn default_max_tokens() -> u32 {
    DEFAULT_MAX_TOKENS
}

/// The output budget a new profile starts with.
///
/// Deliberately far above what any answer needs: a model that reasons spends
/// its budget thinking first, and 4096 — the old default — was spent before
/// the first word of the answer arrived. The number is a ceiling, not a
/// target; a provider whose model caps output lower says so in its refusal,
/// and the loop learns that cap from the refusal rather than guessing one.
pub const DEFAULT_MAX_TOKENS: u32 = 200_000;

fn default_true() -> bool {
    true
}

/// A starting point for a provider the user is about to configure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub label: String,
    pub kind: ProviderKind,
    pub base_url: String,
    /// A plausible model to start from. Base URLs are stable; model names are
    /// not, so treat this as a placeholder and prefer runtime discovery.
    pub suggested_model: String,
    /// True for endpoints that run on the user's own machine — no key needed,
    /// and nothing leaves the device.
    pub local: bool,
}

/// Endpoints worth offering out of the box.
///
/// Pure data, so the settings screen can render before any backend call.
/// Deliberately includes domestic Chinese providers and local runtimes, because
/// "bring your own LLM" is useless if the list assumes everyone can reach
/// api.openai.com.
// A table, and it reads as one: label, dialect, endpoint, model, local. One
// provider per line is the whole point — rustfmt would give each of these
// eleven entries six lines and turn a list anyone can scan into three screens.
#[rustfmt::skip]
pub fn presets() -> Vec<Preset> {
    use ProviderKind::*;
    let p = |label: &str, kind, base_url: &str, model: &str, local| Preset {
        label: label.to_string(),
        kind,
        base_url: base_url.to_string(),
        suggested_model: model.to_string(),
        local,
    };

    vec![
        p("Anthropic", Anthropic, "https://api.anthropic.com/v1", "claude-sonnet-5", false),
        p("OpenAI", OpenAiCompatible, "https://api.openai.com/v1", "gpt-4o", false),
        p("DeepSeek", OpenAiCompatible, "https://api.deepseek.com/v1", "deepseek-chat", false),
        p("Moonshot / Kimi", OpenAiCompatible, "https://api.moonshot.cn/v1", "moonshot-v1-32k", false),
        p("Zhipu / GLM", OpenAiCompatible, "https://open.bigmodel.cn/api/paas/v4", "glm-4-plus", false),
        p("DashScope / Qwen", OpenAiCompatible, "https://dashscope.aliyuncs.com/compatible-mode/v1", "qwen-plus", false),
        p("SiliconFlow", OpenAiCompatible, "https://api.siliconflow.cn/v1", "", false),
        p("OpenRouter", OpenAiCompatible, "https://openrouter.ai/api/v1", "", false),
        p("Ollama (local)", OpenAiCompatible, "http://localhost:11434/v1", "", true),
        p("LM Studio (local)", OpenAiCompatible, "http://localhost:1234/v1", "", true),
        p("vLLM / custom", OpenAiCompatible, "http://localhost:8000/v1", "", true),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything the assistant sends the frontend crosses the IPC boundary as
    /// JSON, and the transcript comes straight back on the next turn. A variant
    /// that cannot round-trip breaks the conversation on the second question,
    /// which is a long way from where the mistake was made.
    #[test]
    fn conversation_content_survives_the_wire() {
        let message = Message {
            role: Role::Assistant,
            content: vec![
                Content::Text {
                    text: "checking the project".into(),
                },
                Content::ToolUse {
                    id: "call_1".into(),
                    name: "project_status".into(),
                    input: serde_json::json!({ "path": "." }),
                },
                Content::ToolResult {
                    id: "call_1".into(),
                    content: "{}".into(),
                    is_error: false,
                },
                Content::Attachment {
                    path: "src/main.rs".into(),
                    text: "fn main() {}".into(),
                    lines: Some(AttachedLines {
                        first: 3,
                        last: 9,
                        total: 40,
                    }),
                },
                Content::Thinking {
                    text: "the manifest names a chip".into(),
                },
            ],
        };

        let json = serde_json::to_string(&message).expect("serialize");
        assert!(
            json.contains("\"type\":\"thinking\""),
            "the panel matches the tag: {json}"
        );
        let back: Message = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.content.len(), 5);
        assert_eq!(
            back.text(),
            "checking the project",
            "neither an attachment nor the thinking is prose to show; the panel draws each its own way"
        );
        assert!(
            back.content[4].prose().is_none(),
            "thinking never goes back to a model as text"
        );
        assert!(
            matches!(&back.content[1], Content::ToolUse { name, .. } if name == "project_status"),
            "tool calls must survive: the next turn replays them to the model",
        );
        let framed = back.content[3]
            .prose()
            .expect("an attachment reaches the model as text");
        assert!(
            framed.contains("src/main.rs") && framed.contains("fn main() {}"),
            "{framed}"
        );
        assert!(
            framed.contains("lines 3–9 of its 40") && framed.contains("read_file"),
            "a part says it is one, and where the rest is: {framed}"
        );
        let whole = Content::Attachment {
            path: "src/main.rs".into(),
            text: "fn main() {}".into(),
            lines: None,
        };
        assert!(!whole.prose().unwrap().contains("lines"));
        assert!(
            !serde_json::to_string(&whole).unwrap().contains("lines"),
            "a whole file carries no range on the wire"
        );
    }

    /// The verdict is matched on by the frontend, so its tags are a contract:
    /// `verdict` on the outside, `reason` on the inside, camelCase throughout.
    #[test]
    fn a_provider_check_survives_the_wire_with_its_tags() {
        let reachable = ProviderCheck::Reachable {
            model: "gpt-4o".into(),
            models_listed: 12,
            model_listed: Some(true),
        };
        let json = serde_json::to_value(&reachable).expect("serialize");
        assert_eq!(json["verdict"], "reachable");
        assert_eq!(json["modelsListed"], 12);
        assert_eq!(json["modelListed"], true);
        let back: ProviderCheck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, reachable);

        let unchecked = ProviderCheck::NotChecked {
            model: "x".into(),
            why: NotCheckedReason::NoModelListing { status: 404 },
        };
        let json = serde_json::to_value(&unchecked).expect("serialize");
        assert_eq!(json["verdict"], "notChecked");
        assert_eq!(json["why"]["reason"], "noModelListing");
        assert_eq!(json["why"]["status"], 404);
        let back: ProviderCheck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, unchecked);
    }

    #[test]
    fn stream_events_survive_the_wire() {
        for event in [
            AgentEvent::Chat(ChatEvent::TextDelta { text: "hi".into() }),
            AgentEvent::Chat(ChatEvent::Usage {
                input_tokens: 12,
                cached_tokens: 8,
                output_tokens: 34,
            }),
            AgentEvent::Chat(ChatEvent::Done {
                stop: StopReason::EndTurn,
            }),
            AgentEvent::ToolStarted {
                id: "1".into(),
                name: "memory_report".into(),
                input: serde_json::Value::Null,
            },
            AgentEvent::ToolFinished {
                id: "1".into(),
                name: "memory_report".into(),
                ok: true,
            },
        ] {
            let json = serde_json::to_string(&event).expect("serialize");
            serde_json::from_str::<AgentEvent>(&json)
                .unwrap_or_else(|e| panic!("{json} did not round-trip: {e}"));
        }
    }
}

#[cfg(test)]
mod attach_tests {
    use super::{ATTACH_AROUND, ATTACH_WHOLE, AttachedLines, Content};

    fn file(lines: usize) -> String {
        (0..lines)
            .map(|i| format!("let line_{i} = {i}; // a line of ordinary length\n"))
            .collect()
    }

    fn attach(text: &str, focus: Option<(usize, usize)>) -> (String, Option<AttachedLines>) {
        match Content::attach("src/main.rs", text, focus) {
            Content::Attachment { text, lines, .. } => (text, lines),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_short_file_goes_whole() {
        let (text, lines) = attach("fn main() {}\n", Some((0, 0)));
        assert_eq!(text, "fn main() {}\n");
        assert_eq!(lines, None);
    }

    /// The lines around the cursor, exactly as they are in the file, and
    /// which ones they are.
    #[test]
    fn a_long_file_sends_the_lines_around_the_cursor() {
        let file = file(1000);
        let (text, lines) = attach(&file, Some((500, 500)));
        let lines = lines.expect("a part says which");
        assert_eq!(
            lines.total, 1001,
            "a text ending in a newline has an empty last line"
        );
        assert!(lines.first <= 501 && 501 <= lines.last, "{lines:?}");
        assert!(lines.last - lines.first <= 2 * ATTACH_AROUND as u32);
        assert!(text.len() <= ATTACH_WHOLE);
        let expected: Vec<&str> = file
            .split('\n')
            .skip(lines.first as usize - 1)
            .take((lines.last - lines.first + 1) as usize)
            .collect();
        assert_eq!(text, expected.join("\n"));
        assert!(text.contains("let line_500 ="));
    }

    #[test]
    fn with_no_cursor_or_at_the_top_the_part_runs_down() {
        let file = file(1000);
        for focus in [None, Some((0, 0))] {
            let (text, lines) = attach(&file, focus);
            assert_eq!(lines.unwrap().first, 1);
            assert!(text.starts_with("let line_0 ="));
        }
    }

    /// A selection is what the question is about: all of it when it fits,
    /// from its start when it does not.
    #[test]
    fn a_selection_is_sent_and_a_long_one_from_its_start() {
        let file = file(1000);
        let (text, _) = attach(&file, Some((200, 210)));
        assert!(text.contains("let line_200 =") && text.contains("let line_210 ="));
        let (text, lines) = attach(&file, Some((100, 900)));
        assert_eq!(lines.unwrap().first, 101);
        assert!(text.starts_with("let line_100 ="));
        assert!(text.len() <= ATTACH_WHOLE);
    }

    /// One line longer than the budget is cut at a character — the CJK
    /// case is the one that panics when it is not — and says so.
    #[test]
    fn a_file_of_one_long_line_is_cut_at_a_character() {
        let long: String = std::iter::repeat_n('中', ATTACH_WHOLE).collect();
        let (text, lines) = attach(&long, Some((0, 0)));
        assert!(text.ends_with("[cut]"));
        assert!(text.starts_with("中中中"));
        assert!(text.len() <= ATTACH_WHOLE + 12);
        assert_eq!(lines.unwrap().total, 1);
    }
}
