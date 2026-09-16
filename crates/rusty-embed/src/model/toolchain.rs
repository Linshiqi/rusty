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

/// Whether this cargo is the one rust-analyzer asks for a flag it no longer
/// has — the reason an Xtensa project loads with no dependency graph.
///
/// rust-analyzer hands `cargo metadata` a copy of the lockfile so the real
/// one is not rewritten under the user, and picks how to say so from the
/// toolchain's version: `--lockfile-path` below 1.95, `-Zlockfile-path` with
/// `CARGO_RESOLVER_LOCKFILE_PATH` up to 1.97, the variable alone after that.
/// cargo dropped the flag *in* 1.95 — and a nightly is a pre-release, which
/// semver puts **below** the version it is becoming. So a cargo calling
/// itself `1.95.0-nightly` is asked for the flag it has just lost, every
/// `cargo metadata` fails with `unexpected argument '--lockfile-path'`, and
/// rust-analyzer carries on without dependencies.
///
/// Exactly that one version, and only as a pre-release: `1.94.0-nightly`
/// still has the flag and `1.96.0-nightly` is asked the new way, so both are
/// fine, and a released `1.95.0` is never a nightly at all. Espressif's fork
/// sits on `1.95.0-nightly`, which is why this is the normal state of an
/// Xtensa project rather than a bad week on nightly.
pub fn cargo_loses_dependencies(version_line: &str) -> bool {
    let Some(version) = version_line.split_whitespace().nth(1) else {
        return false;
    };
    let Some((number, pre)) = version.split_once('-') else {
        return false;
    };
    !pre.is_empty() && number == "1.95.0"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window is one version wide and the pre-release is the whole of
    /// it: a rule that read "1.95 or older" would condemn every toolchain
    /// anybody has used for a year, and one that ignored the pre-release
    /// would condemn the release that fixed it.
    #[test]
    fn only_a_nightly_of_the_one_version_loses_its_dependencies() {
        assert!(cargo_loses_dependencies(
            "cargo 1.95.0-nightly (f2d3ce0bd 2026-03-21) (1.95.0.0)"
        ));
        assert!(cargo_loses_dependencies(
            "cargo 1.95.0-beta.2 (abc 2026-01-01)"
        ));
        assert!(
            !cargo_loses_dependencies("cargo 1.94.0-nightly (abc 2025-12-01)"),
            "1.94's cargo still has the flag rust-analyzer sends it",
        );
        assert!(
            !cargo_loses_dependencies("cargo 1.97.0-nightly (abc 2026-05-01)"),
            "from 1.95 the copy is named by an environment variable instead",
        );
        assert!(
            !cargo_loses_dependencies("cargo 1.95.0 (0e3d7325 2026-04-02)"),
            "the release is not a pre-release, and sorts above the boundary",
        );
        assert!(!cargo_loses_dependencies(
            "cargo 1.98.1 (48a229ce 2026-09-01)"
        ));
        assert!(!cargo_loses_dependencies("cargo"));
        assert!(!cargo_loses_dependencies(""));
    }
}
