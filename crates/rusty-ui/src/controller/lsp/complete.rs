//! Completion and signature help: asked as the word is typed, and shown
//! only while it is still the word being typed.

use super::*;

/// How long a word already showing waits, after a keystroke, before it is
/// asked about again: a burst of typing is one question, not one per key.
/// The first ask about a word is not delayed.
const REASK_AFTER: Duration = Duration::from_millis(40);

/// How long a failed ask waits before its one retry.
const RETRY_AFTER: Duration = Duration::from_millis(150);

/// Ask what could complete the word that starts at `word_start` on `line`,
/// with the caret at `col` — at once, or after [`REASK_AFTER`] unless a later
/// keystroke asks first.
///
/// The buffer is synced to the server first, without waiting for the pulse:
/// completion after typing `.` is about the text as of *that keystroke*, and a
/// 250ms-stale server answers about the wrong world. `did_change` dedups, so
/// the extra sync costs nothing when the pulse already ran.
///
/// Every ask is numbered and anchored (`state::CompletionAsk`). An answer is
/// shown while its word is still the word being typed and nothing newer is
/// showing — so the first answer for a word appears as soon as it lands,
/// with later asks for the same word still out, and an answer for a word the
/// caret has left is dropped. An ask that fails is made once more if it is
/// still the latest: rust-analyzer cancels a request that a newer edit
/// overtakes (`content modified`), and a word whose one ask was cancelled
/// used to get no completion at all.
pub fn request_completion(
    state: AppState,
    path: String,
    line: u32,
    col: u32,
    word_start: u32,
    now: bool,
) {
    if state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let mut asked = 0;
    state.editor.completion_ask.update_value(|ask| {
        ask.count += 1;
        asked = ask.count;
        ask.anchor = Some((path.clone(), line, word_start));
    });
    let at = Ask { path, line, col };
    if now {
        ask_completion(state, at, word_start, asked, true);
    } else {
        set_timeout(
            move || {
                if latest_ask(state) == Some(asked) {
                    ask_completion(state, at, word_start, asked, true);
                }
            },
            REASK_AFTER,
        );
    }
}

/// Close the popup and forget every answer on its way: the caret has left
/// the word, or the text moved under it.
pub fn dismiss_completion(state: AppState) {
    state.editor.completion_ask.update_value(|ask| {
        ask.count += 1;
        ask.anchor = None;
    });
    if state.editor.completion.with_untracked(Option::is_some) {
        state.editor.completion.set(None);
    }
}

/// The latest ask, while one is in force.
fn latest_ask(state: AppState) -> Option<u64> {
    state
        .editor
        .completion_ask
        .try_with_value(|ask| ask.anchor.as_ref().map(|_| ask.count))
        .flatten()
}

fn ask_completion(state: AppState, at: Ask, word_start: u32, asked: u64, retry: bool) {
    let sync = Sync {
        path: at.path.clone(),
        text: state.editor.draft.get_untracked(),
    };
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::lsp::CHANGE, &sync).await;
        let answer = ipc::call::<_, rusty_lsp::CompletionList>(cmd::lsp::COMPLETE, &at).await;
        let anchor = (at.path.clone(), at.line, word_start);
        let still = state
            .editor
            .completion_ask
            .try_with_value(|ask| ask.anchor.as_ref() == Some(&anchor))
            .unwrap_or(false);
        if !still || state.active_path_now().as_deref() != Some(at.path.as_str()) {
            return;
        }
        match answer {
            Ok(list) => {
                let newer = state
                    .editor
                    .completion
                    .with_untracked(|popup| popup.as_ref().is_some_and(|p| p.asked > asked));
                if newer {
                    return;
                }
                if list.items.is_empty() {
                    // Nothing for this word — which only the newest ask may say.
                    if latest_ask(state) == Some(asked) {
                        state.editor.completion.set(None);
                    }
                    return;
                }
                state
                    .editor
                    .completion
                    .set(Some(crate::state::CompletionPopup {
                        path: at.path,
                        line: at.line,
                        word_start,
                        items: list.items,
                        incomplete: list.incomplete,
                        reply: list.reply,
                        asked,
                    }));
            }
            Err(_) if retry && latest_ask(state) == Some(asked) => {
                set_timeout(
                    move || {
                        if latest_ask(state) == Some(asked) {
                            ask_completion(state, at, word_start, asked, false);
                        }
                    },
                    RETRY_AFTER,
                );
            }
            Err(_) => {}
        }
    });
}

/// Ask what an accepted completion brings with it besides the insertion —
/// the `use` line for an item that was not in scope — and hand the edits
/// to `then`. `reply` names the answer the item was picked from. Nothing
/// arrives for an item that needs none.
pub fn resolve_completion(
    state: AppState,
    path: String,
    reply: u64,
    index: u32,
    then: impl FnOnce(Vec<rusty_lsp::ActionEdit>) + 'static,
) {
    if state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        reply: u64,
        index: u32,
    }
    spawn_local(async move {
        if let Ok(edits) = ipc::call::<_, Vec<rusty_lsp::ActionEdit>>(
            cmd::lsp::RESOLVE_COMPLETION,
            &Args { path, reply, index },
        )
        .await
            && !edits.is_empty()
        {
            then(edits);
        }
    });
}

/// Ask what call the caret sits inside, for the signature card.
///
/// Syncs the draft first, like completion does: an answer about stale text
/// highlights the wrong parameter.
pub fn request_signature(state: AppState, path: String, line: u32, col: u32) {
    if state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let sync = Sync {
        path: path.clone(),
        text: state.editor.draft.get_untracked(),
    };
    let ask = Ask {
        path: path.clone(),
        line,
        col,
    };
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::lsp::CHANGE, &sync).await;
        let answer = ipc::call::<_, Option<rusty_lsp::SignatureInfo>>(cmd::lsp::SIGNATURE, &ask)
            .await
            .ok()
            .flatten();
        let current = state.active_path_now();
        if current.as_deref() == Some(path.as_str()) {
            // None clears: the server saying "no call here" is how the card
            // learns the caret left the parentheses.
            state
                .editor
                .signature
                .set(answer.map(|info| (path, line, info)));
        }
    });
}
