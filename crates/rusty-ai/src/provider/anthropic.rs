//! Anthropic Messages API.
//!
//! Kept native rather than routed through an OpenAI-compatible shim: the tool
//! use format, the system prompt placement, and the streaming event model are
//! all different enough that a shim would lose information — particularly
//! `input_json_delta`, which is how tool arguments actually arrive.

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

/// Wire version pinned deliberately: Anthropic requires the header and a
/// floating value would make failures non-reproducible. Shared with the model
/// listing, which speaks to the same API.
pub(crate) const API_VERSION: &str = "2023-06-01";

pub struct Anthropic {
    profile: String,
    base_url: String,
    model: String,
    api_key: String,
    http: reqwest::Client,
}

impl Anthropic {
    /// `http` is built by [`crate::config::build`] with the crate's one client
    /// policy — proxy, timeouts — rather than here, so a provider cannot
    /// quietly end up with a different one.
    pub fn new(
        profile: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        http: reqwest::Client,
    ) -> Self {
        Self {
            profile: profile.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key: api_key.into(),
            http,
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

        let request = self
            .http
            .post(&endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&body);
        let response = http::send_retrying(request, &endpoint).await?;
        let response = super::check_status(response, &profile).await?;

        Ok(decode(response.bytes_stream(), profile))
    }
}

/// The SSE body as events. Same shape and same two policies as the
/// OpenAI-compatible decoder, and for the same reasons — see there.
///
/// This side used to skip any data line it could not parse, on the theory
/// that `ping` and other bookkeeping had shapes we did not model. They do not
/// need skipping: every event carries a `type`, and an unknown one parses as
/// [`StreamEvent::Other`]. What the skip actually hid was a line that was not
/// an event at all — a proxy's HTML error page, say — which then read as a
/// stream that ended early with nothing to say.
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
        // Content blocks are addressed by index; tool ids arrive once, at
        // block start, and every later delta only carries the index.
        let mut tool_ids: Vec<(u64, String)> = Vec::new();
        let mut stop = StopReason::EndTurn;

        while let Some(event) = events.next().await {
            let event = event.map_err(|e| Error::protocol(&profile, e.to_string()))?;
            let parsed: StreamEvent = serde_json::from_str(&event.data)
                .map_err(|e| Error::protocol(&profile, format!("{e}: {}", event.data)))?;

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
                    Err(Error::Upstream {
                        profile: profile.clone(),
                        message: error.message,
                    })?;
                }
                StreamEvent::Other => {}
            }
        }

        yield ChatEvent::Done { stop };
    };

    Box::pin(stream)
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
    /// `ping`, and whatever the API adds next. Bookkeeping, not failure.
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

#[cfg(test)]
mod tests {
    use futures_util::stream;

    use super::*;

    async fn replay(body: &'static str) -> Vec<Result<ChatEvent>> {
        decode(
            stream::iter([Ok::<&[u8], std::convert::Infallible>(body.as_bytes())]),
            "test".to_string(),
        )
        .collect()
        .await
    }

    /// An overloaded server is not a parse failure, and the message is the
    /// server's own.
    #[tokio::test]
    async fn an_error_event_carries_the_providers_words() {
        let events = replay(
            "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\
             \"message\":\"Overloaded\"}}\n\n",
        )
        .await;
        let error = events
            .into_iter()
            .find_map(Result::err)
            .expect("an error event ends the stream in an error");
        assert!(
            matches!(&error, Error::Upstream { message, .. } if message == "Overloaded"),
            "{error}",
        );
    }

    /// The old decoder skipped this line and ended the stream quietly, so a
    /// proxy's error page read as a model with nothing to say.
    #[tokio::test]
    async fn a_line_that_does_not_parse_is_an_error_not_a_skip() {
        let events = replay(
            "event: message_delta\ndata: this is not json\n\n\
             event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        )
        .await;
        let error = events
            .into_iter()
            .find_map(Result::err)
            .expect("an unreadable line is reported, not skipped");
        assert!(
            matches!(&error, Error::Protocol { detail, .. } if detail.contains("this is not json")),
            "{error}",
        );
    }

    #[tokio::test]
    async fn pings_are_bookkeeping_and_the_answer_streams_through() {
        let body = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":",
            "{\"input_tokens\":25,\"output_tokens\":1}}}\n\n",
            "event: ping\ndata: {\"type\":\"ping\"}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,",
            "\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,",
            "\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":",
            "{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let events: Vec<ChatEvent> = replay(body)
            .await
            .into_iter()
            .collect::<Result<_>>()
            .expect("a clean stream");

        assert!(matches!(
            &events[0],
            ChatEvent::Usage {
                input_tokens: 25,
                output_tokens: 1
            }
        ));
        assert!(matches!(&events[1], ChatEvent::TextDelta { text } if text == "Hello"));
        assert!(matches!(
            &events[2],
            ChatEvent::Usage {
                input_tokens: 0,
                output_tokens: 3
            }
        ));
        assert!(matches!(
            events.last(),
            Some(ChatEvent::Done {
                stop: StopReason::EndTurn
            })
        ));
        assert_eq!(events.len(), 4, "the ping produced nothing, as it should");
    }

    #[tokio::test]
    async fn tool_arguments_are_routed_by_block_index() {
        let body = concat!(
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,",
            "\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"memory_report\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,",
            "\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"a\\\":\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,",
            "\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"1}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":",
            "{\"stop_reason\":\"tool_use\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let events: Vec<ChatEvent> = replay(body)
            .await
            .into_iter()
            .collect::<Result<_>>()
            .expect("a clean stream");

        assert!(matches!(
            &events[0],
            ChatEvent::ToolCallStart { id, name } if id == "toolu_1" && name == "memory_report"
        ));
        assert!(matches!(
            &events[1],
            ChatEvent::ToolCallDelta { id, partial_json } if id == "toolu_1" && partial_json == "{\"a\":"
        ));
        assert!(matches!(
            &events[2],
            ChatEvent::ToolCallDelta { id, partial_json } if id == "toolu_1" && partial_json == "1}"
        ));
        assert!(matches!(&events[3], ChatEvent::ToolCallEnd { id } if id == "toolu_1"));
        assert!(matches!(
            events.last(),
            Some(ChatEvent::Done {
                stop: StopReason::ToolUse
            })
        ));
    }
}
