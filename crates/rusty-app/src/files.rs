//! Reading and writing the project's files.

use rusty_edit::{Document, Entry};
use tauri::State;

use crate::{error::CommandError, state::AppState};

/// The project tree, with build output already excluded.
#[tauri::command]
pub async fn file_tree(state: State<'_, AppState>) -> Result<Vec<Entry>, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(rusty_edit::read_tree(&root)?)
}

/// One file, highlighted.
#[tauri::command]
pub async fn open_file(path: String, state: State<'_, AppState>) -> Result<Document, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    // The grammars are parsed from a bundled dump and take long enough that
    // doing it per file is noticeable, so the set is built once and kept.
    Ok(state.files().open(&root, &path)?)
}

/// Re-highlight an unsaved buffer.
#[tauri::command]
pub async fn highlight_text(
    path: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_edit::Line>, CommandError> {
    let files = state.files();
    tokio::task::spawn_blocking(move || files.highlight_source(&path, &text))
        .await
        .map_err(|e| CommandError::new(format!("highlighting panicked: {e}")))
}

#[tauri::command]
pub async fn save_file(
    path: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(rusty_edit::save(&root, &path, &text)?)
}
