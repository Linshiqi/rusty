//! `deps` and `features`: the Cargo workspace's resolved graph, and what a
//! feature selection costs in it.

use std::path::Path;

use anyhow::{Context, Result};
use rusty_core::{FeatureSelection, Workspace, WorkspaceReport};

use super::emit;

/// `rusty deps`: the workspace's health, from its resolved graph.
pub(crate) fn deps(path: &Path, json: bool) -> Result<()> {
    let workspace = Workspace::load(path)
        .with_context(|| format!("loading workspace at {}", path.display()))?;
    let report = workspace.report()?;
    if json {
        emit(&report)?;
    } else {
        print_deps(&report);
    }
    Ok(())
}

/// `rusty features`: what a feature selection costs, and every feature a
/// package declares.
pub(crate) fn features(
    package: String,
    path: &Path,
    features: Vec<String>,
    no_default_features: bool,
    json: bool,
) -> Result<()> {
    let workspace = Workspace::load(path)
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
    Ok(())
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
