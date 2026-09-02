//! Tauri commands.
//!
//! This layer is deliberately thin: it locates what is open, calls into
//! `rusty-embed`, `rusty-core` or `rusty-ai`, and converts errors. No analysis
//! lives here.
//!
//! It also honours the boundary rule from `docs/extensibility.md` — nothing
//! crossing into the WebView is anything other than a `model` type. No guppy
//! handles, no `Workspace`, no API keys.
//!
//! And nothing here blocks an async worker. Every filesystem walk, process
//! spawn, keychain read and `workbench.toml` write goes through [`blocking`]:
//! the async workers are shared by every command in flight, and a `probe-rs
//! list` waiting on USB enumeration used to freeze the whole window for as
//! long as it took.

use std::path::{Path, PathBuf};

use rusty_ai::{Preset, ProviderCheck, ProviderConfig, ToolDef, ToolRegistry, secrets};
use rusty_core::{FeatureImpact, FeatureRow, FeatureSelection, Workspace, WorkspaceReport};
use rusty_embed::{
    Board, Chip, CommandPlan, EmbeddedProject, Explanation, Firmware, FlashAction, MemoryReport,
    Probe, SerialPort, ToolchainReport, Transport, WizardChoice, WizardOption, catalog::Catalog,
    device, firmware, flash, memory, project, toolchain, wizard,
};
// The storage layer goes by its own name so it cannot be confused with
// rusty_ai's `config` at a call site.
use rusty_embed::config as storage;
use tauri::State;

use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

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
    let firmware = {
        let root = root.clone();
        blocking("detection", move || project::firmware_root(&root)).await?
    };
    let detected = {
        let root = root.clone();
        blocking("detection", move || detected_at(&root, &firmware)).await??
    };

    // `cargo metadata` takes seconds on a real workspace.
    let (workspace, report, workspace_error) = {
        let root = root.clone();
        blocking("the Cargo analysis", move || match Workspace::load(&root) {
            Ok(workspace) => match workspace.report() {
                Ok(report) => (Some(workspace), Some(report), None),
                Err(e) => (None, None, Some(e.to_string())),
            },
            Err(e) => (None, None, Some(e.to_string())),
        })
        .await?
    };

    state.open(root.clone(), workspace).await;
    // Recorded backend-side, at the single point every open goes through, so
    // the list exists for the CLI and the next launch without the frontend
    // having to remember to say so. Under the workbench lock like every other
    // writer of the file.
    let opened = root.display().to_string();
    state
        .with_workbench("recording the recent project", move || {
            storage::record_recent(&opened)
        })
        .await?;

    Ok(OpenResult {
        project: detected,
        workspace: report,
        workspace_error,
    })
}

/// Re-read the project's files without reopening it.
///
/// Detection runs where the *firmware* is, which for an ordinary project is
/// the directory that was opened and for a workspace with an excluded
/// bare-metal crate is that crate. `root` is then put back to what the user
/// opened, because that is what it means everywhere it is read: the title
/// bar's project name, and the key the per-project tab strip is stored under.
///
/// `chip_source` carries the difference. It exists so a wrong answer can be
/// traced to the file that produced it, and "the chip came from a
/// subdirectory" is exactly that kind of fact.
#[tauri::command]
pub async fn project_status(state: State<'_, AppState>) -> Answer<EmbeddedProject> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let firmware = state.firmware_root().await.unwrap_or_else(|| root.clone());
    blocking("detection", move || detected_at(&root, &firmware)).await?
}

/// Detection for a project whose firmware may live one directory down.
///
/// Shared by `open_project` and `project_status`: two derivations of the same
/// answer is two chances for the status bar and the Problems panel to
/// disagree about which chip this is.
fn detected_at(root: &Path, firmware: &Path) -> Answer<EmbeddedProject> {
    let mut project = project::detect(firmware)?;
    if firmware != root {
        if let Ok(name) = firmware.strip_prefix(root) {
            let where_from = format!("in {}/", name.display());
            project.chip_source = Some(match project.chip_source {
                Some(source) => format!("{source}, {where_from}"),
                None => where_from,
            });
        }
        // Back to what the user opened. `root` means "the project directory"
        // everywhere it is read — the title bar's name, and the key the
        // per-project tab strip is stored under — and neither of those is the
        // firmware crate.
        project.root = root.display().to_string();
    }
    Ok(project)
}

/// Direct dependencies with their latest stable versions from crates.io.
/// Slow by nature (one index request per crate), so it only runs when the
/// Crates panel asks.
#[tauri::command]
pub async fn crate_report(state: State<'_, AppState>) -> Answer<Vec<rusty_core::CrateRow>> {
    let workspace = state
        .workspace()
        .await
        .ok_or_else(|| CommandError::new("the Cargo analysis is not available for this project"))?;
    blocking("the crate report", move || {
        let deps = rusty_core::registry::direct_dependencies(workspace.graph());
        let proxy = rusty_embed::net::effective_proxy();
        rusty_core::registry::annotate_latest(deps, proxy)
    })
    .await
}

/// A setting typed into a text field: trimmed, and empty means unset.
fn typed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The proxy setting and what detection currently sees, for the settings page.
///
/// Detection reads the registry on Windows — a process spawn, not a lookup.
#[tauri::command]
pub async fn proxy_setting() -> Answer<serde_json::Value> {
    blocking("reading the proxy setting", || {
        let stored = storage::workbench().proxy;
        let detected = rusty_embed::net::system_proxy();
        serde_json::json!({ "stored": stored, "detected": detected })
    })
    .await
}

/// The stored shortcut overrides, id → chord.
#[tauri::command]
pub async fn keybinds() -> Answer<std::collections::BTreeMap<String, String>> {
    blocking("reading the shortcuts", || storage::workbench().keybinds).await
}

/// Whether modal editing is on. Read at startup by every window.
#[tauri::command]
pub async fn vim_enabled() -> Answer<bool> {
    blocking("reading the editor mode", || storage::workbench().vim).await
}

/// Turn modal editing on or off, for good and for every window.
#[tauri::command]
pub async fn set_vim(enabled: bool, state: State<'_, AppState>) -> Answer<()> {
    state
        .update_workbench(move |workbench| workbench.vim = enabled)
        .await
}

/// The stored display language, or `None` for "follow the system".
///
/// Read at startup by every window, like `vim_enabled`: two windows in two
/// languages is not a shrug.
#[tauri::command]
pub async fn display_locale() -> Answer<Option<String>> {
    blocking("reading the display language", || {
        storage::workbench().locale
    })
    .await
}

/// Choose the display language, for good and for every window.
#[tauri::command]
pub async fn set_display_locale(tag: Option<String>, state: State<'_, AppState>) -> Answer<()> {
    let locale = typed(tag);
    state
        .update_workbench(move |workbench| workbench.locale = locale)
        .await
}

/// Override one shortcut, or clear the override (chord = null) so the
/// built-in default applies again.
#[tauri::command]
pub async fn set_keybind(
    id: String,
    chord: Option<String>,
    state: State<'_, AppState>,
) -> Answer<()> {
    let chord = typed(chord);
    state
        .update_workbench(move |workbench| match chord {
            None => {
                workbench.keybinds.remove(&id);
            }
            Some(chord) => {
                workbench.keybinds.insert(id, chord);
            }
        })
        .await
}

/// Store the proxy choice: null/"auto" = detect, "none" = direct, else a URL.
#[tauri::command]
pub async fn set_proxy_setting(value: Option<String>, state: State<'_, AppState>) -> Answer<()> {
    let proxy = typed(value).filter(|value| value != "auto");
    state
        .update_workbench(move |workbench| workbench.proxy = proxy)
        .await
}

#[tauri::command]
pub async fn workspace_report(state: State<'_, AppState>) -> Answer<WorkspaceReport> {
    let workspace = state
        .workspace()
        .await
        .ok_or_else(CommandError::no_workspace)?;
    Ok(blocking("the workspace report", move || workspace.report()).await??)
}

#[tauri::command]
pub async fn project_path(state: State<'_, AppState>) -> Answer<Option<String>> {
    Ok(state.root().await.map(|p| p.display().to_string()))
}

/// Projects opened before, newest first — what launch reopens and File lists.
#[tauri::command]
pub async fn recent_projects() -> Answer<Vec<String>> {
    blocking("reading the recent projects", || {
        storage::workbench().recent_projects
    })
    .await
}

/// Drop a recent that no longer exists. Called when reopening one fails, so a
/// moved project stops being offered every launch.
#[tauri::command]
pub async fn forget_recent(path: String, state: State<'_, AppState>) -> Answer<()> {
    state
        .with_workbench("forgetting a recent project", move || {
            storage::forget_recent(&path)
        })
        .await
}

/// Where rusty keeps its data, for the settings screen to show — the answer
/// to "what is this folder and may I delete it".
#[tauri::command]
pub async fn storage_location() -> Answer<Option<rusty_embed::StorageLocation>> {
    blocking("reading the data directory's location", storage::location).await
}

/// How much disk the data directory is using. Separate from `storage_location`
/// because it walks the tree, and most callers only want the path.
#[tauri::command]
pub async fn storage_footprint() -> Answer<u64> {
    blocking("measuring the data directory", storage::footprint).await
}

/// Move the data directory. Copies, switches the pointer, leaves the original
/// in place; with `take_existing` it adopts what the target already holds.
#[tauri::command]
pub async fn relocate_storage(
    path: String,
    take_existing: bool,
    state: State<'_, AppState>,
) -> Answer<rusty_embed::RelocateReport> {
    let report = blocking("relocation", move || {
        storage::relocate(Path::new(&path), take_existing)
    })
    .await??;
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

/// Remember a project's open editors.
///
/// Called on every tab switch, which is what made the lock necessary: this
/// read-modify-write landing between another writer's read and its save is
/// how a shortcut or a proxy setting used to vanish on the next launch.
#[tauri::command]
pub async fn record_tabs(
    root: String,
    tabs: Vec<String>,
    active: Option<String>,
    state: State<'_, AppState>,
) -> Answer<()> {
    state
        .with_workbench("saving the tab strip", move || {
            storage::record_tabs(&root, tabs, active)
        })
        .await
}

/// What a project had open last time.
#[tauri::command]
pub async fn project_tabs(root: String) -> Answer<Option<rusty_embed::ProjectTabs>> {
    blocking("reading the tab strip", move || storage::tabs_for(&root)).await
}

/// The assistant profile last chosen, and setting it.
///
/// Never the key: that lives in the OS credential store and is fetched by the
/// backend at the moment of the request, so it never enters the window.
#[tauri::command]
pub async fn assistant_choice() -> Answer<Option<rusty_embed::AssistantChoice>> {
    blocking("reading the assistant profile", || {
        storage::workbench().assistant
    })
    .await
}

/// Store the assistant profile. A save that fails says so — a profile the
/// user chose and the next launch does not remember is not a shrug.
#[tauri::command]
pub async fn set_assistant_choice(
    choice: rusty_embed::AssistantChoice,
    state: State<'_, AppState>,
) -> Answer<()> {
    state
        .update_workbench(move |workbench| workbench.assistant = Some(choice))
        .await
}

/// The part's pins, and which of them this project's source names.
///
/// Absent capabilities are reported inside the answer rather than as an
/// error: the claims are worth showing on their own, and a panel that went
/// blank because a device description was missing would be a panel nobody
/// trusts the next time either.
#[tauri::command]
pub async fn pin_report(state: State<'_, AppState>) -> Answer<Option<rusty_embed::PinReport>> {
    // The opened directory for the claims — they are opened in the editor —
    // and the firmware directory for the device description, which is where
    // esp-hal put it. The same path on an ordinary project.
    let Some(root) = state.root().await else {
        return Ok(None);
    };
    let Some(firmware) = state.firmware_root().await else {
        return Ok(None);
    };
    let Some(chip) = state.chip().await else {
        return Ok(None);
    };
    Ok(Some(
        blocking("reading the pin map", move || {
            rusty_embed::pins::report(&root, &firmware, &chip)
        })
        .await?,
    ))
}

/// What switching this project to another chip would change.
///
/// Both chips are resolved from the catalogue rather than taken on trust: a
/// target triple and a toolchain requirement are the two things this must not
/// get wrong, and the catalogue is where they are stated.
#[tauri::command]
pub async fn plan_migration(
    chip: String,
    state: State<'_, AppState>,
) -> Answer<rusty_embed::Migration> {
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    let catalog = state.catalog().await;
    blocking("planning the migration", move || {
        let detected = project::detect(&root)?;
        let current = detected.chip.ok_or_else(|| {
            CommandError::new(
                "rusty cannot tell which chip this project builds for, so it cannot tell what \
                 a switch would change. Set the target in .cargo/config.toml first.",
            )
        })?;
        let find = |id: &str| {
            catalog
                .chips()
                .iter()
                .find(|c| c.id == id)
                .cloned()
                .ok_or_else(|| CommandError::new(format!("{id} is not in the chip catalogue.")))
        };
        Ok(rusty_embed::migrate::plan(
            &root,
            &find(&current)?,
            &find(&chip)?,
        ))
    })
    .await?
}

/// Carry out a migration and report the files written.
#[tauri::command]
pub async fn apply_migration(
    plan: rusty_embed::Migration,
    state: State<'_, AppState>,
) -> Answer<Vec<String>> {
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    blocking("the migration", move || {
        rusty_embed::migrate::apply(&root, &plan)
    })
    .await?
    .map_err(CommandError::new)
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
/// Probes six tools, each a process; nothing about it belongs on an async
/// worker.
#[tauri::command]
pub async fn toolchain_report(state: State<'_, AppState>) -> Answer<ToolchainReport> {
    let root = state.firmware_root().await;
    blocking("the toolchain report", move || {
        let detected = root.and_then(|root| project::detect(&root).ok());
        toolchain::report(detected.as_ref())
    })
    .await
}

// ─── built firmware ──────────────────────────────────────────────────────────

/// Binaries this project has produced, newest first.
///
/// Every device screen needs a path to an ELF, and the alternative to this is a
/// file picker in each of them — which is a file browser wearing a workbench's
/// clothes.
#[tauri::command]
pub async fn firmware_list(state: State<'_, AppState>) -> Answer<Vec<Firmware>> {
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    blocking("listing the firmware", move || {
        let configured = project::detect(&root)
            .ok()
            .and_then(|p| p.configured_target);
        firmware::list(&root, configured.as_deref())
    })
    .await
}

// ─── memory ──────────────────────────────────────────────────────────────────

/// Analyse a built firmware image.
///
/// Passing the path explicitly also records it, so the assistant's
/// `memory_report` tool can reach the same binary the panel is showing.
#[tauri::command]
pub async fn memory_report(elf_path: String, state: State<'_, AppState>) -> Answer<MemoryReport> {
    let path = PathBuf::from(&elf_path);
    let root = state.firmware_root().await;
    let report = {
        let path = path.clone();
        blocking("the memory report", move || {
            let chip_id = root.and_then(|root| project::detect(&root).ok().and_then(|p| p.chip));
            memory::analyze(&path, chip_id.as_deref())
        })
        .await??
    };
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
    blocking("listing the serial ports", move || {
        device::list_serial_ports(catalog.as_ref())
    })
    .await
}

/// Debug probes, via `probe-rs list`.
///
/// A process that waits on USB enumeration — seconds, on a hub with a few
/// devices — so off the IPC thread, where it used to freeze the window for
/// exactly that long.
#[tauri::command]
pub async fn debug_probes() -> Answer<Vec<Probe>> {
    blocking("listing the probes", device::list_probes).await
}

/// "This cannot be the chip you are building for", when the port says so.
///
/// Pure: the device row already knows the port names boards; the plan knowing
/// it too is the difference between "espflash failed on a chip magic mismatch"
/// and a sentence naming both chips. A probe reports its own target — it is
/// not a bridge chip that could belong to several boards — so it warns of
/// nothing. Belongs in `rusty_embed::flash` beside `chip_mismatch`; kept here
/// with a test so the move is mechanical.
fn flash_warning(
    chip_id: &str,
    transport: &Transport,
    ports: &[SerialPort],
    catalog: &Catalog,
) -> Option<String> {
    let candidates = match transport {
        Transport::Serial { port } => {
            let names = ports
                .iter()
                .find(|found| &found.name == port)
                .map(|found| found.boards.clone())
                .unwrap_or_default();
            flash::chips_behind(catalog, &names)
        }
        Transport::Probe { .. } => Vec::new(),
    };
    flash::chip_mismatch(chip_id, &candidates)
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
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    let catalog = state.catalog().await;
    blocking("planning the flash", move || {
        let chip_id = project::detect(&root)?.chip.ok_or_else(|| {
            CommandError::new(
                "The target chip is unknown, so rusty cannot choose a flashing command. \
                 Fix the problems listed in the Project panel first.",
            )
        })?;
        // Enumerated only when the plan needs it: a probe asks nothing of the
        // serial ports.
        let ports = match &transport {
            Transport::Serial { .. } => device::list_serial_ports(&catalog),
            Transport::Probe { .. } => Vec::new(),
        };
        let warning = flash_warning(&chip_id, &transport, &ports, &catalog);

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
    })
    .await?
}

/// The C-compiler precondition for scaffolding, pure: which compiler a chip's
/// C is compiled by, and the refusal when it is missing or unknown.
///
/// `scaffold` already refuses rather than lay half a scaffold over somebody's
/// code; this is the same rule applied to the other precondition, which is not
/// about the files at all: `cc` shells out to a cross compiler, and four
/// correct new files whose build cannot find one is a worse answer than a
/// refusal that names it. `on_path` is passed in so the rule is a test. It
/// belongs in `rusty_embed::scaffold` beside the file check; kept here so the
/// move is mechanical.
fn c_compiler_gate(chip: Option<&Chip>, on_path: impl Fn(&str) -> bool) -> Result<(), String> {
    let Some(chip) = chip else {
        // No chip means no cross compiler to require; the host's `cc` is
        // whatever it is and not rusty's to judge.
        return Ok(());
    };
    match toolchain::c_compiler(chip.arch) {
        Some((binary, install)) if !on_path(binary) => Err(format!(
            "This project builds for {}, so C in it is compiled by `{binary}`, and that is \
             not on PATH. Nothing has been written. Install it — {install} — and the \
             Toolchain panel will show it before you try again.",
            chip.name,
        )),
        None => Err(format!(
            "rusty does not know which C compiler a {} project uses, so it will not scaffold \
             C it cannot say how to build. Nothing has been written.",
            chip.arch.label(),
        )),
        Some(_) => Ok(()),
    }
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

    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;

    let direction = match direction.as_str() {
        "rust-calls-c" => Direction::RustCallsC,
        "c-calls-rust" => Direction::CCallsRust,
        other => {
            return Err(CommandError::new(format!(
                "{other} is not a direction rusty can scaffold",
            )));
        }
    };

    blocking("scaffolding", move || {
        // Before anything is written — see `c_compiler_gate`.
        let detected = project::detect(&root)?;
        let chip = detected.chip.as_deref().and_then(rusty_embed::chip::by_id);
        c_compiler_gate(chip.as_ref(), |binary| {
            toolchain::on_path_pub(binary).is_some()
        })
        .map_err(CommandError::new)?;

        let scaffold = rusty_embed::scaffold::c_interop(&root, direction)
            .map_err(|e| CommandError::new(e.to_string()))?;
        Ok(rusty_embed::ScaffoldReport {
            written: scaffold.written,
            command: scaffold.command,
            next: scaffold.next,
        })
    })
    .await?
}

/// Is there a newer rusty? Blocking work — ureq is synchronous — so it
/// goes to a blocking thread rather than stalling the async runtime for as
/// long as a proxy takes to time out.
#[tauri::command]
pub async fn check_update() -> Answer<rusty_embed::UpdateStatus> {
    blocking("the update check", rusty_embed::update::check).await
}

/// A link rusty may hand to the desktop, or why not.
///
/// Only https, and only RFC 3986's own alphabet with every `%` a complete
/// escape. Not because the openers need it — none of them goes through a
/// shell any more — but because a URL is the one string here that came from
/// outside (GitHub's `html_url`), and a rule about what it may contain is
/// cheaper than reasoning about what each opener does with a byte it did not
/// expect.
fn checked_url(url: &str) -> Result<&str, CommandError> {
    if !url.starts_with("https://") {
        return Err(CommandError::new("Only https links can be opened."));
    }
    let allowed =
        |byte: u8| byte.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&byte);
    if let Some(bad) = url.bytes().find(|byte| !allowed(*byte)) {
        return Err(CommandError::new(format!(
            "This link carries `{}`, which is not a character a URL can contain, so it was \
             not opened.",
            char::from(bad).escape_default(),
        )));
    }
    let bytes = url.as_bytes();
    let complete_escape = |at: usize| {
        bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(at + 2).is_some_and(u8::is_ascii_hexdigit)
    };
    if let Some(at) = (0..bytes.len()).find(|&at| bytes[at] == b'%' && !complete_escape(at)) {
        return Err(CommandError::new(format!(
            "This link has a `%` at position {at} that is not a percent-encoding, so it was \
             not opened.",
        )));
    }
    Ok(url)
}

/// Hand a URL to the desktop. No plugin: the platform openers are stable and
/// this is the only thing rusty opens externally.
///
/// On Windows the opener is `rundll32 url.dll,FileProtocolHandler`, not `cmd
/// /C start`: `start` is a cmd built-in, so the URL went through cmd's parser,
/// and `&`, `|` and `^` in a query string were operators rather than
/// characters. The URL handler takes the string as one argument and passes it
/// on as one.
#[tauri::command]
pub async fn open_url(url: String) -> Answer<()> {
    let url = checked_url(&url)?.to_string();
    blocking("opening the link", move || {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("rundll32.exe", &["url.dll,FileProtocolHandler"])
        } else if cfg!(target_os = "macos") {
            ("open", &[])
        } else {
            ("xdg-open", &[])
        };
        std::process::Command::new(program)
            .args(args)
            .arg(&url)
            .spawn()
            .map(|_| ())
            .map_err(|e| CommandError::new(format!("could not open {url}: {e}")))
    })
    .await?
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
    Ok(blocking("the feature rows", move || {
        workspace.feature_rows(&selection)
    })
    .await??)
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
    Ok(blocking("the feature impact", move || {
        workspace.feature_impact(&selection)
    })
    .await??)
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
pub async fn ai_key_configured(profile: String) -> Answer<bool> {
    blocking("reading the credential store", move || {
        secrets::is_configured(&profile)
    })
    .await
}

#[tauri::command]
pub async fn ai_store_key(profile: String, api_key: String) -> Answer<()> {
    Ok(blocking("writing the credential store", move || {
        secrets::store(&profile, &api_key)
    })
    .await??)
}

#[tauri::command]
pub async fn ai_delete_key(profile: String) -> Answer<()> {
    Ok(blocking("writing the credential store", move || {
        secrets::delete(&profile)
    })
    .await??)
}

/// The two things every provider call needs from the machine: the profile's
/// key and the proxy. One blocking hop for both — the keychain is IO, and the
/// proxy setting is a registry query on Windows.
///
/// The proxy is the same `effective_proxy` the tool installer and the update
/// check use, so the assistant reaches its endpoint on exactly the machines
/// where they reach theirs.
pub(crate) async fn ai_inputs(profile: String) -> Answer<(Option<String>, rusty_ai::Http)> {
    Ok(blocking("reading the assistant's key and proxy", move || {
        let key = secrets::load(&profile)?;
        let http = rusty_ai::Http {
            proxy: rusty_embed::net::effective_proxy(),
        };
        Ok::<_, rusty_ai::Error>((key, http))
    })
    .await??)
}

/// Ask the endpoint which models it serves.
///
/// Model names drift far too fast to ship as a hardcoded list, and a
/// self-hosted server's names are unknowable in advance.
#[tauri::command]
pub async fn ai_list_models(config: ProviderConfig) -> Answer<Vec<String>> {
    let (key, http) = ai_inputs(config.profile.clone()).await?;
    Ok(rusty_ai::config::list_models(&config, key, &http).await?)
}

/// Verify a provider profile end to end without starting a conversation.
///
/// What comes back is what one real request established, as facts the
/// frontend words — never a sentence, and never a success inferred from a
/// request that failed: a refused key, an unreachable host and a timeout are
/// the errors they are.
#[tauri::command]
pub async fn ai_check_provider(config: ProviderConfig) -> Answer<ProviderCheck> {
    let (key, http) = ai_inputs(config.profile.clone()).await?;
    Ok(rusty_ai::config::check(&config, key, &http).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str, boards: &[&str]) -> SerialPort {
        SerialPort {
            name: name.to_string(),
            bridge: None,
            boards: boards.iter().map(|b| b.to_string()).collect(),
            likely_board: true,
            usb: None,
        }
    }

    fn serial(port: &str) -> Transport {
        Transport::Serial {
            port: port.to_string(),
        }
    }

    /// The warning names both chips when the port's boards all carry another
    /// part, and says nothing when the evidence is thinner than that.
    #[test]
    fn a_port_that_cannot_carry_the_projects_chip_is_named_before_the_flash() {
        let catalog = Catalog::builtin();
        let ports = vec![
            port("COM3", &["ESP32-C3-DevKitM-1"]),
            port("COM4", &["ESP32-C3-DevKitM-1", "ESP32-DevKitC V4"]),
            port("COM5", &[]),
        ];

        let warning = flash_warning("esp32", &serial("COM3"), &ports, &catalog)
            .expect("a C3 board is not an esp32 project's board");
        assert!(warning.contains("esp32c3"), "{warning}");
        assert!(warning.contains("esp32"), "{warning}");

        assert_eq!(
            flash_warning("esp32", &serial("COM4"), &ports, &catalog),
            None,
            "one of the candidates is the project's chip — no evidence of a mismatch",
        );
        assert_eq!(
            flash_warning("esp32", &serial("COM5"), &ports, &catalog),
            None,
            "an adapter rusty does not recognise is not evidence of anything",
        );
        assert_eq!(
            flash_warning("esp32", &serial("COM9"), &ports, &catalog),
            None,
            "a port that is not in the list is not in the list",
        );
        assert_eq!(
            flash_warning(
                "esp32",
                &Transport::Probe { identifier: None },
                &ports,
                &catalog
            ),
            None,
            "a probe reports its own target; it is not a bridge chip",
        );
    }

    /// The gate refuses with the compiler's name and its install route, and
    /// says in as many words that nothing was written.
    #[test]
    fn scaffolding_refuses_before_writing_when_the_cross_compiler_is_missing() {
        let xtensa = rusty_embed::chip::by_id("esp32").expect("the classic ESP32 is catalogued");
        let riscv = rusty_embed::chip::by_id("esp32c3").expect("the C3 is catalogued");
        let cortex = rusty_embed::chip::by_id("stm32f103").expect("an STM32 is catalogued");

        let missing = c_compiler_gate(Some(&xtensa), |_| false).unwrap_err();
        assert!(missing.contains("xtensa-esp-elf-gcc"), "{missing}");
        assert!(
            missing.contains("espup"),
            "the install route travels with the refusal: {missing}"
        );
        assert!(missing.contains("Nothing has been written"), "{missing}");

        let missing = c_compiler_gate(Some(&riscv), |_| false).unwrap_err();
        assert!(missing.contains("riscv32-esp-elf-gcc"), "{missing}");

        assert_eq!(
            c_compiler_gate(Some(&xtensa), |binary| binary == "xtensa-esp-elf-gcc"),
            Ok(()),
            "the right compiler on PATH is all it asks",
        );
        assert!(
            c_compiler_gate(Some(&riscv), |binary| binary == "xtensa-esp-elf-gcc").is_err(),
            "the other architecture's compiler does not count",
        );

        let unknown = c_compiler_gate(Some(&cortex), |_| true).unwrap_err();
        assert!(
            unknown.contains("does not know which C compiler"),
            "a part whose compiler rusty has not verified is refused, not guessed: {unknown}",
        );
        assert!(unknown.contains("Nothing has been written"), "{unknown}");

        assert_eq!(
            c_compiler_gate(None, |_| false),
            Ok(()),
            "no chip, no cross compiler to require"
        );
    }

    /// The URL rule, pure: https only, RFC 3986's alphabet only, and every `%`
    /// a complete escape.
    #[test]
    fn only_a_well_formed_https_link_is_handed_to_the_desktop() {
        let release = "https://github.com/Linshiqi/rusty/releases/tag/v0.3.0";
        assert_eq!(checked_url(release).unwrap(), release);
        let query = "https://example.com/a?b=1&c=2#frag";
        assert_eq!(
            checked_url(query).unwrap(),
            query,
            "& and # are the URL's own characters — no shell is involved any more",
        );
        assert!(
            checked_url("https://example.com/a%20b").is_ok(),
            "a complete escape"
        );

        assert!(checked_url("http://example.com").is_err(), "https only");
        assert!(checked_url("file:///etc/passwd").is_err());
        assert!(
            checked_url("https://example.com/a b").is_err(),
            "a space is not URL"
        );
        assert!(
            checked_url("https://example.com/a|b").is_err(),
            "nor a pipe"
        );
        assert!(
            checked_url("https://example.com/a^b").is_err(),
            "nor cmd's escape"
        );
        assert!(
            checked_url("https://example.com/\"a\"").is_err(),
            "nor a quote"
        );
        assert!(
            checked_url("https://example.com/a<b>c").is_err(),
            "nor redirection"
        );
        assert!(
            checked_url("https://example.com/%PATH%").is_err(),
            "%PA is not an escape"
        );
        assert!(
            checked_url("https://example.com/a%2").is_err(),
            "a truncated escape"
        );
        assert!(
            checked_url("https://example.com/naïve").is_err(),
            "an IRI has to be encoded first"
        );
        assert!(
            checked_url("https://example.com/a\nb").is_err(),
            "no control characters"
        );
    }

    #[test]
    fn a_typed_setting_is_trimmed_and_blank_is_unset() {
        assert_eq!(typed(None), None);
        assert_eq!(typed(Some("".into())), None);
        assert_eq!(typed(Some("   ".into())), None);
        assert_eq!(typed(Some(" zh-CN ".into())), Some("zh-CN".to_string()));
    }
}
