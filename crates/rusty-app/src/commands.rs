//! Tauri commands.
//!
//! This layer is deliberately thin: it locates what is open, calls into
//! `rusty-embed`, `rusty-core` or `rusty-ai`, and converts errors. No analysis
//! lives here.
//!
//! It also honours the boundary rule from `docs/extensibility.md` — nothing
//! crossing into the WebView is anything other than a `model` type. No guppy
//! handles, no `Workspace`, no API keys.

use std::path::PathBuf;

use rusty_ai::{Preset, ProviderConfig, ToolDef, ToolRegistry, config, secrets};
use rusty_core::{FeatureImpact, FeatureRow, FeatureSelection, Workspace, WorkspaceReport};
use rusty_embed::{
    Board, Chip, CommandPlan, EmbeddedProject, Explanation, Firmware, FlashAction, MemoryReport,
    Probe, SerialPort, ToolchainReport, Transport, WizardChoice, WizardOption, device, firmware,
    flash, memory, project, toolchain, wizard,
};
use tauri::State;

use crate::{error::CommandError, state::AppState};

type Answer<T> = Result<T, CommandError>;

// ─── project ─────────────────────────────────────────────────────────────────

/// What was found when opening a folder.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenResult {
    pub project: EmbeddedProject,
    /// The Cargo analysis, when `cargo metadata` succeeded.
    ///
    /// Optional on purpose: a misconfigured embedded project often fails
    /// `cargo metadata`, and that is precisely when its diagnosis matters most.
    /// Refusing to open would hide the one screen that explains the problem.
    pub workspace: Option<WorkspaceReport>,
    /// Why the Cargo analysis is absent, if it is.
    pub workspace_error: Option<String>,
}

#[tauri::command]
pub async fn open_project(path: String, state: State<'_, AppState>) -> Answer<OpenResult> {
    let root = PathBuf::from(&path);
    let detected = project::detect(&root)?;

    let (workspace, report, workspace_error) = match Workspace::load(&root) {
        Ok(workspace) => match workspace.report() {
            Ok(report) => (Some(workspace), Some(report), None),
            Err(e) => (None, None, Some(e.to_string())),
        },
        Err(e) => (None, None, Some(e.to_string())),
    };

    state.open(root, workspace).await;

    Ok(OpenResult {
        project: detected,
        workspace: report,
        workspace_error,
    })
}

/// Re-read the project's files without reopening it.
#[tauri::command]
pub async fn project_status(state: State<'_, AppState>) -> Answer<EmbeddedProject> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(project::detect(&root)?)
}

#[tauri::command]
pub async fn workspace_report(state: State<'_, AppState>) -> Answer<WorkspaceReport> {
    let workspace = state
        .workspace()
        .await
        .ok_or_else(CommandError::no_workspace)?;
    Ok(workspace.report()?)
}

#[tauri::command]
pub async fn project_path(state: State<'_, AppState>) -> Answer<Option<String>> {
    Ok(state.root().await.map(|p| p.display().to_string()))
}

// ─── chips and toolchain ─────────────────────────────────────────────────────

/// Every part rusty knows about, after the user's and project's overlays.
#[tauri::command]
pub async fn chip_catalogue(state: State<'_, AppState>) -> Answer<Vec<Chip>> {
    Ok(state.catalog().await.chips().to_vec())
}

/// Every board, with where each definition came from.
#[tauri::command]
pub async fn board_catalogue(state: State<'_, AppState>) -> Answer<Vec<Board>> {
    Ok(state.catalog().await.boards().to_vec())
}

/// Catalogue files that failed to load.
///
/// Surfaced rather than swallowed: a user who wrote a board file and cannot
/// find their board needs to be told the file did not parse, not left to
/// wonder whether rusty read it at all.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProblemDto {
    pub path: String,
    pub detail: String,
}

#[tauri::command]
pub async fn catalog_problems(state: State<'_, AppState>) -> Answer<Vec<CatalogProblemDto>> {
    Ok(state
        .catalog()
        .await
        .problems()
        .iter()
        .map(|p| CatalogProblemDto {
            path: p.path.clone(),
            detail: p.detail.clone(),
        })
        .collect())
}

/// Machine tooling, cross-checked against the open project when there is one.
#[tauri::command]
pub async fn toolchain_report(state: State<'_, AppState>) -> Answer<ToolchainReport> {
    let detected = match state.root().await {
        Some(root) => project::detect(&root).ok(),
        None => None,
    };
    Ok(toolchain::report(detected.as_ref()))
}

// ─── built firmware ──────────────────────────────────────────────────────────

/// Binaries this project has produced, newest first.
///
/// Every device screen needs a path to an ELF, and the alternative to this is a
/// file picker in each of them — which is a file browser wearing a workbench's
/// clothes.
#[tauri::command]
pub async fn firmware_list(state: State<'_, AppState>) -> Answer<Vec<Firmware>> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let configured = project::detect(&root)
        .ok()
        .and_then(|p| p.configured_target);
    Ok(firmware::list(&root, configured.as_deref()))
}

// ─── memory ──────────────────────────────────────────────────────────────────

/// Analyse a built firmware image.
///
/// Passing the path explicitly also records it, so the assistant's
/// `memory_report` tool can reach the same binary the panel is showing.
#[tauri::command]
pub async fn memory_report(
    elf_path: String,
    state: State<'_, AppState>,
) -> Answer<MemoryReport> {
    let path = PathBuf::from(&elf_path);
    let chip_id = match state.root().await {
        Some(root) => project::detect(&root).ok().and_then(|p| p.chip),
        None => None,
    };
    let report = memory::analyze(&path, chip_id.as_deref())?;
    state.set_firmware(Some(path)).await;
    Ok(report)
}

// ─── new project ─────────────────────────────────────────────────────────────

/// Generator options, with what each one costs.
#[tauri::command]
pub fn wizard_options() -> Vec<WizardOption> {
    wizard::options()
}

/// What the current selection commits the user to.
///
/// Called on every change in the wizard, not just at the end: the point is to
/// answer "what does this choice mean" while the choice is still being made.
#[tauri::command]
pub fn explain_choice(choice: WizardChoice) -> Vec<Explanation> {
    wizard::explain(&choice)
}

#[tauri::command]
pub fn plan_new_project(choice: WizardChoice) -> Answer<CommandPlan> {
    Ok(wizard::plan(&choice)?)
}

// ─── devices ─────────────────────────────────────────────────────────────────

/// Serial ports currently attached, named against the board catalogue and with
/// likely boards first.
#[tauri::command]
pub async fn serial_ports(state: State<'_, AppState>) -> Answer<Vec<SerialPort>> {
    let catalog = state.catalog().await;
    Ok(device::list_serial_ports(catalog.as_ref()))
}

/// Debug probes, via `probe-rs list`.
#[tauri::command]
pub fn debug_probes() -> Vec<Probe> {
    device::list_probes()
}

/// Work out the command without running it.
///
/// The UI shows this before the user commits, and the assistant can quote it.
/// Separating the decision from the execution is also what makes the choice of
/// tool and flags testable without a board attached.
#[tauri::command]
pub async fn plan_flash(
    transport: Transport,
    action: FlashAction,
    firmware: String,
    defmt: bool,
    baud: Option<u32>,
    state: State<'_, AppState>,
) -> Answer<CommandPlan> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let chip_id = project::detect(&root)?
        .chip
        .ok_or_else(|| CommandError::new(
            "The target chip is unknown, so rusty cannot choose a flashing command. \
             Fix the problems listed in the Project panel first.",
        ))?;

    Ok(flash::plan(&flash::FlashRequest {
        chip_id,
        transport,
        action,
        firmware: PathBuf::from(firmware),
        defmt,
        baud,
    })?)
}

// ─── features ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn feature_rows(
    selection: FeatureSelection,
    state: State<'_, AppState>,
) -> Answer<Vec<FeatureRow>> {
    let workspace = state
        .workspace()
        .await
        .ok_or_else(CommandError::no_workspace)?;
    Ok(workspace.feature_rows(&selection)?)
}

#[tauri::command]
pub async fn feature_impact(
    selection: FeatureSelection,
    state: State<'_, AppState>,
) -> Answer<FeatureImpact> {
    let workspace = state
        .workspace()
        .await
        .ok_or_else(CommandError::no_workspace)?;
    Ok(workspace.feature_impact(&selection)?)
}

// ─── AI configuration ────────────────────────────────────────────────────────

#[tauri::command]
pub fn ai_presets() -> Vec<Preset> {
    rusty_ai::presets()
}

/// The tools the assistant can call, so the UI can show what it is allowed to
/// do before the user asks anything.
#[tauri::command]
pub fn ai_tools() -> Vec<ToolDef> {
    ToolRegistry::workbench().defs()
}

/// Whether a profile has a key on file. Deliberately returns a boolean and
/// never the key itself — the settings screen has no reason to hold a secret.
#[tauri::command]
pub fn ai_key_configured(profile: String) -> bool {
    secrets::is_configured(&profile)
}

#[tauri::command]
pub fn ai_store_key(profile: String, api_key: String) -> Answer<()> {
    Ok(secrets::store(&profile, &api_key)?)
}

#[tauri::command]
pub fn ai_delete_key(profile: String) -> Answer<()> {
    Ok(secrets::delete(&profile)?)
}

/// Ask the endpoint which models it serves.
///
/// Model names drift far too fast to ship as a hardcoded list, and a
/// self-hosted server's names are unknowable in advance.
#[tauri::command]
pub async fn ai_list_models(config: ProviderConfig) -> Answer<Vec<String>> {
    Ok(config::list_models(&config).await?)
}

/// Verify a provider profile end to end without starting a conversation.
///
/// Reports the first failure in the user's own terms — wrong key, unreachable
/// endpoint, bad model name — instead of letting them discover it mid-answer.
#[tauri::command]
pub async fn ai_check_provider(config: ProviderConfig) -> Answer<String> {
    let provider = rusty_ai::config::build(&config)?;
    let models = config::list_models(&config).await.unwrap_or_default();

    if models.is_empty() {
        return Ok(format!("Reachable. Using {}.", provider.model()));
    }
    if models.iter().any(|m| m == provider.model()) {
        return Ok(format!("Reachable. {} is available.", provider.model()));
    }
    Ok(format!(
        "Reachable, but `{}` is not in the {} models this endpoint lists.",
        provider.model(),
        models.len()
    ))
}
