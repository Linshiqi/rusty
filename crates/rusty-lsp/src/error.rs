//! Language-server failures, stated in terms of what to do.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// Nothing to spawn. The message names the fix because the symptom —
    /// no squiggles, no completion — does not.
    #[error(
        "rust-analyzer is not installed. `rustup component add rust-analyzer \
         --toolchain stable` puts it in the stable toolchain, which is the one \
         rusty uses even for projects pinned to another."
    )]
    NotFound,

    #[error("could not start rust-analyzer")]
    Spawn(#[source] std::io::Error),

    #[error("could not talk to rust-analyzer")]
    Io(#[source] std::io::Error),

    /// The server took longer than the request budget. Common while the index
    /// is cold; callers that can retry, retry.
    #[error("rust-analyzer did not answer `{method}` in time")]
    Timeout { method: String },

    /// The server went away with the request outstanding. Distinct from a
    /// timeout because it is answered at once: every caller waiting on a
    /// dead server used to sit out the full budget to learn the same thing.
    #[error("rust-analyzer exited before answering `{method}`")]
    Exited { method: String },

    #[error("rust-analyzer refused `{method}`: {message}")]
    Server { method: String, message: String },

    /// A call hierarchy item handed back that is not the JSON the server
    /// sent. It crosses to the frontend and back untouched, so this is
    /// damage on the way, not anything the server said.
    #[error("the call hierarchy item is not the server's JSON: {0}")]
    Item(#[source] serde_json::Error),

    /// An edit the server sent could not be put on disk — a file it names
    /// could not be read or written — and nothing is left changed. Every
    /// file is read before any is written, and a write that fails once
    /// others have succeeded puts those back first: a rename lands whole or
    /// not at all, because half of one is a build that fails in the caller
    /// you did not see. `undone` names the files that were written and then
    /// put back, none when the failure came before any write.
    #[error("could not apply the edit to {path}: {source} — {}", nothing_changed(.undone))]
    Apply {
        path: String,
        #[source]
        source: std::io::Error,
        undone: Vec<String>,
    },

    /// A write failed part-way through an edit, and so did putting back the
    /// files written before it: the edit is half on disk. `left` names every
    /// file that no longer holds what it held, for somebody to look at.
    #[error(
        "could not apply the edit to {path}: {source} — and could not put back \
         what had been written, so these are left changed: {}",
        .left.join(", ")
    )]
    PartlyApplied {
        path: String,
        #[source]
        source: std::io::Error,
        left: Vec<String>,
    },
}

/// How [`Error::Apply`] ends: nothing changed, and whether that took
/// putting anything back.
fn nothing_changed(undone: &[String]) -> &'static str {
    if undone.is_empty() {
        "nothing was changed"
    } else {
        "what had been written was put back, so nothing was changed"
    }
}
