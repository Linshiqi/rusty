//! Writing a file: Ctrl+S, format-on-save, auto-save, and every draft
//! before something runs.

use super::*;

/// Write the draft back without being asked, a beat after typing stopped.
///
/// Not [`format_then_save`]: rustfmt under the fingers rewrites the line
/// being typed, and mid-expression it cannot parse at all. So: write, and
/// move the *document* forward to exactly the bytes written — the same
/// write Ctrl+S makes (`write_draft`), less the write of a draft the disk
/// already holds. The dirty dot is `draft != document.text`, so it clears
/// by itself and stays honest — it lights again the moment the next key is
/// pressed. The draft is never touched.
pub fn autosave_file(state: AppState) {
    write_draft(state, false);
}

/// Write the draft to disk and move the document forward to exactly the
/// bytes written — what both a save and an auto-save do. `always` is a save
/// somebody asked for, which writes even a draft the disk already holds.
fn write_draft(state: AppState, always: bool) {
    let Some(document) = state.editor.document.with_untracked(Clone::clone) else {
        return;
    };
    // A dependency's source is not this project's to change; the backend
    // would refuse the path anyway, but a red banner for pressing Ctrl+S in
    // a file that *looks* editable would blame the user for our affordance.
    if document.read_only {
        return;
    }
    let path = document.path.clone();
    let text = state.editor.draft.get_untracked();
    if !always && text == document.text {
        return;
    }
    state.editor.note_written(&path, &text);
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
    for (path, text) in &writes {
        state.editor.note_written(path, text);
    }
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

/// Write the current draft back — Ctrl+S.
///
/// Written the way auto-save writes, not written and then read back. The
/// read-back put the disk's copy into `draft`, and every key pressed
/// during the two round trips went with it; on a machine busy with `cargo
/// check` those trips are slow enough to type through, and a save that
/// lost the last few keys, with the dirty dot still lit, read as a save
/// that did not happen. It also announced the open file to rust-analyzer
/// again and asked for its colours and hints afresh, on every save.
pub fn save_file(state: AppState) {
    write_draft(state, true);
}

/// How long a save waits for rustfmt, and no longer — VS Code's rule for
/// its own format-on-save. rustfmt answers in a tenth of that; a machine
/// busy with `cargo check` can make it wait, and the save goes ahead
/// unformatted rather than wait with it.
const FORMAT_BUDGET: std::time::Duration = std::time::Duration::from_millis(1000);

/// Format with rustfmt, then save.
///
/// The save never waits on the format for long ([`FORMAT_BUDGET`]), and a
/// format is used only when it is of the text still on screen: rustfmt
/// answering about a draft that has been typed on since would put back the
/// text from before the typing. A file that does not parse comes back
/// unformatted without a word (`rusty_edit::format_rust`); a real failure —
/// no rustfmt, a bad `rustfmt.toml` — goes to the dock, and the save still
/// happens. `apply` is the editor's own hand: it re-echoes the text and puts
/// the caret back, because the DOM element lives with the view, not here.
pub fn format_then_save(
    state: AppState,
    caret: Option<(u32, u32)>,
    apply: impl Fn(&str, Option<(u32, u32)>) + 'static,
) {
    use std::{cell::Cell, rc::Rc};

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

    let path = document.path;
    let sent = state.editor.draft.get_untracked();
    // Whichever comes first, the answer or the budget, saves; the other
    // then does nothing.
    let settled = Rc::new(Cell::new(false));
    let still_here = move |path: &str| state.active_path_now().as_deref() == Some(path);
    {
        let (settled, path) = (Rc::clone(&settled), path.clone());
        set_timeout(
            move || {
                if !settled.replace(true) && still_here(&path) {
                    save_file(state);
                }
            },
            FORMAT_BUDGET,
        );
    }
    let args = PathText {
        path: path.clone(),
        text: sent.clone(),
    };
    spawn_local(async move {
        let answer = ipc::call::<_, rusty_edit::Formatted>(cmd::files::FORMAT, &args).await;
        if settled.replace(true) || !still_here(&path) {
            return;
        }
        match answer {
            Ok(formatted)
                if formatted.changed && state.editor.draft.with_untracked(|d| *d == sent) =>
            {
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
