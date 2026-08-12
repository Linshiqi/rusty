//! The wire contract between `rusty-core` and every frontend.
//!
//! These types are the single source of truth for the Tauri backend, the CLI,
//! and the Leptos frontend — which `use` them directly rather than through
//! generated bindings, so the contract cannot drift.
//!
//! This module must stay free of IO and of anything that will not compile to
//! `wasm32-unknown-unknown`: it is the only part of the crate the frontend
//! links against. Versions and paths are plain strings so the JSON stays
//! decoupled from whatever crate versions the backend happens to build with.

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Top level
// ─────────────────────────────────────────────────────────────────────────────

/// Everything the Overview page needs, in one payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrateRow {
    pub name: String,
    /// The version the lockfile resolved.
    pub current: String,
    /// The newest stable version crates.io lists, when it answered.
    pub latest: Option<String>,
    /// Why `latest` is absent — an unreachable index is a normal state for
    /// these machines, and it lands here rather than as fake data.
    pub note: Option<String>,
}

/// One direct dependency, its resolved version, and what crates.io knows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceReport {
    pub workspace: WorkspaceInfo,
    pub vitals: Vitals,
    pub members: Vec<MemberInfo>,
    pub duplicates: Vec<DuplicateGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    /// Absolute path to the directory holding the workspace `Cargo.toml`.
    pub root: String,
    /// Best-effort display name: the root package, else the directory name.
    pub name: String,
    /// Highest edition declared by any member.
    pub edition: Option<String>,
    /// Highest `rust-version` declared by any member — the effective MSRV.
    pub rust_version: Option<String>,
    /// The target triple the analysis was resolved for.
    pub target_platform: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Overview vitals — the six readouts on the home screen
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vitals {
    /// Number of crates in the workspace itself.
    pub workspace_crates: usize,
    pub workspace_kinds: KindCounts,
    /// Every package in the resolved graph, excluding workspace members.
    pub resolved_deps: usize,
    /// Third-party packages depended on directly by at least one member.
    pub direct_deps: usize,
    /// `resolved_deps - direct_deps`.
    pub transitive_deps: usize,
    /// Crate names that resolved to more than one version.
    pub duplicate_groups: usize,
    /// Redundant compilation units caused by those duplicates.
    ///
    /// A crate at 3 versions contributes 2 — the extra builds you would not
    /// pay for if the tree were unified.
    pub duplicate_extra_units: usize,
    /// Packages that ship a build script, which serialize the build graph.
    pub build_scripts: usize,
    /// Proc-macro crates, which must be built for the host even when
    /// cross-compiling.
    pub proc_macros: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KindCounts {
    pub lib: usize,
    pub bin: usize,
    pub proc_macro: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Workspace members
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberInfo {
    pub name: String,
    pub version: String,
    pub manifest_path: String,
    /// `lib`, `bin`, `proc-macro`, and so on, as declared by the targets.
    pub kinds: Vec<String>,
    /// Direct dependencies declared by this member, third-party and internal.
    pub direct_deps: usize,
    /// Size of this member's transitive dependency closure, excluding itself.
    pub total_deps: usize,
    /// Every feature this member declares, sorted, `default` first if present.
    pub features: Vec<String>,
    /// What `default` expands to, one level deep.
    pub default_features: Vec<String>,
    pub has_build_script: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Duplicate versions — "why do I have two base64?"
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub name: String,
    pub versions: Vec<DuplicateVersion>,
    /// True when every resolved version falls in the same semver-compatible
    /// range, which means cargo *could* have unified them and something
    /// (usually a lockfile pin or a `=` requirement) stopped it.
    ///
    /// False means the versions are genuinely incompatible and unifying them
    /// requires a dependency to move, not a `cargo update`.
    pub unifiable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateVersion {
    pub version: String,
    /// Opaque package id — stable within one analysis, use it to cross-link.
    pub id: String,
    /// Who asked for this particular version. This is the answer to the
    /// question the user actually has.
    pub pulled_by: Vec<Provenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub package: String,
    pub version: String,
    /// The requirement as written in that package's manifest, e.g. `^0.21`.
    pub req: String,
    pub kind: DepKind,
    /// True when the requirement comes from the user's own workspace — those
    /// are the ones they can actually change.
    pub is_workspace_member: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

// ─────────────────────────────────────────────────────────────────────────────
// Feature impact — the live matrix
// ─────────────────────────────────────────────────────────────────────────────

/// A feature selection to simulate, mirroring cargo's own flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureSelection {
    /// Workspace member to resolve for.
    pub package: String,
    /// Features to turn on, as in `--features`.
    #[serde(default)]
    pub features: Vec<String>,
    /// As in the absence of `--no-default-features`.
    #[serde(default = "default_true")]
    pub default_features: bool,
}

fn default_true() -> bool {
    true
}

/// What a feature selection costs, relative to that package's defaults.
///
/// The counts come from a real cargo resolution simulated over the whole
/// workspace under resolver v2, so feature unification is already applied —
/// turning a feature off only removes a crate if nothing else still needs it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureImpact {
    pub package: String,
    pub selection: FeatureSelection,
    /// Packages in the resolved graph under this selection.
    pub resolved_crates: usize,
    /// Packages resolved under the package's default features.
    pub baseline_crates: usize,
    /// `resolved_crates - baseline_crates`.
    ///
    /// `i32`, not `i64`: a dependency-count delta cannot overflow it, and a
    /// 64-bit integer would generate a TypeScript `bigint` that never matches
    /// the plain JSON number actually sent over the wire.
    pub delta_crates: i32,
    /// Crates this selection pulls in that the baseline does not.
    pub added: Vec<String>,
    /// Crates the baseline pulls in that this selection does not.
    pub removed: Vec<String>,
    /// Change in the number of proc-macro and build-script crates. These land
    /// on the build's critical path far more often than their count suggests,
    /// so a small positive number here can cost more wall clock than a large
    /// `delta_crates`.
    pub delta_build_units: i32,
}

/// One row of the feature matrix: a declared feature and what it costs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureRow {
    pub name: String,
    /// Whether this feature is on under the selection this row was computed
    /// for — the switch position in the UI.
    pub enabled: bool,
    /// True when `default` enables this feature, directly or transitively.
    pub in_default: bool,
    /// Other features of the same package this one directly turns on.
    pub enables: Vec<String>,
    /// What flipping this one switch costs, holding every other feature where
    /// it is. Positive means crates get added by flipping, negative means
    /// crates get removed — so an enabled feature that pulls its weight shows a
    /// negative number, and a disabled one that would be expensive shows a
    /// positive one.
    pub marginal_crates: i32,
}
