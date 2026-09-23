//! Diagnostics, pulled.
//!
//! LSP 3.17 lets the client ask for a document's diagnostics instead of
//! waiting to be sent them, and this client asks. After the build-data
//! workspace switch, rust-analyzer never recomputes *pushed* diagnostics for
//! files already open — they are wiped and stay gone, on every project shape.
//! Under the pull model it sends `workspace/diagnostic/refresh` and the
//! client re-requests, so freshness is this client's job, which it can
//! actually do.
//!
//! Requests cannot be made from the reader thread — it would wait on a reply
//! only itself can read — so every refresh hops threads through the poke
//! channel to the loop here.

use std::{
    sync::{Weak, mpsc::Receiver},
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use crate::{
    client::Shared,
    convert,
    error::{Error, Result},
    model::FileDiagnostic,
    uri::uri_to_relative,
};

/// How long to wait between attempts while the server is busy.
const RETRY: Duration = Duration::from_millis(600);
/// How many times a pull is retried before the poke is given up on. The
/// next edit or refresh pokes again anyway.
const ATTEMPTS: u32 = 10;

/// Pull diagnostics for pokes, forever, coalescing bursts.
///
/// Retries while the server is busy: a pull during indexing answers with
/// "content modified" or blocks, and both mean "later", not "never".
///
/// Holds the session **weakly**. The thread blocks on the poke channel,
/// whose sender lives in [`Shared`]; once the client and the reader thread
/// are gone, `Shared` drops, the channel closes, and this returns. Holding
/// it strongly kept `Shared` alive for ever — every open document's text,
/// and the events sender the consumer was blocked on: `events.recv()` in
/// the app never saw the end, and a thread leaked there per session too.
pub(crate) fn pull_loop(poke: Receiver<String>, shared: Weak<Shared>) {
    thread::spawn(move || {
        while let Ok(first) = poke.recv() {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            // Typing produces a poke per pulse; only the newest matters.
            let mut wanted = vec![first];
            while let Ok(more) = poke.try_recv() {
                if !wanted.contains(&more) {
                    wanted.push(more);
                }
            }
            for path in wanted {
                // Closed since the poke: its analysis is no longer this
                // client's to keep (`LspClient::did_close`).
                if !shared.is_open(&path) {
                    continue;
                }
                for attempt in 1..=ATTEMPTS {
                    match pull(&shared, &path) {
                        Ok(items) => {
                            // And closed while the answer was on its way.
                            if !shared.is_open(&path) {
                                break;
                            }
                            shared
                                .pulled
                                .lock()
                                .expect("lsp pulled")
                                .insert(path.clone(), items);
                            shared.emit_diagnostics(&path);
                            break;
                        }
                        // A dead server answers nothing later either.
                        Err(Error::Exited { .. }) => return,
                        Err(_) if attempt < ATTEMPTS => thread::sleep(RETRY),
                        Err(_) => {}
                    }
                }
            }
        }
    });
}

/// One `textDocument/diagnostic` round trip.
fn pull(shared: &Shared, path: &str) -> Result<Vec<FileDiagnostic>> {
    let uri = shared.uri(path);
    let report = shared.request(
        "textDocument/diagnostic",
        json!({ "textDocument": { "uri": uri } }),
    )?;
    // A "full" report carries items; "unchanged" cannot happen because no
    // previousResultId is ever sent.
    let items = report
        .get("items")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    Ok(convert::diagnostics(
        &items,
        shared.text_of(path).as_deref(),
        shared.encoding(),
    ))
}

/// What the frontend is shown for a file: rust-analyzer's own diagnostics
/// and the check's, together, with exact repeats dropped. The check reports
/// a crate once for every workspace that builds it — a `core` shared by the
/// host workspace and the firmware's is every error twice — and a list of
/// doubles reads as twice as much wrong. Two sources saying the same thing
/// in different words are both kept: which one is right is not this
/// function's call.
pub(crate) fn merged(pulled: &[FileDiagnostic], pushed: &[FileDiagnostic]) -> Vec<FileDiagnostic> {
    let mut out: Vec<FileDiagnostic> = Vec::with_capacity(pulled.len() + pushed.len());
    for item in pulled.iter().chain(pushed) {
        if !out.contains(item) {
            out.push(item.clone());
        }
    }
    out
}

/// Record one publishDiagnostics — with pull negotiated, the check's results
/// for a file — and emit the file's merged set.
pub(crate) fn publish(shared: &Shared, params: &Value) {
    let Some(uri) = params["uri"].as_str() else {
        return;
    };
    // Diagnostics for files outside the project — a dependency's source — have
    // nowhere to be shown; the file panel cannot open them.
    let Some(path) = uri_to_relative(uri, &shared.root) else {
        return;
    };

    // An empty push is the check clearing the file — at the start of a run,
    // or because the error is fixed — and it clears only the check's part.
    // It used to be ignored for an open file, as a wipe of the native
    // results, back when pushes were the only place those came from; now
    // they never do, and ignoring it left fixed errors on screen. A pull is
    // still asked for, since the edit that fixed one may have fixed others.
    let items = convert::diagnostics(
        &params["diagnostics"],
        shared.text_of(&path).as_deref(),
        shared.encoding(),
    );
    {
        let mut pushed = shared.pushed.lock().expect("lsp pushed");
        if items.is_empty() {
            pushed.remove(&path);
        } else {
            pushed.insert(path.clone(), items.clone());
        }
    }
    if items.is_empty() && shared.is_open(&path) {
        shared.poke_pull(&path);
    }
    shared.emit_diagnostics(&path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DiagSeverity;

    fn diagnostic(line: u32, source: &str, message: &str) -> FileDiagnostic {
        FileDiagnostic {
            severity: DiagSeverity::Error,
            message: message.into(),
            source: Some(source.into()),
            code: None,
            start_line: line,
            start_col: 11,
            end_line: line,
            end_col: 19,
        }
    }

    /// The reported case: rustc's error arrives from the check and the pull
    /// has nothing — the merge keeps the error, where sending them one after
    /// the other let the empty pull replace it.
    #[test]
    fn an_empty_pull_does_not_take_the_check_error_away() {
        let rustc = diagnostic(6, "rustc", "cannot find type `Vector3d` in this scope");
        assert_eq!(merged(&[], std::slice::from_ref(&rustc)), vec![rustc]);
    }

    /// The same error from the host workspace's check and the firmware's is
    /// one error; the same position said differently by two sources is two.
    #[test]
    fn exact_repeats_go_and_different_words_stay() {
        let rustc = diagnostic(6, "rustc", "cannot find type `Vector3d` in this scope");
        let native = diagnostic(6, "rust-analyzer", "unresolved type");
        let merged = merged(
            std::slice::from_ref(&native),
            &[rustc.clone(), rustc.clone()],
        );
        assert_eq!(merged, vec![native, rustc]);
    }
}
