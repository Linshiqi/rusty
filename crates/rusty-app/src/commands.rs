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
// `config` unqualified is rusty_ai's (provider probing); the storage layer goes
// by its own name so the two cannot be confused at a call site.
use rusty_embed::config as storage;
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

    state.open(root.clone(), workspace).await;
    // Recorded backend-side, at the single point every open goes through, so
    // the list exists for the CLI and the next launch without the frontend
    // having to remember to say so.
    storage::record_recent(&root.display().to_string());

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

/// Direct dependencies with their latest stable versions from crates.io.
/// Slow by nature (one index request per crate), so it only runs when the
/// Crates panel asks.
#[tauri::command]
pub async fn crate_report(
    state: State<'_, AppState>,
) -> Answer<Vec<rusty_core::CrateRow>> {
    let workspace = state
        .workspace()
        .await
        .ok_or_else(|| CommandError::new("the Cargo analysis is not available for this project"))?;
    tokio::task::spawn_blocking(move || {
        let deps = rusty_core::registry::direct_dependencies(workspace.graph());
        let proxy = rusty_embed::simulate::effective_proxy();
        rusty_core::registry::annotate_latest(deps, proxy)
    })
    .await
    .map_err(|e| CommandError::new(format!("crate report panicked: {e}")))
}

/// The proxy setting and what detection currently sees, for the settings page.
#[tauri::command]
pub async fn proxy_setting() -> Answer<serde_json::Value> {
    let stored = storage::workbench().proxy;
    let detected = rusty_embed::simulate::system_proxy();
    Ok(serde_json::json!({ "stored": stored, "detected": detected }))
}

/// The stored shortcut overrides, id → chord.
#[tauri::command]
pub async fn keybinds() -> Answer<std::collections::BTreeMap<String, String>> {
    Ok(storage::workbench().keybinds)
}

/// Override one shortcut, or clear the override (chord = null) so the
/// built-in default applies again.
#[tauri::command]
pub async fn set_keybind(id: String, chord: Option<String>) -> Answer<()> {
    let mut state = storage::workbench();
    match chord.as_deref().map(str::trim) {
        None | Some("") => {
            state.keybinds.remove(&id);
        }
        Some(chord) => {
            state.keybinds.insert(id, chord.to_string());
        }
    }
    storage::save_workbench(&state).map_err(CommandError::from)?;
    Ok(())
}

/// Store the proxy choice: null/"auto" = detect, "none" = direct, else a URL.
#[tauri::command]
pub async fn set_proxy_setting(value: Option<String>) -> Answer<()> {
    let mut state = storage::workbench();
    state.proxy = match value.as_deref().map(str::trim) {
        None | Some("") | Some("auto") => None,
        Some(other) => Some(other.to_string()),
    };
    storage::save_workbench(&state).map_err(CommandError::from)?;
    Ok(())
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

/// Projects opened before, newest first — what launch reopens and File lists.
#[tauri::command]
pub fn recent_projects() -> Vec<String> {
    storage::workbench().recent_projects
}

/// Drop a recent that no longer exists. Called when reopening one fails, so a
/// moved project stops being offered every launch.
#[tauri::command]
pub fn forget_recent(path: String) {
    storage::forget_recent(&path);
}

/// Where rusty keeps its data, for the settings screen to show — the answer
/// to "what is this folder and may I delete it".
#[tauri::command]
pub fn storage_location() -> Option<rusty_embed::StorageLocation> {
    storage::location()
}

/// Move the data directory. Copies, switches the pointer, leaves the original
/// in place; with `take_existing` it adopts what the target already holds.
#[tauri::command]
pub async fn relocate_storage(
    path: String,
    take_existing: bool,
    state: State<'_, AppState>,
) -> Answer<rusty_embed::RelocateReport> {
    let report = tokio::task::spawn_blocking(move || {
        storage::relocate(std::path::Path::new(&path), take_existing)
    })
    .await
    .map_err(|e| CommandError::new(format!("relocation panicked: {e}")))??;
    // The cached catalogue was layered from the old directory.
    state.drop_catalog().await;
    Ok(report)
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
#[tauri::command]
pub async fn catalog_problems(
    state: State<'_, AppState>,
) -> Answer<Vec<rusty_embed::CatalogProblem>> {
    Ok(state.catalog().await.problems().to_vec())
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

    // What is plausibly on the other end, before the plan is offered. The
    // device row already knows the port names boards; the plan knowing it
    // too is the difference between "espflash failed on a chip magic
    // mismatch" and a sentence naming both chips.
    let candidates = match &transport {
        Transport::Serial { port } => {
            let catalog = state.catalog().await;
            let names = device::list_serial_ports(&catalog)
                .into_iter()
                .find(|found| &found.name == port)
                .map(|found| found.boards)
                .unwrap_or_default();
            flash::chips_behind(&catalog, &names)
        }
        // A probe reports its own target; it is not a bridge chip that
        // could belong to several boards.
        Transport::Probe { .. } => Vec::new(),
    };
    let warning = flash::chip_mismatch(&chip_id, &candidates);

    let mut plan = flash::plan(&flash::FlashRequest {
        chip_id,
        transport,
        action,
        firmware: PathBuf::from(firmware),
        defmt,
        baud,
    })?;
    plan.warning = warning;
    Ok(plan)
}

/// Write the C-interop scaffolding, in whichever direction.
///
/// Returns what it wrote and what still has to run, so the panel can say so
/// rather than leaving somebody to discover a build.rs they did not expect.
#[tauri::command]
pub async fn scaffold_c_interop(
    direction: String,
    state: State<'_, AppState>,
) -> Answer<rusty_embed::ScaffoldReport> {
    use rusty_embed::scaffold::Direction;

    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let direction = match direction.as_str() {
        "rust-calls-c" => Direction::RustCallsC,
        "c-calls-rust" => Direction::CCallsRust,
        other => {
            return Err(CommandError::new(format!(
                "{other} is not a direction rusty can scaffold",
            )));
        }
    };
    let scaffold = tokio::task::spawn_blocking(move || {
        rusty_embed::scaffold::c_interop(&root, direction)
    })
    .await
    .map_err(|e| CommandError::new(format!("scaffolding panicked: {e}")))?
    .map_err(|e| CommandError::new(e.to_string()))?;

    Ok(rusty_embed::ScaffoldReport {
        written: scaffold.written,
        command: scaffold.command,
        next: scaffold.next,
    })
}

/// Is there a newer rusty? Blocking work — ureq is synchronous — so it
/// goes to a blocking thread rather than stalling the async runtime for as
/// long as a proxy takes to time out.
#[tauri::command]
pub async fn check_update() -> Answer<rusty_embed::UpdateStatus> {
    tokio::task::spawn_blocking(rusty_embed::update::check)
        .await
        .map_err(|e| CommandError::new(format!("the update check panicked: {e}")))
}

/// Hand a URL to the desktop. Six lines and no plugin: the platform
/// openers are stable and this is the only thing rusty opens externally.
#[tauri::command]
pub async fn open_url(url: String) -> Answer<()> {
    // Only http(s), and only what rusty itself produced. A general
    // "run whatever the frontend says" would be a shell injection with a
    // friendly name.
    if !url.starts_with("https://") {
        return Err(CommandError::new("Only https links can be opened."));
    }
    let (program, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("cmd", vec!["/C", "start", ""])
    } else if cfg!(target_os = "macos") {
        ("open", vec![])
    } else {
        ("xdg-open", vec![])
    };
    std::process::Command::new(program)
        .args(args)
        .arg(&url)
        .spawn()
        .map_err(|e| CommandError::new(format!("could not open {url}: {e}")))?;
    Ok(())
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
