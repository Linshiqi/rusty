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

/// One direct dependency, its resolved version, and what crates.io knows.
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

/// Everything the Overview page needs, in one payload.
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

// ─────────────────────────────────────────────────────────────────────────────
// Disk
// ─────────────────────────────────────────────────────────────────────────────

/// Where a project's builds went on disk, and what of it is dead weight.
///
/// Sizes are bytes; every path is absolute and a plain string. What is
/// *stale* is decided by [`crate::disk`]'s rules and summarised here per
/// group; the paths themselves stay on the backend, which recomputes them
/// when asked to sweep rather than trusting a list sent back over the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskReport {
    /// The build directory, as `cargo metadata` names it.
    pub target_dir: String,
    /// True when it is not `<project>/target` — a `build.target-dir` in the
    /// project's or the user's cargo config points builds elsewhere, which is
    /// how several projects share one set of compiled dependencies.
    pub shared: bool,
    /// Whether the build directory exists at all.
    pub exists: bool,
    pub total_bytes: u64,
    pub files: u64,
    /// The volume the build directory sits on.
    pub volume: Option<Volume>,
    /// One per `<triple>/<profile>` (or `<profile>` for the host).
    pub trees: Vec<BuildTree>,
    /// Everything at the top level that is not a build tree: docs, rusty's
    /// own simulator images, temporary and staging directories.
    pub extras: Vec<DiskItem>,
    /// Cargo's own caches under `CARGO_HOME`, for scale — they are usually
    /// small next to the build directory, and saying so stops people
    /// cleaning the wrong thing.
    pub cargo_home: Vec<DiskItem>,
    /// `CARGO_HOME` itself, when known — where a shared build directory
    /// would naturally live.
    pub cargo_home_dir: Option<String>,
    /// Debug symbols kept apart from the binaries they describe (`.pdb` on
    /// MSVC, `.dSYM` on macOS): the one kind of artifact a profile setting
    /// shrinks several-fold.
    pub debuginfo_bytes: u64,
    /// Where the scan could not be sure and therefore marked nothing — a
    /// dep-info file it could not read, a fingerprint in a format it does not
    /// know. Nothing is deleted on a guess, so these are the reasons a
    /// number may be lower than the truth.
    pub warnings: Vec<String>,
}

/// Free and total space on the volume holding the build directory.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    pub free_bytes: u64,
    pub total_bytes: u64,
}

/// One build tree: a profile's artifacts for one target.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildTree {
    /// `None` for the host.
    pub triple: Option<String>,
    /// `debug`, `release`, or a custom profile's name.
    pub profile: String,
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    /// Whether a build currently holds this tree's lock. Nothing in a locked
    /// tree is removed.
    pub locked: bool,
    pub groups: Vec<DiskGroup>,
}

/// One kind of artifact inside a tree, with how much of it is stale and why.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskGroup {
    pub kind: DiskKind,
    pub bytes: u64,
    pub files: u64,
    pub stale_bytes: u64,
    pub stale_files: u64,
    /// The stale bytes by reason, so the view can say *why* rather than
    /// only *how much*.
    pub stale_by_reason: Vec<StaleSummary>,
}

/// What lives in a build tree, by directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiskKind {
    /// `deps/`: every compiled crate, one set of files per unit hash.
    Deps,
    /// `incremental/`: rustc's per-crate incremental caches.
    Incremental,
    /// `build/`: build scripts, compiled and run.
    BuildScripts,
    /// `.fingerprint/`: cargo's freshness records, tiny but numerous.
    Fingerprints,
    /// The final binaries cargo uplifts beside `deps/`, and `examples/`.
    Binaries,
    /// Anything else in the tree.
    Other,
}

/// Why an artifact is judged unnecessary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum StaleReason {
    /// The lockfile resolves this package to another version now; artifacts
    /// of the old one are never read again unless the lockfile moves back.
    VersionGone { package: String, version: String },
    /// The package is no longer in the dependency graph at all.
    PackageGone { package: String },
    /// An incremental cache not touched for this many days. Removing it costs
    /// one non-incremental compile of that crate the next time it changes.
    Idle { days: u32 },
    /// One of a crate's older incremental caches. rustc keys the cache on the
    /// unit's flags, so every distinct feature set, profile override or
    /// wrapper leaves a cache of its own, used again only if exactly that
    /// combination is built again; a hot workspace grows a hundred per crate.
    /// Only the newest `keep` survive. Removing one costs the same as `Idle`.
    Superseded { keep: u32 },
}

/// Stale bytes and files under one reason kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleSummary {
    /// `version-gone`, `package-gone` or `idle` — the reason's kind, so the
    /// frontend translates it by name.
    pub reason: String,
    pub bytes: u64,
    pub files: u64,
}

/// One directory outside the build trees: a top-level extra, or a cargo
/// home cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskItem {
    /// A stable name the frontend translates: `docs`, `sim-images`,
    /// `registry-src`, … — or the directory's own name for one the scan does
    /// not know, which it then also refuses to remove.
    pub label: String,
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    /// Whether removing it is something rusty will do on request. What it
    /// costs to remove is in the label's translation, not guessed here.
    pub removable: bool,
}

/// What a sweep or a removal did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepReport {
    pub removed_bytes: u64,
    pub removed_items: u64,
    /// Trees left alone because a build held their lock.
    pub locked: Vec<String>,
    /// Paths that could not be removed, with the error.
    pub failed: Vec<String>,
}

/// Which stale artifacts a sweep removes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepPolicy {
    /// Artifacts of dependency versions the lockfile no longer resolves.
    pub version_gone: bool,
    /// Artifacts of packages no longer in the graph.
    pub package_gone: bool,
    /// A crate's older incremental caches, beyond the newest few.
    pub superseded: bool,
    /// Incremental caches idle for at least this many days; `None` leaves
    /// them all.
    pub idle_days: Option<u32>,
    /// One tree's path, or `None` for every tree.
    pub tree: Option<String>,
}

impl Default for SweepPolicy {
    fn default() -> Self {
        SweepPolicy {
            version_gone: true,
            package_gone: true,
            superseded: true,
            idle_days: Some(7),
            tree: None,
        }
    }
}
