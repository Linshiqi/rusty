//! Looking at and changing the project's files.
//!
//! [`model`] is the tree and the highlighted document, as data; reading,
//! walking and highlighting are the modules beside it.
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

mod document;
mod entries;
mod error;
mod format;
/// The one rule for what is not shown, searched or watched, and the walk
/// that applies it.
mod hidden;
mod highlight;
mod search;
pub mod text_file;
mod tree;
mod watch;

pub use document::{Files, create, is_library_source, read_bytes, save};
pub use entries::{
    NameProblem, absolute, copy_entry, delete_entry, drop_target, is_within, move_entry,
    moved_path, name_problem, name_problem_on, parent_dir, rename_entry,
};
pub use error::{Error, Result};
pub use format::{clang_style, format_c, format_rust};
pub use modules::scan as scan_unlinked;
pub use search::{Query as SearchQuery, replace, search};
pub use tree::read as read_tree;
pub use watch::{Watch, watch};
