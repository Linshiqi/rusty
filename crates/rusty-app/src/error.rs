use serde::Serialize;

/// What a failed command sends back to the UI.
///
/// The cause chain is carried separately rather than flattened into one string:
/// for a broken workspace the outer message is generic ("could not read the
/// Cargo workspace") while cargo's own diagnostic — the actionable part — is
/// two levels down. The UI shows the head and lets the user expand the rest.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub message: String,
    pub causes: Vec<String>,
}

impl CommandError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            causes: Vec::new(),
        }
    }

    /// Nothing has been opened yet. Common on a cold start, not a bug.
    pub fn no_project() -> Self {
        Self::new("No project is open. Choose a folder containing a Cargo.toml.")
    }

    /// A project is open but `cargo metadata` did not succeed for it.
    ///
    /// Distinct from [`Self::no_project`] on purpose: a misconfigured embedded
    /// project opens fine and diagnoses fine, but has no dependency graph, and
    /// conflating the two would send the user to reopen a folder that is
    /// already open.
    pub fn no_workspace() -> Self {
        Self::new(
            "This project has no resolved Cargo workspace — `cargo metadata` did not \
             succeed. Check the Project panel for what is wrong.",
        )
    }

    fn from_source(error: &dyn std::error::Error) -> Self {
        let mut causes = Vec::new();
        let mut current = error.source();
        while let Some(cause) = current {
            causes.push(cause.to_string());
            current = cause.source();
        }
        Self {
            message: error.to_string(),
            causes,
        }
    }
}

impl From<rusty_core::Error> for CommandError {
    fn from(error: rusty_core::Error) -> Self {
        Self::from_source(&error)
    }
}

impl From<rusty_ai::Error> for CommandError {
    fn from(error: rusty_ai::Error) -> Self {
        Self::from_source(&error)
    }
}

impl From<rusty_embed::Error> for CommandError {
    fn from(error: rusty_embed::Error) -> Self {
        Self::from_source(&error)
    }
}

impl From<rusty_term::Error> for CommandError {
    fn from(error: rusty_term::Error) -> Self {
        Self::from_source(&error)
    }
}

impl From<rusty_edit::Error> for CommandError {
    fn from(error: rusty_edit::Error) -> Self {
        Self::from_source(&error)
    }
}

impl From<rusty_lsp::Error> for CommandError {
    fn from(error: rusty_lsp::Error) -> Self {
        Self::from_source(&error)
    }
}

impl From<serde_json::Error> for CommandError {
    fn from(error: serde_json::Error) -> Self {
        Self::from_source(&error)
    }
}
