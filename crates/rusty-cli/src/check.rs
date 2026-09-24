//! `check` and `size`: what the project builds for and whether it can, and
//! where a built image's bytes went.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusty_embed::{
    EmbeddedProject, MemoryReport, Problem, Severity, ToolchainReport, memory, project, toolchain,
};

use super::{emit, human};

/// `rusty check`: the project's chip and toolchain, every mismatch between
/// them, and a non-zero exit when anything blocks the build.
pub(crate) fn check(path: &Path, json: bool) -> Result<()> {
    let detected =
        project::detect(path).with_context(|| format!("inspecting {}", path.display()))?;
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
    Ok(())
}

/// `rusty size`: where a built firmware's bytes went, by section and by
/// crate.
pub(crate) fn size(elf: PathBuf, path: PathBuf, json: bool) -> Result<()> {
    // A directory names the project as well as the build to look in;
    // a file is the build, and `--path` names the project.
    let (elf, project_dir) = if elf.is_dir() {
        (None, elf)
    } else {
        (Some(elf), path)
    };
    // Where the firmware is built, which in the standard layout is the
    // excluded firmware crate: the chip is known there, and the image is
    // under its `target/`, not the opened directory's.
    let firmware_dir = project::firmware_root(&project_dir);
    let chip = project::detect(&firmware_dir).ok().and_then(|p| p.chip);
    let elf = match elf {
        Some(elf) => elf,
        None => rusty_embed::firmware::newest_in_project(&project_dir)
            .map(|firmware| PathBuf::from(firmware.path))
            .with_context(|| {
                format!(
                    "no built firmware found under {}: build first, or name the ELF",
                    firmware_dir.join("target").display()
                )
            })?,
    };
    let report = memory::analyze(&elf, chip.as_deref())
        .with_context(|| format!("reading {}", elf.display()))?;
    if json {
        emit(&report)?;
    } else {
        print_size(&report);
    }
    Ok(())
}

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
