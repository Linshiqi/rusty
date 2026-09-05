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
//!
//! This module is the simulator, and nothing else. The `.rusty/sim.toml` file
//! format is [`board_file`]; version pins, the installer and the download
//! ladder live in `install`; proxy policy in `net`; finding a binary in
//! `tools`.

mod board_file;

use std::path::{Path, PathBuf};

use crate::install::GDB_RELEASE;
use crate::model::{CommandPlan, EmbeddedProject, PartDef, SimDebug, SimPlan, SimTool};
use crate::{project, toolchain, tools};

pub use board_file::save as save_board;

/// Chips Espressif's QEMU actually models, with the system emulator each
/// needs. Kept small and honest — c6/h2/p4 have no machine model yet.
const MACHINES: &[(&str, &str)] = &[
    ("esp32c3", "qemu-system-riscv32"),
    ("esp32", "qemu-system-xtensa"),
    ("esp32s3", "qemu-system-xtensa"),
];

/// What the machine has that a plan depends on, resolved once and handed in.
///
/// Handed in rather than read inside [`plan`] so a plan can be tested against
/// a directory a test made: the positive half of "the debugger reads the ELF
/// the run built" needs a gdb to exist, and the machine running the test may
/// have none — CI has none. [`Machine::here`] is what the app uses.
pub(crate) struct Machine {
    /// The data directory's `tools/`, where the installer unpacks QEMU and the
    /// debuggers. `None` when there is no data directory.
    tools: Option<PathBuf>,
    /// `CARGO_TARGET_DIR`, which outranks `[build] target-dir` for the cargo
    /// this plan will spawn — it inherits rusty's environment.
    target_dir: Option<String>,
}

impl Machine {
    pub(crate) fn here() -> Self {
        Machine {
            tools: tools::data_tools_dir(),
            target_dir: std::env::var("CARGO_TARGET_DIR")
                .ok()
                .filter(|dir| !dir.trim().is_empty()),
        }
    }

    fn find(&self, binary: &str) -> Option<PathBuf> {
        tools::find_in(binary, self.tools.as_deref())
    }
}

/// QEMU's data directory beside an emulator binary — `bin/../share/qemu`,
/// the layout both Espressif's package and rusty's use — when it is there.
/// The ROM images live in it, and a QEMU that cannot find it boots nothing.
fn qemu_data_dir(emulator: &Path) -> Option<PathBuf> {
    let data = emulator.parent()?.parent()?.join("share").join("qemu");
    data.is_dir().then_some(data)
}

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
    let qemu = match machine.find(emulator) {
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
        CommandPlan {
            program: "cargo".to_string(),
            args: vec![
                "build".to_string(),
                "--config".to_string(),
                "profile.dev.opt-level=0".to_string(),
            ],
            display: "cargo build --config profile.dev.opt-level=0".to_string(),
            rationale: "unoptimised, so a breakpoint stops on the line you set it on rather \
                        than the next one the optimiser left standing"
                .to_string(),
            warning: None,
        }
    } else {
        CommandPlan {
            program: "cargo".to_string(),
            args: vec!["build".to_string(), "--release".to_string()],
            display: "cargo build --release".to_string(),
            rationale: "the project's own toolchain builds the exact firmware a device would get"
                .to_string(),
            warning: None,
        }
    };
    let image_args = vec![
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
        args: image_args,
        rationale: "merges bootloader, partition table and app into the bootable flash image \
                    QEMU maps as the SPI flash"
            .to_string(),
        warning: None,
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
        display: format!("{emulator} {}", qemu_args.join(" ")),
        program: qemu.to_string_lossy().into_owned(),
        args: qemu_args,
        rationale: "boots the image in Espressif's QEMU; the serial console streams here \
                    until stopped"
            .to_string(),
        warning: None,
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

    let (board, notes) = match board_file::load(root, chip) {
        Some(loaded) => (Some(loaded.board), loaded.note.into_iter().collect()),
        None => (None, Vec::new()),
    };

    SimPlan {
        supported: true,
        reason: None,
        missing,
        steps: vec![build, image_step, run],
        board,
        parts: user_parts(root),
        debug,
        debug_tool,
        notes,
    }
}

/// The gdb that can debug this project's chip, if it is installed.
///
/// Architecture decides: an Xtensa gdb cannot debug a RISC-V image, and the
/// error it produces names neither the chip nor the fix.
pub fn gdb_for(project: &EmbeddedProject) -> Option<PathBuf> {
    let xtensa = project
        .configured_target
        .as_deref()
        .is_some_and(|t| t.starts_with("xtensa"));
    find_gdb(xtensa, &Machine::here())
}

/// The debugger for one architecture, by the same ladder every other binary
/// is found with. The Xtensa build is named after the chip family it was
/// built for rather than the archive it came in.
fn find_gdb(xtensa: bool, machine: &Machine) -> Option<PathBuf> {
    machine.find(if xtensa {
        "xtensa-esp32-elf-gdb"
    } else {
        "riscv32-esp-elf-gdb"
    })
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
fn elf_path(
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

/// The user's own part definitions, from `.rusty/parts/*.toml`.
///
/// A file that does not parse is skipped rather than sinking the whole
/// library; the panel offers what could be read.
pub fn user_parts(root: &Path) -> Vec<PartDef> {
    #[derive(serde::Deserialize)]
    struct File {
        name: String,
        color: Option<String>,
    }

    let Ok(entries) = std::fs::read_dir(root.join(".rusty/parts")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(parsed) = toml::from_str::<File>(&text) else {
            continue;
        };
        out.push(PartDef {
            name: parsed.name,
            color: parsed.color.unwrap_or_else(|| "green".to_string()),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Create the directory the image step writes into. espflash does not make
/// parent directories, and "os error 3" from a missing folder reads like a
/// broken tool rather than a missing mkdir.
pub fn prepare(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root.join("target/rusty-sim"))
}

/// The line only rusty's GPIO model emits, and the thing to look for in a
/// binary to know whether it has one.
///
/// A marker in the emulator's own output rather than a version file beside
/// it: the question is "does *this* binary keep pin state", and a user who
/// dropped Espressif's build into the same directory must get the right
/// answer. The CI gate greps the same literal for the same reason.
const GPIO_MODEL_MARKER: &[u8] = b"[rusty:gpio@";

/// Does this emulator model GPIO, or is it the stock one whose write handler
/// is an empty function?
///
/// Everything downstream branches on this: with the model, the board shows
/// what a pin *is* and a button drives the register the firmware reads;
/// without it, the firmware has to narrate its own pins over the serial line
/// and a button arrives as a `B14=1` message. Both work — but claiming the
/// first while running the second would show a dark LED for correct firmware,
/// which is the failure this whole path exists to remove.
///
/// Cached on path, length and mtime, because it is asked once per run and the
/// answer costs a scan of a hundred-megabyte file.
pub fn has_gpio_model(qemu: &Path) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    type Stamp = (std::path::PathBuf, u64, Option<std::time::SystemTime>);
    static SEEN: OnceLock<Mutex<HashMap<Stamp, bool>>> = OnceLock::new();

    let Ok(meta) = std::fs::metadata(qemu) else {
        return false;
    };
    let stamp: Stamp = (qemu.to_path_buf(), meta.len(), meta.modified().ok());

    let cache = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(seen) = cache.lock()
        && let Some(known) = seen.get(&stamp)
    {
        return *known;
    }

    let found = scan_for(qemu, GPIO_MODEL_MARKER);
    if let Ok(mut seen) = cache.lock() {
        seen.insert(stamp, found);
    }
    found
}

/// Is `needle` anywhere in this file?
///
/// Chunked with an overlap of `needle.len() - 1`, so a marker straddling a
/// chunk boundary is still found — reading a hundred megabytes into memory to
/// avoid thinking about that is the version of this that makes the toolchain
/// panel stutter.
fn scan_for(path: &Path, needle: &[u8]) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let overlap = needle.len() - 1;
    let mut buffer = vec![0u8; 1 << 20];
    let mut filled = 0usize;
    loop {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => return false,
            Ok(read) => {
                filled += read;
                if buffer[..filled].windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
                if filled + overlap >= buffer.len() {
                    buffer.copy_within(filled - overlap..filled, 0);
                    filled = overlap;
                }
            }
            Err(_) => return false,
        }
    }
}

/// Where pin changes leave the emulator and host-driven levels go back in.
///
/// Its own chardev, not the serial line: the UART belongs to the firmware,
/// and interleaving the two would make each unreadable to whoever wanted the
/// other. `-global` rather than `-device` because the machine creates the
/// GPIO device itself — there is no `-device` line to hang a chardev off.
///
/// QEMU listens and rusty connects, which is the arrangement the CI gate
/// proves; having rusty listen would be a second arrangement nothing has
/// booted.
pub fn pins_args(port: u16) -> Vec<String> {
    vec![
        "-chardev".to_string(),
        format!("socket,id=pins,host=127.0.0.1,port={port},server=on,wait=off"),
        "-global".to_string(),
        "driver=esp32.gpio,property=pins,value=pins".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project directory with the manifest a plan needs to read.
    fn firmware(manifest: &str) -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("rusty-sim")
            .tempdir()
            .expect("tempdir");
        std::fs::write(dir.path().join("Cargo.toml"), manifest).expect("manifest");
        dir
    }

    const BLINKY: &str = "[package]\nname = \"blinky\"\nversion = \"0.1.0\"\n";

    fn project(root: &Path, chip: Option<&str>, target: Option<&str>) -> EmbeddedProject {
        EmbeddedProject {
            root: root.display().to_string(),
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
            c_interop: Default::default(),
        }
    }

    fn c3(root: &Path) -> EmbeddedProject {
        project(root, Some("esp32c3"), Some("riscv32imc-unknown-none-elf"))
    }

    /// A machine whose tools directory is a temp dir, with the binaries named
    /// in `installed` present under `<family>/bin/`.
    fn machine(dir: &Path, installed: &[(&str, &str)]) -> Machine {
        let tools = dir.join("tools");
        for (family, binary) in installed {
            let path = tools.join(family).join("bin").join(tools::exe(binary));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"").unwrap();
        }
        Machine {
            tools: Some(tools),
            target_dir: None,
        }
    }

    /// A debug run is a different build, not just different QEMU flags.
    ///
    /// Dropping `--release` would not be enough: esp-generate's template
    /// sets `[profile.dev] opt-level = "s"`, so the dev profile optimises
    /// too and breakpoints still move off the line they were set on.
    #[test]
    fn a_debug_run_builds_unoptimised_and_takes_that_elf() {
        let dir = firmware(BLINKY);
        // A gdb exists on this machine — the one the test put there — so the
        // debugger half of the plan is asserted rather than shrugged past.
        // The first version of this test used `is_some_and` so that CI, which
        // has no gdb, would pass, and `is_some_and` is false for `None`: CI
        // failed on exactly the machine the leniency was for.
        let machine = machine(
            dir.path(),
            &[("riscv32-esp-elf-gdb", "riscv32-esp-elf-gdb")],
        );
        let release = plan_on(&c3(dir.path()), false, &machine);
        let debug = plan_on(&c3(dir.path()), true, &machine);

        assert!(
            release.steps[0].display.contains("--release"),
            "the ordinary run builds what a device would get: {}",
            release.steps[0].display,
        );
        assert!(
            debug.steps[0].display.contains("profile.dev.opt-level=0"),
            "the debug run overrides the profile on the command line rather than editing \
             anybody's manifest: {}",
            debug.steps[0].display,
        );
        assert!(
            debug.steps[1].display.contains("/debug/blinky"),
            "and images the ELF that build produced: {}",
            debug.steps[1].display,
        );
        assert!(
            release.steps[1].display.contains("/release/blinky"),
            "while a release run images its own: {}",
            release.steps[1].display,
        );
        // The two plans name *different* binaries. That is the whole reason
        // only the run that armed the target may say which one gdb reads:
        // pointing the debugger at the release ELF while the unoptimised
        // image ran reported the breakpoint six lines down and never hit it,
        // with nothing anywhere saying the two were different builds.
        let debugger = debug.debug.expect("the gdb the test installed is found");
        assert!(debugger.elf.contains("/debug/blinky"), "{}", debugger.elf);
        assert!(
            debugger.gdb_command.contains("riscv32-esp-elf-gdb"),
            "{}",
            debugger.gdb_command,
        );
        assert!(
            release
                .debug
                .is_some_and(|d| d.elf.contains("/release/blinky")),
            "each reading the build it belongs to",
        );
        assert!(
            debug.debug_tool.is_none(),
            "nothing to install when it is there"
        );
    }

    /// Exactly one of "here is the debugger" and "here is how to install it"
    /// — never both, never neither. The negative half cannot be forced on a
    /// machine whose PATH carries a gdb, so it is the pair that is pinned.
    #[test]
    fn a_plan_offers_the_debugger_or_its_installer_and_never_both() {
        let dir = firmware(BLINKY);
        let bare = Machine {
            tools: Some(dir.path().join("tools")),
            target_dir: None,
        };
        let plan = plan_on(&c3(dir.path()), true, &bare);
        assert!(plan.supported, "{:?}", plan.reason);
        assert_ne!(
            plan.debug.is_some(),
            plan.debug_tool.is_some(),
            "debug={:?} debug_tool={:?}",
            plan.debug,
            plan.debug_tool,
        );
        if let Some(tool) = &plan.debug_tool {
            assert_eq!(tool.name, "riscv32-esp-elf-gdb");
            assert!(tool.install.contains(GDB_RELEASE), "{}", tool.install);
        }
    }

    /// The binary is what the manifest says it is, not the package name by
    /// assumption and never `app` by default.
    #[test]
    fn the_elf_follows_a_renamed_binary_and_refuses_to_guess_one() {
        let renamed = firmware(
            "[package]\nname = \"blinky\"\n\n[[bin]]\nname = \"firmware\"\npath = \"src/main.rs\"\n",
        );
        assert_eq!(
            elf_path(
                renamed.path(),
                "riscv32imc-unknown-none-elf",
                "release",
                None
            )
            .as_deref(),
            Ok("target/riscv32imc-unknown-none-elf/release/firmware"),
        );

        let two = firmware(
            "[package]\nname = \"blinky\"\n\n[[bin]]\nname = \"one\"\n\n[[bin]]\nname = \"two\"\n",
        );
        let refusal = elf_path(two.path(), "riscv32imc-unknown-none-elf", "release", None)
            .expect_err("two binaries is a question, not an answer");
        assert!(
            refusal.contains("one") && refusal.contains("two"),
            "both are named: {refusal}"
        );

        let nameless = firmware("[dependencies]\nesp-hal = \"1\"\n");
        let refusal = elf_path(
            nameless.path(),
            "riscv32imc-unknown-none-elf",
            "release",
            None,
        )
        .expect_err("no name is a refusal, not `app`");
        assert!(refusal.contains("no [package]"), "{refusal}");
        assert!(!refusal.contains("app/"), "{refusal}");

        // And through the plan, so the panel sees the reason.
        let plan = plan_on(&c3(nameless.path()), false, &machine(nameless.path(), &[]));
        assert!(!plan.supported);
        assert!(plan.reason.is_some_and(|r| r.contains("Cargo.toml")));
    }

    /// A build that lands somewhere other than `target/` has to be imaged
    /// from there; `CARGO_TARGET_DIR` in the environment outranks the config
    /// file, as it does for cargo.
    #[test]
    fn the_elf_follows_a_moved_target_directory() {
        let dir = firmware(BLINKY);
        std::fs::create_dir_all(dir.path().join(".cargo")).unwrap();
        std::fs::write(
            dir.path().join(".cargo/config.toml"),
            "[build]\ntarget = \"riscv32imc-unknown-none-elf\"\ntarget-dir = \"out/\"\n",
        )
        .unwrap();
        assert_eq!(
            elf_path(dir.path(), "riscv32imc-unknown-none-elf", "debug", None).as_deref(),
            Ok("out/riscv32imc-unknown-none-elf/debug/blinky"),
            "the trailing slash the file spelled does not double up",
        );
        assert_eq!(
            elf_path(
                dir.path(),
                "riscv32imc-unknown-none-elf",
                "debug",
                Some("/tmp/builds")
            )
            .as_deref(),
            Ok("/tmp/builds/riscv32imc-unknown-none-elf/debug/blinky"),
        );
    }

    #[test]
    fn user_parts_load_and_bad_files_are_skipped() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-parts")
            .tempdir()
            .expect("tempdir");
        let parts = dir.path().join(".rusty/parts");
        std::fs::create_dir_all(&parts).expect("dirs");
        std::fs::write(
            parts.join("relay.toml"),
            "name = \"relay\"\ncolor = \"red\"\n",
        )
        .expect("write");
        std::fs::write(parts.join("buzzer.toml"), "name = \"buzzer\"\n").expect("write");
        std::fs::write(parts.join("broken.toml"), "not = = toml").expect("write");

        let defs = user_parts(dir.path());
        assert_eq!(defs.len(), 2, "{defs:?}");
        assert_eq!(defs[0].name, "buzzer");
        assert_eq!(defs[0].color, "green");
        assert_eq!(defs[1].name, "relay");
        assert_eq!(defs[1].color, "red");
        assert!(user_parts(Path::new("nowhere")).is_empty());
    }

    /// The board a plan carries is drawn for the chip being simulated, and a
    /// file that says otherwise is answered in the plan's notes rather than
    /// by drawing the other part's header.
    #[test]
    fn a_board_file_for_another_chip_is_drawn_for_this_one_and_noted() {
        let dir = firmware(BLINKY);
        std::fs::create_dir_all(dir.path().join(".rusty")).unwrap();
        std::fs::write(
            dir.path().join(".rusty/sim.toml"),
            "[board]\nchip = \"esp32\"\n[[led]]\npin = 26\n",
        )
        .unwrap();
        let plan = plan_on(&c3(dir.path()), false, &machine(dir.path(), &[]));
        let board = plan.board.expect("the board is still drawn");
        assert_eq!(board.chip, "esp32c3", "pin rows follow the build");
        assert_eq!(board.leds[0].pin, 26, "and the parts are still the user's");
        assert!(
            plan.notes
                .iter()
                .any(|n| n.contains("esp32") && n.contains("esp32c3")),
            "the disagreement is said: {:?}",
            plan.notes,
        );
    }

    /// The marker has to be found wherever it lands, including across a read
    /// boundary — which is the case a chunked scan gets wrong, and it gets it
    /// wrong silently: the answer is "this is the stock emulator", and the
    /// board quietly goes back to trusting the firmware's narration.
    #[test]
    fn the_model_marker_is_found_even_when_it_straddles_a_chunk() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-scan")
            .tempdir()
            .expect("tempdir");

        let chunk = 1 << 20;
        for offset in [0usize, 4096, chunk - 6, chunk, chunk + 1, chunk * 2 - 3] {
            let path = dir.path().join(format!("marked-{offset}.bin"));
            let mut bytes = vec![b'.'; offset + GPIO_MODEL_MARKER.len() + 4096];
            bytes[offset..offset + GPIO_MODEL_MARKER.len()].copy_from_slice(GPIO_MODEL_MARKER);
            std::fs::write(&path, &bytes).expect("write");
            assert!(
                scan_for(&path, GPIO_MODEL_MARKER),
                "marker at byte {offset} was missed",
            );
        }

        // And a binary without it must not be mistaken for one with it: the
        // stock emulator answering "yes" is a board claiming pin state it
        // does not have.
        let plain = dir.path().join("stock.bin");
        std::fs::write(&plain, vec![b'.'; chunk * 2]).expect("write");
        assert!(!scan_for(&plain, GPIO_MODEL_MARKER));
        assert!(!has_gpio_model(&plain));

        // A path that is not there is not a model, and must not panic.
        assert!(!has_gpio_model(&dir.path().join("absent")));
    }

    #[test]
    fn the_pin_channel_is_a_chardev_of_its_own() {
        let args = pins_args(4444);
        let joined = args.join(" ");
        assert!(joined.contains("socket,id=pins"), "{joined}");
        assert!(joined.contains("port=4444"), "{joined}");
        // QEMU listens, rusty connects — the arrangement CI boots. `wait=off`
        // so a run still starts when nothing ever connects.
        assert!(joined.contains("server=on"), "{joined}");
        assert!(joined.contains("wait=off"), "{joined}");
        // -global, because the machine creates the device; there is no
        // -device line to attach the chardev to.
        assert!(
            joined.contains("-global driver=esp32.gpio,property=pins,value=pins"),
            "{joined}",
        );
        // Never the serial line: that one belongs to the firmware.
        assert!(!joined.contains("-serial"), "{joined}");
    }

    #[test]
    fn an_unmodelled_chip_is_refused_with_the_supported_list() {
        let dir = firmware(BLINKY);
        let plan = plan_on(
            &project(
                dir.path(),
                Some("esp32c6"),
                Some("riscv32imac-unknown-none-elf"),
            ),
            false,
            &machine(dir.path(), &[]),
        );
        assert!(!plan.supported);
        let reason = plan.reason.expect("names the problem");
        assert!(reason.contains("esp32c6"), "{reason}");
        assert!(
            reason.contains("esp32c3"),
            "the alternatives are listed: {reason}"
        );
    }

    #[test]
    fn a_supported_chip_plans_three_inspectable_steps() {
        let dir = firmware(BLINKY);
        let sim = plan_on(&c3(dir.path()), false, &machine(dir.path(), &[]));
        assert!(sim.supported, "{:?}", sim.reason);
        assert_eq!(sim.steps.len(), 3);
        assert_eq!(sim.steps[0].display, "cargo build --release");
        assert!(
            sim.steps[1].display.contains("save-image"),
            "{}",
            sim.steps[1].display
        );
        assert!(sim.steps[1].display.contains("--merge"));
        assert!(
            sim.steps[2].display.contains("-M esp32c3"),
            "{}",
            sim.steps[2].display
        );
        assert!(sim.steps[2].display.contains("if=mtd"));
        assert!(
            sim.notes.is_empty(),
            "nothing to note about a plain project"
        );
    }

    /// The ROM directory is named on the command line when it exists beside
    /// the emulator, and left to QEMU when it does not: rusty's Windows
    /// build cannot find `../share/qemu` on its own, and Espressif's needs
    /// no help — `-L` is right for both.
    #[test]
    fn the_rom_directory_is_named_when_it_sits_beside_the_emulator() {
        let dir = firmware(BLINKY);
        let bare = machine(dir.path(), &[("qemu", "qemu-system-riscv32")]);
        let without = plan_on(&c3(dir.path()), false, &bare);
        assert!(
            !without.steps[2].args.iter().any(|a| a == "-L"),
            "no share/qemu, nothing to name: {}",
            without.steps[2].display
        );

        let share = dir
            .path()
            .join("tools")
            .join("qemu")
            .join("share")
            .join("qemu");
        std::fs::create_dir_all(&share).unwrap();
        let with = plan_on(&c3(dir.path()), false, &bare);
        let args = &with.steps[2].args;
        let at = args.iter().position(|a| a == "-L").expect("-L present");
        assert_eq!(
            std::path::Path::new(&args[at + 1]),
            share.as_path(),
            "{}",
            with.steps[2].display
        );
    }

    /// A tool the plan found missing travels with the refusal, so the panel
    /// can offer the install beside the reason rather than after it.
    #[test]
    fn no_chip_and_no_target_refuse_rather_than_guess() {
        let dir = firmware(BLINKY);
        let machine = machine(dir.path(), &[]);
        assert!(!plan_on(&project(dir.path(), None, None), false, &machine).supported);
        let sim = plan_on(&project(dir.path(), Some("esp32c3"), None), false, &machine);
        assert!(!sim.supported);
        assert!(sim.reason.expect("says why").contains(".cargo/config.toml"));
        if !sim.missing.is_empty() {
            assert!(
                sim.missing.iter().all(|tool| !tool.install.is_empty()),
                "every missing tool says how to get it: {:?}",
                sim.missing,
            );
        }
    }
}
