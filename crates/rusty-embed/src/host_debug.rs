//! Debugging a test on this machine, under gdb.
//!
//! The simulator's debug run has one binary and one target, both chosen by
//! the run that built the image. A host test has neither. `cargo test` builds
//! one executable per test target — the library's, each integration test's,
//! each binary's — and which of them holds the test named in the editor is
//! not something the file's path says reliably. So the binaries are asked: a
//! test executable lists what it holds (`<exe> <filter> --list`), and a filter
//! that lists a test in exactly one of them names the one to run. Two is a
//! question this module refuses to answer by picking, and none is the filter
//! that would make `cargo test` exit zero having run nothing.
//!
//! Whether gdb can read the binary at all is a property of the *target*, not
//! of gdb: an msvc build carries its debug information in a PDB, which gdb
//! does not read, so every breakpoint would land nowhere and every stop would
//! show a bare address. That is refused with the reason, before the build.

use std::path::{Path, PathBuf};

use crate::{Error, Result, model::CommandPlan, process};

/// The build the dock shows: every test target, compiled and not run.
///
/// No filter and no package: the filter names a test, not a target, and
/// cargo would build everything anyway. `--no-run` is the whole point — the
/// running is gdb's.
pub fn build_plan() -> CommandPlan {
    CommandPlan {
        program: "cargo".to_string(),
        args: vec!["test".to_string(), "--no-run".to_string()],
        display: "cargo test --no-run".to_string(),
        rationale: "Builds the test binaries so gdb can read one".to_string(),
        warning: None,
    }
}

/// The same build again, answered as JSON — instant, since nothing changed —
/// which is the only way cargo says *where* it put each test executable.
pub fn built_json(root: &Path) -> Result<String> {
    let output = process::command("cargo")
        .args(["test", "--no-run", "--message-format=json"])
        .current_dir(root)
        .output()
        .map_err(|source| Error::Spawn {
            tool: "cargo".to_string(),
            source,
        })?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The test executables in cargo's JSON output — one message per line, and
/// the ones that matter are `compiler-artifact` messages built with the test
/// profile whose `executable` is a path rather than `null` (a library's
/// non-test build has none).
pub fn test_executables(json_lines: &str) -> Vec<PathBuf> {
    json_lines
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| message["profile"]["test"] == true)
        .filter_map(|message| message["executable"].as_str().map(PathBuf::from))
        .collect()
}

/// Whether `<exe> <filter> --list` named a test: libtest prints one
/// `path::to::test: test` line per match, then a count. A binary with no match
/// prints only the count.
pub fn lists_a_test(listing: &str) -> bool {
    listing
        .lines()
        .any(|line| line.trim_end().ends_with(": test"))
}

/// The one executable among `built` holding a test that `filter` matches.
///
/// Asked of each binary rather than guessed from the file's path, because the
/// mapping from a source file to a test target is cargo's private knowledge:
/// `src/lib.rs` tests live in the library binary, `tests/x.rs` in its own,
/// and a `#[path]` attribute can put a module anywhere.
pub fn binary_holding(built: &[PathBuf], filter: &str) -> Result<PathBuf> {
    let mut holding = Vec::new();
    for exe in built {
        let output = process::command(exe)
            .args([filter, "--list"])
            .output()
            .map_err(|source| Error::Spawn {
                tool: exe.display().to_string(),
                source,
            })?;
        if lists_a_test(&String::from_utf8_lossy(&output.stdout)) {
            holding.push(exe.clone());
        }
    }
    match holding.len() {
        1 => Ok(holding.remove(0)),
        0 => Err(Error::Refused {
            detail: format!(
                "No test binary lists a test matching `{filter}`. `cargo test {filter}` would \
                 exit successfully having run nothing, so nothing is started. Check that the \
                 test compiles into one of: {}.",
                names(built),
            ),
        }),
        _ => Err(Error::Refused {
            detail: format!(
                "`{filter}` matches tests in {} binaries ({}), and running one would silently \
                 skip the others. Qualify the name with its module so exactly one holds it.",
                holding.len(),
                names(&holding),
            ),
        }),
    }
}

fn names(paths: &[PathBuf]) -> String {
    let named: Vec<String> = paths
        .iter()
        .filter_map(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();
    if named.is_empty() {
        "no test binaries at all".to_string()
    } else {
        named.join(", ")
    }
}

/// Why gdb cannot debug what this toolchain builds, if it cannot.
///
/// The msvc target's debug information is a PDB. gdb reads DWARF; handed a
/// PDB build it loads, sets breakpoints that never hit and shows addresses
/// where lines should be — a session that looks alive and answers nothing.
pub fn gdb_reads(host: &str) -> Result<()> {
    if host.ends_with("-msvc") {
        return Err(Error::Refused {
            detail: format!(
                "This machine's Rust builds for {host}, whose debug information is a PDB \
                 that gdb cannot read: breakpoints would land nowhere and every stop would \
                 show a bare address. Debug host tests on Linux or macOS, or build with the \
                 x86_64-pc-windows-gnu target and a MinGW gdb. rusty will not switch a \
                 project's target on its own."
            ),
        });
    }
    Ok(())
}

/// The host toolchain's target triple, as `rustc -vV` reports it — asked in
/// the project, so a `rust-toolchain.toml` pin answers rather than the default.
pub fn host_triple(root: &Path) -> Result<String> {
    let output = process::command("rustc")
        .arg("-vV")
        .current_dir(root)
        .output()
        .map_err(|source| Error::Spawn {
            tool: "rustc".to_string(),
            source,
        })?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_string())
        .ok_or_else(|| Error::Refused {
            detail: "`rustc -vV` did not report a host triple, so whether gdb can read \
                     what it builds cannot be said."
                .to_string(),
        })
}

/// A gdb for this machine's own binaries. The chips' debuggers —
/// `riscv32-esp-elf-gdb`, `xtensa-esp32-elf-gdb` — are found the same way and
/// are not it: neither can debug an x86-64 process.
pub fn host_gdb() -> Option<PathBuf> {
    crate::tools::find("gdb")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARGO_JSON: &str = concat!(
        r#"{"reason":"compiler-artifact","package_id":"p","target":{"name":"velo"},"profile":{"test":false},"executable":null}"#,
        "\n",
        r#"{"reason":"compiler-artifact","package_id":"p","target":{"name":"velo"},"profile":{"test":true},"executable":"/t/debug/deps/velo-1a2b"}"#,
        "\n",
        r#"{"reason":"compiler-artifact","package_id":"p","target":{"name":"cli"},"profile":{"test":true},"executable":"/t/debug/deps/cli-3c4d"}"#,
        "\n",
        r#"{"reason":"compiler-message","message":{"rendered":"warning: unused"}}"#,
        "\n",
        r#"{"reason":"build-finished","success":true}"#,
        "\n",
    );

    /// The library's ordinary build has no executable and the message stream
    /// carries diagnostics and a footer; only the test-profile artifacts
    /// with a path are binaries gdb could run.
    #[test]
    fn only_test_artifacts_with_a_path_are_executables() {
        let found = test_executables(CARGO_JSON);
        assert_eq!(
            found,
            vec![
                PathBuf::from("/t/debug/deps/velo-1a2b"),
                PathBuf::from("/t/debug/deps/cli-3c4d"),
            ]
        );
    }

    /// A line that is not JSON — cargo's own progress if colour leaked, a
    /// panic — is skipped, not a reason to find nothing.
    #[test]
    fn a_non_json_line_is_skipped() {
        let mixed = format!("   Compiling velo v0.1.0\n{CARGO_JSON}");
        assert_eq!(test_executables(&mixed).len(), 2);
    }

    #[test]
    fn a_listing_with_a_test_line_holds_the_test_and_a_bare_count_does_not() {
        assert!(lists_a_test(
            "throttle::tests::window_allows_until_full: test\n\n1 test, 0 benchmarks\n"
        ));
        assert!(!lists_a_test("\n0 tests, 0 benchmarks\n"));
        // A benchmark is not a test, and the count line is not a test either.
        assert!(!lists_a_test(
            "bench::speed: benchmark\n\n0 tests, 1 benchmark\n"
        ));
    }

    /// The refusal names the target and says why, so the person reading it
    /// learns the fact rather than "debugging failed".
    #[test]
    fn an_msvc_host_is_refused_by_name_and_a_gnu_one_is_not() {
        let refused = gdb_reads("x86_64-pc-windows-msvc").expect_err("PDB is not DWARF");
        let text = refused.to_string();
        assert!(text.contains("x86_64-pc-windows-msvc"), "{text}");
        assert!(text.contains("PDB"), "{text}");
        assert!(gdb_reads("x86_64-pc-windows-gnu").is_ok());
        assert!(gdb_reads("x86_64-unknown-linux-gnu").is_ok());
        assert!(gdb_reads("aarch64-apple-darwin").is_ok());
    }

    /// Nothing built, nothing to ask: the refusal says so rather than
    /// listing an empty set as if the filter were at fault.
    #[test]
    fn no_binaries_is_a_refusal_that_says_so() {
        let refused = binary_holding(&[], "anything").expect_err("nothing to run");
        assert!(refused.to_string().contains("no test binaries at all"));
    }
}
