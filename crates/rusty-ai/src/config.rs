//! Turning a stored provider profile into a live client.
//!
//! The configuration *types* and the preset table live in [`crate::model`] so
//! the settings screen can render them in wasm before any backend call. What
//! remains here needs a socket and a keychain.

use serde::Deserialize;

use crate::{
    error::{Error, Result},
    model::{ProviderConfig, ProviderKind},
    provider::{Provider, anthropic::Anthropic, openai::OpenAiCompatible},
    secrets,
};

/// Build a live provider, pulling the key from the OS credential store.
///
/// Local runtimes (Ollama, LM Studio, vLLM) usually need no key at all, so a
/// missing secret is only an error when the endpoint is remote.
pub fn build(config: &ProviderConfig) -> Result<Box<dyn Provider>> {
    let api_key = match secrets::load(&config.profile)? {
        Some(key) => key,
        None if is_local(&config.base_url) => "not-needed".to_string(),
        None => {
            return Err(Error::MissingKey {
                profile: config.profile.clone(),
            });
        }
    };

    Ok(match config.kind {
        ProviderKind::OpenAiCompatible => Box::new(OpenAiCompatible::new(
            &config.profile,
            &config.base_url,
            &config.model,
            api_key,
            config.supports_tools,
        )),
        ProviderKind::Anthropic => Box::new(Anthropic::new(
            &config.profile,
            &config.base_url,
            &config.model,
            api_key,
        )),
    })
}

fn is_local(base_url: &str) -> bool {
    base_url.contains("localhost") || base_url.contains("127.0.0.1") || base_url.contains("[::1]")
}

/// Ask an OpenAI-compatible endpoint which models it serves.
///
/// This is why presets ship without hardcoded model lists: `GET /models` works
/// on every compatible server — including self-hosted ones whose model names we
/// could not possibly know — and never goes stale.
pub async fn list_models(config: &ProviderConfig) -> Result<Vec<String>> {
    if config.kind == ProviderKind::Anthropic {
        // Anthropic serves this under the same path but requires its own auth
        // headers; until that is wired, report nothing rather than guessing.
        return Ok(Vec::new());
    }

    let endpoint = format!("{}/models", config.base_url.trim_end_matches('/'));
    let mut request = reqwest::Client::new().get(&endpoint);
    if let Some(key) = secrets::load(&config.profile)? {
        request = request.bearer_auth(key);
    }

    let response = request
        .send()
        .await
        .map_err(|e| Error::transport(endpoint.clone(), e))?;

    if !response.status().is_success() {
        return Err(Error::Http {
            profile: config.profile.clone(),
            status: response.status().as_u16(),
            body: response.text().await.unwrap_or_default(),
        });
    }

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
