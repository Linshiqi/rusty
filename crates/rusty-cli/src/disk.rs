//! `disk` and `sweep`: the build directory measured, and what is stale in
//! it removed.

use std::path::{Path, PathBuf};

use anyhow::Result;
use rusty_core::Workspace;

use super::{emit, human};

/// `rusty disk`: the build directory's size, and what of it is stale.
pub(crate) fn disk(path: &Path, idle_days: u32, keep_variants: u32, json: bool) -> Result<()> {
    let (target_dir, current) = disk_context(path);
    let scan = rusty_core::disk::scan(
        &target_dir,
        path,
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
    Ok(())
}

/// `rusty sweep`: what `disk` calls stale, listed — and removed with
/// `--apply`.
pub(crate) fn sweep(path: &Path, idle_days: u32, keep_variants: u32, apply: bool) -> Result<()> {
    let (target_dir, current) = disk_context(path);
    let policy = rusty_core::SweepPolicy {
        idle_days: Some(idle_days),
        keep_variants,
        ..rusty_core::SweepPolicy::default()
    };
    if apply {
        let report = rusty_core::disk::sweep(&target_dir, path, &current, &policy)?;
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
            path,
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
    Ok(())
}

/// The build directory and the yardstick for a `disk` or `sweep` run: from
/// the workspace when it loads, and a bare `target/` with an empty yardstick
/// when it does not. No dependency artifact is then judged stale, only the
/// incremental caches, as idle or superseded; the note printed here says
/// so, and so do the report's warnings.
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
