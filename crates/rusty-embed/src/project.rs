//! Work out what an opened project is actually targeting.
//!
//! Embedded Rust spreads this across four files that can disagree with each
//! other, and when they do the compiler's complaint points at none of them:
//!
//! | File | What it decides |
//! |---|---|
//! | `Cargo.toml` | HAL crate, and usually the chip via an `esp-hal` feature |
//! | `.cargo/config.toml` | target triple, runner |
//! | `rust-toolchain.toml` | whether you get the Xtensa compiler |
//! | `memory.x` / `partitions.csv` | link-time layout |
//!
//! Detecting each independently and then *cross-checking* them is the point.
//! A chip feature saying `esp32c3` next to a target triple saying
//! `xtensa-esp32-none-elf` is a real and common mistake, and nothing in the
//! toolchain will tell you which of the two you meant.

use std::path::Path;

use crate::{
    chip,
    error::{Error, Result},
    model::{EmbeddedProject, Problem, Runtime, Severity, Vendor},
};

/// Crates whose presence identifies the framework in use.
const FRAMEWORK_CRATES: &[(&str, &str)] = &[
    ("esp-hal", "bare-metal HAL"),
    ("esp-hal-embassy", "embassy time driver for esp-hal"),
    ("esp-wifi", "Wi-Fi / BLE on bare metal"),
    ("esp-idf-hal", "ESP-IDF HAL (std)"),
    ("esp-idf-svc", "ESP-IDF services (std)"),
    ("esp-idf-sys", "ESP-IDF bindings (std)"),
    ("embassy-executor", "async executor"),
    ("embedded-hal", "portable peripheral traits"),
    ("defmt", "deferred formatting logs"),
    ("esp-backtrace", "panic handler and backtraces"),
    ("esp-println", "println over UART/JTAG"),
];

/// Inspect a project directory.
pub fn detect(root: &Path) -> Result<EmbeddedProject> {
    let manifest_path = root.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(Error::NotACargoProject(root.display().to_string()));
    }

    let manifest = read_toml(&manifest_path)?;
    let mut evidence = vec!["Cargo.toml".to_string()];

    let deps = collect_dependency_names(&manifest);
    let frameworks: Vec<String> = FRAMEWORK_CRATES
        .iter()
        .filter(|(name, _)| deps.iter().any(|d| d == name))
        .map(|(name, purpose)| format!("{name} — {purpose}"))
        .collect();

    let uses_defmt = deps.iter().any(|d| d == "defmt");
    let uses_embassy = deps.iter().any(|d| d.starts_with("embassy-"));

    // Runtime is decided by which HAL family is present, not by guessing.
    let runtime = if deps.iter().any(|d| d.starts_with("esp-idf-")) {
        Some(Runtime::EspIdf)
    } else if deps.iter().any(|d| d == "esp-hal") {
        Some(Runtime::BareMetal)
    } else {
        None
    };

    let (mut chip_id, mut chip_source) = chip_from_manifest(&manifest);

    let configured_target = read_build_target(root, &mut evidence)?;
    let configured_toolchain = read_toolchain_channel(root, &mut evidence)?;

    // Fall back to the target triple only when the manifest was silent, and
    // only when the triple names exactly one part.
    if chip_id.is_none()
        && let Some(target) = &configured_target
    {
        let candidates = chip::chips_for_target(target);
        if candidates.len() == 1 {
            chip_id = Some(candidates[0].id.clone());
            chip_source = Some(".cargo/config.toml target".to_string());
        }
    }

    for extra in ["memory.x", "partitions.csv", "sdkconfig.defaults", "build.rs"] {
        if root.join(extra).is_file() {
            evidence.push(extra.to_string());
        }
    }

    let mut project = EmbeddedProject {
        root: root.display().to_string(),
        chip: chip_id,
        chip_source,
        runtime,
        configured_target,
        configured_toolchain,
        frameworks,
        uses_defmt,
        uses_embassy,
        evidence,
        problems: Vec::new(),
    };
    project.problems = diagnose(&project);
    Ok(project)
}

/// Cross-check what the four files claim.
///
/// Ordered most-blocking first, because the panel shows them in order and the
/// first one is usually the cause of the rest.
fn diagnose(project: &EmbeddedProject) -> Vec<Problem> {
    let mut problems = Vec::new();

    let Some(chip_id) = &project.chip else {
        problems.push(Problem {
            severity: Severity::Blocking,
            title: "Target chip unknown".into(),
            detail: "No `esp-hal` chip feature and no recognisable target triple. \
                     Without a chip, rusty cannot pick a toolchain, flash, or size \
                     the binary."
                .into(),
            fix_command: None,
        });
        return problems;
    };

    let Some(chip) = chip::by_id(chip_id) else {
        problems.push(Problem {
            severity: Severity::Warning,
            title: format!("Unrecognised chip `{chip_id}`"),
            detail: "rusty does not have this part in its catalogue, so chip-specific \
                     checks are skipped. Flashing and building still work."
                .into(),
            fix_command: None,
        });
        return problems;
    };

    // The expected triple depends on the runtime; without one, both are
    // acceptable and there is nothing to check.
    if let (Some(configured), Some(runtime)) = (&project.configured_target, project.runtime) {
        match chip.target_for(runtime) {
            Some(expected) if expected != configured => {
                problems.push(Problem {
                    severity: Severity::Blocking,
                    title: "Target triple does not match the chip".into(),
                    detail: format!(
                        "`.cargo/config.toml` builds for `{configured}`, but {} with \
                         {} needs `{expected}`. The build will either fail to link or \
                         produce a binary for the wrong core.",
                        chip.name,
                        runtime.label()
                    ),
                    fix_command: Some(format!("# set target = \"{expected}\" in .cargo/config.toml")),
                });
            }
            None => {
                problems.push(Problem {
                    severity: Severity::Blocking,
                    title: format!("{} has no {} target", chip.name, runtime.label()),
                    detail: format!(
                        "There is no supported Rust target for {} on this part.",
                        runtime.label()
                    ),
                    fix_command: None,
                });
            }
            _ => {}
        }
    } else if project.configured_target.is_none() {
        problems.push(Problem {
            severity: Severity::Blocking,
            title: "No target configured".into(),
            detail: format!(
                "`.cargo/config.toml` sets no `[build] target`, so cargo will build \
                 for this machine instead of {}. The result will compile and then \
                 fail to do anything useful.",
                chip.name
            ),
            fix_command: Some(format!(
                "# add [build] target = \"{}\" to .cargo/config.toml",
                chip.bare_metal_target
            )),
        });
    }

    // The single most common first-build failure.
    if chip.needs_esp_toolchain() {
        match project.configured_toolchain.as_deref() {
            Some("esp") => {}
            Some(other) => problems.push(Problem {
                severity: Severity::Blocking,
                title: format!("{} needs the `esp` toolchain", chip.name),
                detail: format!(
                    "This part is {} and upstream rustc cannot emit code for it. \
                     `rust-toolchain.toml` pins `{other}`, which will fail with an \
                     unknown-target error. The `esp` toolchain ships a forked LLVM and \
                     is installed by espup.",
                    chip.arch.label()
                ),
                fix_command: Some("espup install".into()),
            }),
            None => problems.push(Problem {
                severity: Severity::Warning,
                title: format!("{} needs the `esp` toolchain, and none is pinned", chip.name),
                detail: "Builds will use whatever toolchain happens to be default. Pin \
                         it so the project builds the same way on every machine."
                    .into(),
                fix_command: Some("# add [toolchain] channel = \"esp\" to rust-toolchain.toml".into()),
            }),
        }
    } else if project.configured_toolchain.as_deref() == Some("esp") {
        problems.push(Problem {
            severity: Severity::Warning,
            title: format!("{} does not need the `esp` toolchain", chip.name),
            detail: format!(
                "This part is {}, which stock Rust supports. Pinning `esp` still works \
                 but forces everyone building this project to install espup.",
                chip.arch.label()
            ),
            fix_command: None,
        });
    }

    if project.uses_defmt {
        problems.push(Problem {
            severity: Severity::Info,
            title: "defmt logging detected".into(),
            detail: "The monitor will decode frames against this build's ELF. Reflash \
                     after changing log strings or the decoding drifts."
                .into(),
            fix_command: None,
        });
    }

    problems
}

// ─── file readers ────────────────────────────────────────────────────────────

/// Parse a TOML *document*.
///
/// `toml::Value`'s `FromStr` parses a single value, not a document, so it
/// rejects every real manifest at the first `[section]` header. `Table` is the
/// document type.
fn read_toml(path: &Path) -> Result<toml::Table> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.display().to_string(),
        source,
    })?;
    text.parse::<toml::Table>().map_err(|source| Error::Toml {
        path: path.display().to_string(),
        source,
    })
}

/// Dependency names from every section, including target-specific ones — an
/// embedded manifest routinely puts the HAL under
/// `[target.'cfg(target_arch = "riscv32")'.dependencies]`.
fn collect_dependency_names(manifest: &toml::Table) -> Vec<String> {
    let mut names = Vec::new();
    let mut push_table = |table: Option<&toml::Value>| {
        if let Some(toml::Value::Table(t)) = table {
            names.extend(t.keys().cloned());
        }
    };

    push_table(manifest.get("dependencies"));
    push_table(manifest.get("build-dependencies"));
    push_table(manifest.get("dev-dependencies"));

    if let Some(toml::Value::Table(targets)) = manifest.get("target") {
        for spec in targets.values() {
            push_table(spec.get("dependencies"));
        }
    }
    names
}

/// The chip named by an `esp-hal`-family feature.
///
/// These crates take the part as a feature (`features = ["esp32c3"]`), which
/// makes the manifest the most authoritative source available offline.
/// Vendors to consult, in the order their HAL crates are checked.
///
/// Each contributes its own list of crates that carry the part number as a
/// feature — see [`Vendor::chip_feature_crates`]. Which crate gets *cited*
/// matters: half a dozen `esp-*` crates take the same chip feature, and a user
/// told their chip came from `esp-backtrace` would go and edit the wrong line.
/// Plain alphabetical order reports exactly that, since `esp-backtrace` sorts
/// before `esp-hal`.
const VENDORS: &[Vendor] = &[Vendor::Espressif, Vendor::St];

fn chip_from_manifest(manifest: &toml::Table) -> (Option<String>, Option<String>) {
    let known: Vec<String> = chip::catalogue().into_iter().map(|c| c.id).collect();

    if let Some(deps) = manifest.get("dependencies")
        && let Some((id, source)) = chip_from_dependency_table(deps, &known)
    {
        return (Some(id), Some(source));
    }

    if let Some(toml::Value::Table(targets)) = manifest.get("target") {
        for spec in targets.values() {
            if let Some(deps) = spec.get("dependencies")
                && let Some((id, source)) = chip_from_dependency_table(deps, &known)
            {
                return (Some(id), Some(source));
            }
        }
    }
    (None, None)
}

fn chip_from_dependency_table(deps: &toml::Value, known: &[String]) -> Option<(String, String)> {
    let toml::Value::Table(deps) = deps else {
        return None;
    };

    // A dependency with no `features` key is not a failure, it is simply not
    // the one carrying the chip — keep looking.
    let scan = |crate_name: &str, spec: &toml::Value| -> Option<(String, String)> {
        let features = spec.get("features")?.as_array()?;
        features.iter().filter_map(|f| f.as_str()).find_map(|feature| {
            let normalized = chip::normalize(feature);
            known
                .contains(&normalized)
                .then(|| (normalized, format!("{crate_name} feature `{feature}`")))
        })
    };

    for vendor in VENDORS {
        for preferred in vendor.chip_feature_crates() {
            if let Some(spec) = deps.get(*preferred)
                && let Some(found) = scan(preferred, spec)
            {
                return Some(found);
            }
        }
    }

    // Anything else that happens to carry a known part number. Last resort, so
    // a HAL this build does not know about still gets the chip right.
    deps.iter().find_map(|(name, spec)| scan(name, spec))
}

/// `[build] target` from `.cargo/config.toml`, accepting the legacy
/// extension-less spelling that older templates still use.
fn read_build_target(root: &Path, evidence: &mut Vec<String>) -> Result<Option<String>> {
    for name in [".cargo/config.toml", ".cargo/config"] {
        let path = root.join(name);
        if !path.is_file() {
            continue;
        }
        evidence.push(name.to_string());
        let config = read_toml(&path)?;
        if let Some(target) = config
            .get("build")
            .and_then(|b| b.get("target"))
            .and_then(|t| t.as_str())
        {
            return Ok(Some(target.to_string()));
        }
        return Ok(None);
    }
    Ok(None)
}

fn read_toolchain_channel(root: &Path, evidence: &mut Vec<String>) -> Result<Option<String>> {
    for name in ["rust-toolchain.toml", "rust-toolchain"] {
        let path = root.join(name);
        if !path.is_file() {
            continue;
        }
        evidence.push(name.to_string());
        // The extension-less form is sometimes a bare channel name rather than
        // TOML, which is why this does not just parse and index.
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Read {
            path: path.display().to_string(),
            source,
        })?;
        if let Ok(value) = text.parse::<toml::Table>()
            && let Some(channel) = value
                .get("toolchain")
                .and_then(|t| t.get("channel"))
                .and_then(|c| c.as_str())
        {
            return Ok(Some(channel.to_string()));
        }
        let bare = text.trim();
        if !bare.is_empty() && !bare.contains('\n') {
            return Ok(Some(bare.to_string()));
        }
        return Ok(None);
    }
    Ok(None)
}
