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

    /// Asked to act on a terminal that is not open. Worth naming rather than
    /// silently ignoring: it means the frontend and backend disagree about
    /// whether a tab exists.
    #[error("no terminal is open")]
    NotOpen,
}
