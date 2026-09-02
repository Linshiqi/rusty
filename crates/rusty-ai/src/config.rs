//! Turning a stored provider profile into a live client, and checking one.
//!
//! The configuration *types* and the preset table live in [`crate::model`] so
//! the settings screen can render them in wasm before any backend call. What
//! remains here needs a socket.
//!
//! The key is *passed in*, resolved by the host from the credential store. A
//! keychain read is blocking IO and the host is the one with a blocking thread
//! to run it on; it also means everything here can be exercised against a
//! loopback socket on a machine with no credential store at all — which is
//! what a CI runner is, and how the tests below run.

use serde::Deserialize;

use crate::{
    error::{Error, Result},
    http::{self, Http},
    model::{NotCheckedReason, ProviderCheck, ProviderConfig, ProviderKind},
    provider::{
        self, Provider,
        anthropic::{API_VERSION, Anthropic},
        openai::OpenAiCompatible,
    },
};

/// The key a request will carry, or why none can be made.
///
/// Local runtimes (Ollama, LM Studio, vLLM) usually need no key at all, so a
/// missing secret is only an error when the endpoint is remote.
pub fn resolve_key(config: &ProviderConfig, stored: Option<String>) -> Result<Option<String>> {
    match stored {
        Some(key) => Ok(Some(key)),
        None if is_local(&config.base_url) => Ok(None),
        None => Err(Error::MissingKey {
            profile: config.profile.clone(),
        }),
    }
}

/// Build a live provider around the stored key.
pub fn build(
    config: &ProviderConfig,
    stored_key: Option<String>,
    http: &Http,
) -> Result<Box<dyn Provider>> {
    // A local server that insists on a bearer header at all usually accepts
    // any non-empty one; the placeholder is what makes those work.
    let api_key = resolve_key(config, stored_key)?.unwrap_or_else(|| "not-needed".to_string());
    let client = http::client(http, http::CHAT_TOTAL)?;

    Ok(match config.kind {
        ProviderKind::OpenAiCompatible => Box::new(OpenAiCompatible::new(
            &config.profile,
            &config.base_url,
            &config.model,
            api_key,
            config.supports_tools,
            client,
        )),
        ProviderKind::Anthropic => Box::new(Anthropic::new(
            &config.profile,
            &config.base_url,
            &config.model,
            api_key,
            client,
        )),
    })
}

fn is_local(base_url: &str) -> bool {
    base_url.contains("localhost") || base_url.contains("127.0.0.1") || base_url.contains("[::1]")
}

/// Ask the endpoint which models it serves.
///
/// This is why presets ship without hardcoded model lists: `GET /models` works
/// on every compatible server — including self-hosted ones whose model names we
/// could not possibly know — and never goes stale. Anthropic serves the same
/// path under its own headers, and gets a page size: its default page is
/// twenty, and a check that missed the model on page two would call it absent.
pub async fn list_models(
    config: &ProviderConfig,
    stored_key: Option<String>,
    http: &Http,
) -> Result<Vec<String>> {
    let key = resolve_key(config, stored_key)?;
    let client = http::client(http, http::PROBE_TOTAL)?;
    let base = config.base_url.trim_end_matches('/');

    let (endpoint, request) = match config.kind {
        ProviderKind::OpenAiCompatible => {
            let endpoint = format!("{base}/models");
            let mut request = client.get(&endpoint);
            if let Some(key) = &key {
                request = request.bearer_auth(key);
            }
            (endpoint, request)
        }
        ProviderKind::Anthropic => {
            let endpoint = format!("{base}/models?limit=1000");
            let mut request = client
                .get(&endpoint)
                .header("anthropic-version", API_VERSION);
            if let Some(key) = &key {
                request = request.header("x-api-key", key);
            }
            (endpoint, request)
        }
    };

    let response = http::send_retrying(request, &endpoint).await?;
    let response = provider::check_status(response, &config.profile).await?;

    #[derive(Deserialize)]
    struct Models {
        #[serde(default)]
        data: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        id: String,
    }

    let models: Models = response
        .json()
        .await
        .map_err(|e| Error::protocol(&config.profile, e.to_string()))?;

    let mut ids: Vec<String> = models.data.into_iter().map(|m| m.id).collect();
    ids.sort();
    Ok(ids)
}

/// Verify a profile end to end without starting a conversation.
///
/// One real request, with the key, and the answer is what that request
/// established — nothing is inferred from a request that failed. A refused
/// key, an unreachable host and a timeout come back as the errors they are.
/// The one status that is *not* an error is a `/models` that does not exist:
/// some gateways serve chat and nothing else, and that is reported as
/// unchecked rather than as either verdict.
pub async fn check(
    config: &ProviderConfig,
    stored_key: Option<String>,
    http: &Http,
) -> Result<ProviderCheck> {
    match list_models(config, stored_key, http).await {
        Ok(models) => Ok(verdict(&config.model, &models)),
        Err(Error::Http { status, .. }) if status == 404 || status == 405 => {
            Ok(ProviderCheck::NotChecked {
                model: config.model.clone(),
                why: NotCheckedReason::NoModelListing { status },
            })
        }
        Err(error) => Err(error),
    }
}

/// What a model list says about the configured model. Pure, so the rule is a
/// test rather than something discovered against a live endpoint.
pub fn verdict(model: &str, models: &[String]) -> ProviderCheck {
    ProviderCheck::Reachable {
        model: model.to_string(),
        models_listed: models.len(),
        // An empty list is an endpoint that lists nothing, not one that lists
        // everything but this; there is nothing to check against.
        model_listed: (!models.is_empty()).then(|| models.iter().any(|m| m == model)),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    use super::*;

    fn profile(base_url: &str, kind: ProviderKind) -> ProviderConfig {
        ProviderConfig {
            profile: "test".into(),
            kind,
            base_url: base_url.into(),
            model: "m1".into(),
            max_tokens: 16,
            temperature: None,
            supports_tools: true,
        }
    }

    fn json_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len(),
        )
    }

    /// A server on a loopback port: one canned response per connection, in
    /// order, each connection closed after its answer. Returns the base URL
    /// and the request heads it saw — how many, and what they carried.
    fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let heads = Arc::clone(&seen);
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut socket, _)) = listener.accept() else {
                    return;
                };
                // The whole head before answering: a response to a request
                // still being written is a reset on the client's side.
                let mut head = Vec::new();
                let mut chunk = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&chunk[..n]),
                    }
                }
                heads
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head).into_owned());
                let _ = socket.write_all(response.as_bytes());
            }
        });
        (base, seen)
    }

    #[test]
    fn a_missing_key_is_only_an_error_away_from_the_machine() {
        let remote = profile("https://api.example.com/v1", ProviderKind::OpenAiCompatible);
        assert!(matches!(
            resolve_key(&remote, None),
            Err(Error::MissingKey { profile }) if profile == "test"
        ));
        assert_eq!(
            resolve_key(&remote, Some("k".into())).unwrap(),
            Some("k".into())
        );

        let local = profile("http://localhost:11434/v1", ProviderKind::OpenAiCompatible);
        assert_eq!(
            resolve_key(&local, None).unwrap(),
            None,
            "Ollama wants no key, and asking for one would block the local path",
        );
    }

    #[test]
    fn the_verdict_says_what_the_list_said() {
        let listed = |m: &str| m.to_string();
        assert_eq!(
            verdict("m1", &[listed("m0"), listed("m1")]),
            ProviderCheck::Reachable {
                model: "m1".into(),
                models_listed: 2,
                model_listed: Some(true),
            },
        );
        assert_eq!(
            verdict("m1", &[listed("m0")]),
            ProviderCheck::Reachable {
                model: "m1".into(),
                models_listed: 1,
                model_listed: Some(false),
            },
        );
        assert_eq!(
            verdict("m1", &[]),
            ProviderCheck::Reachable {
                model: "m1".into(),
                models_listed: 0,
                model_listed: None,
            },
            "nothing listed is nothing to check against, not a missing model",
        );
    }

    /// The bug this file's header describes: a check that made no request, or
    /// whose request failed, reported "Reachable".
    #[tokio::test]
    async fn a_refused_connection_is_an_error_not_a_verdict() {
        // A port nothing listens on: bind one, learn it, let it go.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let config = profile(
            &format!("http://127.0.0.1:{port}/v1"),
            ProviderKind::OpenAiCompatible,
        );
        let error = check(&config, None, &Http::default())
            .await
            .expect_err("nothing is listening, so nothing was checked");
        assert!(matches!(error, Error::Transport { .. }), "{error}");
    }

    #[tokio::test]
    async fn a_rejected_key_is_reported_as_the_key() {
        let (base, _) = serve(vec![json_response(
            "401 Unauthorized",
            r#"{"error":{"message":"Incorrect API key"}}"#,
        )]);
        let config = profile(&base, ProviderKind::OpenAiCompatible);
        let error = check(&config, Some("sk-wrong".into()), &Http::default())
            .await
            .expect_err("a 401 is not a verdict");
        assert!(
            matches!(error, Error::Unauthorized { status: 401, .. }),
            "{error}"
        );
    }

    #[tokio::test]
    async fn an_endpoint_that_lists_no_models_is_unchecked_not_reachable() {
        let (base, _) = serve(vec![json_response("404 Not Found", "{}")]);
        let config = profile(&base, ProviderKind::OpenAiCompatible);
        assert_eq!(
            check(&config, None, &Http::default()).await.unwrap(),
            ProviderCheck::NotChecked {
                model: "m1".into(),
                why: NotCheckedReason::NoModelListing { status: 404 },
            },
        );
    }

    #[tokio::test]
    async fn a_listed_model_is_reported_from_what_the_endpoint_said() {
        let (base, seen) = serve(vec![json_response(
            "200 OK",
            r#"{"data":[{"id":"m1"},{"id":"m0"}]}"#,
        )]);
        let config = profile(&base, ProviderKind::OpenAiCompatible);
        assert_eq!(
            check(&config, Some("sk-test".into()), &Http::default())
                .await
                .unwrap(),
            ProviderCheck::Reachable {
                model: "m1".into(),
                models_listed: 2,
                model_listed: Some(true),
            },
        );
        let heads = seen.lock().unwrap();
        assert_eq!(heads.len(), 1);
        assert!(
            heads[0].contains("authorization: Bearer sk-test")
                || heads[0].contains("Authorization: Bearer sk-test"),
            "the key travels as a bearer token: {}",
            heads[0],
        );
    }

    /// Anthropic used to be answered with an empty list and no request at
    /// all, which the old verdict then called reachable. It has a `/models`;
    /// it wants its own headers.
    #[tokio::test]
    async fn anthropic_is_asked_for_real_with_its_own_headers() {
        let (base, seen) = serve(vec![json_response(
            "200 OK",
            r#"{"data":[{"id":"m1","type":"model"}],"has_more":false}"#,
        )]);
        let config = profile(&base, ProviderKind::Anthropic);
        let models = list_models(&config, Some("sk-ant".into()), &Http::default())
            .await
            .unwrap();
        assert_eq!(models, vec!["m1".to_string()]);

        let heads = seen.lock().unwrap();
        assert_eq!(heads.len(), 1, "one real request went out");
        let head = heads[0].to_ascii_lowercase();
        assert!(head.starts_with("get /v1/models?limit=1000 "), "{head}");
        assert!(head.contains("x-api-key: sk-ant"), "{head}");
        assert!(
            head.contains(&format!("anthropic-version: {API_VERSION}")),
            "{head}"
        );
        assert!(
            !head.contains("authorization:"),
            "no bearer header — that is the other dialect: {head}"
        );
    }

    /// A busy server is asked once more, before any of the body is read.
    #[tokio::test]
    async fn a_busy_server_is_asked_once_more() {
        let (base, seen) = serve(vec![
            "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nContent-Length: 0\r\n\
             Connection: close\r\n\r\n"
                .to_string(),
            json_response("200 OK", r#"{"data":[{"id":"m1"}]}"#),
        ]);
        let config = profile(&base, ProviderKind::OpenAiCompatible);
        let models = list_models(&config, None, &Http::default())
            .await
            .expect("the second answer is the one that counts");
        assert_eq!(models, vec!["m1".to_string()]);
        assert_eq!(seen.lock().unwrap().len(), 2, "exactly one retry");
    }

    /// And only once: two failures are a failure to report.
    #[tokio::test]
    async fn a_server_that_stays_busy_is_reported_not_hammered() {
        let busy = || {
            "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nContent-Length: 0\r\n\
             Connection: close\r\n\r\n"
                .to_string()
        };
        let (base, seen) = serve(vec![busy(), busy(), busy()]);
        let config = profile(&base, ProviderKind::OpenAiCompatible);
        let error = list_models(&config, None, &Http::default())
            .await
            .expect_err("still busy is an error");
        assert!(matches!(error, Error::Http { status: 503, .. }), "{error}");
        assert_eq!(seen.lock().unwrap().len(), 2, "one retry, not a loop");
    }
}
