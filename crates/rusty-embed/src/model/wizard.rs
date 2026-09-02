//! Starting a new project.

use serde::{Deserialize, Serialize};

use super::Runtime;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WizardChoice {
    pub chip: String,
    pub runtime: Runtime,
    /// Crate name for the new project.
    pub name: String,
    /// Generator option ids, e.g. `embassy`, `wifi`, `alloc`.
    #[serde(default)]
    pub options: Vec<String>,
}

/// A generator option, with what turning it on costs.
///
/// A model type rather than a DTO in the Tauri layer: the frontend renders
/// these, and rule 1 is that it `use`s model types directly. A struct declared
/// beside the command would have to be mirrored by hand in the frontend, which
/// is the drift the shared types exist to make impossible.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WizardOption {
    /// What `esp-generate -o` expects.
    pub id: String,
    pub label: String,
    /// What it commits the project to, in the user's terms.
    pub detail: String,
    /// Options this one cannot work without.
    ///
    /// `esp-generate` enforces these and rejects the entire run when they are
    /// missing, so the wizard needs them to avoid offering a combination that
    /// cannot succeed.
    #[serde(default)]
    pub requires: Vec<String>,
}

/// What one choice in the wizard commits the user to.
///
/// The reason the wizard exists. A list of chip names tells a beginner nothing
/// about the fact that half of them require downloading a forked compiler.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Explanation {
    pub topic: String,
    pub detail: String,
    /// A concrete follow-on — a command to run, a target that gets used.
    pub consequence: Option<String>,
}
