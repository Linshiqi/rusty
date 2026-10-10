//! The language server's commands.
//!
//! Same shape as the terminal's: one long-lived `lsp_start` that streams events
//! for the life of the server, and short calls for everything else. The client
//! blocks on a pipe, so every call crosses onto a blocking thread rather than
//! starving an async worker.
//!
//! **A question about a file's text carries the text** (`rusty_lsp::Draft`)
//! and is given to the server first, in the same task (`synced`). Sent as a
//! change and then a question, the two were two calls, two tasks on the
//! blocking pool and two round trips per keystroke for completion — and two
//! flows' changes could arrive in either order, the older written over the
//! newer. The client skips a draft older than the one it holds.

use std::sync::Arc;

use rusty_lsp::{CompletionList, Draft, HoverInfo, Location, LspClient, LspEvent};
use tauri::{State, ipc::Channel};

use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

/// Start the open project's language server — rust-analyzer for a Cargo
/// project, clangd for a PlatformIO or CMake one — and stream what it says.
///
/// An unavailable server is an event, not an error: the editor works without
/// one — no squiggles, no completion — and a red banner about a missing
/// optional tool would be crying wolf.
#[tauri::command]
pub async fn lsp_start(
    on_event: Channel<LspEvent>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let root = state.require_root().await?;

    // rust-analyzer analyses a Cargo workspace; a PlatformIO or CMake
    // project's C and C++ are clangd's, read through the compile database
    // the build writes, with the part's cross compiler asked for its system
    // headers — without that, every `#include <string.h>` in Arm firmware
    // is "file not found" (measured with `clangd --check`).
    if !root.join("Cargo.toml").is_file() {
        let spawned = blocking("the language server task", {
            let root = root.clone();
            move || {
                let project = rusty_embed::project::detect(&root).ok();
                let catalog = rusty_embed::catalog::Catalog::load(Some(&root));
                let compile_commands = project
                    .as_ref()
                    .and_then(|p| rusty_embed::buildsys::compile_commands_dir(&root, p));
                let globs = rusty_embed::buildsys::query_driver_globs(&catalog);
                LspClient::spawn_clangd(&root, compile_commands.as_deref(), &globs, None)
            }
        })
        .await?;
        return match spawned {
            Ok((client, events)) => serve(&state, on_event, client, events).await,
            Err(e) => {
                let _ = on_event.send(LspEvent::Unavailable {
                    message: e.to_string(),
                    install: Some(CLANGD_INSTALL.to_string()),
                });
                Ok(())
            }
        };
    }

    // What the firmware builds for, so cfg resolution matches the chip rather
    // than the host. Detection already worked this out; not passing it along
    // would have rust-analyzer analysing a `no_std` project as if it were a
    // desktop one.
    let hint = tokio::task::spawn_blocking({
        let root = root.clone();
        move || {
            rusty_embed::project::detect(&root)
                .ok()
                // A target the project carries as a description (a CH32's
                // `riscv32ec-unknown-none-elf.json`) is named by its file in
                // .cargo/config.toml, which rust-analyzer's cargo reads for
                // itself; its stem as `cargo.target` is a triple no rustc
                // knows, and the check would fail on it.
                .filter(|project| {
                    !project
                        .chip
                        .as_deref()
                        .and_then(rusty_embed::chip::by_id)
                        .is_some_and(|chip| {
                            chip.toolchain == rusty_embed::ToolchainRequirement::NightlyBuildStd
                        })
                })
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
    let spawned = blocking("the language server task", move || {
        LspClient::spawn(
            &root,
            hint.as_deref(),
            named.as_deref().map(std::path::Path::new),
        )
    })
    .await?;

    // The C a `cc` build script compiles is clangd's, beside rust-analyzer,
    // read through a compile database rusty writes — cc records no command
    // line — naming the part's cross compiler, which `--query-driver` asks
    // for its system headers.
    let companion = blocking("the language server task", {
        let root = state.require_root().await?;
        move || {
            let compiler = rusty_embed::project::detect(&root)
                .ok()
                .and_then(|project| project.chip)
                .and_then(|id| rusty_embed::chip::by_id(&id))
                .and_then(|chip| chip.c_compiler)
                .map(|compiler| compiler.binary);
            let database =
                rusty_embed::buildsys::write_cargo_compile_commands(&root, compiler.as_deref())?;
            let catalog = rusty_embed::catalog::Catalog::load(Some(&root));
            let globs = rusty_embed::buildsys::query_driver_globs(&catalog);
            Some(LspClient::spawn_clangd(
                &root,
                Some(&database),
                &globs,
                None,
            ))
        }
    })
    .await?;

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

    serve_with(&state, on_event, client, events, companion).await
}

/// Where clangd comes from: LLVM's own releases, or the system's package.
const CLANGD_INSTALL: &str =
    "install clangd (https://clangd.llvm.org/installation) and put it on PATH";

/// Hold a started server and stream what it says until it ends.
async fn serve(
    state: &AppState,
    on_event: Channel<LspEvent>,
    client: LspClient,
    events: rusty_lsp::Events,
) -> Result<(), CommandError> {
    serve_with(state, on_event, client, events, None).await
}

/// `serve`, with a second server beside the first when one was asked for:
/// its diagnostics and refreshes go down the same stream, and nothing else
/// it says — its progress, its health, its exit — since the status bar is
/// the first server's.
async fn serve_with(
    state: &AppState,
    on_event: Channel<LspEvent>,
    client: LspClient,
    events: rusty_lsp::Events,
    companion: Option<rusty_lsp::Result<(LspClient, rusty_lsp::Events)>>,
) -> Result<(), CommandError> {
    state.set_lsp(Some(Arc::new(client))).await;
    let _ = on_event.send(LspEvent::Ready {});

    match companion {
        Some(Ok((companion, events))) => {
            let server = companion.kind();
            state.set_companion(Some(Arc::new(companion))).await;
            let _ = on_event.send(LspEvent::Companion {
                server,
                message: None,
                install: None,
            });
            let on_event = on_event.clone();
            tauri::async_runtime::spawn_blocking(move || {
                while let Some(event) = events.recv() {
                    let passed =
                        matches!(event, LspEvent::Diagnostics { .. } | LspEvent::Refresh {});
                    if passed && on_event.send(event).is_err() {
                        break;
                    }
                }
            });
        }
        Some(Err(e)) => {
            let _ = on_event.send(LspEvent::Companion {
                server: rusty_lsp::ServerKind::Clangd,
                message: Some(e.to_string()),
                install: Some(CLANGD_INSTALL.to_string()),
            });
        }
        None => {}
    }

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
    ask(state.lsp_for(&path).await, move |client| {
        client.did_open(&path, &text)
    })
    .await
}

#[tauri::command]
pub async fn lsp_change(
    path: String,
    draft: Draft,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        client.sync_draft(&path, &draft)
    })
    .await
}

#[tauri::command]
pub async fn lsp_saved(path: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        client.did_save(&path)
    })
    .await
}

#[tauri::command]
pub async fn lsp_close(path: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        client.did_close(&path)
    })
    .await
}

#[tauri::command]
pub async fn lsp_complete(
    path: String,
    line: u32,
    col: u32,
    draft: Option<Draft>,
    state: State<'_, AppState>,
) -> Result<CompletionList, CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        synced(client, &path, draft.as_ref())?;
        client.completion(&path, line, col)
    })
    .await
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
    ask(state.lsp_for(&path).await, move |client| {
        client.resolve_completion(&path, reply, index)
    })
    .await
}

#[tauri::command]
pub async fn lsp_hover(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Option<HoverInfo>, CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        client.hover(&path, line, col)
    })
    .await
}

/// Quick fixes and refactorings at the caret, edits pre-resolved.
#[tauri::command]
pub async fn lsp_code_actions(
    path: String,
    line: u32,
    col: u32,
    draft: Option<Draft>,
    state: State<'_, AppState>,
) -> Result<rusty_lsp::CodeActions, CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        synced(client, &path, draft.as_ref())?;
        client.code_actions(&path, line, col)
    })
    .await
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
    ask(state.lsp_for(&path).await, move |client| {
        client.apply_action_elsewhere(&path, reply, index)
    })
    .await
}

/// The document's semantic colouring — the colours only the compiler's view
/// can produce.
#[tauri::command]
pub async fn lsp_semantic(
    path: String,
    lines: Option<(u32, u32)>,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::SemanticSpan>, CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        client.semantic_tokens(&path, lines)
    })
    .await
}

/// The pulse after an edit, in one command: the edit given to the server,
/// then the semantic colours over `lines` (the whole file when `None`) and,
/// when `hints` names a range, the inlay hints over it. Each answer is
/// `None` when the server did not give one — an error here is the warm-up
/// or an edit overtaking the question, and the editor keeps what it shows.
#[tauri::command]
pub async fn lsp_painted(
    path: String,
    draft: Draft,
    lines: Option<(u32, u32)>,
    hints: Option<(u32, u32)>,
    state: State<'_, AppState>,
) -> Result<rusty_lsp::Painted, CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        client.sync_draft(&path, &draft)?;
        Ok(rusty_lsp::Painted {
            semantic: client.semantic_tokens(&path, lines).ok(),
            hints: hints.and_then(|(from, to)| client.inlay_hints(&path, from, to).ok()),
        })
    })
    .await
}

/// The signature of the call the caret is inside, for parameter hints.
#[tauri::command]
pub async fn lsp_signature(
    path: String,
    line: u32,
    col: u32,
    draft: Option<Draft>,
    state: State<'_, AppState>,
) -> Result<Option<rusty_lsp::SignatureInfo>, CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        synced(client, &path, draft.as_ref())?;
        client.signature_help(&path, line, col)
    })
    .await
}

#[tauri::command]
pub async fn lsp_definition(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Option<Location>, CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        client.definition(&path, line, col)
    })
    .await
}

/// Every use of the symbol at this position, its declaration included.
#[tauri::command]
pub async fn lsp_references(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::Place>, CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        client.references(&path, line, col)
    })
    .await
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
    ask(state.lsp_for(&path).await, move |client| {
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
    ask(state.lsp_for(&path).await, move |client| {
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
    ask(state.lsp_for(&path).await, move |client| {
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
    ask(state.lsp_for(&path).await, move |client| {
        client.document_symbols(&path)
    })
    .await
}

/// Symbols across the workspace whose names match `query`.
#[tauri::command]
pub async fn lsp_workspace_symbols(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::Symbol>, CommandError> {
    // Every server's, the project's own first: in a Cargo project with C,
    // `#` finds a C function as well as a Rust one.
    let mut found = Vec::new();
    for client in state.servers().await {
        let query = query.clone();
        found.extend(on_blocking(client, move |client| client.workspace_symbols(&query)).await?);
    }
    Ok(found)
}

/// The function at this position, where a call hierarchy starts.
#[tauri::command]
pub async fn lsp_call_hierarchy(
    path: String,
    line: u32,
    col: u32,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::CallItem>, CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        client.call_hierarchy(&path, line, col)
    })
    .await
}

/// The inlay hints over lines `from..to` of a file, and the mark of the
/// text they are about — the editor draws them only over that text.
#[tauri::command]
pub async fn lsp_inlay_hints(
    path: String,
    from: u32,
    to: u32,
    draft: Option<Draft>,
    state: State<'_, AppState>,
) -> Result<rusty_lsp::InlayHints, CommandError> {
    ask(state.lsp_for(&path).await, move |client| {
        synced(client, &path, draft.as_ref())?;
        client.inlay_hints(&path, from, to)
    })
    .await
}

/// Who calls the function `item` names, or what it calls: `item` is the
/// server's own description of it, handed back untouched.
#[tauri::command]
pub async fn lsp_calls(
    item: String,
    incoming: bool,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_lsp::Call>, CommandError> {
    // The item is the server's own description, and it names its file: the
    // server that described it is the one to ask.
    let uri = serde_json::from_str::<serde_json::Value>(&item)
        .ok()
        .and_then(|item| item["uri"].as_str().map(str::to_string))
        .unwrap_or_default();
    ask(state.lsp_for(&uri).await, move |client| {
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
    let files = state.files();
    ask(state.lsp_for(&path).await, move |client| {
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
}

/// The draft a question was asked over, given to the server before the
/// question in the same task. Without one the question is about whatever
/// the server holds.
fn synced(client: &LspClient, path: &str, draft: Option<&Draft>) -> rusty_lsp::Result<()> {
    match draft {
        Some(draft) => client.sync_draft(path, draft),
        None => Ok(()),
    }
}

/// Ask `server` — the one `AppState::lsp_for` picked for the file — on the
/// blocking pool, since the client waits on a pipe, and answer with nothing
/// when there is no server. The editor works without one: a list that is
/// empty while rust-analyzer starts is the warm-up talking, as a definition
/// that finds nothing is.
async fn ask<T: Default + Send + 'static>(
    server: Option<Arc<LspClient>>,
    question: impl FnOnce(&LspClient) -> rusty_lsp::Result<T> + Send + 'static,
) -> Result<T, CommandError> {
    match server {
        Some(client) => on_blocking(client, question).await,
        None => Ok(T::default()),
    }
}

/// `question`, asked of `client` on the blocking pool.
async fn on_blocking<T: Send + 'static>(
    client: Arc<LspClient>,
    question: impl FnOnce(&LspClient) -> rusty_lsp::Result<T> + Send + 'static,
) -> Result<T, CommandError> {
    Ok(blocking("the language server task", move || question(&client)).await??)
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
    let Some(client) = state.lsp_for(&path).await else {
        return Err(CommandError::new(
            "no language server is running, so nothing knows where this symbol is used",
        ));
    };
    on_blocking(client, move |client| {
        client.rename(&path, line, col, &new_name)
    })
    .await
}
