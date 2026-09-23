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

mod entries;
mod groups;
mod save;
mod window;

pub use entries::*;
pub use groups::*;
pub use save::*;
pub use window::*;

/// Open a file for reading and editing.
///
/// Already on screen: nothing happens — a re-read here would replace an
/// unsaved draft with the disk's older text, which is how editors eat work.
/// Parked: the tab is fronted with its draft intact. New: fetched, and
/// whatever was on screen is parked.
pub fn open_file(state: AppState, path: String) {
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

    let args = PathArg { path };
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
    // Restored on both sides, and the other side has read it already.
    if other_holds(state, &path) && open_view(state, &path) {
        return;
    }

    let args = PathArg { path: path.clone() };
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
    let args = PathArg { path: path.clone() };
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
    let args = PathArg { path };
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

    let args = PathArg { path };
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
