//! What Rust and Espressif tooling this machine has, and whether it matches
//! what the open project needs.
//!
//! This panel exists because of one specific failure: a beginner picks an
//! ESP32 or ESP32-S3, runs `cargo build`, and gets
//! `error: toolchain 'stable' does not support target 'xtensa-esp32-none-elf'`.
//! Nothing in that message mentions espup, and the fix is not discoverable from
//! it. Detecting the mismatch before the build is most of the value here.

use std::process::Command;

use crate::simulate::on_path;
use crate::{
    chip,
    model::{
        CommandPlan, EmbeddedProject, Problem, Severity, ToolStatus, Toolchain,
        ToolchainReport, ToolchainStatus,
    },
};

/// External binaries the workbench drives: name, purpose, install command, and
/// whether every project needs it.
const TOOLS: &[(&str, &str, &str, bool)] = &[
    (
        "rustup",
        "Manages Rust toolchains and targets",
        "https://rustup.rs",
        true,
    ),
    (
        "espup",
        "Installs the Xtensa toolchain; only needed for ESP32 / S2 / S3",
        "cargo install espup",
        false,
    ),
    (
        "espflash",
        "Flashes and monitors over USB serial — the usual path with no debug probe",
        "cargo install espflash",
        false,
    ),
    (
        "probe-rs",
        "Flashes and debugs through a JTAG/SWD probe, and decodes defmt over RTT",
        "cargo install probe-rs-tools",
        false,
    ),
    (
        "esp-generate",
        "Generates bare-metal project templates",
        "cargo install esp-generate",
        false,
    ),
    (
        "rust-analyzer",
        "Completion, diagnostics and navigation in the editor",
        "rustup component add rust-analyzer",
        false,
    ),
    (
        "ldproxy",
        "Linker shim required by ESP-IDF (std) builds",
        "cargo install ldproxy",
        false,
    ),
];

/// Whether a binary is on PATH — the same lookup the tool probe uses, so a
/// caller cannot check for a tool under one rule and find it under another.
pub fn on_path_pub(name: &str) -> Option<std::path::PathBuf> {
    on_path(name)
}

/// The C compiler a project for this architecture needs, and how to get it.
///
/// Not in [`TOOLS`] because it is the one entry that depends on the open
/// project: `cc` shells out to a *cross* compiler, and Xtensa's and RISC-V's
/// are different binaries from different places. A single "is there a C
/// compiler" answer would be true and useless.
///
/// The asymmetry is real and worth stating rather than smoothing over: espup
/// installs the Xtensa one as part of the toolchain it exists to manage, and
/// installs nothing for RISC-V, because upstream rustc already emits RISC-V
/// and needs no help. So a RISC-V project that wants C has to fetch a
/// toolchain nothing else asked it to.
pub fn c_compiler(arch: crate::model::Arch) -> Option<(&'static str, &'static str)> {
    match arch {
        crate::model::Arch::Xtensa => Some((
            "xtensa-esp-elf-gcc",
            "espup install — the Xtensa toolchain it manages includes the C compiler",
        )),
        crate::model::Arch::RiscV => Some((
            "riscv32-esp-elf-gcc",
            "download riscv32-esp-elf from \
             https://github.com/espressif/crosstool-NG/releases and put its bin/ on PATH \
             — espup does not install it, because Rust needs no help emitting RISC-V",
        )),
        // A Cortex-M project's C compiler is `arm-none-eabi-gcc`, but rusty
        // has not verified that path against a real project and will not
        // claim it from memory.
        crate::model::Arch::CortexM => None,
    }
}

/// How to install one of the tools rusty drives.
///
/// The table already knew this and only the toolchain panel was reading it, so
/// every other caller reported "not found" and stopped there — which is exactly
/// the half-answer this workbench exists to avoid.
pub fn install_command(tool: &str) -> Option<&'static str> {
    TOOLS
        .iter()
        .find(|(name, ..)| *name == tool)
        .map(|(_, _, install, _)| *install)
}

/// The steps that install one tool, ready for the shared session runner —
/// every line of every step streams into the dock, and only a failure sends
/// anyone to the manual command.
///
/// One table drives probing, manual instructions and one-click installs, so
/// a tool cannot be probed under one spelling and installed under another.
/// rustup itself is the one thing this cannot install: it is the installer.
pub fn install_steps(tool: &str) -> Result<Vec<CommandPlan>, String> {
    let cargo_install = |package: &str, why: &str| -> Vec<CommandPlan> {
        vec![CommandPlan {
            program: "cargo".to_string(),
            args: vec![
                "install".to_string(),
                package.to_string(),
                "--locked".to_string(),
            ],
            display: format!("cargo install {package} --locked"),
            rationale: why.to_string(),
                         warning: None,
             }]
    };

    match tool {
        "espflash" => Ok(cargo_install(
            "espflash",
            "builds espflash into ~/.cargo/bin, where flashing and the simulator look",
        )),
        "probe-rs" => Ok(cargo_install(
            "probe-rs-tools",
            "the probe-rs CLI: JTAG/SWD flashing, debugging, defmt over RTT",
        )),
        "esp-generate" => Ok(cargo_install(
            "esp-generate",
            "the template generator behind File > New project",
        )),
        "ldproxy" => Ok(cargo_install(
            "ldproxy",
            "the linker shim ESP-IDF (std) builds route through",
        )),
        "rust-analyzer" => Ok(vec![CommandPlan {
            program: "rustup".to_string(),
            args: vec![
                "component".to_string(),
                "add".to_string(),
                "rust-analyzer".to_string(),
                "--toolchain".to_string(),
                "stable".to_string(),
            ],
            display: "rustup component add rust-analyzer --toolchain stable".to_string(),
            rationale: "the stable component; rusty resolves it directly, so the esp \
                        toolchain's missing component stops mattering"
                .to_string(),
                                               warning: None,
                                   }]),
        // Two steps by design: the first is quick, the second downloads the
        // Xtensa toolchain and is honestly slow — better one visible slow
        // step than a guide page nobody finds.
        "espup" => Ok(vec![
            CommandPlan {
                program: "cargo".to_string(),
                args: vec![
                    "install".to_string(),
                    "espup".to_string(),
                    "--locked".to_string(),
                ],
                display: "cargo install espup --locked".to_string(),
                rationale: "the Xtensa toolchain manager itself".to_string(),
                            warning: None,
            },
            CommandPlan {
                program: "espup".to_string(),
                args: vec!["install".to_string()],
                display: "espup install".to_string(),
                rationale: "downloads the esp toolchain (Xtensa rustc + gcc) — a gigabyte-\
                            class download, so this step takes minutes"
                    .to_string(),
                            warning: None,
            },
        ]),
        "rustup" => Err(
            "rustup is the installer everything else rides on — get it from \
             https://rustup.rs, then everything here becomes one click"
                .to_string(),
        ),
        other => Err(format!("no install recipe for {other}")),
    }
}

/// Inspect the machine.
pub fn status() -> ToolchainStatus {
    let toolchains = list_toolchains();
    let has_esp_toolchain = toolchains.iter().any(|t| t.is_esp);

    ToolchainStatus {
        toolchains,
        installed_targets: list_installed_targets(),
        tools: TOOLS
            .iter()
            .map(|(name, purpose, install, required)| {
                // Presence decides; the version is asked for only once the
                // binary is known to exist, so a tool that has no `--version`
                // is still installed and one that is absent costs no spawn.
                let path = on_path(name);
                ToolStatus {
                    name: (*name).to_string(),
                    purpose: (*purpose).to_string(),
                    version: path.as_ref().and_then(|_| probe_version(name)),
                    path: path.map(|found| found.display().to_string()),
                    install_command: (*install).to_string(),
                    required: *required,
                }
            })
            .collect(),
        has_esp_toolchain,
    }
}

/// Machine state plus what this project needs from it.
pub fn report(project: Option<&EmbeddedProject>) -> ToolchainReport {
    let mut status = status();
    let mut problems = Vec::new();

    let chip = project
        .and_then(|p| p.chip.as_deref())
        .and_then(chip::by_id);

    let needs_esp_toolchain = chip.as_ref().is_some_and(|c| c.needs_esp_toolchain());

    // The C compiler, listed only once a chip says which one. It is reported
    // whether or not this project speaks C: "can I add C to this" is a
    // question people ask before they have, and answering it only after they
    // try is the failure this panel exists to prevent.
    if let Some((binary, install)) = chip.as_ref().and_then(|c| c_compiler(c.arch)) {
        let path = on_path(binary);
        status.tools.push(ToolStatus {
            name: binary.to_string(),
            purpose: format!(
                "Compiles C into the build for {} — needed by `cc`, bindgen and \
                 esp-idf-sys, and by nothing else",
                chip.as_ref().map_or("this part", |c| c.name.as_str()),
            ),
            version: path.as_ref().and_then(|_| probe_version(binary)),
            path: path.map(|found| found.display().to_string()),
            install_command: install.to_string(),
            // Only projects that actually speak C need it, and the detection
            // that knows whether this one does lives in `project::detect`.
            required: false,
        });
    }

    // The required target follows from chip + runtime; either being unknown
    // means there is nothing to check rather than something to complain about.
    let required_target = match (&chip, project.and_then(|p| p.runtime)) {
        (Some(chip), Some(runtime)) => chip.target_for(runtime).map(str::to_string),
        _ => project.and_then(|p| p.configured_target.clone()),
    };

    let required_target_installed = match &required_target {
        // Xtensa targets are shipped inside the espup toolchain rather than
        // added through rustup, so `rustup target list` never mentions them and
        // its absence is not evidence of anything.
        Some(target) if target.starts_with("xtensa-") => status.has_esp_toolchain,
        Some(target) => status.installed_targets.iter().any(|t| t == target),
        None => true,
    };

    if needs_esp_toolchain && !status.has_esp_toolchain {
        let chip_name = chip.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        problems.push(Problem {
            severity: Severity::Blocking,
            title: "Xtensa toolchain missing".into(),
            detail: format!(
                "{chip_name} is Xtensa, which upstream rustc cannot target. Without the \
                 `esp` toolchain the build fails with an unknown-target error that does \
                 not mention espup. Installing it takes a while — it downloads a forked \
                 LLVM."
            ),
            fix_command: Some("espup install".into()),
        });
    }

    if let Some(target) = &required_target
        && !required_target_installed
        && !target.starts_with("xtensa-")
    {
        problems.push(Problem {
            severity: Severity::Blocking,
            title: format!("Target `{target}` not installed"),
            detail: "cargo will refuse to build for a target rustup has not added.".into(),
            fix_command: Some(format!("rustup target add {target}")),
        });
    }

    // Only complain about a flashing tool if the project could actually be
    // flashed — a workspace that is not an embedded project should not be
    // nagged about espflash.
    if project.is_some_and(|p| p.chip.is_some()) {
        let has_flasher = status
            .tools
            .iter()
            .any(|t| matches!(t.name.as_str(), "espflash" | "probe-rs") && t.is_installed());
        if !has_flasher {
            problems.push(Problem {
                severity: Severity::Blocking,
                title: "No way to flash the board".into(),
                detail: "Neither espflash nor probe-rs is installed. espflash is the \
                         simpler choice — it needs only the USB cable. probe-rs adds \
                         breakpoint debugging and defmt over RTT, but wants a probe."
                    .into(),
                fix_command: Some("cargo install espflash".into()),
            });
        }
    }

    if project.is_some_and(|p| p.runtime == Some(crate::model::Runtime::EspIdf)) {
        let has_ldproxy = status
            .tools
            .iter()
            .any(|t| t.name == "ldproxy" && t.is_installed());
        if !has_ldproxy {
            problems.push(Problem {
                severity: Severity::Blocking,
                title: "ldproxy missing".into(),
                detail: "ESP-IDF (std) builds link through ldproxy. Without it the build \
                         fails at the link step with a linker-not-found error."
                    .into(),
                fix_command: Some("cargo install ldproxy".into()),
            });
        }
    }

    ToolchainReport {
        status,
        required_target,
        required_target_installed,
        needs_esp_toolchain,
        problems,
    }
}

// ─── probing ─────────────────────────────────────────────────────────────────

fn list_toolchains() -> Vec<Toolchain> {
    let Some(out) = run("rustup", &["toolchain", "list"]) else {
        return Vec::new();
    };
    out.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let is_default = line.contains("(default)");
            let name = line
                .split_whitespace()
                .next()
                .unwrap_or(line)
                .to_string();
            // espup names its toolchain `esp`; rustup shows it without a host
            // triple suffix because it is a custom install.
            let is_esp = name == "esp" || name.starts_with("esp-");
            Toolchain {
                name,
                is_default,
                is_esp,
            }
        })
        .collect()
}

fn list_installed_targets() -> Vec<String> {
    run("rustup", &["target", "list", "--installed"])
        .map(|out| {
            out.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// First line of `<tool> --version`, or `None` when the binary is absent.
fn probe_version(tool: &str) -> Option<String> {
    let out = run(tool, &["--version"])?;
    out.lines()
        .next()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args);
    no_console_window(&mut command);

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    // Some tools report their version on stderr.
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).into_owned();
    }
    Some(text)
}

/// Keep child processes from flashing a console window.
///
/// Without this every version probe blinks a black rectangle over the UI, and
/// the toolchain panel probes six tools on open.
#[cfg(windows)]
pub(crate) fn no_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(crate) fn no_console_window(_command: &mut Command) {}
