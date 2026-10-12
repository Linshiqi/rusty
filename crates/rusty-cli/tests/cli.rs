//! The CLI's contracts, driven through the built binary: the exit code CI keys
//! on, and the JSON shapes a bug report is pasted from. Neither had a test,
//! so either could have drifted with nothing to say so.

use std::{path::PathBuf, process::Command};

fn rusty() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rusty-cli"))
}

/// A real Cargo workspace with no chip in it — rusty-core's own fixture.
fn host_workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("rusty-core")
        .join("tests")
        .join("fixtures")
        .join("feature-lab")
}

/// `check` exits non-zero when something blocks the build, and its JSON
/// carries both halves of the diagnosis — the project and the toolchain —
/// because that is the payload the desktop app renders and the one to paste
/// into an issue.
#[test]
fn check_exits_non_zero_on_a_blocking_problem_and_emits_both_halves() {
    let output = rusty()
        .args(["check", "--json"])
        .arg(host_workspace())
        .output()
        .expect("run rusty-cli");
    assert_eq!(
        output.status.code(),
        Some(1),
        "a host workspace names no chip, which blocks an embedded build; stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON on stdout");
    assert!(json.get("project").is_some(), "{json}");
    assert!(json.get("toolchain").is_some(), "{json}");
    let problems = json["project"]["problems"].as_array().expect("problems");
    assert!(
        problems.iter().any(|p| {
            p["severity"]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case("blocking"))
        }),
        "the missing chip is the blocking problem: {problems:?}",
    );
}

/// The catalogue is the data users extend, so its JSON is a contract too.
#[test]
fn the_catalogue_lists_the_built_in_parts_as_json() {
    let output = rusty()
        .args(["catalog", "--json"])
        .output()
        .expect("run rusty-cli");
    assert!(output.status.success());
    let chips: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON on stdout");
    assert!(
        chips
            .as_array()
            .expect("an array of chips")
            .iter()
            .any(|chip| chip["id"] == "esp32c3"),
        "{chips}"
    );
}

/// `size` takes a project directory now, not only an ELF path — and when the
/// project has never been built it says so rather than reading nothing.
#[test]
fn size_on_an_unbuilt_project_says_there_is_nothing_to_measure() {
    let output = rusty()
        .arg("size")
        .arg(host_workspace())
        .output()
        .expect("run rusty-cli");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no built firmware"), "{stderr}");
}
