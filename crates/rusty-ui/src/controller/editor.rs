//! The editor: the tree, the tabs, and the document in front of you.
//!
//! Parking is the load-bearing idea here. A tab that leaves the screen keeps
//! its draft, its caret and its history, so coming back to it is coming back
//! to where you were rather than to the top of the file.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_edit::{Document, Entry};
use rusty_embed::{LogLevel, LogLine, LogStream};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{AppState, EditHistory, LspStatus, ParkedEditor},
};

/// Create a file or directory, then show it — the tree refreshes, and a new
/// file opens in the editor, because "New file" means "I want to type in it".
pub fn create_entry(state: AppState, path: String, dir: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        dir: bool,
    }

    if !state.has_project() {
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

/// What shell the terminal will start.
/// The shells the picker can offer. Loaded once; a machine does not grow
/// shells mid-session often enough to poll for.
pub fn load_shell_choices(state: AppState) {
    if state.term.choices.with_untracked(|c| !c.is_empty()) {
        return;
    }
    track(
        state,
        async move { ipc::get::<Vec<rusty_embed::ShellChoice>>(cmd::terminal::SHELLS).await },
        move |choices| state.term.choices.set(choices),
    );
}

pub fn load_shell_info(state: AppState) {
    track(
        state,
        async move { ipc::call::<_, rusty_embed::ShellInfo>(cmd::terminal::SHELL_INFO, &()).await },
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
    if !state.has_project() {
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

    let active = state
        .editor
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
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
        async move { ipc::call::<_, Document>(cmd::files::OPEN, &args).await },
        move |document| show_document(state, document, true),
    );
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
    let active = state
        .editor
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
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
    use wasm_bindgen::JsCast;
    let element = web_sys::window()?
        .document()?
        .get_element_by_id("editor-area")?
        .dyn_into::<web_sys::HtmlTextAreaElement>()
        .ok()?;
    let units = element.selection_start().ok().flatten()? as usize;
    let text = state.editor.draft.get_untracked();
    let mut seen = 0usize;
    let mut line = 0u32;
    let mut col = 0u32;
    for ch in text.chars() {
        if seen >= units {
            break;
        }
        seen += ch.len_utf16();
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    Some((line, col))
}

fn clear_editor_transients(state: AppState) {
    state.editor.completion.set(None);
    state.editor.signature.set(None);
    state.editor.hover.set(None);
    state.editor.semantic.set(None);
    state.editor.actions.set(None);
}

/// Front an already open tab, parking the current one.
pub fn activate_tab(state: AppState, path: String) {
    let active = state
        .editor
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
    if active.as_deref() == Some(path.as_str()) {
        return;
    }
    park_active(state);
    if !front_parked(state, &path) {
        // A strip entry with no parked body should not exist; refusing to
        // guess beats showing a stale document as if it were current.
        state.editor.tabs.update(|tabs| tabs.retain(|t| t != &path));
    }
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
    state.editor.draft.set(entry.draft.clone());
    state.editor.echo_text.set(entry.draft);
    state.editor.highlighted.set(entry.highlighted);
    state.editor.document.set(Some(entry.document));
    if let Some((line, col)) = entry.caret {
        state.editor.reveal.set(Some(rusty_lsp::Location {
            path: path.to_string(),
            line,
            col,
            external: false,
        }));
    }
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
    let active = state
        .editor
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
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
        let confirmed = web_sys::window()
            .map(|w| {
                w.confirm_with_message(&format!(
                    "{path} has unsaved changes.\nClose the tab and discard them?"
                ))
                .unwrap_or(false)
            })
            .unwrap_or(false);
        if !confirmed {
            return;
        }
    }

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
        let fronted = next.is_some_and(|n| front_parked(state, &n));
        if !fronted {
            state.editor.document.set(None);
            state.editor.draft.set(String::new());
            state.editor.echo_text.set(String::new());
            state.editor.highlighted.set(Vec::new());
            state.editor.history.set(EditHistory::default());
        }
    }
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

    let active = state
        .editor
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
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

    let Some(path) = state
        .editor
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()))
    else {
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
