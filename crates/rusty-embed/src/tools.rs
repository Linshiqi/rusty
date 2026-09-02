//! Finding the binaries the workbench drives.
//!
//! Its own module because four callers need it and none of them is about
//! simulation: the toolchain panel probes with it, `process::command` puts these
//! directories on a child's PATH, the installer checks whether its work
//! landed, and the simulator looks for QEMU. It lived in `simulate.rs`, which
//! meant `toolchain.rs` importing `simulate::find_tool` to answer "is espflash
//! installed" — a dependency that says nothing true about either module.
//!
//! **One ladder, in one order, for every binary** — [`find`]:
//!
//! 1. rusty's own `tools/` in the data directory: every `<family>/bin/` under
//!    it, and `<family>/` itself for the archives that unpack with no `bin/`.
//!    First, because a tool rusty downloaded on request has to be found or the
//!    panel keeps offering to install it again.
//! 2. `$CARGO_HOME/bin` (else `~/.cargo/bin`), where `cargo install` puts
//!    espflash and friends. Usually on PATH too — but not in a window opened
//!    before rustup ran, which is exactly the first-run machine.
//! 3. PATH.
//!
//! Three finders used to each have their own order and two of them disagreed:
//! QEMU was PATH first and gdb was the data directory first, and espflash's
//! cargo fallback ignored `CARGO_HOME` while the pin map honoured it. A caller
//! that checks for a tool under one rule and runs it under another reports it
//! installed and then fails to start it.
//!
//! The child's PATH is a different question with the opposite answer:
//! `process::command` *appends* these directories, because what a child
//! resolves by name is `cc` asking for a compiler, and the one the user put on
//! PATH themselves is the one they meant. This ladder answers "which binary
//! does rusty run", and there the copy rusty installed wins.

use std::path::{Path, PathBuf};

use crate::config;

/// A binary's file name on this platform.
pub(crate) fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Where a binary is, by the ladder in the module header, or `None` when this
/// machine has none.
pub(crate) fn find(name: &str) -> Option<PathBuf> {
    find_in(name, data_tools_dir().as_deref())
}

/// [`find`] with the data directory's `tools/` given rather than resolved, so
/// the ladder can be tested against a directory a test made rather than the
/// machine it runs on.
pub(crate) fn find_in(name: &str, tools: Option<&Path>) -> Option<PathBuf> {
    let file = exe(name);
    if let Some(tools) = tools {
        for family in tool_families(tools) {
            for candidate in [family.join("bin").join(&file), family.join(&file)] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    if let Some(bin) = cargo_bin() {
        let candidate = bin.join(&file);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    on_path(name)
}

/// The first match for a binary on PATH, and nothing else — the question
/// [`find`] asks last.
pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(exe(name));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// `tools/` in the data directory — where every archive the installer fetches
/// unpacks.
pub(crate) fn data_tools_dir() -> Option<PathBuf> {
    config::data_dir().map(|d| d.join("tools"))
}

/// Every `bin/` under rusty's tools directory, for putting on a child's PATH.
///
/// `cc` invokes the cross compiler *by name*, so a compiler rusty unpacked
/// into its own directory is one cargo cannot find however correctly the
/// panel reports it. Handing the directories to the child is what closes
/// that gap without touching the user's environment.
pub(crate) fn tool_bin_dirs() -> Vec<PathBuf> {
    let Some(tools) = data_tools_dir() else {
        return Vec::new();
    };
    tool_families(&tools)
        .into_iter()
        .map(|family| family.join("bin"))
        .filter(|bin| bin.is_dir())
        .collect()
}

/// The directories directly under `tools/`, sorted by name.
///
/// Sorted so the answer does not depend on the filesystem's enumeration
/// order: two families carrying the same binary name would otherwise be found
/// in a different order on different disks.
fn tool_families(tools: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(tools) else {
        return Vec::new();
    };
    let mut families: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    families.sort();
    families
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// cargo's home — `$CARGO_HOME`, else `~/.cargo` — where `cargo install` puts
/// binaries and where the registry keeps its sources.
pub(crate) fn cargo_home() -> Option<PathBuf> {
    cargo_home_from(
        std::env::var_os("CARGO_HOME").map(PathBuf::from),
        home_dir(),
    )
}

/// `cargo_home`'s rule, over values a test can choose.
fn cargo_home_from(configured: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    configured.or_else(|| home.map(|home| home.join(".cargo")))
}

fn cargo_bin() -> Option<PathBuf> {
    cargo_home().map(|home| home.join("bin"))
}

/// The platform in Espressif's asset naming — which rusty's own packages
/// reuse, so one ladder of URLs covers both.
///
/// Transcribed from the release's actual asset list rather than assembled
/// from `env::consts`: `x86_64-w64-mingw32` is not a string any pair of those
/// constants spells, and an asset name that is *nearly* right 404s exactly
/// like a network problem.
pub(crate) fn host_platform() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-w64-mingw32",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-linux-gnu",
        ("linux", "aarch64") => "aarch64-linux-gnu",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binary_name_carries_the_platforms_extension() {
        let named = exe("espflash");
        if cfg!(windows) {
            assert_eq!(named, "espflash.exe");
        } else {
            assert_eq!(named, "espflash");
        }
    }

    /// Probing must never fail the caller: a machine with none of these tools
    /// installed is the normal state before the toolchain panel is read.
    #[test]
    fn probing_for_something_absent_answers_none() {
        assert!(on_path("a-binary-nobody-has-installed-xyzzy").is_none());
        assert!(find("a-binary-nobody-has-installed-xyzzy").is_none());
    }

    /// Both layouts the installer produces are searched: QEMU and the
    /// debuggers unpack to `<family>/bin/`, espflash's archive to `<family>/`.
    /// Missing the second is how a downloaded espflash kept being offered for
    /// download.
    #[test]
    fn the_data_directory_is_searched_with_and_without_a_bin_level() {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        let qemu = tools.join("qemu/bin").join(exe("qemu-system-riscv32"));
        let espflash = tools.join("espflash").join(exe("espflash"));
        for file in [&qemu, &espflash] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"").unwrap();
        }

        assert_eq!(find_in("qemu-system-riscv32", Some(&tools)), Some(qemu));
        assert_eq!(find_in("espflash", Some(&tools)), Some(espflash));
        assert_eq!(
            find_in("a-binary-nobody-has-installed-xyzzy", Some(&tools)),
            None,
        );
        // The directory itself is not a binary, and a name matching a family
        // must not be answered with a folder.
        assert_eq!(find_in("qemu", Some(&tools)), None);
    }

    /// The copy rusty installed wins over whatever else the machine has, or
    /// the panel offers to install a tool that is already there.
    #[test]
    fn the_data_directory_comes_before_the_rest_of_the_ladder() {
        // `cargo` is on every machine that can run these tests; a copy in the
        // tools directory has to be the one reported.
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        let ours = tools.join("cargo-shim/bin").join(exe("cargo"));
        std::fs::create_dir_all(ours.parent().unwrap()).unwrap();
        std::fs::write(&ours, b"").unwrap();

        assert_eq!(find_in("cargo", Some(&tools)), Some(ours));
        assert!(
            find_in("cargo", None).is_some(),
            "and without a tools directory the ladder still reaches PATH",
        );
    }

    /// `CARGO_HOME` outranks the home directory, as it does for cargo itself;
    /// the pin map honoured this and the espflash finder did not, so the two
    /// disagreed about where cargo's binaries were.
    #[test]
    fn cargo_home_is_the_variable_when_set_and_the_home_directory_otherwise() {
        assert_eq!(
            cargo_home_from(
                Some(PathBuf::from("/opt/cargo")),
                Some(PathBuf::from("/home/me"))
            ),
            Some(PathBuf::from("/opt/cargo")),
        );
        assert_eq!(
            cargo_home_from(None, Some(PathBuf::from("/home/me"))),
            Some(PathBuf::from("/home/me").join(".cargo")),
        );
        assert_eq!(cargo_home_from(None, None), None);
    }
}
