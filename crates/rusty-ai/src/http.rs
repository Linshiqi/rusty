//! One HTTP client policy for every request this crate makes.
//!
//! Three things were wrong with `reqwest::Client::new()` in three places, and
//! each read as something else from the outside. **No proxy:** the workbench
//! has a proxy setting, the tool installer and the update check honour it, and
//! the assistant — on exactly the machines that need one — could not reach its
//! endpoint, with a timeout for a diagnosis. **No timeout:** an SSE stream
//! that stalled stalled `ai_ask` for ever, and the panel sat on its spinner.
//! **No retry:** a 429 from a rate-limited key or a 503 from a busy server was
//! the end of the question, when the same request a second later would have
//! answered.
//!
//! The proxy is *passed in*. `rusty_embed::net::effective_proxy()` is the one
//! reading of the setting and of the system, and this crate takes what it
//! decided rather than deciding again — `None` here means direct, not "let
//! reqwest detect something", because a user who set "none" meant none.

use std::time::Duration;

use crate::error::{Error, Result};

/// How this crate reaches the network. Filled in by the host from the
/// workbench's settings; `Default` is a direct connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Http {
    /// The proxy to route through — `http://`, `https://` or `socks5://` — or
    /// `None` for a direct connection.
    pub proxy: Option<String>,
}

/// Longest a connection may take to be established. A blackholed proxy has to
/// fail fast enough that the user learns of it from an error rather than from
/// a spinner.
const CONNECT: Duration = Duration::from_secs(20);

/// Longest the socket may go silent mid-response. Reset on every byte, so a
/// slow model that keeps talking is fine and a stalled one is an error. Two
/// minutes because reasoning models do go quiet for a while before the first
/// token, and a gap that long with nothing on the wire is a dead stream.
const IDLE: Duration = Duration::from_secs(120);

/// A whole streamed answer: `max_tokens` at a slow model, plus the gaps.
pub(crate) const CHAT_TOTAL: Duration = Duration::from_secs(15 * 60);

/// A one-shot query — a model list, a check. Anything longer is a failure
/// that should be reported as one.
pub(crate) const PROBE_TOTAL: Duration = Duration::from_secs(30);

/// A client with the timeouts above and the proxy given, or none at all.
pub(crate) fn client(http: &Http, total: Duration) -> Result<reqwest::Client> {
    let builder = reqwest::Client::builder()
        .connect_timeout(CONNECT)
        .read_timeout(IDLE)
        .timeout(total);
    let builder = match &http.proxy {
        Some(url) => builder.proxy(reqwest::Proxy::all(url.as_str()).map_err(|source| {
            Error::HttpClient {
                detail: format!("the proxy setting `{url}` is not a proxy URL this client can use"),
                source: Box::new(source),
            }
        })?),
        // Not reqwest's own detection: the setting has already been read by
        // the host, and "none" has to mean none.
        None => builder.no_proxy(),
    };
    builder.build().map_err(|source| Error::HttpClient {
        detail: "the TLS backend could not be initialised".to_string(),
        source: Box::new(source),
    })
}

/// Send, and on a 429 or a 5xx send once more after a pause.
///
/// Once, and only before a byte of the body has been read: a retry after the
/// stream has started would replay whatever had already reached the screen.
/// The pause honours `Retry-After` when the server names one and is otherwise
/// a second; either is capped, because a server asking for an hour is a server
/// to report, not to wait for.
pub(crate) async fn send_retrying(
    request: reqwest::RequestBuilder,
    endpoint: &str,
) -> Result<reqwest::Response> {
    // Cloned before sending, because sending consumes the builder. A body that
    // cannot be cloned — a stream — means no retry, which none of this crate's
    // requests are.
    let again = request.try_clone();
    let response = request
        .send()
        .await
        .map_err(|e| Error::transport(endpoint, e))?;
    if !retryable(response.status().as_u16()) {
        return Ok(response);
    }
    let Some(again) = again else {
        return Ok(response);
    };
    let wait = backoff(
        response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
    );
    tokio::time::sleep(wait).await;
    again
        .send()
        .await
        .map_err(|e| Error::transport(endpoint, e))
}

/// Rate limiting and server-side failure. A 4xx other than 429 is about the
/// request, and will not change on a second try.
pub(crate) fn retryable(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

/// How long to wait before the one retry: the server's `Retry-After` in
/// seconds when it gave one, a second when it did not, and never more than
/// ten — past that the user is better told than kept waiting. A `Retry-After`
/// given as a date rather than seconds is treated as not given.
pub(crate) fn backoff(retry_after: Option<&str>) -> Duration {
    const DEFAULT: Duration = Duration::from_secs(1);
    const CAP: Duration = Duration::from_secs(10);
    retry_after
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map_or(DEFAULT, Duration::from_secs)
        .min(CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_one_retry_waits_what_the_server_asked_within_reason() {
        assert_eq!(
            backoff(None),
            Duration::from_secs(1),
            "no header, one second"
        );
        assert_eq!(backoff(Some("3")), Duration::from_secs(3));
        assert_eq!(backoff(Some(" 0 ")), Duration::ZERO, "zero is an answer");
        assert_eq!(
            backoff(Some("3600")),
            Duration::from_secs(10),
            "an hour is a server to report, not to wait for",
        );
        assert_eq!(
            backoff(Some("Wed, 21 Oct 2026 07:28:00 GMT")),
            Duration::from_secs(1),
            "a date is not parsed; the default stands",
        );
    }

    #[test]
    fn only_rate_limits_and_server_failures_are_retried() {
        assert!(retryable(429));
        assert!(retryable(500));
        assert!(retryable(503));
        assert!(!retryable(200));
        assert!(!retryable(400), "a bad request stays bad");
        assert!(!retryable(401), "a rejected key stays rejected");
        assert!(!retryable(404));
    }

    /// The failure mode this guards: a proxy setting that is not a URL used to
    /// be discovered as a transport error naming the *endpoint*, which sent
    /// people to check their base URL.
    #[test]
    fn a_proxy_that_is_not_a_url_is_named_as_the_problem() {
        let error = client(
            &Http {
                proxy: Some("not a proxy".into()),
            },
            PROBE_TOTAL,
        )
        .expect_err("an unusable proxy must not build a client");
        assert!(
            matches!(&error, Error::HttpClient { detail, .. } if detail.contains("not a proxy")),
            "{error}",
        );

        client(&Http::default(), PROBE_TOTAL).expect("direct always builds");
        client(
            &Http {
                proxy: Some("http://127.0.0.1:7890".into()),
            },
            PROBE_TOTAL,
        )
        .expect("an http proxy URL builds");
        client(
            &Http {
                proxy: Some("socks5://127.0.0.1:7890".into()),
            },
            PROBE_TOTAL,
        )
        .expect("a socks5 proxy URL builds — the feature is on for this reason");
    }
}
