//! Where a project's builds went on disk, and what of it is dead weight.
//!
//! A Rust build directory only grows. Every `cargo update` leaves the previous
//! version of each moved dependency behind, compiled, alongside the new one;
//! every dependency dropped from the manifest leaves its artifacts; every
//! crate's incremental cache outlives the last time the crate changed. Nothing
//! in cargo removes any of it short of `cargo clean`, which removes the
//! things still needed too. One machine here had 140 GB under one `target/`
//! and a full disk, and the compiler's report of that was `IO failure on
//! output stream`.
//!
//! Stable cargo records nothing about when an artifact was last *used* — a
//! build that finds a unit fresh touches none of its files (`-Z mtime-on-use`
//! is nightly-only) — so "recently used" cannot be read off the filesystem.
//! What can be read is *what the build would need today*: the resolved
//! dependency graph, from `cargo metadata`. Every compiled unit names its
//! source in its dep-info file, and a registry source names the package and
//! the exact version. An artifact whose version the lockfile no longer
//! resolves is never read again unless the lockfile moves back; one whose
//! package left the graph is never read again at all. Those two rules,
//! plus an idle threshold for the incremental caches (which rustc rebuilds
//! from nothing at the cost of one slower compile), are what "stale" means
//! here. Anything the scan cannot classify with certainty is kept and said so
//! in the report — refuse rather than guess, applied to deletion, where a
//! guess costs a rebuild at best.
//!
//! Everything that removes goes through this module's own re-scan: a caller
//! names a policy or a tree, never a list of paths, so a stale list from an
//! earlier scan cannot delete what a build since then created.

mod fs;
mod judge;
mod sweep;
mod tree;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use guppy::graph::{PackageGraph, PackageSource};

use crate::model::{DiskItem, DiskReport, StaleReason};

use fs::{measure, same_dir, sorted_entries};
use sweep::{cargo_home, cargo_home_items};
use tree::TreeScan;

pub use fs::volume_of;
pub use sweep::{remove_cargo_cache, remove_tree, sweep};

/// What the resolved graph holds — the yardstick for what a build directory
/// still needs.
#[derive(Debug, Default, Clone)]
pub struct Current {
    /// Every `(name, version)` the lockfile resolves.
    versions: HashSet<(String, String)>,
    /// Every package name in the graph.
    names: HashSet<String>,
    /// Full revisions of git dependencies.
    git_revs: HashSet<String>,
    /// Workspace members and path dependencies, as crate names (`_` for `-`),
    /// which is how their incremental caches are named.
    local: HashSet<String>,
}

impl Current {
    pub fn from_graph(graph: &PackageGraph) -> Self {
        let mut current = Current::default();
        for package in graph.packages() {
            let name = package.name().to_string();
            current
                .versions
                .insert((name.clone(), package.version().to_string()));
            match package.source() {
                PackageSource::Workspace(_) | PackageSource::Path(_) => {
                    current.local.insert(crate_name(&name));
                }
                PackageSource::External(source) => {
                    if source.starts_with("git+")
                        && let Some((_, rev)) = source.rsplit_once('#')
                    {
                        current.git_revs.insert(rev.to_string());
                    }
                }
            }
            current.names.insert(name);
        }
        current
    }

    /// Whether nothing at all is known — a scan with an empty yardstick would
    /// judge every dependency gone, so it judges none.
    fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// A package name as rustc spells the crate: `-` becomes `_`.
fn crate_name(package: &str) -> String {
    package.replace('-', "_")
}

/// How stale artifacts are judged.
#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    /// An incremental cache untouched for this many days is idle.
    pub idle_days: u32,
    /// How many of a crate's incremental caches to keep, newest first; the
    /// rest are superseded. Four covers the combinations a workspace
    /// alternates between — build, test, check and clippy.
    pub keep_variants: u32,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            idle_days: 7,
            keep_variants: 4,
        }
    }
}

/// A scan: the report the frontend draws, and the paths behind its stale
/// numbers, which only [`sweep`](fn@sweep) reads.
pub struct Scan {
    pub report: DiskReport,
    stale: Vec<Stale>,
}

#[derive(Debug, Clone)]
struct Stale {
    tree: PathBuf,
    path: PathBuf,
    bytes: u64,
    reason: StaleReason,
}

impl Scan {
    /// Every stale path with its reason, largest first — for the CLI's
    /// listing and for tests.
    pub fn stale_paths(&self) -> Vec<(PathBuf, u64, StaleReason)> {
        let mut out: Vec<_> = self
            .stale
            .iter()
            .map(|s| (s.path.clone(), s.bytes, s.reason.clone()))
            .collect();
        out.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        out
    }
}

/// Measure the build directory and judge what in it is stale.
///
/// `project_root` is where the volume is measured when the build directory
/// does not exist yet, and what decides `shared`.
pub fn scan(
    target_dir: &Path,
    project_root: &Path,
    current: &Current,
    options: ScanOptions,
) -> Scan {
    let mut warnings = Vec::new();
    if current.is_empty() {
        warnings.push(
            "the dependency graph could not be read, so no dependency artifact is marked \
             stale; only the incremental caches are judged, by age and by count"
                .to_string(),
        );
    }
    let exists = target_dir.is_dir();
    let shared = !same_dir(target_dir, &project_root.join("target"));
    let volume = volume_of(if exists { target_dir } else { project_root });

    // Build trees in the order the walk finds them: `<profile>` for the
    // host, `<triple>/<profile>` for a target.
    let mut found: Vec<(PathBuf, Option<String>, String)> = Vec::new();
    let mut extras = Vec::new();
    let mut total_bytes = 0;
    let mut total_files = 0;

    if exists {
        for entry in sorted_entries(target_dir) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_file() {
                total_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                total_files += 1;
                continue;
            }
            if is_profile_tree(&path) {
                found.push((path, None, name));
                continue;
            }
            let profiles: Vec<_> = sorted_entries(&path)
                .into_iter()
                .filter(|e| is_profile_tree(&e.path()))
                .collect();
            if !profiles.is_empty() {
                for profile in profiles {
                    let profile_name = profile.file_name().to_string_lossy().to_string();
                    found.push((profile.path(), Some(name.clone()), profile_name));
                }
                continue;
            }
            let (bytes, files) = measure(&path);
            total_bytes += bytes;
            total_files += files;
            let (label, removable) = extra_label(&name);
            extras.push(DiskItem {
                label: label.to_string(),
                path: path.to_string_lossy().to_string(),
                bytes,
                files,
                removable,
            });
        }
    }

    let mut tree_scan = TreeScan {
        current,
        options,
        stale: Vec::new(),
        warnings,
        debuginfo_bytes: 0,
    };
    let mut trees = Vec::new();
    for (path, triple, profile) in &found {
        let tree = tree_scan.analyze(path, triple.as_deref(), profile);
        total_bytes += tree.bytes;
        total_files += tree.files;
        trees.push(tree);
    }
    extras.sort_by_key(|item| std::cmp::Reverse(item.bytes));
    trees.sort_by_key(|tree| std::cmp::Reverse(tree.bytes));

    Scan {
        report: DiskReport {
            target_dir: target_dir.to_string_lossy().to_string(),
            shared,
            exists,
            total_bytes,
            files: total_files,
            volume,
            trees,
            extras,
            cargo_home: cargo_home_items(),
            cargo_home_dir: cargo_home().map(|home| home.to_string_lossy().to_string()),
            debuginfo_bytes: tree_scan.debuginfo_bytes,
            warnings: tree_scan.warnings,
        },
        stale: tree_scan.stale,
    }
}

fn is_profile_tree(dir: &Path) -> bool {
    dir.join(".fingerprint").is_dir() || dir.join("deps").is_dir()
}

/// What a top-level directory that is not a build tree is, and whether
/// rusty removes it on request.
fn extra_label(name: &str) -> (&'static str, bool) {
    match name {
        "doc" => ("docs", true),
        "rusty-sim" => ("sim-images", true),
        "tmp" => ("tmp", true),
        "package" => ("package", true),
        "rust-analyzer" => ("check", true),
        n if n.starts_with("flycheck") => ("check", true),
        "wasm-bindgen" => ("staging", true),
        _ => ("other", false),
    }
}
