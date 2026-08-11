use std::collections::BTreeMap;

use guppy::{
    Version,
    graph::{PackageGraph, PackageLink, PackageMetadata},
};

use crate::model::*;

/// Find every crate that resolved to more than one version, and for each
/// version, who asked for it.
///
/// Workspace members are skipped — a workspace cannot contain the same crate
/// twice, so any name collision there is not the duplication users mean.
pub fn analyze(graph: &PackageGraph) -> Vec<DuplicateGroup> {
    let mut by_name: BTreeMap<&str, Vec<PackageMetadata<'_>>> = BTreeMap::new();
    for pkg in graph.packages() {
        if pkg.in_workspace() {
            continue;
        }
        by_name.entry(pkg.name()).or_default().push(pkg);
    }

    by_name
        .into_iter()
        .filter(|(_, pkgs)| pkgs.len() > 1)
        .map(|(name, mut pkgs)| {
            // Newest first — that is usually the one to unify on.
            pkgs.sort_by(|a, b| b.version().cmp(a.version()));

            let unifiable = pkgs
                .windows(2)
                .all(|pair| compat_range(pair[0].version()) == compat_range(pair[1].version()));

            DuplicateGroup {
                name: name.to_string(),
                versions: pkgs.iter().map(provenance_of).collect(),
                unifiable,
            }
        })
        .collect()
}

fn provenance_of(pkg: &PackageMetadata<'_>) -> DuplicateVersion {
    let mut pulled_by: Vec<Provenance> = pkg
        .reverse_direct_links()
        .map(|link| {
            let from = link.from();
            Provenance {
                package: from.name().to_string(),
                version: from.version().to_string(),
                req: link.version_req().to_string(),
                kind: dep_kind(&link),
                is_workspace_member: from.in_workspace(),
            }
        })
        .collect();

    // Surface the user's own crates first — those are the requirements they can
    // actually edit.
    pulled_by.sort_by(|a, b| {
        b.is_workspace_member
            .cmp(&a.is_workspace_member)
            .then_with(|| a.package.cmp(&b.package))
            .then_with(|| a.version.cmp(&b.version))
    });

    DuplicateVersion {
        version: pkg.version().to_string(),
        id: pkg.id().to_string(),
        pulled_by,
    }
}

fn dep_kind(link: &PackageLink<'_>) -> DepKind {
    // A link can be present in several sections at once; report the one that
    // affects the shipped artifact most.
    if link.normal().is_present() {
        DepKind::Normal
    } else if link.build().is_present() {
        DepKind::Build
    } else {
        DepKind::Dev
    }
}

/// Cargo's semver-compatibility bucket for a version.
///
/// Two versions unify only if they land in the same bucket: `1.2.3` and `1.9.0`
/// both bucket to `1`, but `0.21.7` and `0.22.1` bucket to `0.21` and `0.22`
/// and can never be merged without a dependency moving.
fn compat_range(v: &Version) -> String {
    if v.major != 0 {
        format!("{}", v.major)
    } else if v.minor != 0 {
        format!("0.{}", v.minor)
    } else {
        format!("0.0.{}", v.patch)
    }
}
