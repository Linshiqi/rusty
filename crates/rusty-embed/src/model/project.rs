//! What was found out about an opened project, and what to do about it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{CommandPlan, Runtime};

/// What rusty could work out about an opened project.
///
/// Every field is optional because a half-configured project is the normal
/// state of the world — and saying "I could not tell, here is why" is more
/// useful than guessing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedProject {
    pub root: String,
    /// Chip id, if it could be determined.
    pub chip: Option<String>,
    /// How the chip was determined, so the user can correct a wrong guess.
    pub chip_source: Option<String>,
    pub runtime: Option<Runtime>,
    /// Target triple from `.cargo/config.toml`, if set.
    pub configured_target: Option<String>,
    /// Toolchain channel from `rust-toolchain.toml`, if set.
    pub configured_toolchain: Option<String>,
    /// HAL and framework crates found in the manifest.
    pub frameworks: Vec<String>,
    /// Whether `defmt` is a dependency — decides whether the monitor should
    /// decode logs or show them raw.
    pub uses_defmt: bool,
    /// Whether `embassy-executor` is present.
    pub uses_embassy: bool,
    /// What C this project already speaks, if any.
    #[serde(default)]
    pub c_interop: CInterop,
    /// Files that informed the detection, relative to the root.
    pub evidence: Vec<String>,
    /// Things that will stop a build, in the order worth fixing them.
    pub problems: Vec<Problem>,
}

/// Something wrong with the project or the machine, stated in terms of what to
/// do about it.
///
/// **`kind` and `args` are what make this translatable.** The English is still
/// here and is still the answer for anything that does not translate — the CLI
/// prints it, and so does a window whose language has no entry for this
/// diagnostic. But prose with values baked into it cannot be looked up, so the
/// stable name travels beside it and the values travel apart from it. Same
/// shape as a tool's purpose, for the same reason: the frontend keys on the
/// name, never on the sentence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub severity: Severity,
    /// A stable slug naming *which* diagnostic this is. Empty means "no
    /// name" — untranslatable, and the English below stands.
    #[serde(default)]
    pub kind: String,
    /// What `{placeholders}` in the translation stand for. The English in
    /// `title`/`detail` already has them substituted; a translation has not.
    #[serde(default)]
    pub args: BTreeMap<String, String>,
    /// One line, what is wrong.
    pub title: String,
    /// Why it matters, in the user's terms.
    pub detail: String,
    /// A command that fixes it, when one exists. Shown as a copyable button
    /// rather than run automatically — installing toolchains is the user's
    /// call, not ours.
    pub fix_command: Option<String>,
}

impl Problem {
    /// A named diagnostic with no arguments.
    ///
    /// **Every diagnostic goes through here, with its kind as a literal.** The
    /// frontend's coverage test reads the kinds off this crate's source to
    /// check each has a translation; a `Problem { .. }` literal or a computed
    /// kind is invisible to it and silently untranslated.
    pub fn new(
        severity: Severity,
        kind: &str,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Problem {
            severity,
            kind: kind.to_string(),
            args: BTreeMap::new(),
            title: title.into(),
            detail: detail.into(),
            fix_command: None,
        }
    }

    /// One `{name}` the translation may need.
    #[must_use]
    pub fn arg(mut self, name: &str, value: impl Into<String>) -> Self {
        self.args.insert(name.to_string(), value.into());
        self
    }

    /// The command that fixes it.
    #[must_use]
    pub fn fix(mut self, command: impl Into<String>) -> Self {
        self.fix_command = Some(command.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// The build cannot succeed until this is fixed.
    Blocking,
    /// The build works but something is inconsistent or suboptimal.
    Warning,
    /// Worth knowing, no action required.
    Info,
}

/// How a Rust firmware project meets C.
///
/// Reported, not guessed: every entry names the file that proves it. A
/// workbench that says "this project uses bindgen" without being able to
/// point at the line is a workbench that will eventually be wrong about it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CInterop {
    /// Crates that pull a C toolchain into the build, each with what it
    /// does — `cc`, `bindgen`, `esp-idf-sys`.
    pub via: Vec<String>,
    /// C and C++ sources carried in the project itself.
    pub sources: u32,
    /// True when the crate is built as a library C can link against —
    /// `staticlib` or `cdylib` in the manifest.
    pub exports_to_c: bool,
    /// The files that prove the above.
    pub evidence: Vec<String>,
}

impl CInterop {
    /// Whether anything at all was found — the panel shows nothing rather
    /// than an empty heading for a pure-Rust project.
    pub fn is_empty(&self) -> bool {
        self.via.is_empty() && self.sources == 0 && !self.exports_to_c
    }
}

/// What a scaffolding run wrote, on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldReport {
    pub written: Vec<String>,
    /// The dependency to add, as a command the user watches run.
    pub command: Option<CommandPlan>,
    /// The step scaffolding cannot do for you, in one sentence.
    pub next: String,
}

/// The part's pins, and what the project's own source says about them.
///
/// Two independent halves on purpose. The claims come from the source text and
/// are always available; the capabilities come from the HAL's own device
/// description and are available only when it can be found. A pin map that
/// showed one as if it were the other would be the confident wrong answer this
/// workbench is written against, so [`Self::source`] says which is which.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinReport {
    pub chip: String,
    /// Every pin the part has, in numeric order. Empty when the device
    /// description could not be read — [`Self::note`] then says why, and the
    /// claims below are still worth showing.
    pub pins: Vec<PinInfo>,
    /// Where the capabilities came from, for a user who wants to check them.
    pub source: Option<String>,
    /// Why there are no capabilities, in terms the caller can act on.
    pub note: Option<String>,
    /// Pins the source names that the part does not have. After a chip switch
    /// this is the whole of the work left, so it is not buried in `pins`.
    pub unknown: Vec<PinClaim>,
}

/// One pin, as the vendor describes it and as the project uses it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinInfo {
    pub gpio: u32,
    /// No output driver at all — ESP32's 34..39. Assigning an LED here
    /// compiles and does nothing.
    pub input_only: bool,
    /// `ADC1_CH4`, `DAC1`, `TOUCH7` — what an analog use needs.
    pub analog: Vec<String>,
    /// What the pin is wired to on essentially every module, from its
    /// function at mux level 0: the SPI flash, the USB pair, the console.
    /// Using one of these is a board that stops booting, not a compile error.
    pub reserved: Option<String>,
    /// Where the project's own source names this pin.
    pub claims: Vec<PinClaim>,
}

/// One place the source text names a pin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinClaim {
    pub gpio: u32,
    /// Project-relative, `/`-separated.
    pub file: String,
    /// Zero-based, like every line number that crosses this boundary.
    pub line: u32,
    /// The line itself, trimmed — enough to recognise the site without
    /// opening it.
    pub text: String,
}

/// What switching a project from one chip to another would change.
///
/// The plan crosses the wire and comes back to be applied, so what runs is
/// exactly what the user was shown — the same contract the flash plans have.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Migration {
    pub from: String,
    pub to: String,
    /// Files that would be edited, in the order they would be written.
    pub files: Vec<FileChange>,
    /// What this deliberately does not do. Never empty for a plan that can
    /// run: a chip switch that said nothing about pins would be exactly the
    /// plausible-looking answer that costs somebody an afternoon.
    pub notes: Vec<String>,
    /// Why it will not run at all, when it will not.
    pub blocker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Project-relative, `/`-separated.
    pub path: String,
    pub edits: Vec<Edit>,
}

/// One substitution. An empty `before` appends; an empty `after` deletes.
///
/// `before` is the exact text as it stands in the file — line endings, quote
/// style and spacing included — because `apply` re-reads the file and refuses
/// when the text is not there. A canonical respelling of what was read would
/// fail that check on a file nobody had touched.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edit {
    pub before: String,
    pub after: String,
}
