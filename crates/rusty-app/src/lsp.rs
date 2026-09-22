//! The language server's commands.
//!
//! Same shape as the terminal's: one long-lived `lsp_start` that streams events
//! for the life of the server, and short calls for everything else. The client
//! blocks on a pipe, so every call crosses onto a blocking thread rather than
//! starving an async worker.

use std::sync::Arc;

use rusty_lsp::{CompletionList, HoverInfo, Location, LspClient, LspEvent};
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
            rusty_embed::project::detect(&root)
                .ok()
                .and_then(|project| {
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

    // A rust-analyzer the user named in Settings (`workbench.toml`'s
    // `rust_analyzer`), when there is one: a copy rusty would not find by
    // itself. Read here rather than once at boot, so a change applies to the
    // next project opened without restarting the window.
    let named = tokio::task::spawn_blocking(|| rusty_embed::config::workbench().rust_analyzer)
        .await
        .unwrap_or_default();
    let spawned = tokio::task::spawn_blocking(move || {
        LspClient::spawn(
            &root,
            hint.as_deref(),
            named.as_deref().map(std::path::Path::new),
        )
    })
    .await
    .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))?;

    let (client, events) = match spawned {
        Ok(pair) => pair,
        Err(e) => {
            // The toolchain table's recipe, not a copy of it: the copy here
            // had no `--toolchain stable`, and rustup answers for the
            // directory it runs in, so in an esp project it tried to add the
            // component to `esp` — which cannot take one — and the button
            // did nothing twice.
            let _ = on_event.send(LspEvent::Unavailable {
                message: e.to_string(),
                install: rusty_embed::toolchain::install_command("rust-analyzer"),
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
pub async fn lsp_close(path: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(());
    };
    tokio::task::spawn_blocking(move || client.did_close(&path))
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
) -> Result<CompletionList, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(CompletionList::default());
    };
    Ok(
        tokio::task::spawn_blocking(move || client.completion(&path, line, col))
            .await
            .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??,
    )
}

/// The edits an accepted completion makes besides the insertion — the
/// import for an item that was not in scope. `reply` names the answer the
/// item was picked from.
#[tauri::command]
pub async fn lsp_resolve_completion(
    path: String,
    reply: u64,
    index: u32,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::ActionEdit>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(Vec::new());
    };
    Ok(
        tokio::task::spawn_blocking(move || client.resolve_completion(&path, reply, index))
            .await
            .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??,
    )
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
    Ok(
        tokio::task::spawn_blocking(move || client.hover(&path, line, col))
            .await
            .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??,
    )
}

/// Quick fixes and refactorings at the caret, edits pre-resolved.
#[tauri::command]
pub async fn lsp_code_actions(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<rusty_lsp::CodeActions, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(rusty_lsp::CodeActions::default());
    };
    Ok(
        tokio::task::spawn_blocking(move || client.code_actions(&path, line, col))
            .await
            .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??,
    )
}

/// The part of an accepted quick fix that lands in other files, written
/// there. Answers with the files that changed.
#[tauri::command]
pub async fn lsp_apply_action(
    path: String,
    reply: u64,
    index: u32,
    state: State<'_, AppState>,
) -> Result<Vec<String>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(Vec::new());
    };
    Ok(
        tokio::task::spawn_blocking(move || client.apply_action_elsewhere(&path, reply, index))
            .await
            .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??,
    )
}

/// The document's semantic colouring — the colours only the compiler's view
/// can produce.
#[tauri::command]
pub async fn lsp_semantic(
    path: String,
    lines: Option<(u32, u32)>,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::SemanticSpan>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(Vec::new());
    };
    Ok(
        tokio::task::spawn_blocking(move || client.semantic_tokens(&path, lines))
            .await
            .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??,
    )
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

/// Every use of the symbol at this position, its declaration included.
#[tauri::command]
pub async fn lsp_references(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::Place>, CommandError> {
    places(state, move |client| client.references(&path, line, col)).await
}

/// What implements the trait or method at this position, or the impls of
/// the type there.
#[tauri::command]
pub async fn lsp_implementations(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::Place>, CommandError> {
    places(state, move |client| {
        client.implementations(&path, line, col)
    })
    .await
}

/// Where the type of the thing at this position is defined.
#[tauri::command]
pub async fn lsp_type_definition(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::Place>, CommandError> {
    places(state, move |client| {
        client.type_definition(&path, line, col)
    })
    .await
}

/// The other places in the file the name at this position occurs.
#[tauri::command]
pub async fn lsp_highlights(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::EditRange>, CommandError> {
    places(state, move |client| {
        client.document_highlights(&path, line, col)
    })
    .await
}

/// The file's outline, flattened in document order.
#[tauri::command]
pub async fn lsp_document_symbols(
    path: String,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::Symbol>, CommandError> {
    places(state, move |client| client.document_symbols(&path)).await
}

/// Symbols across the workspace whose names match `query`.
#[tauri::command]
pub async fn lsp_workspace_symbols(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::Symbol>, CommandError> {
    places(state, move |client| client.workspace_symbols(&query)).await
}

/// The function at this position, where a call hierarchy starts.
#[tauri::command]
pub async fn lsp_call_hierarchy(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::CallItem>, CommandError> {
    places(state, move |client| client.call_hierarchy(&path, line, col)).await
}

/// The inlay hints over lines `from..to` of a file.
#[tauri::command]
pub async fn lsp_inlay_hints(
    path: String,
    from: u32,
    to: u32,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::InlayHint>, CommandError> {
    places(state, move |client| client.inlay_hints(&path, from, to)).await
}

/// Who calls the function `item` names, or what it calls: `item` is the
/// server's own description of it, handed back untouched.
#[tauri::command]
pub async fn lsp_calls(
    item: String,
    incoming: bool,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::Call>, CommandError> {
    places(state, move |client| {
        if incoming {
            client.incoming_calls(&item)
        } else {
            client.outgoing_calls(&item)
        }
    })
    .await
}

/// The macro call at this position expanded all the way down, as a
/// read-only Rust document — `None` when nothing there is a macro, or no
/// server is running to ask.
///
/// Headed as VS Code heads it, which is what a Rust programmer who has
/// expanded a macro before has seen: generated code, so the comment is code
/// and stays English like the expansion under it.
#[tauri::command]
pub async fn lsp_expand_macro(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Option<rusty_edit::Document>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(None);
    };
    let files = state.files();
    Ok(tokio::task::spawn_blocking(move || {
        let Some(expanded) = client.expand_macro(&path, line, col)? else {
            return Ok::<_, rusty_lsp::Error>(None);
        };
        let heading = format!("// Recursive expansion of {} macro", expanded.name);
        let rule = format!("// {}", "=".repeat(heading.chars().count() - 3));
        // rust-analyzer starts some expansions with a line break of its own.
        let expansion = expanded
            .expansion
            .trim_start_matches(['\r', '\n'])
            .trim_end();
        let text = format!("{heading}\n{rule}\n\n{expansion}\n");
        let shown = rusty_edit::expansion_path(&path, line, &expanded.name);
        Ok(Some(files.virtual_document(&shown, "expansion.rs", text)))
    })
    .await
    .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??)
}

/// Ask the server on the blocking pool, and answer with nothing when there
/// is no server: a list that is empty while rust-analyzer starts is the
/// warm-up talking, as a definition that finds nothing is.
async fn places<T: Send + 'static>(
    state: State<'_, AppState>,
    ask: impl FnOnce(&rusty_lsp::LspClient) -> rusty_lsp::Result<Vec<T>> + Send + 'static,
) -> Result<Vec<T>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Ok(Vec::new());
    };
    Ok(tokio::task::spawn_blocking(move || ask(&client))
        .await
        .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??)
}

/// Rename the symbol at this position across the whole project.
///
/// Refuses without a server rather than doing nothing quietly: a rename that
/// silently changed one file and not its callers is a broken build the user
/// would find at the next compile, blaming their own edit.
#[tauri::command]
pub async fn lsp_rename(
    path: String,
    line: u32,
    col: u32,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, CommandError> {
    let Some(client) = state.lsp().await else {
        return Err(CommandError::new(
            "rust-analyzer is not running, so nothing knows where this symbol is used",
        ));
    };
    Ok(
        tokio::task::spawn_blocking(move || client.rename(&path, line, col, &new_name))
            .await
            .map_err(|e| CommandError::new(format!("the language server task panicked: {e}")))??,
    )
}
