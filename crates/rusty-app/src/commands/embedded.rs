//! The chip and what is known about it: the catalogue, the pin map, a
//! switch to another part, the toolchain the project needs, and the
//! firmware it has built.

use std::path::PathBuf;

use rusty_embed::{
    Board, Chip, Firmware, MemoryReport, ToolchainReport, firmware, memory, project, toolchain,
};
use tauri::State;

use super::Answer;
use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

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
