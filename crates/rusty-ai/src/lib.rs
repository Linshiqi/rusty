//! The AI layer of the rusty workbench.
//!
//! Two commitments shape this crate:
//!
//! **Bring your own model.** Users supply their own endpoint and key — a
//! frontier API, a domestic provider, or a model running on their own machine.
//! Keys live in the OS credential store and every request is issued from Rust,
//! so credentials never reach the WebView. See [`config`] and [`secrets`].
//!
//! **The analyses are the tools.** The assistant does not read
//! `.cargo/config.toml` and guess at why a build fails. It calls into
//! [`rusty_embed`] and [`rusty_core`] and gets the actual toolchain mismatch,
//! the actual per-crate flash use, the actual resolved dependency graph. That
//! matters most in this domain, where errors habitually point away from their
//! cause. See [`tools`].
//!
//! ```no_run
//! use rusty_ai::{AgentEvent, Assistant, Http, Message, ToolContext, config, secrets};
//! use rusty_core::Workspace;
//!
//! # async fn demo() -> Result<(), rusty_ai::Error> {
//! let workspace = Workspace::load(".")?;
//! let settings: rusty_ai::ProviderConfig = todo!("from settings");
//!
//! // The key and the proxy are the host's to resolve: the keychain read is
//! // blocking IO, and the proxy is a workbench setting.
//! let key = secrets::load(&settings.profile)?;
//! let assistant = Assistant::new(config::build(&settings, key, &Http::default())?);
//! let context = ToolContext::with_workspace(&workspace)
//!     .with_firmware("target/riscv32imc-unknown-none-elf/release/blinky");
//! let mut history = vec![Message::user("Why won't this fit in flash?")];
//!
//! assistant
//!     .ask(&context, &mut history, &mut |event: AgentEvent| {
//!         if let AgentEvent::Chat(chat) = event {
//!             // stream to the UI
//!             let _ = chat;
//!         }
//!     })
//!     .await?;
//! # Ok(())
//! # }
//! ```

pub mod model;
pub use model::*;

#[cfg(feature = "backend")]
pub mod config;
#[cfg(feature = "backend")]
mod error;
#[cfg(feature = "backend")]
pub mod http;
#[cfg(feature = "backend")]
pub mod provider;
#[cfg(feature = "backend")]
pub mod secrets;
#[cfg(feature = "backend")]
pub mod tools;

#[cfg(feature = "backend")]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(feature = "backend")]
use futures_util::StreamExt;

#[cfg(feature = "backend")]
pub use error::{Error, Result};
#[cfg(feature = "backend")]
pub use http::Http;
#[cfg(feature = "backend")]
pub use provider::{ChatRequest, Provider};
#[cfg(feature = "backend")]
pub use tools::{Tool, ToolContext, ToolRegistry};

#[cfg(feature = "backend")]
use provider::ToolCallAccumulator;

/// What the assistant is told about itself.
///
/// The load-bearing instruction is the one about preferring tools over
/// inference. Embedded Rust is unusually good at producing errors that point
/// away from their cause, and a model reading those strings will write a
/// fluent, plausible, wrong answer. The tools exist so it does not have to
/// guess — and the prompt has to say so, because guessing is the default.
pub const SYSTEM_PROMPT: &str = "\
You are the assistant inside rusty, a workbench for embedded Rust. The user is \
most likely working on an Espressif ESP32 part; STM32 is also supported.

You have tools that compute exact facts about the open project: which chip it \
targets and whether its four configuration files agree, what is installed on \
this machine versus what the project needs, where the firmware's bytes went by \
crate, and what a Cargo feature selection really costs. Prefer them over \
reasoning from file contents or from memory.

You can also read the project itself: list_files shows what it contains, \
search_project finds where something is mentioned, and read_file returns a \
file's text with line numbers. The user may send the file they have open \
along with their question; it arrives in their message, marked with its \
path. For a question about a document, a chapter or a piece of code, read it \
before answering — every file in the project is one call away, and an answer \
about a file you have not read is a guess.

This matters more here than in most domains, because embedded errors routinely \
name something other than their cause:

- An unsupported-target error on an ESP32, S2 or S3 usually means the Xtensa \
  toolchain is missing. rustc never mentions espup. Check toolchain_status \
  before theorising.
- A linker message saying a region overflowed names a byte count and nothing \
  about what filled it. Call memory_report; it attributes bytes to crates.
- A project that builds but does nothing on the board often has no target \
  configured at all, so cargo silently built for the host. project_status \
  reports that directly.

Be concrete: name the chip, the crate, the byte count, the exact command. When \
a tool reports a fix command, give it verbatim. When a number looks surprising, \
say why — initialised data costs both flash and RAM; two coupled features can \
each show zero because either one alone keeps the shared dependency alive.

If a tool says it needs something that is not open or not built yet, ask the \
user for it. Do not substitute a guess.";

/// The agent loop: ask, run whatever tools the model calls, ask again.
#[cfg(feature = "backend")]
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

#[cfg(feature = "backend")]
impl Assistant {
    pub fn new(provider: Box<dyn Provider>) -> Self {
        Self {
            provider,
            tools: ToolRegistry::workbench(),
            system: SYSTEM_PROMPT.to_string(),
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
            // A provider whose model caps output below what was asked refuses
            // the request and names its cap in the refusal. Asking again at
            // the named cap is not a guess — it is the provider's own number
            // — and it is what lets one large default serve every provider.
            // Each pass lowers the budget strictly, so this ends.
            let mut stream = loop {
                let request = ChatRequest {
                    system: Some(self.system.clone()),
                    messages: history.clone(),
                    tools: tools.clone(),
                    max_tokens,
                    temperature: None,
                };
                match self.provider.chat(request).await {
                    Ok(stream) => break stream,
                    Err(error) => match output_cap_named_in(&error, max_tokens) {
                        Some(cap) => {
                            max_tokens = cap;
                            self.learned_cap.store(cap, Ordering::Relaxed);
                        }
                        None => return Err(error),
                    },
                }
            };
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
            // The thinking first, as it happened; a turn that is all thinking
            // — the budget spent before the answer began — is still kept, so
            // the transcript shows what the money bought.
            if !thinking.is_empty() {
                content.push(Content::Thinking { text: thinking });
            }
            if !text.is_empty() {
                content.push(Content::Text { text });
            }
            content.extend(tool_uses.iter().cloned());
            if !content.is_empty() {
                history.push(Message::assistant(content));
            }

            // A model can signal tool use without emitting a parseable call;
            // treating that as "done" is better than looping on nothing.
            if stop != StopReason::ToolUse || tool_uses.is_empty() {
                return Ok(());
            }

            let results = tool_uses
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
                .collect();

            history.push(Message::tool_results(results));
        }

        Ok(())
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

/// The output cap a provider named in a refusal of `asked`, when it named one.
///
/// Only a refusal is read — a 4xx before the stream, or the error a server
/// puts inside a 200 — and only the provider's own words, never this crate's
/// framing: the status code in "answered 400" would otherwise read as a cap
/// of 400.
#[cfg(feature = "backend")]
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
#[cfg(feature = "backend")]
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

#[cfg(all(test, feature = "backend"))]
mod cap_tests {
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
