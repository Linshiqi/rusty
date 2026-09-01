//! Anthropic Messages API.
//!
//! Kept native rather than routed through an OpenAI-compatible shim: the tool
//! use format, the system prompt placement, and the streaming event model are
//! all different enough that a shim would lose information — particularly
//! `input_json_delta`, which is how tool arguments actually arrive.

use async_stream::try_stream;
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ChatRequest, EventStream, Provider};
use crate::{
    error::{Error, Result},
    model::{ChatEvent, Content, Role, StopReason},
};

/// Wire version pinned deliberately: Anthropic requires the header and a
/// floating value would make failures non-reproducible.
const API_VERSION: &str = "2023-06-01";

pub struct Anthropic {
    profile: String,
    base_url: String,
    model: String,
    api_key: String,
    http: reqwest::Client,
}

impl Anthropic {
    pub fn new(
        profile: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            profile: profile.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key: api_key.into(),
            http: reqwest::Client::new(),
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/messages", self.base_url)
    }
}

#[async_trait]
impl Provider for Anthropic {
    fn id(&self) -> &str {
        &self.profile
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn chat(&self, request: ChatRequest) -> Result<EventStream> {
        let endpoint = self.endpoint();
        let profile = self.profile.clone();

        let mut body = json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "stream": true,
            "messages": to_messages(&request),
        });
        if let Some(system) = &request.system {
            body["system"] = json!(system);
        }
        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "input_schema": tool.input_schema,
                        })
                    })
                    .collect(),
            );
        }

        let response = self
            .http
            .post(&endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::transport(endpoint.clone(), e))?;

        let response = super::openai::check_status(response, &profile).await?;

        let stream = try_stream! {
            let mut events = response.bytes_stream().eventsource();
            // Content blocks are addressed by index; tool ids arrive once, at
            // block start, and every later delta only carries the index.
            let mut tool_ids: Vec<(u64, String)> = Vec::new();
            let mut stop = StopReason::EndTurn;

            while let Some(event) = events.next().await {
                let event = event.map_err(|e| Error::protocol(&profile, e.to_string()))?;
                let parsed: StreamEvent = match serde_json::from_str(&event.data) {
                    Ok(parsed) => parsed,
                    // Ping and other bookkeeping events carry shapes we do not
                    // model; skipping them is correct, not a failure.
                    Err(_) => continue,
                };

                match parsed {
                    StreamEvent::MessageStart { message } => {
                        if let Some(usage) = message.usage {
                            yield ChatEvent::Usage {
                                input_tokens: usage.input_tokens,
                                output_tokens: usage.output_tokens,
                            };
                        }
                    }
                    StreamEvent::ContentBlockStart { index, content_block } => {
                        if let ContentBlock::ToolUse { id, name } = content_block {
                            tool_ids.push((index, id.clone()));
                            yield ChatEvent::ToolCallStart { id, name };
                        }
                    }
                    StreamEvent::ContentBlockDelta { index, delta } => match delta {
                        BlockDelta::TextDelta { text } => {
                            yield ChatEvent::TextDelta { text };
                        }
                        BlockDelta::InputJsonDelta { partial_json } => {
                            if let Some((_, id)) = tool_ids.iter().find(|(i, _)| *i == index) {
                                yield ChatEvent::ToolCallDelta {
                                    id: id.clone(),
                                    partial_json,
                                };
                            }
                        }
                        BlockDelta::Other => {}
                    },
                    StreamEvent::ContentBlockStop { index } => {
                        if let Some((_, id)) = tool_ids.iter().find(|(i, _)| *i == index) {
                            yield ChatEvent::ToolCallEnd { id: id.clone() };
                        }
                    }
                    StreamEvent::MessageDelta { delta, usage } => {
                        if let Some(reason) = delta.stop_reason {
                            stop = match reason.as_str() {
                                "tool_use" => StopReason::ToolUse,
                                "max_tokens" => StopReason::MaxTokens,
                                "end_turn" | "stop_sequence" => StopReason::EndTurn,
                                _ => StopReason::Other,
                            };
                        }
                        if let Some(usage) = usage {
                            yield ChatEvent::Usage {
                                input_tokens: usage.input_tokens,
                                output_tokens: usage.output_tokens,
                            };
                        }
                    }
                    StreamEvent::MessageStop => break,
                    StreamEvent::Error { error } => {
                        Err(Error::protocol(&profile, error.message))?;
                    }
                    StreamEvent::Other => {}
                }
            }

            yield ChatEvent::Done { stop };
        };

        Ok(Box::pin(stream))
    }
}

/// Anthropic has no `tool` role: results go back as *user* messages containing
/// `tool_result` blocks. That reshaping is the whole reason this function
/// differs from the OpenAI one.
fn to_messages(request: &ChatRequest) -> Vec<Value> {
    request
        .messages
        .iter()
        .map(|message| {
            let role = match message.role {
                Role::Assistant => "assistant",
                Role::User | Role::Tool => "user",
            };
            let blocks: Vec<Value> = message
                .content
                .iter()
                .map(|content| match content {
                    Content::Text { text } => json!({ "type": "text", "text": text }),
                    Content::ToolUse { id, name, input } => json!({
                        "type": "tool_use", "id": id, "name": name, "input": input
                    }),
                    Content::ToolResult {
                        id,
                        content,
                        is_error,
                    } => json!({
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": content,
                        "is_error": is_error,
                    }),
                })
                .collect();
            json!({ "role": role, "content": blocks })
        })
        .collect()
}

// ─── wire types ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamEvent {
    MessageStart {
        message: MessageStart,
    },
    ContentBlockStart {
        index: u64,
        content_block: ContentBlock,
    },
    ContentBlockDelta {
        index: u64,
        delta: BlockDelta,
    },
    ContentBlockStop {
        index: u64,
    },
    MessageDelta {
        delta: MessageDeltaBody,
        #[serde(default)]
        usage: Option<Usage>,
    },
    MessageStop,
    Error {
        error: ApiError,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct MessageStart {
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    ToolUse {
        id: String,
        name: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BlockDelta {
    TextDelta {
        text: String,
    },
    InputJsonDelta {
        partial_json: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct MessageDeltaBody {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}
