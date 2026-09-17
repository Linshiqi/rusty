//! Reading and writing the project's files.

use base64::Engine;
use rusty_edit::{Document, Entry};
use tauri::State;

use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

/// The project tree, with build output and dot-entries already excluded.
/// A walk of the whole project, so off the async thread.
#[tauri::command]
pub async fn file_tree(state: State<'_, AppState>) -> Result<Vec<Entry>, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("reading the tree", move || rusty_edit::read_tree(&root)).await??)
}

/// The `.rs` files under a crate's `src/` that no `mod` declaration reaches.
///
/// rust-analyzer's own verdict (`unlinked-file`) is authoritative and arrives
/// only for a file somebody has opened; the tree has to dim one before that,
/// so this reads the declarations itself and refuses wherever it cannot be
/// sure — see `rusty_edit::modules`.
#[tauri::command]
pub async fn unlinked_files(
    state: State<'_, AppState>,
) -> Result<Option<Vec<String>>, CommandError> {
    let Some(root) = state.root().await else {
        return Ok(None);
    };
    blocking("reading the module tree", move || {
        rusty_edit::scan_unlinked(&root)
    })
    .await
}

/// A new empty file or directory. Refuses names that already exist.
#[tauri::command]
pub async fn create_entry(
    path: String,
    dir: bool,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("creating the entry", move || {
        rusty_edit::create(&root, &path, dir)
    })
    .await??)
}

/// Move an entry into a directory (`""` for the root), keeping its name.
/// Answers with the new relative path; refuses a name that is taken.
#[tauri::command]
pub async fn move_entry(
    from: String,
    into: String,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("moving the entry", move || {
        rusty_edit::move_entry(&root, &from, &into)
    })
    .await??)
}

/// Copy an entry into a directory, under a free name. Answers with the
/// copy's relative path.
#[tauri::command]
pub async fn copy_entry(
    from: String,
    into: String,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("copying the entry", move || {
        rusty_edit::copy_entry(&root, &from, &into)
    })
    .await??)
}

/// Rename an entry in place. Answers with the new relative path.
#[tauri::command]
pub async fn rename_entry(
    from: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("renaming the entry", move || {
        rusty_edit::rename_entry(&root, &from, &name)
    })
    .await??)
}

/// Move an entry to the recycle bin.
#[tauri::command]
pub async fn delete_entry(path: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    Ok(blocking("deleting the entry", move || {
        rusty_edit::delete_entry(&root, &path)
    })
    .await??)
}

/// Show an entry in the platform's file manager, selected — Explorer's
/// `/select`, Finder's `open -R`, and the containing folder elsewhere,
/// where no file manager takes a selection portably.
#[tauri::command]
pub async fn reveal_entry(path: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    blocking("revealing the entry", move || {
        let target = rusty_edit::absolute(&root, &path)?;
        let mut command = if cfg!(target_os = "windows") {
            let mut command = rusty_embed::process::command("explorer");
            // One argument, no space after the comma: Explorer parses the
            // switch and the path out of a single string.
            command.arg(format!("/select,{}", target.display()));
            command
        } else if cfg!(target_os = "macos") {
            let mut command = rusty_embed::process::command("open");
            command.arg("-R").arg(&target);
            command
        } else {
            let mut command = rusty_embed::process::command("xdg-open");
            command.arg(target.parent().unwrap_or(&target));
            command
        };
        command.spawn().map(|_| ()).map_err(|error| {
            rusty_edit::Error::Io(format!("could not open the file manager: {error}"))
        })
    })
    .await??;
    Ok(())
}

/// A file in its own OS window — the same frontend, booted straight into
/// the editor by a `detach` query parameter. Asking twice focuses the
/// window that already exists instead of stacking a second copy.
#[tauri::command]
pub async fn open_editor_window(path: String, app: tauri::AppHandle) -> Result<(), CommandError> {
    use tauri::Manager;

    // FNV-1a over the path: stable, short, and two files cannot collide in
    // practice on one project.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let label = format!("edit-{hash:x}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        return Ok(());
    }

    let encoded = query_encode(&path);
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(format!("index.html?detach={encoded}").into()),
    )
    .title(format!("{name} — rusty"))
    .inner_size(980.0, 720.0)
    .build()
    .map_err(|error| CommandError::new(format!("could not open the editor window: {error}")))?;
    Ok(())
}

/// A value for a window's query string: everything outside
/// `[A-Za-z0-9._-/]` as `%XX`, which is exactly what the frontend's decoder
/// undoes. Shared by the editor window and the commit window.
pub(crate) fn query_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' | b'/' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// Send a detached editor's file back to the main window and close it.
///
/// The way back from a torn-off editor, which otherwise only had a close
/// button — the file went out and could not come home, so the tab had to be
/// reopened by hand. Dragging a window back the way VSCode does needs native
/// drop targets between windows; this is the same destination by a button.
///
/// The main window is told by event rather than by command because it is the
/// receiver, not the caller: nothing in it asked for this.
#[tauri::command]
pub async fn reattach_editor_window(
    path: String,
    window: tauri::Window,
    app: tauri::AppHandle,
) -> Result<(), CommandError> {
    use tauri::{Emitter, Manager};

    let main = app.get_webview_window("main").ok_or_else(|| {
        CommandError::new(
            "The main window is gone, so there is nowhere to \
                                          put this file back.",
        )
    })?;
    main.emit("rusty://reattach", path)
        .map_err(|error| CommandError::new(format!("could not hand the file over: {error}")))?;
    let _ = main.set_focus();
    // Only after the hand-off: closing first would leave the file nowhere if
    // the emit failed.
    let _ = window.close();
    Ok(())
}

/// One file, highlighted.
#[tauri::command]
pub async fn open_file(path: String, state: State<'_, AppState>) -> Result<Document, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    // The grammars are parsed from a bundled dump and take long enough that
    // doing it per file is noticeable, so the set is built once and kept.
    // Highlighting a large file is still work worth a blocking thread.
    let files = state.files();
    Ok(blocking("opening the file", move || files.open(&root, &path)).await??)
}

/// One file's bytes as base64 — how a figure in a page and an image opened
/// from the tree reach the screen, the way a picture in a diff does
/// (`git_blob`). The frontend adds the MIME type it read off the extension;
/// the read is confined to the project and capped, in `rusty_edit`.
#[tauri::command]
pub async fn read_blob(path: String, state: State<'_, AppState>) -> Result<String, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let bytes = blocking("reading the picture", move || {
        rusty_edit::read_bytes(&root, &path)
    })
    .await??;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Open a dependency's source read-only — where goto-definition lands when
/// the answer lives in esp-hal or `core` rather than in the project.
#[tauri::command]
pub async fn open_external(
    path: String,
    state: State<'_, AppState>,
) -> Result<Document, CommandError> {
    let files = state.files();
    Ok(
        tokio::task::spawn_blocking(move || files.open_external(&path))
            .await
            .map_err(|e| CommandError::new(format!("opening panicked: {e}")))??,
    )
}

/// Repaint an unsaved buffer against the painting the editor holds — `base`
/// the number it came with, `stale` the lines it is showing plain.
#[tauri::command]
pub async fn repaint_text(
    path: String,
    text: String,
    base: Option<u32>,
    stale: Option<(u32, u32)>,
    state: State<'_, AppState>,
) -> Result<rusty_edit::Repaint, CommandError> {
    let files = state.files();
    tokio::task::spawn_blocking(move || files.repaint(&path, &text, base, stale))
        .await
        .map_err(|e| CommandError::new(format!("highlighting panicked: {e}")))
}

/// Highlight a fenced code block by the language its fence names, for the
/// Markdown page. No project has to be open: a README in the assistant's
/// answer is a page too.
#[tauri::command]
pub async fn highlight_snippet(
    lang: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<Vec<rusty_edit::Line>, CommandError> {
    let files = state.files();
    tokio::task::spawn_blocking(move || files.highlight_snippet(&lang, &text))
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
    Ok(blocking("saving the file", move || {
        rusty_edit::save(&root, &path, &text)
    })
    .await??)
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

/// Rewrite every match the search would have listed.
///
/// `drafts` is the frontend's list of open editors with unsaved changes. It
/// is passed rather than inferred because only the window knows: the backend
/// holds no editor state, and a replace that wrote under a draft would put the
/// old text back on the next save.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceArgs {
    query: String,
    replacement: String,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    include: String,
    exclude: String,
    /// Open editors with unsaved changes. See the doc above.
    drafts: Vec<String>,
}

#[tauri::command]
pub async fn replace_in_project(
    args: ReplaceArgs,
    state: State<'_, AppState>,
) -> Result<rusty_edit::ReplaceOutcome, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let spec = rusty_edit::SearchQuery {
        text: args.query,
        case_sensitive: args.case_sensitive,
        whole_word: args.whole_word,
        regex: args.regex,
        include: args.include,
        exclude: args.exclude,
    };
    let replacement = args.replacement;
    let drafts = args.drafts;
    tokio::task::spawn_blocking(move || rusty_edit::replace(&root, &spec, &replacement, &drafts))
        .await
        .map_err(|e| CommandError::new(format!("replace panicked: {e}")))
}

/// Watch the project and stream a batch every time it settles.
///
/// Long-lived, like `lsp_start`: it does not resolve while the watcher is up,
/// so a resolved promise on the frontend means the watch ended.
///
/// A failure to start is an event's worth of nothing rather than an error. A
/// watcher can fail for reasons the user cannot act on — an exhausted inotify
/// budget, a network share — and the workbench still works without one; a red
/// banner about it would be crying wolf, exactly as it would for a missing
/// rust-analyzer.
///
/// The watcher lives in `AppState`'s slot, not in this loop. Replacing it —
/// the next project, or the next call here — drops it, and dropping it is
/// what closes `changes` and ends the loop. Before the slot existed the loop
/// itself held the watcher and waited for a failed `send` to let go, and a
/// send only fails once the WebView is gone: every project switch left one
/// more `notify` handle and one more thread behind, still pushing the old
/// tree's changes into a window that had moved on.
#[tauri::command]
pub async fn watch_project(
    on_change: tauri::ipc::Channel<rusty_edit::FileChanges>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    use tauri::Manager;

    let root = state.root().await.ok_or_else(CommandError::no_project)?;

    let watched = root.clone();
    let started = blocking("the file watcher", move || rusty_edit::watch(&watched)).await?;
    let Ok((watch, changes)) = started else {
        return Ok(());
    };

    let ticket = state.start_watch(watch).await;
    blocking("the file watcher", move || {
        // rust-analyzer watches nothing itself — it asked this client to
        // (`rusty_lsp::watched`) — so every batch is told to it as well:
        // what a structural change added and removed, as the difference
        // between two listings, and which files' contents moved.
        let mut known = relevant_files(&root);
        while let Ok(batch) = changes.recv() {
            let events = if batch.tree {
                let after = relevant_files(&root);
                let events = rusty_lsp::watched::file_events(&known, &after, &batch.changed);
                known = after;
                events
            } else {
                rusty_lsp::watched::file_events(&known, &known, &batch.changed)
            };
            if !events.is_empty()
                && let Some(client) = tauri::async_runtime::block_on(app.state::<AppState>().lsp())
            {
                let _ = client.did_change_watched_files(&events);
            }
            if on_change.send(batch).is_err() {
                // The WebView is gone — the one thing a failed send means.
                break;
            }
        }
    })
    .await?;
    // Only if the slot still holds *this* watcher. The loop also ends because
    // a successor replaced it, and the successor's entry is not ours to clear.
    state.release_watch(ticket).await;
    Ok(())
}

/// Every file under the project rust-analyzer reads, as the tree lists it —
/// the same walk and the same ignore rules, so what the server is told about
/// and what the Files panel shows cannot disagree about what exists.
fn relevant_files(root: &std::path::Path) -> std::collections::BTreeSet<String> {
    fn gather(entries: &[Entry], out: &mut std::collections::BTreeSet<String>) {
        for entry in entries {
            if entry.is_dir {
                gather(&entry.children, out);
            } else if rusty_lsp::watched::relevant(&entry.path) {
                out.insert(entry.path.clone());
            }
        }
    }
    let mut out = std::collections::BTreeSet::new();
    if let Ok(tree) = rusty_edit::read_tree(root) {
        gather(&tree, &mut out);
    }
    out
}
