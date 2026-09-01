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
    #[serde(rename_all = "camelCase")]
    Usage {
        input_tokens: u32,
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
    4096
}

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
            ],
        };

        let json = serde_json::to_string(&message).expect("serialize");
        let back: Message = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.content.len(), 3);
        assert_eq!(back.text(), "checking the project");
        assert!(
            matches!(&back.content[1], Content::ToolUse { name, .. } if name == "project_status"),
            "tool calls must survive: the next turn replays them to the model",
        );
    }

    #[test]
    fn stream_events_survive_the_wire() {
        for event in [
            AgentEvent::Chat(ChatEvent::TextDelta { text: "hi".into() }),
            AgentEvent::Chat(ChatEvent::Usage {
                input_tokens: 12,
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
