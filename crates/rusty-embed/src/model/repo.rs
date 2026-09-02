//! Where users go.

/// The repository: source, downloads, and somewhere to report a fault.
///
/// One place, since the source went public. It was two for a while — a
/// private repository that built, and a public one that published — because
/// GitHub answers 404 rather than 403 for a private repository, so an update
/// check pointed at the source failed for every user and said "not found",
/// which reads as "there is no release" rather than "you cannot see this".
///
/// Named here, in the one module both sides compile: `update.rs` is
/// backend-only and the Help menu is wasm. Written out in full rather than
/// concatenated — `concat!` takes literals and not constants, and building
/// them with `format!` would make runtime strings that no longer fit in a
/// `Copy` action.
pub const REPO: &str = "https://github.com/Linshiqi/rusty";
pub const REPO_RELEASES: &str = "https://github.com/Linshiqi/rusty/releases";
pub const REPO_ISSUES: &str = "https://github.com/Linshiqi/rusty/issues/new/choose";

/// The releases API for [`REPO`]. Anonymous calls work because it is public.
pub const RELEASES_API: &str = "https://api.github.com/repos/Linshiqi/rusty/releases/latest";
