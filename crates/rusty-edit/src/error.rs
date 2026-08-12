//! File failures, stated in terms of what the user did.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not read {path}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("could not save {path}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// rustfmt refused or could not run. The message is rustfmt's own reason,
    /// or what to install when the binary is missing — either way something
    /// the user can act on, because a save that silently skips formatting
    /// teaches people the feature is broken.
    #[error("{message}")]
    Format { message: String },

    /// A path that climbs out of the project.
    ///
    /// The frontend sends relative paths back, and `../../..` in one of them
    /// would let a panel read or overwrite anything on the machine. Refused
    /// rather than normalised, because a caller asking for it is either
    /// confused or hostile and neither deserves the file.
    #[error("{path} is outside the project")]
    Outside { path: String },
}
