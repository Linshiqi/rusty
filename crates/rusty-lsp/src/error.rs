//! Language-server failures, stated in terms of what to do.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// Nothing to spawn. The message names the fix because the symptom —
    /// no squiggles, no completion — does not.
    #[error(
        "rust-analyzer is not installed. `rustup component add rust-analyzer` \
         puts it in the stable toolchain, which is the one rusty uses even for \
         projects pinned to another."
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

    /// An edit the server sent could not be put on disk — the file it names
    /// could not be read or written. Nothing was applied: a rename lands
    /// whole or not at all, because half of one is a build that fails in
    /// the caller you did not see.
    #[error("could not apply the edit to {path}: {source}")]
    Apply {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
