//! The updater's configuration, pinned to the code that names the repository.
//!
//! `plugins.updater.endpoints` pointed at `Linshiqi/rusty-releases` — the
//! public mirror from when the source was private — for ten releases after
//! the feed had moved beside the source, and nothing said so: the check in
//! use went to the GitHub API by a constant of its own, and the endpoint was
//! read by nobody. Now that the updater reads it, a wrong one is an app that
//! can never find an update and never says why.

use base64::Engine as _;

#[test]
fn the_updater_reads_the_feed_beside_the_source_and_carries_a_real_key() {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json parses");
    let updater = &conf["plugins"]["updater"];

    let endpoints = updater["endpoints"]
        .as_array()
        .expect("the updater names its endpoints");
    assert_eq!(endpoints.len(), 1, "one feed, the release's own");
    assert_eq!(
        endpoints[0].as_str().unwrap(),
        format!("{}/latest/download/latest.json", rusty_embed::REPO_RELEASES),
        "the feed is the newest release's latest.json in the repository the code names",
    );

    let pubkey = updater["pubkey"].as_str().expect("a public key");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(pubkey)
        .expect("the key is base64 of a minisign public key file");
    let text = String::from_utf8(decoded).expect("a text key file");
    assert!(
        text.starts_with("untrusted comment: minisign public key: "),
        "not a minisign public key: {text}",
    );
    assert_eq!(
        text.lines().count(),
        2,
        "a comment line and the key line, nothing else",
    );

    assert_eq!(
        updater["windows"]["installMode"], "passive",
        "the installer shows its progress and restarts the app; quiet would hide a failure",
    );
}
