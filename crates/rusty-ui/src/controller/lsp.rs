//! Asking rust-analyzer, and absorbing what it says back.
//!
//! Requests are fired without waiting for the debounced sync: a completion
//! that arrives after the keystroke it was for is a completion nobody wanted.

use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_embed::{LogLevel, LogLine, LogStream};
use rusty_i18n::t;
use rusty_lsp::{HoverInfo, LspEvent};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{AppState, HoverCard, LspStatus, PaintAsk, PaintState},
};

/// The buffer as the server should now see it. Sent ahead of every request
/// that reads the caret, so the answer is about this keystroke's text.
#[derive(serde::Serialize)]
struct Sync {
    path: String,
    text: String,
}

/// A position-anchored request: completion, signature help, code actions.
/// One shape, defined once — it was declared inside each of the three
/// functions that use it.
#[derive(serde::Serialize)]
struct Ask {
    path: String,
    line: u32,
    col: u32,
}

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

/// Ask what quick fixes exist at the caret, after syncing the draft — an
/// answer about stale text splices into the wrong place.
pub fn request_actions(state: AppState, path: String, line: u32, col: u32) {
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
        let Ok(answer) = ipc::call::<_, rusty_lsp::CodeActions>(cmd::lsp::ACTIONS, &ask).await
        else {
            return;
        };
        let current = state.active_path_now();
        if current.as_deref() == Some(path.as_str()) {
            if answer.fixes.is_empty() {
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: t!("misc.no-quick-fixes"),
                    level: None,
                });
            } else {
                state.editor.actions.set(Some((path, line, answer)));
            }
        }
    });
}

/// Write the part of an accepted quick fix that lands in other files — the
/// `mod` line a fix for an undeclared file puts in its parent module. Those
/// edits land on disk, so a file among them with an unsaved draft refuses
/// the whole fix, by name: the next Ctrl+S there would overwrite them with
/// the draft's stale bytes. Says what changed, as a rename does.
pub fn apply_action_elsewhere(
    state: AppState,
    path: String,
    reply: u64,
    index: u32,
    files: Vec<String>,
) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        reply: u64,
        index: u32,
    }
    if let Some(dirty) = files.iter().find(|file| state.is_dirty(file)) {
        state.push_log(LogLine {
            stream: LogStream::Stderr,
            text: t!("misc.save-first", path = dirty.clone()),
            level: Some(LogLevel::Warn),
        });
        return;
    }
    track(
        state,
        async move {
            ipc::call::<_, Vec<String>>(cmd::lsp::APPLY_ACTION, &Args { path, reply, index }).await
        },
        move |changed| {
            state.push_log(LogLine {
                stream: LogStream::Stdout,
                text: t!("misc.edited-elsewhere", count = changed.len().to_string()),
                level: None,
            });
        },
    );
}

/// Ask for the document's semantic colouring, and keep it only if the answer
/// still describes what is on screen.
pub fn request_semantic(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        lines: Option<(u32, u32)>,
    }

    if !path.ends_with(".rs") || state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let count = state.editor.highlighted.with_untracked(Vec::len) as u32;
    let lines = (count > SEMANTIC_WHOLE_LINES).then(|| {
        let (from, to) = state.editor.drawn_lines.get_value();
        (
            from.saturating_sub(SEMANTIC_MARGIN),
            to.saturating_add(SEMANTIC_MARGIN).min(count),
        )
    });
    let args = Args {
        path: path.clone(),
        lines,
    };
    spawn_local(async move {
        // Errors and empties are the warm-up talking; the lexical base colour
        // stays up either way, so there is nothing to report.
        let Ok(mut spans) =
            ipc::call::<_, Vec<rusty_lsp::SemanticSpan>>(cmd::lsp::SEMANTIC, &args).await
        else {
            return;
        };
        // In order, which the echo finds a line's spans by halving.
        spans.sort_by_key(|span| (span.line, span.start_col));
        let current = state.active_path_now();
        if current.as_deref() == Some(path.as_str()) && !spans.is_empty() {
            state.editor.semantic.set(Some((path, spans)));
            state.editor.semantic_lines.set_value(lines);
        }
    });
}

/// Where the name at this position occurs in its file, for the editor to
/// mark. Only the latest ask is answered: the caret has moved on from the
/// others.
pub fn request_highlights(state: AppState, path: String, line: u32, col: u32) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        line: u32,
        col: u32,
    }

    static ASKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    if !path.ends_with(".rs") || state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let asked = ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let args = Args {
        path: path.clone(),
        line,
        col,
    };
    spawn_local(async move {
        let answer = ipc::call::<_, Vec<rusty_lsp::EditRange>>(cmd::lsp::HIGHLIGHTS, &args).await;
        if ASKED.load(std::sync::atomic::Ordering::Relaxed) != asked
            || state.active_path_now().as_deref() != Some(path.as_str())
        {
            return;
        }
        let ranges = answer.unwrap_or_default();
        let _ = state
            .editor
            .occurrences
            .try_set((!ranges.is_empty()).then_some((path, ranges)));
    });
}

/// Files longer than this are asked for their semantic colours around the
/// lines on screen rather than whole.
const SEMANTIC_WHOLE_LINES: u32 = 3_000;

/// How many lines either side of the drawn ones such a request covers, so a
/// scroll of a screen or two stays inside the answer.
const SEMANTIC_MARGIN: u32 = 400;

/// Whether the semantic colours on hand cover document lines `from..to` —
/// always, for a file short enough to be asked about whole.
pub fn semantic_covers(state: AppState, from: u32, to: u32) -> bool {
    let count = state.editor.highlighted.with_untracked(Vec::len) as u32;
    count <= SEMANTIC_WHOLE_LINES
        || state
            .editor
            .semantic_lines
            .get_value()
            .is_some_and(|(first, end)| first <= from && to <= end)
}

// ─── the language server ─────────────────────────────────────────────────────

/// Start rust-analyzer for the open project and route what it says into state.
pub fn start_lsp(state: AppState) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    if !state.has_project_now() {
        return;
    }
    // A stale channel keeps sending after a restart; the session number is how
    // its events are told apart from the live one.
    let session = state.lsp.session.get_untracked() + 1;
    state.lsp.session.set(session);
    state.lsp.status.set(LspStatus::Starting);
    state.lsp.progress.set(None);

    let channel = ipc::Channel::new();
    let on_event = Closure::wrap(Box::new(move |value: JsValue| {
        if state.lsp.session.get_untracked() != session {
            return;
        }
        if let Ok(event) = serde_wasm_bindgen::from_value::<LspEvent>(value) {
            apply_lsp_event(state, event);
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_event);
    on_event.forget();

    #[derive(serde::Serialize)]
    struct Args {}

    spawn_local(async move {
        let _ = ipc::call_streaming::<_, ()>(cmd::lsp::START, &Args {}, "onEvent", &channel).await;
        // The stream ended: the server exited or was replaced. Only the owner
        // of the current session gets to say so.
        if state.lsp.session.get_untracked() == session
            && state.lsp.status.get_untracked() == LspStatus::Ready
        {
            state.lsp.status.set(LspStatus::Off);
        }
    });
}

fn apply_lsp_event(state: AppState, event: LspEvent) {
    match event {
        LspEvent::Ready {} => {
            state.lsp.status.set(LspStatus::Ready);
            // A new session says nothing about itself until it does; the
            // last one's verdict is not this one's.
            state.lsp.health.set(None);
            // A file opened before the server came up was never announced —
            // in either group.
            for group in state.open_groups() {
                if let Some(path) = group.active_path_now() {
                    lsp_open_doc(path.clone(), group.editor.draft.get_untracked());
                    request_semantic(group, path);
                }
            }
        }
        LspEvent::Unavailable { message, install } => {
            state.lsp.status.set(LspStatus::Missing);
            state.push_log(LogLine {
                stream: LogStream::Stderr,
                text: message,
                level: Some(LogLevel::Warn),
            });
            if let Some(install) = install {
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: format!("$ {install}"),
                    level: None,
                });
            }
        }
        LspEvent::Diagnostics { path, items } => {
            state.lsp.diagnostics.update(|by_file| {
                if items.is_empty() {
                    by_file.remove(&path);
                } else {
                    by_file.insert(path, items);
                }
            });
        }
        LspEvent::Progress { text } => state.lsp.progress.set(text),
        // The difference between "nothing completes here" and "no crate at
        // all": kept in the status bar until the server says otherwise, and
        // said once in the dock, where the reason can be read. Once, not per
        // notification — rust-analyzer repeats its state on every change.
        LspEvent::Health { level, message } => {
            let next = (level != rusty_lsp::HealthLevel::Ok).then_some((level, message));
            if state.lsp.health.with_untracked(|now| *now != next) {
                if let Some((_, Some(text))) = &next {
                    state.push_log(LogLine {
                        stream: LogStream::Stderr,
                        text: format!("rust-analyzer: {text}"),
                        level: Some(LogLevel::Warn),
                    });
                }
                state.lsp.health.set(next);
            }
        }
        LspEvent::Exited {} => {
            state.lsp.progress.set(None);
            state.lsp.health.set(None);
            if state.lsp.status.get_untracked() == LspStatus::Ready {
                state.lsp.status.set(LspStatus::Off);
            }
        }
    }
}

/// Fire-and-forget document sync. Failures are dropped, not bannered: the
/// editor works without a server, and every keystroke would otherwise be a
/// chance to cry wolf.
fn lsp_sync(command: &'static str, args: impl serde::Serialize + 'static) {
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(command, &args).await;
    });
}

pub fn lsp_open_doc(path: String, text: String) {
    // rust-analyzer is only ever told about Rust. Announcing `.git/info/
    // exclude` as a document got every line a "Syntax Error: expected an
    // item" — sixty-eight problems from a file that was never code.
    if !path.ends_with(".rs") {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }
    lsp_sync(cmd::lsp::OPEN, Args { path, text });
}

/// Tell the server the buffer was replaced from outside the editor.
///
/// The watcher's path: rust-analyzer holds its own copy of every open
/// document and has no idea the disk moved, so a file reloaded underneath it
/// leaves the server answering about the previous text — completions at
/// offsets that no longer exist, diagnostics on lines that are gone.
pub(super) fn lsp_changed_doc(path: String, text: String) {
    if !path.ends_with(".rs") {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }
    lsp_sync(cmd::lsp::CHANGE, Args { path, text });
}

pub(super) fn lsp_saved_doc(path: String) {
    if !path.ends_with(".rs") {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }
    lsp_sync(cmd::lsp::SAVED, Args { path });
}

/// Tell the server the editor no longer holds this file — a tab closed, or
/// a file moved out from under its old name — so it reads the disk for it
/// again (`LspClient::did_close`). The client ignores a file it was never
/// told about, so this needs no bookkeeping of what was announced.
pub(super) fn lsp_closed_doc(path: String) {
    if !path.ends_with(".rs") {
        return;
    }
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }
    lsp_sync(cmd::lsp::CLOSE, Args { path });
}

/// Ask what the thing at this position is, for the tooltip.
///
/// Silent on failure and on `None`: hover is ambient, and a banner about a
/// hover would be absurd. The reply is dropped if the user has moved to
/// another file by the time it lands.
pub fn request_hover(state: AppState, path: String, line: u32, col: u32) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        line: u32,
        col: u32,
    }

    let args = Args {
        path: path.clone(),
        line,
        col,
    };

    // What is wrong here, if anything — read before the server is asked and
    // shown above whatever it says. Over a squiggle the error *is* the
    // question; the type of an expression that does not compile is a footnote
    // to it, and showing only the type reads as an editor that cannot see the
    // red line under the cursor.
    let problem = problem_at(state, &path, line, col);

    spawn_local(async move {
        let info = ipc::call::<_, Option<HoverInfo>>(cmd::lsp::HOVER, &args)
            .await
            .ok()
            .flatten();
        // A card for the diagnostic even when the server has nothing to say
        // about the position, which is common at exactly the places that are
        // broken enough to be underlined.
        if problem.is_none() && info.is_none() {
            return;
        }
        let current = state.active_path_now();
        if current.as_deref() != Some(path.as_str()) {
            return;
        }

        let mut text = String::new();
        if let Some(problem) = &problem {
            let label = match problem.severity {
                rusty_lsp::DiagSeverity::Error => "error",
                rusty_lsp::DiagSeverity::Warning => "warning",
                _ => "note",
            };
            match &problem.code {
                Some(code) => text.push_str(&format!("**{label}[{code}]**\n\n")),
                None => text.push_str(&format!("**{label}**\n\n")),
            }
            text.push_str(&problem.message);
            if info.is_some() {
                text.push_str("\n\n---\n\n");
            }
        }
        if let Some(info) = &info {
            text.push_str(&info.text);
        }

        // The diagnostic's own span when there is one: it is what the reader
        // pointed at, and it is what "moved away" has to be measured against
        // or the card closes while the pointer is still over the red line.
        let range = match (&problem, info.as_ref().and_then(|i| i.range)) {
            (Some(problem), _) => rusty_lsp::EditRange {
                start_line: problem.start_line,
                start_col: problem.start_col,
                end_line: problem.end_line,
                end_col: problem.end_col,
            },
            (None, Some(range)) => range,
            // No range from the server means "just this cell" — the card
            // still needs one to decide what counts as moving away.
            (None, None) => rusty_lsp::EditRange {
                start_line: line,
                start_col: col,
                end_line: line,
                end_col: col + 1,
            },
        };
        state.editor.hover.set(Some(HoverCard {
            path: path.clone(),
            range,
            text,
            line,
            col,
            fixes: rusty_lsp::CodeActions::default(),
        }));

        // What the server offers to do about the problem, on the card that
        // names it. Asked only where there *is* a problem: that is where a
        // fix exists, and it is the whole of the reason somebody hovers a
        // red line. An `impl Trait for T {}` with no members is the case
        // this exists for — the fix is rust-analyzer's own "Implement
        // missing members", reachable until now only by putting the caret
        // there and pressing Ctrl+.
        //
        // It arrives *after* the card rather than with it: a `codeAction`
        // resolves every offer it did not come with, which is a second or
        // more on a cold index, and a tooltip that waits for it is a
        // tooltip that does not appear. The buttons grow onto the card
        // that is already up, and only if it is still that card — the
        // pointer moves on while this is in flight.
        if problem.is_none() {
            return;
        }
        let ask = Ask {
            path: path.clone(),
            line,
            col,
        };
        let Ok(answer) = ipc::call::<_, rusty_lsp::CodeActions>(cmd::lsp::ACTIONS, &ask).await
        else {
            return;
        };
        if answer.fixes.is_empty() {
            return;
        }
        state.editor.hover.update(|card| {
            if let Some(card) = card
                && card.path == path
                && (card.line, card.col) == (line, col)
            {
                card.fixes = answer;
            }
        });
    });
}

/// The diagnostic under a position, worst first.
///
/// Errors outrank warnings at the same spot: two squiggles overlap often —
/// an unused import that is also a type error — and the one that stops the
/// build is the one being asked about.
fn problem_at(
    state: AppState,
    path: &str,
    line: u32,
    col: u32,
) -> Option<rusty_lsp::FileDiagnostic> {
    state
        .lsp
        .diagnostics
        .with_untracked(|by_file| worst_at(by_file.get(path)?, line, col).cloned())
}

/// The pure half of [`problem_at`], so the ranking is pinned by tests rather
/// than by eye — overlapping squiggles are exactly where it would go wrong.
fn worst_at(
    diagnostics: &[rusty_lsp::FileDiagnostic],
    line: u32,
    col: u32,
) -> Option<&rusty_lsp::FileDiagnostic> {
    let mut found: Option<&rusty_lsp::FileDiagnostic> = None;
    for diagnostic in diagnostics {
        let after_start = (diagnostic.start_line, diagnostic.start_col) <= (line, col);
        let before_end = (line, col) < (diagnostic.end_line, diagnostic.end_col);
        if !(after_start && before_end) {
            continue;
        }
        let better = found.is_none_or(|best| {
            matches!(diagnostic.severity, rusty_lsp::DiagSeverity::Error)
                && !matches!(best.severity, rusty_lsp::DiagSeverity::Error)
        });
        if better {
            found = Some(diagnostic);
        }
    }
    found
}

/// Jump to wherever the thing at this position is defined.
///
/// The target lands in `state.editor.reveal`; if it is in another file, that file is
/// opened first and the editor applies the reveal once the document arrives.
/// Which places a command asks the server for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceQuery {
    References,
    Implementations,
    TypeDefinition,
}

/// Where the thing at the caret of this group's editor is used, implemented
/// or typed.
///
/// One implementation or type definition is a jump, as a definition is; more
/// than one, or any references at all, is a list in the finder, titled with
/// the name asked about — even an empty one, which says there were none
/// rather than letting a key seem to do nothing.
pub fn find_places(state: AppState, query: PlaceQuery) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        line: u32,
        col: u32,
    }

    let Some(path) = state.active_path_now() else {
        return;
    };
    if !path.ends_with(".rs") {
        return;
    }
    let Some((line, col)) = caret_position(state) else {
        return;
    };
    let name = state
        .editor
        .draft
        .with_untracked(|text| word_around(text, line, col));
    let command = match query {
        PlaceQuery::References => cmd::lsp::REFERENCES,
        PlaceQuery::Implementations => cmd::lsp::IMPLEMENTATIONS,
        PlaceQuery::TypeDefinition => cmd::lsp::TYPE_DEFINITION,
    };
    let args = Args { path, line, col };
    spawn_local(async move {
        // Errors are the server warming up, as they are for a definition.
        let Ok(mut places) = ipc::call::<_, Vec<rusty_lsp::Place>>(command, &args).await else {
            return;
        };
        // By file, then down each file: the order a list is read in. The
        // server's own puts the declaration wherever its index found it.
        places.sort_by(|a, b| {
            let (a, b) = (&a.location, &b.location);
            (a.external, &a.path, a.line, a.col).cmp(&(b.external, &b.path, b.line, b.col))
        });
        if places.len() == 1 && query != PlaceQuery::References {
            go_to(state, places[0].location.clone());
            return;
        }
        let title = match query {
            PlaceQuery::References => t!("places.references", name = name.clone()),
            PlaceQuery::Implementations => t!("places.implementations", name = name.clone()),
            PlaceQuery::TypeDefinition => t!("places.type-definition", name = name.clone()),
        };
        state
            .layout
            .quick_places
            .set(Some(crate::state::PlaceList { title, places }));
        state.layout.quick_seed.set(String::new());
        state.layout.quick_open.set(true);
    });
}

/// The identifier a position is on or just after — what a list of its uses
/// is titled with.
fn word_around(text: &str, line: u32, col: u32) -> String {
    let chars: Vec<char> = text
        .split('\n')
        .nth(line as usize)
        .unwrap_or_default()
        .chars()
        .collect();
    let is_word = |c: &&char| c.is_alphanumeric() || **c == '_';
    let at = (col as usize).min(chars.len());
    let before = chars[..at].iter().rev().take_while(is_word).count();
    let after = chars[at..].iter().take_while(is_word).count();
    chars[at - before..at + after].iter().collect()
}

/// Ask for the symbols the finder lists: the outline of this group's file
/// for `@`, the workspace's matching `words` for `#`. The answer is kept with
/// its ask, so a slow reply to `#gp` is not shown under `#gpio`.
pub fn ask_symbols(state: AppState, workspace: bool, words: String) {
    #[derive(serde::Serialize)]
    struct File {
        path: String,
    }
    #[derive(serde::Serialize)]
    struct Workspace {
        query: String,
    }

    if state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    spawn_local(async move {
        let (ask, answer) = if workspace {
            let ask = format!("#{words}");
            let answer = ipc::call::<_, Vec<rusty_lsp::Symbol>>(
                cmd::lsp::WORKSPACE_SYMBOLS,
                &Workspace { query: words },
            )
            .await;
            (ask, answer)
        } else {
            let Some(path) = state.active_path_now().filter(|path| path.ends_with(".rs")) else {
                return;
            };
            let ask = format!("@{path}");
            let answer =
                ipc::call::<_, Vec<rusty_lsp::Symbol>>(cmd::lsp::DOCUMENT_SYMBOLS, &File { path })
                    .await;
            (ask, answer)
        };
        if let Ok(symbols) = answer {
            state
                .layout
                .quick_symbols
                .set(Some(crate::state::SymbolAnswer { ask, symbols }));
        }
    });
}

/// Open where a location is, with the caret on it, remembering where the
/// caret was for Back. Outside the project, the file opens read-only.
pub fn go_to(state: AppState, location: rusty_lsp::Location) {
    // Files and Search keep an editor on screen; from anywhere else, the
    // jump lands in Files — a finder row picked over the Git panel.
    let panel = state.layout.panel.get_untracked();
    if panel != "files" && panel != "search" {
        state.layout.panel.set("files".to_string());
    }
    let current = state.active_path_now();
    if current.as_deref() != Some(location.path.as_str()) {
        if location.external {
            open_external(state, location.path.clone());
        } else {
            open_file(state, location.path.clone());
        }
    }
    remember_jump(state, &location);
    state.editor.reveal.set(Some(location));
}

pub fn goto_definition(state: AppState, path: String, line: u32, col: u32) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        line: u32,
        col: u32,
    }

    let args = Args { path, line, col };
    spawn_local(async move {
        // "No definition" is a normal answer over whitespace or a keyword, and
        // an error here is the server warming up. Neither is worth a banner.
        if let Ok(Some(location)) =
            ipc::call::<_, Option<rusty_lsp::Location>>(cmd::lsp::DEFINITION, &args).await
        {
            go_to(state, location);
        }
    });
}

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

    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        text: String,
    }

    if path.ends_with(".rs") && state.lsp.status.get_untracked() == LspStatus::Ready {
        request_semantic(state, path.clone());
        lsp_sync(
            cmd::lsp::CHANGE,
            Args {
                path: path.clone(),
                text: text.clone(),
            },
        );
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
            let mut placed = false;
            state.editor.highlighted.update(|lines| {
                placed =
                    crate::paint::place(lines, &ask.sent, &now, answer.from as usize, answer.lines);
            });
            if placed {
                state.editor.paint.set_value(PaintState {
                    version: Some(answer.version),
                    stale: ask.since,
                });
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

#[cfg(test)]
mod hover_tests {
    use super::worst_at;
    use rusty_lsp::{DiagSeverity, FileDiagnostic};

    fn diag(severity: DiagSeverity, line: u32, from: u32, to: u32) -> FileDiagnostic {
        FileDiagnostic {
            severity,
            message: format!("{severity:?} at {from}..{to}"),
            source: None,
            code: None,
            start_line: line,
            start_col: from,
            end_line: line,
            end_col: to,
        }
    }

    #[test]
    fn only_a_diagnostic_covering_the_position_counts() {
        let diagnostics = [diag(DiagSeverity::Error, 3, 4, 9)];
        assert!(
            worst_at(&diagnostics, 3, 4).is_some(),
            "the first column is inside"
        );
        assert!(worst_at(&diagnostics, 3, 8).is_some());
        // Half-open: the end column is where the squiggle stops, so hovering
        // there is hovering past it.
        assert!(worst_at(&diagnostics, 3, 9).is_none());
        assert!(worst_at(&diagnostics, 3, 3).is_none());
        assert!(
            worst_at(&diagnostics, 2, 5).is_none(),
            "another line entirely"
        );
    }

    #[test]
    fn an_error_outranks_a_warning_at_the_same_place() {
        // Two squiggles overlap often — an unused import that is also a type
        // error — and the one that stops the build is the one being asked
        // about. Both orders, because a first-match loop passes one of them
        // by accident.
        let warning_first = [
            diag(DiagSeverity::Warning, 1, 0, 10),
            diag(DiagSeverity::Error, 1, 2, 6),
        ];
        let error_first = [
            diag(DiagSeverity::Error, 1, 2, 6),
            diag(DiagSeverity::Warning, 1, 0, 10),
        ];
        for diagnostics in [warning_first, error_first] {
            let found = worst_at(&diagnostics, 1, 4).expect("one covers this");
            assert!(
                matches!(found.severity, DiagSeverity::Error),
                "the warning won at column 4",
            );
        }
    }

    #[test]
    fn a_warning_still_shows_where_no_error_covers_it() {
        let diagnostics = [
            diag(DiagSeverity::Warning, 1, 0, 10),
            diag(DiagSeverity::Error, 1, 2, 6),
        ];
        let found = worst_at(&diagnostics, 1, 8).expect("the warning covers this");
        assert!(matches!(found.severity, DiagSeverity::Warning));
    }
}
