//! Terminal failures, stated in terms of what went wrong.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// The pseudo-terminal itself refused. On Windows this usually means
    /// ConPTY is unavailable, which is a Windows older than 1809.
    #[error("could not open a terminal: {0}")]
    Pty(String),

    #[error("could not send input to the terminal")]
    Write(#[source] std::io::Error),
}

impl Error {
    /// The pseudo-terminal's refusal, in its own words.
    pub(crate) fn pty(error: impl std::fmt::Display) -> Self {
        Error::Pty(error.to_string())
    }
}
