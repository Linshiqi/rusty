/// Errors from the AI layer.
///
/// These are shown to the user verbatim, and with bring-your-own keys the most
/// common failures are configuration mistakes rather than bugs — so each
/// variant says what to fix, not just what broke.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no API key stored for provider `{profile}` — add one in Settings")]
    MissingKey { profile: String },

    #[error("`{profile}` rejected the API key (HTTP {status})")]
    Unauthorized { profile: String, status: u16 },

    #[error("`{profile}` returned HTTP {status}: {body}")]
    Http {
        profile: String,
        status: u16,
        body: String,
    },

    #[error("could not reach `{endpoint}` — check the base URL and your network")]
    Transport {
        endpoint: String,
        #[source]
        source: Box<reqwest::Error>,
    },

    #[error("`{profile}` sent a response this client could not parse: {detail}")]
    Protocol { profile: String, detail: String },

    #[error("no tool named `{0}`")]
    UnknownTool(String),

    /// A tool was called before the thing it inspects existed.
    ///
    /// Phrased for the model rather than the user, because it goes back as a
    /// tool result: telling it what to ask for is what stops it inventing an
    /// answer instead.
    #[error("this needs {needed}. {hint}")]
    MissingContext { needed: String, hint: String },

    #[error("tool `{name}` was called with invalid arguments: {detail}")]
    BadToolArguments { name: String, detail: String },

    #[error("the OS credential store is unavailable")]
    Keychain(#[from] keyring::Error),

    #[error(transparent)]
    Analysis(#[from] rusty_core::Error),

    #[error(transparent)]
    Embedded(#[from] rusty_embed::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn transport(endpoint: impl Into<String>, source: reqwest::Error) -> Self {
        Error::Transport {
            endpoint: endpoint.into(),
            source: Box::new(source),
        }
    }

    pub(crate) fn protocol(profile: impl Into<String>, detail: impl Into<String>) -> Self {
        Error::Protocol {
            profile: profile.into(),
            detail: detail.into(),
        }
    }
}
