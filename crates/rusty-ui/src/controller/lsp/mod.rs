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

mod complete;
mod hover;
mod paint;
mod places;
mod pulse;

pub use complete::*;
pub use hover::*;
pub use paint::*;
pub use places::*;
pub use pulse::*;

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
                    request_semantic(group, path.clone());
                    request_hints(group, path);
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
        // Once the refreshes stop for a moment: rust-analyzer sends one per
        // change of heart while it loads, and each would be a round trip for
        // every file on screen.
        LspEvent::Refresh {} => {
            static TURN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let turn = TURN.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            set_timeout(
                move || {
                    if TURN.load(std::sync::atomic::Ordering::Relaxed) != turn {
                        return;
                    }
                    for group in state.open_groups() {
                        if let Some(path) = group.active_path_now() {
                            request_semantic(group, path.clone());
                            request_hints(group, path);
                        }
                    }
                },
                Duration::from_millis(300),
            );
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
