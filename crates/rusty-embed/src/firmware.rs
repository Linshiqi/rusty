//! Finding what the project has already built.
//!
//! Three callers need a path to an ELF — the memory dashboard, the flasher, and
//! the assistant's `memory_report` tool — and none of them can be handed one by
//! the user without turning a workbench into a file browser.
//!
//! The obvious shortcut is to compose `target/<triple>/<profile>/<crate>` from
//! the manifest. It is wrong whenever a project renames its binary, declares
//! `[[bin]]`, or sets `CARGO_TARGET_DIR`, and the failure it produces is a
//! file-not-found that names a path nobody asked for. Walking the directory
//! instead means the answer is either a real file or an honest "nothing built
//! yet, run `cargo build`".

use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::model::Firmware;

/// Every ELF under `target/<triple>/<profile>/`, newest first.
///
/// Never fails: an unreadable or absent target directory means nothing has been
/// built, which is a normal state and not an error worth a dialog. Callers
/// distinguish "no builds" from "no project" themselves, because only they know
/// which they were asking about.
pub fn list(root: &Path, configured_target: Option<&str>) -> Vec<Firmware> {
    let mut found = Vec::new();

    let Ok(entries) = fs::read_dir(root.join("target")) else {
        return found;
    };

    for entry in entries.flatten() {
        let triple_dir = entry.path();
        if !triple_dir.is_dir() {
            continue;
        }
        // A target-triple directory is one that *contains* build profiles.
        // Recognising it structurally beats pattern-matching the name: it costs
        // nothing to be right about `riscv32imc-unknown-none-elf` and about
        // whatever triple a vendor invents next, and it excludes `target/debug`
        // — host build scripts and proc macros, which never reach a device.
        let Some(triple) = triple_dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        for profile in ["release", "debug"] {
            let dir = triple_dir.join(profile);
            let Ok(files) = fs::read_dir(&dir) else {
                continue;
            };

            for file in files.flatten() {
                let path = file.path();
                if !file.file_type().is_ok_and(|t| t.is_file()) || !is_elf(&path) {
                    continue;
                }
                let Some(name) = path.file_stem().and_then(|n| n.to_str()) else {
                    continue;
                };
                let metadata = file.metadata().ok();

                found.push(Firmware {
                    path: path.display().to_string(),
                    name: name.to_string(),
                    profile: profile.to_string(),
                    target: triple.to_string(),
                    bytes: metadata.as_ref().map_or(0, |m| m.len()),
                    modified: metadata
                        .and_then(|m| m.modified().ok())
                        .and_then(epoch_secs),
                    matches_configured_target: configured_target == Some(triple),
                });
            }
        }
    }

    // Newest first: "the one I just built" is what anyone opening this means,
    // and it is the only ordering that stays right as a project gains binaries.
    found.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    found
}

/// The build to use when the caller has not chosen one.
///
/// Prefers one built for the configured target over a newer one built for
/// something else. A stale image for the previous chip is the worst possible
/// default: it flashes cleanly and then behaves like a hardware fault.
///
/// Callers must report which file this returned. Choosing silently is fine —
/// it is a real file, not a guess — but staying silent about *which* one is
/// how the wrong binary gets analysed for twenty minutes.
pub fn newest(root: &Path, configured_target: Option<&str>) -> Option<Firmware> {
    let all = list(root, configured_target);
    all.iter()
        .find(|f| f.matches_configured_target)
        .cloned()
        .or_else(|| all.into_iter().next())
}

fn epoch_secs(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Read the magic rather than trust the extension.
///
/// A cross-compiled binary has no extension on any host, so there is nothing to
/// match on — and the same directory holds `.d` files, `.rlib`s and, on Windows,
/// host `.exe`s. Offering a non-ELF here would surface as a parse error two
/// screens later, blamed on the wrong thing.
fn is_elf(path: &Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == *b"\x7fELF"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Lay out a target directory the way cargo does, with a decoy in each
    /// place something non-ELF really appears.
    ///
    /// Modification times are set explicitly rather than left to write order:
    /// the ordering rules are the whole point of these tests, and a filesystem
    /// whose timestamp resolution is coarser than the writes would make them
    /// pass or fail by luck.
    fn scratch() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();

        let elf = |path: &Path, age_secs: u64| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let file = fs::File::create(path).unwrap();
            (&file).write_all(b"\x7fELF\x01\x01\x01").unwrap();
            file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(age_secs))
                .unwrap();
        };

        // The riscv build is the newer one. The xtensa build is older, and in
        // the tests below it is the one the project is configured for.
        elf(
            &root.join("target/xtensa-esp32-none-elf/debug/blinky"),
            1_000,
        );
        elf(
            &root.join("target/riscv32imc-unknown-none-elf/release/blinky"),
            2_000,
        );

        // A host build script binary. Under `target/debug`, which has no triple
        // above it, so the structural rule must skip the whole tree.
        elf(&root.join("target/debug/build-script-build"), 3_000);

        // Cargo's own leavings, in the same directory as the real binary.
        let noise = root.join("target/riscv32imc-unknown-none-elf/release/blinky.d");
        fs::write(&noise, "blinky: src/main.rs").unwrap();
        fs::create_dir_all(root.join("target/riscv32imc-unknown-none-elf/release/deps")).unwrap();

        dir
    }

    #[test]
    fn the_newest_build_is_listed_first() {
        let dir = scratch();
        let found = list(dir.path(), None);
        assert_eq!(
            found.first().map(|f| f.target.as_str()),
            Some("riscv32imc-unknown-none-elf")
        );
    }

    #[test]
    fn finds_built_elfs_and_nothing_else() {
        let dir = scratch();
        let found = list(dir.path(), None);

        let names: Vec<_> = found.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(found.len(), 2, "found {names:?}");

        assert!(
            found.iter().all(|f| f.name == "blinky"),
            "the .d file and the deps directory are not firmware: {names:?}",
        );
        assert!(
            !found.iter().any(|f| f.target == "debug"),
            "target/debug holds host build scripts, not device images: {names:?}",
        );
    }

    #[test]
    fn the_configured_target_is_marked_not_filtered() {
        let dir = scratch();
        let found = list(dir.path(), Some("xtensa-esp32-none-elf"));

        let matching: Vec<_> = found
            .iter()
            .filter(|f| f.matches_configured_target)
            .map(|f| f.target.as_str())
            .collect();
        assert_eq!(matching, ["xtensa-esp32-none-elf"]);

        // Kept, not hidden. A binary built for the wrong chip is the thing the
        // user most needs to be shown — silently omitting it turns "why is my
        // board behaving strangely" into an unanswerable question.
        assert!(
            found
                .iter()
                .any(|f| f.target == "riscv32imc-unknown-none-elf"),
            "a mismatched build must still be listed",
        );
    }

    #[test]
    fn the_default_choice_prefers_the_right_chip_over_the_newer_file() {
        let dir = scratch();
        // riscv is a thousand seconds newer; xtensa is what the project is
        // configured for. Flashing the newer, wrong-chip image would succeed
        // and then misbehave in a way that looks like broken hardware.
        let chosen = newest(dir.path(), Some("xtensa-esp32-none-elf")).expect("a build");
        assert_eq!(chosen.target, "xtensa-esp32-none-elf");

        // With nothing configured there is no better signal than recency.
        let fallback = newest(dir.path(), None).expect("a build");
        assert_eq!(fallback.target, "riscv32imc-unknown-none-elf");
    }

    #[test]
    fn a_project_that_never_built_is_empty_rather_than_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(list(dir.path(), None).is_empty());
    }
}
