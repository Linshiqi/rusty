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
mod channel;
pub mod headless;

mod machine;
mod models;
mod plan;
mod qemu;
mod sheet;

pub use board_file::save as save_board;
pub use channel::{PinChannel, Sensor, Start, connect, pin_level, start_of};
pub use machine::gdb_for;
pub use models::{has_adc_model, has_gpio_model, has_peripherals, has_wave_model};
pub use plan::{plan, prepare};
pub use qemu::{free_port, pins_args, qmp, qmp_args};
#[cfg(test)]
pub(crate) use sheet::load_board_for_test;
pub use sheet::{kit_rows_for, resolve_symbols};

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::machine::*;
    use super::models::*;
    use super::plan::*;
    use super::*;
    use crate::install::GDB_RELEASE;
    use crate::model::{EmbeddedProject, SimPlan};
    use crate::tools;

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
            firmware_dir: None,
            playground: None,
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
            bundled: None,
            target_dir: None,
        }
    }

    /// Three generations at once, which is what a machine that has been
    /// upgraded a few times actually holds. The most capable wins wherever
    /// it is: taking the first on the ladder put a GPIO-only build from a
    /// year ago in front of the bundle's, and firmware reading a bus there
    /// waits for ever.
    #[test]
    fn the_most_capable_copy_wins_even_when_none_is_current() {
        let dir = firmware(BLINKY);
        let write = |root: &Path, contents: &[u8]| {
            let path = root
                .join("qemu")
                .join("bin")
                .join(tools::exe("qemu-system-riscv32"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
            path
        };
        // What a data directory holds after an install from long ago.
        write(&dir.path().join("data"), b"....[rusty:gpio@....");
        // And the bundle of a rusty one version behind this one.
        let bundled = write(
            &dir.path().join("bundle"),
            b"[rusty:gpio@ [rusty:adc@ [rusty:i2c@ [rusty:spi@",
        );
        let machine = Machine {
            tools: Some(dir.path().join("data")),
            bundled: Some(dir.path().join("bundle")),
            target_dir: None,
        };
        assert_eq!(
            machine.find_emulator("qemu-system-riscv32"),
            Some(bundled),
            "neither is current, so the one with more models wins",
        );
    }

    /// Playing a signal is its own capability: a build that carries every
    /// peripheral and no tables is still current for everything else — its
    /// plan says so and names no limit — while one with tables is preferred
    /// to it and says it can play one.
    #[test]
    fn playing_a_signal_is_a_capability_of_its_own() {
        let dir = firmware(BLINKY);
        let write = |root: &Path, contents: &[u8]| {
            let path = root
                .join("qemu")
                .join("bin")
                .join(tools::exe("qemu-system-riscv32"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
            path
        };
        let every: &[u8] =
            b"[rusty:gpio@ [rusty:adc@ [rusty:i2c@ [rusty:spi@ [rusty:pwm@ [rusty:rmt@ [rusty:sw@";
        let before = write(&dir.path().join("data"), every);
        let with_tables = write(
            &dir.path().join("bundle"),
            &[every, b" [rusty:wave@"].concat(),
        );
        assert!(has_peripherals(&before) && !has_wave_model(&before));
        assert!(has_wave_model(&with_tables));

        let both = Machine {
            tools: Some(dir.path().join("data")),
            bundled: Some(dir.path().join("bundle")),
            target_dir: None,
        };
        assert_eq!(
            both.find_emulator("qemu-system-riscv32"),
            Some(with_tables),
            "the build that can play a signal is the more capable one",
        );
        let plan = plan_on(&c3(dir.path()), false, &both);
        assert!(plan.emulator.expect("found").waves);

        let only_before = Machine {
            tools: Some(dir.path().join("data")),
            bundled: None,
            target_dir: None,
        };
        let plan = plan_on(&c3(dir.path()), false, &only_before);
        assert!(plan.limits.is_empty(), "nothing else is out of date");
        let emulator = plan.emulator.expect("found");
        assert!(emulator.peripherals && !emulator.waves);
    }

    /// An early build of rusty's QEMU in the data directory — pins, no
    /// converter, no buses — loses to the current build in the bundle, and
    /// the plan says what the one it chose can do. A stock build is still
    /// found when it is all there is.
    #[test]
    fn the_current_build_of_the_emulator_wins_wherever_it_is() {
        let dir = firmware(BLINKY);
        let write = |root: &Path, contents: &[u8]| {
            let path = root
                .join("qemu")
                .join("bin")
                .join(tools::exe("qemu-system-riscv32"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
            path
        };
        // Pins and the converter and the buses, and *not* the duty timer
        // or the strip channel: an emulator a generation behind this
        // rusty, which is what a data directory holds after an upgrade.
        let early = write(
            &dir.path().join("data"),
            b"[rusty:gpio@ [rusty:adc@ [rusty:i2c@ [rusty:spi@",
        );
        let current = write(
            &dir.path().join("bundle"),
            b"[rusty:gpio@ [rusty:adc@ [rusty:i2c@ [rusty:spi@ [rusty:pwm@ [rusty:rmt@ \
              [rusty:sw@",
        );
        let both = Machine {
            tools: Some(dir.path().join("data")),
            bundled: Some(dir.path().join("bundle")),
            target_dir: None,
        };
        assert_eq!(both.find_emulator("qemu-system-riscv32"), Some(current));
        assert!(!has_peripherals(&early));

        let only_early = Machine {
            tools: Some(dir.path().join("data")),
            bundled: None,
            target_dir: None,
        };
        assert_eq!(
            only_early.find_emulator("qemu-system-riscv32"),
            Some(early.clone()),
            "the early build is still better than none",
        );
        let plan = plan_on(&c3(dir.path()), false, &only_early);
        let emulator = plan.emulator.expect("found");
        assert!(emulator.gpio_model);
        assert!(!emulator.peripherals, "and the plan says what it lacks");
    }

    /// The ESP32's interrupts are compiled into the Xtensa binary alone, so
    /// only that binary is asked for their marker. A C3 build carrying
    /// every other model is current; the same models in an Xtensa binary
    /// without the matrix's are a generation behind — an ESP32 whose timer
    /// never interrupts — and the plan for an ESP32 says so where the plan
    /// for a C3 on the same machine says nothing.
    #[test]
    fn only_the_xtensa_emulator_is_asked_for_the_esp32_interrupts() {
        let dir = firmware(BLINKY);
        let every: &[u8] =
            b"[rusty:gpio@ [rusty:adc@ [rusty:i2c@ [rusty:spi@ [rusty:pwm@ [rusty:rmt@ [rusty:sw@";
        let install = |root: &str, xtensa: &[u8]| {
            let tools = dir.path().join(root);
            for (binary, contents) in [
                ("qemu-system-riscv32", every),
                ("qemu-system-xtensa", xtensa),
            ] {
                let path = tools.join("qemu").join("bin").join(tools::exe(binary));
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, contents).unwrap();
            }
            Machine {
                tools: Some(tools),
                bundled: None,
                target_dir: None,
            }
        };
        let esp32 = project(dir.path(), Some("esp32"), Some("xtensa-esp32-none-elf"));
        let limits = |plan: SimPlan| -> Vec<String> {
            plan.limits.into_iter().map(|limit| limit.kind).collect()
        };
        // What the plan calls a build that is not out of date.
        let is_current = |qemu: &Path| has_gpio_model(qemu) && has_peripherals(qemu);

        let behind = install("behind", every);
        let c3_build = behind.find_emulator("qemu-system-riscv32").expect("found");
        assert!(
            is_current(&c3_build),
            "a C3 build is not asked for the ESP32's matrix"
        );
        let esp32_build = behind.find_emulator("qemu-system-xtensa").expect("found");
        assert!(!is_current(&esp32_build));
        assert_eq!(limits(plan_on(&esp32, false, &behind)), ["esp32-outdated"]);
        assert!(limits(plan_on(&c3(dir.path()), false, &behind)).is_empty());

        let with_matrix = [every, &b" misc.esp32.intmatrix.status"[..]].concat();
        let current = install("current", &with_matrix);
        let esp32_build = current.find_emulator("qemu-system-xtensa").expect("found");
        assert!(is_current(&esp32_build));
        assert!(limits(plan_on(&esp32, false, &current)).is_empty());
    }

    /// Both sockets are the emulator's own. The pin channel waits for rusty
    /// before the guest boots, because the sheet has to be declared before
    /// the firmware reaches for it; the monitor does not, because nothing
    /// about a run depends on it.
    #[test]
    fn the_pin_channel_waits_for_rusty_and_the_monitor_does_not() {
        let pins = pins_args(5555).join(" ");
        assert!(pins.contains("id=pins"), "{pins}");
        assert!(pins.contains("server=on,wait=on"), "{pins}");
        assert!(pins.contains("driver=esp32.gpio,property=pins"), "{pins}");

        let qmp = qmp_args(5556).join(" ");
        assert_eq!(qmp, "-qmp tcp:127.0.0.1:5556,server=on,wait=off");
    }

    /// The plan names the emulator it will boot and whether it models the
    /// pins — read off the binary, so a stock copy dropped over rusty's (or
    /// the reverse) answers for itself.
    #[test]
    fn the_plan_says_whether_its_emulator_models_the_pins() {
        let dir = firmware(BLINKY);
        let with_qemu = machine(dir.path(), &[("qemu", "qemu-system-riscv32")]);
        let plan = plan_on(&c3(dir.path()), false, &with_qemu);
        let emulator = plan.emulator.clone().expect("an emulator was found");
        assert_eq!(emulator.name, "qemu-system-riscv32");
        assert!(!emulator.gpio_model, "an empty file is not rusty's build");
        assert!(plan.missing.iter().all(|t| t.name != "qemu-system-riscv32"));

        std::fs::write(&emulator.path, b"...[rusty:gpio@1] 0=1...").unwrap();
        let plan = plan_on(&c3(dir.path()), false, &with_qemu);
        assert!(plan.emulator.is_some_and(|e| e.gpio_model));

        // No emulator at all: nothing to say about one, and it is missing.
        let bare = machine(dir.path().join("bare").as_path(), &[]);
        let plan = plan_on(&c3(dir.path()), false, &bare);
        assert!(plan.emulator.is_none());
        assert!(plan.missing.iter().any(|t| t.name == "qemu-system-riscv32"));
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
            bundled: None,
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
        assert_eq!(
            board.parts[0].symbol, "Device:LED",
            "and the parts are still the user's, migrated"
        );
        assert!(
            board.wires.iter().any(|w| w.from.pin == "GPIO26"),
            "on the pin the old file named: {:?}",
            board.wires
        );
        assert!(
            board.symbols.iter().any(|s| s.id() == "Device:LED"),
            "with its symbol resolved for the frontend"
        );
        assert!(
            plan.library.iter().any(|s| s.id() == "rusty:Pot"),
            "and the whole library offered"
        );
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
        // QEMU listens, rusty connects — the arrangement CI boots.
        assert!(joined.contains("server=on"), "{joined}");
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
