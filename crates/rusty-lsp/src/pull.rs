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
    model::{FileDiagnostic, LspEvent},
    uri::{path_to_uri, uri_to_relative},
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
                for attempt in 1..=ATTEMPTS {
                    match pull(&shared, &path) {
                        Ok(items) => {
                            let _ = shared.events.send(LspEvent::Diagnostics {
                                path: path.clone(),
                                items,
                            });
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
    let uri = path_to_uri(&shared.root.join(path));
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

/// Convert one publishDiagnostics into the wire model and emit it.
pub(crate) fn publish(shared: &Shared, params: &Value) {
    let Some(uri) = params["uri"].as_str() else {
        return;
    };
    // Diagnostics for files outside the project — a dependency's source — have
    // nowhere to be shown; the file panel cannot open them.
    let Some(path) = uri_to_relative(uri, &shared.root) else {
        return;
    };

    // Pushed emptiness for a document the puller owns is exactly the wipe this
    // client moved to the pull model to escape; the pull that follows the next
    // refresh or edit is authoritative. Pushes still matter for files nothing
    // has opened — whole-project results land there.
    let items = convert::diagnostics(
        &params["diagnostics"],
        shared.text_of(&path).as_deref(),
        shared.encoding(),
    );
    if items.is_empty() && shared.is_open(&path) {
        shared.poke_pull(&path);
        return;
    }

    let _ = shared.events.send(LspEvent::Diagnostics { path, items });
}
