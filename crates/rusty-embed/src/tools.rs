//! Finding the binaries the workbench drives.
//!
//! Its own module because four callers need it and none of them is about
//! simulation: the toolchain panel probes with it, `process::spawn` puts these
//! directories on a child's PATH, the installer checks whether its work
//! landed, and the simulator looks for QEMU. It lived in `simulate.rs`, which
//! meant `toolchain.rs` importing `simulate::find_tool` to answer "is espflash
//! installed" — a dependency that says nothing true about either module.
//!
//! Two places a tool can be, and the order matters: what rusty unpacked into
//! its own data directory, then PATH. A tool the user installed themselves is
//! the one they meant, but a tool rusty downloaded on request has to be found
//! or the panel keeps offering to install it again.

use std::path::PathBuf;

use crate::config;

/// A binary's file name on this platform.
pub(crate) fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// A binary rusty may have downloaded itself, or one the machine already had.
///
/// `tools/<family>/bin/` first, then PATH — the order `find_gdb` and
/// `find_qemu` have always used, made available to everything else that
/// probes. Without it, a tool rusty installed on request reports as absent
/// and the panel keeps offering to install it again.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    if let Some(tools) = config::data_dir().map(|d| d.join("tools")) {
        for family in ["riscv32-esp-elf", "xtensa-esp-elf"] {
            let bundled = tools.join(family).join("bin").join(exe(name));
            if bundled.is_file() {
                return Some(bundled);
            }
        }
    }
    on_path(name)
}

/// The first match for a binary on PATH, and nothing else — the question
/// `find_tool` asks last.
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

/// Every `bin/` under rusty's tools directory, for putting on a child's PATH.
///
/// `cc` invokes the cross compiler *by name*, so a compiler rusty unpacked
/// into its own directory is one cargo cannot find however correctly the
/// panel reports it. Handing the directories to the child is what closes
/// that gap without touching the user's environment.
pub fn tool_bin_dirs() -> Vec<PathBuf> {
    let Some(tools) = config::data_dir().map(|d| d.join("tools")) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&tools) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path().join("bin"))
        .filter(|bin| bin.is_dir())
        .collect()
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
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
        assert!(find_tool("a-binary-nobody-has-installed-xyzzy").is_none());
    }
}
