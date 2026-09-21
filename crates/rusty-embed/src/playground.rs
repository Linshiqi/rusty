//! The playground: a project rusty keeps for trying things, so writing code
//! and watching it run on the simulated board needs no new project, no
//! folder and no generator — Wokwi's "new project", without the account and
//! without the cloud.
//!
//! One per chip, at `<data dir>/playground/<chip>/`, written from templates
//! compiled into the binary the first time it is opened — and never over a
//! file that is there, because what somebody wrote in it yesterday is
//! theirs today. [`reset`] puts the example back when they ask for it.
//!
//! The templates are the proven projects, not new ones: the C3's is
//! `examples/blink-rust`'s shape and lockfile, which the emulator's gate 7
//! boots, and the ESP32's is `qemu/esp32-probe`'s, which gate 16 boots —
//! with a release profile made for editing rather than shipping (no LTO,
//! incremental), since the whole point is that an edit runs in seconds.
//! They sit in `data/playground/` with an `.in` on every name, so no tool
//! walking the repository takes a template for a project of its own.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
// Spelled in the model, because the window offers the same list and opens
// the same file.
use crate::model::{PLAYGROUND_CHIPS as CHIPS, PLAYGROUND_MAIN as MAIN};

/// A template file: where it goes in the project, and what it says.
type File = (&'static str, &'static str);

const BUILD_RS: &str = include_str!("../data/playground/build.rs.in");

fn template(chip: &str) -> Option<[File; 7]> {
    match chip {
        "esp32c3" => Some([
            (
                "Cargo.toml",
                include_str!("../data/playground/esp32c3/Cargo.toml.in"),
            ),
            (
                "Cargo.lock",
                include_str!("../data/playground/esp32c3/Cargo.lock.in"),
            ),
            ("build.rs", BUILD_RS),
            (
                "rust-toolchain.toml",
                include_str!("../data/playground/esp32c3/rust-toolchain.toml.in"),
            ),
            (
                ".cargo/config.toml",
                include_str!("../data/playground/esp32c3/cargo-config.toml.in"),
            ),
            (MAIN, include_str!("../data/playground/esp32c3/main.rs.in")),
            (
                ".rusty/sim.toml",
                include_str!("../data/playground/esp32c3/sim.toml.in"),
            ),
        ]),
        "esp32" => Some([
            (
                "Cargo.toml",
                include_str!("../data/playground/esp32/Cargo.toml.in"),
            ),
            (
                "Cargo.lock",
                include_str!("../data/playground/esp32/Cargo.lock.in"),
            ),
            ("build.rs", BUILD_RS),
            (
                "rust-toolchain.toml",
                include_str!("../data/playground/esp32/rust-toolchain.toml.in"),
            ),
            (
                ".cargo/config.toml",
                include_str!("../data/playground/esp32/cargo-config.toml.in"),
            ),
            (MAIN, include_str!("../data/playground/esp32/main.rs.in")),
            (
                ".rusty/sim.toml",
                include_str!("../data/playground/esp32/sim.toml.in"),
            ),
        ]),
        _ => None,
    }
}

fn templates_for(chip: &str) -> Result<[File; 7]> {
    template(chip).ok_or_else(|| {
        Error::refused(format!(
            "There is no playground for {chip} — there is one for each of {}.",
            CHIPS.join(" and ")
        ))
    })
}

/// Where a chip's playground lives.
pub fn dir(data: &Path, chip: &str) -> PathBuf {
    data.join("playground").join(chip)
}

/// The chip whose playground `root` is, or `None` for any other project —
/// which decides how the window lays a project out.
pub fn chip_of(data: &Path, root: &Path) -> Option<&'static str> {
    CHIPS
        .into_iter()
        .find(|chip| same_path(&dir(data, chip), root))
}

fn same_path(a: &Path, b: &Path) -> bool {
    let spell = |p: &Path| {
        p.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    };
    spell(a) == spell(b)
}

/// The playground for `chip`, ready to open: every template file written
/// that is not there yet, and not one that is.
pub fn prepare(data: &Path, chip: &str) -> Result<PathBuf> {
    let files = templates_for(chip)?;
    let root = dir(data, chip);
    for (relative, text) in files {
        let path = root.join(relative);
        if !path.exists() {
            write(&path, text)?;
        }
    }
    Ok(root)
}

/// Put the example back: every template file written again, whatever it
/// says now. `target/` is left alone, so the next Run compiles the example
/// and not every dependency again.
pub fn reset(data: &Path, chip: &str) -> Result<PathBuf> {
    let files = templates_for(chip)?;
    let root = dir(data, chip);
    for (relative, text) in files {
        write(&root.join(relative), text)?;
    }
    Ok(root)
}

/// Keep what is in the playground as a project of its own, at `dest`: every
/// file but the build, copied — `target/` is what one `cargo build` makes
/// again, and it runs to gigabytes. `dest` has to be empty or not there yet,
/// because a copy that met a file already in somebody's folder would have to
/// overwrite it or stop half way, and neither is a copy.
pub fn keep(data: &Path, chip: &str, dest: &Path) -> Result<PathBuf> {
    templates_for(chip)?;
    let root = dir(data, chip);
    if !root.join("Cargo.toml").is_file() {
        return Err(Error::refused(format!(
            "There is no {chip} playground to keep yet — open it first."
        )));
    }
    // Inside itself, the copy would meet its own output on the way down.
    if dest.ancestors().any(|above| same_path(above, &root)) {
        return Err(Error::refused(format!(
            "{} is inside the playground — choose a folder somewhere else to keep it in.",
            dest.display()
        )));
    }
    let occupied = std::fs::read_dir(dest).is_ok_and(|mut entries| entries.next().is_some());
    if occupied {
        return Err(Error::refused(format!(
            "{} is not empty — choose an empty folder, or make a new one, to keep the playground in.",
            dest.display()
        )));
    }
    copy_tree(&root, dest, true)?;
    Ok(dest.to_path_buf())
}

/// Every file under `from`, copied to the same place under `to`, less the
/// top level's `target/`.
fn copy_tree(from: &Path, to: &Path, top: bool) -> Result<()> {
    let unreadable = |source| Error::Read {
        path: from.display().to_string(),
        source,
    };
    let entries = std::fs::read_dir(from).map_err(unreadable)?;
    std::fs::create_dir_all(to).map_err(|source| Error::Write {
        path: to.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(unreadable)?;
        let kind = entry.file_type().map_err(unreadable)?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            if top && entry.file_name() == "target" {
                continue;
            }
            copy_tree(&entry.path(), &target, false)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target).map_err(|source| Error::Write {
                path: target.display().to_string(),
                source,
            })?;
        }
    }
    Ok(())
}

fn write(path: &Path, text: &str) -> Result<()> {
    let fail = |source| Error::Write {
        path: path.display().to_string(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(fail)?;
    }
    std::fs::write(path, text).map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opening writes the project; opening again after somebody has written
    /// in it keeps what they wrote; reset puts the example back.
    #[test]
    fn a_playground_is_written_once_and_kept() {
        let data = tempfile::tempdir().unwrap();
        let root = prepare(data.path(), "esp32c3").unwrap();
        assert!(root.join("Cargo.toml").is_file());
        assert!(root.join(".cargo/config.toml").is_file());
        assert!(root.join(".rusty/sim.toml").is_file());
        let main = root.join(MAIN);
        assert!(std::fs::read_to_string(&main).unwrap().contains("#[main]"));

        std::fs::write(&main, "// mine").unwrap();
        prepare(data.path(), "esp32c3").unwrap();
        assert_eq!(std::fs::read_to_string(&main).unwrap(), "// mine");

        reset(data.path(), "esp32c3").unwrap();
        assert!(std::fs::read_to_string(&main).unwrap().contains("#[main]"));
    }

    /// Keeping copies the project and not its build, and refuses a folder
    /// that has something in it or that is inside the playground itself.
    #[test]
    fn a_playground_is_kept_without_its_build_and_only_somewhere_empty() {
        let data = tempfile::tempdir().unwrap();
        let root = prepare(data.path(), "esp32c3").unwrap();
        std::fs::write(root.join(MAIN), "// mine").unwrap();
        std::fs::create_dir_all(root.join("target/release")).unwrap();
        std::fs::write(root.join("target/release/app"), "elf").unwrap();

        let out = tempfile::tempdir().unwrap();
        let dest = out.path().join("blink");
        assert_eq!(keep(data.path(), "esp32c3", &dest).unwrap(), dest);
        assert_eq!(std::fs::read_to_string(dest.join(MAIN)).unwrap(), "// mine");
        assert!(dest.join(".cargo/config.toml").is_file());
        assert!(dest.join(".rusty/sim.toml").is_file());
        assert!(dest.join("Cargo.lock").is_file());
        assert!(!dest.join("target").exists(), "the build is not kept");

        let again = keep(data.path(), "esp32c3", &dest).unwrap_err().to_string();
        assert!(again.contains("not empty"), "{again}");
        let inside = keep(data.path(), "esp32c3", &root.join("copy"))
            .unwrap_err()
            .to_string();
        assert!(inside.contains("inside the playground"), "{inside}");
        let unopened = keep(data.path(), "esp32", &out.path().join("other"))
            .unwrap_err()
            .to_string();
        assert!(unopened.contains("open it first"), "{unopened}");
    }

    /// A chip with no template is refused by name, with the ones there are.
    #[test]
    fn a_chip_without_a_playground_is_refused_with_the_ones_there_are() {
        let data = tempfile::tempdir().unwrap();
        let err = prepare(data.path(), "stm32f411").unwrap_err().to_string();
        assert!(err.contains("stm32f411"), "{err}");
        assert!(err.contains("esp32c3"), "{err}");
    }

    /// Each template is a project rusty itself reads the way it reads any
    /// other: the chip it detects is the playground's and the target is set.
    #[test]
    fn every_playground_is_a_project_rusty_detects_as_its_chip() {
        let data = tempfile::tempdir().unwrap();
        for chip in CHIPS {
            let root = prepare(data.path(), chip).unwrap();
            let project = crate::project::detect(&root).unwrap();
            assert_eq!(project.chip.as_deref(), Some(chip), "{chip}");
            assert!(project.configured_target.is_some(), "{chip}");
            assert_eq!(chip_of(data.path(), &root), Some(chip));
        }
        assert_eq!(chip_of(data.path(), &data.path().join("elsewhere")), None);
    }

    /// Each board is wired the way its comment says, by the sheet's own
    /// rules: no wire to a pin that is not there, the LED lit when its GPIO
    /// is high and dark when it is low, and the button pulling its GPIO to
    /// ground. A template whose drawing the rules disagreed with would open
    /// every new user's first minute on a warning.
    #[test]
    fn every_playground_s_board_is_wired_as_its_comment_says() {
        use std::collections::{HashMap, HashSet};

        let data = tempfile::tempdir().unwrap();
        for (chip, led, button) in [("esp32c3", 0u8, 4u8), ("esp32", 2, 4)] {
            let root = prepare(data.path(), chip).unwrap();
            let sheet = crate::simulate::load_board_for_test(&root, chip)
                .unwrap_or_else(|| panic!("{chip}: the board file loads"));
            let gpio = crate::chip::by_id(chip).unwrap().gpio;
            let rows = crate::nets::kit_rows(chip, &gpio);
            let evaluate = |high: bool| {
                crate::nets::evaluate(crate::nets::Inputs {
                    sheet: &sheet,
                    rows: &rows,
                    gpio: &HashMap::from([(led, high)]),
                    pressed: &HashSet::new(),
                })
            };
            let lit = evaluate(true);
            assert!(lit.warnings.is_empty(), "{chip}: {:?}", lit.warnings);
            assert!(lit.is_lit("D1"), "{chip}: GPIO{led} high lights the LED");
            assert!(!evaluate(false).is_lit("D1"), "{chip}: and low does not");
            assert_eq!(
                crate::nets::button_drives(&sheet, &rows, "SW1"),
                Some((button, false)),
                "{chip}: the button pulls GPIO{button} to ground",
            );
            // And its numbers: the sheet solves rather than refusing, and
            // with the LED's pin high the lamp carries what the resistor
            // allows — (3.3 V − 2 V) / 220 Ω, near six milliamps.
            let solved = crate::circuit::operating_point(
                &sheet,
                &rows,
                &HashSet::new(),
                &std::collections::BTreeMap::from([(led, true)]),
            )
            .unwrap_or_else(|why| panic!("{chip}: the sheet solves: {why}"));
            let amps = solved
                .reading("D1")
                .unwrap_or_else(|| panic!("{chip}: the LED is in the circuit"))
                .through
                .abs();
            assert!(
                (0.004..0.008).contains(&amps),
                "{chip}: {amps} A through D1"
            );
        }
    }
}
