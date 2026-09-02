//! The workbench's own affairs: what it remembers, where it keeps it, and
//! whether there is a newer one.
//!
//! These are wire types. `workbench.toml` is written and read through
//! `config.rs`'s own private structs, so a field renamed here for the
//! frontend's sake cannot silently drop a key from everybody's file.

use serde::{Deserialize, Serialize};

/// The assistant profile, as the frontend and the backend agree on it.
///
/// A separate type from `rusty_ai::ProviderConfig` on purpose — the same rule
/// that keeps `catalog.rs` from serialising `model` types. That one is a
/// contract with the provider layer; this one crosses the IPC boundary, and
/// coupling them means a refactor on one side rewriting the other.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantChoice {
    pub profile: String,
    pub kind: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub supports_tools: Option<bool>,
}

/// One project's open editors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectTabs {
    /// The project root as the user spelled it when it was recorded. Matching
    /// is by the filesystem's idea of the same directory, so a different
    /// spelling finds it — the trap `recent_projects` already learned.
    pub root: String,
    pub tabs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
}

/// Where rusty keeps its data, for the settings screen to show.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageLocation {
    pub path: String,
    /// True when no pointer and no env override is in play.
    pub is_default: bool,
    /// True when `RUSTY_CONFIG_DIR` decided — relocating from the UI would be
    /// silently outvoted, so the UI disables it and says why.
    pub env_override: bool,
}

/// What a relocation did, so the user can verify before deleting the old copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocateReport {
    pub from: String,
    pub to: String,
    pub copied_files: usize,
    pub adopted: bool,
}

/// What an update check found.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The running build.
    pub current: String,
    /// The newest published version, when the check reached GitHub.
    pub latest: Option<String>,
    /// Where to get it.
    pub url: Option<String>,
    /// True only when `latest` is genuinely ahead of `current`.
    pub newer: bool,
    /// Why the check could not answer — no network is the normal state of a
    /// workbench on a bench, so this is a note rather than an error.
    pub note: Option<String>,
}
