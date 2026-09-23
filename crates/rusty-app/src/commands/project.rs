//! Opening a project, reading it again, and the playground — which opens
//! like a project and stays out of the recents list.

use std::path::{Path, PathBuf};

use rusty_embed::config as storage;
use rusty_embed::{EmbeddedProject, project};
use tauri::State;

use super::Answer;
use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

/// What was found when opening a folder.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenResult {
    pub project: EmbeddedProject,
}

#[tauri::command]
pub async fn open_project(path: String, state: State<'_, AppState>) -> Answer<OpenResult> {
    let root = PathBuf::from(&path);
    let opened = open_at(root.clone(), &state).await?;
    // Recorded backend-side, at the single point every open goes through, so
    // the list exists for the CLI and the next launch without the frontend
    // having to remember to say so. Under the workbench lock like every other
    // writer of the file.
    let path = root.display().to_string();
    state
        .with_workbench("recording the recent project", move || {
            storage::record_recent(&path)
        })
        .await?;
    Ok(opened)
}

/// Detect and hold a project — everything opening one means but the recents
/// list, which a playground stays out of: it has its own door, and a list of
/// the projects somebody works on is not the place for rusty's scratch pad.
async fn open_at(root: PathBuf, state: &AppState) -> Answer<OpenResult> {
    let firmware = {
        let root = root.clone();
        blocking("detection", move || project::firmware_root(&root)).await?
    };
    let detected = {
        let root = root.clone();
        blocking("detection", move || detected_at(&root, &firmware)).await??
    };

    // The Cargo analysis is *not* awaited here. It resolves the whole
    // dependency graph, which is hundreds of milliseconds warm and seconds
    // on a project whose lockfile does not exist yet — and the frontend is
    // told nothing at all until this command answers, so a switch showed the
    // old project for the whole of it. `AppState::workspace` loads it when
    // the first panel that needs it asks.
    state.open(root).await;
    Ok(OpenResult { project: detected })
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
            // The same fact as a value: the title bar's Test keys on it,
            // and matching ", in " out of the prose would be the English
            // comparison every other refusal here avoids.
            project.firmware_dir = Some(name.display().to_string());
        }
        // Back to what the user opened. `root` means "the project directory"
        // everywhere it is read — the title bar's name, and the key the
        // per-project tab strip is stored under — and neither of those is the
        // firmware crate.
        project.root = root.display().to_string();
    }
    // Opened by its own door or through File > Open alike, a playground is
    // laid out as one.
    project.playground = storage::data_dir()
        .and_then(|data| rusty_embed::playground::chip_of(&data, root))
        .map(str::to_string);
    Ok(project)
}

#[tauri::command]
pub async fn project_path(state: State<'_, AppState>) -> Answer<Option<String>> {
    Ok(state.root().await.map(|p| p.display().to_string()))
}

/// The playground for a chip, opened — written the first time and kept
/// after. Held like any project, and left out of the recents list.
#[tauri::command]
pub async fn open_playground(chip: String, state: State<'_, AppState>) -> Answer<OpenResult> {
    let data = playground_home()?;
    let root = blocking("preparing the playground", move || {
        rusty_embed::playground::prepare(&data, &chip)
    })
    .await??;
    open_at(root, &state).await
}

/// Put the playground's example back, whatever was written over it.
#[tauri::command]
pub async fn reset_playground(chip: String) -> Answer<()> {
    let data = playground_home()?;
    blocking("restoring the playground's example", move || {
        rusty_embed::playground::reset(&data, &chip)
    })
    .await??;
    Ok(())
}

/// Copy the playground to a folder of the user's as a project of its own,
/// and say where it went — the frontend opens it there.
#[tauri::command]
pub async fn keep_playground(chip: String, dest: String) -> Answer<String> {
    let data = playground_home()?;
    let kept = blocking("keeping the playground as a project", move || {
        rusty_embed::playground::keep(&data, &chip, Path::new(&dest))
    })
    .await??;
    Ok(kept.display().to_string())
}

fn playground_home() -> Answer<PathBuf> {
    storage::data_dir()
        .ok_or_else(|| CommandError::new("There is no data directory to keep a playground in."))
}
