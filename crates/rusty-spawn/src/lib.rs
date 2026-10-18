//! The rules every child process of rusty is started under, and the three
//! directories those rules need — one copy, for every crate.
//!
//! rusty-core, rusty-edit, rusty-lsp, rusty-git and rusty-dbg may not depend
//! on rusty-embed, and each carried its own three lines of "no console
//! window on Windows", its own `env_remove("RUSTUP_TOOLCHAIN")`, and its own
//! reading of `HOME` and `CARGO_HOME`. Six spellings of one rule is six
//! places for the next fix to miss: the home-directory rule had already
//! drifted (one copy honoured `CARGO_HOME`, another did not, and the two
//! disagreed about where cargo's binaries were), and the rustfmt finder had
//! neither the PATH rung nor the `--version` check the rust-analyzer finder
//! had learned from the CI runners. So the rules live here, below every
//! other crate, and each crate calls them.
//!
//! - [`no_console_window`]: a GUI process has no console, so a child that
//!   inherits none is given one — a black window flashed over the UI for
//!   every `cargo metadata`, `rustc -vV`, `git status` and `rustup which`.
//! - [`without_shim_toolchain`]: `RUSTUP_TOOLCHAIN` leaks from `cargo run`
//!   (the rustup shim sets it for *rusty's own* build) into every child, and
//!   rustup lets it outrank the project's `rust-toolchain.toml`; a spawned
//!   `cargo build` then compiles an esp-pinned Xtensa project with stable and
//!   dies with "can't find crate for `core`", and a spawned `rustup target
//!   list` answers for the wrong toolchain. The project's own pin decides.
//! - [`quiet`]: both, on a fresh `Command`.
//! - [`home_dir`], [`cargo_home`], [`rustup_home`]: the user's home and the
//!   two tool homes rustup and cargo define, each honouring the variable the
//!   tool itself honours.
//! - [`host_triple`]: what `rustc -vV` calls this machine, asked in a
//!   directory so a pinned toolchain answers.
//! - [`rustup_component`]: where a rustup component's binary is — and *that*
//!   it is one. A file called `rust-analyzer` on PATH is usually rustup's
//!   proxy, which exists on every machine with rustup whether or not the
//!   component does; existence is not installation, and only a binary that
//!   answers `--version` with the component's own name is taken.
//!
//! **Nothing with a dependency goes in here.** This crate sits under every
//! other crate in the workspace, including the ones that compile for the
//! window alone, and a dependency added here is a dependency added to all
//! of them. What it needs is `std::process`, `std::env` and `std::path`,
//! and that is all it may need: a rule that wants a crate belongs in
//! rusty-embed's `process`, which layers the tool directories and espup's
//! environment on top of [`quiet`].

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

/// Keep a child process from flashing a console window on Windows.
#[cfg(windows)]
pub fn no_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

/// Keep a child process from flashing a console window on Windows.
#[cfg(not(windows))]
pub fn no_console_window(_command: &mut Command) {}

/// Drop the `RUSTUP_TOOLCHAIN` the rustup shim set for rusty's own build, so
/// the child answers for the project's pin and not for the toolchain that
/// built rusty. See the crate header.
pub fn without_shim_toolchain(command: &mut Command) {
    command.env_remove("RUSTUP_TOOLCHAIN");
}

/// A `Command` for `program` under both rules: no console window, no
/// inherited `RUSTUP_TOOLCHAIN`.
pub fn quiet(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    without_shim_toolchain(&mut command);
    no_console_window(&mut command);
    command
}

/// The user's home: `USERPROFILE` on Windows (`HOME` when a shell set one
/// and the profile is missing), `HOME` elsewhere.
pub fn home_dir() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME")
    };
    home.map(PathBuf::from)
}

/// cargo's home — `$CARGO_HOME`, else `~/.cargo` — where `cargo install`
/// puts binaries and where the registry keeps its sources.
pub fn cargo_home() -> Option<PathBuf> {
    cargo_home_from(
        std::env::var_os("CARGO_HOME").map(PathBuf::from),
        home_dir(),
    )
}

/// `cargo_home`'s rule, over values a test can choose.
fn cargo_home_from(configured: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    configured.or_else(|| home.map(|home| home.join(".cargo")))
}

/// `$RUSTUP_HOME`, else `~/.rustup` — rustup's own rule.
pub fn rustup_home() -> Option<PathBuf> {
    std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".rustup")))
}

/// The host toolchain's target triple, as `rustc -vV` reports it.
///
/// `std::env::consts` cannot produce a real triple (it has no vendor or
/// ABI), so the authority is asked. Asked in `cwd` when one is given, so a
/// `rust-toolchain.toml` pin answers rather than the default; `None` when
/// rustc is not there or did not say.
pub fn host_triple(cwd: Option<&Path>) -> Option<String> {
    let mut command = quiet("rustc");
    command.arg("-vV");
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let output = command.output().ok().filter(|out| out.status.success())?;
    host_from_rustc(&String::from_utf8_lossy(&output.stdout))
}

/// The `host:` line of `rustc -vV`'s output.
pub fn host_from_rustc(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
}

/// Where a rustup component's binary is: the first of
/// [`component_candidates`] that answers as the component.
pub fn rustup_component(name: &str) -> Option<PathBuf> {
    rustup_component_version(name).map(|(path, _)| path)
}

/// [`rustup_component`], with the first line of what the binary answered
/// for `--version` — the panel shows it beside the tool.
pub fn rustup_component_version(name: &str) -> Option<(PathBuf, String)> {
    component_candidates(name)
        .into_iter()
        .find_map(|candidate| verified_component(name, &candidate).map(|said| (candidate, said)))
}

/// The files a rustup component's binary may be, in the order to try them:
/// what `rustup which --toolchain stable` names (the copy the editor runs —
/// the bare name on PATH is rustup's proxy, which dispatches by the
/// project's pinned toolchain, and an esp-pinned project has no
/// rust-analyzer there), then what `rustup which` names for the default
/// toolchain, then the name on every directory of PATH. Each is a file;
/// none is yet known to be the component — that is [`verified_component`].
pub fn component_candidates(name: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for toolchain in [Some("stable"), None] {
        let mut command = quiet("rustup");
        command.arg("which");
        if let Some(toolchain) = toolchain {
            command.args(["--toolchain", toolchain]);
        }
        command.arg(name);
        if let Ok(out) = command.output()
            && out.status.success()
        {
            let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if path.is_file() && !found.contains(&path) {
                found.push(path);
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        let exe = exe_name(name);
        for candidate in std::env::split_paths(&path).map(|dir| dir.join(&exe)) {
            if candidate.is_file() && !found.contains(&candidate) {
                found.push(candidate);
            }
        }
    }
    found
}

/// The first line `candidate` answers `--version` with, when it answers as
/// the component `name` — its own name first. rustup's proxy for a missing
/// component answers with an error on stderr and a non-zero exit; a stale
/// or foreign binary with neither.
pub fn verified_component(name: &str, candidate: &Path) -> Option<String> {
    let mut command = quiet(candidate);
    command.arg("--version");
    let out = command.output().ok().filter(|out| out.status.success())?;
    let said = String::from_utf8_lossy(&out.stdout);
    is_component_version(name, &said).then(|| {
        said.lines()
            .next()
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    })
}

/// What a component prints for `--version` begins with its name; what
/// rustup's proxy prints for a missing one does not (and goes to stderr).
pub fn is_component_version(name: &str, said: &str) -> bool {
    said.trim_start().starts_with(name)
}

/// A tool's file name on this platform: `name.exe` on Windows.
pub fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real binary names itself; the proxy's complaint — in either of
    /// its wordings — an empty answer, and another component's name are
    /// not it.
    #[test]
    fn a_component_is_present_when_its_version_says_so_not_when_a_file_exists() {
        assert!(is_component_version(
            "rust-analyzer",
            "rust-analyzer 1.98.0 (5f3ac7d 2026-08-30)"
        ));
        assert!(is_component_version(
            "rust-analyzer",
            "rust-analyzer 1.98.0 (88d9e12ae 2026-08-18)\n"
        ));
        assert!(is_component_version(
            "rust-analyzer",
            "  rust-analyzer 0.3.2500-standalone"
        ));
        assert!(!is_component_version(
            "rust-analyzer",
            "error: unknown binary 'rust-analyzer' in toolchain 'esp'"
        ));
        assert!(!is_component_version(
            "rust-analyzer",
            "error: 'rust-analyzer' is not installed for the toolchain 'stable-x86_64-pc-windows-msvc'\n"
        ));
        assert!(!is_component_version("rust-analyzer", ""));
        assert!(!is_component_version(
            "rust-analyzer",
            "rustfmt 1.8.0-stable"
        ));
        assert!(is_component_version(
            "rustfmt",
            "rustfmt 1.8.0-stable (6b00bc3 2025-01-30)"
        ));
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

    /// The triple is the `host:` line and nothing else of what rustc says;
    /// an answer without one is no triple, not an empty one.
    #[test]
    fn the_host_triple_is_read_off_rustcs_own_line() {
        let said = "rustc 1.98.0 (abcdef 2026-09-01)\nbinary: rustc\ncommit-hash: abcdef\n\
                    host: aarch64-apple-darwin\nrelease: 1.98.0\nLLVM version: 21.1.0\n";
        assert_eq!(
            host_from_rustc(said).as_deref(),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(host_from_rustc("rustc 1.98.0\n"), None);
        assert_eq!(host_from_rustc("host: \n"), None);
    }

    /// A child under `quiet` carries no `RUSTUP_TOOLCHAIN`, whatever this
    /// process inherited from the shim.
    #[test]
    fn the_shims_toolchain_is_removed_from_a_quiet_child() {
        let command = quiet("rustc");
        assert!(
            command
                .get_envs()
                .any(|(name, value)| name == "RUSTUP_TOOLCHAIN" && value.is_none()),
            "{:?}",
            command.get_envs().collect::<Vec<_>>()
        );
    }
}
