//! The documents the server has been shown: opening, changing, saving and
//! closing them, the text each was last sent as, and what the frontend is
//! told about each one's diagnostics. A position in a request is converted
//! here too, against that text.
//!
//! **What this client records about a document and what it writes to the
//! server go in one order** (`Shared::order`). Every call runs on its own
//! thread of the blocking pool, and each of these used to record first and
//! write afterwards with nothing held between: two changes could reach the
//! server in the other order — a delta worked out against a text the server
//! had not been sent yet, and its copy of the file wrong from then on — a
//! close could land after the open that followed it, and a question asked
//! after a change could be written ahead of it and answered about the text
//! before. That last one is how inlay hints came to be drawn a line below
//! their code: the answer was about the file before two lines became one.
//!
//! A lock of its own, and not the documents': the thread that reads the
//! server takes the documents' lock for every pushed diagnostic, and a
//! write to a server whose pipe is full waits for that thread to read. Held
//! across the write, the documents' lock would be two pipes waiting on each
//! other. Nothing the reader does takes the order.

use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use super::{LspClient, Shared, transport::Asked};
use crate::{
    error::Result,
    model::{Draft, LspEvent},
    positions::{content_change, scalar_to_character},
    pull,
    uri::path_to_uri,
    watched::FileChange,
};

/// A document as this client last sent it.
pub(super) struct Doc {
    version: i64,
    text: String,
    /// `Draft::seq` of the editor's text this is, or 0 for a text that came
    /// with none. A draft older than this is skipped.
    drafted: u64,
}

impl LspClient {
    /// Show the server a document. Idempotent: opening what is already open is
    /// a no-op, so "reopen after save" needs no bookkeeping in the caller.
    pub fn did_open(&self, path: &str, text: &str) -> Result<()> {
        let opened = {
            let _order = self.shared.order.lock().expect("lsp order");
            self.open_in_order(path, text, 0)?
        };
        if opened {
            // After the notification is on the wire, never before: the
            // puller's request races for the writer, and a pull that
            // overtakes the open is answered for a document the server has
            // not seen.
            self.shared.poke_pull(path);
        }
        Ok(())
    }

    /// [`Self::did_open`] with the order held: recorded and written. Whether
    /// anything was opened.
    fn open_in_order(&self, path: &str, text: &str, drafted: u64) -> Result<bool> {
        {
            let mut docs = self.shared.docs.lock().expect("lsp docs");
            if docs.contains_key(path) {
                return Ok(false);
            }
            docs.insert(
                path.to_string(),
                Doc {
                    version: 1,
                    text: text.to_string(),
                    drafted,
                },
            );
        }
        let language = match self.shared.kind.language_id(path) {
            Some(language) => language,
            None if path.ends_with(".toml") => "toml",
            None => "plaintext",
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
        Ok(true)
    }

    /// Tell the server the document now reads `new_text`.
    ///
    /// The delta is computed here, against the last text sent, so callers just
    /// hand over the whole buffer and a keystroke still travels as one
    /// character.
    pub fn did_change(&self, path: &str, new_text: &str) -> Result<()> {
        self.change(path, new_text, None)
    }

    /// The editor's text of a file, given to the server unless the client
    /// already holds a newer one (`Draft`): two drafts sent at once reach
    /// here in either order, and the older written last would leave the
    /// server with a text the editor has moved on from until the next edit.
    pub fn sync_draft(&self, path: &str, draft: &Draft) -> Result<()> {
        self.change(path, &draft.text, Some(draft.seq))
    }

    fn change(&self, path: &str, new_text: &str, seq: Option<u64>) -> Result<()> {
        let encoding = self.shared.encoding();
        {
            // The delta is against the text before it, so it has to arrive
            // after that text did: recorded and written in one turn.
            let _order = self.shared.order.lock().expect("lsp order");
            let change = {
                let mut docs = self.shared.docs.lock().expect("lsp docs");
                docs.get_mut(path).map(|doc| {
                    if seq.is_some_and(|seq| seq < doc.drafted) {
                        return None;
                    }
                    doc.drafted = seq.unwrap_or(doc.drafted);
                    (doc.text != new_text).then(|| {
                        let change = content_change(&doc.text, new_text, encoding);
                        doc.version += 1;
                        doc.text = new_text.to_string();
                        (doc.version, change)
                    })
                })
            };
            match change {
                // Not open: opening it is how the server comes to have this
                // text.
                None => {
                    self.open_in_order(path, new_text, seq.unwrap_or(0))?;
                }
                // What the server has already.
                Some(None) => return Ok(()),
                Some(Some((version, (start, end, replacement)))) => self.shared.notify(
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
                )?,
            }
        }
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
        {
            // In its turn: an open that follows this close must reach the
            // server after it, or the server ends with the file closed and
            // every change to it after that refused.
            let _order = self.shared.order.lock().expect("lsp order");
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
            self.shared.notify(
                "textDocument/didClose",
                json!({ "textDocument": { "uri": self.shared.uri(path) } }),
            )?;
        }
        self.shared.pulled.lock().expect("lsp pulled").remove(path);
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
        // A server that pulls nothing has pushed the whole answer already.
        if !self.pulls.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
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

    /// Ask about a document as it stands: its text — what the server was
    /// last sent, or what is on disk for a file it was never shown — and a
    /// request made from that text, written in the same turn. No change can
    /// be sent between the two, so the text returned is the text the answer
    /// is about; the answer is waited for after the turn is given up.
    pub(crate) fn ask_about(
        &self,
        path: &str,
        method: &str,
        params: impl FnOnce(&str) -> Value,
    ) -> Result<(String, Asked)> {
        let _order = self.order.lock().expect("lsp order");
        let text = self.text_of(path).unwrap_or_default();
        let asked = self.ask(method, params(&text))?;
        Ok((text, asked))
    }

    /// The document's text: what was last sent when it is open, what is on
    /// disk when it is not.
    pub(crate) fn text_of(&self, path: &str) -> Option<String> {
        self.open_text(path)
            .or_else(|| std::fs::read_to_string(self.root.join(path)).ok())
    }
}
