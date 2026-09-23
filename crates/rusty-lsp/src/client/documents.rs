//! The documents the server has been shown: opening, changing, saving and
//! closing them, the text each was last sent as, and what the frontend is
//! told about each one's diagnostics. A position in a request is converted
//! here too, against that text.

use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use super::{LspClient, Shared};
use crate::{
    error::Result,
    model::LspEvent,
    positions::{content_change, scalar_to_character},
    pull,
    uri::path_to_uri,
    watched::FileChange,
};

/// A document as this client last sent it.
pub(super) struct Doc {
    version: i64,
    text: String,
}

impl LspClient {
    /// Show the server a document. Idempotent: opening what is already open is
    /// a no-op, so "reopen after save" needs no bookkeeping in the caller.
    pub fn did_open(&self, path: &str, text: &str) -> Result<()> {
        {
            let mut docs = self.shared.docs.lock().expect("lsp docs");
            if docs.contains_key(path) {
                return Ok(());
            }
            docs.insert(
                path.to_string(),
                Doc {
                    version: 1,
                    text: text.to_string(),
                },
            );
        }
        let language = match path.rsplit('.').next() {
            Some("rs") => "rust",
            Some("toml") => "toml",
            _ => "plaintext",
        };
        self.shared.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": self.shared.uri(path),
                    "languageId": language,
                    "version": 1,
                    "text": text,
                }
            }),
        )?;
        // After the notification is on the wire, never before: the puller's
        // request races for the writer, and a pull that overtakes the open
        // is answered for a document the server has not seen.
        self.shared.poke_pull(path);
        Ok(())
    }

    /// Tell the server the document now reads `new_text`.
    ///
    /// The delta is computed here, against the last text sent, so callers just
    /// hand over the whole buffer and a keystroke still travels as one
    /// character.
    pub fn did_change(&self, path: &str, new_text: &str) -> Result<()> {
        let encoding = self.shared.encoding();
        let (version, start, end, replacement) = {
            let mut docs = self.shared.docs.lock().expect("lsp docs");
            let Some(doc) = docs.get_mut(path) else {
                drop(docs);
                return self.did_open(path, new_text);
            };
            if doc.text == new_text {
                return Ok(());
            }
            let (start, end, replacement) = content_change(&doc.text, new_text, encoding);
            doc.version += 1;
            doc.text = new_text.to_string();
            (doc.version, start, end, replacement)
        };

        self.shared.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": self.shared.uri(path), "version": version },
                "contentChanges": [{
                    "range": {
                        "start": { "line": start.0, "character": start.1 },
                        "end": { "line": end.0, "character": end.1 },
                    },
                    "text": replacement,
                }],
            }),
        )?;
        // Same ordering as `did_open`: a pull that overtakes the change on
        // the wire is answered for the previous version.
        self.shared.poke_pull(path);
        Ok(())
    }

    /// The document was written to disk. This is what triggers a fresh
    /// `cargo check`, so it is where most diagnostics come from.
    pub fn did_save(&self, path: &str) -> Result<()> {
        self.shared.notify(
            "textDocument/didSave",
            json!({ "textDocument": { "uri": self.shared.uri(path) } }),
        )
    }

    /// Files changed on disk, as rusty's own watcher saw them
    /// (`watched::file_events`). Sent only once the server has asked to be
    /// told — before that it is reading the workspace fresh, and a server
    /// that never asks is watching for itself.
    pub fn did_change_watched_files(&self, events: &[(String, FileChange)]) -> Result<()> {
        if events.is_empty() || !self.shared.watching.load(Ordering::Acquire) {
            return Ok(());
        }
        let changes: Vec<Value> = events
            .iter()
            .map(|(path, change)| json!({ "uri": self.shared.uri(path), "type": *change as u8 }))
            .collect();
        self.shared.notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": changes }),
        )
    }

    /// The editor stopped holding this file.
    ///
    /// An open document is the client's to keep: rust-analyzer answers from
    /// the text it was sent and ignores the disk for it. This was never
    /// sent, so a file closed in the editor stayed, to the server, the text
    /// it had when it was open — whatever a `git checkout` or another editor
    /// did to it afterwards — and every file ever opened stayed in its
    /// memory. Closing hands it back to the disk.
    ///
    /// The server's own analysis of the file goes with it: it is asked for
    /// per open document, and nothing would keep it current. What the check
    /// found stands, as it does for any file nobody opened.
    pub fn did_close(&self, path: &str) -> Result<()> {
        let was_open = self
            .shared
            .docs
            .lock()
            .expect("lsp docs")
            .remove(path)
            .is_some();
        if !was_open {
            return Ok(());
        }
        self.shared.pulled.lock().expect("lsp pulled").remove(path);
        self.shared.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": self.shared.uri(path) } }),
        )?;
        self.shared.emit_diagnostics(path);
        Ok(())
    }

    /// A frontend scalar column as a protocol position.
    pub(super) fn protocol_position(&self, path: &str, line: u32, col: u32) -> Value {
        let encoding = self.shared.encoding();
        let docs = self.shared.docs.lock().expect("lsp docs");
        let character = docs
            .get(path)
            .and_then(|doc| doc.text.split('\n').nth(line as usize))
            .map(|line_text| scalar_to_character(line_text, col, encoding))
            .unwrap_or(col);
        json!({ "line": line, "character": character })
    }

    /// A document and a position in it, as most requests are asked.
    pub(super) fn position_params(&self, path: &str, line: u32, col: u32) -> Value {
        json!({
            "textDocument": { "uri": self.shared.uri(path) },
            "position": self.protocol_position(path, line, col),
        })
    }
}

impl Shared {
    /// A project-relative path as the URI the server knows it by.
    pub(crate) fn uri(&self, path: &str) -> String {
        path_to_uri(&self.root.join(path))
    }

    /// Tell the frontend what is wrong in `path`, from both sources at once.
    pub(crate) fn emit_diagnostics(&self, path: &str) {
        let pulled = self
            .pulled
            .lock()
            .expect("lsp pulled")
            .get(path)
            .cloned()
            .unwrap_or_default();
        let pushed = self
            .pushed
            .lock()
            .expect("lsp pushed")
            .get(path)
            .cloned()
            .unwrap_or_default();
        let _ = self.events.send(LspEvent::Diagnostics {
            path: path.to_string(),
            items: pull::merged(&pulled, &pushed),
        });
    }

    pub(crate) fn poke_pull(&self, path: &str) {
        if let Some(poke) = self.poke.lock().expect("lsp poke").as_ref() {
            let _ = poke.send(path.to_string());
        }
    }

    pub(super) fn poke_all_open(&self) {
        let open: Vec<String> = self
            .docs
            .lock()
            .expect("lsp docs")
            .keys()
            .cloned()
            .collect();
        for path in open {
            self.poke_pull(&path);
        }
    }

    pub(crate) fn is_open(&self, path: &str) -> bool {
        self.docs.lock().expect("lsp docs").contains_key(path)
    }

    /// The document as this client last sent it, if it is open.
    pub(crate) fn open_text(&self, path: &str) -> Option<String> {
        self.docs
            .lock()
            .expect("lsp docs")
            .get(path)
            .map(|doc| doc.text.clone())
    }

    /// The document's text: what was last sent when it is open, what is on
    /// disk when it is not.
    pub(crate) fn text_of(&self, path: &str) -> Option<String> {
        self.open_text(path)
            .or_else(|| std::fs::read_to_string(self.root.join(path)).ok())
    }
}
