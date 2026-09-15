//! Starting a project, with the consequences spelled out.
//!
//! `esp-generate` already asks the right questions. What it does not do — what
//! nothing does — is say what each answer commits you to. A beginner picking
//! "ESP32" over "ESP32-C3" from a list has no way to know that one of those
//! means downloading a forked LLVM and the other does not; picking `std` over
//! `no_std` decides whether they get threads and sockets or a 30-second first
//! build instead of a five-minute one.
//!
//! So the generator invocation is a [`CommandPlan`] like any other, and the
//! interesting output is [`explain`].

use std::path::Path;

use crate::{
    chip,
    error::{Error, Result},
    model::{CommandPlan, Explanation, Runtime, ToolchainRequirement, WizardChoice, WizardLayout},
};

/// Options `esp-generate` understands, what each one costs, and what it cannot
/// work without.
///
/// Kept here rather than fetched from the generator so the wizard can render
/// and explain before anything is installed — which is exactly the moment a
/// first-time user is at.
///
/// The requirements are the part that earns its place. `esp-generate` enforces
/// them and rejects the whole run with `Invalid options provided`, which the
/// wizard used to discover only after the user had chosen a folder. Knowing
/// them here means a combination that cannot work is never offered.
const OPTIONS: &[(&str, &str, &str, &[&str])] = &[
    (
        "embassy",
        "Async executor",
        "Adds embassy-executor and its time driver. Lets you write `async fn` \
         against peripherals instead of polling. Costs some flash and a build \
         that pulls in more crates.",
        &[],
    ),
    (
        "wifi",
        "Wi-Fi and BLE",
        "Adds esp-wifi. By far the largest single thing you can enable — it \
         brings a blob-backed stack that dominates both flash and RAM. Leave it \
         off until you need a radio.",
        // The radio stack allocates, and its driver lives behind esp-hal's
        // unstable surface. `esp-generate` rejects the whole run without both.
        &["alloc", "unstable-hal"],
    ),
    (
        "alloc",
        "Heap allocator",
        "Adds esp-alloc and a global allocator, so `Vec` and `String` work. \
         Without it you are limited to fixed-size buffers. Many crates will not \
         compile no_std without this.",
        &[],
    ),
    (
        "defmt",
        "Deferred formatting logs",
        "Log strings stay in the ELF instead of being written to flash, so \
         logging costs a fraction of the space. The trade is that logs are only \
         readable through a decoder that has the matching ELF — reflash and \
         re-open the monitor together.",
        &[],
    ),
    (
        "probe-rs",
        "Configure for a debug probe",
        "Sets the cargo runner to probe-rs, which gives breakpoints and RTT. \
         Needs a probe; without one, leave this off and flash over USB serial.",
        &[],
    ),
    (
        "unstable-hal",
        "Unstable esp-hal APIs",
        "Turns on esp-hal's `unstable` feature. Some drivers — the radio among \
         them — are not reachable otherwise. The cost is that those APIs can \
         change in a patch release.",
        &[],
    ),
];

pub fn options() -> Vec<crate::model::WizardOption> {
    OPTIONS
        .iter()
        .map(|(id, label, detail, _)| crate::model::WizardOption {
            id: id.to_string(),
            label: label.to_string(),
            detail: detail.to_string(),
            // Already closed over, so the frontend ticks one list and is done.
            // Sending the direct requirements instead would mean walking the
            // graph again over there — a second implementation of this function
            // that can disagree with the one the generator is checked against.
            requires: requirements(id),
        })
        .collect()
}

/// What an option cannot work without, including its requirements' own.
///
/// Transitive, so a caller turning one switch on gets everything it needs in
/// one step rather than discovering a second missing option after fixing the
/// first.
pub fn requirements(id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = vec![id.to_string()];
    while let Some(current) = queue.pop() {
        let Some((_, _, _, requires)) = OPTIONS.iter().find(|(o, ..)| *o == current) else {
            continue;
        };
        for required in *requires {
            if !out.iter().any(|o| o == required) {
                out.push((*required).to_string());
                queue.push((*required).to_string());
            }
        }
    }
    out
}

/// Where a generated project ends up: the crate's name under `parent`, which
/// is what both generators do today.
///
/// One function rather than that assumption spelled at each caller, so the
/// day a generator starts sanitising a hyphen or a capital the change is here
/// and the UI opens the folder that was actually made.
pub fn destination(parent: &std::path::Path, choice: &WizardChoice) -> std::path::PathBuf {
    parent.join(&choice.name)
}

/// The command that creates the project.
pub fn plan(choice: &WizardChoice) -> Result<CommandPlan> {
    let chip = chip::by_id(&choice.chip).ok_or_else(|| Error::UnknownChip {
        chip: choice.chip.clone(),
    })?;

    if chip.target_for(choice.runtime).is_none() {
        return Err(Error::UnsupportedRuntime {
            chip: chip.name.clone(),
            runtime: choice.runtime.label().to_string(),
        });
    }

    // Refuse here rather than letting the generator do it. `esp-generate` says
    // "Invalid options provided" and exits, which the user meets *after*
    // choosing a folder — and the message names the missing option without
    // saying it can simply be turned on.
    for option in &choice.options {
        for required in requirements(option) {
            if !choice.options.contains(&required) {
                return Err(Error::MissingOption {
                    option: option.clone(),
                    required,
                });
            }
        }
    }

    // In the workspace layout the generator makes the *firmware* crate, under
    // the project directory named after the choice; the scaffold around it
    // is rusty's (`scaffold_workspace`). So the crate the generator is asked
    // for is always `firmware` there, whatever the project is called.
    let crate_name = match choice.layout {
        WizardLayout::Single => choice.name.clone(),
        WizardLayout::Workspace => {
            valid_name(&choice.name)?;
            "firmware".to_string()
        }
    };

    let (program, args, rationale) = match choice.runtime {
        Runtime::BareMetal => {
            let mut args = vec!["--headless".to_string(), "--chip".into(), chip.id.clone()];
            for option in &choice.options {
                args.push("-o".into());
                args.push(option.clone());
            }
            args.push(crate_name.clone());
            (
                "esp-generate",
                args,
                "esp-generate is the bare-metal template generator maintained by \
                 the esp-rs project.",
            )
        }
        Runtime::EspIdf => (
            "cargo",
            vec![
                "generate".to_string(),
                "esp-rs/esp-idf-template".into(),
                "cargo".into(),
                "--name".into(),
                crate_name.clone(),
            ],
            "std projects come from the esp-idf-template rather than \
             esp-generate, because they link the ESP-IDF C framework.",
        ),
    };

    let display = std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");

    Ok(CommandPlan {
        program: program.to_string(),
        args,
        display,
        rationale: rationale.to_string(),
        warning: None,
    })
}

/// What this set of choices commits the user to.
///
/// Ordered so the expensive, hard-to-reverse commitments come first: which
/// toolchain you must install is a bigger fact than whether you get a heap.
pub fn explain(choice: &WizardChoice) -> Vec<Explanation> {
    let Some(chip) = chip::by_id(&choice.chip) else {
        return vec![Explanation {
            topic: "Unknown chip".into(),
            detail: format!("`{}` is not in rusty's catalogue.", choice.chip),
            consequence: None,
        }];
    };

    let mut out = Vec::new();

    out.push(Explanation {
        topic: format!("{} is {}", chip.name, chip.arch.label()),
        detail: match chip.toolchain {
            ToolchainRequirement::EspXtensa => {
                "Upstream Rust cannot target Xtensa. You will need the `esp` \
                 toolchain, which ships a forked LLVM and takes a while to \
                 install."
                    .to_string()
            }
            ToolchainRequirement::Stock => {
                "Stock Rust supports this target. No forked toolchain, and \
                 anyone cloning the project can build it with plain rustup."
                    .to_string()
            }
        },
        consequence: chip
            .toolchain
            .install_command()
            .map(|c| format!("Run `{c}` once before the first build.")),
    });

    let target = chip.target_for(choice.runtime).unwrap_or("(unsupported)");
    out.push(Explanation {
        topic: format!("{} on {}", choice.runtime.label(), chip.name),
        detail: match choice.runtime {
            Runtime::BareMetal => "No operating system and no C framework. Fast builds, small \
                 binaries, and only the peripherals the HAL exposes — no \
                 threads, no filesystem, no sockets unless you add a stack."
                .to_string(),
            Runtime::EspIdf => "Links Espressif's C framework, so you get `std`: threads, \
                 sockets, a filesystem, and every ESP-IDF component. The first \
                 build downloads and compiles that framework, which takes \
                 minutes rather than seconds."
                .to_string(),
        },
        consequence: Some(format!("Builds for `{target}`.")),
    });

    if choice.layout == WizardLayout::Workspace {
        out.push(Explanation {
            topic: "Two crates, one workspace".into(),
            detail: format!(
                "`core` holds the logic that touches no hardware, and `cargo test` at the \
                 root runs it on this machine. `firmware` is the {} binary, excluded from \
                 the workspace so the root's tests never try to build it for the host; \
                 rusty builds, flashes and simulates it from its own directory.",
                chip.name
            ),
            consequence: Some(format!(
                "Creates `{name}/Cargo.toml`, `{name}/core/` and `{name}/firmware/`; the \
                 firmware depends on `{name}-core`.",
                name = choice.name
            )),
        });
    }

    for option in &choice.options {
        if let Some((_, label, detail, _)) = OPTIONS.iter().find(|(id, ..)| id == option) {
            out.push(Explanation {
                topic: (*label).to_string(),
                detail: (*detail).to_string(),
                consequence: None,
            });
        }
    }

    // Only worth saying when there is a real choice to be made.
    if choice.options.iter().any(|o| o == "defmt")
        && !choice.options.iter().any(|o| o == "probe-rs")
    {
        out.push(Explanation {
            topic: "defmt without a probe".into(),
            detail: "defmt works over USB serial too, but the monitor has to be \
                     told to decode it and pointed at the ELF."
                .into(),
            consequence: Some(
                "rusty passes `--log-format defmt --elf` for you when you monitor.".into(),
            ),
        });
    }

    if !chip.radios.iter().any(|r| r == "none" || r == "no radio")
        && !choice.options.iter().any(|o| o == "wifi")
    {
        out.push(Explanation {
            topic: "Radios stay off".into(),
            detail: format!(
                "{} has {}, but nothing is enabled. That keeps the binary small; \
                 adding the radio later is a feature flag, not a rewrite.",
                chip.name,
                chip.radios.join(", ")
            ),
            consequence: None,
        });
    }

    out
}

// ─── the workspace around a generated crate ──────────────────────────────────

/// The files that make a directory holding a generated `firmware/` crate the
/// standard embedded workspace: a root manifest that lists `core` and
/// excludes `firmware`, a `core` crate with one function and one test, a
/// README that says why the split exists, and the dependency line that lets
/// the firmware call into `core`.
///
/// Written after the generator has succeeded, into files the generator did
/// not make — every one is created rather than overwritten, so a generator
/// that one day writes a root manifest of its own is a refusal here and not
/// a silent replacement.
pub fn scaffold_workspace(root: &Path, choice: &WizardChoice) -> Result<()> {
    let name = valid_name(&choice.name)?;
    let core = format!("{name}-core");
    let chip = chip::by_id(&choice.chip)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| choice.chip.clone());

    write_new(&root.join("Cargo.toml"), &root_manifest(name, &chip))?;
    write_new(&root.join("core").join("Cargo.toml"), &core_manifest(&core))?;
    write_new(&root.join("core").join("src").join("lib.rs"), CORE_LIB)?;
    write_new(&root.join("README.md"), &readme(name, &core, &chip))?;
    write_new(&root.join("rustfmt.toml"), "edition = \"2024\"\n")?;
    write_new(&root.join(".gitignore"), "/target\n")?;
    add_dependency(&root.join("firmware").join("Cargo.toml"), &core)
}

/// A name cargo accepts for a package, since the workspace layout turns it
/// into `<name>-core`: letters, digits, `-` and `_`, starting with a letter
/// or digit. The generator checks its own argument; this is the one it never
/// sees.
fn valid_name(name: &str) -> Result<&str> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && name.starts_with(|c: char| c.is_ascii_alphanumeric());
    if ok {
        Ok(name)
    } else {
        Err(Error::Refused {
            detail: format!(
                "`{name}` is not a name cargo accepts for a crate — use letters, digits, `-` \
                 and `_`, starting with a letter or a digit."
            ),
        })
    }
}

fn write_new(path: &Path, text: &str) -> Result<()> {
    if path.exists() {
        return Err(Error::Exists {
            path: path.display().to_string(),
        });
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Write {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::write(path, text).map_err(|source| Error::Write {
        path: path.display().to_string(),
        source,
    })
}

/// `core = { path = "../core" }` under the firmware's `[dependencies]`,
/// inserted textually so everything the generator wrote — comments, order,
/// version specs — survives byte for byte, as `migrate.rs` treats a manifest.
fn add_dependency(manifest: &Path, core: &str) -> Result<()> {
    let text = std::fs::read_to_string(manifest).map_err(|source| Error::Read {
        path: manifest.display().to_string(),
        source,
    })?;
    let line = format!("{core} = {{ path = \"../core\" }}\n");
    if text.contains(&line) {
        return Ok(());
    }
    let patched = match text.find("[dependencies]\n") {
        Some(at) => {
            let after = at + "[dependencies]\n".len();
            format!("{}{line}{}", &text[..after], &text[after..])
        }
        None => format!("{}\n[dependencies]\n{line}", text.trim_end_matches('\n')),
    };
    std::fs::write(manifest, patched).map_err(|source| Error::Write {
        path: manifest.display().to_string(),
        source,
    })
}

fn root_manifest(name: &str, chip: &str) -> String {
    format!(
        "# {name}: a workspace of two crates, and the split is the whole point.\n\
         #\n\
         # `core` is the part that touches no hardware, so `cargo test` at this root\n\
         # runs it on this machine. `firmware` is the {chip} binary and holds every\n\
         # line that reads a pin; a `use esp_hal` in `core` is a build break, which\n\
         # is what keeps the logic testable at a desk.\n\
         #\n\
         # `firmware` is deliberately NOT a workspace member: it needs its own\n\
         # toolchain and target, and `cargo test` at the root would try to build it\n\
         # for the host. Build it from its own directory — rusty does.\n\
         \n\
         [workspace]\n\
         members  = [\"core\"]\n\
         exclude  = [\"firmware\"]\n\
         resolver = \"3\"\n\
         \n\
         [workspace.package]\n\
         version = \"0.1.0\"\n\
         edition = \"2024\"\n"
    )
}

fn core_manifest(core: &str) -> String {
    format!(
        "[package]\n\
         name = \"{core}\"\n\
         version.workspace = true\n\
         edition.workspace = true\n\
         \n\
         [dependencies]\n"
    )
}

const CORE_LIB: &str = "\
//! The part of the firmware that touches no hardware.
//!
//! Everything here builds and tests on the host: `cargo test` at the
//! workspace root runs it on this machine, with the standard library
//! available to the tests and nothing else. Keep it that way — a `use
//! esp_hal` here is a build break on purpose, because the moment the maths
//! needs a pin it stops being testable at a desk.

#![cfg_attr(not(test), no_std)]

/// Hold a value inside the range an actuator accepts.
///
/// The smallest useful thing a control loop needs and the hardware does not
/// provide — and the first thing worth a test, because a motor asked for
/// 120% does something the desk cannot show.
pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_outside_the_range_is_held_at_its_edge() {
        assert_eq!(clamp(1.5, 0.0, 1.0), 1.0);
        assert_eq!(clamp(-0.2, 0.0, 1.0), 0.0);
        assert_eq!(clamp(0.4, 0.0, 1.0), 0.4);
    }
}
";

fn readme(name: &str, core: &str, chip: &str) -> String {
    let ident = core.replace('-', "_");
    format!(
        "# {name}\n\
         \n\
         Two crates, and the split is the whole point.\n\
         \n\
         - `core/` — `{core}`: the logic that touches no hardware. `cargo test` at\n  \
           this root runs it on this machine.\n\
         - `firmware/` — the {chip} binary. Every line that reads a pin lives here,\n  \
           and it is *excluded* from the workspace on purpose: it needs its own\n  \
           toolchain and target, and `cargo test` at the root would otherwise try\n  \
           to build it for the host.\n\
         \n\
         ```bash\n\
         cargo test                              # the half that runs here\n\
         cd firmware && cargo build --release    # the chip's half\n\
         ```\n\
         \n\
         `firmware` depends on `{core}` (`{ident}` in Rust), so what the tests\n\
         prove is what the board runs.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(chip: &str, runtime: Runtime, options: &[&str]) -> WizardChoice {
        WizardChoice {
            chip: chip.to_string(),
            runtime,
            name: "blinky".to_string(),
            options: options.iter().map(|o| o.to_string()).collect(),
            layout: WizardLayout::Single,
        }
    }

    fn workspace(chip: &str) -> WizardChoice {
        WizardChoice {
            layout: WizardLayout::Workspace,
            ..choice(chip, Runtime::BareMetal, &[])
        }
    }

    /// The generator makes the firmware crate under the project directory,
    /// so it is asked for `firmware`, whatever the project is called.
    #[test]
    fn a_workspace_asks_the_generator_for_the_firmware_crate() {
        let workspace_plan = plan(&workspace("esp32c3")).unwrap();
        assert_eq!(workspace_plan.args.last().unwrap(), "firmware");
        assert_eq!(
            plan(&choice("esp32c3", Runtime::BareMetal, &[]))
                .unwrap()
                .args
                .last()
                .unwrap(),
            "blinky",
            "one crate keeps the project's own name"
        );
        let std = plan(&WizardChoice {
            runtime: Runtime::EspIdf,
            ..workspace("esp32c3")
        })
        .unwrap();
        assert!(std.args.windows(2).any(|w| w == ["--name", "firmware"]));
    }

    #[test]
    fn the_workspace_scaffold_lists_core_and_excludes_firmware() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("firmware/src")).unwrap();
        std::fs::write(
            root.join("firmware/Cargo.toml"),
            "[package]\nname = \"firmware\"\nedition = \"2024\"\n\n[dependencies]\nesp-hal = \"1\"\n",
        )
        .unwrap();

        scaffold_workspace(root, &workspace("esp32c3")).unwrap();

        let manifest: toml::Table =
            toml::from_str(&std::fs::read_to_string(root.join("Cargo.toml")).unwrap()).unwrap();
        let workspace_table = manifest["workspace"].as_table().unwrap();
        assert_eq!(
            workspace_table["members"].as_array().unwrap()[0].as_str(),
            Some("core")
        );
        assert_eq!(
            workspace_table["exclude"].as_array().unwrap()[0].as_str(),
            Some("firmware")
        );
        assert!(root.join("core/src/lib.rs").is_file());
        let core: toml::Table =
            toml::from_str(&std::fs::read_to_string(root.join("core/Cargo.toml")).unwrap())
                .unwrap();
        assert_eq!(core["package"]["name"].as_str(), Some("blinky-core"));

        // The dependency lands under the generator's own `[dependencies]`,
        // and everything else in the manifest is untouched.
        let firmware = std::fs::read_to_string(root.join("firmware/Cargo.toml")).unwrap();
        assert!(
            firmware.contains(
                "[dependencies]\nblinky-core = { path = \"../core\" }\nesp-hal = \"1\"\n"
            ),
            "{firmware}"
        );
        assert!(firmware.starts_with("[package]\nname = \"firmware\""));
        assert!(root.join("README.md").is_file());
        assert!(root.join(".gitignore").is_file());

        // Twice is a refusal, not a rewrite.
        assert!(matches!(
            scaffold_workspace(root, &workspace("esp32c3")),
            Err(Error::Exists { .. })
        ));
    }

    #[test]
    fn a_name_cargo_would_refuse_is_refused_before_anything_is_generated() {
        for bad in ["", "my project", "-lead", "驱动"] {
            let refused = plan(&WizardChoice {
                name: bad.to_string(),
                ..workspace("esp32c3")
            });
            assert!(
                matches!(refused, Err(Error::Refused { .. })),
                "{bad:?} should be refused"
            );
        }
        assert!(valid_name("cf-drone_rs2").is_ok());
    }

    #[test]
    fn the_workspace_layout_is_explained_with_what_it_creates() {
        let explanations = explain(&workspace("esp32c3"));
        let note = explanations
            .iter()
            .find(|e| e.topic.contains("workspace"))
            .expect("the layout is a commitment worth explaining");
        assert!(note.detail.contains("ESP32-C3"));
        assert!(note.consequence.as_deref().unwrap().contains("blinky-core"));
        assert!(
            !explain(&choice("esp32c3", Runtime::BareMetal, &[]))
                .iter()
                .any(|e| e.topic.contains("workspace")),
            "one crate has nothing to say about a workspace"
        );
    }

    #[test]
    fn bare_metal_uses_esp_generate_with_the_chip_and_options() {
        let plan = plan(&choice(
            "esp32c3",
            Runtime::BareMetal,
            &["embassy", "alloc"],
        ))
        .unwrap();

        assert_eq!(plan.program, "esp-generate");
        assert!(plan.args.contains(&"--chip".to_string()));
        assert!(plan.args.contains(&"esp32c3".to_string()));
        // Options are repeated `-o` flags, not a comma list.
        assert_eq!(plan.args.iter().filter(|a| *a == "-o").count(), 2);
        assert_eq!(plan.args.last().unwrap(), "blinky");
    }

    #[test]
    fn std_projects_come_from_a_different_generator() {
        let plan = plan(&choice("esp32c3", Runtime::EspIdf, &[])).unwrap();
        assert_eq!(plan.program, "cargo");
        assert!(plan.args.iter().any(|a| a.contains("esp-idf-template")));
        assert!(plan.rationale.contains("ESP-IDF"));
    }

    /// The P4 has no std target. Generating a project that cannot build is
    /// worse than refusing, because the failure surfaces minutes later as a
    /// linker error.
    #[test]
    fn an_impossible_combination_is_refused_up_front() {
        let err = plan(&choice("esp32p4", Runtime::EspIdf, &[]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("ESP32-P4"), "{err}");
    }

    #[test]
    fn xtensa_parts_lead_with_the_toolchain_they_demand() {
        let explanations = explain(&choice("esp32s3", Runtime::BareMetal, &[]));
        let first = &explanations[0];

        assert!(first.topic.contains("Xtensa"));
        assert!(first.detail.contains("forked LLVM"));
        assert_eq!(
            first.consequence.as_deref(),
            Some("Run `espup install` once before the first build.")
        );
    }

    #[test]
    fn riscv_parts_say_no_extra_toolchain_is_needed() {
        let explanations = explain(&choice("esp32c3", Runtime::BareMetal, &[]));
        assert!(explanations[0].detail.contains("Stock Rust"));
        assert!(explanations[0].consequence.is_none());
    }

    #[test]
    fn the_runtime_choice_names_the_target_it_implies() {
        let bare = explain(&choice("esp32c3", Runtime::BareMetal, &[]));
        assert!(
            bare[1]
                .consequence
                .as_deref()
                .unwrap()
                .contains("riscv32imc-unknown-none-elf")
        );

        let std = explain(&choice("esp32c3", Runtime::EspIdf, &[]));
        assert!(std[1].consequence.as_deref().unwrap().contains("espidf"));
        assert!(
            std[1].detail.contains("minutes"),
            "the build cost is the surprise"
        );
    }

    #[test]
    fn selected_options_are_explained_and_unselected_ones_are_not() {
        let explanations = explain(&choice("esp32c3", Runtime::BareMetal, &["wifi"]));
        let topics: Vec<&str> = explanations.iter().map(|e| e.topic.as_str()).collect();

        assert!(topics.contains(&"Wi-Fi and BLE"));
        assert!(!topics.contains(&"Heap allocator"));
        // With a radio enabled, the "radios stay off" note must not appear.
        assert!(!topics.iter().any(|t| t.contains("stay off")));
    }

    #[test]
    fn defmt_without_a_probe_gets_the_extra_warning() {
        let with_probe = explain(&choice(
            "esp32c3",
            Runtime::BareMetal,
            &["defmt", "probe-rs"],
        ));
        assert!(
            !with_probe
                .iter()
                .any(|e| e.topic.contains("without a probe"))
        );

        let serial_only = explain(&choice("esp32c3", Runtime::BareMetal, &["defmt"]));
        let note = serial_only
            .iter()
            .find(|e| e.topic.contains("without a probe"))
            .expect("serial defmt needs the decoder note");
        assert!(
            note.consequence
                .as_deref()
                .unwrap()
                .contains("--log-format defmt")
        );
    }

    #[test]
    fn every_generator_option_carries_its_cost() {
        for (id, label, detail, _) in OPTIONS {
            assert!(!label.is_empty(), "{id} has no label");
            assert!(
                detail.len() > 60,
                "{id}: the point of this table is the explanation"
            );
        }
    }

    /// The combination that sent a user round the houses: `esp-generate` exits
    /// with "Invalid options provided", and it did so only after they had
    /// already chosen where the project should go.
    #[test]
    fn wifi_cannot_be_chosen_alone() {
        let choice = WizardChoice {
            chip: "esp32".into(),
            runtime: Runtime::BareMetal,
            name: "firmware".into(),
            options: vec!["wifi".into()],
            layout: WizardLayout::Single,
        };

        let error = plan(&choice).unwrap_err().to_string();
        assert!(error.contains("wifi"), "{error}");
        assert!(
            error.contains("alloc") || error.contains("unstable-hal"),
            "the refusal has to name what is missing: {error}",
        );
    }

    #[test]
    fn wifi_with_everything_it_needs_is_planned() {
        let choice = WizardChoice {
            chip: "esp32".into(),
            runtime: Runtime::BareMetal,
            name: "firmware".into(),
            options: vec!["wifi".into(), "alloc".into(), "unstable-hal".into()],
            layout: WizardLayout::Single,
        };

        let plan = plan(&choice).expect("a valid combination must plan");
        assert!(plan.display.contains("-o wifi"), "{}", plan.display);
    }

    /// The frontend ticks `requires` and nothing else, so it has to be the
    /// whole set — not just the first level.
    #[test]
    fn the_advertised_requirements_are_already_closed_over() {
        let options = options();
        for option in &options {
            for required in &option.requires {
                let deeper = options
                    .iter()
                    .find(|o| &o.id == required)
                    .map(|o| o.requires.clone())
                    .unwrap_or_default();
                for transitive in deeper {
                    assert!(
                        option.requires.contains(&transitive),
                        "{} advertises {required} but not its own requirement {transitive}",
                        option.id,
                    );
                }
            }
        }
    }
}
