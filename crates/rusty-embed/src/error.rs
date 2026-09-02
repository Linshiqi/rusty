//! The one error type this crate answers with.
//!
//! One, deliberately. For a while there were three — this enum, a second one
//! in `scaffold`, and a dozen functions answering `Result<_, String>` — and a
//! caller had to know which module it was talking to before it could decide
//! how to fail. The strings were already actionable sentences, so the two
//! variants that absorbed them carry the sentence and nothing else.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("`{path}` could not be read")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("`{path}` is not valid TOML")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    #[error("no Cargo.toml at `{0}` — open the folder that contains one")]
    NotACargoProject(String),

    #[error("`{path}` could not be written")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// A file rusty was asked to create that is already there. Scaffolding
    /// refuses before its first write, so this arrives before anything has
    /// changed.
    #[error("{path} already exists — rusty will not overwrite code you wrote")]
    Exists { path: String },

    /// A value that would not serialise. A bug in rusty rather than in the
    /// user's file, and said so rather than blamed on the disk.
    #[error("`{path}` could not be encoded ({detail}) — this is a bug in rusty, please report it")]
    Encode { path: String, detail: String },

    /// The configuration store itself misbehaving — a relocation refused, an
    /// anchor that cannot exist. Stated in the user's terms because the fix is
    /// always theirs to make.
    #[error("{detail}")]
    Config { detail: String },

    /// Something rusty declines to do, in terms the caller can act on: a tool
    /// it has no recipe for, a migration whose plan no longer matches the
    /// file, a debugger nobody has built for this platform. The sentence is
    /// the whole answer — refusing rather than guessing is the rule, and a
    /// refusal that did not say why would be a guess in disguise.
    #[error("{detail}")]
    Refused { detail: String },

    /// A fetch that failed on every route. The detail names the last route
    /// and what it said, because "download failed" alone points nowhere: whether
    /// the proxy, the TLS or the socket refused is the entire diagnosis.
    #[error("{detail}")]
    Download { detail: String },

    #[error("`{path}` is not a readable ELF ({detail}) — build the project first")]
    Elf { path: String, detail: String },

    #[error("could not run `{tool}` — is it on PATH?")]
    Spawn {
        tool: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{chip} has no serial bootloader — flash it through a debug probe instead")]
    NoSerialBootloader { chip: String },

    /// A port that would not open. Usually something else already has it: a
    /// running monitor, another window, a terminal left attached. The OS says
    /// "access denied", which reads like a driver fault, so name the cause.
    #[error("{port} would not open ({message}) — a monitor or another program may already hold it")]
    SerialPort { port: String, message: String },

    #[error("`{chip}` is not a part rusty knows about")]
    UnknownChip { chip: String },

    #[error("{chip} has no {runtime} target — that combination cannot be built")]
    UnsupportedRuntime { chip: String, runtime: String },

    /// A generator option that cannot work without another one.
    ///
    /// Caught before running rather than after: `esp-generate` rejects the whole
    /// invocation with "Invalid options provided", which arrives once the user
    /// has already chosen where the project should go.
    #[error("`{option}` cannot be used without `{required}` — turn that on as well.")]
    MissingOption { option: String, required: String },

    #[error(
        "probe-rs needs an exact target name for `{chip}`, which depends on package and \
         flash size. Run `probe-rs chip list` and pick the one matching your board."
    )]
    UnknownProbeTarget { chip: String },
}

impl Error {
    /// A refusal, in one sentence the caller can act on.
    pub(crate) fn refused(detail: impl Into<String>) -> Self {
        Error::Refused {
            detail: detail.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
