//! The editor: the tree, the tabs, and the document in front of you.
//!
//! Parking is the load-bearing idea here. A tab that leaves the screen keeps
//! its draft, its caret and its history, so coming back to it is coming back
//! to where you were rather than to the top of the file.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_edit::{Document, Entry};
use rusty_embed::{LogLevel, LogLine, LogStream};
use rusty_i18n::t;

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{AppState, LspStatus, ParkedEditor, ParkedViewport},
};

/// Create a file or directory, then show it — the tree refreshes, and a new
/// file opens in the editor, because "New file" means "I want to type in it".
pub fn create_entry(state: AppState, path: String, dir: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        dir: bool,
    }

    if !state.has_project_now() {
        return;
    }
    let opened = path.clone();
    let args = Args { path, dir };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::CREATE, &args).await },
        move |()| {
            refresh_tree(state);
            if !dir {
                open_file(state, opened.clone());
            }
        },
    );
}

/// Where a moved or renamed path went, for everything that holds paths.
///
/// A file is the path itself; a directory is the path and everything under
/// it, so `src` → `firmware/src` takes `src/main.rs` along. `None` for a
/// path the move did not touch — `src2/a.rs` is not under `src`, which is
/// why the separator is checked and not just the prefix.
pub(crate) fn retarget(path: &str, from: &str, to: &str, is_dir: bool) -> Option<String> {
    if path == from {
        return Some(to.to_string());
    }
    if is_dir
        && let Some(rest) = path.strip_prefix(from)
        && rest.starts_with('/')
    {
        return Some(format!("{to}{rest}"));
    }
    None
}

/// Move the tabs, the parked editors, the document on screen, the expanded
/// folders and the source-view choices along with a path that moved on disk.
///
/// Before the watcher hears about it, on purpose: its batch would find the
/// old paths gone and mark every open file under them as vanished, when
/// they are all still there under the new name. rust-analyzer learns the
/// new name from the buffer the editor already holds.
fn follow_move(state: AppState, from: &str, to: &str, is_dir: bool) {
    let moved = |path: &str| retarget(path, from, to, is_dir);
    let mut announced = std::collections::HashSet::new();
    for group in state.open_groups() {
        group.editor.tabs.update(|tabs| {
            for tab in tabs.iter_mut() {
                if let Some(new) = moved(tab) {
                    *tab = new;
                }
            }
        });
        let mut reopened = Vec::new();
        group.editor.parked.update(|parked| {
            for entry in parked.iter_mut() {
                if let Some(new) = moved(&entry.document.path) {
                    let old = std::mem::replace(&mut entry.document.path, new.clone());
                    reopened.push((old, new, entry.draft.clone()));
                }
            }
        });
        let active = group.editor.document.with_untracked(|d| {
            d.as_ref()
                .and_then(|d| moved(&d.path).map(|new| (d.path.clone(), new)))
        });
        if let Some((old, new)) = active {
            group.editor.document.update(|d| {
                if let Some(d) = d {
                    d.path = new.clone();
                }
            });
            reopened.push((old, new, group.editor.draft.get_untracked()));
        }
        // The old name closed and the new one opened: left open, the server
        // kept a document at a path that no longer exists — a second copy
        // of the module, under its old name, for as long as the session ran.
        // Once per file: a file open on both sides moved once.
        for (old, new, text) in reopened {
            if announced.insert(old.clone()) {
                lsp_closed_doc(old);
                lsp_open_doc(new, text);
            }
        }
    }
    // The undo history is the file's, under whatever it is called now.
    state.editor.histories.update_value(|all| {
        let moved: Vec<(String, String)> = all
            .keys()
            .filter_map(|path| moved(path).map(|new| (path.clone(), new)))
            .collect();
        for (old, new) in moved {
            if let Some(history) = all.remove(&old) {
                all.insert(new, history);
            }
        }
    });
    let rename_all = |list: &mut Vec<String>| {
        for path in list.iter_mut() {
            if let Some(new) = moved(path) {
                *path = new;
            }
        }
    };
    state.editor.expanded.update(rename_all);
    state.editor.source_view.update(rename_all);
    state.editor.stale.update(rename_all);
    // Pictures are cached by path; the next look re-reads them.
    state
        .editor
        .images
        .update(|images| images.retain(|path, _| moved(path).is_none()));
    remember_tabs(state);
}

/// Move an entry into a directory (`""` for the root): a drop on the tree,
/// or Cut followed by Paste. The backend answers with where it landed.
pub fn move_entry(state: AppState, from: String, into: String, is_dir: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        from: String,
        into: String,
    }

    if !state.has_project_now() {
        return;
    }
    let args = Args {
        from: from.clone(),
        into,
    };
    track(
        state,
        async move { ipc::call::<_, String>(cmd::files::MOVE, &args).await },
        move |to| {
            if to != from {
                follow_move(state, &from, &to, is_dir);
            }
            refresh_tree(state);
        },
    );
}

/// Copy an entry into a directory. The copy takes a free name where its
/// own is taken, so pasting beside the original is ` copy`.
pub fn copy_entry(state: AppState, from: String, into: String) {
    #[derive(serde::Serialize)]
    struct Args {
        from: String,
        into: String,
    }

    if !state.has_project_now() {
        return;
    }
    let args = Args { from, into };
    track(
        state,
        async move { ipc::call::<_, String>(cmd::files::COPY, &args).await },
        move |_created| refresh_tree(state),
    );
}

/// Rename an entry in place. The open tabs follow, as after a move.
pub fn rename_entry(state: AppState, from: String, name: String, is_dir: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        from: String,
        name: String,
    }

    if !state.has_project_now() {
        return;
    }
    let args = Args {
        from: from.clone(),
        name,
    };
    track(
        state,
        async move { ipc::call::<_, String>(cmd::files::RENAME, &args).await },
        move |to| {
            if to != from {
                follow_move(state, &from, &to, is_dir);
            }
            refresh_tree(state);
        },
    );
}

/// Move an entry to the recycle bin, after asking. The question names the
/// bin the platform has, and every tab under the entry closes with it.
pub fn delete_entry(state: AppState, path: String, is_dir: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    if !state.has_project_now() {
        return;
    }
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    let question = if host_is_windows() {
        t!("tree.delete-confirm-bin", name = name)
    } else {
        t!("tree.delete-confirm-trash", name = name)
    };
    spawn_local(async move {
        // Through `ipc::confirm`, never `window.confirm`: see `close_tab`.
        if !ipc::confirm(&question).await {
            return;
        }
        let args = Args { path: path.clone() };
        track(
            state,
            async move { ipc::call::<_, ()>(cmd::files::DELETE, &args).await },
            move |()| {
                close_under(state, &path, is_dir);
                refresh_tree(state);
            },
        );
    });
}

/// Close every tab at or under a path that is gone. Without asking: the
/// question was asked about the file, and a draft of a deleted file has
/// nowhere left to be saved.
fn close_under(state: AppState, path: &str, is_dir: bool) {
    for group in state.open_groups() {
        let doomed: Vec<String> = group
            .editor
            .tabs
            .get_untracked()
            .into_iter()
            .filter(|tab| retarget(tab, path, "", is_dir).is_some())
            .collect();
        for tab in doomed {
            remove_tab(group, tab);
        }
    }
}

/// Select the entry in the platform's file manager.
pub fn reveal_entry(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::REVEAL, &Args { path }).await },
        |()| {},
    );
}

/// Which desktop this window is on, for the words that differ: Explorer or
/// Finder, the Recycle Bin or the Trash.
pub fn host_is_windows() -> bool {
    host_platform().starts_with("Win")
}

pub fn host_is_mac() -> bool {
    host_platform().starts_with("Mac")
}

fn host_platform() -> String {
    web_sys::window()
        .and_then(|w| w.navigator().platform().ok())
        .unwrap_or_default()
}

/// Float a file into its own OS window.
pub fn detach_file(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::DETACH, &Args { path }).await },
        |()| {},
    );
}

/// Push the remembered interface scale to the webview. Through `track`, so
/// "command not found" — the stale-backend symptom — surfaces as a banner
/// instead of a slider that silently does nothing.
pub fn apply_ui_zoom(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        factor: f64,
    }
    let factor = state.layout.zoom.get_untracked();
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::window::SET_ZOOM, &Args { factor }).await },
        |()| {},
    );
}

/// The shells the picker can offer. Loaded once; a machine does not grow
/// shells mid-session often enough to poll for.
pub fn load_shell_choices(state: AppState) {
    if state.term.choices.with_untracked(|c| !c.is_empty()) {
        return;
    }
    track(
        state,
        async move { ipc::get::<Vec<rusty_term::ShellChoice>>(cmd::terminal::SHELLS).await },
        move |choices| state.term.choices.set(choices),
    );
}

/// What shell the terminal will start, and where the choice came from.
pub fn load_shell_info(state: AppState) {
    track(
        state,
        async move { ipc::call::<_, rusty_term::ShellInfo>(cmd::terminal::SHELL_INFO, &()).await },
        move |info| state.term.info.set(Some(info)),
    );
}

/// Store the shell preference and restart the shell so it takes effect —
/// a preference that waits for the next launch reads as a broken setting.
pub fn set_terminal_shell(state: AppState, value: Option<String>) {
    #[derive(serde::Serialize)]
    struct Args {
        value: Option<String>,
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::terminal::SET_SHELL, &Args { value }).await },
        move |()| {
            close_terminal(state);
            load_shell_info(state);
        },
    );
}

// ─── detached windows ────────────────────────────────────────────────────────

/// Hand this window's file back to the shell and close.
pub fn reattach(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }
    let args = Args { path };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::REATTACH, &args).await },
        |()| {},
    );
}

/// Reopen a file a detached window is handing back.
///
/// Installed by the shell only: a detached window is one file's editor, and
/// reopening somebody else's tab is exactly the project-wide behaviour it is
/// supposed to stay out of.
pub fn watch_reattach(state: AppState) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    // Guarded like every other call that runs at mount. `catch` on an async
    // extern turns a *rejected promise* into `Err` and nothing else: with no
    // `window.__TAURI__`, the shim throws synchronously, the generated glue
    // swallows that and hands back `undefined`, and the wasm side then calls
    // `.then` on it. The uncaught TypeError kills the executor, so the page
    // paints once and answers nothing afterwards — no banner, no clue.
    if !ipc::backend_available() {
        return;
    }

    #[derive(serde::Deserialize)]
    struct Event {
        payload: String,
    }

    let handler = Closure::wrap(Box::new(move |event: JsValue| {
        if let Ok(event) = serde_wasm_bindgen::from_value::<Event>(event) {
            open_file(state, event.payload);
        }
    }) as Box<dyn FnMut(JsValue)>);
    // Taken before forgetting, because the handle is what `listen` needs and
    // the closure has to outlive this task either way.
    let js = handler.as_ref().clone();
    handler.forget();
    spawn_local(async move {
        let _ = ipc::listen("rusty://reattach", js).await;
    });
}

/// Re-read the project tree, and which of its files the module tree reaches.
pub fn refresh_tree(state: AppState) {
    if !state.has_project_now() {
        return;
    }
    track(
        state,
        async move { ipc::call::<_, Vec<Entry>>(cmd::files::TREE, &()).await },
        move |entries| state.editor.tree.set(entries),
    );
    refresh_unlinked(state);
}

/// Which `.rs` files no `mod` declaration reaches, for the tree to dim.
///
/// Not through `track`: a project whose module tree cannot be read is not a
/// failure worth a banner — the answer is a shade of grey. `None` is the
/// refusal (`rusty_edit::modules` claims nothing where it cannot be sure)
/// and travels as itself, so nothing here can turn a doubt into a dim, and
/// a call that fails outright leaves the last claim alone rather than
/// silently un-dimming every file in the project.
pub fn refresh_unlinked(state: AppState) {
    spawn_local(async move {
        if let Ok(claim) = ipc::call::<_, Option<Vec<String>>>(cmd::files::UNLINKED, &()).await {
            set_if_changed(state.editor.unlinked, claim);
        }
    });
}

/// Open a file for reading and editing.
///
/// Already on screen: nothing happens — a re-read here would replace an
/// unsaved draft with the disk's older text, which is how editors eat work.
/// Parked: the tab is fronted with its draft intact. New: fetched, and
/// whatever was on screen is parked.
pub fn open_file(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    let active = state.active_path_now();
    if active.as_deref() == Some(path.as_str()) {
        return;
    }
    if state
        .editor
        .parked
        .with_untracked(|parked| parked.iter().any(|e| e.document.path == path))
    {
        activate_tab(state, path);
        return;
    }
    // The other group holds it: a second view of the same document, never a
    // second read of the disk beside an unsaved draft (`views.rs`).
    if other_holds(state, &path) && open_view(state, &path) {
        return;
    }

    let args = Args { path };
    track(
        state,
        async move { ipc::call::<_, Document>(cmd::files::OPEN, &args).await },
        move |document| show_document(state, document, true),
    );
}

/// Open a file the *workbench* asked for rather than the user: a tab being
/// restored from `workbench.toml`.
///
/// A path that is no longer there drops off the strip instead of raising a
/// banner. The strip is remembered per project directory, and a project can
/// be deleted and recreated at the same path — the wizard does exactly that
/// — so a remembered `firmware/src/bin/main.rs` over a directory that now
/// holds something else greeted a freshly generated project with a red
/// error about a file nobody had asked for. `restore_tabs` has always said
/// it fails quietly; it went through the ordinary path, which banners.
pub(crate) fn reopen_file(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    // Restored on both sides, and the other side has read it already.
    if other_holds(state, &path) && open_view(state, &path) {
        return;
    }

    let args = Args { path: path.clone() };
    spawn_local(async move {
        match ipc::call::<_, Document>(cmd::files::OPEN, &args).await {
            Ok(document) => show_document(state, document, true),
            // Gone. Take it off the strip in every group that lists it, so
            // the next click cannot ask again.
            Err(_) => {
                for editor in state.groups.editors {
                    editor.tabs.update(|tabs| tabs.retain(|tab| tab != &path));
                }
                remember_tabs(state);
            }
        }
    });
}

/// Fetch a project file as a picture — a figure in a page, an image opened
/// from the tree — into `editor.images`, once per path.
///
/// Not through `track`: a figure that is not on disk is a chip in the page
/// saying so, not a banner over the workbench. A chapter with a missing
/// figure is a document with a typo in it, not a failure of the tool. And
/// only for paths whose extension names an image the WebView can draw; a
/// `<img src="notes.pdf">` is left to the page to describe.
pub fn load_image(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    let Some(mime) = rusty_git::image_mime(&path) else {
        return;
    };
    let asked = state
        .editor
        .images
        .with_untracked(|images| images.contains_key(&path));
    if asked {
        return;
    }
    state.editor.images.update(|images| {
        images.insert(path.clone(), crate::state::ImageLoad::Loading);
    });
    let args = Args { path: path.clone() };
    spawn_local(async move {
        let outcome = match ipc::call::<_, String>(cmd::files::BLOB, &args).await {
            Ok(encoded) => crate::state::ImageLoad::Ready(format!("data:{mime};base64,{encoded}")),
            Err(error) => crate::state::ImageLoad::Failed(error.message),
        };
        state.editor.images.update(|images| {
            images.insert(path, outcome);
        });
    });
}

/// Highlight a fenced code block for the Markdown page, once per distinct
/// block.
///
/// The runs land in `editor.snippets` under a hash of the language and the
/// text, and an empty entry goes in before the request leaves, so a page
/// re-rendered on every keystroke asks for each block once. The same block
/// in two chapters, or in a page and an answer, is one request. Without a
/// backend — the trunk-only preview — the block stays as it was written.
pub fn highlight_snippet(state: AppState, lang: String, text: String) {
    #[derive(serde::Serialize)]
    struct Args {
        lang: String,
        text: String,
    }

    if !ipc::backend_available() {
        return;
    }
    let key = crate::state::snippet_key(&lang, &text);
    let asked = state
        .editor
        .snippets
        .with_untracked(|snippets| snippets.contains_key(&key));
    if asked {
        return;
    }
    state.editor.snippets.update(|snippets| {
        // A book's worth of blocks is a few hundred; a session that reads
        // many books need not keep them all.
        if snippets.len() > 2_000 {
            snippets.clear();
        }
        snippets.insert(key, Vec::new());
    });
    let args = Args { lang, text };
    spawn_local(async move {
        if let Ok(lines) =
            ipc::call::<_, Vec<rusty_edit::Line>>(cmd::files::HIGHLIGHT_SNIPPET, &args).await
        {
            state.editor.snippets.update(|snippets| {
                snippets.insert(key, lines);
            });
        }
    });
}

/// Re-read the active document from disk and replace it in place — the tail
/// of a save, where disk and draft have just been made equal.
fn reload_active(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    let args = Args { path };
    track(
        state,
        async move { ipc::call::<_, Document>(cmd::files::OPEN, &args).await },
        move |document| show_document(state, document, true),
    );
}

/// Put a freshly loaded document on screen.
///
/// A different path parks the current editor first; the same path replaces it
/// in place, which is how a save's re-read lands without disturbing the strip.
pub(super) fn show_document(state: AppState, document: Document, announce: bool) {
    let active = state.active_path_now();
    let reread = active.as_deref() == Some(document.path.as_str());
    if active.is_some() && !reread {
        park_active(state);
    }
    // A file read fresh is a new document, and its history starts empty —
    // unless the other side holds it, whose edits the history is of.
    if !reread && !other_holds(state, &document.path) {
        state.editor.forget_history(&document.path);
    }
    state.editor.tabs.update(|tabs| {
        if !tabs.iter().any(|t| t == &document.path) {
            tabs.push(document.path.clone());
        }
    });
    // Any parked copy is staler than what was just fetched.
    state
        .editor
        .parked
        .update(|parked| parked.retain(|e| e.document.path != document.path));
    clear_editor_transients(state);
    // The draft is seeded from the document exactly once, here. Setting it
    // anywhere else would overwrite whatever had been typed.
    state.editor.draft.set(document.text.clone());
    state.editor.echo_text.set(document.text.clone());
    state.editor.highlighted.set(document.lines.clone());
    painted_whole(state, document.paint, None);
    if announce && !document.read_only && state.lsp.status.get_untracked() == LspStatus::Ready {
        lsp_open_doc(document.path.clone(), document.text.clone());
        request_semantic(state, document.path.clone());
        request_hints(state, document.path.clone());
    }
    // A different document opens at the top. The working area's scroller is
    // one DOM element for every document that passes through it, so without
    // this the new file opened wherever the old one had been scrolled to. A
    // save's re-read is the same path and keeps its place.
    if active.as_deref() != Some(document.path.as_str()) {
        state.editor.viewport.set(Some(ParkedViewport {
            path: document.path.clone(),
            top: 0,
            left: 0,
            caret: None,
        }));
    }
    // One of the two doors a file comes on screen through; `front_parked` is
    // the other. Ctrl+Tab's order is read off this.
    state
        .editor
        .recent
        .update_value(|recent| recent.touch(&document.path));
    state.editor.document.set(Some(document));
    // A save's re-read: what the disk holds now, for the other view too.
    if reread {
        share_document(state);
    }
}

/// A whole painting went on screen: say which it is and which of its lines
/// are plain, and drop the repaint on its way, whose answer is about
/// another painting.
pub(crate) fn painted_whole(state: AppState, version: Option<u32>, stale: crate::paint::Lines) {
    state
        .editor
        .paint
        .set_value(crate::state::PaintState { version, stale });
    state.editor.painting.set_value(None);
}

/// Stash the on-screen editor into the parked set, caret and all.
pub(super) fn park_active(state: AppState) {
    let Some(document) = state.editor.document.get_untracked() else {
        return;
    };
    let entry = ParkedEditor {
        draft: state.editor.draft.get_untracked(),
        highlighted: state.editor.highlighted.get_untracked(),
        paint: state.editor.paint.get_value(),
        caret: active_caret(state),
        folds: state.editor.folds.get_untracked(),
        viewport: viewport_position(state),
        document,
    };
    state.editor.parked.update(|parked| {
        parked.retain(|e| e.document.path != entry.document.path);
        parked.push(entry);
    });
}

/// The active editor's caret as (line, scalar column), read off the DOM.
///
/// The controller reaching into the DOM is unusual, but the alternative is
/// threading a caret through every caller of every function that might park —
/// and the editor's textarea is as much a singleton as the signals are.
fn active_caret(state: AppState) -> Option<(u32, u32)> {
    caret_position(state)
}

fn clear_editor_transients(state: AppState) {
    // Folds describe *this* document's line numbers. Carried into another
    // file they would collapse whatever happens to be at those lines, which
    // is a file that opens with its middle missing.
    state.editor.folds.set(rusty_edit::Folded::default());
    dismiss_completion(state);
    state.editor.signature.set(None);
    state.editor.hover.set(None);
    state.editor.semantic.set(None);
    state.editor.hints.set(None);
    state.editor.cursors.set(Vec::new());
    state.editor.semantic_lines.set_value(None);
    state.editor.occurrences.set(None);
    // A new document opens at its top; the view says otherwise once it draws.
    state.editor.drawn_lines.set_value((0, 0));
    state.editor.actions.set(None);
    // A viewport still waiting for a view that never mounted belongs to a
    // document that is no longer coming.
    state.editor.viewport.set(None);
    // Whatever the textarea was behind on is not on screen any more.
    state.editor.lagging.set(false);
}

/// Front an already open tab, parking the current one.
///
/// A strip entry with no parked body is a tab the last session left open
/// and nobody has clicked since: `restore_tabs` lists the strip whole and
/// reads only the active file, so the others are read here, on the click,
/// exactly as a click in the tree reads them. This used to treat such an
/// entry as corrupt and drop it — and after every restart the first click
/// on any restored tab closed it instead of opening it.
pub fn activate_tab(state: AppState, path: String) {
    let active = state.active_path_now();
    if active.as_deref() == Some(path.as_str()) {
        return;
    }
    let parked = state
        .editor
        .parked
        .with_untracked(|parked| parked.iter().any(|e| e.document.path == path));
    if !parked {
        open_file(state, path);
        return;
    }
    park_active(state);
    front_parked(state, &path);
}

/// Move a parked editor onto the screen. False when no such entry exists.
pub(super) fn front_parked(state: AppState, path: &str) -> bool {
    let mut taken = None;
    state.editor.parked.update(|parked| {
        if let Some(at) = parked.iter().position(|e| e.document.path == path) {
            taken = Some(parked.remove(at));
        }
    });
    let Some(entry) = taken else {
        return false;
    };
    clear_editor_transients(state);
    let dirty = entry.draft != entry.document.text;
    let read_only = entry.document.read_only;
    // After `clear_editor_transients` has reset them, or the tab comes back
    // flat while its caret comes back where it was.
    state.editor.folds.set(entry.folds);
    state.editor.draft.set(entry.draft.clone());
    state.editor.echo_text.set(entry.draft);
    state.editor.highlighted.set(entry.highlighted);
    painted_whole(state, entry.paint.version, entry.paint.stale);
    state
        .editor
        .recent
        .update_value(|recent| recent.touch(path));
    state.editor.document.set(Some(entry.document));
    // Back to what was on screen, not merely to the caret: the view puts the
    // caret where it was without scrolling, then the scroller where it was.
    // A jump into this file that follows (`open_at`, a definition, Back)
    // sets `reveal`, and the view lets that win.
    state.editor.viewport.set(Some(ParkedViewport {
        path: path.to_string(),
        top: entry.viewport.0,
        left: entry.viewport.1,
        caret: entry.caret,
    }));
    // An edited draft's parked highlight may be a pulse behind; freshen it.
    // Clean or read-only tabs have nothing to freshen.
    if dirty && !read_only {
        schedule_pulse(state);
    }
    if !read_only {
        request_semantic(state, path.to_string());
        request_hints(state, path.to_string());
    }
    true
}

/// Close a tab. Discarding unsaved work requires saying so first.
pub fn close_tab(state: AppState, path: String) {
    let active = state.active_path_now();
    let is_active = active.as_deref() == Some(path.as_str());

    let dirty = if is_active {
        state.editor.document.with_untracked(|d| {
            d.as_ref().is_some_and(|d| {
                !d.read_only && state.editor.draft.with_untracked(|draft| draft != &d.text)
            })
        })
    } else {
        state.editor.parked.with_untracked(|parked| {
            parked
                .iter()
                .find(|e| e.document.path == path)
                .is_some_and(|e| !e.document.read_only && e.draft != e.document.text)
        })
    };
    // Open on the other side too, the draft is not going anywhere: that view
    // holds the same document, and closing this one loses nothing.
    if dirty && !other_holds(state, &path) {
        // Asked through `ipc::confirm`, never `window.confirm` directly: in
        // the app that global is the dialog plugin's async shim, and read as
        // a boolean it is always "no" — a dirty tab that could not be closed.
        let question = t!("misc.discard-confirm", path = path.clone());
        spawn_local(async move {
            if ipc::confirm(&question).await {
                remove_tab(state, path);
            }
        });
        return;
    }
    remove_tab(state, path);
}

/// The close itself, once any question about unsaved work has been answered.
fn remove_tab(state: AppState, path: String) {
    let active = state.active_path_now();
    let is_active = active.as_deref() == Some(path.as_str());
    let next = if is_active {
        neighbour_after_close(&state.editor.tabs.get_untracked(), &path)
    } else {
        None
    };
    state.editor.tabs.update(|tabs| tabs.retain(|t| t != &path));
    state
        .editor
        .parked
        .update(|parked| parked.retain(|e| e.document.path != path));
    // A file no view holds any more: the server goes back to the disk for
    // it, and its undo history goes with the document.
    if !other_holds(state, &path) {
        lsp_closed_doc(path.clone());
        state.editor.forget_history(&path);
    }

    if is_active {
        clear_editor_transients(state);
        let fronted = next.as_deref().is_some_and(|n| front_parked(state, n));
        if !fronted {
            clear_screen(state);
            // The neighbour is listed and has no body: restored last session
            // and never clicked. Read it, as `activate_tab` would — a blank
            // pane under a strip that still names a file reads as a failure.
            if let Some(next) = next {
                open_file(state, next);
            }
        }
    }
    settle_groups(state);
}

/// Nothing on screen: the state a group is in before its first file and
/// after its last.
fn clear_screen(state: AppState) {
    state.editor.document.set(None);
    state.editor.draft.set(String::new());
    state.editor.echo_text.set(String::new());
    state.editor.highlighted.set(Vec::new());
    painted_whole(state, None, None);
}

// ─── two groups ─────────────────────────────────────────────────────────────────

/// Open a file in the right-hand group — VS Code's "Open to the Side" —
/// splitting the editor if it is not split yet.
///
/// "Beside" is the right group from *either* side. With two groups there is
/// nothing further right, and the first version sent a file to "the other
/// group": from the right group that moved it left, and when it was the
/// right group's last file the right group vanished under the click — read,
/// correctly, as the split closing for no reason. A file the left group
/// holds opens on the right as a second view of the same document and stays
/// on the left, as VS Code's split does (`views.rs`); one the right group
/// holds is fronted there.
pub fn open_beside(state: AppState, path: String) {
    let second = state.group(crate::state::Group::Second);
    state.layout.split.set(true);
    state.layout.focus.set(second.group);
    open_file(second, path);
}

/// The left strip's split button and Ctrl+\: the left group's file opens on
/// the right as well. Only the left group has the button — there is nothing
/// further right of the right group.
pub fn split_active(state: AppState) {
    let first = state.group(crate::state::Group::First);
    let Some(path) = first.active_path_now() else {
        return;
    };
    open_beside(first, path);
}

/// After a group lost a file. A second group with nothing left closes; a
/// first group with nothing left while the second has files takes them, so
/// the split never shows an empty pane on the left with the work on the
/// right, and never outlives having two things to compare.
fn settle_groups(state: AppState) {
    if !state.layout.split.get_untracked() {
        return;
    }
    let first = state.group(crate::state::Group::First);
    let second = state.group(crate::state::Group::Second);
    if second.editor.tabs.with_untracked(Vec::is_empty) {
        state.layout.split.set(false);
        state.layout.focus.set(crate::state::Group::First);
    } else if first.editor.tabs.with_untracked(Vec::is_empty) {
        let active = second.active_path_now();
        park_active(second);
        first.editor.tabs.set(second.editor.tabs.get_untracked());
        first
            .editor
            .parked
            .set(second.editor.parked.get_untracked());
        second.editor.tabs.set(Vec::new());
        second.editor.parked.set(Vec::new());
        clear_editor_transients(second);
        clear_screen(second);
        state.layout.split.set(false);
        state.layout.focus.set(crate::state::Group::First);
        if let Some(active) = active
            && !front_parked(first, &active)
        {
            open_file(first, active);
        }
    }
}

/// Forget everything a group holds: the project is changing under it.
pub fn reset_group(state: AppState) {
    state.editor.tabs.set(Vec::new());
    state.editor.parked.set(Vec::new());
    // Keyed by path, and the next project has a `src/main.rs` too.
    state.editor.histories.update_value(|all| all.clear());
    clear_editor_transients(state);
    clear_screen(state);
    state.editor.reveal.set(None);
    state.find.open.set(false);
    state.find.replace_open.set(false);
    state.find.query.set(String::new());
    state.find.index.set(0);
}

/// Fold the file tree away or bring it back: the Files switcher's second
/// click, Ctrl+B and the View menu.
pub fn toggle_tree(state: AppState) {
    let hidden = !state.layout.tree_hidden.get_untracked();
    state.layout.tree_hidden.set(hidden);
    crate::state::remember_tree_hidden(hidden);
}

/// Which tab takes the screen when this one closes: the one after it, else
/// the one before, else nothing.
fn neighbour_after_close(tabs: &[String], closing: &str) -> Option<String> {
    let at = tabs.iter().position(|t| t == closing)?;
    tabs.get(at + 1)
        .or_else(|| at.checked_sub(1).and_then(|i| tabs.get(i)))
        .cloned()
}

/// Open a dependency's source read-only — where goto-definition lands when the
/// answer lives in esp-hal or `core`.
pub fn open_external(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    let active = state.active_path_now();
    if active.as_deref() == Some(path.as_str()) {
        return;
    }
    if state
        .editor
        .parked
        .with_untracked(|parked| parked.iter().any(|e| e.document.path == path))
    {
        activate_tab(state, path);
        return;
    }

    let args = Args { path };
    track(
        state,
        async move { ipc::call::<_, Document>(cmd::files::OPEN_EXTERNAL, &args).await },
        // announce=false, deliberately: the server already knows this file as
        // part of the sysroot or a dependency, and announcing it as an
        // editable document would be a lie the read-only flag exists to
        // prevent.
        move |document| show_document(state, document, false),
    );
}

/// Write the draft back without being asked, a beat after typing stopped.
///
/// Not [`save_file`]: that one re-reads the file afterwards, which is right
/// for Ctrl+S — the user has stopped — and is an editor eating work here.
/// The round trip takes tens of milliseconds, typing continues through it,
/// and the re-read then puts the disk's copy into `draft` over the keys
/// pressed since. Nor [`format_then_save`]: rustfmt under the fingers
/// rewrites the line being typed, and mid-expression it cannot parse at all,
/// so every second would put a failure in the dock.
///
/// So: write, and move the *document* forward to exactly the bytes written.
/// The dirty dot is `draft != document.text`, so it clears by itself and
/// stays honest — it lights again the moment the next key is pressed. The
/// draft is never touched.
pub fn autosave_file(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }

    let Some(document) = state.editor.document.with_untracked(Clone::clone) else {
        return;
    };
    // The refusal a manual save makes: a library's source is not ours.
    if document.read_only {
        return;
    }
    let path = document.path.clone();
    let text = state.editor.draft.get_untracked();
    if text == document.text {
        return;
    }
    let args = Args {
        path: path.clone(),
        text: text.clone(),
    };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::SAVE, &args).await },
        move |()| {
            lsp_saved_doc(path.clone());
            clear_stale(state, &path);
            // What is on disk is now this text, whatever has been typed
            // since. Only the document this write was for: the tab may have
            // changed under the round trip.
            state.editor.document.update(|open| {
                if let Some(open) = open
                    && open.path == path
                {
                    open.text = text.clone();
                }
            });
            if state.active_path_now().as_deref() == Some(path.as_str()) {
                share_document(state);
            }
        },
    );
}

/// Every unsaved draft written — the file in front in either group and
/// every tab parked behind them — and then `then`, once all of it is on
/// disk. What Build, Run and Flash do first: what runs is the code on
/// screen, the way Wokwi's Run and VS Code's launch save before they start.
///
/// Written the way auto-save writes (`autosave_file`): each document moves
/// forward to exactly the bytes written and the draft is never touched, so
/// a key pressed during the round trip is not replaced by the disk's copy.
/// A write that fails stops here with the banner — building the version on
/// disk while the screen shows another is the thing this exists to prevent.
pub fn save_all_then(state: AppState, then: impl FnOnce() + 'static) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }

    // One write per path: a file open on both sides has one draft, kept the
    // same in both groups.
    let mut writes: Vec<(String, String)> = Vec::new();
    for editor in state.groups.editors {
        let active = editor.document.with_untracked(|doc| {
            doc.as_ref()
                .filter(|doc| !doc.read_only && !rusty_edit::is_expansion(&doc.path))
                .map(|doc| (doc.path.clone(), doc.text.clone()))
        });
        if let Some((path, on_disk)) = active {
            let draft = editor.draft.get_untracked();
            if draft != on_disk && !writes.iter().any(|(p, _)| p == &path) {
                writes.push((path, draft));
            }
        }
        editor.parked.with_untracked(|parked| {
            for tab in parked {
                let path = &tab.document.path;
                if !tab.document.read_only
                    && !rusty_edit::is_expansion(path)
                    && tab.draft != tab.document.text
                    && !writes.iter().any(|(p, _)| p == path)
                {
                    writes.push((path.clone(), tab.draft.clone()));
                }
            }
        });
    }
    if writes.is_empty() {
        then();
        return;
    }

    state.app.in_flight.update(|n| *n += 1);
    spawn_local(async move {
        for (path, text) in writes {
            let args = Args {
                path: path.clone(),
                text: text.clone(),
            };
            if let Err(error) = ipc::call::<_, ()>(cmd::files::SAVE, &args).await {
                state.app.in_flight.update(|n| *n = n.saturating_sub(1));
                state.app.error.set(Some(error));
                return;
            }
            lsp_saved_doc(path.clone());
            clear_stale(state, &path);
            // Every copy of this document, in both groups, is now these
            // bytes on disk — the file in front and a tab parked behind.
            for editor in state.groups.editors {
                editor.document.update(|open| {
                    if let Some(open) = open
                        && open.path == path
                    {
                        open.text = text.clone();
                    }
                });
                editor.parked.update(|parked| {
                    for tab in parked.iter_mut() {
                        if tab.document.path == path {
                            tab.document.text = text.clone();
                        }
                    }
                });
            }
        }
        state.app.in_flight.update(|n| *n = n.saturating_sub(1));
        then();
    });
}

/// Write the current draft back.
pub fn save_file(state: AppState) {
    // A dependency's source is not this project's to change; the backend would
    // refuse the path anyway, but a red banner for pressing Ctrl+S in a file
    // that *looks* editable would blame the user for our affordance.
    if state
        .editor
        .document
        .with_untracked(|d| d.as_ref().is_some_and(|d| d.read_only))
    {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }

    let Some(path) = state.active_path_now() else {
        return;
    };
    let args = Args {
        path: path.clone(),
        text: state.editor.draft.get_untracked(),
    };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::SAVE, &args).await },
        move |()| {
            lsp_saved_doc(path.clone());
            // Whatever the disk held is gone now — the user chose this write
            // over it, so the warning has done its job and must not linger.
            clear_stale(state, &path);
            // Re-read so the highlighting matches what is now on disk, and so
            // the saved/unsaved marker clears against real content rather than
            // against an assumption that the write did what was asked.
            reload_active(state, path.clone());
        },
    );
}

/// Format with rustfmt, then save.
///
/// A rustfmt failure — usually a parse error mid-edit — never blocks the
/// save; the reason goes to the dock instead. `apply` is the editor's own
/// hand: it re-echoes the text and puts the caret back, because the DOM
/// element lives with the view, not here.
pub fn format_then_save(
    state: AppState,
    caret: Option<(u32, u32)>,
    apply: impl Fn(&str, Option<(u32, u32)>) + 'static,
) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }

    let Some(document) = state.editor.document.with_untracked(Clone::clone) else {
        return;
    };
    if document.read_only {
        return;
    }
    // The backend names the grammar as syntect does, `Rust`.
    let is_rust = document
        .language
        .as_deref()
        .is_some_and(|language| language.eq_ignore_ascii_case("rust"))
        || document.path.ends_with(".rs");
    if !is_rust {
        save_file(state);
        return;
    }

    let args = Args {
        path: document.path,
        text: state.editor.draft.get_untracked(),
    };
    spawn_local(async move {
        match ipc::call::<_, rusty_edit::Formatted>(cmd::files::FORMAT, &args).await {
            Ok(formatted) if formatted.changed => {
                state.editor.draft.set(formatted.text.clone());
                apply(&formatted.text, caret);
            }
            Ok(_) => {}
            Err(error) => {
                // The save below still happens — an unformatted save is a
                // save; a blocked one is data loss waiting for a fix.
                state.push_log(LogLine {
                    stream: LogStream::Stderr,
                    text: format!("rustfmt skipped this save: {}", error.message),
                    level: Some(LogLevel::Warn),
                });
            }
        }
        save_file(state);
    });
}

#[cfg(test)]
mod tab_tests {
    use super::{neighbour_after_close, retarget};

    fn tabs(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn the_next_tab_inherits_the_screen() {
        let strip = tabs(&["a.rs", "b.rs", "c.rs"]);
        assert_eq!(
            neighbour_after_close(&strip, "b.rs").as_deref(),
            Some("c.rs")
        );
    }

    #[test]
    fn the_last_tab_falls_back_to_the_previous() {
        let strip = tabs(&["a.rs", "b.rs"]);
        assert_eq!(
            neighbour_after_close(&strip, "b.rs").as_deref(),
            Some("a.rs")
        );
    }

    #[test]
    fn closing_the_only_tab_leaves_nothing() {
        let strip = tabs(&["a.rs"]);
        assert_eq!(neighbour_after_close(&strip, "a.rs"), None);
    }

    #[test]
    fn closing_a_tab_not_in_the_strip_is_a_no_op() {
        let strip = tabs(&["a.rs"]);
        assert_eq!(neighbour_after_close(&strip, "zz.rs"), None);
    }

    /// A directory takes everything under it; a file takes only itself; and
    /// `src2` is not under `src`, however the prefix reads.
    #[test]
    fn a_moved_path_carries_what_is_under_it_and_nothing_beside_it() {
        assert_eq!(
            retarget("src/main.rs", "src", "firmware/src", true).as_deref(),
            Some("firmware/src/main.rs")
        );
        assert_eq!(
            retarget("src", "src", "firmware/src", true).as_deref(),
            Some("firmware/src")
        );
        assert_eq!(retarget("src2/a.rs", "src", "firmware/src", true), None);
        assert_eq!(
            retarget("a.rs", "a.rs", "lib/b.rs", false).as_deref(),
            Some("lib/b.rs")
        );
        assert_eq!(retarget("a.rs/x", "a.rs", "b.rs", false), None);
        assert_eq!(retarget("README.md", "src", "firmware/src", true), None);
    }
}
