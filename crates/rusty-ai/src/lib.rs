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
pub use agent::{Assistant, wire_history};
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
///
/// It is sent with every round of every question, so it says each thing
/// once: what a tool is for is the tool's description, and this is only what
/// holds across them.
pub const SYSTEM_PROMPT: &str = "\
You are the assistant inside rusty, a workbench for embedded Rust — mostly \
Espressif ESP32 parts, STM32 as well.

Your tools compute exact facts about the open project and this machine: the \
chip and whether the project's configuration files agree, what is installed \
against what the project needs, where the firmware's bytes went, what a Cargo \
feature costs, and attitude arithmetic. Prefer them to reasoning from file \
contents or memory, because embedded errors routinely name something other \
than their cause: an unsupported target on an ESP32, S2 or S3 usually means \
the Xtensa toolchain is missing (toolchain_status), a region overflow says \
nothing about what filled it (memory_report), and firmware that builds and \
does nothing on the board often built for the host (project_status).

list_files, search_project and read_file read the project. The file the user \
has open may come with their question, marked with its path — the whole file, \
or the part around their cursor with read_file for the rest. Read a file \
before answering about it; an answer about a file you have not read is a \
guess. Earlier questions' attachments and long tool answers are sent again \
as short notes; call the tool again if you need one.

Answer concretely and no longer than the question needs: the chip, the \
crate, the byte count, the exact command — a tool's fix command verbatim. \
Say what an attitude looks like from math_sheet's `instrument` reading, not \
from the signs of its Euler angles. If a tool needs something that is not \
open or not built yet, ask the user for it rather than guessing.";
