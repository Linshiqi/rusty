//! The project's history, the way Fork shows it: a graph of commits with their
//! branches and tags, what each one changed, and the branches to move between.
//!
//! `model` and `graph` do no IO — the window lays out nothing itself and
//! draws exactly what `graph` computed, so which lane a commit is in is one
//! fact. `repo` runs `git` and parses its answers, and the parsers are pure
//! functions with the real output pinned in their tests.
//!
//! **`git`, not a library.** libgit2 would be a second implementation of a
//! repository format to keep in step with the one on PATH, and every question
//! this crate asks — the log, a commit's patch, the branches — is one `git`
//! invocation with a machine-readable format. The user's own git, with the
//! user's own config, credentials and hooks, is the honest thing to drive.

pub mod diff;
pub mod graph;
pub mod log;
pub mod model;
pub(crate) mod parse;
pub mod plan;
pub mod repo;
pub mod url;

pub use model::*;
pub use url::repo_name;

pub use repo::{Error, Result};
