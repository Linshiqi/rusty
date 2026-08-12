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

    #[error("rust-analyzer refused `{method}`: {message}")]
    Server { method: String, message: String },
}
