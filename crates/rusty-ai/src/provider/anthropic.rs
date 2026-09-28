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
    model::{ChatEvent, Content, ProviderKind, Role, StopReason},
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
    async fn chat(&self, request: ChatRequest) -> Result<EventStream> {
        let endpoint = self.endpoint();
        let profile = self.profile.clone();

        let mut body = json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "stream": true,
            "messages": to_messages(&request),
        });
        // Anthropic caches a prompt only where it is told to, and the prompt
        // is mostly repetition: the tools and this system prompt, about three
        // and a half thousand tokens, open every request, and each round of
        // the agent loop sends the whole of the round before it again. So two
        // breakpoints — here, which holds the tools and the system prompt
        // (the cache's order is tools, system, messages), and on the last
        // block of the conversation (`to_messages`), which the next round
        // then reads back at a tenth of the price. A prompt shorter than the
        // model's minimum is simply not cached.
        if let Some(system) = &request.system {
            body["system"] = json!([{
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" },
            }]);
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

        let request = super::authorize(
            ProviderKind::Anthropic,
            self.http.post(&endpoint),
            Some(&self.api_key),
        )
        .json(&body);
        let response = super::send(request, &endpoint, &profile).await?;

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
        // The prompt's size arrives at the start and the answer's at the
        // end, as running totals; reported once, whole, before `Done`. Two
        // halves reported apart read as a round that took no input.
        let mut usage: Option<Usage> = None;

        while let Some(event) = events.next().await {
            let event = event.map_err(|e| Error::protocol(&profile, e.to_string()))?;
            let parsed: StreamEvent = super::parse_data(&event.data, &profile)?;

            match parsed {
                StreamEvent::MessageStart { message } => {
                    if let Some(report) = message.usage {
                        usage = Some(usage.unwrap_or_default().and(report));
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
                    BlockDelta::ThinkingDelta { thinking } => {
                        yield ChatEvent::ThinkingDelta { text: thinking };
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
                StreamEvent::MessageDelta { delta, usage: report } => {
                    if let Some(reason) = delta.stop_reason {
                        stop = match reason.as_str() {
                            "tool_use" => StopReason::ToolUse,
                            "max_tokens" => StopReason::MaxTokens,
                            "end_turn" | "stop_sequence" => StopReason::EndTurn,
                            _ => StopReason::Other,
                        };
                    }
                    if let Some(report) = report {
                        usage = Some(usage.unwrap_or_default().and(report));
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

        if let Some(usage) = usage {
            yield usage.event();
        }
        yield ChatEvent::Done { stop };
    };

    Box::pin(stream)
}

/// Anthropic has no `tool` role: results go back as *user* messages containing
/// `tool_result` blocks. That reshaping is the whole reason this function
/// differs from the OpenAI one.
///
/// Two blocks carry cache breakpoints (see `chat`). The last, because the
/// next round's request begins with everything up to it, byte for byte. And
/// the last before the question being answered, because the conversation
/// before it goes out in every round of every later question exactly as it
/// does now — `wire_history` makes it so — and the next question can read
/// it back from there.
fn to_messages(request: &ChatRequest) -> Vec<Value> {
    let mut messages = blocks_of(request);
    let question = request
        .messages
        .iter()
        .rposition(|message| message.role == Role::User);
    let before = question.and_then(|at| at.checked_sub(1));
    for at in [before, messages.len().checked_sub(1)]
        .into_iter()
        .flatten()
    {
        let last = messages[at]["content"]
            .as_array_mut()
            .and_then(|blocks| blocks.last_mut());
        // An empty text block cannot carry a breakpoint; the API refuses it.
        if let Some(block) = last.filter(|block| block["text"] != "") {
            block["cache_control"] = json!({ "type": "ephemeral" });
        }
    }
    messages
}

fn blocks_of(request: &ChatRequest) -> Vec<Value> {
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
                .filter_map(|content| {
                    Some(match content {
                        Content::Text { text } => json!({ "type": "text", "text": text }),
                        // An attached file is a text block framed as the file
                        // it is — the same prose the OpenAI path sends.
                        Content::Attachment { .. } => json!({
                            "type": "text",
                            "text": content.prose().unwrap_or_default(),
                        }),
                        // What the model thought is for the reader; sent back
                        // it would be an unsigned thinking block, which the API
                        // refuses.
                        Content::Thinking { .. } => return None,
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
    /// Extended thinking, when a model streams it. Shown, never sent back.
    ThinkingDelta {
        thinking: String,
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

/// A usage report. `input_tokens` is only the part of the prompt after the
/// last cache breakpoint once caching is on: the whole prompt is that, what
/// the cache was written with and what it supplied. Every count is optional
/// because the compatible servers that speak this dialect leave some out or
/// send `null`.
#[derive(Deserialize, Default, Clone, Copy)]
struct Usage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
}

impl Usage {
    /// This report taken into what came before. The counts are running
    /// totals — `message_start` has the prompt and an output of one,
    /// `message_delta` the output so far and, from newer servers, the prompt
    /// again — so the larger of the two is the one that is current.
    fn and(self, later: Usage) -> Usage {
        let most = |a: Option<u32>, b: Option<u32>| a.max(b);
        Usage {
            input_tokens: most(self.input_tokens, later.input_tokens),
            cache_creation_input_tokens: most(
                self.cache_creation_input_tokens,
                later.cache_creation_input_tokens,
            ),
            cache_read_input_tokens: most(
                self.cache_read_input_tokens,
                later.cache_read_input_tokens,
            ),
            output_tokens: most(self.output_tokens, later.output_tokens),
        }
    }

    fn event(self) -> ChatEvent {
        let count = |n: Option<u32>| n.unwrap_or(0);
        let cached = count(self.cache_read_input_tokens);
        ChatEvent::Usage {
            input_tokens: count(self.input_tokens)
                .saturating_add(count(self.cache_creation_input_tokens))
                .saturating_add(cached),
            cached_tokens: cached,
            output_tokens: count(self.output_tokens),
        }
    }
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

        assert!(matches!(&events[0], ChatEvent::TextDelta { text } if text == "Hello"));
        // The prompt from the start and the answer from the end, as one
        // report. Reported apart, the window showed the second: a round
        // that took no input at all.
        assert!(
            matches!(
                &events[1],
                ChatEvent::Usage {
                    input_tokens: 25,
                    cached_tokens: 0,
                    output_tokens: 3
                }
            ),
            "{events:?}"
        );
        assert!(matches!(
            events.last(),
            Some(ChatEvent::Done {
                stop: StopReason::EndTurn
            })
        ));
        assert_eq!(events.len(), 3, "the ping produced nothing, as it should");
    }

    /// With caching on, `input_tokens` is only what came after the last
    /// breakpoint; the prompt is that, what was written to the cache and
    /// what was read from it. A newer server repeats the prompt's counts in
    /// `message_delta`, and some send `null` where they have nothing.
    #[tokio::test]
    async fn the_prompt_is_counted_whole_with_the_cache_read_named() {
        let body = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":",
            "{\"input_tokens\":40,\"cache_creation_input_tokens\":200,",
            "\"cache_read_input_tokens\":3400,\"output_tokens\":1}}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":",
            "{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":40,",
            "\"cache_creation_input_tokens\":null,\"cache_read_input_tokens\":3400,",
            "\"output_tokens\":12}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let events: Vec<ChatEvent> = replay(body)
            .await
            .into_iter()
            .collect::<Result<_>>()
            .expect("a clean stream");
        let usage: Vec<&ChatEvent> = events
            .iter()
            .filter(|e| matches!(e, ChatEvent::Usage { .. }))
            .collect();
        assert!(
            matches!(
                usage[..],
                [ChatEvent::Usage {
                    input_tokens: 3640,
                    cached_tokens: 3400,
                    output_tokens: 12
                }]
            ),
            "{events:?}"
        );
    }

    fn request(messages: Vec<crate::model::Message>) -> ChatRequest {
        ChatRequest {
            system: Some("You are a test.".into()),
            messages,
            tools: Vec::new(),
            max_tokens: 16,
            temperature: None,
        }
    }

    /// The breakpoint rides the last block of the conversation, whatever
    /// kind of block that is, and only that one: the next round's request
    /// begins with everything up to it.
    #[test]
    fn the_last_block_carries_the_conversations_cache_breakpoint() {
        use crate::model::Message;

        let messages = to_messages(&request(vec![
            Message::user("How big is the firmware?"),
            Message::assistant(vec![Content::ToolUse {
                id: "t1".into(),
                name: "memory_report".into(),
                input: json!({}),
            }]),
            Message {
                role: Role::Tool,
                content: vec![Content::ToolResult {
                    id: "t1".into(),
                    content: "{\"flash\":85300}".into(),
                    is_error: false,
                }],
            },
        ]));
        assert_eq!(marked(&messages), [(2, 0)], "{messages:#?}");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
    }

    /// Which blocks carry `cache_control`, as (message, block).
    fn marked(messages: &[Value]) -> Vec<(usize, usize)> {
        messages
            .iter()
            .enumerate()
            .flat_map(|(m, message)| {
                message["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .filter(|(_, block)| block.get("cache_control").is_some())
                    .map(move |(b, _)| (m, b))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// From the second question on, the conversation before it is a
    /// breakpoint too: it goes out unchanged in every later question, so the
    /// next one reads it from the cache rather than at full price.
    #[test]
    fn the_conversation_before_the_question_is_a_breakpoint_of_its_own() {
        use crate::model::Message;

        let messages = to_messages(&request(vec![
            Message::user("Which chip is this?"),
            Message::assistant(vec![Content::Text {
                text: "An ESP32-C3.".into(),
            }]),
            Message::user("And what is missing?"),
            Message::assistant(vec![Content::ToolUse {
                id: "t1".into(),
                name: "toolchain_status".into(),
                input: json!({}),
            }]),
            Message {
                role: Role::Tool,
                content: vec![Content::ToolResult {
                    id: "t1".into(),
                    content: "{}".into(),
                    is_error: false,
                }],
            },
        ]));
        assert_eq!(marked(&messages), [(1, 0), (4, 0)], "{messages:#?}");
    }

    /// An empty text block cannot carry a breakpoint — the API refuses the
    /// request — so none is set there.
    #[test]
    fn an_empty_text_block_is_left_unmarked() {
        let messages = to_messages(&request(vec![crate::model::Message::user("")]));
        assert!(messages[0]["content"][0].get("cache_control").is_none());
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
