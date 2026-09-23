//! The agent loop: ask, run whatever tools the model calls, ask again.

use std::sync::atomic::{AtomicU32, Ordering};

use futures_util::StreamExt;

use crate::{
    error::{Error, Result},
    model::{AgentEvent, ChatEvent, Content, DEFAULT_MAX_TOKENS, Message, StopReason, ToolDef},
    provider::{ChatRequest, EventStream, Provider},
    tools::{ToolContext, ToolRegistry},
};

/// The agent loop: ask, run whatever tools the model calls, ask again.
pub struct Assistant {
    provider: Box<dyn Provider>,
    tools: ToolRegistry,
    system: String,
    max_tokens: u32,
    /// Ceiling on tool round-trips per question. A model that keeps calling
    /// tools without concluding would otherwise spend the user's money in a
    /// loop — and with BYO keys that is their money, not ours.
    max_turns: usize,
    /// The output cap the provider named while refusing `max_tokens`, when it
    /// did; zero until then. Read by the host after a question so the next
    /// one starts there instead of being refused again.
    learned_cap: AtomicU32,
}

impl Assistant {
    pub fn new(provider: Box<dyn Provider>) -> Self {
        Self {
            provider,
            tools: ToolRegistry::workbench(),
            system: crate::SYSTEM_PROMPT.to_string(),
            max_tokens: DEFAULT_MAX_TOKENS,
            max_turns: 8,
            learned_cap: AtomicU32::new(0),
        }
    }

    /// Replace the built-in registry with one the caller assembled.
    ///
    /// The extension seam, and today an unused one: an MCP client will
    /// `register` a server's tools on a [`ToolRegistry`] and hand it here. It
    /// stays public for that consumer rather than being carved out and put
    /// back, because the shape of the loop — one registry, chosen before the
    /// first turn — is the part worth fixing now.
    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Run one question to completion, executing tool calls along the way.
    ///
    /// `history` is appended to in place, so the caller keeps the conversation
    /// and can send it back for the next question.
    /// `on_event` is `Send` so the returned future is too — Tauri's async
    /// command handlers require it, and a callback that pins the loop to one
    /// thread would be a poor default regardless.
    pub async fn ask(
        &self,
        ctx: &ToolContext<'_>,
        history: &mut Vec<Message>,
        on_event: &mut (dyn FnMut(AgentEvent) + Send),
    ) -> Result<()> {
        let tools = if self.provider.supports_tools() {
            self.tools.defs()
        } else {
            Vec::new()
        };

        let mut max_tokens = self.max_tokens;
        for _ in 0..self.max_turns {
            let stream = self.open_turn(history, &tools, &mut max_tokens).await?;
            let turn = read_turn(stream, on_event).await?;
            if !turn.content.is_empty() {
                history.push(Message::assistant(turn.content));
            }

            // A model can signal tool use without emitting a parseable call;
            // treating that as "done" is better than looping on nothing.
            if turn.stop != StopReason::ToolUse || turn.tool_uses.is_empty() {
                return Ok(());
            }

            let results = self.run_tools(&turn.tool_uses, ctx, on_event);
            history.push(Message::tool_results(results));
        }

        Ok(())
    }

    /// The stream of the next turn.
    ///
    /// A provider whose model caps output below what was asked refuses the
    /// request and names its cap in the refusal. Asking again at the named
    /// cap is not a guess — it is the provider's own number — and it is what
    /// lets one large default serve every provider. Each pass lowers the
    /// budget strictly, so this ends.
    async fn open_turn(
        &self,
        history: &[Message],
        tools: &[ToolDef],
        max_tokens: &mut u32,
    ) -> Result<EventStream> {
        loop {
            let request = ChatRequest {
                system: Some(self.system.clone()),
                messages: history.to_vec(),
                tools: tools.to_vec(),
                max_tokens: *max_tokens,
                temperature: None,
            };
            match self.provider.chat(request).await {
                Ok(stream) => return Ok(stream),
                Err(error) => match output_cap_named_in(&error, *max_tokens) {
                    Some(cap) => {
                        *max_tokens = cap;
                        self.learned_cap.store(cap, Ordering::Relaxed);
                    }
                    None => return Err(error),
                },
            }
        }
    }

    /// Every call the model made, run, with what each answered.
    fn run_tools(
        &self,
        tool_uses: &[Content],
        ctx: &ToolContext<'_>,
        on_event: &mut (dyn FnMut(AgentEvent) + Send),
    ) -> Vec<Content> {
        tool_uses
            .iter()
            .filter_map(|use_| match use_ {
                Content::ToolUse { id, name, input } => {
                    on_event(AgentEvent::ToolStarted {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    });

                    // Tool failures go back to the model as results rather
                    // than aborting: a bad argument is something it can fix
                    // on the next turn, and the user gets an answer instead
                    // of an error dialog.
                    let (content, is_error) = match self.tools.call(name, input, ctx) {
                        Ok(value) => (value.to_string(), false),
                        Err(e) => (e.to_string(), true),
                    };

                    on_event(AgentEvent::ToolFinished {
                        id: id.clone(),
                        name: name.clone(),
                        ok: !is_error,
                    });

                    Some(Content::ToolResult {
                        id: id.clone(),
                        content,
                        is_error,
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// The output cap the provider named while refusing the budget it was
    /// asked for, if it refused. The host remembers it for the next question,
    /// so a provider is refused once per session rather than once per ask.
    pub fn learned_cap(&self) -> Option<u32> {
        match self.learned_cap.load(Ordering::Relaxed) {
            0 => None,
            cap => Some(cap),
        }
    }
}

/// One turn of the model's, as it streamed.
struct Turn {
    /// What it said, as the history keeps it: the thinking first, as it
    /// happened, then the text, then the calls.
    content: Vec<Content>,
    /// The calls it made, their arguments parsed.
    tool_uses: Vec<Content>,
    stop: StopReason,
}

/// Read one turn's stream to its end, passing every event on as it comes.
async fn read_turn(
    mut stream: EventStream,
    on_event: &mut (dyn FnMut(AgentEvent) + Send),
) -> Result<Turn> {
    let mut text = String::new();
    let mut thinking = String::new();
    let mut calls = ToolCallAccumulator::default();
    let mut stop = StopReason::EndTurn;

    while let Some(event) = stream.next().await {
        let event = event?;
        match &event {
            ChatEvent::TextDelta { text: delta } => text.push_str(delta),
            ChatEvent::ThinkingDelta { text: delta } => thinking.push_str(delta),
            ChatEvent::ToolCallStart { id, name } => calls.start(id.clone(), name.clone()),
            ChatEvent::ToolCallDelta { id, partial_json } => calls.push(id, partial_json),
            ChatEvent::Done { stop: reason } => stop = *reason,
            _ => {}
        }
        on_event(AgentEvent::Chat(event));
    }

    let tool_uses = calls.finish();
    let mut content = Vec::new();
    // A turn that is all thinking — the budget spent before the answer
    // began — is still kept, so the transcript shows what the money bought.
    if !thinking.is_empty() {
        content.push(Content::Thinking { text: thinking });
    }
    if !text.is_empty() {
        content.push(Content::Text { text });
    }
    content.extend(tool_uses.iter().cloned());
    Ok(Turn {
        content,
        tool_uses,
        stop,
    })
}

/// Streamed tool-call fragments, assembled into finished calls.
///
/// Both wire formats deliver a call's arguments as partial JSON across many
/// events, and both decoders pass the fragments on as they arrive; this is
/// where they become one call.
#[derive(Debug, Default)]
struct ToolCallAccumulator {
    calls: Vec<(String, String, String)>, // id, name, partial json
}

impl ToolCallAccumulator {
    fn start(&mut self, id: String, name: String) {
        self.calls.push((id, name, String::new()));
    }

    fn push(&mut self, id: &str, fragment: &str) {
        if let Some(call) = self.calls.iter_mut().find(|c| c.0 == id) {
            call.2.push_str(fragment);
        }
    }

    /// Finished calls, with arguments parsed. A call whose JSON never became
    /// valid is returned with a null input so the caller can report a tool
    /// error rather than silently dropping the model's intent.
    fn finish(self) -> Vec<Content> {
        self.calls
            .into_iter()
            .map(|(id, name, json)| Content::ToolUse {
                id,
                name,
                input: serde_json::from_str(&json).unwrap_or(serde_json::Value::Null),
            })
            .collect()
    }
}

/// The output cap a provider named in a refusal of `asked`, when it named one.
///
/// Only a refusal is read — a 4xx before the stream, or the error a server
/// puts inside a 200 — and only the provider's own words, never this crate's
/// framing: the status code in "answered 400" would otherwise read as a cap
/// of 400.
fn output_cap_named_in(error: &Error, asked: u32) -> Option<u32> {
    match error {
        Error::Http { status, body, .. } if (400..500).contains(status) => {
            output_cap_in(body, asked)
        }
        Error::Upstream { message, .. } => output_cap_in(message, asked),
        _ => None,
    }
}

/// The cap named in a provider's message about output tokens: the largest
/// whole number below `asked`, from a message that is about output at all.
///
/// Written against three refusals, verbatim in the tests: Anthropic's
/// `max_tokens: 200000 > 64000, which is the maximum allowed number of
/// output tokens for …`, OpenAI's `max_tokens is too large: 200000. This
/// model supports at most 16384 completion tokens …`, and DeepSeek's `the
/// valid range of max_tokens is [1, 8192]`. The largest number below the ask
/// rather than the smallest, because a message names small numbers for other
/// reasons; nothing under 256, because no model's output cap is.
fn output_cap_in(message: &str, asked: u32) -> Option<u32> {
    let lower = message.to_ascii_lowercase();
    let about_output = [
        "max_tokens",
        "max tokens",
        "output token",
        "completion token",
        "context length",
        "maximum context",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if !about_output {
        return None;
    }
    lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == ','))
        .filter(|token| {
            token.chars().next().is_some_and(|c| c.is_ascii_digit())
                && token.chars().all(|c| c.is_ascii_digit() || c == ',')
        })
        .filter_map(|token| token.replace(',', "").parse::<u32>().ok())
        .filter(|&n| n >= 256 && n < asked)
        .max()
}

#[cfg(test)]
mod tests {
    use super::output_cap_in;

    #[test]
    fn the_cap_a_provider_names_is_read_off_its_refusal() {
        assert_eq!(
            output_cap_in(
                "max_tokens: 200000 > 64000, which is the maximum allowed number of output \
                 tokens for claude-sonnet-4-5-20250929",
                200_000,
            ),
            Some(64_000),
        );
        assert_eq!(
            output_cap_in(
                "max_tokens is too large: 200000. This model supports at most 16384 \
                 completion tokens, whereas you provided 200000.",
                200_000,
            ),
            Some(16_384),
        );
        assert_eq!(
            output_cap_in(
                "Invalid max_tokens value, the valid range of max_tokens is [1, 8192]",
                200_000,
            ),
            Some(8_192),
        );
        assert_eq!(
            output_cap_in(
                "{\"error\":{\"message\":\"max_tokens is too large: 200,000. This model \
                 supports at most 16,384 completion tokens\",\"code\":400}}",
                200_000,
            ),
            Some(16_384),
            "thousands separators are read, and a code beside the message is not a cap",
        );
    }

    #[test]
    fn a_refusal_about_something_else_names_no_cap() {
        assert_eq!(
            output_cap_in(
                "The model `gpt-5-mini-2025` does not exist or you do not have access to it.",
                200_000,
            ),
            None,
        );
        assert_eq!(
            output_cap_in("max_tokens must be at least 1", 200_000),
            None,
            "no number below the ask that could be a cap",
        );
        assert_eq!(
            output_cap_in("max_tokens: 200000 > 200000", 200_000),
            None,
            "the ask itself is not a cap below it",
        );
    }
}
