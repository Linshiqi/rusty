//! Feature unification simulation — the analysis the whole product is built on.
//!
//! Answering "what does turning this feature off actually cost me?" cannot be
//! done by reading a manifest. Cargo unifies features across the entire
//! workspace, so a feature only stops pulling a crate in when *nothing else*
//! still needs it. guppy can replay cargo's resolver over the in-memory graph,
//! which lets us answer the question in milliseconds instead of a rebuild.

use std::collections::BTreeSet;

use guppy::graph::{
    DependencyDirection, PackageGraph, PackageMetadata,
    cargo::{CargoOptions, CargoResolverVersion},
    feature::{FeatureGraph, FeatureId},
};

use crate::{
    error::{Error, Result},
    model::*,
};

/// The outcome of one simulated resolution.
struct Resolution {
    /// `name version` for every third-party package that would be built.
    packages: BTreeSet<String>,
    /// How many of those are proc-macros or carry a build script. These land on
    /// the build's critical path far more often than their count suggests.
    build_units: usize,
}

/// Simulate `selection` and report what it costs relative to the package's
/// declared defaults.
pub fn impact(graph: &PackageGraph, selection: &FeatureSelection) -> Result<FeatureImpact> {
    let member = member_of(graph, &selection.package)?;

    let baseline_selection = FeatureSelection {
        package: selection.package.clone(),
        features: Vec::new(),
        default_features: true,
    };
    let baseline = resolve(graph, &member, &baseline_selection)?;
    let current = resolve(graph, &member, selection)?;

    Ok(FeatureImpact {
        package: selection.package.clone(),
        selection: selection.clone(),
        resolved_crates: current.packages.len(),
        baseline_crates: baseline.packages.len(),
        delta_crates: current.packages.len() as i32 - baseline.packages.len() as i32,
        added: current
            .packages
            .difference(&baseline.packages)
            .cloned()
            .collect(),
        removed: baseline
            .packages
            .difference(&current.packages)
            .cloned()
            .collect(),
        delta_build_units: current.build_units as i32 - baseline.build_units as i32,
    })
}

/// One row per declared feature, with the cost of flipping that one switch
/// while holding every other feature where it currently is.
pub fn rows(graph: &PackageGraph, selection: &FeatureSelection) -> Result<Vec<FeatureRow>> {
    let member = member_of(graph, &selection.package)?;
    let feature_graph = graph.feature_graph();

    let names: Vec<&str> = member
        .named_features()
        .filter(|name| *name != "default")
        .collect();

    // Expand `default` into concrete feature names so every later resolution
    // can be expressed without it. Anything `default` enables that is not a
    // named feature of this package (a `dep:` or `other-crate/feat` link) is
    // invisible here, which is why the baseline below is re-resolved from the
    // expanded list rather than reusing `selection` directly.
    let enabled: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| is_enabled(&feature_graph, &member, selection, name))
        .collect();

    let baseline = resolve(graph, &member, &explicit(selection, &enabled))?;

    let mut rows = Vec::with_capacity(names.len());
    for &name in &names {
        let on = enabled.contains(&name);
        let flipped: Vec<&str> = if on {
            enabled.iter().copied().filter(|f| *f != name).collect()
        } else {
            enabled.iter().copied().chain([name]).collect()
        };
        let flipped = resolve(graph, &member, &explicit(selection, &flipped))?;

        rows.push(FeatureRow {
            name: name.to_string(),
            enabled: on,
            in_default: feature_graph
                .is_default_feature(FeatureId::named(member.id(), name))
                .unwrap_or(false),
            enables: enables(&feature_graph, &member, name),
            marginal_crates: flipped.packages.len() as i32 - baseline.packages.len() as i32,
        });
    }

    Ok(rows)
}

// ─────────────────────────────────────────────────────────────────────────────

fn member_of<'g>(graph: &'g PackageGraph, name: &str) -> Result<PackageMetadata<'g>> {
    graph
        .workspace()
        .member_by_name(name)
        .map_err(|_| Error::NotAMember {
            name: name.to_string(),
        })
}

/// A selection with `default` already expanded, so flipping one switch does not
/// silently drag `default` along with it.
fn explicit(selection: &FeatureSelection, features: &[&str]) -> FeatureSelection {
    FeatureSelection {
        package: selection.package.clone(),
        features: features.iter().map(|f| f.to_string()).collect(),
        default_features: false,
    }
}

fn is_enabled(
    feature_graph: &FeatureGraph<'_>,
    member: &PackageMetadata<'_>,
    selection: &FeatureSelection,
    name: &str,
) -> bool {
    if selection.features.iter().any(|f| f == name) {
        return true;
    }
    selection.default_features
        && feature_graph
            .is_default_feature(FeatureId::named(member.id(), name))
            .unwrap_or(false)
}

/// Which of this package's own features `name` directly turns on.
///
/// Cross-package links (`sqlx/postgres`) and `dep:` activations live on the
/// other package's node and need manifest text to attribute properly — those
/// are not reported here yet.
fn enables<'g>(
    feature_graph: &FeatureGraph<'g>,
    member: &PackageMetadata<'g>,
    name: &'g str,
) -> Vec<String> {
    let from = FeatureId::named(member.id(), name);
    let mut out: Vec<String> = member
        .named_features()
        .filter(|other| *other != name)
        .filter(|other| {
            feature_graph
                .directly_depends_on(from, FeatureId::named(member.id(), other))
                .unwrap_or(false)
        })
        .map(str::to_string)
        .collect();
    out.sort();
    out
}

fn resolve<'g>(
    graph: &'g PackageGraph,
    member: &PackageMetadata<'g>,
    selection: &FeatureSelection,
) -> Result<Resolution> {
    let cargo_set = graph
        .feature_graph()
        .query_forward(feature_ids(member, selection)?)?
        .resolve()
        .into_cargo_set(&options())?;

    let mut packages = BTreeSet::new();
    let mut build_units = 0;

    // Target and host graphs are resolved separately under resolver v2, and a
    // crate can legitimately appear in both. Deduplicate by identity so the
    // count matches what cargo would actually build.
    for (_platform, features) in cargo_set.all_features() {
        for pkg in features
            .to_package_set()
            .packages(DependencyDirection::Forward)
        {
            if pkg.in_workspace() {
                continue;
            }
            if packages.insert(format!("{} {}", pkg.name(), pkg.version()))
                && (pkg.is_proc_macro() || pkg.has_build_script())
            {
                build_units += 1;
            }
        }
    }

    Ok(Resolution {
        packages,
        build_units,
    })
}

fn feature_ids<'g>(
    member: &PackageMetadata<'g>,
    selection: &FeatureSelection,
) -> Result<Vec<FeatureId<'g>>> {
    let package_id = member.id();
    let mut ids = vec![FeatureId::base(package_id)];

    if selection.default_features && member.has_default_feature() {
        ids.push(FeatureId::named(package_id, "default"));
    }

    for wanted in &selection.features {
        // Borrow the name out of the graph rather than the request, so the
        // resulting FeatureId lives as long as the graph does.
        let name = member
            .named_features()
            .find(|name| *name == wanted.as_str())
            .ok_or_else(|| Error::UnknownFeature {
                package: selection.package.clone(),
                feature: wanted.clone(),
            })?;
        ids.push(FeatureId::named(package_id, name));
    }

    Ok(ids)
}

fn options() -> CargoOptions<'static> {
    let mut options = CargoOptions::new();
    // V2 and V3 resolve *features* identically — V3 only adds MSRV-aware
    // version selection, which has already happened by the time cargo hands us
    // a resolved graph.
    options.set_resolver(CargoResolverVersion::V2);
    // Dev-dependencies are not part of what you ship; including them would make
    // every feature look more expensive than it is.
    options.set_include_dev(false);
    options
}
