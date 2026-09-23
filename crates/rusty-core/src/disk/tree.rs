//! One build tree, counted group by group — `deps/`, build scripts,
//! fingerprints, incremental caches and everything else — with each group's
//! stale share judged as it is counted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::fs::{measure, newest_mtime, sorted_entries, tree_locked};
use super::judge::{
    Verdict, package_of_unit, run_script_hash, script_fingerprints, unit_hash, verdict_of,
};
use super::{Current, ScanOptions, Stale};
use crate::model::{BuildTree, DiskGroup, DiskKind, StaleReason, StaleSummary};

/// One incremental cache: its directory, bytes, files, and when rustc last
/// wrote to it.
type Variant = (PathBuf, u64, u64, Option<SystemTime>);

/// What every tree in one scan is judged against, and what the trees add up
/// to between them.
pub(super) struct TreeScan<'a> {
    pub(super) current: &'a Current,
    pub(super) options: ScanOptions,
    /// The stale paths of every tree measured so far, tree after tree.
    pub(super) stale: Vec<Stale>,
    pub(super) warnings: Vec<String>,
    pub(super) debuginfo_bytes: u64,
}

impl TreeScan<'_> {
    /// Measure one build tree and judge what in it is stale.
    pub(super) fn analyze(
        &mut self,
        path: &Path,
        triple: Option<&str>,
        profile: &str,
    ) -> BuildTree {
        let mut tally = Tally {
            tree: path,
            groups: HashMap::new(),
            found: Vec::new(),
        };
        let mut stale_hashes = self.deps(&mut tally);
        self.build_scripts(&mut tally, &mut stale_hashes);
        Self::fingerprints(&mut tally, &stale_hashes);
        self.incremental(&mut tally);
        self.everything_else(&mut tally);

        let (groups, stale) = tally.finish();
        self.stale.extend(stale);
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

    /// Units in `deps/`, grouped by the hash cargo gives every file of one
    /// unit — `libserde-06a2…rlib`, `serde-06a2….d`, `serde-06a2….rmeta` are
    /// one unit — so a verdict on the dep-info file covers the unit. Answers
    /// with the hashes found stale and why, for the fingerprints to follow.
    fn deps(&mut self, tally: &mut Tally<'_>) -> HashMap<String, StaleReason> {
        let current = self.current;
        let deps_dir = tally.tree.join("deps");
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
            tally.count(DiskKind::Deps, bytes, 1);
            if is_debuginfo(&name) {
                self.debuginfo_bytes += bytes;
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
                    Err(detail) => self
                        .warnings
                        .push(format!("{}: {detail}", dep_info.display())),
                }
            }
        }
        for (hash, reason) in &stale_hashes {
            for (file, bytes) in units.get(hash).into_iter().flatten() {
                tally.stale(DiskKind::Deps, file, *bytes, 1, reason);
            }
        }
        stale_hashes
    }

    /// Build scripts: `build/<pkg>-<hash>/` is either the compiled script
    /// (judged by its own dep-info, like any unit) or the directory it ran in,
    /// linked to its script through cargo's fingerprint record — or, when
    /// that record cannot be read, judged only by whether the package is in
    /// the graph at all. The hashes found stale join `stale_hashes`.
    fn build_scripts(
        &mut self,
        tally: &mut Tally<'_>,
        stale_hashes: &mut HashMap<String, StaleReason>,
    ) {
        let current = self.current;
        let build_dir = tally.tree.join("build");
        let fingerprint_dir = tally.tree.join(".fingerprint");
        let script_hash_by_fingerprint = script_fingerprints(&fingerprint_dir);
        let mut compile_verdicts: HashMap<String, StaleReason> = HashMap::new();
        let mut run_dirs: Vec<(PathBuf, String, u64, u64)> = Vec::new();
        for entry in sorted_entries(&build_dir) {
            let dir = entry.path();
            if !dir.is_dir() {
                let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                tally.count(DiskKind::BuildScripts, bytes, 1);
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let (bytes, files) = measure(&dir);
            tally.count(DiskKind::BuildScripts, bytes, files);
            self.debuginfo_bytes += debuginfo_in(&dir);
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
                        tally.stale(DiskKind::BuildScripts, &dir, bytes, files, &reason);
                        stale_hashes.insert(hash.to_string(), reason);
                    }
                    Ok(Verdict::Live) => {}
                    Err(detail) => self
                        .warnings
                        .push(format!("{}: {detail}", dep_info.display())),
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
                tally.stale(DiskKind::BuildScripts, &dir, bytes, files, &reason);
                stale_hashes.insert(hash, reason);
            }
        }
    }

    /// Fingerprints follow their units.
    fn fingerprints(tally: &mut Tally<'_>, stale_hashes: &HashMap<String, StaleReason>) {
        for entry in sorted_entries(&tally.tree.join(".fingerprint")) {
            let dir = entry.path();
            let (bytes, files) = measure(&dir);
            tally.count(DiskKind::Fingerprints, bytes, files);
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(hash) = unit_hash(&name)
                && let Some(reason) = stale_hashes.get(hash)
            {
                tally.stale(DiskKind::Fingerprints, &dir, bytes, files, reason);
            }
        }
    }

    /// Incremental caches: named for the crate, judged by whether the crate
    /// is still local to this workspace, by when rustc last wrote to the
    /// cache — and, since rustc keys the cache on the unit's flags and a hot
    /// workspace grows a hundred variants per crate, by whether it is among
    /// the crate's newest few.
    fn incremental(&self, tally: &mut Tally<'_>) {
        let (current, options) = (self.current, self.options);
        let idle_after = Duration::from_secs(u64::from(options.idle_days) * 86_400);
        let now = SystemTime::now();
        let mut variants: HashMap<String, Vec<Variant>> = HashMap::new();
        for entry in sorted_entries(&tally.tree.join("incremental")) {
            let dir = entry.path();
            let (bytes, files) = measure(&dir);
            tally.count(DiskKind::Incremental, bytes, files);
            let name = entry.file_name().to_string_lossy().to_string();
            let Some((crate_name, _)) = name.rsplit_once('-') else {
                continue;
            };
            if !current.is_empty() && !current.local.contains(crate_name) {
                tally.stale(
                    DiskKind::Incremental,
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
                tally.stale(
                    DiskKind::Incremental,
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
                tally.stale(
                    DiskKind::Incremental,
                    dir,
                    *bytes,
                    *files,
                    &StaleReason::Superseded {
                        keep: options.keep_variants,
                    },
                );
            }
        }
    }

    /// Everything else in the tree: the uplifted binaries and examples, and
    /// whatever a tool put there.
    fn everything_else(&mut self, tally: &mut Tally<'_>) {
        for entry in sorted_entries(tally.tree) {
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
                self.debuginfo_bytes += bytes;
            }
            let kind = if file.is_dir() && name != "examples" {
                DiskKind::Other
            } else {
                DiskKind::Binaries
            };
            tally.count(kind, bytes, files);
        }
    }
}

/// One tree as it is measured: a group for each kind of artifact in it, and
/// the paths found stale.
struct Tally<'a> {
    tree: &'a Path,
    groups: HashMap<DiskKind, DiskGroup>,
    /// The tree's stale paths, in the order they were found.
    found: Vec<Stale>,
}

impl Tally<'_> {
    /// Count `bytes` and `files` under `kind`, creating the group on first use.
    fn count(&mut self, kind: DiskKind, bytes: u64, files: u64) {
        let group = self.groups.entry(kind).or_insert_with(|| DiskGroup {
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

    /// Count `path` as stale in its group, under `reason`, and list it for a
    /// sweep to remove.
    fn stale(&mut self, kind: DiskKind, path: &Path, bytes: u64, files: u64, reason: &StaleReason) {
        if let Some(group) = self.groups.get_mut(&kind) {
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
        self.found.push(Stale {
            tree: self.tree.to_path_buf(),
            path: path.to_path_buf(),
            bytes,
            reason: reason.clone(),
        });
    }

    /// The groups, largest first and each with its largest reason first, and
    /// the stale paths behind them.
    fn finish(self) -> (Vec<DiskGroup>, Vec<Stale>) {
        let mut groups: Vec<DiskGroup> = self.groups.into_values().collect();
        groups.sort_by_key(|group| std::cmp::Reverse(group.bytes));
        for group in &mut groups {
            group
                .stale_by_reason
                .sort_by_key(|summary| std::cmp::Reverse(summary.bytes));
        }
        (groups, self.found)
    }
}

pub(super) fn reason_name(reason: &StaleReason) -> &'static str {
    match reason {
        StaleReason::VersionGone { .. } => "version-gone",
        StaleReason::PackageGone { .. } => "package-gone",
        StaleReason::Idle { .. } => "idle",
        StaleReason::Superseded { .. } => "superseded",
    }
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
