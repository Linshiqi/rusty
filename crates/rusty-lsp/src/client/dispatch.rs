//! What the server says unasked. Its requests to this client are answered —
//! every one, since a request left unanswered stalls the session — and its
//! notifications become the session's state and the frontend's events:
//! diagnostics, progress, the server's health, and asks to ask again.

use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use super::Shared;
use crate::{
    convert,
    model::{HealthLevel, LspEvent},
    pull,
};

/// One message from the server, whatever it is.
pub(super) fn dispatch(shared: &Shared, message: Value) {
    let id = message.get("id").cloned();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);

    match (id, method.as_deref()) {
        (Some(id), Some(method)) => answer(shared, id, method, &message["params"]),
        (Some(id), None) => {
            if let Some(id) = id.as_i64()
                && let Some(waiter) = shared.pending.lock().expect("lsp pending").remove(&id)
            {
                let _ = waiter.send(Some(message));
            }
        }
        (None, Some("textDocument/publishDiagnostics")) => {
            pull::publish(shared, &message["params"]);
        }
        (None, Some("$/progress")) => {
            shared.progress(&message["params"]);
        }
        // The server's own health — see [`LspEvent::Health`]. `quiescent` is
        // not carried: what the editor needs to say is whether the workspace
        // loaded, and a health of `ok` while still indexing is already told
        // by the progress line.
        (None, Some("experimental/serverStatus")) => {
            let params = &message["params"];
            // Settled after loading: run the check. rust-analyzer runs it on
            // a save and at no other time, and a reload clears its results —
            // measured on a workspace sharing `core` with its firmware, the
            // errors arrived, the build-data reload wiped them, and nothing
            // brought them back until the next save.
            let quiescent = params["quiescent"].as_bool().unwrap_or(false);
            if quiescent && !shared.quiescent.swap(true, Ordering::AcqRel) {
                let _ = shared.notify("rust-analyzer/runFlycheck", json!({ "textDocument": null }));
                // And what the editor draws over the text is asked for again.
                // rust-analyzer's own refresh comes while it is still loading,
                // and what is asked then comes back empty — measured as a
                // restarted server answering every refresh with no hints and
                // then saying nothing more, the hints appearing only once
                // somebody typed. Settled is when an answer is whole.
                let _ = shared.events.send(LspEvent::Refresh {});
            } else if !quiescent {
                shared.quiescent.store(false, Ordering::Release);
            }
            let level = match params["health"].as_str() {
                Some("error") => HealthLevel::Error,
                Some("warning") => HealthLevel::Warning,
                _ => HealthLevel::Ok,
            };
            let _ = shared.events.send(LspEvent::Health {
                level,
                message: params["message"].as_str().map(health_text),
            });
        }
        // A message the server asked to have shown. Only the two that name a
        // failure travel; `info` and `log` are narration.
        (None, Some("window/showMessage")) => {
            let params = &message["params"];
            let level = match params["type"].as_u64() {
                Some(1) => HealthLevel::Error,
                Some(2) => HealthLevel::Warning,
                _ => return,
            };
            let _ = shared.events.send(LspEvent::Health {
                level,
                message: params["message"].as_str().map(health_text),
            });
        }
        // Logs: narration, not state.
        _ => {}
    }
}

/// A server-to-client request. Everything rust-analyzer sends with our
/// declared capabilities is satisfied by an empty answer — but it must *get*
/// one, or it waits forever and the session silently stalls.
fn answer(shared: &Shared, id: Value, method: &str, params: &Value) {
    match method {
        "workspace/diagnostic/refresh" => {
            // The server just switched workspaces (build data arrived, a
            // dependency changed) and wants every diagnostic re-requested.
            // This is the moment the push model silently wiped instead.
            let _ = shared.respond(id, Value::Null);
            shared.poke_all_open();
        }
        "workspace/inlayHint/refresh" | "workspace/semanticTokens/refresh" => {
            // The same for what the editor draws over the text: asked for
            // before the workspace loaded, it came back thin.
            let _ = shared.respond(id, Value::Null);
            let _ = shared.events.send(LspEvent::Refresh {});
        }
        "client/registerCapability" => {
            let watches = params["registrations"].as_array().is_some_and(|all| {
                all.iter()
                    .any(|r| r["method"] == "workspace/didChangeWatchedFiles")
            });
            if watches {
                shared.watching.store(true, Ordering::Release);
            }
            let _ = shared.respond(id, Value::Null);
        }
        "workspace/configuration" => {
            let asked = params["items"].as_array().map_or(0, Vec::len);
            let _ = shared.respond(id, Value::Array(vec![Value::Null; asked]));
        }
        _ => {
            let _ = shared.respond(id, Value::Null);
        }
    }
}

/// The server's own words, with a sentence in front of them where rusty
/// recognises what the failure really is. See [`convert::explain_health`].
fn health_text(message: &str) -> String {
    match convert::explain_health(message) {
        Some(named) => format!("{named}\n\n{message}"),
        None => message.to_string(),
    }
}

/// One thing the server has said it is doing.
#[derive(Debug, Clone, Default)]
pub(super) struct Progress {
    title: String,
    message: Option<String>,
    percentage: Option<u64>,
}

impl Shared {
    /// Fold one `$/progress` notification into the table of work in flight
    /// and tell the frontend what that table now says. A `begin` opens an
    /// entry, `report` updates it, `end` closes it; a report for a token
    /// never begun is ignored rather than invented.
    fn progress(&self, params: &Value) {
        let token = match &params["token"] {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let value = &params["value"];
        let text = {
            let mut table = self.progress.lock().expect("lsp progress");
            match value["kind"].as_str() {
                Some("begin") => {
                    table.insert(
                        token,
                        Progress {
                            title: value["title"].as_str().unwrap_or("working").to_string(),
                            message: value["message"].as_str().map(str::to_string),
                            percentage: value["percentage"].as_u64(),
                        },
                    );
                }
                Some("report") => {
                    if let Some(entry) = table.get_mut(&token) {
                        if let Some(message) = value["message"].as_str() {
                            entry.message = Some(message.to_string());
                        }
                        if let Some(percentage) = value["percentage"].as_u64() {
                            entry.percentage = Some(percentage);
                        }
                    }
                }
                Some("end") => {
                    table.remove(&token);
                }
                _ => return,
            }
            progress_summary(table.values())
        };
        let _ = self.events.send(LspEvent::Progress { text });
    }
}

/// The work in flight as one line: `Indexing 45% esp-hal · Fetching`. The
/// first two entries, since a status bar has room for one thought.
fn progress_summary<'a>(entries: impl Iterator<Item = &'a Progress>) -> Option<String> {
    let parts: Vec<String> = entries
        .take(2)
        .map(|entry| {
            let mut text = entry.title.clone();
            if let Some(percentage) = entry.percentage {
                text.push_str(&format!(" {percentage}%"));
            }
            if let Some(message) = &entry.message {
                text.push(' ');
                text.push_str(message);
            }
            text
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}
