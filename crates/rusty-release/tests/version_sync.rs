//! One version number, in the two places that state it.
//!
//! The workspace manifest is what the window, `rusty-cli --version` and the
//! updater call themselves, and the newest `## v…` heading in CHANGELOG.md
//! is what the release publishes as its notes and the feed carries as the
//! update's. The release workflow stamps the tag into the manifest, so a
//! release cannot ship the wrong number — but the repository itself said
//! 0.6.12 for twelve releases, and a development build at HEAD called itself
//! that and was offered its own release as an update. Bump both in the
//! release commit; this is what notices when one is missed.

fn newest_changelog_version() -> String {
    include_str!("../../../CHANGELOG.md")
        .lines()
        .find_map(|line| line.strip_prefix("## v"))
        .expect("CHANGELOG.md has a `## v<version>` heading")
        .trim()
        .to_string()
}

#[test]
fn the_manifest_and_the_changelog_agree_on_the_version() {
    // Every crate inherits the workspace's version, so this one's is it.
    assert_eq!(
        newest_changelog_version(),
        env!("CARGO_PKG_VERSION"),
        "the newest CHANGELOG heading is not this version — a release commit bumps the \
         version and adds the section together",
    );
}
