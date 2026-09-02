use std::fmt;

/// Errors surfaced by `rusty-core`.
///
/// These are meant to be rendered directly in the UI, so each variant carries
/// enough context to be actionable without a stack trace.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `cargo metadata` failed — usually a broken manifest or an unresolvable
    /// dependency. The underlying cargo output is the useful part here.
    #[error("could not read the Cargo workspace at `{path}`")]
    Metadata {
        path: String,
        #[source]
        source: Box<guppy::Error>,
    },

    /// `cargo metadata` could not be started, or exited unsuccessfully. The
    /// detail is cargo's own stderr, which names the manifest line at fault.
    #[error("could not run `cargo metadata` for `{path}`: {detail}")]
    CargoRun { path: String, detail: String },

    /// A package name was supplied that is not a workspace member.
    #[error("`{name}` is not a member of this workspace")]
    NotAMember { name: String },

    /// A feature was requested that the package does not declare.
    #[error("`{package}` has no feature named `{feature}`")]
    UnknownFeature { package: String, feature: String },

    /// Anything guppy rejects while walking the graph.
    #[error(transparent)]
    Graph(#[from] guppy::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn metadata(path: impl fmt::Display, source: guppy::Error) -> Self {
        Error::Metadata {
            path: path.to_string(),
            source: Box::new(source),
        }
    }
}
