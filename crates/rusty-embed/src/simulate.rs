//! Wokwi-style simulation, on Espressif's QEMU.
//!
//! Local-first deliberately: Espressif ships QEMU builds with ESP machine
//! models (`-M esp32c3` and friends), which boot the very image `espflash`
//! would put on a real board — ROM, second-stage bootloader, partition
//! table, app — and speak UART on stdio. No account, no cloud, no token.
//!
//! The loop is three commands, each inspectable in the panel before it runs:
//!
//! 1. `cargo build --release` — the project's own toolchain does the work.
//! 2. `espflash save-image --merge` — a bootable 4MB flash image, the same
//!    bytes a device would hold.
//! 3. `qemu-system-<arch> -M <chip> -nographic` — serial streams back into
//!    the dock until stopped.
//!
//! Refusals name what is missing and how to get it. A chip QEMU has no
//! machine model for is refused with the list of ones it has — a plausible
//! "it might work" would cost someone an afternoon.

use std::path::{Path, PathBuf};

use crate::config;
use crate::model::{CommandPlan, EmbeddedProject, SimPlan, SimTool};

/// Chips Espressif's QEMU actually models, with the system emulator each
/// needs. Kept small and honest — c6/h2/p4 have no machine model yet.
const MACHINES: &[(&str, &str)] = &[
    ("esp32c3", "qemu-system-riscv32"),
    ("esp32", "qemu-system-xtensa"),
    ("esp32s3", "qemu-system-xtensa"),
];

/// Everything needed to simulate `project`, or exactly why not.
pub fn plan(project: &EmbeddedProject) -> SimPlan {
    let Some(chip) = project.chip.as_deref() else {
        return SimPlan {
            supported: false,
            reason: Some(
                "no chip could be detected for this project, and a simulator needs to know \
                 which machine to model — set the target in .cargo/config.toml"
                    .to_string(),
            ),
            missing: Vec::new(),
            steps: Vec::new(),
        };
    };

    let Some((_, emulator)) = MACHINES.iter().find(|(name, _)| *name == chip) else {
        let known: Vec<&str> = MACHINES.iter().map(|(name, _)| *name).collect();
        return SimPlan {
            supported: false,
            reason: Some(format!(
                "QEMU has no machine model for {chip}; it can model {}",
                known.join(", "),
            )),
            missing: Vec::new(),
            steps: Vec::new(),
        };
    };

    let mut missing = Vec::new();
    let espflash = match find_espflash() {
        Some(path) => path,
        None => {
            missing.push(SimTool {
                name: "espflash".to_string(),
                install: "cargo install espflash".to_string(),
            });
            PathBuf::from("espflash")
        }
    };
    let qemu = match find_qemu(emulator) {
        Some(path) => path,
        None => {
            missing.push(SimTool {
                name: emulator.to_string(),
                install: format!(
                    "download the {emulator} build from \
                     https://github.com/espressif/qemu/releases and unpack it into the data \
                     directory's tools/qemu/"
                ),
            });
            PathBuf::from(emulator)
        }
    };

    let Some(target) = project.configured_target.as_deref() else {
        return SimPlan {
            supported: false,
            reason: Some(
                "no build target in .cargo/config.toml — the simulator cannot guess where \
                 the ELF will land"
                    .to_string(),
            ),
            missing,
            steps: Vec::new(),
        };
    };
    let binary = package_name(Path::new(&project.root)).unwrap_or_else(|| "app".to_string());
    let elf = format!("target/{target}/release/{binary}");
    let image = "target/rusty-sim/flash.bin".to_string();

    let build = CommandPlan {
        program: "cargo".to_string(),
        args: vec!["build".to_string(), "--release".to_string()],
        display: "cargo build --release".to_string(),
        rationale: "the project's own toolchain builds the exact firmware a device would get"
            .to_string(),
    };
    let mut image_args = vec![
        "save-image".to_string(),
        "--chip".to_string(),
        chip.to_string(),
        "--merge".to_string(),
        elf.clone(),
        image.clone(),
    ];
    let image_step = CommandPlan {
        display: format!("espflash {}", image_args.join(" ")),
        program: espflash.to_string_lossy().into_owned(),
        args: std::mem::take(&mut image_args),
        rationale: "merges bootloader, partition table and app into the bootable flash image \
                    QEMU maps as the SPI flash"
            .to_string(),
    };
    let qemu_args = vec![
        "-M".to_string(),
        chip.to_string(),
        "-nographic".to_string(),
        "-drive".to_string(),
        format!("file={image},if=mtd,format=raw"),
        "-serial".to_string(),
        "mon:stdio".to_string(),
    ];
    let run = CommandPlan {
        display: format!("{emulator} {}", qemu_args.join(" ")),
        program: qemu.to_string_lossy().into_owned(),
        args: qemu_args,
        rationale: "boots the image in Espressif's QEMU; the serial console streams here \
                    until stopped"
            .to_string(),
    };

    SimPlan {
        supported: true,
        reason: None,
        missing,
        steps: vec![build, image_step, run],
    }
}

/// The QEMU release every install pulls — the version this pipeline is
/// proven against. Bumped deliberately, not discovered at run time: an
/// installer that fetches "latest" breaks the day upstream changes layout.
const QEMU_RELEASE: &str = "esp-develop-9.2.2-20260417";
const QEMU_VERSION: &str = "esp_develop_9.2.2_20260417";

/// How to install a tool the plan reported missing, as inspectable steps —
/// one click in the panel, the dock shows every line, and only a failure
/// sends anyone to the manual instructions.
pub fn install_steps(tool: &str) -> std::result::Result<Vec<CommandPlan>, String> {
    if tool == "espflash" {
        return Ok(vec![CommandPlan {
            program: "cargo".to_string(),
            args: vec![
                "install".to_string(),
                "espflash".to_string(),
                "--locked".to_string(),
            ],
            display: "cargo install espflash --locked".to_string(),
            rationale: "builds espflash into ~/.cargo/bin, where the simulator looks"
                .to_string(),
        }]);
    }

    if let Some(arch) = tool.strip_prefix("qemu-system-") {
        if !cfg!(windows) {
            return Err(format!(
                "one-click install only knows the Windows build so far — download the                  {tool} build from https://github.com/espressif/qemu/releases/tag/{QEMU_RELEASE}                  and unpack it into the data directory's tools/qemu/"
            ));
        }
        let Some(tools) = config::data_dir().map(|d| d.join("tools")) else {
            return Err("the data directory could not be resolved".to_string());
        };
        std::fs::create_dir_all(&tools)
            .map_err(|e| format!("could not create {}: {e}", tools.display()))?;
        let archive = tools.join(format!("qemu-{arch}.tar.xz"));
        let url = format!(
            "https://github.com/espressif/qemu/releases/download/{QEMU_RELEASE}/qemu-{arch}-softmmu-{QEMU_VERSION}-x86_64-w64-mingw32.tar.xz"
        );
        let archive_text = archive.to_string_lossy().into_owned();
        let tools_text = tools.to_string_lossy().into_owned();
        return Ok(vec![
            CommandPlan {
                program: "curl".to_string(),
                args: vec![
                    "-L".to_string(),
                    "--fail".to_string(),
                    "-o".to_string(),
                    archive_text.clone(),
                    url.clone(),
                ],
                display: format!("curl -L --fail -o {archive_text} {url}"),
                rationale: "downloads Espressif's own QEMU build — curl ships with Windows 10+"
                    .to_string(),
            },
            CommandPlan {
                program: "tar".to_string(),
                args: vec![
                    "-xf".to_string(),
                    archive_text.clone(),
                    "-C".to_string(),
                    tools_text.clone(),
                ],
                display: format!("tar -xf {archive_text} -C {tools_text}"),
                rationale: "unpacks into the data directory's tools/qemu — bsdtar handles                             .tar.xz and also ships with Windows"
                    .to_string(),
            },
            CommandPlan {
                program: "cmd".to_string(),
                args: vec!["/c".to_string(), "del".to_string(), archive_text.clone()],
                display: format!("del {archive_text}"),
                rationale: "drops the 38MB archive now that it is unpacked".to_string(),
            },
        ]);
    }

    Err(format!("no installer for {tool}"))
}

/// Create the directory the image step writes into. espflash does not make
/// parent directories, and "os error 3" from a missing folder reads like a
/// broken tool rather than a missing mkdir.
pub fn prepare(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join("target/rusty-sim"))
}

/// espflash: PATH first, then the copy the workbench keeps in its data
/// directory, then cargo's bin dir.
fn find_espflash() -> Option<PathBuf> {
    if let Some(path) = on_path("espflash") {
        return Some(path);
    }
    if let Some(tools) = config::data_dir().map(|d| d.join("tools/espflash").join(exe("espflash")))
        && tools.is_file()
    {
        return Some(tools);
    }
    let cargo = home_dir()?.join(".cargo/bin").join(exe("espflash"));
    cargo.is_file().then_some(cargo)
}

/// QEMU: PATH first, then the data directory's tools/qemu/bin.
fn find_qemu(emulator: &str) -> Option<PathBuf> {
    if let Some(path) = on_path(emulator) {
        return Some(path);
    }
    let tools = config::data_dir()?.join("tools/qemu/bin").join(exe(emulator));
    tools.is_file().then_some(tools)
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(exe(name));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// The `[package] name` of the project's manifest — where cargo will put the
/// ELF. A scan, like the edition scan in rusty-edit, for the same reason.
fn package_name(root: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package
            && let Some(rest) = line.strip_prefix("name")
        {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(chip: Option<&str>, target: Option<&str>) -> EmbeddedProject {
        EmbeddedProject {
            root: ".".to_string(),
            chip: chip.map(str::to_string),
            chip_source: None,
            runtime: None,
            configured_target: target.map(str::to_string),
            configured_toolchain: None,
            frameworks: Vec::new(),
            uses_defmt: false,
            uses_embassy: false,
            evidence: Vec::new(),
            problems: Vec::new(),
        }
    }

    #[test]
    fn install_steps_know_their_tools_and_refuse_strangers() {
        let espflash = install_steps("espflash").expect("espflash installs");
        assert_eq!(espflash.len(), 1);
        assert!(espflash[0].display.contains("cargo install espflash"));

        if cfg!(windows) {
            let qemu = install_steps("qemu-system-xtensa").expect("qemu installs");
            assert!(qemu[0].display.contains("qemu-xtensa-softmmu"), "{}", qemu[0].display);
            assert!(qemu[0].display.contains(super::QEMU_RELEASE));
            assert!(qemu[1].display.starts_with("tar -xf"));
        }

        assert!(install_steps("probe-rs").is_err(), "unknown tools are named, not guessed");
    }

    #[test]
    fn an_unmodelled_chip_is_refused_with_the_supported_list() {
        let plan = plan(&project(Some("esp32c6"), Some("riscv32imac-unknown-none-elf")));
        assert!(!plan.supported);
        let reason = plan.reason.expect("names the problem");
        assert!(reason.contains("esp32c6"), "{reason}");
        assert!(reason.contains("esp32c3"), "the alternatives are listed: {reason}");
    }

    #[test]
    fn a_supported_chip_plans_three_inspectable_steps() {
        let sim = plan(&project(Some("esp32c3"), Some("riscv32imc-unknown-none-elf")));
        assert!(sim.supported, "{:?}", sim.reason);
        assert_eq!(sim.steps.len(), 3);
        assert_eq!(sim.steps[0].display, "cargo build --release");
        assert!(sim.steps[1].display.contains("save-image"), "{}", sim.steps[1].display);
        assert!(sim.steps[1].display.contains("--merge"));
        assert!(sim.steps[2].display.contains("-M esp32c3"), "{}", sim.steps[2].display);
        assert!(sim.steps[2].display.contains("if=mtd"));
    }

    #[test]
    fn no_chip_and_no_target_refuse_rather_than_guess() {
        assert!(!plan(&project(None, None)).supported);
        let sim = plan(&project(Some("esp32c3"), None));
        assert!(!sim.supported);
        assert!(sim.reason.expect("says why").contains(".cargo/config.toml"));
    }
}
