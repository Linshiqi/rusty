//! The project's history, the way Fork shows it: a graph of commits with their
//! branches and tags, what each one changed, and the branches to move between.
//!
//! Split like every other crate here. `model` and `graph` compile to wasm and
//! do no IO — the frontend lays out nothing itself and draws exactly what the
//! backend computed, so the two cannot disagree about which lane a commit is
//! in. `repo` is behind `backend`: it runs `git` and parses its answers, and
//! the parsers are pure functions with the real output pinned in their tests.
//!
//! **`git`, not a library.** libgit2 would be a second implementation of a
//! repository format to keep in step with the one on PATH, and every question
//! this crate asks — the log, a commit's patch, the branches — is one `git`
//! invocation with a machine-readable format. The user's own git, with the
//! user's own config, credentials and hooks, is the honest thing to drive.

pub mod diff;
pub mod graph;
pub mod model;
#[cfg(feature = "backend")]
pub mod parse;
#[cfg(feature = "backend")]
pub mod repo;
pub mod url;

pub use model::*;
pub use url::repo_name;

#[cfg(feature = "backend")]
pub use repo::{Error, Result};
