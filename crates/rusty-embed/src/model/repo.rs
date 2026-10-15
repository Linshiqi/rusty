//! Where users go.

/// The repository: source, downloads, and somewhere to report a fault.
///
/// One place, since the source went public. It was two for a while — a
/// private repository that built, and a public one that published — because
/// GitHub answers 404 rather than 403 for a private repository, so an update
/// check pointed at the source failed for every user and said "not found",
/// which reads as "there is no release" rather than "you cannot see this".
///
/// Named once: the updater's feed (`update::feed_url`) and the Help
/// menu's links are built from these. Written out in full rather than
/// concatenated — `concat!` takes literals and not constants.
pub const REPO: &str = "https://github.com/Linshiqi/rusty";
pub const REPO_RELEASES: &str = "https://github.com/Linshiqi/rusty/releases";
pub const REPO_ISSUES: &str = "https://github.com/Linshiqi/rusty/issues/new/choose";
