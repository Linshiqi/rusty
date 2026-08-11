//! `rusty` — the workbench without the window.
//!
//! Everything the desktop app shows is computed here too. Keeping a real CLI
//! from day one is what makes CI and team integration a wiring job rather than
//! a rewrite — and `rusty check --json` is the thing to paste into a bug report
//! when someone's board will not build.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rusty_core::{FeatureSelection, Workspace, WorkspaceReport};
use rusty_embed::{
    EmbeddedProject, MemoryReport, Problem, Severity, ToolchainReport, catalog::Catalog, device,
    memory, project, toolchain,
};

#[derive(Parser)]
#[command(
    name = "rusty",
    version,
    about = "Embedded Rust workbench: projects, toolchains, boards, and binary size"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Why this project will or will not build: chip, toolchain, and every
    /// mismatch between them.
    ///
    /// The first thing to run when something is wrong, and the thing to paste
    /// into a bug report. Exits non-zero if anything blocking was found, so it
    /// drops straight into CI.
    Check {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Parts and boards rusty knows about, including any the project adds.
    Catalog {
        /// Show boards instead of chips.
        #[arg(long)]
        boards: bool,
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Serial ports and debug probes currently attached.
    Devices {
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Where a built firmware's bytes went, by section and by crate.
    Size {
        /// The linked ELF, e.g. target/riscv32imc-unknown-none-elf/release/blinky
        elf: PathBuf,
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Cargo dependency health: duplicates, direct vs transitive, build scripts.
    Deps {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// What a feature selection costs, relative to the package's defaults.
    Features {
        package: String,
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long, value_delimiter = ',')]
        features: Vec<String>,
        #[arg(long)]
        no_default_features: bool,
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Check { path, json } => {
            let detected = project::detect(&path)
                .with_context(|| format!("inspecting {}", path.display()))?;
            let toolchain = toolchain::report(Some(&detected));

            if json {
                emit(&serde_json::json!({
                    "project": detected,
                    "toolchain": toolchain,
                }))?;
            } else {
                print_check(&detected, &toolchain);
            }

            // Blocking problems are build failures waiting to happen, so CI
            // should hear about them as a non-zero exit rather than having to
            // grep the output.
            let blocking = detected
                .problems
                .iter()
                .chain(toolchain.problems.iter())
                .filter(|p| p.severity == Severity::Blocking)
                .count();
            if blocking > 0 {
                std::process::exit(1);
            }
        }

        Command::Catalog { boards, path, json } => {
            let catalog = Catalog::load(Some(&path));
            report_catalog_problems(&catalog);

            if json && boards {
                emit(&catalog.boards())?;
            } else if json {
                emit(&catalog.chips())?;
            } else if boards {
                for board in catalog.boards() {
                    let flash = board
                        .flash_bytes
                        .map(|b| format!("{} flash", human(b as u64)))
                        .unwrap_or_else(|| "flash unknown".into());
                    println!(
                        "  {:<28} {:<10} {:<14} [{}]",
                        board.name,
                        board.chip,
                        flash,
                        board.source.label()
                    );
                }
            } else {
                for chip in catalog.chips() {
                    println!(
                        "  {:<10} {:<24} {:<14} {}",
                        chip.id,
                        chip.name,
                        chip.arch.label(),
                        chip.bare_metal_target
                    );
                }
            }
        }

        Command::Devices { path, json } => {
            let catalog = Catalog::load(Some(&path));
            let ports = device::list_serial_ports(&catalog);
            let probes = device::list_probes();

            if json {
                emit(&serde_json::json!({ "ports": ports, "probes": probes }))?;
            } else {
                if ports.is_empty() {
                    println!("no serial ports");
                }
                for port in &ports {
                    // The board name is what the user recognises; the bridge
                    // chip is the fallback when nothing in the catalogue matches.
                    let what = if !port.boards.is_empty() {
                        port.boards.join(" / ")
                    } else {
                        port.bridge.clone().unwrap_or_else(|| "unknown".into())
                    };
                    println!("  {:<12} {}", port.name, what);
                }
                for probe in &probes {
                    println!("  probe        {}", probe.description);
                }
            }
        }

        Command::Size { elf, path, json } => {
            let chip = project::detect(&path).ok().and_then(|p| p.chip);
            let report = memory::analyze(&elf, chip.as_deref())
                .with_context(|| format!("reading {}", elf.display()))?;
            if json {
                emit(&report)?;
            } else {
                print_size(&report);
            }
        }

        Command::Deps { path, json } => {
            let workspace = Workspace::load(&path)
                .with_context(|| format!("loading workspace at {}", path.display()))?;
            let report = workspace.report()?;
            if json {
                emit(&report)?;
            } else {
                print_deps(&report);
            }
        }

        Command::Features {
            package,
            path,
            features,
            no_default_features,
            json,
        } => {
            let workspace = Workspace::load(&path)
                .with_context(|| format!("loading workspace at {}", path.display()))?;
            let selection = FeatureSelection {
                package,
                features,
                default_features: !no_default_features,
            };
            let impact = workspace.feature_impact(&selection)?;
            let rows = workspace.feature_rows(&selection)?;

            if json {
                emit(&serde_json::json!({ "impact": impact, "rows": rows }))?;
            } else {
                print_features(&impact, &rows);
            }
        }
    }
    Ok(())
}

fn emit<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn report_catalog_problems(catalog: &Catalog) {
    // To stderr, so `--json` output stays machine-readable while a broken board
    // file still gets noticed.
    for problem in catalog.problems() {
        eprintln!("warning: {} — {}", problem.path, problem.detail);
    }
}

fn human(bytes: u64) -> String {
    const KB: u64 = 1024;
    match bytes {
        b if b >= KB * KB => format!("{:.1} MB", b as f64 / (KB * KB) as f64),
        b if b >= KB => format!("{:.1} KB", b as f64 / KB as f64),
        b => format!("{b} B"),
    }
}

// ─── rendering ───────────────────────────────────────────────────────────────

fn print_check(project: &EmbeddedProject, toolchain: &ToolchainReport) {
    println!("{}", project.root);
    println!(
        "chip {} | {} | target {} | toolchain {}",
        project.chip.as_deref().unwrap_or("unknown"),
        project
            .runtime
            .map(|r| r.label())
            .unwrap_or("runtime unknown"),
        project.configured_target.as_deref().unwrap_or("unset"),
        project.configured_toolchain.as_deref().unwrap_or("unpinned"),
    );
    if let Some(source) = &project.chip_source {
        println!("  (chip from {source})");
    }

    let problems: Vec<&Problem> = project
        .problems
        .iter()
        .chain(toolchain.problems.iter())
        .collect();

    if problems.is_empty() {
        println!("\nno problems found");
        return;
    }

    println!();
    for problem in problems {
        let tag = match problem.severity {
            Severity::Blocking => "BLOCKING",
            Severity::Warning => "warning ",
            Severity::Info => "note    ",
        };
        println!("{tag}  {}", problem.title);
        // Indented so a wall of detail stays scannable while remaining
        // copy-pasteable into an issue.
        for line in wrap(&problem.detail, 74) {
            println!("          {line}");
        }
        if let Some(fix) = &problem.fix_command {
            println!("          $ {fix}");
        }
        println!();
    }
}

fn print_size(report: &MemoryReport) {
    let totals = &report.totals;
    println!("{}", report.elf_path);
    print!("flash {}", human(totals.flash_bytes));
    match totals.ram_fraction() {
        Some(fraction) => println!(
            "   ram {} of {} ({:.0}% static)",
            human(totals.ram_bytes),
            human(totals.ram_capacity.unwrap_or(0) as u64),
            fraction * 100.0
        ),
        None => println!("   ram {}", human(totals.ram_bytes)),
    }

    println!("\nSECTIONS");
    for section in report.sections.iter().take(10) {
        println!(
            "  {:<20} {:>10}  {}",
            section.name,
            human(section.size),
            section.kind.label()
        );
    }

    println!("\nBY CRATE");
    for krate in report.crates.iter().take(15) {
        println!(
            "  {:<24} {:>10}   code {:>9}  bss {:>9}",
            krate.name,
            human(krate.total),
            human(krate.code),
            human(krate.bss)
        );
    }
    if report.unattributed_bytes > 0 {
        println!(
            "  {:<24} {:>10}   (C, assembly, ROM stubs)",
            "unattributed",
            human(report.unattributed_bytes)
        );
    }
}

fn print_deps(report: &WorkspaceReport) {
    let v = &report.vitals;
    println!("{}  {}", report.workspace.name, report.workspace.root);
    println!(
        "{} workspace crates | {} deps ({} direct) | {} duplicate groups | {} build scripts",
        v.workspace_crates, v.resolved_deps, v.direct_deps, v.duplicate_groups, v.build_scripts
    );

    if report.duplicates.is_empty() {
        println!("\nno duplicate versions");
        return;
    }
    println!("\nDUPLICATES");
    for group in &report.duplicates {
        println!(
            "  {}  [{}]",
            group.name,
            if group.unifiable {
                "unifiable"
            } else {
                "not unifiable"
            }
        );
        for version in &group.versions {
            println!("    {}", version.version);
            for source in version.pulled_by.iter().take(3) {
                println!(
                    "      <- {} {} wants {}{}",
                    source.package,
                    source.version,
                    source.req,
                    if source.is_workspace_member { "  [yours]" } else { "" }
                );
            }
        }
    }
}

fn print_features(impact: &rusty_core::FeatureImpact, rows: &[rusty_core::FeatureRow]) {
    println!(
        "{}: {} crates ({:+} vs default {})",
        impact.package, impact.resolved_crates, impact.delta_crates, impact.baseline_crates
    );

    if !impact.removed.is_empty() {
        println!("\nremoved ({}):", impact.removed.len());
        for name in impact.removed.iter().take(12) {
            println!("  - {name}");
        }
    }
    if !impact.added.is_empty() {
        println!("\nadded ({}):", impact.added.len());
        for name in impact.added.iter().take(12) {
            println!("  + {name}");
        }
    }

    if rows.is_empty() {
        return;
    }
    println!("\nFEATURES");
    let width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
    for row in rows {
        println!(
            "  [{}] {:width$}  {:>+5} crates if flipped{}",
            if row.enabled { "x" } else { " " },
            row.name,
            row.marginal_crates,
            if row.in_default { "  (default)" } else { "" },
            width = width
        );
    }
}

/// Break a paragraph at word boundaries.
///
/// Hand-rolled rather than pulling a crate: it is fifteen lines, and the CLI's
/// dependency list is something rusty itself would complain about.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
