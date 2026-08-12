//! Looking at and changing the project's files.
//!
//! Split by the `backend` feature as every crate here is: [`model`] is the tree
//! and the highlighted document the Leptos frontend draws, and compiles to
//! wasm32; reading, walking and highlighting sit behind `backend`.
//!
//! Files and highlighting only. Completion, diagnostics and navigation come
//! from rust-analyzer over LSP and live in their own crate — a language server
//! is a long-running process with a protocol, and bolting it onto the code that
//! reads a file would put a socket behind `open`.

pub mod model;

pub use model::*;

#[cfg(feature = "backend")]
mod document;
#[cfg(feature = "backend")]
mod error;
#[cfg(feature = "backend")]
mod format;
#[cfg(feature = "backend")]
mod highlight;
#[cfg(feature = "backend")]
mod search;
#[cfg(feature = "backend")]
mod tree;

#[cfg(feature = "backend")]
pub use document::{Files, save};
#[cfg(feature = "backend")]
pub use error::{Error, Result};
#[cfg(feature = "backend")]
pub use format::format_rust;
#[cfg(feature = "backend")]
pub use search::search;
#[cfg(feature = "backend")]
pub use tree::read as read_tree;
