//! One version number, in the three places that state it.
//!
//! The workspace manifest is what `rusty-cli --version` and the frontend
//! print, `tauri.conf.json` is what the app and its updater call themselves,
//! and the newest `## v…` heading in CHANGELOG.md is what the release
//! publishes as its notes. The release workflow stamps the tag into the
//! first two, so a release cannot ship the wrong number — but the repository
//! itself said 0.6.12 for twelve releases, and a development build at HEAD
//! called itself that, was offered its own release as an update, and showed
//! "current version 0.6.12" in every screenshot of the update sheet. Bump
//! all three in the release commit; this is what notices when one is missed.

fn workspace_version() -> String {
    // rusty-app inherits the workspace's version, so its own is the
    // workspace's — read at compile time, no manifest parsing needed.
    env!("CARGO_PKG_VERSION").to_string()
}

fn config_version() -> String {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json parses");
    conf["version"]
        .as_str()
        .expect("tauri.conf.json names a version")
        .to_string()
}

fn newest_changelog_version() -> String {
    include_str!("../../../CHANGELOG.md")
        .lines()
        .find_map(|line| line.strip_prefix("## v"))
        .expect("CHANGELOG.md has a `## v<version>` heading")
        .trim()
        .to_string()
}

#[test]
fn the_manifest_the_config_and_the_changelog_agree_on_the_version() {
    let workspace = workspace_version();
    assert_eq!(
        config_version(),
        workspace,
        "tauri.conf.json and [workspace.package].version differ — bump both in the release commit",
    );
    assert_eq!(
        newest_changelog_version(),
        workspace,
        "the newest CHANGELOG heading is not this version — a release commit bumps the \
         version and adds the section together",
    );
}
