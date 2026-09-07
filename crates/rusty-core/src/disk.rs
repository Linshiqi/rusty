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

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use guppy::graph::{PackageGraph, PackageSource};

use crate::error::{Error, Result};
use crate::model::{
    BuildTree, DiskGroup, DiskItem, DiskKind, DiskReport, StaleReason, StaleSummary, SweepPolicy,
    SweepReport, Volume,
};

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

    /// A yardstick built by hand — tests, and the CLI when no workspace can
    /// be loaded but a scan is still wanted (nothing is then stale except
    /// idle incremental caches).
    pub fn new(
        versions: impl IntoIterator<Item = (String, String)>,
        local: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut current = Current::default();
        for (name, version) in versions {
            current.names.insert(name.clone());
            current.versions.insert((name, version));
        }
        for name in local {
            current.names.insert(name.clone());
            current.local.insert(crate_name(&name));
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

/// One incremental cache: its directory, bytes, files, and when rustc last
/// wrote to it.
type Variant = (PathBuf, u64, u64, Option<SystemTime>);

/// A scan: the report the frontend draws, and the paths behind its stale
/// numbers, which only [`sweep`] reads.
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

    let mut trees = Vec::new();
    let mut extras = Vec::new();
    let mut stale = Vec::new();
    let mut total_bytes = 0;
    let mut total_files = 0;
    let mut debuginfo_bytes = 0;

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
                let tree = analyze_tree(
                    &path,
                    None,
                    &name,
                    current,
                    options,
                    &mut stale,
                    &mut warnings,
                    &mut debuginfo_bytes,
                );
                total_bytes += tree.bytes;
                total_files += tree.files;
                trees.push(tree);
                continue;
            }
            let profiles: Vec<fs::DirEntry> = sorted_entries(&path)
                .into_iter()
                .filter(|e| is_profile_tree(&e.path()))
                .collect();
            if !profiles.is_empty() {
                for profile in profiles {
                    let profile_name = profile.file_name().to_string_lossy().to_string();
                    let tree = analyze_tree(
                        &profile.path(),
                        Some(&name),
                        &profile_name,
                        current,
                        options,
                        &mut stale,
                        &mut warnings,
                        &mut debuginfo_bytes,
                    );
                    total_bytes += tree.bytes;
                    total_files += tree.files;
                    trees.push(tree);
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
            debuginfo_bytes,
            warnings,
        },
        stale,
    }
}

/// Free and total space on the volume holding `path`.
pub fn volume_of(path: &Path) -> Option<Volume> {
    let free_bytes = fs4::available_space(path).ok()?;
    let total_bytes = fs4::total_space(path).ok()?;
    Some(Volume {
        free_bytes,
        total_bytes,
    })
}

/// Whether a build holds this tree's lock right now. Cargo takes
/// `<tree>/.cargo-lock` for the whole of a build, and so does rust-analyzer's
/// check; nothing in a locked tree is removed.
pub fn tree_locked(tree: &Path) -> bool {
    use fs4::fs_std::FileExt;
    let Ok(file) = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(tree.join(".cargo-lock"))
    else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(true) => {
            let _ = FileExt::unlock(&file);
            false
        }
        Ok(false) | Err(_) => true,
    }
}

/// Remove what the policy names, tree by tree, after a fresh scan.
pub fn sweep(
    target_dir: &Path,
    project_root: &Path,
    current: &Current,
    policy: &SweepPolicy,
) -> Result<SweepReport> {
    let options = ScanOptions {
        idle_days: policy.idle_days.unwrap_or(u32::MAX),
        ..ScanOptions::default()
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
/// [`cargo_home_items`]. Only the removable ones; the index and the git
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
pub fn cargo_home_items() -> Vec<DiskItem> {
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

fn cargo_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home));
    }
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join(".cargo"))
}

// ─── one build tree ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn analyze_tree(
    path: &Path,
    triple: Option<&str>,
    profile: &str,
    current: &Current,
    options: ScanOptions,
    stale: &mut Vec<Stale>,
    warnings: &mut Vec<String>,
    debuginfo_bytes: &mut u64,
) -> BuildTree {
    let mut groups: HashMap<DiskKind, DiskGroup> = HashMap::new();
    // Units in `deps/`, grouped by the hash cargo gives every file of one
    // unit — `libserde-06a2…rlib`, `serde-06a2….d`, `serde-06a2….rmeta` are
    // one unit — so a verdict on the dep-info file covers the unit.
    let deps_dir = path.join("deps");
    let mut units: HashMap<String, Vec<(PathBuf, u64)>> = HashMap::new();
    let mut dep_infos: HashMap<String, PathBuf> = HashMap::new();
    for entry in sorted_entries(&deps_dir) {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let file = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let bytes = if metadata.is_dir() {
            measure(&file).0
        } else {
            metadata.len()
        };
        add(&mut groups, DiskKind::Deps, bytes, 1);
        if is_debuginfo(&name) {
            *debuginfo_bytes += bytes;
        }
        if let Some(hash) = unit_hash(&name) {
            if name.ends_with(".d") {
                dep_infos.insert(hash.to_string(), file.clone());
            }
            units
                .entry(hash.to_string())
                .or_default()
                .push((file, bytes));
        }
    }
    let mut stale_hashes: HashMap<String, StaleReason> = HashMap::new();
    if !current.is_empty() {
        for (hash, dep_info) in &dep_infos {
            match verdict_of(dep_info, current) {
                Ok(Verdict::Stale(reason)) => {
                    stale_hashes.insert(hash.clone(), reason);
                }
                Ok(Verdict::Live) => {}
                Err(detail) => warnings.push(format!("{}: {detail}", dep_info.display())),
            }
        }
    }
    for (hash, reason) in &stale_hashes {
        for (file, bytes) in units.get(hash).into_iter().flatten() {
            push_stale(
                stale,
                &mut groups,
                DiskKind::Deps,
                path,
                file,
                *bytes,
                1,
                reason,
            );
        }
    }

    // Build scripts: `build/<pkg>-<hash>/` is either the compiled script
    // (judged by its own dep-info, like any unit) or the directory it ran in,
    // linked to its script through cargo's fingerprint record — or, when
    // that record cannot be read, judged only by whether the package is in
    // the graph at all.
    let build_dir = path.join("build");
    let fingerprint_dir = path.join(".fingerprint");
    let script_hash_by_fingerprint = script_fingerprints(&fingerprint_dir);
    let mut compile_verdicts: HashMap<String, StaleReason> = HashMap::new();
    let mut run_dirs: Vec<(PathBuf, String, u64, u64)> = Vec::new();
    for entry in sorted_entries(&build_dir) {
        let dir = entry.path();
        if !dir.is_dir() {
            let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            add(&mut groups, DiskKind::BuildScripts, bytes, 1);
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let (bytes, files) = measure(&dir);
        add(&mut groups, DiskKind::BuildScripts, bytes, files);
        *debuginfo_bytes += debuginfo_in(&dir);
        let Some(hash) = unit_hash(&name) else {
            continue;
        };
        let dep_info = sorted_entries(&dir)
            .into_iter()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "d"));
        match dep_info {
            Some(dep_info) if !current.is_empty() => match verdict_of(&dep_info, current) {
                Ok(Verdict::Stale(reason)) => {
                    compile_verdicts.insert(hash.to_string(), reason.clone());
                    push_stale(
                        stale,
                        &mut groups,
                        DiskKind::BuildScripts,
                        path,
                        &dir,
                        bytes,
                        files,
                        &reason,
                    );
                    stale_hashes.insert(hash.to_string(), reason);
                }
                Ok(Verdict::Live) => {}
                Err(detail) => warnings.push(format!("{}: {detail}", dep_info.display())),
            },
            Some(_) => {}
            None => run_dirs.push((dir, hash.to_string(), bytes, files)),
        }
    }
    for (dir, hash, bytes, files) in run_dirs {
        if current.is_empty() {
            continue;
        }
        let package = package_of_unit(
            dir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_ref(),
        );
        let reason = match run_script_hash(&fingerprint_dir, &package, &hash)
            .and_then(|fingerprint| script_hash_by_fingerprint.get(&fingerprint).cloned())
        {
            Some(script_hash) => compile_verdicts.get(&script_hash).cloned(),
            None if !current.names.contains(&package) => Some(StaleReason::PackageGone {
                package: package.clone(),
            }),
            None => None,
        };
        if let Some(reason) = reason {
            push_stale(
                stale,
                &mut groups,
                DiskKind::BuildScripts,
                path,
                &dir,
                bytes,
                files,
                &reason,
            );
            stale_hashes.insert(hash, reason);
        }
    }

    // Fingerprints follow their units.
    for entry in sorted_entries(&fingerprint_dir) {
        let dir = entry.path();
        let (bytes, files) = measure(&dir);
        add(&mut groups, DiskKind::Fingerprints, bytes, files);
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(hash) = unit_hash(&name)
            && let Some(reason) = stale_hashes.get(hash)
        {
            push_stale(
                stale,
                &mut groups,
                DiskKind::Fingerprints,
                path,
                &dir,
                bytes,
                files,
                reason,
            );
        }
    }

    // Incremental caches: named for the crate, judged by whether the crate
    // is still local to this workspace, by when rustc last wrote to the
    // cache — and, since rustc keys the cache on the unit's flags and a hot
    // workspace grows a hundred variants per crate, by whether it is among
    // the crate's newest few.
    let idle_after = Duration::from_secs(u64::from(options.idle_days) * 86_400);
    let now = SystemTime::now();
    let mut variants: HashMap<String, Vec<Variant>> = HashMap::new();
    for entry in sorted_entries(&path.join("incremental")) {
        let dir = entry.path();
        let (bytes, files) = measure(&dir);
        add(&mut groups, DiskKind::Incremental, bytes, files);
        let name = entry.file_name().to_string_lossy().to_string();
        let Some((crate_name, _)) = name.rsplit_once('-') else {
            continue;
        };
        if !current.is_empty() && !current.local.contains(crate_name) {
            push_stale(
                stale,
                &mut groups,
                DiskKind::Incremental,
                path,
                &dir,
                bytes,
                files,
                &StaleReason::PackageGone {
                    package: crate_name.to_string(),
                },
            );
            continue;
        }
        let touched = newest_mtime(&dir);
        if options.idle_days != u32::MAX
            && let Some(touched) = touched
            && now.duration_since(touched).unwrap_or_default() >= idle_after
        {
            push_stale(
                stale,
                &mut groups,
                DiskKind::Incremental,
                path,
                &dir,
                bytes,
                files,
                &StaleReason::Idle {
                    days: options.idle_days,
                },
            );
            continue;
        }
        variants
            .entry(crate_name.to_string())
            .or_default()
            .push((dir, bytes, files, touched));
    }
    for caches in variants.values_mut() {
        caches.sort_by_key(|cache| std::cmp::Reverse(cache.3));
        for (dir, bytes, files, _) in caches.iter().skip(options.keep_variants as usize) {
            push_stale(
                stale,
                &mut groups,
                DiskKind::Incremental,
                path,
                dir,
                *bytes,
                *files,
                &StaleReason::Superseded {
                    keep: options.keep_variants,
                },
            );
        }
    }

    // Everything else in the tree: the uplifted binaries and examples, and
    // whatever a tool put there.
    for entry in sorted_entries(path) {
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(
            name.as_str(),
            "deps" | "build" | ".fingerprint" | "incremental"
        ) {
            continue;
        }
        let file = entry.path();
        let (bytes, files) = if file.is_dir() {
            measure(&file)
        } else {
            (entry.metadata().map(|m| m.len()).unwrap_or(0), 1)
        };
        if is_debuginfo(&name) {
            *debuginfo_bytes += bytes;
        }
        let kind = if file.is_dir() && name != "examples" {
            DiskKind::Other
        } else {
            DiskKind::Binaries
        };
        add(&mut groups, kind, bytes, files);
    }

    let mut groups: Vec<DiskGroup> = groups.into_values().collect();
    groups.sort_by_key(|group| std::cmp::Reverse(group.bytes));
    for group in &mut groups {
        group
            .stale_by_reason
            .sort_by_key(|summary| std::cmp::Reverse(summary.bytes));
    }
    BuildTree {
        triple: triple.map(str::to_string),
        profile: profile.to_string(),
        path: path.to_string_lossy().to_string(),
        bytes: groups.iter().map(|g| g.bytes).sum(),
        files: groups.iter().map(|g| g.files).sum(),
        locked: tree_locked(path),
        groups,
    }
}

/// Count `bytes` and `files` under `kind`, creating the group on first use.
fn add(groups: &mut HashMap<DiskKind, DiskGroup>, kind: DiskKind, bytes: u64, files: u64) {
    let group = groups.entry(kind).or_insert_with(|| DiskGroup {
        kind,
        bytes: 0,
        files: 0,
        stale_bytes: 0,
        stale_files: 0,
        stale_by_reason: Vec::new(),
    });
    group.bytes += bytes;
    group.files += files;
}

#[allow(clippy::too_many_arguments)]
fn push_stale(
    stale: &mut Vec<Stale>,
    groups: &mut HashMap<DiskKind, DiskGroup>,
    kind: DiskKind,
    tree: &Path,
    path: &Path,
    bytes: u64,
    files: u64,
    reason: &StaleReason,
) {
    if let Some(group) = groups.get_mut(&kind) {
        group.stale_bytes += bytes;
        group.stale_files += files;
        let name = reason_name(reason);
        match group.stale_by_reason.iter_mut().find(|s| s.reason == name) {
            Some(summary) => {
                summary.bytes += bytes;
                summary.files += files;
            }
            None => group.stale_by_reason.push(StaleSummary {
                reason: name.to_string(),
                bytes,
                files,
            }),
        }
    }
    stale.push(Stale {
        tree: tree.to_path_buf(),
        path: path.to_path_buf(),
        bytes,
        reason: reason.clone(),
    });
}

fn reason_name(reason: &StaleReason) -> &'static str {
    match reason {
        StaleReason::VersionGone { .. } => "version-gone",
        StaleReason::PackageGone { .. } => "package-gone",
        StaleReason::Idle { .. } => "idle",
        StaleReason::Superseded { .. } => "superseded",
    }
}

// ─── judging one unit ────────────────────────────────────────────────────────

enum Verdict {
    Live,
    Stale(StaleReason),
}

/// Read a unit's dep-info file and judge it against the graph.
fn verdict_of(dep_info: &Path, current: &Current) -> std::result::Result<Verdict, String> {
    let text = fs::read_to_string(dep_info).map_err(|e| format!("could not read: {e}"))?;
    if text.trim().is_empty() {
        // A compile that never finished — a killed build, a full disk. Cargo
        // rebuilds the unit regardless, so there is nothing to judge.
        return Ok(Verdict::Live);
    }
    let sources = dep_info_sources(&text).ok_or("not a dep-info file")?;
    Ok(judge_sources(&sources, current))
}

/// The source paths a dep-info file names: everything after the first
/// `output: ` on its first line. Windows paths carry `:\`, never `: `, so the
/// first colon-space is the separator.
pub fn dep_info_sources(text: &str) -> Option<Vec<String>> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let (_, rest) = line.split_once(": ")?;
    Some(
        rest.split_whitespace()
            .map(|s| s.trim_end_matches('\\').to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

/// Where a unit's sources come from, read off the first path that says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// `<registry>/src/<index>/<name>-<version>/…`
    Registry { dir: String },
    /// `<cargo home>/git/checkouts/<repo>-<hash>/<rev>/…`
    Git { repo: String, rev: String },
    /// A path in the workspace or a path dependency.
    Local,
}

pub fn origin_of(sources: &[String]) -> Origin {
    for source in sources {
        let segments: Vec<&str> = source.split(['\\', '/']).collect();
        for (index, window) in segments.windows(2).enumerate() {
            if window == ["registry", "src"]
                && let Some(dir) = segments.get(index + 3)
            {
                return Origin::Registry {
                    dir: (*dir).to_string(),
                };
            }
            if window == ["git", "checkouts"]
                && let (Some(repo), Some(rev)) = (segments.get(index + 2), segments.get(index + 3))
            {
                return Origin::Git {
                    repo: (*repo).to_string(),
                    rev: (*rev).to_string(),
                };
            }
        }
    }
    Origin::Local
}

fn judge_sources(sources: &[String], current: &Current) -> Verdict {
    match origin_of(sources) {
        Origin::Local => Verdict::Live,
        Origin::Registry { dir } => {
            let candidates = split_package_dir(&dir);
            if candidates
                .iter()
                .any(|(name, version)| current.versions.contains(&(name.clone(), version.clone())))
            {
                return Verdict::Live;
            }
            match candidates
                .iter()
                .find(|(name, _)| current.names.contains(name))
            {
                Some((package, version)) => Verdict::Stale(StaleReason::VersionGone {
                    package: package.clone(),
                    version: version.clone(),
                }),
                // A directory that splits into no `name-semver` pair is not a
                // registry crate as cargo lays them out; do not judge it.
                None if candidates.is_empty() => Verdict::Live,
                None => Verdict::Stale(StaleReason::PackageGone {
                    package: candidates[0].0.clone(),
                }),
            }
        }
        Origin::Git { repo, rev } => {
            if current.git_revs.iter().any(|full| full.starts_with(&rev)) {
                return Verdict::Live;
            }
            // `<repo>-<hash>`: the repository's name is what precedes the
            // last dash. Judged by name, since the graph knows packages, not
            // repositories: a name still present is a moved revision, one
            // absent is a dependency dropped.
            let package = repo
                .rsplit_once('-')
                .map_or(repo.as_str(), |(name, _)| name);
            if current.names.contains(package) {
                Verdict::Stale(StaleReason::VersionGone {
                    package: package.to_string(),
                    version: rev,
                })
            } else {
                Verdict::Stale(StaleReason::PackageGone {
                    package: package.to_string(),
                })
            }
        }
    }
}

/// Every `(name, version)` reading of a registry directory name. Names may
/// contain dashes and versions may too (`1.0.0-beta.1`), so each dash is
/// tried and kept when what follows parses as a semver version — for
/// `windows-sys-0.52.0` that is one reading, for `sha-1-0.10.1` it is two.
pub fn split_package_dir(dir: &str) -> Vec<(String, String)> {
    dir.match_indices('-')
        .filter_map(|(at, _)| {
            let (name, version) = (&dir[..at], &dir[at + 1..]);
            (!name.is_empty() && semver::Version::parse(version).is_ok())
                .then(|| (name.to_string(), version.to_string()))
        })
        .collect()
}

/// The 16-hex-digit unit hash cargo suffixes every artifact and fingerprint
/// directory with, when the name carries one.
pub fn unit_hash(file_name: &str) -> Option<&str> {
    let stem = file_name
        .split_once('.')
        .map_or(file_name, |(stem, _)| stem);
    let (_, hash) = stem.rsplit_once('-')?;
    (hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(hash)
}

/// The package a `<pkg>-<hash>` directory belongs to.
fn package_of_unit(dir_name: &str) -> String {
    dir_name
        .rsplit_once('-')
        .map_or(dir_name, |(name, _)| name)
        .to_string()
}

/// Every compiled build script's fingerprint hash → its unit hash, read from
/// `.fingerprint/<pkg>-<hash>/build-script-build-script-build`, whose content
/// is the fingerprint's hash in hex.
fn script_fingerprints(fingerprint_dir: &Path) -> HashMap<u64, String> {
    let mut map = HashMap::new();
    for entry in sorted_entries(fingerprint_dir) {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(hash) = unit_hash(&name) else {
            continue;
        };
        for file in sorted_entries(&entry.path()) {
            let file_name = file.file_name().to_string_lossy().to_string();
            if file_name.starts_with("build-script-")
                && !file_name.ends_with(".json")
                && let Ok(text) = fs::read_to_string(file.path())
                && let Ok(value) = u64::from_str_radix(text.trim(), 16)
            {
                map.insert(value, hash.to_string());
            }
        }
    }
    map
}

/// The fingerprint hash of the script a run directory ran, from the run
/// unit's fingerprint record: its `deps` list has one entry, the script,
/// whose last field is that hash. `None` when the record is missing or not
/// the shape this was written against — a caller then falls back to the
/// package-level rule rather than guessing.
fn run_script_hash(fingerprint_dir: &Path, package: &str, hash: &str) -> Option<u64> {
    let dir = fingerprint_dir.join(format!("{package}-{hash}"));
    let record = sorted_entries(&dir).into_iter().find(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        name.starts_with("run-build-script-") && name.ends_with(".json")
    })?;
    let text = fs::read_to_string(record.path()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let deps = value.get("deps")?.as_array()?;
    let script = deps
        .iter()
        .find(|dep| dep.get(1).and_then(|n| n.as_str()) == Some("build_script_build"))?;
    script.as_array()?.last()?.as_u64()
}

// ─── the filesystem ──────────────────────────────────────────────────────────

/// Bytes and files under `path`, symlinks not followed.
pub fn measure(path: &Path) -> (u64, u64) {
    let mut bytes = 0;
    let mut files = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else {
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                files += 1;
            }
        }
    }
    (bytes, files)
}

/// The newest modification time of any file under `dir`, or the directory's
/// own when it holds none.
fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if let Ok(modified) = metadata.modified() {
                newest = Some(newest.map_or(modified, |n| n.max(modified)));
            }
        }
    }
    newest.or_else(|| fs::metadata(dir).and_then(|m| m.modified()).ok())
}

fn debuginfo_in(dir: &Path) -> u64 {
    let mut bytes = 0;
    for entry in sorted_entries(dir) {
        let name = entry.file_name().to_string_lossy().to_string();
        if is_debuginfo(&name) {
            bytes += if entry.path().is_dir() {
                measure(&entry.path()).0
            } else {
                entry.metadata().map(|m| m.len()).unwrap_or(0)
            };
        }
    }
    bytes
}

fn is_debuginfo(name: &str) -> bool {
    name.ends_with(".pdb") || name.ends_with(".dSYM") || name.ends_with(".dwp")
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

fn sorted_entries(dir: &Path) -> Vec<fs::DirEntry> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)
        .map(|iter| iter.flatten().collect())
        .unwrap_or_default();
    entries.sort_by_key(fs::DirEntry::file_name);
    entries
}

fn remove_path(path: &Path) -> Result<()> {
    let io = |source| Error::Io {
        what: "remove".into(),
        path: path.display().to_string(),
        source,
    };
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(io)
    } else {
        fs::remove_file(path).map_err(io)
    }
}

/// Whether two paths name the same directory, by canonical form when both
/// exist and by text otherwise.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A build directory laid out the way cargo lays one out, with the sizes
    /// and mtimes the rules read — real files, since the rules read real
    /// files.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "rusty-disk-{name}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("project")).unwrap();
            Fixture { root }
        }

        fn project(&self) -> PathBuf {
            self.root.join("project")
        }

        fn target(&self) -> PathBuf {
            self.project().join("target")
        }

        fn write(&self, relative: &str, bytes: usize) -> PathBuf {
            let path = self.target().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, vec![b'x'; bytes]).unwrap();
            path
        }

        fn write_text(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.target().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
            path
        }

        /// A unit in `deps/`: the dep-info naming `source`, and an rlib.
        fn unit(&self, tree: &str, crate_name: &str, hash: &str, source: &str, rlib_bytes: usize) {
            self.write_text(
                &format!("{tree}/deps/{crate_name}-{hash}.d"),
                &format!(
                    "E:\\proj\\target\\{tree}\\deps\\{crate_name}-{hash}.d: {source}\n\n{source}:\n"
                ),
            );
            self.write(
                &format!("{tree}/deps/lib{crate_name}-{hash}.rlib"),
                rlib_bytes,
            );
            self.write(&format!("{tree}/deps/lib{crate_name}-{hash}.rmeta"), 10);
            self.write(
                &format!("{tree}/.fingerprint/{crate_name}-{hash}/lib-{crate_name}"),
                16,
            );
        }

        fn age(&self, relative: &str, days: u64) {
            let path = self.target().join(relative);
            let old = SystemTime::now() - Duration::from_secs(days * 86_400);
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    const REGISTRY: &str = "C:\\Users\\me\\.cargo\\registry\\src\\index.crates.io-1949cf8c6b5b557f";

    fn current() -> Current {
        Current::new(
            [
                ("serde".to_string(), "1.0.229".to_string()),
                ("windows-sys".to_string(), "0.52.0".to_string()),
            ],
            ["my-app".to_string()],
        )
    }

    fn lay_out(fixture: &Fixture) {
        // serde: the version the lockfile has, and the one it moved away from.
        fixture.unit(
            "debug",
            "serde",
            "aaaaaaaaaaaaaaaa",
            &format!("{REGISTRY}\\serde-1.0.200\\src\\lib.rs"),
            5_000,
        );
        fixture.unit(
            "debug",
            "serde",
            "bbbbbbbbbbbbbbbb",
            &format!("{REGISTRY}\\serde-1.0.229\\src\\lib.rs"),
            6_000,
        );
        // A dashed name, current.
        fixture.unit(
            "debug",
            "windows_sys",
            "cccccccccccccccc",
            &format!("{REGISTRY}\\windows-sys-0.52.0\\src\\lib.rs"),
            7_000,
        );
        // A dependency dropped from the graph entirely.
        fixture.unit(
            "debug",
            "left_pad",
            "dddddddddddddddd",
            &format!("{REGISTRY}\\left-pad-1.0.0\\src\\lib.rs"),
            800,
        );
        // The workspace's own crate: local, never stale by version.
        fixture.unit("debug", "my_app", "eeeeeeeeeeeeeeee", "src\\main.rs", 900);
        // Debug symbols beside a test binary.
        fixture.write("debug/deps/my_app-eeeeeeeeeeeeeeee.pdb", 4_000);
        // Incremental: a live crate touched today, the same crate idle for a
        // month, and a crate that is no longer in the workspace.
        fixture.write(
            "debug/incremental/my_app-1a2b3c4d5e6f7g/s-fresh-abc/query-cache.bin",
            300,
        );
        fixture.write(
            "debug/incremental/my_app-9z8y7x6w5v4u3t/s-old-def/query-cache.bin",
            400,
        );
        fixture.age(
            "debug/incremental/my_app-9z8y7x6w5v4u3t/s-old-def/query-cache.bin",
            30,
        );
        fixture.write(
            "debug/incremental/gone_crate-0000000000000/s-x-y/query-cache.bin",
            500,
        );
        // Five more variants of the live crate, a day old: with four kept,
        // two of them are superseded and the fresh one never is.
        for variant in [
            "v1v1v1v1v1v1v",
            "v2v2v2v2v2v2v",
            "v3v3v3v3v3v3v",
            "v4v4v4v4v4v4v",
            "v5v5v5v5v5v5v",
        ] {
            let relative = format!("debug/incremental/my_app-{variant}/s-a-b/query-cache.bin");
            fixture.write(&relative, 100);
            fixture.age(&relative, 1);
        }
        // Build scripts: the old serde's compiled script and its run
        // directory, linked through the fingerprint record.
        fixture.write_text(
            "debug/build/serde-aaaaaaaaaaaaaaaa/build_script_build-aaaaaaaaaaaaaaaa.d",
            &format!("x: {REGISTRY}\\serde-1.0.200\\build.rs\n"),
        );
        fixture.write(
            "debug/build/serde-aaaaaaaaaaaaaaaa/build_script_build-aaaaaaaaaaaaaaaa.exe",
            2_000,
        );
        fixture.write_text(
            "debug/.fingerprint/serde-aaaaaaaaaaaaaaaa/build-script-build-script-build",
            "00000000deadbeef",
        );
        fixture.write("debug/build/serde-ffffffffffffffff/out/generated.rs", 3_000);
        fixture.write_text(
            "debug/build/serde-ffffffffffffffff/output",
            "cargo:rerun-if-changed=build.rs\n",
        );
        fixture.write_text(
            "debug/.fingerprint/serde-ffffffffffffffff/run-build-script-build-script-build.json",
            &format!(
                "{{\"rustc\":1,\"deps\":[[42,\"build_script_build\",false,{}]]}}",
                0x0000_0000_dead_beefu64
            ),
        );
        // A run directory whose package is gone, with no readable record.
        fixture.write("debug/build/left-pad-1111111111111111/out/x.rs", 600);
        fixture.write_text("debug/build/left-pad-1111111111111111/output", "");
        // The uplifted binary.
        fixture.write("debug/my-app.exe", 1_000);
        // A second tree, for another target, with one live unit.
        fixture.unit(
            "xtensa-esp32-none-elf/release",
            "serde",
            "bbbbbbbbbbbbbbbb",
            &format!("{REGISTRY}\\serde-1.0.229\\src\\lib.rs"),
            2_500,
        );
        // Extras.
        fixture.write("doc/serde/index.html", 1_200);
        fixture.write("rusty-sim/app.bin", 1_300);
        fixture.write("mystery/thing.bin", 1_400);
        fixture.write_text(".rustc_info.json", "{}");
        fixture.write_text(
            "CACHEDIR.TAG",
            "Signature: 8a477f597d28d172789f06886806bc55",
        );
    }

    #[test]
    fn a_dep_info_names_its_registry_package_and_version() {
        let text = format!(
            "E:\\p\\target\\debug\\deps\\serde-06a2225fab2cda3a.d: {REGISTRY}\\serde-1.0.229\\src\\lib.rs {REGISTRY}\\serde-1.0.229\\src\\de.rs\n"
        );
        let sources = dep_info_sources(&text).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(
            origin_of(&sources),
            Origin::Registry {
                dir: "serde-1.0.229".into()
            }
        );
        // Forward slashes and a git checkout.
        let git = vec![
            "/home/me/.cargo/git/checkouts/esp-hal-3f2a9b/0123abc/esp-hal/src/lib.rs".to_string(),
        ];
        assert_eq!(
            origin_of(&git),
            Origin::Git {
                repo: "esp-hal-3f2a9b".into(),
                rev: "0123abc".into()
            }
        );
        // A member: relative paths, no registry.
        assert_eq!(
            origin_of(&["crates\\rusty-git\\src\\lib.rs".to_string()]),
            Origin::Local
        );
        assert!(dep_info_sources("garbage without a separator").is_none());
    }

    #[test]
    fn a_registry_directory_splits_at_every_dash_that_starts_a_version() {
        assert_eq!(
            split_package_dir("windows-sys-0.52.0"),
            vec![("windows-sys".to_string(), "0.52.0".to_string())]
        );
        assert_eq!(
            split_package_dir("foo-1.0.0-beta.1"),
            vec![("foo".to_string(), "1.0.0-beta.1".to_string())]
        );
        // `sha-1` the crate, or `sha` at a version that is not semver.
        assert_eq!(
            split_package_dir("sha-1-0.10.1"),
            vec![("sha-1".to_string(), "0.10.1".to_string())]
        );
        assert!(split_package_dir("no-version-here").is_empty());
        assert_eq!(
            unit_hash("libserde-06a2225fab2cda3a.rlib"),
            Some("06a2225fab2cda3a")
        );
        assert_eq!(
            unit_hash("serde-06a2225fab2cda3a.d"),
            Some("06a2225fab2cda3a")
        );
        assert_eq!(unit_hash("my-app.exe"), None);
        assert_eq!(
            unit_hash("my_app-1a2b3c4d5e6f7g"),
            None,
            "incremental hashes are not hex"
        );
    }

    #[test]
    fn the_scan_marks_gone_versions_gone_packages_and_idle_caches_and_nothing_else() {
        let fixture = Fixture::new("scan");
        lay_out(&fixture);
        let scan = scan(
            &fixture.target(),
            &fixture.project(),
            &current(),
            ScanOptions::default(),
        );
        let report = &scan.report;
        assert!(report.exists);
        assert!(!report.shared);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.trees.len(), 2);
        let host = report
            .trees
            .iter()
            .find(|t| t.triple.is_none())
            .expect("host tree");
        assert_eq!(host.profile, "debug");
        let xtensa = report
            .trees
            .iter()
            .find(|t| t.triple.is_some())
            .expect("xtensa tree");
        assert_eq!(xtensa.triple.as_deref(), Some("xtensa-esp32-none-elf"));
        assert_eq!(xtensa.profile, "release");

        let stale = scan.stale_paths();
        let names: Vec<String> = stale
            .iter()
            .map(|(p, ..)| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        // The old serde, all of it: three deps files, the fingerprint, the
        // compiled script and its run directory.
        for expected in [
            "libserde-aaaaaaaaaaaaaaaa.rlib",
            "libserde-aaaaaaaaaaaaaaaa.rmeta",
            "serde-aaaaaaaaaaaaaaaa.d",
            "serde-aaaaaaaaaaaaaaaa",
            "serde-ffffffffffffffff",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "{expected} missing from {names:?}"
            );
        }
        assert_eq!(
            names
                .iter()
                .filter(|n| n.as_str() == "serde-aaaaaaaaaaaaaaaa")
                .count(),
            2,
            "the fingerprint directory and the compiled script share the name"
        );
        // The dropped dependency and its run directory, by package.
        assert!(names.contains(&"libleft_pad-dddddddddddddddd.rlib".to_string()));
        assert!(names.contains(&"left-pad-1111111111111111".to_string()));
        // The idle cache and the gone crate's cache, not the fresh one.
        assert!(names.contains(&"my_app-9z8y7x6w5v4u3t".to_string()));
        assert!(names.contains(&"gone_crate-0000000000000".to_string()));
        assert!(!names.contains(&"my_app-1a2b3c4d5e6f7g".to_string()));
        // Live things are not there.
        for live in [
            "libserde-bbbbbbbbbbbbbbbb.rlib",
            "libwindows_sys-cccccccccccccccc.rlib",
            "libmy_app-eeeeeeeeeeeeeeee.rlib",
            "my-app.exe",
        ] {
            assert!(!names.contains(&live.to_string()), "{live} wrongly stale");
        }
        let reasons: HashSet<&str> = stale.iter().map(|(.., r)| reason_name(r)).collect();
        assert_eq!(
            reasons,
            HashSet::from(["version-gone", "package-gone", "idle", "superseded"])
        );
        assert_eq!(
            names.iter().filter(|n| n.starts_with("my_app-v")).count(),
            2,
            "two of five day-old variants fall outside the four kept: {names:?}"
        );

        // The groups add up and say why.
        let deps = host
            .groups
            .iter()
            .find(|g| g.kind == DiskKind::Deps)
            .unwrap();
        assert_eq!(
            deps.stale_bytes,
            5_000
                + 10
                + deps_d_len(&fixture, "serde-aaaaaaaaaaaaaaaa")
                + 800
                + 10
                + deps_d_len(&fixture, "left_pad-dddddddddddddddd")
        );
        assert!(
            deps.stale_by_reason
                .iter()
                .any(|s| s.reason == "version-gone")
        );
        assert!(
            deps.stale_by_reason
                .iter()
                .any(|s| s.reason == "package-gone")
        );
        let incremental = host
            .groups
            .iter()
            .find(|g| g.kind == DiskKind::Incremental)
            .unwrap();
        assert_eq!(incremental.stale_bytes, 400 + 500 + 2 * 100);
        assert_eq!(report.debuginfo_bytes, 4_000);

        // Extras: known ones removable, the unknown one not, files at the top
        // level counted but not listed.
        let labels: Vec<(&str, bool)> = report
            .extras
            .iter()
            .map(|e| (e.label.as_str(), e.removable))
            .collect();
        assert!(labels.contains(&("docs", true)));
        assert!(labels.contains(&("sim-images", true)));
        assert!(labels.contains(&("other", false)));
        assert_eq!(report.extras.len(), 3);
        assert!(report.total_bytes > 0);
    }

    fn deps_d_len(fixture: &Fixture, stem: &str) -> u64 {
        fs::metadata(fixture.target().join(format!("debug/deps/{stem}.d")))
            .unwrap()
            .len()
    }

    #[test]
    fn an_empty_yardstick_judges_no_dependency_and_says_so() {
        let fixture = Fixture::new("empty");
        lay_out(&fixture);
        let scan = scan(
            &fixture.target(),
            &fixture.project(),
            &Current::default(),
            ScanOptions::default(),
        );
        assert_eq!(scan.report.warnings.len(), 1);
        let reasons: HashSet<&str> = scan.stale.iter().map(|s| reason_name(&s.reason)).collect();
        assert_eq!(
            reasons,
            HashSet::from(["idle", "superseded"]),
            "the incremental rules need no graph"
        );
    }

    #[test]
    fn a_sweep_removes_exactly_the_stale_paths_and_reports_the_bytes() {
        let fixture = Fixture::new("sweep");
        lay_out(&fixture);
        let before = scan(
            &fixture.target(),
            &fixture.project(),
            &current(),
            ScanOptions::default(),
        );
        let expected: u64 = before.stale.iter().map(|s| s.bytes).sum();
        let report = sweep(
            &fixture.target(),
            &fixture.project(),
            &current(),
            &SweepPolicy::default(),
        )
        .unwrap();
        assert_eq!(report.removed_bytes, expected);
        assert_eq!(report.removed_items as usize, before.stale.len());
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert!(report.locked.is_empty());
        let target = fixture.target();
        assert!(
            !target
                .join("debug/deps/libserde-aaaaaaaaaaaaaaaa.rlib")
                .exists()
        );
        assert!(!target.join("debug/build/serde-ffffffffffffffff").exists());
        assert!(
            !target
                .join("debug/incremental/my_app-9z8y7x6w5v4u3t")
                .exists()
        );
        assert!(
            target
                .join("debug/deps/libserde-bbbbbbbbbbbbbbbb.rlib")
                .exists()
        );
        assert!(
            target
                .join("debug/incremental/my_app-1a2b3c4d5e6f7g")
                .exists()
        );
        assert!(target.join("debug/my-app.exe").exists());
        assert!(
            target
                .join("xtensa-esp32-none-elf/release/deps/libserde-bbbbbbbbbbbbbbbb.rlib")
                .exists()
        );
        // Nothing left to sweep.
        let after = scan(
            &fixture.target(),
            &fixture.project(),
            &current(),
            ScanOptions::default(),
        );
        assert!(after.stale.is_empty(), "{:?}", after.stale_paths());
    }

    #[test]
    fn a_policy_narrows_the_sweep_to_a_reason_and_a_tree() {
        let fixture = Fixture::new("policy");
        lay_out(&fixture);
        let only_idle = SweepPolicy {
            version_gone: false,
            package_gone: false,
            superseded: false,
            idle_days: Some(7),
            tree: None,
        };
        let report = sweep(
            &fixture.target(),
            &fixture.project(),
            &current(),
            &only_idle,
        )
        .unwrap();
        assert_eq!(report.removed_bytes, 400, "the one idle cache");
        assert!(
            fixture
                .target()
                .join("debug/deps/libserde-aaaaaaaaaaaaaaaa.rlib")
                .exists()
        );

        let other_tree = SweepPolicy {
            tree: Some(
                fixture
                    .target()
                    .join("xtensa-esp32-none-elf/release")
                    .to_string_lossy()
                    .to_string(),
            ),
            ..SweepPolicy::default()
        };
        let report = sweep(
            &fixture.target(),
            &fixture.project(),
            &current(),
            &other_tree,
        )
        .unwrap();
        assert_eq!(report.removed_bytes, 0, "that tree has nothing stale");
        assert!(
            fixture
                .target()
                .join("debug/deps/libserde-aaaaaaaaaaaaaaaa.rlib")
                .exists()
        );
    }

    #[test]
    fn a_locked_tree_is_left_alone() {
        use fs4::fs_std::FileExt;
        let fixture = Fixture::new("locked");
        lay_out(&fixture);
        let lock_path = fixture.target().join("debug/.cargo-lock");
        let lock = fs::File::create(&lock_path).unwrap();
        assert!(lock.try_lock_exclusive().unwrap());
        assert!(tree_locked(&fixture.target().join("debug")));
        let report = sweep(
            &fixture.target(),
            &fixture.project(),
            &current(),
            &SweepPolicy::default(),
        )
        .unwrap();
        assert_eq!(report.removed_bytes, 0);
        assert_eq!(report.locked.len(), 1);
        assert!(
            fixture
                .target()
                .join("debug/deps/libserde-aaaaaaaaaaaaaaaa.rlib")
                .exists()
        );
        let refused = remove_tree(&fixture.target(), &fixture.target().join("debug"));
        assert!(matches!(refused, Err(Error::Refused { .. })));
        FileExt::unlock(&lock).unwrap();
        drop(lock);
        assert!(!tree_locked(&fixture.target().join("debug")));
    }

    #[test]
    fn removing_a_tree_takes_only_what_the_scan_lists() {
        let fixture = Fixture::new("remove");
        lay_out(&fixture);
        let target = fixture.target();
        // Not a tree, not an extra: refused.
        assert!(matches!(
            remove_tree(&target, &target.join("debug/deps")),
            Err(Error::Refused { .. })
        ));
        assert!(matches!(
            remove_tree(&target, &fixture.project().join("src")),
            Err(Error::Refused { .. })
        ));
        // An extra the scan does not know: refused.
        assert!(matches!(
            remove_tree(&target, &target.join("mystery")),
            Err(Error::Refused { .. })
        ));
        assert!(target.join("mystery/thing.bin").exists());
        // A known extra and a whole tree: removed, sizes reported.
        let docs = remove_tree(&target, &target.join("doc")).unwrap();
        assert_eq!(docs.removed_bytes, 1_200);
        assert!(!target.join("doc").exists());
        // A tree's incremental caches alone, leaving its deps.
        let caches = remove_tree(&target, &target.join("debug/incremental")).unwrap();
        assert!(caches.removed_bytes >= 300 + 400 + 500 + 5 * 100);
        assert!(!target.join("debug/incremental").exists());
        assert!(
            target
                .join("debug/deps/libserde-bbbbbbbbbbbbbbbb.rlib")
                .exists()
        );
        let tree = remove_tree(&target, &target.join("xtensa-esp32-none-elf/release")).unwrap();
        assert!(tree.removed_bytes > 2_500);
        assert!(!target.join("xtensa-esp32-none-elf/release").exists());
        assert!(target.join("debug").exists());
    }

    #[test]
    fn a_missing_build_directory_reports_itself_without_a_scan() {
        let fixture = Fixture::new("missing");
        let scan = scan(
            &fixture.target(),
            &fixture.project(),
            &current(),
            ScanOptions::default(),
        );
        assert!(!scan.report.exists);
        assert_eq!(scan.report.total_bytes, 0);
        assert!(scan.report.trees.is_empty());
        assert!(scan.stale.is_empty());
        // The volume is read off the project, which does exist.
        assert!(scan.report.volume.is_some());
    }
}
