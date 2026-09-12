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
    state::{AppState, EditHistory, LspStatus, ParkedEditor, ParkedViewport},
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

/// Re-read the project tree.
pub fn refresh_tree(state: AppState) {
    if !state.has_project_now() {
        return;
    }
    track(
        state,
        async move { ipc::call::<_, Vec<Entry>>(cmd::files::TREE, &()).await },
        move |entries| state.editor.tree.set(entries),
    );
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
    // One group per file. A path the other group holds is fronted there and
    // focus follows — rather than a second draft of the same file that a
    // save from either side would silently overwrite the other with.
    let other = state.other();
    if other
        .editor
        .tabs
        .with_untracked(|tabs| tabs.iter().any(|t| t == &path))
    {
        state.layout.focus.set(other.group);
        open_file(other, path);
        return;
    }

    let args = Args { path };
    track(
        state,
        async move { ipc::call::<_, Document>(cmd::files::OPEN, &args).await },
        move |document| show_document(state, document, true),
    );
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
fn show_document(state: AppState, document: Document, announce: bool) {
    let active = state.active_path_now();
    if active.is_some() && active.as_deref() != Some(document.path.as_str()) {
        park_active(state);
    }
    if active.as_deref() != Some(document.path.as_str()) {
        state.editor.history.set(EditHistory::default());
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
    if announce && !document.read_only && state.lsp.status.get_untracked() == LspStatus::Ready {
        lsp_open_doc(document.path.clone(), document.text.clone());
        request_semantic(state, document.path.clone());
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
    state.editor.document.set(Some(document));
}

/// Stash the on-screen editor into the parked set, caret and all.
fn park_active(state: AppState) {
    let Some(document) = state.editor.document.get_untracked() else {
        return;
    };
    let entry = ParkedEditor {
        draft: state.editor.draft.get_untracked(),
        highlighted: state.editor.highlighted.get_untracked(),
        caret: active_caret(state),
        history: state.editor.history.get_untracked(),
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
    state.editor.completion.set(None);
    state.editor.signature.set(None);
    state.editor.hover.set(None);
    state.editor.semantic.set(None);
    state.editor.actions.set(None);
    // A viewport still waiting for a view that never mounted belongs to a
    // document that is no longer coming.
    state.editor.viewport.set(None);
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
fn front_parked(state: AppState, path: &str) -> bool {
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
    state.editor.history.set(entry.history);
    // After `clear_editor_transients` has reset them, or the tab comes back
    // flat while its caret comes back where it was.
    state.editor.folds.set(entry.folds);
    state.editor.draft.set(entry.draft.clone());
    state.editor.echo_text.set(entry.draft);
    state.editor.highlighted.set(entry.highlighted);
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
    if dirty {
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
    state.editor.history.set(EditHistory::default());
}

// ─── two groups ─────────────────────────────────────────────────────────────────

/// Open a file in the right-hand group — VS Code's "Open to the Side" —
/// splitting the editor if it is not split yet.
///
/// "Beside" is the right group from *either* side. With two groups there is
/// nothing further right, and the first version sent a file to "the other
/// group": from the right group that moved it left, and when it was the
/// right group's last file the right group vanished under the click — read,
/// correctly, as the split closing for no reason. Files move into the right
/// group and never out of it; the right group closes only when its last tab
/// does. A file the left group holds moves across rather than opening
/// twice (one group per file); one the right group holds is fronted there.
pub fn open_beside(state: AppState, path: String) {
    let first = state.group(crate::state::Group::First);
    let second = state.group(crate::state::Group::Second);
    state.layout.split.set(true);
    if first
        .editor
        .tabs
        .with_untracked(|tabs| tabs.iter().any(|t| t == &path))
    {
        transplant(first, second, &path);
    } else {
        // Already on the right, or open nowhere yet: either way `open_file`
        // on the right group does the right thing.
        state.layout.focus.set(second.group);
        open_file(second, path);
    }
}

/// The left strip's split button and Ctrl+\: the left group's file moves to
/// the right group, opening it if need be. Only the left group has the
/// button — there is nothing further right of the right group — and it
/// wants a second tab to leave behind: a left pane emptied by its only file
/// moving across would close again at once, and the click would look like
/// nothing happened.
pub fn split_active(state: AppState) {
    let first = state.group(crate::state::Group::First);
    let Some(path) = first.active_path_now() else {
        return;
    };
    if first.editor.tabs.with_untracked(|tabs| tabs.len() < 2) {
        return;
    }
    open_beside(first, path);
}

/// Carry an open file from one group to the other with its draft, caret and
/// history. The strip it leaves shows its neighbour, as a close would.
fn transplant(from: AppState, to: AppState, path: &str) {
    let was_active = from.active_path_now().as_deref() == Some(path);
    let next = neighbour_after_close(&from.editor.tabs.get_untracked(), path);
    if was_active {
        park_active(from);
    }
    let mut entry = None;
    from.editor.parked.update(|list| {
        if let Some(at) = list.iter().position(|e| e.document.path == path) {
            entry = Some(list.remove(at));
        }
    });
    from.editor.tabs.update(|tabs| tabs.retain(|t| t != path));
    if was_active {
        clear_editor_transients(from);
        if !next.is_some_and(|n| front_parked(from, &n)) {
            clear_screen(from);
        }
    }
    to.layout.focus.set(to.group);
    match entry {
        Some(entry) => {
            to.editor.tabs.update(|tabs| {
                if !tabs.iter().any(|t| t == path) {
                    tabs.push(path.to_string());
                }
            });
            to.editor.parked.update(|list| {
                list.retain(|e| e.document.path != path);
                list.push(entry);
            });
            activate_tab(to, path.to_string());
        }
        // Listed but never loaded — a tab restored from last session and not
        // clicked since. Nothing to carry; the other side reads it fresh.
        None => open_file(to, path.to_string()),
    }
    settle_groups(from);
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
    let is_rust = document.language.as_deref() == Some("rust") || document.path.ends_with(".rs");
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
    use super::neighbour_after_close;

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
}
