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
    /// The second editor group's strip and its file, when the editor was
    /// split. Absent from files written before there were two groups.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub second: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second_active: Option<String>,
}

/// What the editor draws around the code, beyond the text: the four things
/// VS Code draws there and lets people turn off. On unless turned off, as in
/// VS Code; a file written before they existed has them on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorView {
    /// Types and names rust-analyzer infers, beside the line they are about.
    pub inlay_hints: bool,
    /// The whole file, small, down the right-hand edge.
    pub minimap: bool,
    /// The lines that open the blocks the top of the view is in, kept at the
    /// top while the view scrolls through them.
    pub sticky_scroll: bool,
    /// A faint line at every indentation stop.
    pub indent_guides: bool,
}

impl Default for EditorView {
    fn default() -> Self {
        EditorView {
            inlay_hints: true,
            minimap: true,
            sticky_scroll: true,
            indent_guides: true,
        }
    }
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
///
/// The check goes through the Tauri updater against the release feed
/// (`latest.json`), so a `newer` answer is one the app can also *install*:
/// the feed named a signed artifact for this platform. What the feed did not
/// carry is absent rather than invented — a release with no notes says so.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The running build.
    pub current: String,
    /// The newest published version, when the check reached the feed.
    pub latest: Option<String>,
    /// The release page for `latest`, for anyone who would rather download.
    pub url: Option<String>,
    /// True only when `latest` is genuinely ahead of `current`.
    pub newer: bool,
    /// Why the check could not answer — no network is the normal state of a
    /// workbench on a bench, so this is a note rather than an error.
    pub note: Option<String>,
    /// The release notes for `latest`, as Markdown — the CHANGELOG section
    /// the release was published with.
    #[serde(default)]
    pub notes: Option<String>,
    /// When `latest` was published, as `YYYY-MM-DD`.
    #[serde(default)]
    pub date: Option<String>,
    /// The user asked not to be prompted about `latest` again. The launch
    /// check stays quiet for it; a check asked for by hand shows it anyway,
    /// because a menu item that finds nothing while a newer build exists is a
    /// menu item people stop trusting.
    #[serde(default)]
    pub skipped: bool,
}

/// How far an update's download has got.
///
/// `total` is the server's `Content-Length`, absent when it sent none — a
/// bar with no end is drawn as activity rather than as progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub received: u64,
    pub total: Option<u64>,
}
