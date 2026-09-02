//! Following the project when something else changes it.
//!
//! `git checkout` moves half the tree, `cargo add` rewrites a manifest,
//! another editor saves, a build script regenerates a file. Before this, the
//! window kept showing whatever it had read when the project was opened, and
//! the way back was a refresh button somebody had to know about.
//!
//! **The rule that matters is what happens to unsaved work: nothing.** A tab
//! whose draft differs from what was read is never reloaded — it is *marked*,
//! and the marker says the disk moved underneath it. An editor that silently
//! replaced a draft with the disk's copy would be an editor that eats work,
//! and the same reasoning that makes `open_file` refuse to re-read an already
//! open file applies with more force here, because nobody asked for this read
//! at all.
//!
//! A save of our own comes back through the watcher too. That is not a
//! problem and is not filtered: the tab is clean at that moment, so the reload
//! finds identical text. Filtering our own writes would mean keeping a list of
//! paths in flight, and a list like that is wrong exactly when an external
//! change lands in the same window.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_edit::{Document, FileChanges};

use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

/// Watch the open project, and keep watching until it is replaced.
pub fn start_watch(state: AppState) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    if !state.has_project() {
        return;
    }
    // A project switch leaves the old watcher's channel alive until its
    // backend task notices; the session number is how its batches are told
    // apart from the live one. Same shape as the LSP's, and for the same
    // reason: without it, the previous project's tree refreshes this one.
    let session = state.editor.watch_session.get_untracked() + 1;
    state.editor.watch_session.set(session);

    let channel = ipc::Channel::new();
    let on_change = Closure::wrap(Box::new(move |value: JsValue| {
        if state.editor.watch_session.get_untracked() != session {
            return;
        }
        if let Ok(changes) = serde_wasm_bindgen::from_value::<FileChanges>(value) {
            absorb(state, changes);
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_change);
    on_change.forget();

    #[derive(serde::Serialize)]
    struct Args {}

    spawn_local(async move {
        // Never resolves while the watch is up. A failure to start is silence,
        // not a banner: a watcher can fail for reasons the user cannot act on,
        // and the workbench works without one.
        let _ =
            ipc::call_streaming::<_, ()>(cmd::files::WATCH, &Args {}, "onChange", &channel).await;
    });
}

/// Act on one batch.
fn absorb(state: AppState, changes: FileChanges) {
    if changes.tree {
        refresh_tree(state);
    }
    for path in changes.changed {
        follow(state, path);
    }
}

/// Bring one open file back in line with the disk, or mark it if we cannot.
///
/// Public because a write rusty made itself must not wait on the watcher to
/// notice it: the watcher is debounced, and a failure to start it is silence
/// by design. A project-wide replace calls this for each file it changed.
pub fn follow(state: AppState, path: String) {
    let active = state
        .editor
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));

    if active.as_deref() == Some(path.as_str()) {
        let dirty = state.editor.document.with_untracked(|d| {
            d.as_ref().is_some_and(|d| {
                !d.read_only && state.editor.draft.with_untracked(|draft| draft != &d.text)
            })
        });
        if dirty {
            mark_stale(state, path);
        } else {
            reload_open(state, path);
        }
        return;
    }

    // Parked tabs are reloaded in place rather than dropped: a tab that
    // vanished from the strip because a file changed would be a tab the user
    // has to find again, and the caret and history it is holding are the
    // point of parking.
    let parked = state.editor.parked.with_untracked(|list| {
        list.iter()
            .find(|e| e.document.path == path)
            .map(|e| !e.document.read_only && e.draft != e.document.text)
    });
    match parked {
        Some(true) => mark_stale(state, path),
        Some(false) => reload_open(state, path),
        // Not open. The tree refresh above, if there was one, is all that is
        // owed — re-reading a file nobody is looking at costs an IPC round
        // trip per file `cargo add` touched.
        None => {}
    }
}

/// Note that the disk moved under an unsaved draft.
///
/// Deliberately not a dialog. A `git checkout` can touch a dozen open files,
/// and twelve modal prompts is a workbench nobody can use; the strip says
/// which tabs are affected and the user decides when to look.
fn mark_stale(state: AppState, path: String) {
    state.editor.stale.update(|list| {
        if !list.contains(&path) {
            list.push(path);
        }
    });
}

/// Forget a staleness marker — the tab and the disk agree again.
pub fn clear_stale(state: AppState, path: &str) {
    state.editor.stale.update(|list| list.retain(|p| p != path));
}

/// Re-read a file this window has open, active or parked.
///
/// Discard the result if the tab went dirty while the read was in flight. The
/// read is asynchronous and typing is not, so without this a keystroke landing
/// during the round trip would be overwritten by an answer about the text as
/// it was before.
fn reload_open(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    let args = Args { path: path.clone() };
    spawn_local(async move {
        let Ok(document) = ipc::call::<_, Document>(cmd::files::OPEN, &args).await else {
            return;
        };
        let active = state
            .editor
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));

        if active.as_deref() == Some(path.as_str()) {
            let dirty = state.editor.draft.with_untracked(|draft| {
                state
                    .editor
                    .document
                    .with_untracked(|d| d.as_ref().is_some_and(|d| draft != &d.text))
            });
            if dirty {
                mark_stale(state, path);
                return;
            }
            adopt_active(state, document);
            return;
        }

        state.editor.parked.update(|list| {
            if let Some(entry) = list.iter_mut().find(|e| e.document.path == path) {
                if entry.draft != entry.document.text {
                    return; // went dirty in flight
                }
                entry.draft = document.text.clone();
                entry.highlighted = document.lines.clone();
                entry.document = document;
                // The caret is kept. A file that grew by a line above the
                // caret puts it somewhere slightly wrong, which is a great
                // deal better than sending it to the top of the file every
                // time a formatter runs elsewhere.
            }
        });
    });
}

/// Replace the on-screen document without disturbing the strip or the history.
fn adopt_active(state: AppState, document: Document) {
    clear_stale(state, &document.path);
    state.editor.draft.set(document.text.clone());
    state.editor.echo_text.set(document.text.clone());
    state.editor.highlighted.set(document.lines.clone());
    let path = document.path.clone();
    let text = document.text.clone();
    state.editor.document.set(Some(document));
    // The server has its own copy of the buffer and no idea the disk moved.
    if state.lsp.status.get_untracked() == crate::state::LspStatus::Ready {
        lsp_changed_doc(path, text);
    }
}
