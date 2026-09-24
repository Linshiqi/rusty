//! Writing a file: Ctrl+S, format-on-save, auto-save, and every draft
//! before something runs.

use super::*;

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
    let args = PathText {
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
            let args = PathText {
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

    let Some(path) = state.active_path_now() else {
        return;
    };
    let args = PathText {
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

    let args = PathText {
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
                    text: t!("misc.rustfmt-skipped", reason = error.message),
                    level: Some(LogLevel::Warn),
                });
            }
        }
        save_file(state);
    });
}
