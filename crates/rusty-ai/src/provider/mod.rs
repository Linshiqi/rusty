//! Provider-neutral chat.
//!
//! Every provider translates to and from [`crate::model`], so the UI and the
//! agent loop are written once. Switching a user from GPT to a local Ollama
//! model must not change a single line above this layer.
//!
//! The types live in `model` rather than here because the frontend renders them
//! and has to compile to wasm; only the trait and the wire adapters are
//! backend-only.

pub mod anthropic;
pub mod openai;

use std::pin::Pin;

use async_trait::async_trait;
use futures_util::Stream;
use serde::de::DeserializeOwned;

use crate::{
    error::{Error, Result},
    http,
    model::{ChatEvent, Message, ProviderKind, ToolDef},
};

/// One turn's worth of input.
#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    /// Prepended as a system prompt. Providers place this differently — OpenAI
    /// as a message, Anthropic as a top-level field — which is exactly the kind
    /// of difference this layer exists to hide.
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

pub type EventStream = Pin<Box<dyn Stream<Item = Result<ChatEvent>> + Send>>;

/// A request as `kind`'s API expects it to arrive: a bearer token in the
/// OpenAI dialect, Anthropic's own key header beside its pinned version.
/// With no key — a local server — only what the dialect always sends.
///
/// Shared by both dialects' chat and by the model listing, so a request
/// cannot be authorised one way to chat and another to list.
pub(crate) fn authorize(
    kind: ProviderKind,
    request: reqwest::RequestBuilder,
    key: Option<&str>,
) -> reqwest::RequestBuilder {
    match kind {
        ProviderKind::OpenAiCompatible => match key {
            Some(key) => request.bearer_auth(key),
            None => request,
        },
        ProviderKind::Anthropic => {
            let request = request.header("anthropic-version", anthropic::API_VERSION);
            match key {
                Some(key) => request.header("x-api-key", key),
                None => request,
            }
        }
    }
}

/// Send a request to `endpoint` — once more on a 429 or a 5xx, as
/// [`http::send_retrying`] decides — and turn a refusal into the error it
/// means. Every request either dialect makes goes this way.
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    endpoint: &str,
    profile: &str,
) -> Result<reqwest::Response> {
    let response = http::send_retrying(request, endpoint).await?;
    check_status(response, profile).await
}

/// One streamed event's data as `T`, or the error naming the line. A line
/// that does not parse ends the stream in both dialects rather than being
/// skipped — see [`openai::decode`] for why.
pub(crate) fn parse_data<T: DeserializeOwned>(data: &str, profile: &str) -> Result<T> {
    serde_json::from_str(data).map_err(|e| Error::protocol(profile, format!("{e}: {data}")))
}

/// A non-2xx before the stream starts, as the error it means.
///
/// 401 and 403 are the key, and say so. Everything else carries the body,
/// because a provider's 400 is the only place it explains what it disliked.
async fn check_status(response: reqwest::Response, profile: &str) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let code = status.as_u16();
    if code == 401 || code == 403 {
        return Err(Error::Unauthorized {
            profile: profile.to_string(),
            status: code,
        });
    }
    Err(Error::Http {
        profile: profile.to_string(),
        status: code,
        body: response.text().await.unwrap_or_default(),
    })
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// Whether this provider can call tools. A model without tool support can
    /// still chat, but the workbench's analyses will be unavailable to it, so
    /// the UI warns rather than silently degrading.
    fn supports_tools(&self) -> bool {
        true
    }
    async fn chat(&self, request: ChatRequest) -> Result<EventStream>;
}
