//! Reading and writing the project's files.

use rusty_edit::{Document, Entry};
use tauri::State;

use crate::{error::CommandError, state::AppState};

/// The project tree, with build output already excluded.
#[tauri::command]
pub async fn file_tree(
    show_hidden: bool,
    state: State<'_, AppState>,
) -> Result<Vec<Entry>, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(rusty_edit::read_tree(&root, show_hidden)?)
}

/// One file, highlighted.
#[tauri::command]
pub async fn open_file(path: String, state: State<'_, AppState>) -> Result<Document, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    // The grammars are parsed from a bundled dump and take long enough that
    // doing it per file is noticeable, so the set is built once and kept.
    Ok(state.files().open(&root, &path)?)
}

/// Open a dependency's source read-only — where goto-definition lands when
/// the answer lives in esp-hal or `core` rather than in the project.
#[tauri::command]
pub async fn open_external(
    path: String,
    state: State<'_, AppState>,
) -> Result<Document, CommandError> {
    let files = state.files();
    Ok(tokio::task::spawn_blocking(move || files.open_external(&path))
        .await
        .map_err(|e| CommandError::new(format!("opening panicked: {e}")))??)
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

/// Run the buffer through rustfmt without touching disk.
#[tauri::command]
pub async fn format_text(
    path: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<rusty_edit::Formatted, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(
        tokio::task::spawn_blocking(move || rusty_edit::format_rust(&root, &path, &text))
            .await
            .map_err(|e| CommandError::new(format!("formatting panicked: {e}")))??,
    )
}

/// Every place the query appears in the project's files.
#[tauri::command]
pub async fn search_project(
    query: String,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    include: String,
    exclude: String,
    state: State<'_, AppState>,
) -> Result<rusty_edit::SearchResults, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let spec = rusty_edit::SearchQuery {
        text: query,
        case_sensitive,
        whole_word,
        regex,
        include,
        exclude,
    };
    tokio::task::spawn_blocking(move || rusty_edit::search(&root, &spec))
        .await
        .map_err(|e| CommandError::new(format!("search panicked: {e}")))
}
