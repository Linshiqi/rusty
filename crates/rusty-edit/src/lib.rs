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

/// Code folding: what can collapse, and the line map that follows.
pub mod fold;
pub mod lexical;
pub mod model;
/// Which tests a Rust file holds, so the gutter can offer to run them.
pub mod tests_in;

pub use fold::{Folded, Region};
pub use model::*;
pub use tests_in::{Runnable, RunnableKind};

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
mod watch;

#[cfg(feature = "backend")]
pub use document::{Files, create, save};
#[cfg(feature = "backend")]
pub use error::{Error, Result};
#[cfg(feature = "backend")]
pub use format::format_rust;
#[cfg(feature = "backend")]
pub use search::{Query as SearchQuery, search};
#[cfg(feature = "backend")]
pub use tree::read as read_tree;
#[cfg(feature = "backend")]
pub use watch::{Watch, watch};
