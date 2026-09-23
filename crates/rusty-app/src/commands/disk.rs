//! The build directory: how big, how much of it is stale, and removing
//! what the scan says is safe to remove.

use std::path::{Path, PathBuf};

use rusty_core::Workspace;
use rusty_embed::config as storage;
use tauri::State;

use super::Answer;
use crate::state::{AppState, blocking};

/// The project's build directory as a scan sees it: the target directory
/// `cargo metadata` names, and the yardstick from the resolved graph. A
/// workspace that will not load still gets measured — with nothing marked
/// stale but idle caches, and the report saying why.
fn disk_context(root: &Path) -> (PathBuf, rusty_core::disk::Current) {
    match Workspace::load(root) {
        Ok(workspace) => (workspace.target_directory(), workspace.current()),
        Err(_) => (root.join("target"), rusty_core::disk::Current::default()),
    }
}

/// Where this project's builds went on disk, and what of it is stale.
#[tauri::command]
pub async fn disk_report(
    idle_days: Option<u32>,
    state: State<'_, AppState>,
) -> Answer<rusty_core::DiskReport> {
    let root = state.require_root().await?;
    blocking("measuring the build directory", move || {
        let (target_dir, current) = disk_context(&root);
        rusty_core::disk::scan(
            &target_dir,
            &root,
            &current,
            rusty_core::disk::ScanOptions {
                idle_days: idle_days.unwrap_or(7),
                ..rusty_core::disk::ScanOptions::default()
            },
        )
        .report
    })
    .await
}

/// Remove what `policy` names, after a fresh scan of the backend's own.
#[tauri::command]
pub async fn disk_sweep(
    policy: rusty_core::SweepPolicy,
    state: State<'_, AppState>,
) -> Answer<rusty_core::SweepReport> {
    let root = state.require_root().await?;
    Ok(blocking("sweeping the build directory", move || {
        let (target_dir, current) = disk_context(&root);
        rusty_core::disk::sweep(&target_dir, &root, &current, &policy)
    })
    .await??)
}

/// Remove one whole build tree or known extra — only a path the scan itself
/// lists, and never one a build holds the lock on.
#[tauri::command]
pub async fn disk_remove(
    path: String,
    state: State<'_, AppState>,
) -> Answer<rusty_core::SweepReport> {
    let root = state.require_root().await?;
    Ok(blocking("removing a build tree", move || {
        let (target_dir, _) = disk_context(&root);
        rusty_core::disk::remove_tree(&target_dir, Path::new(&path))
    })
    .await??)
}

/// Remove one of cargo's own caches, by the label the report gave it.
#[tauri::command]
pub async fn disk_remove_cache(label: String) -> Answer<rusty_core::SweepReport> {
    Ok(blocking("removing a cargo cache", move || {
        rusty_core::disk::remove_cargo_cache(&label)
    })
    .await??)
}

/// Whether stale artifacts are swept after every successful cargo command.
#[tauri::command]
pub async fn disk_auto_sweep() -> Answer<bool> {
    blocking("reading the workbench settings", || {
        storage::workbench().disk_auto_sweep
    })
    .await
}

#[tauri::command]
pub async fn set_disk_auto_sweep(enabled: bool, state: State<'_, AppState>) -> Answer<()> {
    state
        .update_workbench(move |workbench| workbench.disk_auto_sweep = enabled)
        .await
}
