//! rusty updating itself: the release feed, whether it offers anything
//! newer, and whether a download is what the release was signed as.
//!
//! The feed is [`FEED_FILE`] on the newest GitHub release, written by the
//! release workflow: the `version`, its `notes` (the release's CHANGELOG
//! section), when it was `published`, and per platform the download's
//! `url` and its `signature` — the text of the `.minisig` file published
//! beside it, so a download can be checked by hand with `minisign -Vm
//! <file> -P <PUBLIC_KEY>` as well as by rusty. The release workflow signs
//! with the secret half of [`PUBLIC_KEY`].
//!
//! **Nothing is installed that was not signed for the version announced.**
//! The feed itself is not signed, so a version number in it proves nothing
//! about the file its `url` names: the signature's trusted comment carries
//! the version the release workflow signed, and a signature for another
//! version is refused (`verify`).

use std::path::Path;

use serde::Deserialize;

use crate::model::{REPO_RELEASES, UpdateProgress};

/// The public half of the updater key, as minisign prints it.
pub const PUBLIC_KEY: &str = "RWQGnWOrtZamX1XMoDR8z4f1bA3+RebeUmGgnpwcLIlvCgSekFf/VwXu";

/// The feed's name among the newest release's files.
pub const FEED_FILE: &str = "rusty-update.json";

/// Where the feed is. A debug build takes `RUSTY_UPDATE_FEED` instead,
/// which is how the whole flow is tried without a release.
pub fn feed_url() -> String {
    if cfg!(debug_assertions)
        && let Ok(url) = std::env::var("RUSTY_UPDATE_FEED")
    {
        return url;
    }
    format!("{REPO_RELEASES}/latest/download/{FEED_FILE}")
}

/// The feed, as the release workflow writes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Feed {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    /// RFC 3339.
    #[serde(default)]
    pub published: Option<String>,
    /// Each platform's download, by [`platform`]'s name for it.
    #[serde(default)]
    pub downloads: std::collections::BTreeMap<String, Artifact>,
}

/// One platform's download and its signature.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Artifact {
    pub url: String,
    /// The `.minisig` file's text.
    pub signature: String,
}

impl Feed {
    pub fn parse(text: &str) -> Result<Feed, String> {
        serde_json::from_str(text).map_err(|e| format!("the release feed does not read: {e}"))
    }

    /// The download for the machine this runs on, when the release has one.
    pub fn artifact(&self) -> Option<&Artifact> {
        self.downloads.get(platform())
    }

    /// The day it was published, `YYYY-MM-DD`.
    pub fn date(&self) -> Option<String> {
        self.published
            .as_deref()
            .and_then(|date| date.get(..10))
            .map(str::to_string)
    }
}

/// The feed's name for this machine. One macOS download serves both
/// architectures: the app is universal.
pub fn platform() -> &'static str {
    let arm = cfg!(target_arch = "aarch64");
    if cfg!(windows) {
        if arm {
            "windows-aarch64"
        } else {
            "windows-x86_64"
        }
    } else if cfg!(target_os = "macos") {
        "macos-universal"
    } else if arm {
        "linux-aarch64"
    } else {
        "linux-x86_64"
    }
}

/// Whether `offered` is a later release than `current`. A version that
/// does not read as one is never newer: a feed nobody can compare is not
/// an update.
pub fn is_newer(current: &str, offered: &str) -> bool {
    let read = |v: &str| semver::Version::parse(v.trim().trim_start_matches('v')).ok();
    match (read(current), read(offered)) {
        (Some(current), Some(offered)) => offered > current,
        _ => false,
    }
}

/// Fetch the feed, over every route the proxy settings allow.
pub fn fetch_feed() -> Result<Feed, String> {
    fetch_feed_from(&feed_url())
}

pub fn fetch_feed_from(url: &str) -> Result<Feed, String> {
    use std::{io::Read, time::Duration};
    let mut last = String::new();
    for route in crate::net::proxy_routes() {
        let agent = crate::net::agent(
            route.as_deref(),
            crate::net::Deadlines {
                connect: Duration::from_secs(10),
                headers: Some(Duration::from_secs(20)),
                total: Duration::from_secs(60),
            },
        );
        match agent.get(url).call() {
            Ok(response) => {
                let mut text = String::new();
                response
                    .into_body()
                    .into_reader()
                    .take(4 << 20)
                    .read_to_string(&mut text)
                    .map_err(|e| format!("{url}: {e}"))?;
                return Feed::parse(&text);
            }
            Err(error) => last = format!("{url}: {}", crate::net::error_chain(&error)),
        }
    }
    Err(last)
}

/// Download `artifact` to `dest`, resuming where a route stopped, with the
/// byte counts as they arrive.
pub fn download(
    artifact: &Artifact,
    dest: &Path,
    progress: impl FnMut(UpdateProgress),
) -> Result<(), String> {
    crate::install::download_counted(std::slice::from_ref(&artifact.url), dest, |_| {}, progress)
        .map_err(|e| e.to_string())
}

/// Whether `data` is what the release workflow signed for `version`: the
/// signature verifies against [`PUBLIC_KEY`], and its trusted comment —
/// covered by the signature — names that version. A signature that names
/// none is refused: the feed's own number cannot be trusted alone.
pub fn verify(data: &[u8], signature: &str, version: &str) -> Result<(), String> {
    verify_with(data, signature, version, PUBLIC_KEY)
}

fn verify_with(data: &[u8], signature: &str, version: &str, key: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::from_base64(key.trim())
        .map_err(|e| format!("the public key does not read: {e}"))?;
    let signature = minisign_verify::Signature::decode(signature)
        .map_err(|e| format!("the signature does not read: {e}"))?;
    key.verify(data, &signature, true)
        .map_err(|e| format!("the download is not what was signed: {e}"))?;
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"));
    match signed {
        Some(signed) if signed.trim_start_matches('v') == version.trim_start_matches('v') => Ok(()),
        Some(signed) => Err(format!(
            "the download was signed for {signed}, not the {version} the feed announced"
        )),
        None => Err("the signature names no version".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"{
        "version": "0.7.0",
        "notes": "Changes",
        "published": "2026-10-20T08:00:00Z",
        "downloads": {
            "windows-x86_64": { "signature": "sig", "url": "https://example.com/setup.exe" },
            "linux-x86_64": { "signature": "sig", "url": "https://example.com/rusty.AppImage" }
        }
    }"#;

    #[test]
    fn the_feed_reads_as_the_release_workflow_writes_it() {
        let feed = Feed::parse(FEED).unwrap();
        assert_eq!(feed.version, "0.7.0");
        assert_eq!(feed.date().as_deref(), Some("2026-10-20"));
        assert_eq!(feed.downloads.len(), 2);
        assert!(Feed::parse("{}").is_err(), "a feed without a version");
    }

    #[test]
    fn newer_is_semver_and_a_version_that_does_not_read_is_never_newer() {
        assert!(is_newer("0.6.77", "0.7.0"));
        assert!(is_newer("0.6.9", "0.6.10"), "not string order");
        assert!(!is_newer("0.7.0", "0.7.0"));
        assert!(!is_newer("0.7.0", "v0.6.99"));
        assert!(!is_newer("0.7.0", "latest"));
        assert!(is_newer("0.7.0-rc.1", "0.7.0"));
    }

    /// Signed as the release workflow signs: a key of its own, a trusted
    /// comment carrying the version — and held to both.
    #[test]
    fn a_signature_is_held_to_the_key_and_to_the_version_it_names() {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let key = pair.pk.to_base64();
        let sign = |data: &[u8], comment: &str| {
            let signed = minisign::sign(Some(&pair.pk), &pair.sk, data, Some(comment), None);
            signed.unwrap().to_string()
        };
        let data = b"rusty".as_slice();
        let signature = sign(
            data,
            "timestamp:1760000000\tfile:rusty-setup.exe\tversion:1.2.3",
        );
        assert_eq!(verify_with(data, &signature, "1.2.3", &key), Ok(()));
        assert_eq!(verify_with(data, &signature, "v1.2.3", &key), Ok(()));
        assert!(
            verify_with(data, &signature, "1.2.4", &key).is_err(),
            "another version"
        );
        assert!(
            verify_with(b"rusty?", &signature, "1.2.3", &key).is_err(),
            "other bytes"
        );
        let other = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        assert!(
            verify_with(data, &signature, "1.2.3", &other.pk.to_base64()).is_err(),
            "another key"
        );
        let unversioned = sign(data, "timestamp:1760000000\tfile:rusty-setup.exe");
        assert!(
            verify_with(data, &unversioned, "1.2.3", &key).is_err(),
            "no version named"
        );
        assert!(verify_with(data, "not a signature", "1.2.3", &key).is_err());
    }

    #[test]
    fn the_key_is_a_minisign_public_key() {
        assert!(minisign_verify::PublicKey::from_base64(PUBLIC_KEY).is_ok());
    }
}
