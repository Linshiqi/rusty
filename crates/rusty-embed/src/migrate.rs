//! Moving a project from one chip to another.
//!
//! The question this answers is "must I start again?", and the honest reply
//! is no for the configuration and yes for anything that names a pin. Three
//! files decide which chip a project builds for — the toolchain channel, the
//! target triple and build-std in `.cargo/config.toml`, and the chip feature
//! on every `esp-*` dependency — and all three are mechanical. What a
//! `GPIO26` should become when the part has no GPIO26 is not.
//!
//! So this changes exactly the mechanical part, says so, and leaves the rest
//! to the compiler, which will name every site precisely. Inventing pin
//! substitutions is the failure this workbench exists to avoid.
//!
//! **Edits are textual and surgical, never a parse-and-reserialise.** A
//! manifest rewriter that eats a comment or reorders a table loses somebody's
//! work; `scaffold.rs` refuses to touch `Cargo.toml` for the same reason.
//! Here the substitutions are word-bounded, so `esp32` in `features` and in
//! `--chip esp32` both move while `xtensa-esp32-none-elf` and `esp32c3` are
//! left alone.

use std::path::Path;

use crate::model::{Chip, Edit, FileChange, Migration, ToolchainRequirement};

/// What switching this project to `to` would change, and what it would not.
///
/// Reads; writes nothing. [`apply`] takes the result back, so what runs is
/// exactly what was shown.
pub fn plan(root: &Path, from: &Chip, to: &Chip) -> Migration {
    let mut migration = Migration {
        from: from.id.clone(),
        to: to.id.clone(),
        files: Vec::new(),
        notes: Vec::new(),
        blocker: None,
    };

    if from.id == to.id {
        migration.blocker = Some(format!("This project already builds for {}.", to.name));
        return migration;
    }

    // `.cargo/config.toml` is the one file that must exist and must name the
    // old triple. Without it there is nothing to be sure of, and a guess here
    // produces a project that builds for the wrong part in silence.
    let config_path = root.join(".cargo/config.toml");
    let Ok(config) = std::fs::read_to_string(&config_path) else {
        migration.blocker = Some(
            "This project has no .cargo/config.toml, so nothing states which target it \
             builds for. rusty will not write one from a guess — create the project for \
             the chip you want instead."
                .to_string(),
        );
        return migration;
    };
    if !config.contains(&from.bare_metal_target) {
        migration.blocker = Some(format!(
            ".cargo/config.toml does not mention {}, so rusty cannot tell which text to \
             change. Check the target it sets and switch chips by hand, or create a new \
             project for {}.",
            from.bare_metal_target, to.name,
        ));
        return migration;
    }

    let mut config_edits = vec![Edit {
        before: from.bare_metal_target.clone(),
        after: to.bare_metal_target.clone(),
    }];
    if word_appears(&config, &from.id) {
        // The runner line: `espflash flash --monitor --chip esp32`.
        config_edits.push(Edit {
            before: from.id.clone(),
            after: to.id.clone(),
        });
    }
    // build-std is not a preference. Xtensa has no precompiled `core` at all,
    // and on a stock toolchain the same key is an unstable flag cargo refuses
    // outright — so a project carrying it into a RISC-V switch fails to build
    // with an error about nightly, nowhere near the chip it is really about.
    match (
        from.toolchain == ToolchainRequirement::EspXtensa,
        to.toolchain == ToolchainRequirement::EspXtensa,
    ) {
        (true, false) if let Some(line) = build_std_line(&config) => {
            config_edits.push(Edit {
                before: line,
                after: String::new(),
            });
        }
        (false, true) if build_std_line(&config).is_none() => {
            config_edits.push(Edit {
                before: String::new(),
                after: "\n[unstable]\nbuild-std = [\"core\"]\n".to_string(),
            });
        }
        _ => {}
    }
    migration.files.push(FileChange {
        path: ".cargo/config.toml".to_string(),
        edits: config_edits,
    });

    // The toolchain channel. Only Xtensa forces one; going the other way, a
    // project pinned to `esp` keeps building with Espressif's fork for no
    // reason, which is a slow surprise rather than an error.
    let toolchain_path = root.join("rust-toolchain.toml");
    if let Ok(text) = std::fs::read_to_string(&toolchain_path) {
        match channel_of(&text) {
            Some(channel) => {
                let wanted = if to.toolchain == ToolchainRequirement::EspXtensa {
                    "esp"
                } else if channel == "esp" {
                    "stable"
                } else {
                    // Already a stock channel — nightly, or a pinned version.
                    // Whatever they chose still works for RISC-V.
                    &channel
                };
                if wanted != channel {
                    migration.files.push(FileChange {
                        path: "rust-toolchain.toml".to_string(),
                        edits: vec![Edit {
                            before: format!("channel = \"{channel}\""),
                            after: format!("channel = \"{wanted}\""),
                        }],
                    });
                }
            }
            None => migration.notes.push(
                "rust-toolchain.toml sets no channel that rusty recognises — check it by \
                 hand if the build cannot find a compiler for the new target."
                    .to_string(),
            ),
        }
    } else if to.toolchain == ToolchainRequirement::EspXtensa {
        migration.notes.push(format!(
            "{} needs Espressif's forked toolchain and this project pins none. Add \
             rust-toolchain.toml with channel = \"esp\", or `espup install` and select it.",
            to.name,
        ));
    }

    // The chip feature on every esp-* dependency. Word-bounded, so `esp32`
    // moves and `esp32c3` — already the new id, or another crate's name —
    // does not.
    let manifest_path = root.join("Cargo.toml");
    match std::fs::read_to_string(&manifest_path) {
        Ok(manifest) if word_appears(&manifest, &from.id) => {
            migration.files.push(FileChange {
                path: "Cargo.toml".to_string(),
                edits: vec![Edit {
                    before: from.id.clone(),
                    after: to.id.clone(),
                }],
            });
        }
        Ok(_) => migration.notes.push(format!(
            "Cargo.toml never names {}, so no dependency feature was switched. If a crate \
             selects the chip some other way, that choice is still on the old part.",
            from.id,
        )),
        Err(_) => {}
    }

    // rusty's own file, so there is no contract to protect here beyond the
    // user's layout — only the chip it says it is.
    let sim_path = root.join(".rusty/sim.toml");
    if let Ok(sim) = std::fs::read_to_string(&sim_path)
        && word_appears(&sim, &from.id)
    {
        migration.files.push(FileChange {
            path: ".rusty/sim.toml".to_string(),
            edits: vec![Edit {
                before: from.id.clone(),
                after: to.id.clone(),
            }],
        });
    }

    migration.notes.push(format!(
        "Pins and peripherals in your source are not touched. {} and {} do not have the \
         same GPIOs, and only your code knows what each one should become — build after \
         switching and the compiler names every site.",
        from.name, to.name,
    ));
    if from.arch != to.arch {
        migration.notes.push(format!(
            "This also changes architecture, {} to {}: anything written in assembly, and \
             any interrupt or critical-section code that assumes one of them, needs \
             reading.",
            from.arch.label(),
            to.arch.label(),
        ));
    }
    migration.notes.push(
        "rustflags in .cargo/config.toml are left as they are — they are the project's \
         choice, not the chip's."
            .to_string(),
    );

    migration
}

/// Carry out a plan, returning the files written.
///
/// Every edit must still match: the file is re-read here, and anything that
/// changed since the plan was made stops the whole run rather than leaving a
/// project half switched between two chips.
pub fn apply(root: &Path, migration: &Migration) -> Result<Vec<String>, String> {
    if let Some(blocker) = &migration.blocker {
        return Err(blocker.clone());
    }

    // Check every file before writing any of them.
    let mut staged = Vec::new();
    for file in &migration.files {
        let path = root.join(&file.path);
        let mut text = std::fs::read_to_string(&path)
            .map_err(|e| format!("could not read {}: {e}", file.path))?;
        for edit in &file.edits {
            if edit.before.is_empty() {
                text.push_str(&edit.after);
                continue;
            }
            if !text.contains(&edit.before) {
                return Err(format!(
                    "{} no longer contains `{}` — it changed since this was planned. \
                     Nothing has been written; look at the file and try again.",
                    file.path,
                    edit.before.trim(),
                ));
            }
            text = replace_word(&text, &edit.before, &edit.after);
        }
        staged.push((path, text, file.path.clone()));
    }

    let mut written = Vec::new();
    for (path, text, name) in staged {
        std::fs::write(&path, text).map_err(|e| format!("could not write {name}: {e}"))?;
        written.push(name);
    }
    Ok(written)
}

/// Whether `word` appears not embedded in a longer identifier.
///
/// `esp32` is a prefix of `esp32c3` and a substring of `xtensa-esp32-none-elf`;
/// without this, switching chips renames the triple's middle and produces
/// `xtensa-esp32c3-none-elf`, which is not a target that exists.
fn word_appears(text: &str, word: &str) -> bool {
    boundaries(text, word).next().is_some()
}

fn replace_word(text: &str, from: &str, to: &str) -> String {
    // A multi-line edit (removing the build-std block) is not an identifier
    // and has no boundaries to respect.
    if from.contains('\n') {
        return text.replace(from, to);
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for at in boundaries(text, from).collect::<Vec<_>>() {
        out.push_str(&text[last..at]);
        out.push_str(to);
        last = at + from.len();
    }
    out.push_str(&text[last..]);
    out
}

fn boundaries<'a>(text: &'a str, word: &'a str) -> impl Iterator<Item = usize> + 'a {
    text.match_indices(word).filter_map(move |(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        let joined = |c: Option<char>| {
            c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        };
        (!joined(before) && !joined(after)).then_some(at)
    })
}

/// The text to cut to be rid of `build-std` — the line, and the `[unstable]`
/// header with it when nothing else lives under it.
///
/// A bare `[unstable]` left behind is valid TOML and harmless to cargo, which
/// is exactly why it would survive: nothing complains, and the file quietly
/// accumulates the debris of every switch.
fn build_std_line(config: &str) -> Option<String> {
    let lines: Vec<&str> = config.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.trim_start().starts_with("build-std"))?;

    let header = lines[..at]
        .iter()
        .rposition(|line| line.trim_start().starts_with('['));
    let alone = header.is_some_and(|header| {
        lines[header] .trim() == "[unstable]"
            && lines[header + 1..at]
                .iter()
                .chain(lines[at + 1..].iter().take_while(|line| {
                    !line.trim_start().starts_with('[')
                }))
                .all(|line| line.trim().is_empty())
    });

    let from = if alone { header.expect("checked") } else { at };
    Some(format!("{}\n", lines[from..=at].join("\n")))
}

fn channel_of(toolchain: &str) -> Option<String> {
    toolchain.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "channel").then(|| value.trim().trim_matches('"').to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Arch, Vendor};

    fn chip(id: &str, target: &str, xtensa: bool) -> Chip {
        Chip {
            id: id.to_string(),
            name: id.to_uppercase(),
            vendor: Vendor::Espressif,
            arch: if xtensa { Arch::Xtensa } else { Arch::RiscV },
            cores: 1,
            sram_bytes: 0,
            flash_bytes: None,
            bare_metal_target: target.to_string(),
            std_target: None,
            toolchain: if xtensa {
                ToolchainRequirement::EspXtensa
            } else {
                ToolchainRequirement::Stock
            },
            flashers: Vec::new(),
            probe_rs_target: None,
            radios: Vec::new(),
        }
    }

    fn esp32() -> Chip {
        chip("esp32", "xtensa-esp32-none-elf", true)
    }

    fn esp32c3() -> Chip {
        chip("esp32c3", "riscv32imc-unknown-none-elf", false)
    }

    fn project(dir: &Path) {
        std::fs::create_dir_all(dir.join(".cargo")).unwrap();
        std::fs::write(
            dir.join(".cargo/config.toml"),
            "[target.xtensa-esp32-none-elf]\n\
             runner = \"espflash flash --monitor --chip esp32\"\n\
             \n\
             [build]\n\
             rustflags = [\n  \"-C\", \"link-arg=-nostartfiles\",\n]\n\
             \n\
             target = \"xtensa-esp32-none-elf\"\n\
             \n\
             [unstable]\n\
             build-std = [\"core\"]\n",
        )
        .unwrap();
        std::fs::write(dir.join("rust-toolchain.toml"), "[toolchain]\nchannel = \"esp\"\n")
            .unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[dependencies]\n\
             # the chip feature picks the part\n\
             esp-hal = { version = \"~1.1.0\", features = [\"esp32\", \"unstable\"] }\n\
             esp-println = { version = \"0.15\", features = [\"esp32\"] }\n",
        )
        .unwrap();
    }

    /// The whole point: an Xtensa project becomes a RISC-V one without being
    /// recreated, and every file that decides the chip is covered.
    #[test]
    fn switching_xtensa_to_riscv_rewrites_target_toolchain_and_features() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path());

        let migration = plan(dir.path(), &esp32(), &esp32c3());
        assert!(migration.blocker.is_none(), "{:?}", migration.blocker);
        apply(dir.path(), &migration).expect("applied");

        let config = std::fs::read_to_string(dir.path().join(".cargo/config.toml")).unwrap();
        assert!(config.contains("[target.riscv32imc-unknown-none-elf]"), "{config}");
        assert!(config.contains("target = \"riscv32imc-unknown-none-elf\""), "{config}");
        assert!(config.contains("--chip esp32c3"), "{config}");
        assert!(
            !config.contains("build-std"),
            "a stock toolchain refuses build-std outright: {config}",
        );
        assert!(
            !config.contains("[unstable]"),
            "and the section it lived in goes with it rather than sitting empty: {config}",
        );
        assert!(
            config.contains("link-arg=-nostartfiles"),
            "the project's own rustflags survive: {config}",
        );

        let toolchain = std::fs::read_to_string(dir.path().join("rust-toolchain.toml")).unwrap();
        assert!(toolchain.contains("channel = \"stable\""), "{toolchain}");

        let manifest = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(manifest.contains("features = [\"esp32c3\", \"unstable\"]"), "{manifest}");
        assert!(
            manifest.contains("# the chip feature picks the part"),
            "comments survive, because this edits text rather than reserialising: {manifest}",
        );
        assert!(
            manifest.contains("version = \"~1.1.0\""),
            "and so do version specs: {manifest}",
        );
    }

    /// The trap that makes a naive find-and-replace produce a target triple
    /// that does not exist.
    #[test]
    fn the_chip_id_inside_the_triple_is_not_a_separate_occurrence() {
        assert_eq!(
            replace_word("xtensa-esp32-none-elf", "esp32", "esp32c3"),
            "xtensa-esp32-none-elf",
            "the id is embedded in the triple; the triple is replaced whole or not at all",
        );
        assert_eq!(replace_word("--chip esp32\"", "esp32", "esp32c3"), "--chip esp32c3\"");
        assert_eq!(
            replace_word("features = [\"esp32\"]", "esp32", "esp32c3"),
            "features = [\"esp32c3\"]",
        );
        assert_eq!(
            replace_word("esp32c3 stays", "esp32", "esp32s3"),
            "esp32c3 stays",
            "a longer id starting with the old one is a different chip, not a match",
        );
    }

    /// Going the other way has to *add* what it removed, or an Xtensa build
    /// fails looking for a precompiled core that has never existed.
    #[test]
    fn switching_to_xtensa_restores_build_std_and_the_esp_channel() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".cargo")).unwrap();
        std::fs::write(
            dir.path().join(".cargo/config.toml"),
            "[build]\ntarget = \"riscv32imc-unknown-none-elf\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"stable\"\n",
        )
        .unwrap();

        let migration = plan(dir.path(), &esp32c3(), &esp32());
        apply(dir.path(), &migration).expect("applied");

        let config = std::fs::read_to_string(dir.path().join(".cargo/config.toml")).unwrap();
        assert!(config.contains("target = \"xtensa-esp32-none-elf\""), "{config}");
        assert!(config.contains("build-std = [\"core\"]"), "{config}");
        let toolchain = std::fs::read_to_string(dir.path().join("rust-toolchain.toml")).unwrap();
        assert!(toolchain.contains("channel = \"esp\""), "{toolchain}");
    }

    /// Refuse rather than guess: with nothing stating the current target,
    /// writing one would produce a project that builds for the wrong part in
    /// silence.
    #[test]
    fn a_project_that_names_no_target_is_refused_rather_than_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let migration = plan(dir.path(), &esp32(), &esp32c3());
        assert!(
            migration.blocker.is_some_and(|b| b.contains("config.toml")),
            "the refusal names the missing file",
        );

        std::fs::create_dir_all(dir.path().join(".cargo")).unwrap();
        std::fs::write(
            dir.path().join(".cargo/config.toml"),
            "[build]\ntarget = \"thumbv7em-none-eabihf\"\n",
        )
        .unwrap();
        let migration = plan(dir.path(), &esp32(), &esp32c3());
        assert!(
            migration
                .blocker
                .is_some_and(|b| b.contains("xtensa-esp32-none-elf")),
            "and so does the one for a target it did not expect",
        );
    }

    /// The notes are the honest half — a switch that silently left `GPIO26`
    /// in place would be the plausible-answer failure this workbench is
    /// written against.
    #[test]
    fn the_plan_says_what_it_does_not_do() {
        let dir = tempfile::tempdir().unwrap();
        project(dir.path());
        let migration = plan(dir.path(), &esp32(), &esp32c3());

        assert!(
            migration.notes.iter().any(|n| n.contains("GPIO")),
            "pins are named as out of scope: {:?}",
            migration.notes,
        );
        assert!(
            migration
                .notes
                .iter()
                .any(|n| n.contains("Xtensa") && n.contains("RISC-V")),
            "and so is the architecture change: {:?}",
            migration.notes,
        );
    }
}
