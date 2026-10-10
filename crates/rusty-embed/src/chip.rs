//! Chip lookups against the built-in catalogue.
//!
//! A thin facade over [`crate::catalog`], which is where the data actually
//! lives now. These functions see only what ships in the binary — they are
//! pure, deterministic, and what the tests exercise. Code that must respect a
//! user's or a project's overrides takes a [`Catalog`](crate::catalog::Catalog)
//! explicitly instead.

use crate::{catalog::Catalog, model::Chip};

/// The parsed built-ins — the one copy `Catalog` keeps for the whole process.
/// This module used to hold a second cache of its own, so the same two TOML
/// files were parsed twice and kept twice.
fn builtin() -> &'static Catalog {
    Catalog::builtin_shared()
}

/// Every part that ships with rusty.
pub fn catalogue() -> Vec<Chip> {
    builtin().chips().to_vec()
}

/// Look a part up by its canonical id.
pub fn by_id(id: &str) -> Option<Chip> {
    builtin().chip(id).cloned()
}

/// Accept the spellings that appear in the wild — `ESP32-C3`, `esp32_c3`,
/// `esp32c3` — and normalize to the canonical id.
pub use crate::catalog::normalize;

/// Which parts a target triple could belong to.
///
/// Ambiguous by nature: `riscv32imc-unknown-none-elf` serves both the C2 and
/// the C3, and `thumbv7em-none-eabihf` serves most of the Cortex-M4F world. A
/// caller that needs one answer must disambiguate from the manifest.
pub fn chips_for_target(target: &str) -> Vec<Chip> {
    builtin()
        .chips()
        .iter()
        .filter(|c| c.bare_metal_target == target || c.std_target.as_deref() == Some(target))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Arch, Flasher, Runtime};

    #[test]
    fn the_builtin_catalogue_parses_without_problems() {
        // The data file is compiled in, so a typo in it is a build-time
        // mistake — but one that would otherwise only surface as a part
        // mysteriously missing from the list.
        let catalog = Catalog::builtin();
        assert!(
            catalog.problems().is_empty(),
            "built-in catalogue has problems: {:?}",
            catalog.problems()
        );
        assert!(catalog.chips().len() >= 8);
        assert!(!catalog.boards().is_empty());
    }

    #[test]
    fn architecture_and_target_triple_agree() {
        for c in catalogue() {
            let prefix_matches = match c.arch {
                Arch::Xtensa => c.bare_metal_target.starts_with("xtensa-"),
                Arch::RiscV => c.bare_metal_target.starts_with("riscv32"),
                Arch::CortexM => c.bare_metal_target.starts_with("thumb"),
            };
            assert!(
                prefix_matches,
                "{}: arch {:?} and target `{}` disagree",
                c.id, c.arch, c.bare_metal_target
            );
        }
    }

    /// The forked toolchain is an Xtensa fact, not an Espressif fact — the
    /// RISC-V Espressif parts build on stock Rust, and saying otherwise would
    /// send half of ESP32 users to install espup for nothing.
    #[test]
    fn only_xtensa_parts_need_the_forked_toolchain() {
        for c in catalogue() {
            assert_eq!(
                c.needs_esp_toolchain(),
                c.arch == Arch::Xtensa,
                "{} classified wrong",
                c.id
            );
        }
    }

    #[test]
    fn ids_are_canonical_and_unique() {
        let all = catalogue();
        for c in &all {
            assert_eq!(normalize(&c.id), c.id, "{} is not already canonical", c.id);
        }
        let mut ids: Vec<_> = all.iter().map(|c| c.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "duplicate chip ids");
    }

    #[test]
    fn spellings_from_the_wild_resolve() {
        for spelling in ["ESP32-C3", "esp32_c3", "esp32c3", "Esp32C3"] {
            assert_eq!(by_id(spelling).unwrap().id, "esp32c3", "{spelling}");
        }
        assert_eq!(by_id("STM32F411").unwrap().id, "stm32f411");
        assert!(by_id("nrf9160").is_none());
    }

    #[test]
    fn ambiguous_targets_report_every_candidate() {
        // C2 and C3 share a triple; a caller that assumes one answer would
        // silently mislabel half the projects that use it.
        let shared = chips_for_target("riscv32imc-unknown-none-elf");
        let ids: Vec<_> = shared.iter().map(|c| c.id.as_str()).collect();
        assert!(
            ids.contains(&"esp32c2") && ids.contains(&"esp32c3"),
            "{ids:?}"
        );

        let unique = chips_for_target("xtensa-esp32s3-none-elf");
        assert_eq!(unique.len(), 1);
        assert_eq!(unique[0].id, "esp32s3");
    }

    #[test]
    fn parts_without_a_std_target_do_not_offer_one() {
        // The wizard must not offer a std project where no target exists.
        for id in ["esp32p4", "stm32f411", "stm32f103"] {
            let chip = by_id(id).unwrap();
            assert!(chip.target_for(Runtime::EspIdf).is_none(), "{id}");
            assert!(chip.target_for(Runtime::BareMetal).is_some(), "{id}");
        }
    }

    #[test]
    fn every_part_has_a_way_to_be_flashed() {
        for c in catalogue() {
            assert!(!c.flashers.is_empty(), "{} cannot be flashed", c.id);
            // ST parts have no bootloader espflash speaks, so offering it
            // would send the user down a path that cannot work.
            if c.vendor == "st" {
                assert!(!c.flashers.contains(&Flasher::Espflash), "{}", c.id);
            }
            // A bootloader offered is a bootloader described: the family a
            // UF2 carries, where DFU writes.
            assert_eq!(
                c.flashers.contains(&Flasher::Uf2),
                c.uf2.is_some(),
                "{}: uf2 offered and described together",
                c.id
            );
            assert_eq!(
                c.flashers.contains(&Flasher::Dfu),
                c.dfu.is_some(),
                "{}: dfu offered and described together",
                c.id
            );
        }
    }

    /// What a part says about how it is started, simulated and drawn has to
    /// point at something that exists: a template rusty carries, a QEMU
    /// binary, a header made of pins the die has. Each of these was a
    /// `match` on the chip's id until it was data, and a typo in data is
    /// otherwise a part that silently has no playground, no emulator or
    /// somebody else's pins.
    #[test]
    fn what_a_part_names_exists() {
        for c in catalogue() {
            for generator in [&c.generator, &c.std_generator].into_iter().flatten() {
                if let crate::model::Generator::Template { name } = generator {
                    let template = crate::playground::template(name)
                        .unwrap_or_else(|| panic!("{}: no template `{name}`", c.id));
                    let from = template.chip.unwrap_or_else(|| {
                        panic!("{}: template `{name}` names no chip to move from", c.id)
                    });
                    assert!(by_id(from).is_some(), "{}: template chip `{from}`", c.id);
                }
            }
            // A C template, and what writing it needs: the flags its
            // toolchain file is made of, and the flash its linker script is.
            if let Some(name) = &c.c_template {
                let template = crate::playground::template(name)
                    .unwrap_or_else(|| panic!("{}: no C template `{name}`", c.id));
                assert!(
                    template.chip.is_some_and(|t| by_id(t).is_some()),
                    "{}",
                    c.id
                );
                assert!(
                    crate::buildsys::toolchain_file(&c).is_some() && c.flash_bytes.is_some(),
                    "{}: a C template with no toolchain file or no flash size to fill in",
                    c.id
                );
            }
            if let Some(emulation) = &c.emulation {
                assert_eq!(
                    emulation.binary.is_some(),
                    emulation.kind == crate::model::EmulatorKind::Qemu,
                    "{}: a QEMU binary exactly when QEMU runs it",
                    c.id
                );
            }
            for entry in &c.header {
                let pin = entry.rsplit(':').next().unwrap().parse::<u32>();
                if let Ok(pin) = pin {
                    assert!(c.gpio.contains(&pin), "{}: header pin {pin}", c.id);
                }
            }
            if c.vendor == "espressif" {
                assert!(
                    c.kit.is_some(),
                    "{}: every Espressif part has a devkit",
                    c.id
                );
                assert!(c.gdb.is_some(), "{}: and a debugger", c.id);
            }
        }
        // Every playground's chip is a part, and a template is offered for
        // the parts that name it.
        for template in crate::model::PLAYGROUNDS {
            let template = crate::playground::template(template).expect("a template");
            if let Some(chip) = template.chip {
                assert!(by_id(chip).is_some(), "playground `{}`", template.name);
            }
        }
    }

    /// A vendor added in a project's own file is a vendor like any other:
    /// its parts resolve to its name and HAL, and a part naming a vendor
    /// nobody declared is said rather than dropped.
    #[test]
    fn a_vendor_is_data_a_project_can_add() {
        let root = tempfile::tempdir().unwrap();
        let chips = root.path().join(".rusty").join("chips");
        std::fs::create_dir_all(&chips).unwrap();
        std::fs::write(
            chips.join("nordic.toml"),
            r#"
[[vendor]]
id = "nordic-lte"
name = "Nordic Semiconductor"
hal = "embassy-nrf"
chip_crates = ["embassy-nrf"]
bare_metal_crates = ["embassy-nrf"]

[[chip]]
id = "nrf9160"
name = "nRF9160"
vendor = "nordic-lte"
arch = "cortex-m"
cores = 1
sram_bytes = 262_144
bare_metal_target = "thumbv7em-none-eabihf"
toolchain = "stock"
flashers = ["probe-rs"]

[[chip]]
id = "mystery1"
name = "Mystery"
vendor = "nobody"
arch = "risc-v"
cores = 1
sram_bytes = 1
bare_metal_target = "riscv32imc-unknown-none-elf"
toolchain = "stock"
flashers = ["probe-rs"]
"#,
        )
        .unwrap();
        let catalog = Catalog::load(Some(root.path()));
        let nrf = catalog.chip("nrf9160").expect("the project's part");
        assert_eq!(nrf.vendor_name, "Nordic Semiconductor");
        assert_eq!(
            nrf.runtime_label(Runtime::BareMetal),
            "no_std (embassy-nrf)"
        );
        assert!(catalog.chip("mystery1").is_some(), "kept, not dropped");
        assert!(
            catalog
                .problems()
                .iter()
                .any(|p| p.detail.contains("mystery1") && p.detail.contains("nobody")),
            "{:?}",
            catalog.problems()
        );
    }

    #[test]
    fn every_board_names_a_chip_that_exists() {
        let catalog = Catalog::builtin();
        for board in catalog.boards() {
            assert!(
                catalog.chip(&board.chip).is_some(),
                "board `{}` references unknown chip `{}`",
                board.id,
                board.chip
            );
        }
    }
}
