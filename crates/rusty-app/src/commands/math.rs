//! The math toolbox's sheet, read from and written to the open project.

use rusty_embed::spatial::sheet::MathSheet;
use rusty_embed::spatial::sheet_file;
use tauri::State;

use super::Answer;
use crate::state::{AppState, blocking};

/// The project's `.rusty/math.toml`, or nothing when it has none yet. One
/// that does not read is an error, never an empty sheet the panel would
/// then save over it.
#[tauri::command]
pub async fn math_sheet_load(state: State<'_, AppState>) -> Answer<Option<MathSheet>> {
    let root = state.require_root().await?;
    Ok(blocking("reading the math sheet", move || sheet_file::load(&root)).await??)
}

#[tauri::command]
pub async fn math_sheet_save(sheet: MathSheet, state: State<'_, AppState>) -> Answer<()> {
    let root = state.require_root().await?;
    Ok(blocking("saving the math sheet", move || {
        sheet_file::save(&root, &sheet)
    })
    .await??)
}
