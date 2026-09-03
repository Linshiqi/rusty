//! Where rust-analyzer is, and how it is started.
//!
//! Two pieces of embedded-specific knowledge live here, because getting
//! either wrong looks like "rust-analyzer is broken" rather than like a
//! setting:
//!
//! - The `rust-analyzer` on PATH is usually rustup's proxy, which dispatches
//!   by the project's pinned toolchain — and an ESP project pins `esp`, which
//!   has no rust-analyzer component, so the proxy fails *precisely for the
//!   projects this workbench serves*. The stable toolchain's real binary is
//!   resolved first instead; it analyses any toolchain's project fine, and
//!   reads the pinned toolchain's own sysroot for the target's `core`.
//! - A workspace that excludes its firmware crate gets no IDE services there
//!   unless the excluded manifest is named to the server. [`linked_projects`]
//!   reads `workspace.exclude` for exactly that.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Where rust-analyzer actually is.
///
/// The bare name on PATH is rustup's proxy, which dispatches by the pinned
/// toolchain — and `rust-toolchain.toml` pinning `esp` (every Xtensa project)
/// makes the proxy fail with "unknown binary in toolchain 'esp'". So: stable's
/// real binary first, the active toolchain's second, PATH last.
pub fn find_rust_analyzer() -> Option<PathBuf> {
    for toolchain in [Some("stable"), None] {
        let mut command = Command::new("rustup");
        command.arg("which");
        if let Some(toolchain) = toolchain {
            command.args(["--toolchain", toolchain]);
        }
        command.arg("rust-analyzer");
        no_console_window(&mut command);
        if let Ok(out) = command.output()
            && out.status.success()
        {
            let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if path.is_file() && answers_as_rust_analyzer(&path) {
                return Some(path);
            }
        }
    }

    // No rustup: take PATH literally — but not on faith. The file found there
    // is usually rustup's own proxy, which exists on every machine with
    // rustup whether or not the component does; with it missing, the proxy
    // starts, prints an error and exits, and a client that trusted the file
    // spawned it, lost it, and reported "spawn rust-analyzer" failed. That
    // was the CI runner: no component, a proxy on PATH, and a test that was
    // written to skip without rust-analyzer running instead.
    let path = std::env::var_os("PATH")?;
    let name = if cfg!(windows) {
        "rust-analyzer.exe"
    } else {
        "rust-analyzer"
    };
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file() && answers_as_rust_analyzer(candidate))
}

/// Whether the file is rust-analyzer itself: it answers `--version` with its
/// own name. rustup's proxy for a missing component answers with an error
/// and a non-zero exit; a stale or foreign binary with neither.
fn answers_as_rust_analyzer(candidate: &Path) -> bool {
    let mut command = Command::new(candidate);
    command.arg("--version");
    command.env_remove("RUSTUP_TOOLCHAIN");
    no_console_window(&mut command);
    command.output().is_ok_and(|out| {
        out.status.success() && is_rust_analyzer_version(&String::from_utf8_lossy(&out.stdout))
    })
}

/// What `rust-analyzer --version` prints begins with the name; what the
/// rustup proxy prints for a missing component does not (and goes to stderr).
fn is_rust_analyzer_version(stdout: &str) -> bool {
    stdout.trim_start().starts_with("rust-analyzer")
}

/// The server process, ready to spawn: stdio piped, and the environment as
/// the project would want it.
pub(crate) fn command_for(binary: &Path, root: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // rust-analyzer narrates progress on stderr. Piped-but-undrained
        // would fill the pipe and deadlock the server mid-index, so it is
        // discarded — except when someone is diagnosing "no diagnostics",
        // which is exactly when the server's own complaints are the answer.
        .stderr(if std::env::var_os("RUSTY_LSP_LOG").is_some() {
            Stdio::inherit()
        } else {
            Stdio::null()
        });
    // The rustup shim exports this for rusty's own build, and rust-analyzer
    // has no business inheriting it: the server runs `cargo metadata` in the
    // project, and rustup lets the variable outrank `rust-toolchain.toml`,
    // so an esp-pinned project would be read with stable — the same leak
    // that made a spawned cargo fail with "can't find crate for `core`".
    command.env_remove("RUSTUP_TOOLCHAIN");
    no_console_window(&mut command);
    command
}

/// Manifests rust-analyzer would otherwise never see.
///
/// The layout this exists for is the standard embedded one: a workspace whose
/// host-testable crates are members, and a firmware crate `exclude`d because
/// it needs a bare-metal target and its own toolchain — `cargo test` at the
/// root would otherwise try to build `no_std` firmware for the host.
///
/// rust-analyzer loads *one* workspace from the root, so every file under the
/// excluded directory comes back "not included in any crates, so
/// rust-analyzer can't offer IDE services" — no completion, no diagnostics,
/// no navigation, in exactly the half of the repository this workbench is
/// for. `linkedProjects` is the server's own answer: name the extra manifests
/// and it loads them alongside.
///
/// Read from `workspace.exclude` rather than guessed by walking: a directory
/// the workspace deliberately named is a fact, and linking every `Cargo.toml`
/// under the root would pull in vendored copies and fixtures.
pub(crate) fn linked_projects(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return Vec::new();
    };
    // `Table`, not `Value`: in toml 1.x `Value`'s `FromStr` parses a single
    // TOML *value*, so a whole manifest fails at the first table header with
    // an error that reads as a broken `Cargo.toml`.
    let Ok(manifest) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let Some(excluded) = manifest
        .get("workspace")
        .and_then(|w| w.get("exclude"))
        .and_then(toml::Value::as_array)
    else {
        return Vec::new();
    };
    excluded
        .iter()
        .filter_map(toml::Value::as_str)
        .map(|name| root.join(name).join("Cargo.toml"))
        .filter(|manifest| manifest.is_file())
        .map(|manifest| manifest.to_string_lossy().into_owned())
        .collect()
}

/// Same reason as everywhere else a process is spawned on Windows: without
/// this, every rust-analyzer start flashes a console window over the app.
pub(crate) fn no_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real binary names itself; the proxy's complaint, and an empty
    /// answer, are not it.
    #[test]
    fn only_a_binary_that_names_itself_is_rust_analyzer() {
        assert!(is_rust_analyzer_version(
            "rust-analyzer 1.98.0 (88d9e12ae 2026-08-18)\n"
        ));
        assert!(!is_rust_analyzer_version(""));
        assert!(!is_rust_analyzer_version(
            "error: 'rust-analyzer' is not installed for the toolchain 'stable-x86_64-pc-windows-msvc'\n"
        ));
    }

    /// The layout this exists for: a workspace whose firmware is excluded
    /// because it cross-compiles. Without the link, every file under it comes
    /// back "not included in any crates" and the editor is inert in exactly
    /// the half of the repository this workbench is for.
    #[test]
    fn an_excluded_firmware_crate_is_linked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"core\"]\nexclude = [\"firmware\"]\n",
        )
        .expect("root manifest");
        std::fs::create_dir(root.join("firmware")).expect("mkdir");
        std::fs::write(
            root.join("firmware/Cargo.toml"),
            "[package]\nname = \"fw\"\n",
        )
        .expect("firmware manifest");

        let linked = linked_projects(root);
        assert_eq!(linked.len(), 1, "{linked:?}");
        assert!(linked[0].ends_with("Cargo.toml"));
        assert!(linked[0].contains("firmware"));
    }

    /// An excluded directory that is not a crate — a fixture tree, a vendored
    /// copy — must not be handed to rust-analyzer as a project. It would fail
    /// to load and the failure reads as the server being broken.
    #[test]
    fn an_excluded_directory_with_no_manifest_is_not_linked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nexclude = [\"fixtures\"]\n",
        )
        .expect("root manifest");
        std::fs::create_dir(root.join("fixtures")).expect("mkdir");
        assert!(linked_projects(root).is_empty());
    }

    /// An ordinary project excludes nothing, and must be left entirely alone:
    /// naming `linkedProjects` at all changes how rust-analyzer discovers the
    /// workspace, so the option has to stay absent rather than empty.
    #[test]
    fn a_plain_workspace_links_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"solo\"\n").expect("manifest");
        assert!(linked_projects(root).is_empty());
        assert!(linked_projects(Path::new("/nowhere-at-all")).is_empty());
    }

    /// `cargo tauri dev` runs under the rustup shim, which exports the
    /// toolchain it picked for *rusty's* build. Passed on, rust-analyzer's
    /// own `cargo metadata` would read an esp-pinned project with stable.
    #[test]
    fn the_shims_toolchain_does_not_leak_into_the_server() {
        let command = command_for(Path::new("rust-analyzer"), Path::new("."));
        assert!(
            command
                .get_envs()
                .any(|(name, value)| name == "RUSTUP_TOOLCHAIN" && value.is_none()),
            "RUSTUP_TOOLCHAIN must be removed, not inherited: {:?}",
            command.get_envs().collect::<Vec<_>>(),
        );
    }
}
