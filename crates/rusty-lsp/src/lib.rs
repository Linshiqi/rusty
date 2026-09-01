//! A rust-analyzer client for the workbench.
//!
//! Split by the `backend` feature as every crate here is: [`model`] is what the
//! frontend renders — diagnostics, completions, locations, all in scalar
//! columns — and compiles to wasm32. The process and the protocol sit behind
//! `backend`.
//!
//! [`positions`] is on the wasm side with the model, because the arithmetic it
//! holds is not the server's: the editor converts scalars to UTF-16 at the DOM
//! boundary exactly as this client converts at its own, and the editor used to
//! carry its own untested copy of four of these functions.
//!
//! Its own crate for the same reason the terminal is: a language server is a
//! long-lived process with a wire protocol, and neither belongs inside the
//! crate that reads files.

pub mod model;
pub mod positions;

pub use model::*;

#[cfg(feature = "backend")]
mod client;
#[cfg(feature = "backend")]
mod error;
#[cfg(feature = "backend")]
mod rpc;

#[cfg(feature = "backend")]
pub use client::{Events, LspClient, find_rust_analyzer};
#[cfg(feature = "backend")]
pub use error::{Error, Result};
