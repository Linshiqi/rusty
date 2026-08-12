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

    /// The configuration store itself misbehaving — a relocation refused, an
    /// anchor that cannot exist. Stated in the user's terms because the fix is
    /// always theirs to make.
    #[error("{detail}")]
    Config { detail: String },

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

pub type Result<T> = std::result::Result<T, Error>;
