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
mod agent;
#[cfg(feature = "backend")]
pub mod config;
#[cfg(feature = "backend")]
mod error;
#[cfg(feature = "backend")]
pub mod http;
#[cfg(feature = "backend")]
pub mod mcp;
#[cfg(feature = "backend")]
pub mod provider;
#[cfg(feature = "backend")]
pub mod secrets;
#[cfg(feature = "backend")]
pub mod tools;

#[cfg(feature = "backend")]
pub use agent::Assistant;
#[cfg(feature = "backend")]
pub use error::{Error, Result};
#[cfg(feature = "backend")]
pub use http::Http;
#[cfg(feature = "backend")]
pub use provider::{ChatRequest, Provider};
#[cfg(feature = "backend")]
pub use tools::{LazyWorkspace, Tool, ToolContext, ToolRegistry};

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

For rotations — quaternions, Euler angles, frames, gravity in the body, gyro \
integration — call math_sheet rather than working them out yourself: attitude \
code goes wrong at its conventions, and the tool states them with every answer. \
Called without rows it works out the user's own sheet from the Math panel, with \
the live values the panel shows. Say what an attitude looks like from its \
`instrument` reading — nose up or down, bank, heading — never from the signs of \
its Euler angles, which mean opposite things with Z up and Z down.

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
