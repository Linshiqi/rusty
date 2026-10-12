use std::collections::BTreeSet;

use guppy::{
    PackageId,
    graph::{BuildTargetId, PackageGraph, PackageMetadata, feature::FeatureId},
};

use crate::{error::Result, model::*};

/// One row per workspace member, sorted by name.
pub fn members(graph: &PackageGraph) -> Result<Vec<MemberInfo>> {
    let mut out: Vec<MemberInfo> = graph
        .workspace()
        .iter()
        .map(|pkg| -> Result<MemberInfo> {
            // The transitive closure includes the member itself; the user cares
            // about what it drags in, so subtract it back out.
            let total_deps = graph
                .query_forward([pkg.id()])?
                .resolve()
                .len()
                .saturating_sub(1);

            Ok(MemberInfo {
                name: pkg.name().to_string(),
                version: pkg.version().to_string(),
                manifest_path: pkg.manifest_path().to_string(),
                kinds: kinds_of(&pkg),
                direct_deps: pkg.direct_links().count(),
                total_deps,
                features: declared_features(&pkg),
                default_features: default_expansion(graph, &pkg),
                has_build_script: pkg.has_build_script(),
            })
        })
        .collect::<Result<_>>()?;

    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn vitals(
    graph: &PackageGraph,
    members: &[MemberInfo],
    duplicates: &[DuplicateGroup],
) -> Vitals {
    let ws = graph.workspace();

    let mut kinds = KindCounts::default();
    for pkg in ws.iter() {
        if pkg.is_proc_macro() {
            kinds.proc_macro += 1;
            continue;
        }
        let mut counted_lib = false;
        for target in pkg.build_targets() {
            match target.id() {
                BuildTargetId::Library if !counted_lib => {
                    kinds.lib += 1;
                    counted_lib = true;
                }
                BuildTargetId::Binary(_) => kinds.bin += 1,
                _ => {}
            }
        }
    }

    let workspace_crates = members.len();
    let resolved_deps = graph.packages().filter(|p| !p.in_workspace()).count();

    // A "direct" dependency is one some member names in its own manifest. Count
    // distinct packages, not links — three members depending on serde is one
    // direct dependency, not three.
    let direct: BTreeSet<&PackageId> = ws
        .iter()
        .flat_map(|m| m.direct_links())
        .map(|link| link.to())
        .filter(|p| !p.in_workspace())
        .map(|p| p.id())
        .collect();
    let direct_deps = direct.len();

    // A crate at N versions costs N-1 redundant builds.
    let duplicate_extra_units = duplicates
        .iter()
        .map(|g| g.versions.len().saturating_sub(1))
        .sum();

    Vitals {
        workspace_crates,
        workspace_kinds: kinds,
        resolved_deps,
        direct_deps,
        transitive_deps: resolved_deps.saturating_sub(direct_deps),
        duplicate_groups: duplicates.len(),
        duplicate_extra_units,
        build_scripts: graph.packages().filter(|p| p.has_build_script()).count(),
        proc_macros: graph.packages().filter(|p| p.is_proc_macro()).count(),
    }
}

fn kinds_of(pkg: &PackageMetadata<'_>) -> Vec<String> {
    if pkg.is_proc_macro() {
        return vec!["proc-macro".to_string()];
    }
    let mut kinds = Vec::new();
    for target in pkg.build_targets() {
        let kind = match target.id() {
            BuildTargetId::Library => "lib",
            BuildTargetId::Binary(_) => "bin",
            _ => continue,
        };
        if !kinds.iter().any(|k| k == kind) {
            kinds.push(kind.to_string());
        }
    }
    kinds
}

/// Declared features, with `default` hoisted to the front — it is the one the
/// user reasons about first.
fn declared_features(pkg: &PackageMetadata<'_>) -> Vec<String> {
    let mut features: Vec<String> = pkg.named_features().map(str::to_string).collect();
    features.sort();
    if let Some(pos) = features.iter().position(|f| f == "default") {
        let default = features.remove(pos);
        features.insert(0, default);
    }
    features
}

/// Which of this package's own features are on when `default-features = true`.
///
/// This is the transitive answer, not the one-level manifest text: if
/// `default = ["a"]` and `a = ["b"]`, both `a` and `b` are listed, because both
/// are what the user actually gets.
fn default_expansion<'g>(graph: &'g PackageGraph, pkg: &PackageMetadata<'g>) -> Vec<String> {
    if !pkg.has_default_feature() {
        return Vec::new();
    }
    let feature_graph = graph.feature_graph();
    let mut out: Vec<String> = pkg
        .named_features()
        .filter(|name| *name != "default")
        .filter(|name| {
            feature_graph
                .is_default_feature(FeatureId::named(pkg.id(), name))
                .unwrap_or(false)
        })
        .map(str::to_string)
        .collect();
    out.sort();
    out
}
