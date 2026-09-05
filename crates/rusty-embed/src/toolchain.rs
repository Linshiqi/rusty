//! What Rust and Espressif tooling this machine has, and whether it matches
//! what the open project needs.
//!
//! This panel exists because of one specific failure: a beginner picks an
//! ESP32 or ESP32-S3, runs `cargo build`, and gets
//! `error: toolchain 'stable' does not support target 'xtensa-esp32-none-elf'`.
//! Nothing in that message mentions espup, and the fix is not discoverable from
//! it. Detecting the mismatch before the build is most of the value here.
//!
//! **rustup is asked from the project's directory, for the project's
//! toolchain.** `rustup target list --installed` answers for the *active*
//! toolchain, and which one is active depends on where rustup is asked from —
//! a `rust-toolchain.toml` pin — and on `RUSTUP_TOOLCHAIN`, which `cargo tauri
//! dev` leaks into this process. Asked from rusty's own directory with no
//! project in mind, it answered for whatever the machine's default was: a
//! project pinned to a nightly with the target added only there was reported
//! "Target not installed", and the offered `rustup target add` run from the
//! same wrong place added the target to the default toolchain again, so the
//! problem survived its own fix. [`RustupContext`] carries the directory and
//! the pin, and the fix command names the toolchain so it is right wherever it
//! is pasted.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use crate::{
    chip,
    error::{Error, Result},
    model::{
        CommandPlan, EmbeddedProject, Problem, Severity, ToolStatus, Toolchain, ToolchainReport,
        ToolchainStatus,
    },
    process, tools,
};

/// One external binary the workbench drives: what it is for, how it gets
/// installed, and whether every project needs it.
///
/// **The one table.** Probing, the instruction shown beside a missing tool,
/// and the one-click recipe all read from here, so a tool cannot be probed
/// under one spelling and installed under another. The recipes used to be a
/// second `match` with `--locked` spelled into it while the panel's text said
/// `cargo install espflash` without — two accounts of one fact, and only a
/// reader who compared them would know which one the button ran.
struct Tool {
    name: &'static str,
    purpose: &'static str,
    /// True when no project builds without it.
    required: bool,
    recipe: Recipe,
}

/// How a tool gets onto the machine.
enum Recipe {
    /// Nothing rusty can run: a page to visit, and the sentence that says
    /// why. rustup is the installer everything else rides on, so it is the
    /// one thing that cannot be one-clicked.
    Manual {
        url: &'static str,
        because: &'static str,
    },
    /// `cargo install <package> --locked`, into `~/.cargo/bin` — where
    /// flashing and the simulator look. Redirecting it with `--root` would
    /// put espflash somewhere nothing finds it.
    CargoInstall {
        package: &'static str,
        why: &'static str,
    },
    /// `rustup component add`, on the stable toolchain: rusty resolves the
    /// binary there directly, so the esp toolchain's missing component stops
    /// mattering.
    RustupComponent {
        component: &'static str,
        why: &'static str,
    },
    /// Two steps by design: the first is quick, the second downloads the
    /// Xtensa toolchain and is honestly slow — better one visible slow step
    /// than a guide page nobody finds.
    Espup,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "rustup",
        purpose: "Manages Rust toolchains and targets",
        required: true,
        recipe: Recipe::Manual {
            url: "https://rustup.rs",
            because: "rustup is the installer everything else rides on — get it from \
                      https://rustup.rs, then everything here becomes one click",
        },
    },
    Tool {
        name: "espup",
        purpose: "Installs the Xtensa toolchain; only needed for ESP32 / S2 / S3",
        required: false,
        recipe: Recipe::Espup,
    },
    Tool {
        name: "espflash",
        purpose: "Flashes and monitors over USB serial — the usual path with no debug probe",
        required: false,
        recipe: Recipe::CargoInstall {
            package: "espflash",
            why: "builds espflash into ~/.cargo/bin, where flashing and the simulator look",
        },
    },
    Tool {
        name: "probe-rs",
        purpose: "Flashes and debugs through a JTAG/SWD probe, and decodes defmt over RTT",
        required: false,
        recipe: Recipe::CargoInstall {
            package: "probe-rs-tools",
            why: "the probe-rs CLI: JTAG/SWD flashing, debugging, defmt over RTT",
        },
    },
    Tool {
        name: "esp-generate",
        purpose: "Generates bare-metal project templates",
        required: false,
        recipe: Recipe::CargoInstall {
            package: "esp-generate",
            why: "the template generator behind File > New project",
        },
    },
    Tool {
        name: "rust-analyzer",
        purpose: "Completion, diagnostics and navigation in the editor",
        required: false,
        recipe: Recipe::RustupComponent {
            component: "rust-analyzer",
            why: "the stable component; rusty resolves it directly, so the esp toolchain's \
                  missing component stops mattering",
        },
    },
    Tool {
        name: "ldproxy",
        purpose: "Linker shim required by ESP-IDF (std) builds",
        required: false,
        recipe: Recipe::CargoInstall {
            package: "ldproxy",
            why: "the linker shim ESP-IDF (std) builds route through",
        },
    },
];

impl Recipe {
    /// The steps that install the tool, ready for the shared session runner —
    /// every line of every step streams into the dock, and only a failure
    /// sends anyone to the manual command.
    fn steps(&self) -> Result<Vec<CommandPlan>> {
        let step = |program: &str, args: &[&str], rationale: &str| CommandPlan {
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            display: std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" "),
            rationale: rationale.to_string(),
            warning: None,
        };
        match self {
            Recipe::Manual { because, .. } => Err(Error::refused(*because)),
            Recipe::CargoInstall { package, why } => {
                Ok(vec![step("cargo", &["install", package, "--locked"], why)])
            }
            Recipe::RustupComponent { component, why } => Ok(vec![step(
                "rustup",
                &["component", "add", component, "--toolchain", "stable"],
                why,
            )]),
            Recipe::Espup => Ok(vec![
                step(
                    "cargo",
                    &["install", "espup", "--locked"],
                    "the Xtensa toolchain manager itself",
                ),
                step(
                    "espup",
                    &[
                        "install",
                        "--toolchain-version",
                        crate::install::XTENSA_RUST_VERSION,
                    ],
                    "downloads the esp toolchain (Xtensa rustc + gcc) — a gigabyte-class \
                     download, so this step takes minutes. The version is named because \
                     espup's own \"latest\" lookup asks GitHub's API, which refuses \
                     unauthenticated calls through a busy proxy; `espup update` moves \
                     forward later",
                ),
            ]),
        }
    }

    /// The instruction shown beside a missing tool: exactly what the one-click
    /// recipe would run, joined into one line, or the page to visit when there
    /// is no recipe. Derived from [`Self::steps`] rather than spelled again, so
    /// the text and the button cannot disagree.
    fn command(&self) -> String {
        match self {
            Recipe::Manual { url, .. } => (*url).to_string(),
            other => other
                .steps()
                .map(|steps| {
                    steps
                        .iter()
                        .map(|step| step.display.as_str())
                        .collect::<Vec<_>>()
                        .join(" && ")
                })
                .unwrap_or_default(),
        }
    }

    fn installable(&self) -> bool {
        !matches!(self, Recipe::Manual { .. })
    }
}

fn tool(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// Where a binary rusty drives is on this machine, by the one ladder in
/// [`crate::tools`] — the data directory's `tools/`, then cargo's bin, then
/// PATH. The same lookup the tool probe uses, so a caller cannot check for a
/// tool under one rule and find it under another.
pub fn on_path_pub(name: &str) -> Option<PathBuf> {
    tools::find(name)
}

/// Where the C++ build tools come from — the page, not an installer rusty
/// could run: "Desktop development with C++" is a choice made in it.
pub const MSVC_BUILD_TOOLS: &str = "https://visualstudio.microsoft.com/visual-cpp-build-tools/";

/// Visual Studio's `link.exe` for this machine, found the way rustc finds it.
/// `None` off Windows, and on a Windows without the C++ build tools.
fn msvc_linker() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let target = format!("{}-pc-windows-msvc", std::env::consts::ARCH);
        cc::windows_registry::find_tool(&target, "link.exe").map(|tool| tool.path().to_path_buf())
    }
    #[cfg(not(windows))]
    {
        None
    }
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

/// How to install one of the tools rusty drives, as one line to type.
///
/// The table already knew this and only the toolchain panel was reading it, so
/// every other caller reported "not found" and stopped there — which is exactly
/// the half-answer this workbench exists to avoid.
pub fn install_command(tool_name: &str) -> Option<String> {
    tool(tool_name).map(|tool| tool.recipe.command())
}

/// The steps that install one tool, for `install::install_steps` — which is
/// the public entry, because it also knows about the archives this table does
/// not cover.
pub(crate) fn recipe(tool_name: &str) -> Result<Vec<CommandPlan>> {
    match tool(tool_name) {
        Some(tool) => tool.recipe.steps(),
        None => Err(Error::refused(format!("no install recipe for {tool_name}"))),
    }
}

/// Inspect the machine with no project in mind: the default toolchain answers
/// for the targets. [`report`] is the form that knows about a project.
pub fn status() -> ToolchainStatus {
    status_in(&RustupContext::default())
}

fn status_in(rustup: &RustupContext<'_>) -> ToolchainStatus {
    let toolchains = list_toolchains();
    let has_esp_toolchain = toolchains.iter().any(|t| t.is_esp);

    ToolchainStatus {
        toolchains,
        installed_targets: list_installed_targets(rustup),
        tools: TOOLS
            .iter()
            .map(|tool| {
                // Presence decides; the version is asked for only once the
                // binary is known to exist, so a tool that has no `--version`
                // is still installed and one that is absent costs no spawn.
                // Asked of the copy that was found, not of whatever a bare
                // name resolves to: those can be two different binaries.
                let (path, version) = match tool.recipe {
                    // A rustup component is not a file to find: the file is
                    // there on every machine with rustup. See `component_binary`.
                    Recipe::RustupComponent { component, .. } => component_binary(component),
                    _ => {
                        let path = tools::find(tool.name);
                        let version = path.as_ref().and_then(|found| probe_version(found));
                        (path, version)
                    }
                };
                ToolStatus {
                    name: tool.name.to_string(),
                    purpose: tool.purpose.to_string(),
                    version,
                    path: path.map(|found| found.display().to_string()),
                    install_command: tool.recipe.command(),
                    installable: tool.recipe.installable(),
                    required: tool.required,
                }
            })
            .collect(),
        has_esp_toolchain,
    }
}

/// Machine state plus what this project needs from it.
pub fn report(project: Option<&EmbeddedProject>) -> ToolchainReport {
    let rustup = RustupContext::for_project(project);
    let mut status = status_in(&rustup);
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
        let path = tools::find(binary);
        status.tools.push(ToolStatus {
            name: binary.to_string(),
            purpose: format!(
                "Compiles C into the build for {} — needed by `cc`, bindgen and \
                 esp-idf-sys, and by nothing else",
                chip.as_ref().map_or("this part", |c| c.name.as_str()),
            ),
            version: path.as_ref().and_then(|found| probe_version(found)),
            path: path.map(|found| found.display().to_string()),
            install_command: install.to_string(),
            // The RISC-V one downloads like QEMU and the debuggers do, on
            // the platform whose asset has been checked. The Xtensa one is
            // espup's to install, and clicking Install on *this* row could
            // not say so — its own row can.
            installable: binary == "riscv32-esp-elf-gcc" && cfg!(windows),
            // Only projects that actually speak C need it, and the detection
            // that knows whether this one does lives in `project::detect`.
            required: false,
        });
    }

    // The debug adapter, on the platform that cannot debug host code without
    // one. Rust's default Windows target emits a PDB and gdb reads DWARF, so
    // on Windows "Debug" beside a test needs LLDB, and LLDB is driven through
    // an adapter. Listed, not required: it changes nothing about building,
    // flashing or running tests, and a red badge for an optional tool is the
    // crying-wolf this panel is careful about.
    if cfg!(windows) {
        let path = crate::host_debug::host_adapters().into_iter().next();
        let fetchable = crate::install::codelldb_available();
        status.tools.push(ToolStatus {
            name: "codelldb".to_string(),
            purpose: "Debugs tests and host code on Windows — gdb cannot read this target's \
                      PDB debug information and LLDB can"
                .to_string(),
            version: None,
            path: path.map(|found| found.display().to_string()),
            // A URL where there is nothing to fetch: CodeLLDB publishes no
            // ARM64 Windows build, and the panel shows the link rather than a
            // button that could only fail.
            install_command: if fetchable {
                "download".to_string()
            } else {
                crate::install::codelldb_page()
            },
            installable: fetchable,
            required: false,
        });
    }

    // The linker, on the one host whose Rust does not bring its own. An
    // `-msvc` rustc links through Visual Studio's `link.exe`, and without the
    // C++ build tools every `cargo install` above dies with "linker
    // `link.exe` not found" — after compiling for a minute, on the first
    // run, on the machine of somebody who has just installed Rust and has no
    // idea what a linker is. Found the way rustc finds it, through the same
    // Visual Studio discovery the `cc` crate does, so this row cannot say
    // "missing" about a linker rustc would use. Not installable by rusty: a
    // Visual Studio installer is a decision for the user, so the row is a
    // link — like rustup's.
    if let Some(host) = status.toolchains.iter().find(|t| t.is_default)
        && host.name.contains("-msvc")
    {
        status.tools.push(ToolStatus {
            name: "msvc".to_string(),
            purpose: "The Windows linker (link.exe) — every `cargo install` and every host \
                      build needs it"
                .to_string(),
            version: None,
            path: msvc_linker().map(|found| found.display().to_string()),
            install_command: MSVC_BUILD_TOOLS.to_string(),
            installable: false,
            required: true,
        });
    }

    // The required target follows from chip + runtime; either being unknown
    // means there is nothing to check rather than something to complain about.
    let required_target = match (&chip, project.and_then(|p| p.runtime)) {
        (Some(chip), Some(runtime)) => chip.target_for(runtime).map(str::to_string),
        _ => project.and_then(|p| p.configured_target.clone()),
    };

    let required_target_installed = target_installed(
        required_target.as_deref(),
        &status.installed_targets,
        status.has_esp_toolchain,
    );

    if needs_esp_toolchain && !status.has_esp_toolchain {
        let chip_name = chip.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        problems.push(
            Problem::new(
                Severity::Blocking,
                "xtensa-missing",
                "Xtensa toolchain missing",
                format!(
                    "{chip_name} is Xtensa, which upstream rustc cannot target. Without the \
                     `esp` toolchain the build fails with an unknown-target error that does \
                     not mention espup. Installing it takes a while — it downloads a forked \
                     LLVM."
                ),
            )
            .arg("chip", &chip_name)
            .fix(install_command("espup").unwrap_or_default()),
        );
    }

    if let Some(target) = &required_target
        && !required_target_installed
        && !target.starts_with("xtensa-")
    {
        problems.push(
            Problem::new(
                Severity::Blocking,
                "target-not-installed",
                format!("Target `{target}` not installed"),
                "cargo will refuse to build for a target rustup has not added.",
            )
            .arg("target", target)
            .fix(target_fix(target, rustup.toolchain)),
        );
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
            problems.push(
                Problem::new(
                    Severity::Blocking,
                    "no-flasher",
                    "No way to flash the board",
                    "Neither espflash nor probe-rs is installed. espflash is the \
                     simpler choice — it needs only the USB cable. probe-rs adds \
                     breakpoint debugging and defmt over RTT, but wants a probe.",
                )
                .fix(install_command("espflash").unwrap_or_default()),
            );
        }
    }

    if project.is_some_and(|p| p.runtime == Some(crate::model::Runtime::EspIdf)) {
        let has_ldproxy = status
            .tools
            .iter()
            .any(|t| t.name == "ldproxy" && t.is_installed());
        if !has_ldproxy {
            problems.push(
                Problem::new(
                    Severity::Blocking,
                    "ldproxy-missing",
                    "ldproxy missing",
                    "ESP-IDF (std) builds link through ldproxy. Without it the build \
                     fails at the link step with a linker-not-found error.",
                )
                .fix(install_command("ldproxy").unwrap_or_default()),
            );
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

// ─── the pure parts ──────────────────────────────────────────────────────────

/// Whether the target a project needs is there to build for.
///
/// Xtensa targets are shipped inside the espup toolchain rather than added
/// through rustup, so `rustup target list` never mentions them and their
/// absence from it is not evidence of anything; the `esp` toolchain's
/// presence is the answer. No required target is nothing to check.
fn target_installed(required: Option<&str>, installed: &[String], has_esp: bool) -> bool {
    match required {
        Some(target) if target.starts_with("xtensa-") => has_esp,
        Some(target) => installed.iter().any(|t| t == target),
        None => true,
    }
}

/// The command that adds a target, naming the toolchain when the project pins
/// one — so it is right wherever it is pasted. Bare, `rustup target add` acts
/// on whatever toolchain is active where it is typed, and typed from the
/// wrong directory it fixed the default toolchain while the pinned one stayed
/// short of the target.
fn target_fix(target: &str, toolchain: Option<&str>) -> String {
    match toolchain {
        Some(channel) => format!("rustup target add {target} --toolchain {channel}"),
        None => format!("rustup target add {target}"),
    }
}

/// `rustup toolchain list`, one line per toolchain.
///
/// The annotations changed shape between rustup releases — `(default)` once,
/// `(active, default)` now — so the parenthesised list is read as a list
/// rather than matched as one string. Matching `(default)` reported no
/// default at all on a current rustup.
fn parse_toolchain_list(text: &str) -> Vec<Toolchain> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, annotations) = line.split_once('(').unwrap_or((line, ""));
            let name = name.trim().to_string();
            let is_default = annotations
                .trim_end_matches(')')
                .split(',')
                .any(|word| word.trim() == "default");
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

/// `rustup target list --installed`, one triple per line.
fn parse_target_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

// ─── probing ─────────────────────────────────────────────────────────────────

/// Where, and for which toolchain, rustup is asked about targets — see the
/// module header for why either half being wrong survives its own fix.
#[derive(Default)]
struct RustupContext<'a> {
    /// The project directory, so a `rust-toolchain.toml` pin decides.
    cwd: Option<&'a Path>,
    /// The pinned channel, passed explicitly as well: the two agree, and the
    /// command in the logs then says which toolchain it asked about.
    toolchain: Option<&'a str>,
}

impl<'a> RustupContext<'a> {
    fn for_project(project: Option<&'a EmbeddedProject>) -> Self {
        RustupContext {
            cwd: project.map(|p| Path::new(p.root.as_str())),
            toolchain: project.and_then(|p| p.configured_toolchain.as_deref()),
        }
    }
}

fn list_toolchains() -> Vec<Toolchain> {
    run("rustup", &["toolchain", "list"], None)
        .map(|out| parse_toolchain_list(&out))
        .unwrap_or_default()
}

fn list_installed_targets(rustup: &RustupContext<'_>) -> Vec<String> {
    let mut args = vec!["target", "list", "--installed"];
    if let Some(toolchain) = rustup.toolchain {
        args.extend(["--toolchain", toolchain]);
    }
    run("rustup", &args, rustup.cwd)
        .map(|out| parse_target_list(&out))
        .unwrap_or_default()
}

/// First line of `<tool> --version`, or `None` when it would not run.
/// Where a rustup component's binary is and what it says it is — or nothing.
///
/// The bare name on PATH is rustup's proxy, which exists on every machine
/// with rustup whether or not the component does; with the component missing
/// the proxy starts, prints an error and exits. So presence is not a file: it
/// is the binary `rustup which --toolchain stable` names (the copy the editor
/// runs), or failing that the one on PATH, answering `--version` with the
/// component's own name. The editor's rust-analyzer discovery has followed
/// this rule since the CI runners taught it; the panel did not, and the two
/// disagreed on screen — a setup sheet declaring the machine ready above a
/// status bar saying rust-analyzer was missing.
fn component_binary(component: &str) -> (Option<PathBuf>, Option<String>) {
    let from_rustup = run(
        "rustup",
        &["which", "--toolchain", "stable", component],
        None,
    )
    .map(|out| PathBuf::from(out.trim()));
    for candidate in [from_rustup, tools::find(component)].into_iter().flatten() {
        if !candidate.is_file() {
            continue;
        }
        if let Some(version) =
            probe_version(&candidate).filter(|said| is_component_version(component, said))
        {
            return (Some(candidate), Some(version));
        }
    }
    (None, None)
}

/// What a component prints for `--version` begins with its name; what
/// rustup's proxy prints for a missing one is an error, on stderr, with a
/// non-zero exit — which `run` already turns into nothing.
fn is_component_version(component: &str, said: &str) -> bool {
    said.trim_start().starts_with(component)
}

fn probe_version(tool: &Path) -> Option<String> {
    let out = run(tool, &["--version"], None)?;
    out.lines()
        .next()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
}

/// Run a probe through [`process::command`], so it gets the same environment
/// every other spawn here does — no leaked `RUSTUP_TOOLCHAIN`, no console
/// window — and answer its output, or `None` when it failed.
fn run(program: impl AsRef<OsStr>, args: &[&str], cwd: Option<&Path>) -> Option<String> {
    let mut command = process::command(program);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A current rustup annotates its lines `(active, default)`; an older one
    /// `(default)`. The default has to be found under both, or the panel
    /// reports a machine with toolchains and no default.
    #[test]
    fn the_default_toolchain_is_found_under_both_annotation_shapes() {
        let modern = parse_toolchain_list(
            "stable-x86_64-pc-windows-msvc (active, default)\n\
             nightly-2025-06-01-x86_64-pc-windows-msvc\n\
             esp\n",
        );
        assert_eq!(modern.len(), 3);
        assert!(modern[0].is_default, "{modern:?}");
        assert_eq!(modern[0].name, "stable-x86_64-pc-windows-msvc");
        assert!(!modern[1].is_default);
        assert!(modern[2].is_esp && !modern[2].is_default);

        let older = parse_toolchain_list("stable-x86_64-unknown-linux-gnu (default)\nesp\n");
        assert!(older[0].is_default);
        assert!(!older[0].is_esp);

        // `(active)` alone is a directory override, not the default.
        let overridden = parse_toolchain_list("nightly-x86_64-unknown-linux-gnu (active)\n");
        assert!(!overridden[0].is_default);
    }

    #[test]
    fn the_target_list_is_one_triple_per_line() {
        assert_eq!(
            parse_target_list("riscv32imc-unknown-none-elf\n\nx86_64-pc-windows-msvc \n"),
            vec![
                "riscv32imc-unknown-none-elf".to_string(),
                "x86_64-pc-windows-msvc".to_string(),
            ],
        );
        assert!(parse_target_list("").is_empty());
    }

    /// An Xtensa target is never in `rustup target list`; the esp toolchain's
    /// presence is the evidence. Everything else is looked up literally, and
    /// nothing required is nothing to check.
    #[test]
    fn a_target_is_installed_by_the_rule_its_toolchain_uses() {
        let installed = vec!["riscv32imc-unknown-none-elf".to_string()];
        assert!(target_installed(
            Some("riscv32imc-unknown-none-elf"),
            &installed,
            false
        ));
        assert!(!target_installed(
            Some("riscv32imac-unknown-none-elf"),
            &installed,
            false
        ));
        assert!(
            target_installed(Some("xtensa-esp32-none-elf"), &[], true),
            "shipped inside the esp toolchain, so its absence from the list means nothing",
        );
        assert!(!target_installed(Some("xtensa-esp32-none-elf"), &[], false));
        assert!(target_installed(None, &[], false));
    }

    /// The fix has to survive being pasted anywhere. Without the toolchain it
    /// acts on whatever is active where it is typed, which from the wrong
    /// directory is the default toolchain — and the problem it was offered
    /// for stays exactly as it was.
    #[test]
    fn the_target_fix_names_the_pinned_toolchain() {
        assert_eq!(
            target_fix("riscv32imc-unknown-none-elf", Some("nightly-2025-06-01")),
            "rustup target add riscv32imc-unknown-none-elf --toolchain nightly-2025-06-01",
        );
        assert_eq!(
            target_fix("riscv32imc-unknown-none-elf", None),
            "rustup target add riscv32imc-unknown-none-elf",
        );
    }

    /// The project's directory and its pin both reach rustup, or a pinned
    /// project is answered for the machine's default.
    #[test]
    fn rustup_is_asked_from_the_project_for_the_projects_toolchain() {
        let project = EmbeddedProject {
            root: "E:/work/blinky".to_string(),
            chip: Some("esp32c3".to_string()),
            chip_source: None,
            runtime: None,
            configured_target: Some("riscv32imc-unknown-none-elf".to_string()),
            configured_toolchain: Some("nightly-2025-06-01".to_string()),
            frameworks: Vec::new(),
            uses_defmt: false,
            uses_embassy: false,
            c_interop: Default::default(),
            evidence: Vec::new(),
            problems: Vec::new(),
        };
        let context = RustupContext::for_project(Some(&project));
        assert_eq!(context.cwd, Some(Path::new("E:/work/blinky")));
        assert_eq!(context.toolchain, Some("nightly-2025-06-01"));

        let bare = RustupContext::for_project(None);
        assert!(bare.cwd.is_none() && bare.toolchain.is_none());
    }

    /// The text beside a missing tool is the recipe, not a second spelling of
    /// it: what the button runs is what the panel says.
    /// rustup's proxy is a file called `rust-analyzer` on every machine with
    /// rustup; only the real component answers `--version` with its name.
    #[test]
    fn a_component_is_present_when_its_version_says_so_not_when_a_file_exists() {
        assert!(is_component_version(
            "rust-analyzer",
            "rust-analyzer 1.98.0 (5f3ac7d 2026-08-30)"
        ));
        assert!(is_component_version(
            "rust-analyzer",
            "  rust-analyzer 0.3.2500-standalone"
        ));
        assert!(!is_component_version(
            "rust-analyzer",
            "error: unknown binary 'rust-analyzer' in toolchain 'esp'"
        ));
        assert!(!is_component_version("rust-analyzer", ""));
        assert!(!is_component_version(
            "rust-analyzer",
            "rustfmt 1.8.0-stable"
        ));
    }

    #[test]
    fn the_install_text_is_the_recipe_it_describes() {
        assert_eq!(
            install_command("espflash").as_deref(),
            Some("cargo install espflash --locked")
        );
        assert_eq!(
            install_command("espup"),
            Some(format!(
                "cargo install espup --locked && espup install --toolchain-version {}",
                crate::install::XTENSA_RUST_VERSION
            )),
            "a two-step recipe shows both steps, the second with the pinned version",
        );
        assert_eq!(
            install_command("rust-analyzer").as_deref(),
            Some("rustup component add rust-analyzer --toolchain stable"),
        );
        assert_eq!(
            install_command("rustup").as_deref(),
            Some("https://rustup.rs"),
            "the one tool with no recipe shows where to get it",
        );
        assert_eq!(
            install_command("ldproxy").as_deref(),
            Some("cargo install ldproxy --locked")
        );
        assert_eq!(install_command("mystery"), None);
        for tool in TOOLS {
            assert_eq!(
                recipe(tool.name).is_ok(),
                tool.recipe.installable(),
                "{}: `installable` must mean a recipe exists",
                tool.name,
            );
        }
    }

    /// The setup screen says where each step lands before running it, from
    /// a rule of its own on the wasm side. That rule and this table have to
    /// agree, or the screen claims a directory the recipe never writes to.
    #[test]
    fn every_recipe_lands_where_the_setup_screen_says_it_does() {
        use crate::setup::{Destination, destination_of};
        for tool in TOOLS {
            let expected = match tool.recipe {
                Recipe::CargoInstall { .. } => Destination::CargoBin,
                Recipe::RustupComponent { .. } | Recipe::Espup => Destination::RustupHome,
                // rustup is a link, and the setup screen handles it on its own.
                Recipe::Manual { .. } => continue,
            };
            assert_eq!(destination_of(tool.name), expected, "{}", tool.name);
        }
    }
}
