//! rusty updating itself: the release feed, whether it offers anything
//! newer, and whether a download is what the release was signed as.
//!
//! The feed is `latest.json` on the newest GitHub release — the format the
//! Tauri updater polled, kept so that a copy of rusty from before the window
//! moved to Slint finds, verifies and installs this one: `version`, `notes`
//! (the release's CHANGELOG section), `pub_date`, and per platform the
//! artifact's `url` and its minisign `signature`, base64 of the signature
//! file. The key is the one those copies carry ([`PUBLIC_KEY`]); the release
//! workflow signs with its secret half.
//!
//! **Nothing is installed that was not signed for the version announced.**
//! The feed itself is not signed, so a version number in it proves nothing
//! about the file its `url` names: the signature's trusted comment carries
//! the version the release workflow signed, and a signature for another
//! version is refused (`verify`).

use std::path::Path;

use serde::Deserialize;

use crate::model::{REPO_RELEASES, UpdateProgress};

/// The public half of the updater key: base64 of minisign's public key
/// file, as the Tauri updater's configuration carried it.
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDVGQTY5NkI1QUI2MzlEMDYKUldRR25XT3J0WmFtWDFYTW9EUjh6NGYxYkEzK1JlYmVVbUdnbnB3Y0xJbHZDZ1Nla0ZmL1Z3WHUK";

/// Where the feed is: the newest release's `latest.json`. A debug build
/// takes `RUSTY_UPDATE_FEED` instead, which is how the whole flow is tried
/// without a release.
pub fn feed_url() -> String {
    if cfg!(debug_assertions)
        && let Ok(url) = std::env::var("RUSTY_UPDATE_FEED")
    {
        return url;
    }
    format!("{REPO_RELEASES}/latest/download/latest.json")
}

/// The feed, as the release workflow writes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Feed {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pub_date: Option<String>,
    #[serde(default)]
    pub platforms: std::collections::BTreeMap<String, Artifact>,
}

/// One platform's download and its signature.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub signature: String,
}

impl Feed {
    pub fn parse(text: &str) -> Result<Feed, String> {
        serde_json::from_str(text).map_err(|e| format!("the release feed does not read: {e}"))
    }

    /// The artifact for the machine this runs on, when the release has one.
    pub fn artifact(&self) -> Option<&Artifact> {
        self.platforms.get(platform())
    }

    /// The day it was published, `YYYY-MM-DD`.
    pub fn date(&self) -> Option<String> {
        self.pub_date
            .as_deref()
            .and_then(|date| date.get(..10))
            .map(str::to_string)
    }
}

/// The feed's name for this machine: `windows-x86_64`, `darwin-aarch64`,
/// `linux-x86_64` — Tauri's, so one feed serves both kinds of install.
pub fn platform() -> &'static str {
    if cfg!(windows) {
        "windows-x86_64"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "darwin-aarch64"
        } else {
            "darwin-x86_64"
        }
    } else if cfg!(target_arch = "aarch64") {
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
/// none is from before versions were recorded, and is refused: the feed's
/// own number cannot be trusted alone.
pub fn verify(data: &[u8], signature: &str, version: &str) -> Result<(), String> {
    verify_with(data, signature, version, PUBLIC_KEY)
}

fn verify_with(data: &[u8], signature: &str, version: &str, key: &str) -> Result<(), String> {
    use base64::Engine;
    let text = |b64: &str, what: &str| -> Result<String, String> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| format!("the {what} is not base64: {e}"))?;
        String::from_utf8(bytes).map_err(|_| format!("the {what} is not text"))
    };
    let key = minisign_verify::PublicKey::decode(&text(key, "public key")?)
        .map_err(|e| format!("the public key does not read: {e}"))?;
    let signature = minisign_verify::Signature::decode(&text(signature, "signature")?)
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
        "pub_date": "2026-10-20T08:00:00Z",
        "platforms": {
            "windows-x86_64": { "signature": "c2ln", "url": "https://example.com/setup.exe" },
            "linux-x86_64": { "signature": "c2ln", "url": "https://example.com/rusty.AppImage" }
        }
    }"#;

    #[test]
    fn the_feed_reads_as_the_release_workflow_writes_it() {
        let feed = Feed::parse(FEED).unwrap();
        assert_eq!(feed.version, "0.7.0");
        assert_eq!(feed.date().as_deref(), Some("2026-10-20"));
        assert_eq!(feed.platforms.len(), 2);
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
        use base64::Engine;
        let b64 = |s: String| base64::engine::general_purpose::STANDARD.encode(s);
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let key = b64(pair.pk.to_box().unwrap().to_string());
        let sign = |data: &[u8], comment: &str| {
            let signed = minisign::sign(Some(&pair.pk), &pair.sk, data, Some(comment), None);
            b64(signed.unwrap().to_string())
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
        let stranger = b64(other.pk.to_box().unwrap().to_string());
        assert!(
            verify_with(data, &signature, "1.2.3", &stranger).is_err(),
            "another key"
        );
        let unversioned = sign(data, "timestamp:1760000000\tfile:rusty-setup.exe");
        assert!(
            verify_with(data, &unversioned, "1.2.3", &key).is_err(),
            "no version named"
        );
        assert!(verify_with(data, "not base64!", "1.2.3", &key).is_err());
    }

    /// The key in the code is the one the updater configuration carried,
    /// which every copy of rusty installed before this one checks against.
    #[test]
    fn the_key_is_a_minisign_public_key() {
        use base64::Engine;
        let text = base64::engine::general_purpose::STANDARD
            .decode(PUBLIC_KEY)
            .unwrap();
        let text = String::from_utf8(text).unwrap();
        assert!(minisign_verify::PublicKey::decode(&text).is_ok(), "{text}");
    }
}
