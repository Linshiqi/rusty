//! Asking rust-analyzer, and absorbing what it says back.
//!
//! Requests are fired without waiting for the debounced sync: a completion
//! that arrives after the keystroke it was for is a completion nobody wanted.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_edit::Line as EditLine;
use rusty_embed::{LogLevel, LogLine, LogStream};
use rusty_lsp::{HoverInfo, LspEvent};

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{AppState, LspStatus},
};

/// Ask what could complete at the caret.
///
/// The buffer is synced to the server first, without waiting for the pulse:
/// completion after typing `.` is about the text as of *that keystroke*, and a
/// 250ms-stale server answers about the wrong world. `did_change` dedups, so
/// the extra sync costs nothing when the pulse already ran.
pub fn request_completion(state: AppState, path: String, line: u32, col: u32, word_start: u32) {
    #[derive(serde::Serialize)]
    struct Sync {
        path: String,
        text: String,
    }
    #[derive(serde::Serialize)]
    struct Ask {
        path: String,
        line: u32,
        col: u32,
    }

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
        if let Ok(items) =
            ipc::call::<_, Vec<rusty_lsp::CompletionItem>>(cmd::lsp::COMPLETE, &ask).await
        {
            let current = state
                .editor
                .document
                .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
            if current.as_deref() == Some(path.as_str()) && !items.is_empty() {
                state
                    .editor
                    .completion
                    .set(Some(crate::state::CompletionPopup {
                        path,
                        line,
                        word_start,
                        items,
                    }));
            }
        }
    });
}

/// Ask what call the caret sits inside, for the signature card.
///
/// Syncs the draft first, like completion does: an answer about stale text
/// highlights the wrong parameter.
pub fn request_signature(state: AppState, path: String, line: u32, col: u32) {
    #[derive(serde::Serialize)]
    struct Sync {
        path: String,
        text: String,
    }
    #[derive(serde::Serialize)]
    struct Ask {
        path: String,
        line: u32,
        col: u32,
    }

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
        let current = state
            .editor
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
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
    #[derive(serde::Serialize)]
    struct Sync {
        path: String,
        text: String,
    }
    #[derive(serde::Serialize)]
    struct Ask {
        path: String,
        line: u32,
        col: u32,
    }

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
        let Ok(fixes) =
            ipc::call::<_, Vec<rusty_lsp::CodeActionFix>>(cmd::lsp::ACTIONS, &ask).await
        else {
            return;
        };
        let current = state
            .editor
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
        if current.as_deref() == Some(path.as_str()) {
            if fixes.is_empty() {
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: "no quick fixes at the cursor".to_string(),
                    level: None,
                });
            } else {
                state.editor.actions.set(Some((path, line, fixes)));
            }
        }
    });
}

/// Ask for the document's semantic colouring, and keep it only if the answer
/// still describes what is on screen.
pub fn request_semantic(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
    }

    if !path.ends_with(".rs") || state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let args = Args { path: path.clone() };
    spawn_local(async move {
        // Errors and empties are the warm-up talking; the lexical base colour
        // stays up either way, so there is nothing to report.
        let Ok(spans) =
            ipc::call::<_, Vec<rusty_lsp::SemanticSpan>>(cmd::lsp::SEMANTIC, &args).await
        else {
            return;
        };
        let current = state
            .editor
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
        if current.as_deref() == Some(path.as_str()) && !spans.is_empty() {
            state.editor.semantic.set(Some((path, spans)));
        }
    });
}

// ─── the language server ─────────────────────────────────────────────────────

/// Start rust-analyzer for the open project and route what it says into state.
pub fn start_lsp(state: AppState) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    if !state.has_project() {
        return;
    }
    // A stale channel keeps sending after a restart; the session number is how
    // its events are told apart from the live one.
    let session = state.lsp.session.get_untracked() + 1;
    state.lsp.session.set(session);
    state.lsp.status.set(LspStatus::Starting);

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
            // A file opened before the server came up was never announced.
            if let Some(path) = state
                .editor
                .document
                .with_untracked(|d| d.as_ref().map(|d| d.path.clone()))
            {
                lsp_open_doc(path.clone(), state.editor.draft.get_untracked());
                request_semantic(state, path);
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
        LspEvent::Exited {} => {
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
        let current = state
            .editor
            .document
            .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
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
        state.editor.hover.set(Some((path, range, text)));
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
            let current = state
                .editor
                .document
                .with_untracked(|d| d.as_ref().map(|d| d.path.clone()));
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
    });
}

/// The debounced follow-up to typing: re-highlight the draft and tell the
/// server what it says now.
///
/// Scheduled rather than immediate — each is a round trip, and per keystroke
/// that would re-highlight every letter of a word nobody finished typing.
pub fn schedule_pulse(state: AppState) {
    let generation = state.editor.pulse_gen.get_untracked() + 1;
    state.editor.pulse_gen.set(generation);
    set_timeout(
        move || {
            if state.editor.pulse_gen.get_untracked() == generation {
                edit_pulse(state, generation);
            }
        },
        std::time::Duration::from_millis(250),
    );
}

fn edit_pulse(state: AppState, generation: u64) {
    let Some(path) = state
        .editor
        .document
        .with_untracked(|d| d.as_ref().map(|d| d.path.clone()))
    else {
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

    let args = Args { path, text };
    spawn_local(async move {
        if let Ok(lines) = ipc::call::<_, Vec<EditLine>>(cmd::files::HIGHLIGHT, &args).await {
            // Typing continued while this was in flight: the reply describes a
            // text that no longer exists, and painting it would visibly revert
            // the newest keystrokes until the next pulse.
            if state.editor.pulse_gen.get_untracked() == generation {
                state.editor.highlighted.set(lines);
            }
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
