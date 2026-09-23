//! What an edit sets off once typing pauses: the sync with the server,
//! the repaint, and auto-save.

use super::*;

/// How long after the last keystroke the file is written, when auto-save is
/// on. VS Code's own default for `files.autoSave: afterDelay`, and four
/// times the highlight pulse: a write goes to the disk and through the
/// watcher, where a re-highlight only comes back to this window.
const AUTOSAVE_AFTER: Duration = Duration::from_millis(1000);

/// The debounced follow-up to typing: re-highlight the draft and tell the
/// server what it says now.
///
/// Scheduled rather than immediate — each is a round trip, and per keystroke
/// that would re-highlight every letter of a word nobody finished typing.
///
/// Auto-save rides the same call because this is the one hook every edit
/// path already goes through — a keystroke, a paste, an undo, a completion
/// accepted, a quick fix applied — and a second list of edit sites would be
/// a list that drifts from this one.
pub fn schedule_pulse(state: AppState) {
    // The file open on the other side too is the same document: the edit
    // reaches it now, not when typing pauses (`views.rs`).
    share_edit(state);
    // Marks of where a name occurs are about the text before the edit, and
    // wash whatever moved under them; they come back when the caret rests.
    if state.editor.occurrences.with_untracked(Option::is_some) {
        state.editor.occurrences.set(None);
    }
    let generation = state.editor.pulse_gen.get_untracked() + 1;
    state.editor.pulse_gen.set(generation);
    set_timeout(
        move || {
            if state.editor.pulse_gen.get_untracked() == generation {
                edit_pulse(state);
            }
        },
        std::time::Duration::from_millis(250),
    );
    schedule_autosave(state);
}

/// Write the file a beat after typing stops, when the setting is on.
///
/// Its own counter, not the pulse's: the highlight fires four times as
/// often, and a write that rode it would go out mid-word. Keyed on the
/// *path* as well, because the timer outlives the tab — switching files
/// inside the second would otherwise save the new file's draft under a
/// number the old file's typing set.
fn schedule_autosave(state: AppState) {
    if !state.editor.auto_save.get_untracked() {
        return;
    }
    let Some(path) = state.active_path_now() else {
        return;
    };
    let generation = state.editor.save_gen.get_untracked() + 1;
    state.editor.save_gen.set(generation);
    set_timeout(
        move || {
            if state.editor.save_gen.try_get_untracked() != Some(generation) {
                return;
            }
            if state.active_path_now().as_deref() == Some(path.as_str()) {
                autosave_file(state);
            }
        },
        AUTOSAVE_AFTER,
    );
}

fn edit_pulse(state: AppState) {
    let Some(path) = state.active_path_now() else {
        return;
    };
    let text = state.editor.draft.get_untracked();

    if path.ends_with(".rs") && state.lsp.status.get_untracked() == LspStatus::Ready {
        // Each view of the file asks for its own colours: a long file's are
        // asked for around the lines that view is drawing.
        for group in state.open_groups() {
            if group.active_path_now().as_deref() == Some(path.as_str()) {
                request_semantic(group, path.clone());
            }
        }
        lsp_sync(
            cmd::lsp::CHANGE,
            PathText {
                path: path.clone(),
                text: text.clone(),
            },
        );
        // After the change, so the server answers about this text.
        for group in state.open_groups() {
            if group.active_path_now().as_deref() == Some(path.as_str()) {
                request_hints(group, path.clone());
            }
        }
    }

    repaint(state, path);
}

/// Ask for the lines the edits since the last repaint changed, painted, and
/// put them on screen where those lines now are.
///
/// One ask at a time per group, because each answer is the painting the next
/// is measured against; edits while one is out ask again when it lands. An
/// answer is placed even when typing went on — `paint::place` moves each line
/// to where it is now and leaves the ones edited since plain — because
/// dropping it would leave the backend a painting ahead of the screen, and
/// the next repaint would be the whole file.
pub fn repaint(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
        base: Option<u32>,
        stale: Option<(u32, u32)>,
    }

    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let mut busy = false;
    state.editor.painting.update_value(|ask| {
        if let Some(ask) = ask {
            ask.again = true;
            busy = true;
        }
    });
    if busy {
        return;
    }
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let text = state.editor.echo_text.get_untracked();
    let paint = state.editor.paint.get_value();
    state.editor.painting.set_value(Some(PaintAsk {
        serial,
        path: path.clone(),
        sent: text.clone(),
        since: None,
        again: false,
    }));
    let args = Args {
        path,
        text,
        base: paint.version,
        stale: paint.stale.map(|(from, to)| (from as u32, to as u32)),
    };
    spawn_local(async move {
        let answer = ipc::call::<_, rusty_edit::Repaint>(cmd::files::REPAINT, &args).await;
        let mut landed = None;
        state.editor.painting.update_value(|slot| {
            if slot.as_ref().is_some_and(|ask| ask.serial == serial) {
                landed = slot.take();
            }
        });
        // A whole painting went on screen while this was out, and dropped it.
        let Some(ask) = landed else {
            return;
        };
        let active = state.active_path_now();
        let again = ask.again;
        if let Ok(answer) = answer
            && active.as_deref() == Some(ask.path.as_str())
        {
            let now = state.editor.echo_text.get_untracked();
            // The other view of the file, if there is one, takes the same
            // lines: it has had the same edits.
            let shared = other_holds(state, &ask.path).then(|| answer.lines.clone());
            let mut placed = false;
            state.editor.highlighted.update(|lines| {
                placed =
                    crate::paint::place(lines, &ask.sent, &now, answer.from as usize, answer.lines);
            });
            if placed {
                let paint = PaintState {
                    version: Some(answer.version),
                    stale: ask.since,
                };
                state.editor.paint.set_value(paint);
                if let Some(lines) = shared {
                    share_repaint(
                        state,
                        &ask.path,
                        &ask.sent,
                        answer.from as usize,
                        lines,
                        paint,
                    );
                }
            } else {
                // The lines are not the text line for line, which no edit
                // should allow: start again from plain text, all of it stale,
                // so the next answer is the whole file and fits.
                let plain = crate::paint::all_plain(&now);
                let count = plain.len();
                state.editor.highlighted.set(plain);
                state.editor.paint.set_value(PaintState {
                    version: None,
                    stale: Some((0, count)),
                });
                if let Some(path) = active {
                    repaint(state, path);
                }
                return;
            }
        }
        // Failed, or its file was parked or closed while it was out: the
        // number on screen is left as it was, and if this moved the backend
        // past it the next ask is answered whole.
        if again && let Some(path) = state.active_path_now() {
            repaint(state, path);
        }
    });
}
