//! Judging one unit against the graph: the sources its dep-info file names,
//! where those come from — a registry, a git checkout, the workspace — and
//! whether that package at that version is still resolved. For build
//! scripts, cargo's fingerprint records, which tie a run to its script.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::Current;
use super::fs::sorted_entries;
use crate::model::StaleReason;

pub(super) enum Verdict {
    Live,
    Stale(StaleReason),
}

/// Read a unit's dep-info file and judge it against the graph.
pub(super) fn verdict_of(dep_info: &Path, current: &Current) -> Result<Verdict, String> {
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
pub(super) fn dep_info_sources(text: &str) -> Option<Vec<String>> {
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
pub(super) enum Origin {
    /// `<registry>/src/<index>/<name>-<version>/…`
    Registry { dir: String },
    /// `<cargo home>/git/checkouts/<repo>-<hash>/<rev>/…`
    Git { repo: String, rev: String },
    /// A path in the workspace or a path dependency.
    Local,
}

pub(super) fn origin_of(sources: &[String]) -> Origin {
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
pub(super) fn split_package_dir(dir: &str) -> Vec<(String, String)> {
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
pub(super) fn unit_hash(file_name: &str) -> Option<&str> {
    let stem = file_name
        .split_once('.')
        .map_or(file_name, |(stem, _)| stem);
    let (_, hash) = stem.rsplit_once('-')?;
    (hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(hash)
}

/// The package a `<pkg>-<hash>` directory belongs to.
pub(super) fn package_of_unit(dir_name: &str) -> String {
    dir_name
        .rsplit_once('-')
        .map_or(dir_name, |(name, _)| name)
        .to_string()
}

/// Every compiled build script's fingerprint hash → its unit hash, read from
/// `.fingerprint/<pkg>-<hash>/build-script-build-script-build`, whose content
/// is the fingerprint's hash in hex.
pub(super) fn script_fingerprints(fingerprint_dir: &Path) -> HashMap<u64, String> {
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
pub(super) fn run_script_hash(fingerprint_dir: &Path, package: &str, hash: &str) -> Option<u64> {
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
