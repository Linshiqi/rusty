//! Removing what a scan finds: the stale paths a policy names, one whole
//! tree or known extra named by path, and one of cargo's own caches — which
//! are measured here too, because the report lists them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::fs::{measure, remove_path, same_dir, tree_locked};
use super::{Current, ScanOptions, scan};
use crate::error::{Error, Result};
use crate::model::{BuildTree, DiskItem, StaleReason, SweepPolicy, SweepReport};

/// Remove what the policy names, tree by tree, after a fresh scan.
pub fn sweep(
    target_dir: &Path,
    project_root: &Path,
    current: &Current,
    policy: &SweepPolicy,
) -> Result<SweepReport> {
    let options = ScanOptions {
        idle_days: policy.idle_days.unwrap_or(u32::MAX),
        keep_variants: policy.keep_variants,
    };
    let scan = scan(target_dir, project_root, current, options);
    let mut report = SweepReport::default();
    let wanted: Option<PathBuf> = policy.tree.as_ref().map(PathBuf::from);
    let mut locked: HashSet<PathBuf> = HashSet::new();
    for tree in &scan.report.trees {
        let path = PathBuf::from(&tree.path);
        if let Some(wanted) = &wanted
            && !same_dir(wanted, &path)
        {
            continue;
        }
        if tree.locked || tree_locked(&path) {
            locked.insert(path.clone());
            report.locked.push(tree.path.clone());
        }
    }
    for stale in &scan.stale {
        if let Some(wanted) = &wanted
            && !same_dir(wanted, &stale.tree)
        {
            continue;
        }
        if locked.contains(&stale.tree) {
            continue;
        }
        let allowed = match &stale.reason {
            StaleReason::VersionGone { .. } => policy.version_gone,
            StaleReason::PackageGone { .. } => policy.package_gone,
            StaleReason::Idle { .. } => policy.idle_days.is_some(),
            StaleReason::Superseded { .. } => policy.superseded,
        };
        if !allowed {
            continue;
        }
        match remove_path(&stale.path) {
            Ok(()) => {
                report.removed_bytes += stale.bytes;
                report.removed_items += 1;
            }
            Err(error) => report
                .failed
                .push(format!("{}: {error}", stale.path.display())),
        }
    }
    Ok(report)
}

/// Remove one whole build tree, a tree's incremental caches, or a top-level
/// extra, named by path.
///
/// The path has to be one the scan would list — a `<profile>` or
/// `<triple>/<profile>` tree, that tree's `incremental/`, or an extra the
/// scan marks removable — and its tree must not be locked. Anything else is
/// refused with the reason.
pub fn remove_tree(target_dir: &Path, path: &Path) -> Result<SweepReport> {
    let scan = scan(
        target_dir,
        target_dir,
        &Current::default(),
        ScanOptions::default(),
    );
    let tree = scan
        .report
        .trees
        .iter()
        .find(|t| same_dir(Path::new(&t.path), path));
    let incremental_of = scan.report.trees.iter().find(|t| {
        path.file_name().is_some_and(|name| name == "incremental")
            && path
                .parent()
                .is_some_and(|parent| same_dir(Path::new(&t.path), parent))
    });
    let extra = scan
        .report
        .extras
        .iter()
        .find(|e| same_dir(Path::new(&e.path), path));
    let refuse_locked = |tree: &BuildTree| -> Result<()> {
        if tree.locked || tree_locked(Path::new(&tree.path)) {
            return Err(Error::Refused {
                detail: format!(
                    "a build holds the lock on {}; wait for it to finish",
                    tree.path
                ),
            });
        }
        Ok(())
    };
    let (bytes, files) = match (tree, incremental_of, extra) {
        (Some(tree), _, _) => {
            refuse_locked(tree)?;
            (tree.bytes, tree.files)
        }
        (None, Some(tree), _) => {
            refuse_locked(tree)?;
            measure(path)
        }
        (None, None, Some(extra)) if extra.removable => (extra.bytes, extra.files),
        (None, None, Some(extra)) => {
            return Err(Error::Refused {
                detail: format!(
                    "{} is not something rusty knows to be safe to remove",
                    extra.path
                ),
            });
        }
        (None, None, None) => {
            return Err(Error::Refused {
                detail: format!(
                    "{} is not a build tree, its incremental caches or a known extra under {}",
                    path.display(),
                    target_dir.display()
                ),
            });
        }
    };
    remove_path(path)?;
    Ok(SweepReport {
        removed_bytes: bytes,
        removed_items: files.max(1),
        ..SweepReport::default()
    })
}

/// Remove one of cargo's own caches, named by its label from
/// `cargo_home_items`. Only the removable ones; the index and the git
/// object databases stay.
pub fn remove_cargo_cache(label: &str) -> Result<SweepReport> {
    let item = cargo_home_items()
        .into_iter()
        .find(|item| item.label == label && item.removable)
        .ok_or_else(|| Error::Refused {
            detail: format!("`{label}` is not a cargo cache rusty removes"),
        })?;
    remove_path(Path::new(&item.path))?;
    Ok(SweepReport {
        removed_bytes: item.bytes,
        removed_items: item.files.max(1),
        ..SweepReport::default()
    })
}

/// Cargo's caches under `CARGO_HOME`, measured. The registry sources are
/// re-extracted from the downloaded archives on demand and the archives and
/// git checkouts are re-fetched, so those three are removable; the index and
/// the git object databases are not offered.
pub(super) fn cargo_home_items() -> Vec<DiskItem> {
    let Some(home) = cargo_home() else {
        return Vec::new();
    };
    [
        ("registry-src", "registry/src", true),
        ("registry-cache", "registry/cache", true),
        ("registry-index", "registry/index", false),
        ("git-checkouts", "git/checkouts", true),
        ("git-db", "git/db", false),
    ]
    .into_iter()
    .filter_map(|(label, relative, removable)| {
        let path = home.join(relative);
        if !path.is_dir() {
            return None;
        }
        let (bytes, files) = measure(&path);
        Some(DiskItem {
            label: label.to_string(),
            path: path.to_string_lossy().to_string(),
            bytes,
            files,
            removable,
        })
    })
    .collect()
}

pub(super) fn cargo_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home));
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join(".cargo"))
}
