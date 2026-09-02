//! What a fresh machine is missing, and the order to fix it in.
//!
//! The Toolchain panel has always been able to answer "is X installed" and
//! "here is the button". What it could not do is the thing somebody who has
//! just installed the app actually needs: tell them, without being asked,
//! that four things are missing and offer to fetch all four.
//!
//! This is that list, derived from the same [`ToolchainReport`] the panel
//! draws — one derivation, so the panel and the setup screen cannot disagree
//! about what is missing. It is pure, so the ordering rules below are pinned
//! by tests rather than discovered on somebody's laptop.
//!
//! **Order is not cosmetic.** `rustup` installs everything else, so a machine
//! without it can do nothing and the list collapses to that one item. `espup`
//! must come before the Xtensa target it provides, because `rustup target add
//! xtensa-…` without it fails complaining about an unknown target. And
//! everything that blocks a build comes before anything that does not, so a
//! queue somebody interrupts halfway has fixed the parts that mattered.

use serde::{Deserialize, Serialize};

use crate::model::{ToolchainReport, ToolchainStatus};

/// Where a step's output ends up, so the screen can say so before running it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Destination {
    /// `~/.cargo/bin`, because that is where cargo puts binaries and where
    /// everything else looks for them. Redirecting it with `--root` would
    /// install espflash somewhere flashing cannot find it.
    CargoBin,
    /// rustup's own home — toolchains, components and targets.
    RustupHome,
    /// rusty's data directory, under `tools/`. The one path the user gets to
    /// choose, because it is the only one rusty owns.
    DataDirectory,
    /// Nowhere rusty controls: this step is a link, not a command.
    Manual,
}

/// One thing to install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStep {
    /// The tool's name, as `install_sim_tool` expects it.
    pub tool: String,
    /// What it is for, in the user's terms — they are about to spend disk and
    /// minutes on it and deserve to know why.
    pub purpose: String,
    /// The command line, shown before it runs. Empty for a manual step.
    pub command: String,
    pub destination: Destination,
    /// True when this one takes minutes rather than seconds, so the screen
    /// can say so rather than looking hung.
    pub slow: bool,
    /// Set when rusty cannot do it: the reason, and a URL to send them to.
    pub manual: Option<String>,
}

/// Everything missing, in the order to install it.
///
/// Empty means the machine is ready — which is what the caller shows nothing
/// for. A first-run check that appears when there is nothing to do is a
/// dialog people learn to dismiss without reading.
pub fn plan(report: &ToolchainReport) -> Vec<SetupStep> {
    // rustup first, and alone. `cargo install` and `rustup target add` are
    // both downstream of it, so listing the rest beside an absent rustup
    // offers buttons that cannot work.
    if let Some(missing) = missing_tool(&report.status, "rustup") {
        return vec![SetupStep {
            tool: "rustup".to_string(),
            purpose: missing.purpose.clone(),
            command: String::new(),
            destination: Destination::Manual,
            slow: false,
            manual: Some(
                "Rust itself is not installed. Everything else here rides on it, \
                 so install rustup first — https://rustup.rs — and rusty can do \
                 the rest."
                    .to_string(),
            ),
        }];
    }

    let mut steps = Vec::new();

    // The Xtensa toolchain before the target it provides: `rustup target add
    // xtensa-…` against a machine with no `esp` toolchain fails, and fails
    // with a message about an unknown target rather than a missing espup.
    if report.needs_esp_toolchain && !report.status.has_esp_toolchain {
        steps.push(SetupStep {
            tool: "espup".to_string(),
            purpose: "Installs the Xtensa compiler this chip needs. Nothing \
                      upstream ships one — a stock rustc cannot emit Xtensa at all."
                .to_string(),
            command: "cargo install espup --locked && espup install".to_string(),
            destination: Destination::RustupHome,
            // A gigabyte-class download. Saying so beforehand is the
            // difference between "slow" and "broken".
            slow: true,
            manual: None,
        });
    }

    // Then the tools, in the order the table lists them, required first.
    // Optional ones are offered too — somebody setting up deliberately would
    // rather fetch probe-rs now than discover it missing mid-debug — but
    // after everything that would block a build.
    let mut optional = Vec::new();
    for tool in &report.status.tools {
        if tool.path.is_some() || !tool.installable || tool.name == "rustup" {
            continue;
        }
        let step = SetupStep {
            tool: tool.name.clone(),
            purpose: tool.purpose.clone(),
            command: tool.install_command.clone(),
            destination: destination_of(&tool.name),
            slow: matches!(destination_of(&tool.name), Destination::DataDirectory),
            manual: None,
        };
        if tool.required {
            steps.push(step);
        } else {
            optional.push(step);
        }
    }
    // The target before the optional tools and after espup: espup may have
    // supplied it, and asking for it twice is a step that reports "already
    // installed" and reads as something having gone wrong. Everything that
    // blocks a build comes before anything that does not, so a queue somebody
    // interrupts halfway has fixed the parts that mattered.
    if let Some(target) = &report.required_target
        && !report.required_target_installed
        && !report.needs_esp_toolchain
    {
        steps.push(SetupStep {
            tool: format!("target:{target}"),
            purpose: "The compilation target for this chip. Without it cargo \
                      builds for this machine instead, produces a binary, and \
                      nothing happens on the board."
                .to_string(),
            command: format!("rustup target add {target}"),
            destination: Destination::RustupHome,
            slow: false,
            manual: None,
        });
    }

    steps.extend(optional);
    steps
}

/// Whether a machine can build anything at all for the open project.
///
/// Distinct from `plan(..).is_empty()`: optional tools appear in the plan and
/// do not block. This is what decides whether the setup screen opens itself.
pub fn blocked(report: &ToolchainReport) -> bool {
    plan(report)
        .iter()
        .any(|step| step.manual.is_some() || is_blocking(report, &step.tool))
}

fn is_blocking(report: &ToolchainReport, tool: &str) -> bool {
    if tool.starts_with("target:") || tool == "espup" {
        return true;
    }
    report
        .status
        .tools
        .iter()
        .any(|t| t.name == tool && t.required)
}

fn missing_tool<'a>(
    status: &'a ToolchainStatus,
    name: &str,
) -> Option<&'a crate::model::ToolStatus> {
    status
        .tools
        .iter()
        .find(|t| t.name == name && t.path.is_none())
}

/// Where a tool's install lands, by name. The backend's recipe table is the
/// authority on what each install runs; a test there checks this rule agrees
/// with it, since the two live on opposite sides of the wasm split and cannot
/// share code.
pub(crate) fn destination_of(tool: &str) -> Destination {
    if tool.starts_with("qemu-system-") || tool.ends_with("-gdb") || tool.ends_with("-gcc") {
        Destination::DataDirectory
    } else if tool == "rust-analyzer" || tool == "espup" {
        Destination::RustupHome
    } else {
        Destination::CargoBin
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ToolStatus, Toolchain};

    fn tool(name: &str, installed: bool, required: bool) -> ToolStatus {
        ToolStatus {
            name: name.to_string(),
            purpose: format!("{name} does something"),
            version: None,
            path: installed.then(|| format!("/usr/bin/{name}")),
            install_command: format!("cargo install {name}"),
            installable: name != "rustup",
            required,
        }
    }

    fn report(tools: Vec<ToolStatus>) -> ToolchainReport {
        ToolchainReport {
            status: ToolchainStatus {
                toolchains: vec![Toolchain {
                    name: "stable".to_string(),
                    is_default: true,
                    is_esp: false,
                }],
                installed_targets: Vec::new(),
                tools,
                has_esp_toolchain: false,
            },
            required_target: None,
            required_target_installed: true,
            needs_esp_toolchain: false,
            problems: Vec::new(),
        }
    }

    /// A ready machine gets no screen. One that appears with nothing to do is
    /// a dialog people learn to dismiss without reading, and then dismiss the
    /// time it mattered.
    #[test]
    fn a_complete_machine_has_nothing_to_do() {
        let r = report(vec![
            tool("rustup", true, true),
            tool("espflash", true, true),
        ]);
        assert!(plan(&r).is_empty());
        assert!(!blocked(&r));
    }

    /// Without rustup nothing else can be installed, so offering the rest is
    /// offering buttons that cannot work.
    #[test]
    fn a_machine_without_rust_is_told_that_and_nothing_else() {
        let r = report(vec![
            tool("rustup", false, true),
            tool("espflash", false, true),
            tool("probe-rs", false, false),
        ]);
        let steps = plan(&r);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].tool, "rustup");
        assert_eq!(steps[0].destination, Destination::Manual);
        assert!(
            steps[0]
                .manual
                .as_ref()
                .is_some_and(|m| m.contains("rustup.rs"))
        );
        assert!(blocked(&r));
    }

    #[test]
    fn required_tools_come_before_optional_ones() {
        let r = report(vec![
            tool("rustup", true, true),
            tool("probe-rs", false, false),
            tool("espflash", false, true),
        ]);
        let steps = plan(&r);
        let order: Vec<&str> = steps.iter().map(|s| s.tool.as_str()).collect();
        assert_eq!(order, vec!["espflash", "probe-rs"]);
    }

    /// An optional tool alone is worth offering and must not open the screen
    /// by itself — the project builds without it.
    #[test]
    fn an_optional_tool_is_offered_without_blocking() {
        let r = report(vec![
            tool("rustup", true, true),
            tool("probe-rs", false, false),
        ]);
        assert_eq!(plan(&r).len(), 1);
        assert!(!blocked(&r));
    }

    /// espup supplies the Xtensa target, so asking rustup for it separately
    /// is a step that reports "already installed" and reads as a failure.
    #[test]
    fn the_xtensa_target_is_not_asked_for_separately() {
        let mut r = report(vec![tool("rustup", true, true)]);
        r.needs_esp_toolchain = true;
        r.required_target = Some("xtensa-esp32-none-elf".to_string());
        r.required_target_installed = false;

        let steps = plan(&r);
        assert_eq!(steps.len(), 1, "{steps:?}");
        assert_eq!(steps[0].tool, "espup");
        assert!(steps[0].slow, "a gigabyte download must say so");
    }

    /// Everything that blocks a build comes before anything that does not, so
    /// a queue somebody interrupts halfway has fixed the parts that mattered.
    #[test]
    fn a_missing_target_comes_before_the_optional_tools() {
        let mut r = report(vec![
            tool("rustup", true, true),
            tool("probe-rs", false, false),
            tool("espflash", false, true),
        ]);
        r.required_target = Some("riscv32imc-unknown-none-elf".to_string());
        r.required_target_installed = false;

        let steps = plan(&r);
        let order: Vec<&str> = steps.iter().map(|s| s.tool.as_str()).collect();
        assert_eq!(
            order,
            vec!["espflash", "target:riscv32imc-unknown-none-elf", "probe-rs"]
        );
    }

    /// A RISC-V part needs the target and nothing else.
    #[test]
    fn a_riscv_target_is_added_on_its_own() {
        let mut r = report(vec![tool("rustup", true, true)]);
        r.required_target = Some("riscv32imc-unknown-none-elf".to_string());
        r.required_target_installed = false;

        let steps = plan(&r);
        assert_eq!(steps.len(), 1);
        assert_eq!(
            steps[0].command,
            "rustup target add riscv32imc-unknown-none-elf"
        );
        assert!(blocked(&r), "a missing target stops the build");
    }

    /// Where each thing lands, because the screen states it before running
    /// anything and a wrong claim there is a wrong claim about the user's
    /// disk.
    #[test]
    fn each_step_says_where_it_lands() {
        assert_eq!(destination_of("espflash"), Destination::CargoBin);
        assert_eq!(destination_of("rust-analyzer"), Destination::RustupHome);
        assert_eq!(
            destination_of("qemu-system-riscv32"),
            Destination::DataDirectory
        );
        assert_eq!(
            destination_of("xtensa-esp-elf-gdb"),
            Destination::DataDirectory
        );
    }

    /// A tool rusty has no recipe for is not offered. A button that always
    /// fails is worse than the instructions it hides.
    #[test]
    fn a_tool_with_no_recipe_is_not_offered() {
        let mut missing = tool("mystery", false, true);
        missing.installable = false;
        let r = report(vec![tool("rustup", true, true), missing]);
        assert!(plan(&r).is_empty());
    }
}
