//! OpenAI-compatible chat completions.
//!
//! One implementation covers OpenAI, DeepSeek, Moonshot/Kimi, Zhipu GLM,
//! DashScope/Qwen, OpenRouter, Ollama, vLLM, and LM Studio — they all speak the
//! same `/chat/completions` dialect. That is why this is the default path and
//! Anthropic is the special case, rather than the other way round.

use std::collections::HashMap;

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

pub struct OpenAiCompatible {
    profile: String,
    base_url: String,
    model: String,
    api_key: String,
    http: reqwest::Client,
    supports_tools: bool,
}

impl OpenAiCompatible {
    pub fn new(
        profile: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        supports_tools: bool,
    ) -> Self {
        Self {
            profile: profile.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key: api_key.into(),
            http: reqwest::Client::new(),
            supports_tools,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

#[async_trait]
impl Provider for OpenAiCompatible {
    fn id(&self) -> &str {
        &self.profile
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn supports_tools(&self) -> bool {
        self.supports_tools
    }

    async fn chat(&self, request: ChatRequest) -> Result<EventStream> {
        let endpoint = self.endpoint();
        let profile = self.profile.clone();

        let mut body = json!({
            "model": self.model,
            "messages": to_messages(&request),
            "stream": true,
            // Without this, most compatible servers omit usage entirely on
            // streamed responses, and the user cannot see what they spent.
            "stream_options": { "include_usage": true },
            "max_tokens": request.max_tokens,
        });
        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }
        if self.supports_tools && !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": tool.name,
                                "description": tool.description,
                                "parameters": tool.input_schema,
                            }
                        })
                    })
                    .collect(),
            );
        }

        let response = self
            .http
            .post(&endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::transport(endpoint.clone(), e))?;

        let response = check_status(response, &profile).await?;

        let stream = try_stream! {
            let mut events = response.bytes_stream().eventsource();
            // Only the first fragment of a tool call carries its id and name;
            // every later fragment identifies itself by index alone.
            let mut ids_by_index: HashMap<u64, String> = HashMap::new();
            let mut stop = StopReason::EndTurn;

            while let Some(event) = events.next().await {
                let event = event.map_err(|e| Error::protocol(&profile, e.to_string()))?;
                if event.data.trim() == "[DONE]" {
                    break;
                }

                let chunk: Chunk = serde_json::from_str(&event.data)
                    .map_err(|e| Error::protocol(&profile, format!("{e}: {}", event.data)))?;

                if let Some(usage) = chunk.usage {
                    yield ChatEvent::Usage {
                        input_tokens: usage.prompt_tokens,
                        output_tokens: usage.completion_tokens,
                    };
                }

                let Some(choice) = chunk.choices.into_iter().next() else {
                    continue;
                };

                if let Some(text) = choice.delta.content.filter(|t| !t.is_empty()) {
                    yield ChatEvent::TextDelta { text };
                }

                for call in choice.delta.tool_calls {
                    let id = match call.id {
                        Some(id) => {
                            ids_by_index.insert(call.index, id.clone());
                            if let Some(name) = call.function.name.clone() {
                                yield ChatEvent::ToolCallStart { id: id.clone(), name };
                            }
                            id
                        }
                        None => match ids_by_index.get(&call.index) {
                            Some(id) => id.clone(),
                            // Some servers omit the id entirely and rely on
                            // index. Synthesize one so the call is still usable.
                            None => {
                                let id = format!("call_{}", call.index);
                                ids_by_index.insert(call.index, id.clone());
                                if let Some(name) = call.function.name.clone() {
                                    yield ChatEvent::ToolCallStart { id: id.clone(), name };
                                }
                                id
                            }
                        },
                    };
                    if let Some(partial_json) = call.function.arguments.filter(|a| !a.is_empty()) {
                        yield ChatEvent::ToolCallDelta { id, partial_json };
                    }
                }

                if let Some(reason) = choice.finish_reason {
                    stop = match reason.as_str() {
                        "tool_calls" | "function_call" => StopReason::ToolUse,
                        "length" => StopReason::MaxTokens,
                        "stop" => StopReason::EndTurn,
                        _ => StopReason::Other,
                    };
                    for id in ids_by_index.values() {
                        yield ChatEvent::ToolCallEnd { id: id.clone() };
                    }
                }
            }

            yield ChatEvent::Done { stop };
        };

        Ok(Box::pin(stream))
    }
}

/// Flatten our structured content into the shapes this dialect expects.
///
/// Tool results become their own `role: "tool"` messages here, which is the
/// main structural difference from Anthropic.
fn to_messages(request: &ChatRequest) -> Vec<Value> {
    let mut out = Vec::new();

    if let Some(system) = &request.system {
        out.push(json!({ "role": "system", "content": system }));
    }

    for message in &request.messages {
        match message.role {
            Role::User => {
                out.push(json!({ "role": "user", "content": join_text(&message.content) }));
            }
            Role::Assistant => {
                let tool_calls: Vec<Value> = message
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        Content::ToolUse { id, name, input } => Some(json!({
                            "id": id,
                            "type": "function",
                            "function": { "name": name, "arguments": input.to_string() }
                        })),
                        _ => None,
                    })
                    .collect();

                let text = join_text(&message.content);
                let mut msg = json!({ "role": "assistant" });
                // A tool-calling turn legitimately has no prose; `null` is what
                // the spec asks for there, and some servers reject `""`.
                msg["content"] = if text.is_empty() {
                    Value::Null
                } else {
                    Value::String(text)
                };
                if !tool_calls.is_empty() {
                    msg["tool_calls"] = Value::Array(tool_calls);
                }
                out.push(msg);
            }
            Role::Tool => {
                for content in &message.content {
                    if let Content::ToolResult { id, content, .. } = content {
                        out.push(json!({
                            "role": "tool",
                            "tool_call_id": id,
                            "content": content,
                        }));
                    }
                }
            }
        }
    }

    out
}

fn join_text(content: &[Content]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            Content::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

pub(super) async fn check_status(
    response: reqwest::Response,
    profile: &str,
) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let code = status.as_u16();
    if code == 401 || code == 403 {
        return Err(Error::Unauthorized {
            profile: profile.to_string(),
            status: code,
        });
    }
    Err(Error::Http {
        profile: profile.to_string(),
        status: code,
        body: response.text().await.unwrap_or_default(),
    })
}

// ─── wire types ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Deserialize)]
struct ToolCallDelta {
    #[serde(default)]
    index: u64,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: FunctionDelta,
}

#[derive(Deserialize, Default)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}
