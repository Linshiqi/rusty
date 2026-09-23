//! The Cargo analysis the Crates and Features panels read: the workspace's
//! health, its direct dependencies against crates.io, and what a feature
//! selection costs.

use rusty_core::{FeatureImpact, FeatureRow, FeatureSelection, WorkspaceReport};
use tauri::State;

use super::Answer;
use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

#[tauri::command]
pub async fn workspace_report(state: State<'_, AppState>) -> Answer<WorkspaceReport> {
    let workspace = state
        .workspace()
        .await
        .ok_or_else(CommandError::no_workspace)?;
    Ok(blocking("the workspace report", move || workspace.report()).await??)
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
