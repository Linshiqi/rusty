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
/// Which `.rs` files no `mod` declaration reaches, so the tree can dim them.
pub mod modules;
/// Which tests a Rust file holds, so the gutter can offer to run them.
pub mod tests_in;

pub use fold::{Folded, Region};
pub mod guides;
pub mod lens;
/// The minimap's arithmetic: runs of ink, and where it stands.
pub mod minimap;
/// A server's colours over the painter's.
pub mod semantic;
/// Sticky scroll: which lines stay at the top.
pub mod sticky;
/// What a test harness said: each test's verdict, each failure's place.
pub mod testrun;
/// Modal editing: Vim's keys as a pure state machine.
pub mod vim;
pub use model::*;
pub use tests_in::{Runnable, RunnableKind};

#[cfg(feature = "backend")]
mod document;
#[cfg(feature = "backend")]
mod entries;
#[cfg(feature = "backend")]
mod error;
#[cfg(feature = "backend")]
mod format;
/// The one rule for what is not shown, searched or watched, and the walk
/// that applies it.
#[cfg(feature = "backend")]
mod hidden;
#[cfg(feature = "backend")]
mod highlight;
#[cfg(feature = "backend")]
mod search;
#[cfg(feature = "backend")]
pub mod text_file;
#[cfg(feature = "backend")]
mod tree;
#[cfg(feature = "backend")]
mod watch;

#[cfg(feature = "backend")]
pub use document::{Files, create, is_library_source, read_bytes, save};
#[cfg(feature = "backend")]
pub use entries::{absolute, copy_entry, delete_entry, move_entry, rename_entry};
#[cfg(feature = "backend")]
pub use error::{Error, Result};
#[cfg(feature = "backend")]
pub use format::{clang_style, format_c, format_rust};
#[cfg(feature = "backend")]
pub use modules::scan as scan_unlinked;
#[cfg(feature = "backend")]
pub use search::{Query as SearchQuery, replace, search};
#[cfg(feature = "backend")]
pub use tree::read as read_tree;
#[cfg(feature = "backend")]
pub use watch::{Watch, watch};
