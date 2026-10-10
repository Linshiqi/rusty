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

    if !state.has_project_now() {
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
    } else if changes.changed.iter().any(|path| path.ends_with(".rs")) {
        // Which files the module tree reaches is decided by the `mod` lines
        // *inside* Rust files, so adding one is a content change and the
        // tree never hears about it — the file stayed dim after being
        // declared, which reads as a dim that means nothing. Only for `.rs`,
        // and only when the walk is not happening anyway: the scan reads
        // every Rust file in the project, which is not a thing to do after
        // somebody saves a README.
        refresh_unlinked(state);
    }
    // The history follows the disk too: a commit made in a terminal, a
    // checkout, a fetch. `.git/` itself is a dot directory and unwatched, so
    // this rides on the working-tree changes those actions produce; a
    // refresh is one `git log`, and `refresh_git` is a no-op for a project
    // whose history nobody has opened.
    refresh_git(state);
    for path in changes.changed {
        // A figure redrawn on disk is a stale picture in every page showing
        // it; dropping the cached bytes makes the next look re-read them.
        state.editor.images.update(|images| {
            images.remove(&path);
        });
        follow(state, path);
    }
}

/// Bring one open file back in line with the disk, or mark it if we cannot.
///
/// Public because a write rusty made itself must not wait on the watcher to
/// notice it: the watcher is debounced, and a failure to start it is silence
/// by design. A project-wide replace calls this for each file it changed.
pub fn follow(state: AppState, path: String) {
    // One read however many views the file has: the group it is on screen
    // in, else each group that holds it parked. A view on screen carries the
    // answer to the other side itself (`share_document`), where two reads
    // would each leave a painting of their own for the backend to keep.
    let groups = state.open_groups();
    if let Some(group) = groups
        .iter()
        .find(|group| group.active_path_now().as_deref() == Some(path.as_str()))
    {
        follow_in(*group, path);
        return;
    }
    for group in groups {
        follow_in(group, path.clone());
    }
}

fn follow_in(state: AppState, path: String) {
    let active = state.active_path_now().as_deref() == Some(path.as_str());
    // Parked tabs are reloaded in place rather than dropped: a tab that
    // vanished from the strip because a file changed would be a tab the user
    // has to find again, and the caret and history it is holding are the
    // point of parking.
    let parked = state
        .editor
        .parked
        .with_untracked(|list| list.iter().any(|e| e.document.path == path));
    // Not open: the tree refresh above, if there was one, is all that is
    // owed — re-reading a file nobody is looking at costs an IPC round trip
    // per file `cargo add` touched.
    if active || parked {
        reload_open(state, path);
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

/// Re-read a file this window has open, active or parked, and decide what
/// the disk's copy means for it.
///
/// Read first, decided after. A draft with edits of its own was marked
/// stale the moment the watcher spoke, before anybody looked at the disk —
/// and the watcher speaks for this window's own saves too, so a save
/// followed by more typing put ⚠ beside a tab only rusty had written. Now:
/// the disk holding what the document already says, or what this window
/// last wrote there (`Editor::wrote` — the watcher can overtake a save's
/// answer), is no news; a draft with edits of its own is marked, never
/// replaced; anything else is reloaded. Decided against the state after
/// the read, because the read is asynchronous and typing is not.
fn reload_open(state: AppState, path: String) {
    let args = PathArg { path: path.clone() };
    spawn_local(async move {
        let Ok(document) = ipc::call::<_, Document>(cmd::files::OPEN, &args).await else {
            return;
        };
        let ours = state.editor.wrote(&path, &document.text);

        if state.active_path_now().as_deref() == Some(path.as_str()) {
            let (known, dirty) = state.editor.document.with_untracked(|open| match open {
                Some(open) => (
                    open.text == document.text,
                    !open.read_only && state.editor.draft.with_untracked(|d| *d != open.text),
                ),
                None => (true, false),
            });
            if known {
                return;
            }
            if ours {
                state.editor.document.update(|open| {
                    if let Some(open) = open
                        && open.path == path
                    {
                        open.text = document.text.clone();
                    }
                });
                return;
            }
            if dirty {
                mark_stale(state, path);
                return;
            }
            adopt_active(state, document);
            return;
        }

        let mut dirty = false;
        state.editor.parked.update(|list| {
            if let Some(entry) = list.iter_mut().find(|e| e.document.path == path) {
                if entry.document.text == document.text {
                    return;
                }
                if ours {
                    entry.document.text = document.text.clone();
                    return;
                }
                if !entry.document.read_only && entry.draft != entry.document.text {
                    dirty = true;
                    return;
                }
                entry.draft = document.text.clone();
                entry.highlighted = document.lines.clone();
                entry.paint = crate::state::PaintState {
                    version: document.paint,
                    stale: None,
                };
                entry.document = document;
                // The caret is kept. A file that grew by a line above the
                // caret puts it somewhere slightly wrong, which is a great
                // deal better than sending it to the top of the file every
                // time a formatter runs elsewhere.
            }
        });
        if dirty {
            mark_stale(state, path);
        }
    });
}

/// Replace the on-screen document without disturbing the strip or the history.
fn adopt_active(state: AppState, document: Document) {
    clear_stale(state, &document.path);
    state.editor.draft.set(document.text.clone());
    state.editor.echo_text.set(document.text.clone());
    state.editor.highlighted.set(document.lines.clone());
    super::editor::painted_whole(state, document.paint, None);
    let path = document.path.clone();
    let text = document.text.clone();
    state.editor.document.set(Some(document));
    share_document(state);
    // The server has its own copy of the buffer and no idea the disk moved.
    if state.lsp.status.get_untracked() == crate::state::LspStatus::Ready {
        lsp_changed_doc(state, path, text);
    }
}
