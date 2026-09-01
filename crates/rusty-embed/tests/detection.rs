//! Detection and cross-checking, tested against real project layouts.
//!
//! Each case here is a mistake that actually happens and that the Rust
//! toolchain reports badly or not at all. The point of the panel is to name the
//! mistake before the build does, so these tests assert on *which* problem is
//! reported, not merely that something was.

use std::{fs, path::Path};

use rusty_embed::{
    model::{Runtime, Severity},
    project,
};
use tempfile::TempDir;

/// Build a project tree from `(relative path, contents)` pairs.
fn project_dir(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    for (path, contents) in files {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(&full, contents).expect("write");
    }
    dir
}

fn detect(dir: &Path) -> rusty_embed::EmbeddedProject {
    project::detect(dir).expect("detection should not fail on a valid manifest")
}

fn titles(project: &rusty_embed::EmbeddedProject) -> Vec<&str> {
    project.problems.iter().map(|p| p.title.as_str()).collect()
}

const C3_MANIFEST: &str = r#"
[package]
name = "blinky"
version = "0.1.0"
edition = "2021"

[dependencies]
esp-hal = { version = "0.23", features = ["esp32c3"] }
esp-backtrace = { version = "0.15", features = ["esp32c3", "panic-handler"] }
"#;

#[test]
fn a_well_formed_riscv_project_has_no_blocking_problems() {
    let dir = project_dir(&[
        ("Cargo.toml", C3_MANIFEST),
        (
            ".cargo/config.toml",
            "[build]\ntarget = \"riscv32imc-unknown-none-elf\"\n",
        ),
    ]);
    let project = detect(dir.path());

    assert_eq!(project.chip.as_deref(), Some("esp32c3"));
    assert_eq!(project.runtime, Some(Runtime::BareMetal));
    assert_eq!(
        project.configured_target.as_deref(),
        Some("riscv32imc-unknown-none-elf")
    );
    assert!(
        project
            .chip_source
            .as_deref()
            .is_some_and(|s| s.contains("esp-hal")),
        "the manifest feature should be cited as the source, got {:?}",
        project.chip_source
    );
    assert!(
        !project
            .problems
            .iter()
            .any(|p| p.severity == Severity::Blocking),
        "unexpected blocking problems: {:?}",
        titles(&project)
    );
}

/// The single most common first-build failure in embedded Rust: an Xtensa part
/// with a stock toolchain. rustc's own message never mentions espup.
#[test]
fn xtensa_without_the_esp_toolchain_is_blocking_and_names_espup() {
    let dir = project_dir(&[
        (
            "Cargo.toml",
            r#"
[package]
name = "blinky"
version = "0.1.0"
edition = "2021"

[dependencies]
esp-hal = { version = "0.23", features = ["esp32s3"] }
"#,
        ),
        (
            ".cargo/config.toml",
            "[build]\ntarget = \"xtensa-esp32s3-none-elf\"\n",
        ),
        ("rust-toolchain.toml", "[toolchain]\nchannel = \"stable\"\n"),
    ]);
    let project = detect(dir.path());

    assert_eq!(project.chip.as_deref(), Some("esp32s3"));
    let problem = project
        .problems
        .iter()
        .find(|p| p.title.contains("esp` toolchain"))
        .unwrap_or_else(|| panic!("no toolchain problem in {:?}", titles(&project)));

    assert_eq!(problem.severity, Severity::Blocking);
    assert_eq!(problem.fix_command.as_deref(), Some("espup install"));
    assert!(
        problem.detail.contains("Xtensa"),
        "the reason has to be stated, not just the fix: {}",
        problem.detail
    );
}

/// A RISC-V part pinned to the Xtensa toolchain still builds, so this must not
/// be blocking — but it forces every contributor to install espup for nothing.
#[test]
fn riscv_pinned_to_the_esp_toolchain_warns_without_blocking() {
    let dir = project_dir(&[
        ("Cargo.toml", C3_MANIFEST),
        (
            ".cargo/config.toml",
            "[build]\ntarget = \"riscv32imc-unknown-none-elf\"\n",
        ),
        ("rust-toolchain.toml", "[toolchain]\nchannel = \"esp\"\n"),
    ]);
    let project = detect(dir.path());

    let problem = project
        .problems
        .iter()
        .find(|p| p.title.contains("does not need"))
        .unwrap_or_else(|| panic!("no warning in {:?}", titles(&project)));
    assert_eq!(problem.severity, Severity::Warning);
}

/// Manifest and cargo config disagreeing about the part. The compiler will
/// complain about something else entirely.
#[test]
fn a_chip_target_mismatch_is_blocking_and_states_both_sides() {
    let dir = project_dir(&[
        ("Cargo.toml", C3_MANIFEST),
        (
            ".cargo/config.toml",
            "[build]\ntarget = \"xtensa-esp32-none-elf\"\n",
        ),
    ]);
    let project = detect(dir.path());

    let problem = project
        .problems
        .iter()
        .find(|p| p.title.contains("does not match"))
        .unwrap_or_else(|| panic!("no mismatch reported in {:?}", titles(&project)));

    assert_eq!(problem.severity, Severity::Blocking);
    assert!(
        problem.detail.contains("xtensa-esp32-none-elf")
            && problem.detail.contains("riscv32imc-unknown-none-elf"),
        "both the configured and the expected triple must appear: {}",
        problem.detail
    );
}

/// No `[build] target` at all: cargo silently builds for the host, produces a
/// binary, and the user wonders why nothing happens on the board.
#[test]
fn a_missing_target_is_blocking_because_the_build_still_succeeds() {
    let dir = project_dir(&[("Cargo.toml", C3_MANIFEST)]);
    let project = detect(dir.path());

    let problem = project
        .problems
        .iter()
        .find(|p| p.title.contains("No target configured"))
        .unwrap_or_else(|| panic!("not reported in {:?}", titles(&project)));
    assert_eq!(problem.severity, Severity::Blocking);
    assert!(problem.detail.contains("this machine"));
}

#[test]
fn an_esp_idf_project_is_recognised_as_std() {
    let dir = project_dir(&[
        (
            "Cargo.toml",
            r#"
[package]
name = "wifi-thing"
version = "0.1.0"
edition = "2021"

[dependencies]
esp-idf-svc = "0.51"
esp-idf-hal = "0.45"
"#,
        ),
        (
            ".cargo/config.toml",
            "[build]\ntarget = \"riscv32imc-esp-espidf\"\n",
        ),
        (
            "sdkconfig.defaults",
            "CONFIG_ESP_MAIN_TASK_STACK_SIZE=8000\n",
        ),
    ]);
    let project = detect(dir.path());

    assert_eq!(project.runtime, Some(Runtime::EspIdf));
    // The triple alone identifies the family, and C2/C3 share it — so falling
    // back to the triple must still land on a single chip only when it can.
    assert!(project.chip.is_none() || project.chip.as_deref() == Some("esp32c3"));
    assert!(project.evidence.iter().any(|e| e == "sdkconfig.defaults"));
    assert!(
        project
            .frameworks
            .iter()
            .any(|f| f.starts_with("esp-idf-svc"))
    );
}

/// Embedded manifests routinely gate the HAL behind a target cfg; detection
/// that only reads `[dependencies]` would see an empty project.
#[test]
fn target_gated_dependencies_are_still_found() {
    let dir = project_dir(&[
        (
            "Cargo.toml",
            r#"
[package]
name = "portable"
version = "0.1.0"
edition = "2021"

[target.'cfg(target_arch = "riscv32")'.dependencies]
esp-hal = { version = "0.23", features = ["esp32c6"] }
defmt = "0.3"
"#,
        ),
        (
            ".cargo/config.toml",
            "[build]\ntarget = \"riscv32imac-unknown-none-elf\"\n",
        ),
    ]);
    let project = detect(dir.path());

    assert_eq!(project.chip.as_deref(), Some("esp32c6"));
    assert_eq!(project.runtime, Some(Runtime::BareMetal));
    assert!(project.uses_defmt);
}

#[test]
fn a_bare_rust_toolchain_file_is_read_as_a_channel_name() {
    let dir = project_dir(&[
        ("Cargo.toml", C3_MANIFEST),
        (
            ".cargo/config.toml",
            "[build]\ntarget = \"riscv32imc-unknown-none-elf\"\n",
        ),
        ("rust-toolchain", "nightly-2026-01-15\n"),
    ]);
    let project = detect(dir.path());
    assert_eq!(
        project.configured_toolchain.as_deref(),
        Some("nightly-2026-01-15")
    );
}

#[test]
fn a_non_embedded_project_says_so_rather_than_guessing() {
    let dir = project_dir(&[(
        "Cargo.toml",
        "[package]\nname = \"webserver\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\naxum = \"0.8\"\n",
    )]);
    let project = detect(dir.path());

    assert!(project.chip.is_none());
    assert!(project.runtime.is_none());
    assert_eq!(titles(&project), vec!["Target chip unknown"]);
}

#[test]
fn a_directory_without_a_manifest_is_an_error_not_an_empty_result() {
    let dir = tempfile::tempdir().unwrap();
    let err = project::detect(dir.path()).unwrap_err().to_string();
    assert!(err.contains("Cargo.toml"), "{err}");
}

/// A project that meets C is named as such, with the file that proves each
/// claim — and a pure-Rust one says nothing at all rather than showing an
/// empty heading.
#[test]
fn c_interop_is_reported_with_its_evidence() {
    let dir = project_dir(&[
        (
            "Cargo.toml",
            r#"
[package]
name = "bridge"
version = "0.1.0"

[lib]
crate-type = ["staticlib"]

[dependencies]
esp-hal = { version = "0.23", features = ["esp32c3"] }

[build-dependencies]
cc = "1"
bindgen = "0.70"
"#,
        ),
        (
            "src/lib.rs",
            "#![no_std]
",
        ),
        (
            "csrc/driver.c",
            "int driver_init(void) { return 0; }
",
        ),
        (
            "csrc/driver.h",
            "int driver_init(void);
",
        ),
    ]);
    let project = detect(dir.path());
    let interop = &project.c_interop;

    assert!(!interop.is_empty());
    assert!(
        interop.via.iter().any(|v| v.starts_with("cc —")),
        "cc compiles the C in build.rs: {:?}",
        interop.via,
    );
    assert!(
        interop.via.iter().any(|v| v.starts_with("bindgen —")),
        "bindgen binds its headers: {:?}",
        interop.via,
    );
    assert!(
        interop.exports_to_c,
        "a staticlib is a crate C links against, which is the other direction",
    );
    assert_eq!(interop.sources, 2, "both csrc files counted");
    assert!(
        interop.evidence.iter().any(|e| e == "csrc/"),
        "every claim names the file that proves it: {:?}",
        interop.evidence,
    );

    let plain = project_dir(&[
        (
            "Cargo.toml",
            "[package]
name = \"plain\"
version = \"0.1.0\"

[dependencies]
esp-hal = \"0.23\"
",
        ),
        (
            "src/main.rs",
            "fn main() {}
",
        ),
    ]);
    assert!(
        detect(plain.path()).c_interop.is_empty(),
        "a pure-Rust project claims no C interop",
    );
}
