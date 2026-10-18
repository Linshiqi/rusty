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
    // Only where Rust is built: a PlatformIO or CMake project's row says
    // `required: false`, and its user was told Rust was missing.
    if let Some(missing) = missing_tool(&report.status, "rustup")
        && missing.required
    {
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

    // Then the host's C toolchain, alone, for the same reason: an `-msvc`
    // rustc links through Visual Studio's, a Mac's through the Xcode Command
    // Line Tools', a Linux's through `cc`, and without it every `cargo
    // install` below compiles for a minute and dies with "linker not found"
    // — on the first run, on the machine of somebody who has just installed
    // Rust and has no idea what a linker is. Each row exists only on the host
    // it applies to (`toolchain::HOST_C_TOOLCHAINS`), so at most one of these
    // arms is ever reached.
    for name in crate::toolchain::HOST_C_TOOLCHAINS {
        let Some(missing) = missing_tool(&report.status, name) else {
            continue;
        };
        let manual = match name {
            "msvc" => "The Windows C++ build tools are not installed, so nothing `cargo \
                       install` compiles can be linked. Install Build Tools for Visual \
                       Studio with the \"Desktop development with C++\" workload — \
                       https://visualstudio.microsoft.com/visual-cpp-build-tools/ — then \
                       rusty can do the rest."
                .to_string(),
            "xcode-clt" => format!(
                "The Xcode Command Line Tools are not installed, so this Mac has no C \
                 compiler or linker for anything `cargo install` builds — and no git. Run \
                 `{}` in a terminal (Apple's own dialog takes it from there), then rusty \
                 can do the rest.",
                crate::toolchain::XCODE_CLT_INSTALL
            ),
            // The row carries the distribution's own package line, read off
            // /etc/os-release where the report was made.
            _ => format!(
                "No C compiler answers on this machine, so nothing `cargo install` \
                 compiles can be linked. Install your distribution's C toolchain — {} — \
                 then rusty can do the rest.",
                missing.install_command
            ),
        };
        return vec![SetupStep {
            tool: name.to_string(),
            purpose: missing.purpose.clone(),
            command: String::new(),
            destination: Destination::Manual,
            slow: false,
            manual: Some(manual),
        }];
    }

    let mut steps = Vec::new();

    // The Xtensa toolchain before the target it provides: `rustup target add
    // xtensa-…` against a machine with no `esp` toolchain fails, and fails
    // with a message about an unknown target rather than a missing espup.
    // The command is the recipe table's, read off espup's own row, so the
    // pinned toolchain version is spelled in one place.
    if report.needs_esp_toolchain && !report.status.has_esp_toolchain {
        let command = report
            .status
            .tools
            .iter()
            .find(|t| t.name == "espup")
            .map(|t| t.install_command.clone())
            .unwrap_or_else(|| "cargo install espup --locked && espup install".to_string());
        steps.push(SetupStep {
            tool: "espup".to_string(),
            purpose: "Installs the Xtensa compiler this chip needs. Nothing \
                      upstream ships one — a stock rustc cannot emit Xtensa at all."
                .to_string(),
            command,
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
    // A target the build compiles `core` for has nothing for rustup to add;
    // what it needs is nightly with the source, when that is what is missing.
    if report.builds_std
        && report
            .problems
            .iter()
            .any(|problem| problem.kind == "nightly-missing")
    {
        steps.push(SetupStep {
            tool: "nightly".to_string(),
            purpose: "Nightly Rust with the standard library's source: this chip's target \
                      has no ready-made library, so the build compiles `core` itself."
                .to_string(),
            command: crate::model::ToolchainRequirement::NightlyBuildStd
                .install_command()
                .unwrap_or_default()
                .to_string(),
            destination: Destination::RustupHome,
            slow: true,
            manual: None,
        });
    }
    if let Some(target) = &report.required_target
        && !report.required_target_installed
        && !report.needs_esp_toolchain
        && !report.builds_std
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
    if tool.starts_with("target:") || tool == "espup" || tool == "nightly" {
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

/// Where a tool's install lands, by name. `toolchain`'s recipe table is the
/// authority on what each install runs; a test there checks this rule agrees
/// with it.
pub(crate) fn destination_of(tool: &str) -> Destination {
    if tool.starts_with("qemu-system-") || tool.ends_with("-gdb") || tool.ends_with("-gcc") {
        Destination::DataDirectory
    } else if tool == "rust-analyzer" || tool == "espup" || tool == "nightly" {
        Destination::RustupHome
    } else {
        Destination::CargoBin
    }
}

// --- the Environment page's answer ---

/// Which group of work a tool serves. By name, because the report is a flat
/// list and the grouping is this page's opinion; an unknown tool goes with
/// the editing tools rather than nowhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Build,
    Device,
    Editor,
}

pub fn section_of(tool: &str) -> Section {
    // A cross C compiler builds, whichever part it is for.
    if tool.ends_with("-gcc") {
        return Section::Build;
    }
    if crate::toolchain::HOST_C_TOOLCHAINS.contains(&tool) {
        return Section::Build;
    }
    match tool {
        "ldproxy" | "pio" | "cmake" | "ninja" | "idf.py" => Section::Build,
        "espflash" | "probe-rs" | "wlink" | "codelldb" => Section::Device,
        _ => Section::Editor,
    }
}

/// The tools this project cannot do without that are not there, in the
/// order they install: the required tools, the Xtensa toolchain and the
/// chip's target — read off the report's own fields, the way the setup plan
/// reads them, so this page and the rows under it cannot disagree — and
/// then what the report's blocking problems name: a project with a chip
/// and no flasher cannot reach its board. Targets are `target:<triple>`,
/// as the setup plan spells them.
pub fn needed(report: &ToolchainReport) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |name: String| {
        if !out.contains(&name) {
            out.push(name);
        }
    };
    for tool in &report.status.tools {
        if tool.required && !tool.is_installed() {
            add(tool.name.clone());
        }
    }
    if report.needs_esp_toolchain && !report.status.has_esp_toolchain {
        add("espup".to_string());
    }
    // An Xtensa target comes with the Xtensa toolchain, which is the row
    // that is missing then.
    if let Some(target) = &report.required_target
        && !report.required_target_installed
        && !target.starts_with("xtensa-")
        && !report.builds_std
    {
        add(format!("target:{target}"));
    }
    for problem in &report.problems {
        if problem.severity != crate::model::Severity::Blocking {
            continue;
        }
        match problem.kind.as_str() {
            // The tool the part is usually flashed with, which the
            // problem names; espflash for a report from before it did.
            "no-flasher" => add(problem
                .args
                .get("tool")
                .cloned()
                .unwrap_or_else(|| "espflash".to_string())),
            "nightly-missing" => add("nightly".to_string()),
            "ldproxy-missing" => add("ldproxy".to_string()),
            _ => {}
        }
    }
    out
}

/// Whether rusty's own install can fetch a missing item: a target, nightly
/// or espup always; a tool when its row says so.
pub fn can_install(report: &ToolchainReport, name: &str) -> bool {
    if name.starts_with("target:") || matches!(name, "nightly" | "espup") {
        return true;
    }
    report
        .status
        .tools
        .iter()
        .find(|tool| tool.name == name)
        .is_some_and(|tool| tool.installable)
}

/// The page's answer, before any list.
#[derive(Clone, PartialEq, Debug)]
pub enum Verdict {
    /// Rust itself is absent: nothing else can be installed until it is.
    NeedsRust,
    /// A host without its own C toolchain: the Visual Studio linker on an
    /// `-msvc` host, the Xcode Command Line Tools on macOS, a `cc` on Linux
    /// (`toolchain::HOST_C_TOOLCHAINS`). Nothing `cargo install` builds can
    /// be linked without it, so it is the one item, like a missing rustup.
    NeedsLinker,
    /// Things this project needs, by name.
    Missing(Vec<String>),
    /// Ready; the count is optional tools not installed.
    Ready(usize),
}

pub fn verdict(report: &ToolchainReport) -> Verdict {
    let steps = plan(report);
    if steps
        .iter()
        .any(|s| s.tool == "rustup" && s.manual.is_some())
    {
        return Verdict::NeedsRust;
    }
    if steps.iter().any(|s| {
        crate::toolchain::HOST_C_TOOLCHAINS.contains(&s.tool.as_str()) && s.manual.is_some()
    }) {
        return Verdict::NeedsLinker;
    }
    let needed = needed(report);
    if !needed.is_empty() {
        return Verdict::Missing(needed);
    }
    Verdict::Ready(steps.len())
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
            builds_std: false,
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

    /// A PlatformIO or CMake project builds no Rust, and its report says so
    /// by making rustup optional: a C user without rustup is not stopped at
    /// "Rust itself is not installed", and is not interrupted by the sheet.
    #[test]
    fn a_c_project_is_not_stopped_for_want_of_rust() {
        let r = report(vec![tool("rustup", false, false), tool("pio", true, true)]);
        assert!(plan(&r).iter().all(|step| step.tool != "rustup"));
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

    /// A host without its C toolchain can install nothing: every `cargo
    /// install` compiles and then fails to link. So, like a missing rustup,
    /// it is the one item — and a link rather than a button — on each of the
    /// three hosts, with each host's own fix in the sentence.
    #[test]
    fn a_machine_without_the_linker_is_told_that_first() {
        for (row, fix) in [
            ("msvc", "visual-cpp-build-tools"),
            ("xcode-clt", "xcode-select --install"),
            ("cc", "sudo apt install build-essential"),
        ] {
            let mut missing = tool(row, false, true);
            missing.installable = false;
            // The Linux row carries the distribution's package line, which
            // the plan repeats rather than respelling.
            missing.install_command = "sudo apt install build-essential".to_string();
            let r = report(vec![
                tool("rustup", true, true),
                missing,
                tool("espflash", false, true),
            ]);
            let steps = plan(&r);
            assert_eq!(steps.len(), 1, "{row}: {steps:?}");
            assert_eq!(steps[0].tool, row);
            assert_eq!(steps[0].destination, Destination::Manual);
            assert!(
                steps[0].manual.as_ref().is_some_and(|m| m.contains(fix)),
                "{row}: {:?}",
                steps[0].manual
            );
            assert!(blocked(&r), "{row}");
            assert_eq!(verdict(&r), Verdict::NeedsLinker, "{row}");
        }
    }

    /// The espup step runs what the recipe table says, pinned version and
    /// all — spelled once, on espup's own row.
    #[test]
    fn the_espup_step_takes_its_command_from_the_tool_row() {
        let mut espup = tool("espup", false, false);
        espup.install_command =
            "cargo install espup --locked && espup install --toolchain-version 9.9.9.9".into();
        let mut r = report(vec![tool("rustup", true, true), espup]);
        r.needs_esp_toolchain = true;
        let steps = plan(&r);
        assert_eq!(steps[0].tool, "espup");
        assert!(steps[0].command.ends_with("--toolchain-version 9.9.9.9"));
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

    /// A target the build compiles `core` for is never a `rustup target
    /// add`: what is offered is nightly with its source, and only when that
    /// is what the report found missing.
    #[test]
    fn a_build_std_target_asks_for_nightly_and_never_for_the_target() {
        let mut r = report(vec![tool("rustup", true, true)]);
        r.required_target = Some("riscv32ec-unknown-none-elf".to_string());
        r.required_target_installed = false;
        r.builds_std = true;
        assert!(
            plan(&r).is_empty(),
            "pinned wrong is a note, not an install"
        );
        r.problems.push(crate::model::Problem::new(
            crate::model::Severity::Blocking,
            "nightly-missing",
            "Nightly Rust with its source missing",
            "",
        ));
        let steps = plan(&r);
        let tools: Vec<&str> = steps.iter().map(|s| s.tool.as_str()).collect();
        assert_eq!(tools, ["nightly"]);
        assert!(
            steps[0].command.contains("rust-src"),
            "{}",
            steps[0].command
        );
        assert!(blocked(&r));
    }

    /// The Environment page's answer, read off the same report.
    mod page {
        use super::super::*;
        use crate::model::{
            Problem, Severity, ToolStatus, Toolchain, ToolchainReport, ToolchainStatus,
        };

        fn tool(name: &str, installed: bool, required: bool) -> ToolStatus {
            ToolStatus {
                name: name.to_string(),
                purpose: String::new(),
                version: installed.then(|| "1.0".to_string()),
                path: installed.then(|| format!("/bin/{name}")),
                install_command: format!("cargo install {name}"),
                installable: true,
                required,
            }
        }

        fn report(tools: Vec<ToolStatus>, problems: Vec<Problem>) -> ToolchainReport {
            ToolchainReport {
                status: ToolchainStatus {
                    toolchains: vec![Toolchain {
                        name: "stable".to_string(),
                        is_default: true,
                        is_esp: false,
                    }],
                    installed_targets: vec![],
                    tools,
                    has_esp_toolchain: false,
                },
                required_target: Some("riscv32imc-unknown-none-elf".to_string()),
                required_target_installed: false,
                needs_esp_toolchain: false,
                builds_std: false,
                problems,
            }
        }

        /// Needed is what stops *this project*: the target it cannot build for,
        /// read off the report itself — the first version counted it only when
        /// a problem named it, and a page said "one thing missing" above two
        /// rows marked missing — and the flasher the report's blocking problem
        /// names, since a project with a chip cannot reach its board without
        /// one. An optional tool that is absent is not needed.
        #[test]
        fn needed_is_what_the_project_cannot_do_without() {
            let report = report(
                vec![
                    tool("rustup", true, true),
                    tool("espflash", false, false),
                    tool("probe-rs", false, false),
                ],
                vec![Problem::new(Severity::Blocking, "no-flasher", "t", "d")],
            );
            assert_eq!(
                needed(&report),
                ["target:riscv32imc-unknown-none-elf", "espflash"]
            );
            assert_eq!(
                verdict(&report),
                Verdict::Missing(vec![
                    "target:riscv32imc-unknown-none-elf".to_string(),
                    "espflash".to_string(),
                ])
            );
        }

        /// An Xtensa chip's target comes with the Xtensa toolchain, so the
        /// toolchain is what is missing, once.
        #[test]
        fn an_xtensa_target_is_counted_as_its_toolchain() {
            let mut xtensa = report(vec![tool("rustup", true, true)], vec![]);
            xtensa.required_target = Some("xtensa-esp32-none-elf".to_string());
            xtensa.needs_esp_toolchain = true;
            assert_eq!(needed(&xtensa), ["espup"]);
        }

        /// Rust absent is its own answer: everything else rides on it, so the
        /// page does not offer six buttons that cannot work.
        #[test]
        fn without_rust_the_answer_is_rust() {
            let report = report(vec![tool("rustup", false, true)], vec![]);
            assert_eq!(verdict(&report), Verdict::NeedsRust);
        }

        /// Ready, with the optional tools counted rather than listed as
        /// problems — a red badge for probe-rs on a USB-serial board is crying
        /// wolf.
        #[test]
        fn ready_counts_the_optional_tools_it_did_not_need() {
            let mut ready = report(
                vec![
                    tool("rustup", true, true),
                    tool("espflash", true, false),
                    tool("probe-rs", false, false),
                ],
                vec![],
            );
            ready.required_target_installed = true;
            assert_eq!(verdict(&ready), Verdict::Ready(1));
        }

        #[test]
        fn tools_sit_in_the_group_of_work_they_serve() {
            assert_eq!(section_of("espflash"), Section::Device);
            assert_eq!(section_of("probe-rs"), Section::Device);
            assert_eq!(section_of("msvc"), Section::Build);
            assert_eq!(section_of("xcode-clt"), Section::Build);
            assert_eq!(section_of("cc"), Section::Build);
            assert_eq!(section_of("riscv32-esp-elf-gcc"), Section::Build);
            assert_eq!(section_of("arm-none-eabi-gcc"), Section::Build);
            assert_eq!(section_of("pio"), Section::Build);
            assert_eq!(section_of("cmake"), Section::Build);
            assert_eq!(section_of("clangd"), Section::Editor);
            assert_eq!(section_of("rust-analyzer"), Section::Editor);
            assert_eq!(section_of("something-new"), Section::Editor);
        }
    }
}
