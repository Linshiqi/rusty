//! The machine's Rust and Espressif tooling, and what the open project needs
//! from it.

use serde::{Deserialize, Serialize};

use super::Problem;

/// The state of the machine's Rust and Espressif tooling.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainStatus {
    /// `rustup toolchain list`, normalized to channel names.
    pub toolchains: Vec<Toolchain>,
    /// Targets installed for the toolchain the project's build will use —
    /// probed from the project's own directory, so a `rust-toolchain.toml`
    /// pin is honoured. Without a project, the machine's default toolchain.
    pub installed_targets: Vec<String>,
    /// Espressif and probe tooling found on PATH.
    pub tools: Vec<ToolStatus>,
    /// True when a toolchain named `esp` is present — the Xtensa one espup
    /// installs.
    pub has_esp_toolchain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Toolchain {
    pub name: String,
    pub is_default: bool,
    /// True for the espup-installed Xtensa toolchain.
    pub is_esp: bool,
}

/// One external binary the workbench can drive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    /// Executable name, e.g. `espflash`.
    pub name: String,
    /// What it is for, shown when it is missing so the user can decide whether
    /// they need it at all.
    pub purpose: String,
    /// What `<tool> --version` said, when it says anything. Decoration, not
    /// evidence: `ldproxy` is a linker shim with no CLI at all and panics on
    /// the flag, and treating that as "not installed" is what it used to do.
    pub version: Option<String>,
    /// Where the binary actually is. `None` is what "not installed" means,
    /// and showing it answers the question every one of these raises — which
    /// copy is being used, and on which disk it sits.
    pub path: Option<String>,
    /// How to install it, if absent.
    pub install_command: String,
    /// Whether rusty can install it itself. False means the panel offers no
    /// button: one that always fails is worse than the instructions it hides,
    /// which is the rule the chip picker already follows.
    #[serde(default)]
    pub installable: bool,
    /// False when this tool is only needed for some projects.
    pub required: bool,
}

impl ToolStatus {
    /// Presence on PATH, not a successful `--version`.
    pub fn is_installed(&self) -> bool {
        self.path.is_some()
    }
}

/// Everything the toolchain panel shows: machine state plus what this
/// particular project needs from it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainReport {
    pub status: ToolchainStatus,
    /// Target triple this project needs, when it is known.
    pub required_target: Option<String>,
    /// Whether that target is installed.
    pub required_target_installed: bool,
    /// Whether this project needs the Xtensa toolchain.
    pub needs_esp_toolchain: bool,
    pub problems: Vec<Problem>,
}
