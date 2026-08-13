//! Is there a newer rusty?
//!
//! Checking is separate from installing on purpose. Installing in place
//! needs the signing keypair — without a verified signature an updater is a
//! remote code execution feature — so until that exists this asks GitHub
//! what the latest release is and hands the user the page. The version
//! comparison and the plumbing are the same either way; only the last step
//! changes when signing lands.
//!
//! Over the same proxy ladder every other fetch uses: a workbench that
//! cannot check for updates behind a corporate proxy has not checked.

use crate::model::UpdateStatus;

/// The releases API for the repository this is built from.
const LATEST: &str = "https://api.github.com/repos/Linshiqi/rusty/releases/latest";

/// The running version, and the newest published one if it could be read.
///
/// A failed check is a note, not an error: no network is the normal state of
/// a workbench on a bench, and a modal about it would be noise.
pub fn check() -> UpdateStatus {
    let current = env!("CARGO_PKG_VERSION").to_string();

    match fetch_latest() {
        Ok((latest, url)) => {
            let newer = is_newer(&latest, &current);
            UpdateStatus {
                newer,
                latest: Some(latest),
                url: Some(url),
                note: None,
                current,
            }
        }
        Err(note) => UpdateStatus {
            newer: false,
            latest: None,
            url: None,
            note: Some(note),
            current,
        },
    }
}

/// `(version, release page)` from the releases API.
fn fetch_latest() -> Result<(String, String), String> {
    let mut last = "no route to github".to_string();
    for route in crate::simulate::proxy_routes() {
        let mut builder = ureq::Agent::config_builder()
            .timeout_connect(Some(std::time::Duration::from_secs(10)))
            .timeout_global(Some(std::time::Duration::from_secs(30)));
        if let Some(url) = &route
            && let Ok(proxy) = ureq::Proxy::new(url)
        {
            builder = builder.proxy(Some(proxy));
        }
        let agent: ureq::Agent = builder.build().into();

        // GitHub refuses anonymous calls without a user agent.
        match agent
            .get(LATEST)
            .header("User-Agent", "rusty-workbench")
            .header("Accept", "application/vnd.github+json")
            .call()
        {
            Ok(mut response) => match response.body_mut().read_to_string() {
                Ok(body) => return parse_release(&body),
                Err(error) => last = format!("could not read GitHub's answer: {error}"),
            },
            Err(error) => last = format!("{error}"),
        }
    }
    Err(last)
}

/// The two fields that matter, without pulling a JSON schema for the rest.
fn parse_release(body: &str) -> Result<(String, String), String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("GitHub sent something unreadable: {e}"))?;
    let tag = value
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "GitHub's answer carried no tag_name".to_string())?;
    let url = value
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://github.com/Linshiqi/rusty/releases/latest");
    Ok((tag.trim_start_matches('v').to_string(), url.to_string()))
}

/// Compare `a.b.c`, ignoring a `v` and anything after a dash.
///
/// Its own arithmetic rather than a semver dependency: the comparison is
/// six lines, and the cases that matter — 0.10 beats 0.9, equal is not
/// newer, a pre-release suffix does not make a version bigger — are pinned
/// by the test below.
fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(version: &str) -> Vec<u64> {
        version
            .trim_start_matches('v')
            .split('-')
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|piece| piece.parse().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parts(candidate), parts(current));
    for index in 0..a.len().max(b.len()) {
        let (left, right) = (
            a.get(index).copied().unwrap_or(0),
            b.get(index).copied().unwrap_or(0),
        );
        if left != right {
            return left > right;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_numeric_not_lexical() {
        assert!(is_newer("0.10.0", "0.9.0"), "ten beats nine");
        assert!(is_newer("v1.0.0", "0.9.9"), "a leading v is not part of it");
        assert!(is_newer("0.2.1", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0"), "the same version is not an update");
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(
            !is_newer("0.2.0-rc1", "0.2.0"),
            "a pre-release of a version already installed is not an upgrade",
        );
        assert!(is_newer("0.2", "0.1.9"), "missing pieces count as zero");
    }

    #[test]
    fn a_release_answer_yields_version_and_page() {
        let body = r#"{"tag_name":"v0.3.0","html_url":"https://example.invalid/r/v0.3.0"}"#;
        let (version, url) = parse_release(body).expect("parsed");
        assert_eq!(version, "0.3.0", "the v is stripped for comparison");
        assert!(url.ends_with("v0.3.0"));

        assert!(
            parse_release(r#"{"message":"Not Found"}"#).is_err(),
            "an answer without a tag is a failure, not a silent zero version",
        );
    }
}
