//! The plan: the three commands a run is, each inspectable before it runs,
//! or exactly why there cannot be one.

use std::path::{Path, PathBuf};

use super::board_file;
use super::machine::{MACHINES, Machine, find_gdb, qemu_data_dir};
use super::models::{has_gpio_model, has_peripherals, has_wave_model};
use super::sheet::resolve_symbols;
use crate::install::GDB_RELEASE;
use crate::model::{CommandPlan, EmbeddedProject, Emulator, SimDebug, SimPlan, SimTool};
use crate::{project, toolchain};

/// Everything needed to simulate `project`, or exactly why not.
///
/// `debug` changes the build, not just the QEMU flags: a release build has
/// no code on many lines, so gdb moves a breakpoint to the next line that
/// does, and the margin ends up marking a line execution never reaches.
/// Debug runs build unoptimised — measured at 284 KB against release's
/// 85 KB on the demo project, which is 7% of a 4 MB flash and the right
/// trade when the point is to stop where you clicked.
pub fn plan(project: &EmbeddedProject, debug: bool) -> SimPlan {
    plan_on(project, debug, &Machine::here())
}

pub(crate) fn plan_on(project: &EmbeddedProject, debug: bool, machine: &Machine) -> SimPlan {
    let Some(chip) = project.chip.as_deref() else {
        return SimPlan::refused(
            "no chip could be detected for this project, and a simulator needs to know \
             which machine to model — set the target in .cargo/config.toml",
        );
    };

    let Some((_, emulator)) = MACHINES.iter().find(|(name, _)| *name == chip) else {
        let known: Vec<&str> = MACHINES.iter().map(|(name, _)| *name).collect();
        return SimPlan::refused(format!(
            "QEMU has no machine model for {chip}; it can model {}",
            known.join(", "),
        ));
    };

    let mut missing = Vec::new();
    let espflash = match machine.find("espflash") {
        Some(path) => path,
        None => {
            missing.push(SimTool {
                name: "espflash".to_string(),
                install: toolchain::install_command("espflash").unwrap_or_default(),
            });
            PathBuf::from("espflash")
        }
    };
    let qemu = match machine.find_emulator(emulator) {
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
    // Which emulator, and whether it models the pins. The stock build's GPIO
    // write handler is empty, so a pin read back there is always 0; said
    // beside Run, so a blinky that prints `false` for ever reads as the
    // emulator's doing rather than the driver's.
    let found_emulator = qemu.is_file().then(|| Emulator {
        name: emulator.to_string(),
        path: qemu.display().to_string(),
        gpio_model: has_gpio_model(&qemu),
        peripherals: has_peripherals(&qemu),
        waves: has_wave_model(&qemu),
    });
    // A refusal past this point still carries what it found missing: the
    // panel offers the installs alongside the reason rather than after it.
    let refuse = |reason: String, missing: Vec<SimTool>| {
        let mut plan = SimPlan::refused(reason);
        plan.missing = missing;
        plan
    };

    let Some(target) = project.configured_target.as_deref() else {
        return refuse(
            "no build target in .cargo/config.toml — the simulator cannot guess where \
             the ELF will land"
                .to_string(),
            missing,
        );
    };
    let root = Path::new(&project.root);
    // Cargo's own directory names, and the profile the build below asks
    // for — one decision, spelled once.
    let profile = if debug { "debug" } else { "release" };
    let elf = match elf_path(root, target, profile, machine.target_dir.as_deref()) {
        Ok(elf) => elf,
        Err(reason) => return refuse(reason, missing),
    };
    let image = "target/rusty-sim/flash.bin".to_string();

    // `--config` rather than an edit: the project's own `[profile.dev]`
    // usually sets `opt-level = "s"` — esp-generate's template does, saying
    // the default debug profile is too slow for the hardware — so dropping
    // `--release` alone still optimises, and breakpoints still move.
    // Overriding on the command line leaves their manifest alone and shows
    // in the dock exactly what ran.
    let build = if debug {
        CommandPlan::new(
            "cargo",
            vec![
                "build".to_string(),
                "--config".to_string(),
                "profile.dev.opt-level=0".to_string(),
            ],
            "unoptimised, so a breakpoint stops on the line you set it on rather than the \
             next one the optimiser left standing",
        )
    } else {
        CommandPlan::new(
            "cargo",
            vec!["build".to_string(), "--release".to_string()],
            "the project's own toolchain builds the exact firmware a device would get",
        )
    };
    let image_args = vec![
        "save-image".to_string(),
        "--chip".to_string(),
        chip.to_string(),
        "--merge".to_string(),
        elf.clone(),
        image.clone(),
    ];
    // Shown by the tool's name and run by its path.
    let image_step = CommandPlan {
        program: espflash.to_string_lossy().into_owned(),
        ..CommandPlan::new(
            "espflash",
            image_args,
            "merges bootloader, partition table and app into the bootable flash image QEMU \
             maps as the SPI flash",
        )
    };
    let mut qemu_args = vec![
        "-M".to_string(),
        chip.to_string(),
        "-nographic".to_string(),
        "-drive".to_string(),
        format!("file={image},if=mtd,format=raw"),
        "-serial".to_string(),
        "mon:stdio".to_string(),
    ];
    // Where the ROM images are, said outright. QEMU is meant to find
    // `../share/qemu` beside its own `bin/` on its own, and Espressif's build
    // does; rusty's Windows build, configured with a build-tree prefix, does
    // not — it started, printed "-bios argument not set, and ROM code binary
    // not found" and exited, on a machine where the file sat exactly where
    // the package had put it. `-L` names the directory and is harmless to a
    // build that would have found it anyway.
    if let Some(data) = qemu_data_dir(&qemu) {
        qemu_args.push("-L".to_string());
        qemu_args.push(data.to_string_lossy().into_owned());
    }
    let run = CommandPlan {
        program: qemu.to_string_lossy().into_owned(),
        ..CommandPlan::new(
            *emulator,
            qemu_args,
            "boots the image in Espressif's QEMU; the serial console streams here until \
             stopped",
        )
    };

    // Debugging is optional on top of the same boot: present when the
    // matching gdb exists, an installable card when it does not.
    let xtensa = *emulator == "qemu-system-xtensa";
    let (debug, debug_tool) = match find_gdb(xtensa, machine) {
        Some(gdb) => (
            Some(SimDebug {
                gdb_command: format!(
                    "\"{}\" \"{elf}\" -ex \"target remote :1234\"",
                    gdb.display(),
                ),
                elf: elf.clone(),
                port: 1234,
            }),
            None,
        ),
        None => {
            let family = if xtensa {
                "xtensa-esp-elf-gdb"
            } else {
                "riscv32-esp-elf-gdb"
            };
            (
                None,
                Some(SimTool {
                    name: family.to_string(),
                    install: format!(
                        "download {family} from https://github.com/espressif/binutils-gdb/releases/tag/{GDB_RELEASE} into the data directory's tools/"
                    ),
                }),
            )
        }
    };

    // The symbol library first, so a sheet's parts can be resolved against
    // it and a library file that would not read is said before the sheet
    // that needed it.
    let library = crate::schematic::load(Some(root));
    let mut notes: Vec<String> = library.warnings.clone();
    // And the parts those symbols may answer as, from the same three
    // layers. A declaration that would not read is a note beside the
    // library's, not a refusal: the board is still a board without it.
    let parts = crate::partfile::load(Some(root));
    notes.extend(parts.warnings);
    let board = board_file::load(root, chip).map(|loaded| {
        notes.extend(loaded.note);
        let mut sheet = loaded.sheet;
        resolve_symbols(&mut sheet, &library);
        notes.append(&mut sheet.notes);
        sheet
    });

    // Whether the copy it will boot is older than every model this rusty
    // drives — the one thing that still limits an ESP32. Only a copy that
    // was found: no emulator at all is already in `missing`, and saying it
    // is out of date as well would be two answers to one question.
    let outdated = found_emulator
        .as_ref()
        .is_some_and(|e| !(e.gpio_model && e.peripherals));

    let mut limits = crate::model::SimLimit::for_chip(chip, outdated);
    // A signal needs the build that plays tables; any other run does not,
    // which is why this is a limit of the sheet's and not an out-of-date
    // emulator's.
    if board
        .as_ref()
        .is_some_and(crate::generator::plays_anything)
        && found_emulator
            .as_ref()
            .is_some_and(|emulator| !emulator.waves)
    {
        limits.push(crate::model::SimLimit::signals_outdated());
    }

    SimPlan {
        supported: true,
        reason: None,
        missing,
        emulator: found_emulator,
        steps: vec![build, image_step, run],
        board,
        library: library.symbols,
        parts: parts.specs,
        debug,
        debug_tool,
        notes,
        limits,
    }
}

/// Where cargo will put the ELF, or why that cannot be said.
///
/// Three things decide it, and the first version of this assumed all three:
/// the binary is the `[package]` name unless a `[[bin]]` renames it, and the
/// directory is `target/` unless `[build] target-dir` or `CARGO_TARGET_DIR`
/// moves it. Each assumption fails as a file-not-found from espflash naming a
/// path nobody asked for — and falling back to `app` when no name could be
/// read at all was a guess dressed as an answer. Refuse, and say which of the
/// three is the problem.
pub(super) fn elf_path(
    root: &Path,
    target: &str,
    profile: &str,
    env_target_dir: Option<&str>,
) -> std::result::Result<String, String> {
    let binary = binary_name(root)?;
    let target_dir = env_target_dir
        .map(str::to_string)
        .or_else(|| configured_target_dir(root))
        .unwrap_or_else(|| "target".to_string());
    Ok(format!(
        "{}/{target}/{profile}/{binary}",
        target_dir.trim_end_matches(['/', '\\']),
    ))
}

/// The one binary the manifest builds, by name.
///
/// A single `[[bin]]` names it outright; otherwise it is the package. Two or
/// more `[[bin]]`s is a question with no right answer — the simulator boots
/// one image — so it is asked back rather than settled by picking the first.
fn binary_name(root: &Path) -> std::result::Result<String, String> {
    let manifest = project::read_toml(&root.join("Cargo.toml")).map_err(|error| {
        format!(
            "Cargo.toml could not be read, so the simulator cannot say which ELF the build \
             produces: {error}"
        )
    })?;

    let bins: Vec<&str> = manifest
        .get("bin")
        .and_then(toml::Value::as_array)
        .map(|bins| {
            bins.iter()
                .filter_map(|bin| bin.get("name").and_then(toml::Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    match bins.as_slice() {
        [one] => return Ok((*one).to_string()),
        [] => {}
        many => {
            return Err(format!(
                "Cargo.toml declares {} binaries ({}) and the simulator boots one image; it \
                 will not pick between them — a project with one [[bin]] says which",
                many.len(),
                many.join(", "),
            ));
        }
    }

    manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            "Cargo.toml names no [package] and no [[bin]], so the simulator cannot say which \
             ELF the build produces"
                .to_string()
        })
}

/// `[build] target-dir` from `.cargo/config.toml`, when the project moves its
/// build output. Relative paths are relative to the project root, which is
/// where the build runs, so they can be used as written.
fn configured_target_dir(root: &Path) -> Option<String> {
    [".cargo/config.toml", ".cargo/config"]
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
        .and_then(|path| project::read_toml(&path).ok())
        .and_then(|config| {
            config
                .get("build")
                .and_then(|build| build.get("target-dir"))
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        })
}

/// Create the directory the image step writes into. espflash does not make
/// parent directories, and "os error 3" from a missing folder reads like a
/// broken tool rather than a missing mkdir.
pub fn prepare(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join("target/rusty-sim"))
}
