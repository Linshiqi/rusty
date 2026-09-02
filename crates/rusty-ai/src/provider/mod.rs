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

use crate::{
    error::{Error, Result},
    model::{ChatEvent, Content, Message, ToolDef},
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

/// A non-2xx before the stream starts, as the error it means.
///
/// 401 and 403 are the key, and say so. Everything else carries the body,
/// because a provider's 400 is the only place it explains what it disliked.
/// Shared by both dialects and by the model listing, so the three cannot
/// disagree about what a status means.
pub(crate) async fn check_status(
    response: reqwest::Response,
    profile: &str,
) -> Result<reqwest::Response> {
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
    /// The user-facing profile name, e.g. `deepseek` or `local-ollama`.
    fn id(&self) -> &str;
    fn model(&self) -> &str;
    /// Whether this provider can call tools. A model without tool support can
    /// still chat, but the workbench's analyses will be unavailable to it, so
    /// the UI warns rather than silently degrading.
    fn supports_tools(&self) -> bool {
        true
    }
    async fn chat(&self, request: ChatRequest) -> Result<EventStream>;
}

/// Accumulates streaming tool-call fragments into finished calls.
///
/// Both wire formats deliver tool arguments as partial JSON across many events,
/// so every provider needs this and none of them should reimplement it.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: Vec<(String, String, String)>, // id, name, partial json
}

impl ToolCallAccumulator {
    pub fn start(&mut self, id: String, name: String) {
        self.calls.push((id, name, String::new()));
    }

    pub fn push(&mut self, id: &str, fragment: &str) {
        if let Some(call) = self.calls.iter_mut().find(|c| c.0 == id) {
            call.2.push_str(fragment);
        }
    }

    /// Finished calls, with arguments parsed. A call whose JSON never became
    /// valid is returned with a null input so the caller can report a tool
    /// error rather than silently dropping the model's intent.
    pub fn finish(self) -> Vec<Content> {
        self.calls
            .into_iter()
            .map(|(id, name, json)| Content::ToolUse {
                id,
                name,
                input: serde_json::from_str(&json).unwrap_or(serde_json::Value::Null),
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }
}
