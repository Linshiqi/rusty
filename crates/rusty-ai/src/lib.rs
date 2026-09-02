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
}

#[cfg(feature = "backend")]
impl Assistant {
    pub fn new(provider: Box<dyn Provider>) -> Self {
        Self {
            provider,
            tools: ToolRegistry::workbench(),
            system: SYSTEM_PROMPT.to_string(),
            max_tokens: 4096,
            max_turns: 8,
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

        for _ in 0..self.max_turns {
            let request = ChatRequest {
                system: Some(self.system.clone()),
                messages: history.clone(),
                tools: tools.clone(),
                max_tokens: self.max_tokens,
                temperature: None,
            };

            let mut stream = self.provider.chat(request).await?;
            let mut text = String::new();
            let mut calls = ToolCallAccumulator::default();
            let mut stop = StopReason::EndTurn;

            while let Some(event) = stream.next().await {
                let event = event?;
                match &event {
                    ChatEvent::TextDelta { text: delta } => text.push_str(delta),
                    ChatEvent::ToolCallStart { id, name } => calls.start(id.clone(), name.clone()),
                    ChatEvent::ToolCallDelta { id, partial_json } => calls.push(id, partial_json),
                    ChatEvent::Done { stop: reason } => stop = *reason,
                    _ => {}
                }
                on_event(AgentEvent::Chat(event));
            }

            let tool_uses = calls.finish();
            let mut content = Vec::new();
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
}
