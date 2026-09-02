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
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ChatRequest, EventStream, Provider};
use crate::{
    error::{Error, Result},
    http,
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
    /// `http` is built by [`crate::config::build`] with the crate's one client
    /// policy — proxy, timeouts — rather than here, so a provider cannot
    /// quietly end up with a different one.
    pub fn new(
        profile: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        supports_tools: bool,
        http: reqwest::Client,
    ) -> Self {
        Self {
            profile: profile.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key: api_key.into(),
            http,
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

        let request = self
            .http
            .post(&endpoint)
            .bearer_auth(&self.api_key)
            .json(&body);
        let response = http::send_retrying(request, &endpoint).await?;
        let response = super::check_status(response, &profile).await?;

        Ok(decode(response.bytes_stream(), profile))
    }
}

/// The SSE body as events.
///
/// Separate from `chat` and generic over the byte source so a fixture can be
/// fed to it: every rule below about what a line means was written against a
/// real stream, and a test that could not replay one would leave those rules
/// unchecked until the next real stream broke them.
///
/// Two policies, stated because they were once different on the two
/// providers. **A data line that does not parse ends the stream with an error
/// naming the line.** Skipping it would be a guess that it carried nothing.
/// **A provider that reports failure inside a 200** — `{"error": …}` here —
/// is an [`Error::Upstream`] carrying its own words, because the message is
/// the diagnosis. Every field of a chunk defaults so that keepalive chunks
/// parse, and that is exactly what let an error chunk parse as an empty one
/// and be skipped: the user saw an empty answer and no error.
pub(crate) fn decode<S, B, E>(source: S, profile: String) -> EventStream
where
    S: Stream<Item = std::result::Result<B, E>> + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    let stream = try_stream! {
        // Pinned here rather than demanding `S: Unpin`: reqwest's byte stream
        // is not, and a fixture stream in a test need not be either.
        let mut events = std::pin::pin!(source.eventsource());
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

            if let Some(error) = chunk.error {
                Err(Error::Upstream {
                    profile: profile.clone(),
                    message: error.message(),
                })?;
            }

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

    Box::pin(stream)
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
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

// ─── wire types ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
    /// A failure reported inside a 200. See [`decode`] for why it has to be
    /// modelled rather than left to the defaults.
    #[serde(default)]
    error: Option<ApiError>,
}

/// The provider's error, in the two spellings seen in the wild: OpenAI's
/// object with a `message`, and Ollama's bare string.
#[derive(Deserialize)]
#[serde(untagged)]
enum ApiError {
    Text(String),
    Object { message: String },
}

impl ApiError {
    fn message(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::Object { message } => message,
        }
    }
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

#[cfg(test)]
mod tests {
    use futures_util::stream;

    use super::*;

    /// One SSE body through the decoder, exactly as the bytes off the socket
    /// would go.
    async fn replay(body: &'static str) -> Vec<Result<ChatEvent>> {
        decode(
            stream::iter([Ok::<&[u8], std::convert::Infallible>(body.as_bytes())]),
            "test".to_string(),
        )
        .collect()
        .await
    }

    /// The bug: every field of a chunk defaults, so an error chunk parsed as
    /// an empty chunk, was skipped, and the user saw an empty answer with no
    /// error anywhere.
    #[tokio::test]
    async fn an_error_inside_a_200_is_the_providers_message_not_an_empty_answer() {
        let events = replay(
            "data: {\"error\":{\"message\":\"You exceeded your current quota\",\
             \"type\":\"insufficient_quota\"}}\n\ndata: [DONE]\n\n",
        )
        .await;
        let error = events
            .into_iter()
            .find_map(Result::err)
            .expect("the stream must end in an error, not in a quiet Done");
        assert!(
            matches!(&error, Error::Upstream { message, .. } if message.contains("exceeded your current quota")),
            "{error}",
        );
    }

    #[tokio::test]
    async fn ollamas_bare_string_error_is_read_too() {
        let events = replay("data: {\"error\":\"model 'nope' not found\"}\n\n").await;
        let error = events.into_iter().find_map(Result::err).expect("an error");
        assert!(
            matches!(&error, Error::Upstream { message, .. } if message == "model 'nope' not found"),
            "{error}",
        );
    }

    #[tokio::test]
    async fn a_line_that_is_not_json_ends_the_stream_naming_the_line() {
        let events = replay("data: <html>bad gateway</html>\n\n").await;
        let error = events.into_iter().find_map(Result::err).expect("an error");
        assert!(
            matches!(&error, Error::Protocol { detail, .. } if detail.contains("<html>bad gateway</html>")),
            "{error}",
        );
    }

    #[tokio::test]
    async fn text_and_a_tool_call_are_assembled_from_the_dialect() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Let me \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"check.\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",",
            "\"function\":{\"name\":\"project_status\",\"arguments\":\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,",
            "\"function\":{\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );
        let events: Vec<ChatEvent> = replay(body)
            .await
            .into_iter()
            .collect::<Result<_>>()
            .expect("a clean stream");

        assert!(matches!(&events[0], ChatEvent::TextDelta { text } if text == "Let me "));
        assert!(matches!(&events[1], ChatEvent::TextDelta { text } if text == "check."));
        assert!(matches!(
            &events[2],
            ChatEvent::ToolCallStart { id, name } if id == "call_1" && name == "project_status"
        ));
        assert!(matches!(
            &events[3],
            ChatEvent::ToolCallDelta { id, partial_json } if id == "call_1" && partial_json == "{}"
        ));
        assert!(matches!(&events[4], ChatEvent::ToolCallEnd { id } if id == "call_1"));
        assert!(matches!(
            &events[5],
            ChatEvent::Usage {
                input_tokens: 10,
                output_tokens: 5
            }
        ));
        assert!(matches!(
            events.last(),
            Some(ChatEvent::Done {
                stop: StopReason::ToolUse
            })
        ));
        assert_eq!(events.len(), 7);
    }
}
