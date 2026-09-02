//! Where the caret has been, and finding where it should go next.
//!
//! The tab strip, project search, rename, and the jump list — everything that
//! answers "take me somewhere" rather than "change something".

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_embed::{LogLine, LogStream};
use rusty_i18n::t;

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

/// localStorage key for a project's open tabs. Per the storage doctrine this
/// is WebView-only state whose loss costs a shrug — exactly localStorage's
/// province.
fn tabs_key(root: &str) -> String {
    format!("rusty.tabs.{root}")
}

/// Write the strip to localStorage: open paths, active one first.
pub fn remember_tabs(state: AppState) {
    // Nor to overwrite: one detached window remembering its single tab
    // would wipe the shell's strip for the next launch.
    if state.app.detached.with_untracked(Option::is_some) {
        return;
    }
    let Some(root) = state
        .project
        .detected
        .with_untracked(|p| p.as_ref().map(|p| p.root.clone()))
    else {
        return;
    };
    let active = state.active_path_now();
    let tabs = state.editor.tabs.get_untracked();
    #[derive(serde::Serialize)]
    struct Args {
        root: String,
        tabs: Vec<String>,
        active: Option<String>,
    }
    let args = Args { root, tabs, active };
    // Fire and forget: a tab strip that failed to save is not worth a banner
    // over the edit the user was making when it happened.
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::workbench::RECORD_TABS, &args).await;
    });
}

/// Reopen the tabs the project had last time. Missing files fail their open
/// quietly through the normal error path; the strip simply ends up shorter.
pub fn restore_tabs(state: AppState, root: &str) {
    // A detached window edits one file; the shell's saved strip is not its
    // business to reopen.
    if state.app.detached.with_untracked(Option::is_some) {
        return;
    }
    // Anything this window still has from before the strip became a file.
    // Read once, handed over, and deleted — so an upgrade does not cost
    // somebody the tabs they had open, and the key does not linger to be
    // read again by a later version that no longer understands it.
    let carried = crate::state::local_take(&tabs_key(root))
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());

    #[derive(serde::Serialize)]
    struct Args {
        root: String,
    }
    let args = Args {
        root: root.to_string(),
    };
    spawn_local(async move {
        let stored =
            ipc::call::<_, Option<rusty_embed::ProjectTabs>>(cmd::workbench::PROJECT_TABS, &args)
                .await
                .ok()
                .flatten();

        let (tabs, active) = match (stored, carried) {
            // The file wins where it has an answer: it is the one that
            // survives a reinstall, and a stale WebView copy would undo the
            // last session every time.
            (Some(stored), _) => (stored.tabs, stored.active),
            (None, Some(old)) => (
                old["tabs"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|v| v.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                old["active"].as_str().map(str::to_string),
            ),
            (None, None) => return,
        };

        // The strip comes back whole; only one file is read.
        //
        // Opening all of them was a round trip and a `didOpen` per tab, so a
        // session with fifteen files reopened cost fifteen reads and fifteen
        // notifications to rust-analyzer before the window was usable — and
        // fourteen of those documents were never looked at. VSCode restores
        // the strip and loads on click; `open_file` already fetches a path
        // that is neither active nor parked, so a tab listed here and nowhere
        // else *is* the lazy one, with no new state to hold it.
        //
        // The active one is opened rather than listed, because a strip with
        // nothing on screen behind it is a window that looks like it failed.
        state.editor.tabs.set(tabs.clone());
        let active = active.filter(|path| tabs.iter().any(|t| t == path));
        if let Some(active) = active.or_else(|| tabs.first().cloned()) {
            open_file(state, active);
        }
    });
}

// ─── project search ─────────────────────────────────────────────────────────────

/// Debounced: called on every keystroke in the search box, runs the search
/// only when the typing pauses.
pub fn schedule_search(state: AppState) {
    let generation = state.search.generation.get_untracked() + 1;
    state.search.generation.set(generation);
    set_timeout(
        move || {
            if state.search.generation.get_untracked() == generation {
                run_search(state, generation);
            }
        },
        std::time::Duration::from_millis(250),
    );
}

fn run_search(state: AppState, generation: u64) {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        query: String,
        case_sensitive: bool,
        whole_word: bool,
        regex: bool,
        include: String,
        exclude: String,
    }

    let query = state.search.query.get_untracked();
    if query.trim().is_empty() {
        state.search.results.set(None);
        return;
    }
    let args = Args {
        query,
        case_sensitive: state.search.case.get_untracked(),
        whole_word: state.search.word.get_untracked(),
        regex: state.search.regex.get_untracked(),
        include: state.search.include.get_untracked(),
        exclude: state.search.exclude.get_untracked(),
    };
    spawn_local(async move {
        let result = ipc::call::<_, rusty_edit::SearchResults>(cmd::files::SEARCH, &args).await;
        // Typed since? This answer is about a query nobody is asking anymore.
        if state.search.generation.get_untracked() != generation {
            return;
        }
        match result {
            Ok(results) => state.search.results.set(Some(results)),
            Err(_) => state.search.results.set(None),
        }
    });
}

/// Rewrite every match, and keep what it refused.
///
/// **Not undoable from inside the window**, which decides the shape: the
/// panel has already listed exactly what will change, the open drafts are
/// named and skipped rather than written over, and the outcome stays on
/// screen afterwards because the skipped half is the part somebody has to act
/// on. The tree and the open editors are re-read, since the disk just moved
/// under both.
pub fn replace_all(state: AppState) {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        query: String,
        replacement: String,
        case_sensitive: bool,
        whole_word: bool,
        regex: bool,
        include: String,
        exclude: String,
        drafts: Vec<String>,
    }

    let query = state.search.query.get_untracked();
    if query.trim().is_empty() {
        return;
    }
    // Nested under `args` because the command takes one struct: nine loose
    // parameters is past what clippy allows, and past what anybody can read.
    #[derive(serde::Serialize)]
    struct Outer {
        args: Args,
    }
    let args = Outer {
        args: Args {
            query,
            replacement: state.search.replacement.get_untracked(),
            case_sensitive: state.search.case.get_untracked(),
            whole_word: state.search.word.get_untracked(),
            regex: state.search.regex.get_untracked(),
            include: state.search.include.get_untracked(),
            exclude: state.search.exclude.get_untracked(),
            drafts: state.dirty_paths(),
        },
    };
    track(
        state,
        async move { ipc::call::<_, rusty_edit::ReplaceOutcome>(cmd::files::REPLACE, &args).await },
        move |outcome| {
            // Each file rusty just rewrote, through the same path the watcher
            // uses — an unsaved draft is marked rather than replaced, and a
            // clean editor is re-read. Not left to the watcher itself: it is
            // debounced, and a watcher that failed to start is silent.
            for path in &outcome.changed {
                super::follow(state, path.clone());
            }
            let changed = !outcome.changed.is_empty();
            state.search.outcome.set(Some(outcome));
            if changed {
                // The results are about a project that no longer says that.
                run_search(state, state.search.generation.get_untracked());
            }
        },
    );
}

/// Rename the symbol at the caret, everywhere it is used.
///
/// Saves first, deliberately: the edits land on disk, and an unsaved buffer
/// would overwrite them with its own stale bytes the next time Ctrl+S was
/// pressed. Reloads afterwards so the window shows what is now on disk
/// rather than what it remembers.
pub fn rename_symbol(state: AppState, path: String, line: u32, col: u32, new_name: String) {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        path: String,
        line: u32,
        col: u32,
        new_name: String,
    }

    save_file(state);
    let reopen = path.clone();
    let args = Args {
        path,
        line,
        col,
        new_name,
    };
    track(
        state,
        async move { ipc::call::<_, Vec<String>>(cmd::lsp::RENAME, &args).await },
        move |changed| {
            // Say how far it reached. A rename that touched eleven files and
            // reported nothing leaves you wondering whether it worked.
            state.push_log(LogLine {
                stream: LogStream::Stdout,
                text: match changed.len() {
                    0 => t!("misc.rename-nothing"),
                    n => t!("misc.renamed-in", count = n.to_string()),
                },
                level: None,
            });
            if !changed.is_empty() {
                open_file(state, reopen.clone());
            }
        },
    );
}

/// The editor's textarea, when there is one.
pub(super) fn editor_element() -> Option<web_sys::HtmlElement> {
    use wasm_bindgen::JsCast;
    web_sys::window()?
        .document()?
        .get_element_by_id("editor-area")?
        .dyn_into::<web_sys::HtmlElement>()
        .ok()
}

/// Where the caret is, for the navigation history to remember.
///
/// Read off the DOM rather than tracked in a signal: the caret moves on
/// every keystroke and every click, and a signal updated that often would
/// re-run half the editor's reactivity to record something only jumps ever
/// read.
fn here(state: AppState) -> Option<crate::state::NavPoint> {
    let path = state.active_path_now()?;
    let (line, col) = caret_position(state)?;
    Some(crate::state::NavPoint { path, line, col })
}

/// Where the caret is right now, as `(line, scalar column)`.
///
/// Read off the DOM, in UTF-16 units as a textarea reports it, and converted
/// through `rusty_lsp::positions` — the one implementation of that
/// arithmetic. Two controllers carried their own loops for this, and the two
/// counted the newline in a different order.
pub(super) fn caret_position(state: AppState) -> Option<(u32, u32)> {
    use rusty_lsp::positions::{self, Encoding::Utf16};
    use wasm_bindgen::JsCast;

    let element = editor_element()?
        .dyn_into::<web_sys::HtmlTextAreaElement>()
        .ok()?;
    let units = element.selection_start().ok().flatten()? as usize;
    let text = state.editor.draft.get_untracked();
    let byte = positions::byte_of_character(&text, units, Utf16);
    let (line, character) = positions::offset_to_position(&text, byte, Utf16);
    let line_text = text.split('\n').nth(line as usize).unwrap_or("");
    Some((
        line,
        positions::character_to_scalar(line_text, character, Utf16),
    ))
}

/// Note that we are leaving `here` for somewhere else.
///
/// Called by every jump the editor offers — a definition, a search hit, a
/// problem row — so Back means the same thing whichever one you used. Vim's
/// `Ctrl+O` and the menu read the one list.
pub(super) fn remember_jump(state: AppState, to: &rusty_lsp::Location) {
    let Some(from) = here(state) else {
        return;
    };
    let to = crate::state::NavPoint {
        path: to.path.clone(),
        line: to.line,
        col: to.col,
    };
    state.editor.nav.update(|nav| nav.jump(from, to));
}

/// Back and forward through the positions the caret has visited.
///
/// The navigation every editor has and this one did not: jumping to a
/// definition had no way home at all, in Vim mode or out of it, which on a
/// HAL that wraps four layers deep meant finding the file again by hand.
pub fn nav_back(state: AppState) {
    if let Some(point) = state.editor.nav.try_update(|nav| nav.back()).flatten() {
        travel(state, point);
    }
}

pub fn nav_forward(state: AppState) {
    if let Some(point) = state.editor.nav.try_update(|nav| nav.forward()).flatten() {
        travel(state, point);
    }
}

/// Go to a remembered position without recording it — walking the history is
/// not itself a jump, or Back would never reach the beginning.
fn travel(state: AppState, point: crate::state::NavPoint) {
    let current = state.active_path_now();
    if current.as_deref() != Some(point.path.as_str()) {
        open_file(state, point.path.clone());
    }
    state.editor.reveal.set(Some(rusty_lsp::Location {
        path: point.path,
        line: point.line,
        col: point.col,
        external: false,
    }));
}

/// Open a file at an exact position — how a search hit or a problem row
/// lands in the editor, through the same reveal goto-definition uses.
pub fn open_at(state: AppState, path: String, line: u32, col: u32) {
    // Files and Search both keep an editor on the right, VSCode-style; a
    // jump from either stays put. From anywhere else, land in Files.
    let panel = state.layout.panel.get_untracked();
    if panel != "files" && panel != "search" {
        state.layout.panel.set("files".to_string());
    }
    let current = state.active_path_now();
    if current.as_deref() != Some(path.as_str()) {
        open_file(state, path.clone());
    }
    let target = rusty_lsp::Location {
        path,
        line,
        col,
        external: false,
    };
    remember_jump(state, &target);
    state.editor.reveal.set(Some(target));
}
