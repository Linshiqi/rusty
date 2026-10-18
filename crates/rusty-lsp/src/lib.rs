//! A rust-analyzer client for the workbench.
//!
//! [`model`] is what the editor draws — diagnostics, completions,
//! locations, all in scalar columns.
//!
//! [`positions`] is column arithmetic — scalars, UTF-8 and UTF-16 — in one
//! place, so every conversion follows one rule. [`follow`]
//! is what the server said about one text, carried across an edit to the
//! next — the client's diagnostics and the editor's inlay hints by one
//! rule. And [`squiggle`]: where a diagnostic is drawn on its line, for
//! the editor that draws one and the hover that has to find it there.
//!
//! Its own crate for the same reason the terminal is: a language server is a
//! long-lived process with a wire protocol, and neither belongs inside the
//! crate that reads files.
//!
//! The client is one concern per module: `client` is the session and
//! its transport, `discover` finds and starts the binary, `pull` owns
//! diagnostic freshness, `convert` turns replies into the model, `uri` is the
//! one place a path becomes a URI or comes back, `rpc` frames bytes.

pub mod complete;
pub mod follow;
pub mod model;
pub mod positions;
pub mod squiggle;

pub use model::*;

mod client;
mod convert;
mod discover;
mod error;
mod pull;
mod rpc;
mod uri;
pub mod watched;

pub use client::{Events, LspClient};
pub use discover::{find_clangd, find_rust_analyzer};
pub use error::{Error, Result};
pub use uri::percent_decode;
