//! The language server's commands.
//!
//! Same shape as the terminal's: one long-lived `lsp_start` that streams events
//! for the life of the server, and short calls for everything else. The client
//! blocks on a pipe, so every call crosses onto a blocking thread rather than
//! starving an async worker.

use std::sync::Arc;

use rusty_lsp::{CompletionItem, HoverInfo, Location, LspClient, LspEvent};
use tauri::{State, ipc::Channel};

use crate::{error::CommandError, state::AppState};

/// Start rust-analyzer for the open project and stream what it says.
///
/// An unavailable server is an event, not an error: the editor works without
/// one — no squiggles, no completion — and a red banner about a missing
/// optional tool would be crying wolf.
#[tauri::command]
pub async fn lsp_start(
    on_event: Channel<LspEvent>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let Some(root) = state.root().await else {
        return Err(CommandError::no_project());
    };

    // What the firmware builds for, so cfg resolution matches the chip rather
    // than the host. Detection already worked this out; not passing it along
    // would have rust-analyzer analysing a `no_std` project as if it were a
    // desktop one.
    let hint = tokio::task::spawn_blocking({
        let root = root.clone();
        move || {
            rusty_embed::project::detect(&root).ok().and_then(|project| {
                project.configured_target.or_else(|| {
                    project
                        .chip
                        .and_then(|id| rusty_embed::chip::by_id(&id))
                        .map(|chip| chip.bare_metal_target)
                })
            })
        }
    })
    .await
    .unwrap_or(None);

    let spawned =
        tokio::task::spawn_blocking(move || LspClient::spawn(&root, hint.as_deref()))
            .await
            .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))?;

    let (client, events) = match spawned {
        Ok(pair) => pair,
        Err(e) => {
            let _ = on_event.send(LspEvent::Unavailable {
                message: e.to_string(),
                install: Some("rustup component add rust-analyzer".into()),
            });
            return Ok(());
        }
    };

    state.set_lsp(Some(Arc::new(client))).await;
    let _ = on_event.send(LspEvent::Ready {});

    let _ = tokio::task::spawn_blocking(move || {
        while let Some(event) = events.recv() {
            if on_event.send(event).is_err() {
                break;
            }
        }
    })
    .await;
    Ok(())
}

/// Show the server a document. Quietly nothing without a server — the editor
/// neither knows nor cares whether one came up.
#[tauri::command]
pub async fn lsp_open(
    path: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(());
    };
    tokio::task::spawn_blocking(move || client.did_open(&path, &text))
        .await
        .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??;
    Ok(())
}

#[tauri::command]
pub async fn lsp_change(
    path: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(());
    };
    tokio::task::spawn_blocking(move || client.did_change(&path, &text))
        .await
        .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??;
    Ok(())
}

#[tauri::command]
pub async fn lsp_saved(path: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(());
    };
    tokio::task::spawn_blocking(move || client.did_save(&path))
        .await
        .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??;
    Ok(())
}

#[tauri::command]
pub async fn lsp_complete(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Vec<CompletionItem>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(Vec::new());
    };
    Ok(tokio::task::spawn_blocking(move || client.completion(&path, line, col))
        .await
        .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??)
}

#[tauri::command]
pub async fn lsp_hover(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Option<HoverInfo>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(None);
    };
    Ok(tokio::task::spawn_blocking(move || client.hover(&path, line, col))
        .await
        .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??)
}

/// The document's semantic colouring — the colours only the compiler's view
/// can produce.
#[tauri::command]
pub async fn lsp_semantic(
    path: String,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::SemanticSpan>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(Vec::new());
    };
    Ok(tokio::task::spawn_blocking(move || client.semantic_tokens(&path))
        .await
        .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??)
}

/// The signature of the call the caret is inside, for parameter hints.
#[tauri::command]
pub async fn lsp_signature(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Option<rusty_lsp::SignatureInfo>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(None);
    };
    Ok(
        tokio::task::spawn_blocking(move || client.signature_help(&path, line, col))
            .await
            .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??,
    )
}

#[tauri::command]
pub async fn lsp_definition(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Option<Location>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(None);
    };
    Ok(
        tokio::task::spawn_blocking(move || client.definition(&path, line, col))
            .await
            .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??,
    )
}
