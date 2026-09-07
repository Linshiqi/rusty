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
        /// — or a project directory, whose newest firmware for its configured
        /// target is analysed, the way the desktop app picks one.
        elf: PathBuf,
        /// The project the chip is read from, when `elf` is a file.
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },

    /// Where the project's builds went on disk, and what of it is stale:
    /// artifacts of dependency versions the lockfile no longer resolves, of
    /// packages no longer in the graph, and idle incremental caches.
    Disk {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// An incremental cache untouched for this many days counts as idle.
        #[arg(long, default_value_t = 7)]
        idle_days: u32,
        /// Keep this many of each crate's newest incremental caches; older
        /// ones are superseded.
        #[arg(long, default_value_t = 4)]
        keep_variants: u32,
        #[arg(long)]
        json: bool,
    },

    /// Remove the stale artifacts `disk` lists. Prints what would go and
    /// stops there unless `--apply` is given; nothing a build holds the lock
    /// on is touched either way.
    Sweep {
        #[arg(default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value_t = 7)]
        idle_days: u32,
        /// Keep this many of each crate's newest incremental caches; older
        /// ones are superseded.
        #[arg(long, default_value_t = 4)]
        keep_variants: u32,
        /// Actually remove. Without it this is a dry run.
        #[arg(long)]
        apply: bool,
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
            let detected =
                project::detect(&path).with_context(|| format!("inspecting {}", path.display()))?;
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
            // A directory names the project as well as the build to look in;
            // a file is the build, and `--path` names the project.
            let (elf, project_dir) = if elf.is_dir() {
                (None, elf)
            } else {
                (Some(elf), path)
            };
            let detected = project::detect(&project_dir).ok();
            let chip = detected.as_ref().and_then(|p| p.chip.clone());
            let elf = match elf {
                Some(elf) => elf,
                None => {
                    let configured = detected
                        .as_ref()
                        .and_then(|p| p.configured_target.as_deref());
                    rusty_embed::firmware::newest(&project_dir, configured)
                        .map(|firmware| PathBuf::from(firmware.path))
                        .with_context(|| {
                            format!(
                                "no built firmware found under {}: build first, or name the ELF",
                                project_dir.join("target").display()
                            )
                        })?
                }
            };
            let report = memory::analyze(&elf, chip.as_deref())
                .with_context(|| format!("reading {}", elf.display()))?;
            if json {
                emit(&report)?;
            } else {
                print_size(&report);
            }
        }

        Command::Disk {
            path,
            idle_days,
            keep_variants,
            json,
        } => {
            let (target_dir, current) = disk_context(&path);
            let scan = rusty_core::disk::scan(
                &target_dir,
                &path,
                &current,
                rusty_core::disk::ScanOptions {
                    idle_days,
                    keep_variants,
                },
            );
            if json {
                emit(&scan.report)?;
            } else {
                print_disk(&scan);
            }
        }

        Command::Sweep {
            path,
            idle_days,
            keep_variants,
            apply,
        } => {
            let (target_dir, current) = disk_context(&path);
            let policy = rusty_core::SweepPolicy {
                idle_days: Some(idle_days),
                ..rusty_core::SweepPolicy::default()
            };
            if apply {
                let report = rusty_core::disk::sweep(&target_dir, &path, &current, &policy)?;
                println!(
                    "removed {} items, {}",
                    report.removed_items,
                    human(report.removed_bytes)
                );
                for tree in &report.locked {
                    eprintln!("skipped {tree}: a build holds its lock");
                }
                for failed in &report.failed {
                    eprintln!("failed: {failed}");
                }
            } else {
                let scan = rusty_core::disk::scan(
                    &target_dir,
                    &path,
                    &current,
                    rusty_core::disk::ScanOptions {
                        idle_days,
                        keep_variants,
                    },
                );
                let stale = scan.stale_paths();
                let total: u64 = stale.iter().map(|(_, b, _)| *b).sum();
                for (stale_path, bytes, reason) in stale.iter().take(40) {
                    println!(
                        "  {:>10}  {:<12} {}",
                        human(*bytes),
                        reason_word(reason),
                        stale_path.display()
                    );
                }
                if stale.len() > 40 {
                    println!("  … and {} more", stale.len() - 40);
                }
                println!(
                    "{} in {} items would be removed; run with --apply to remove them",
                    human(total),
                    stale.len()
                );
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
        b if b >= KB * KB * KB => format!("{:.1} GB", b as f64 / (KB * KB * KB) as f64),
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
        project
            .configured_toolchain
            .as_deref()
            .unwrap_or("unpinned"),
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
                    if source.is_workspace_member {
                        "  [yours]"
                    } else {
                        ""
                    }
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

/// The build directory and the yardstick for a `disk` or `sweep` run: from
/// the workspace when it loads, and a bare `target/` with an empty yardstick
/// when it does not — nothing is then judged stale but idle caches, and the
/// report's warnings say so.
fn disk_context(path: &std::path::Path) -> (PathBuf, rusty_core::disk::Current) {
    match Workspace::load(path) {
        Ok(workspace) => (workspace.target_directory(), workspace.current()),
        Err(error) => {
            eprintln!("note: {error}; dependency artifacts are not judged");
            (path.join("target"), rusty_core::disk::Current::default())
        }
    }
}

fn reason_word(reason: &rusty_core::StaleReason) -> &'static str {
    match reason {
        rusty_core::StaleReason::VersionGone { .. } => "old version",
        rusty_core::StaleReason::PackageGone { .. } => "dropped",
        rusty_core::StaleReason::Idle { .. } => "idle",
        rusty_core::StaleReason::Superseded { .. } => "superseded",
    }
}

fn print_disk(scan: &rusty_core::disk::Scan) {
    let report = &scan.report;
    if let Some(volume) = report.volume {
        println!(
            "{}: {} free of {}",
            report.target_dir,
            human(volume.free_bytes),
            human(volume.total_bytes)
        );
    }
    if !report.exists {
        println!("no build directory yet");
        return;
    }
    println!(
        "{} in {} files{}",
        human(report.total_bytes),
        report.files,
        if report.shared {
            " (shared build directory)"
        } else {
            ""
        }
    );
    for tree in &report.trees {
        let stale: u64 = tree.groups.iter().map(|g| g.stale_bytes).sum();
        println!(
            "  {:<44} {:>10}  stale {:>10}{}",
            match &tree.triple {
                Some(triple) => format!("{triple}/{}", tree.profile),
                None => tree.profile.clone(),
            },
            human(tree.bytes),
            human(stale),
            if tree.locked { "  (building)" } else { "" }
        );
        for group in &tree.groups {
            let why: Vec<String> = group
                .stale_by_reason
                .iter()
                .map(|s| format!("{} {}", s.reason, human(s.bytes)))
                .collect();
            println!(
                "      {:<16} {:>10}  {}",
                format!("{:?}", group.kind).to_lowercase(),
                human(group.bytes),
                why.join(", ")
            );
        }
    }
    for extra in &report.extras {
        println!(
            "  {:<44} {:>10}  {}",
            extra.label,
            human(extra.bytes),
            extra.path
        );
    }
    if !report.cargo_home.is_empty() {
        println!("cargo home:");
        for item in &report.cargo_home {
            println!("  {:<44} {:>10}", item.label, human(item.bytes));
        }
    }
    if report.debuginfo_bytes > 0 {
        println!(
            "debug symbols beside binaries: {}",
            human(report.debuginfo_bytes)
        );
    }
    for warning in &report.warnings {
        eprintln!("note: {warning}");
    }
    let stale_total: u64 = scan.stale_paths().iter().map(|(_, b, _)| *b).sum();
    println!(
        "stale in total: {} — `rusty sweep` removes it",
        human(stale_total)
    );
}
