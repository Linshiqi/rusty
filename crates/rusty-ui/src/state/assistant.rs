//! The assistant drawer's state.

use super::*;

/// One tool call the assistant made while answering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolRun {
    pub id: String,
    pub name: String,
    /// `None` while it is still running.
    pub ok: Option<bool>,
}

const PROVIDER_KEY: &str = "rusty.assistant.provider";

/// Whatever this window still holds from before the profile became a file.
///
/// Read once and deleted, so an upgrade does not cost somebody their model
/// choice and the key does not linger to be misread later. It never held a
/// secret — the key itself lives in the OS credential store and is fetched by
/// the backend at the moment of the request, so it never enters this window.
pub fn carried_provider() -> Option<ProviderConfig> {
    serde_json::from_str(&local_take(PROVIDER_KEY)?).ok()
}

/// The conversation and the provider behind it. The key itself is never
/// here — it lives in the OS credential store and never enters the WebView.
#[derive(Clone, Copy)]
pub struct Assistant {
    /// The assistant.
    ///
    /// The transcript lives here, not in the backend: the backend takes a
    /// history and returns the updated one, so closing the panel cannot strand a
    /// conversation and nothing has to be cleaned up when it is reopened.
    pub config: RwSignal<Option<ProviderConfig>>,
    pub presets: RwSignal<Vec<Preset>>,
    pub tools: RwSignal<Vec<ToolDef>>,
    pub conversation: RwSignal<Vec<Message>>,
    /// Prose from the answer in flight, before it becomes a `Message`.
    pub pending: RwSignal<String>,
    /// The model's reasoning in flight, from models that stream it. Shown
    /// dim and folded while it arrives, so a model that thinks for a minute
    /// before its first word is visibly thinking rather than silent.
    pub thinking: RwSignal<String>,
    /// The last answer stopped at the output cap. Nothing in the returned
    /// history says so — the model produced no text to mark — so the window
    /// keeps the fact itself until the next question.
    pub cut_short: RwSignal<bool>,
    /// Tools the current answer has called, in order, with whether each
    /// finished cleanly. Shown live: a model that goes quiet for ten seconds
    /// while resolving a dependency graph looks broken unless it says so.
    pub activity: RwSignal<Vec<ToolRun>>,
    pub streaming: RwSignal<bool>,
    /// Tokens the last answer cost, when the provider reported them. Surfaced
    /// because with bring-your-own keys every token is the user's money.
    pub usage: RwSignal<Option<(u32, u32)>>,
    /// Whether the assistant profile has a key in the OS credential store.
    /// The key itself never comes back here — only whether one exists.
    pub key_stored: RwSignal<bool>,
    /// The assistant drawer on the right, toggled from the title bar.
    pub open: RwSignal<bool>,
}
