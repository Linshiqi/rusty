//! What crates.io knows about the versions this workspace holds.
//!
//! The sparse index (`index.crates.io`) is a static CDN of JSON lines — no
//! auth, no rate ceremony, one GET per crate. Unreachable is a normal state
//! for the machines this workbench serves, so every failure lands as a note
//! on that crate's row rather than as fake data or a dead panel.

use std::sync::Mutex;

use guppy::graph::PackageGraph;
use semver::Version;

use crate::model::CrateRow;

/// The unique direct registry dependencies of the workspace's members, as
/// (name, resolved version). Direct because that is what a person can act on
/// — their own `Cargo.toml` — and the duplicates panel already covers the
/// transitive story.
pub fn direct_dependencies(graph: &PackageGraph) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for member in graph.workspace().iter() {
        for link in member.direct_links() {
            let to = link.to();
            if to.in_workspace() || !to.source().is_crates_io() {
                continue;
            }
            let name = to.name().to_string();
            let version = to.version().to_string();
            match out.iter_mut().find(|(n, _)| *n == name) {
                // Two members resolving different versions is the duplicates
                // panel's story; here the higher one stands for the crate.
                Some((_, existing)) => {
                    if compare_semverish(&version, existing) == std::cmp::Ordering::Greater {
                        *existing = version;
                    }
                }
                None => out.push((name, version)),
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Ask the sparse index for each crate's newest stable version, a few
/// threads at a time. One row per input, always — a crate the index will
/// not answer for gets its reason, not a hole.
pub fn annotate_latest(deps: Vec<(String, String)>, proxy: Option<String>) -> Vec<CrateRow> {
    let results: Mutex<Vec<CrateRow>> = Mutex::new(Vec::with_capacity(deps.len()));
    let work: Mutex<std::vec::IntoIter<(String, String)>> = Mutex::new(deps.into_iter());

    std::thread::scope(|scope| {
        for _ in 0..6 {
            let proxy = proxy.clone();
            let work = &work;
            let results = &results;
            scope.spawn(move || {
                let mut builder = ureq::Agent::config_builder()
                    .timeout_connect(Some(std::time::Duration::from_secs(8)))
                    .timeout_global(Some(std::time::Duration::from_secs(30)));
                if let Some(url) = proxy.as_deref()
                    && let Ok(proxy) = ureq::Proxy::new(url)
                {
                    builder = builder.proxy(Some(proxy));
                }
                let agent: ureq::Agent = builder.build().into();
                loop {
                    let Some((name, current)) = work.lock().expect("work queue").next() else {
                        break;
                    };
                    let row = match latest_of(&agent, &name) {
                        Ok(latest) => CrateRow {
                            name,
                            current,
                            latest: Some(latest),
                            note: None,
                        },
                        Err(note) => CrateRow {
                            name,
                            current,
                            latest: None,
                            note: Some(note),
                        },
                    };
                    results.lock().expect("results").push(row);
                }
            });
        }
    });

    let mut rows = results.into_inner().expect("results");
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

/// The newest non-yanked, non-prerelease version the index lists.
fn latest_of(agent: &ureq::Agent, name: &str) -> Result<String, String> {
    let url = format!("https://index.crates.io/{}", index_path(name));
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|e| format!("index unreachable: {e}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("index read failed: {e}"))?;

    let mut best: Option<Version> = None;
    for line in body.lines() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if entry["yanked"].as_bool() == Some(true) {
            continue;
        }
        let Some(vers) = entry["vers"].as_str() else {
            continue;
        };
        let Ok(version) = Version::parse(vers) else {
            continue;
        };
        if !version.pre.is_empty() {
            continue;
        }
        if best.as_ref().is_none_or(|b| version > *b) {
            best = Some(version);
        }
    }
    best.map(|v| v.to_string())
        .ok_or_else(|| "the index lists no stable version".to_string())
}

/// The sparse index's path scheme: length buckets, then prefix directories.
fn index_path(name: &str) -> String {
    let lower = name.to_lowercase();
    match lower.len() {
        0 => lower,
        1 => format!("1/{lower}"),
        2 => format!("2/{lower}"),
        3 => format!("3/{}/{lower}", &lower[..1]),
        _ => format!("{}/{}/{lower}", &lower[..2], &lower[2..4]),
    }
}

/// Best-effort semver ordering for strings already known to be versions.
fn compare_semverish(a: &str, b: &str) -> std::cmp::Ordering {
    match (Version::parse(a), Version::parse(b)) {
        (Ok(a), Ok(b)) => a.cmp(&b),
        _ => a.cmp(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_path_scheme_matches_the_spec() {
        assert_eq!(index_path("a"), "1/a");
        assert_eq!(index_path("io"), "2/io");
        assert_eq!(index_path("syn"), "3/s/syn");
        assert_eq!(index_path("serde"), "se/rd/serde");
        assert_eq!(index_path("Inflector"), "in/fl/inflector");
    }

    #[test]
    fn semverish_ordering_prefers_real_semver() {
        use std::cmp::Ordering;
        assert_eq!(compare_semverish("1.10.0", "1.9.0"), Ordering::Greater);
        assert_eq!(compare_semverish("0.4.33", "0.4.4"), Ordering::Greater);
    }
}
