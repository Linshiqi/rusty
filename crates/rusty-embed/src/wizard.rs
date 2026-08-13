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

use crate::{
    chip,
    error::{Error, Result},
    model::{CommandPlan, Explanation, Runtime, ToolchainRequirement, WizardChoice},
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

/// Where a generated project ended up.
///
/// Returned rather than inferred by the caller: the generator decides the
/// directory name, and a UI that recomputed it from the crate name would open
/// the wrong folder the first time a generator sanitises a hyphen.
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

    let (program, args, rationale) = match choice.runtime {
        Runtime::BareMetal => {
            let mut args = vec![
                "--headless".to_string(),
                "--chip".into(),
                chip.id.clone(),
            ];
            for option in &choice.options {
                args.push("-o".into());
                args.push(option.clone());
            }
            args.push(choice.name.clone());
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
                choice.name.clone(),
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
            ToolchainRequirement::EspXtensa =>
                "Upstream Rust cannot target Xtensa. You will need the `esp` \
                 toolchain, which ships a forked LLVM and takes a while to \
                 install."
                    .to_string(),
            ToolchainRequirement::Stock =>
                "Stock Rust supports this target. No forked toolchain, and \
                 anyone cloning the project can build it with plain rustup."
                    .to_string(),
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
            Runtime::BareMetal =>
                "No operating system and no C framework. Fast builds, small \
                 binaries, and only the peripherals the HAL exposes — no \
                 threads, no filesystem, no sockets unless you add a stack."
                    .to_string(),
            Runtime::EspIdf =>
                "Links Espressif's C framework, so you get `std`: threads, \
                 sockets, a filesystem, and every ESP-IDF component. The first \
                 build downloads and compiles that framework, which takes \
                 minutes rather than seconds."
                    .to_string(),
        },
        consequence: Some(format!("Builds for `{target}`.")),
    });

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

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(chip: &str, runtime: Runtime, options: &[&str]) -> WizardChoice {
        WizardChoice {
            chip: chip.to_string(),
            runtime,
            name: "blinky".to_string(),
            options: options.iter().map(|o| o.to_string()).collect(),
        }
    }

    #[test]
    fn bare_metal_uses_esp_generate_with_the_chip_and_options() {
        let plan = plan(&choice("esp32c3", Runtime::BareMetal, &["embassy", "alloc"])).unwrap();

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
        assert!(std[1].detail.contains("minutes"), "the build cost is the surprise");
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
        let with_probe = explain(&choice("esp32c3", Runtime::BareMetal, &["defmt", "probe-rs"]));
        assert!(!with_probe.iter().any(|e| e.topic.contains("without a probe")));

        let serial_only = explain(&choice("esp32c3", Runtime::BareMetal, &["defmt"]));
        let note = serial_only
            .iter()
            .find(|e| e.topic.contains("without a probe"))
            .expect("serial defmt needs the decoder note");
        assert!(note.consequence.as_deref().unwrap().contains("--log-format defmt"));
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
