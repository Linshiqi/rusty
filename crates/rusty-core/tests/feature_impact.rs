//! Feature resolution is the analysis the product is built on, so it gets
//! tested against a real workspace rather than a mock graph.
//!
//! Assertions are deliberately about *relationships* (this selection removes
//! serde) rather than exact crate counts, which would break every time an
//! upstream crate splits a dependency out.

use rusty_core::{FeatureSelection, Workspace};

fn lab() -> Workspace {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/feature-lab");
    Workspace::load(path).expect("fixture workspace should load")
}

fn select(features: &[&str], default_features: bool) -> FeatureSelection {
    FeatureSelection {
        package: "feature-lab".to_string(),
        features: features.iter().map(|f| f.to_string()).collect(),
        default_features,
    }
}

fn mentions(list: &[String], crate_name: &str) -> bool {
    list.iter()
        .any(|entry| entry.split(' ').next() == Some(crate_name))
}

#[test]
fn default_selection_is_its_own_baseline() {
    let impact = lab().feature_impact(&select(&[], true)).unwrap();
    assert_eq!(impact.delta_crates, 0);
    assert!(impact.added.is_empty());
    assert!(impact.removed.is_empty());
    assert_eq!(impact.resolved_crates, impact.baseline_crates);
}

#[test]
fn disabling_defaults_removes_the_crates_they_gated() {
    let impact = lab().feature_impact(&select(&[], false)).unwrap();

    assert!(
        impact.delta_crates < 0,
        "--no-default-features should shrink the graph, got {:+}",
        impact.delta_crates
    );
    assert!(
        mentions(&impact.removed, "serde"),
        "json gated serde, so it should be removed: {:?}",
        impact.removed
    );
    assert!(
        mentions(&impact.removed, "camino"),
        "migrations -> paths gated camino: {:?}",
        impact.removed
    );
    assert!(impact.added.is_empty());
}

#[test]
fn enabling_an_off_feature_pulls_its_subtree_in() {
    let impact = lab().feature_impact(&select(&["iter"], true)).unwrap();

    assert!(impact.delta_crates > 0);
    assert!(
        mentions(&impact.added, "itertools"),
        "iter gates itertools: {:?}",
        impact.added
    );
    assert!(impact.removed.is_empty());
}

/// The property that separates real feature-graph analysis from reading the
/// manifest: `migrations` enables `paths`, so turning off *either one alone*
/// changes nothing — the other still pulls camino in. Only turning off both
/// actually removes it.
///
/// This is exactly the confusion the feature matrix exists to make visible, so
/// if it ever regresses to naive arithmetic the numbers become lies.
#[test]
fn coupled_features_only_pay_off_when_both_are_off() {
    let workspace = lab();

    let rows = workspace.feature_rows(&select(&[], true)).unwrap();
    let row = |name: &str| {
        rows.iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("feature-lab should declare `{name}`"))
    };

    assert!(row("migrations").enabled && row("migrations").in_default);
    assert!(row("paths").enabled && row("paths").in_default);

    assert_eq!(
        row("migrations").marginal_crates,
        0,
        "paths is still on, so dropping migrations alone frees nothing"
    );
    assert_eq!(
        row("paths").marginal_crates,
        0,
        "migrations re-enables paths, so dropping paths alone frees nothing"
    );

    // Both off together is the selection that actually pays.
    let both_off = workspace.feature_impact(&select(&["json"], false)).unwrap();
    assert!(
        mentions(&both_off.removed, "camino"),
        "dropping migrations and paths together should free camino: {:?}",
        both_off.removed
    );
}

#[test]
fn feature_rows_report_what_each_feature_enables() {
    let rows = lab().feature_rows(&select(&[], true)).unwrap();

    let migrations = rows.iter().find(|r| r.name == "migrations").unwrap();
    assert_eq!(migrations.enables, vec!["paths".to_string()]);

    let json = rows.iter().find(|r| r.name == "json").unwrap();
    assert!(
        json.enables.is_empty(),
        "json only activates a `dep:`, not another named feature"
    );

    // `default` is the switch the whole panel hangs off, not a row in it.
    assert!(!rows.iter().any(|r| r.name == "default"));

    let off_by_default = rows.iter().find(|r| r.name == "graph").unwrap();
    assert!(!off_by_default.enabled && !off_by_default.in_default);
    assert!(
        off_by_default.marginal_crates > 0,
        "turning graph on should cost crates"
    );
}

#[test]
fn unknown_names_are_rejected_with_a_useful_message() {
    let err = lab()
        .feature_impact(&select(&["postgres"], true))
        .unwrap_err()
        .to_string();
    assert!(err.contains("postgres"), "{err}");

    let err = lab()
        .feature_impact(&FeatureSelection {
            package: "not-a-member".to_string(),
            features: vec![],
            default_features: true,
        })
        .unwrap_err()
        .to_string();
    assert!(err.contains("not-a-member"), "{err}");
}

#[test]
fn workspace_report_describes_the_fixture() {
    let report = lab().report().unwrap();

    assert_eq!(report.vitals.workspace_crates, 1);
    assert_eq!(report.members.len(), 1);

    let member = &report.members[0];
    assert_eq!(member.name, "feature-lab");
    assert_eq!(member.kinds, vec!["lib".to_string()]);
    // Sorted, with `default` hoisted to the front.
    assert_eq!(member.features.first().unwrap(), "default");
    assert!(member.default_features.contains(&"paths".to_string()));
}
