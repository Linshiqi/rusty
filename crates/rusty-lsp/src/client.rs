//! The rust-analyzer session.
//!
//! One server per project, spoken to over stdio. The shape mirrors the
//! terminal's: a reader thread feeds an event channel, requests are correlated
//! by id, and the whole thing dies with the child process.
//!
//! What lives here is the session itself — the process, the transport, the
//! documents it has been shown, and the handshake. Finding the binary is
//! [`discover`], turning replies into the model is [`convert`], keeping
//! diagnostics fresh is [`pull`], and every URI goes through [`uri`].
//!
//! - `check.allTargets` defaults to on, which builds tests and benches. A
//!   `no_std` firmware has no test harness, so that default drowns every real
//!   diagnostic in "can't find crate for `test`". It is turned off.
//! - Diagnostics are **pulled** (LSP 3.17), not just received. After the
//!   build-data workspace switch, r-a never recomputes pushed diagnostics for
//!   open files — they wipe and stay gone. Under pull, it asks the client to
//!   re-request instead, and the puller thread owns freshness.

use std::{
    collections::HashMap,
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    process::Child,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    convert, discover,
    error::{Error, Result},
    model::{
        CodeActionFix, CompletionItem, HoverInfo, Location, LspEvent, SemanticSpan, SignatureInfo,
    },
    positions::{Encoding, content_change, scalar_to_character},
    pull, rpc,
    uri::{path_to_uri, uri_to_absolute, uri_to_relative},
};

/// How long a request may take before the caller is told rather than kept
/// waiting. Generous because a cold index answers slowly; callers that want to
/// retry — completion during startup — retry above this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// How long `shutdown` may take on the way out. rust-analyzer answers it in
/// milliseconds; one that cannot is about to be killed anyway.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a server told to `exit` is given to do so before `kill`.
const EXIT_GRACE: Duration = Duration::from_millis(500);

/// How many lazily-resolved code actions get a `codeAction/resolve` round
/// trip per request, and how long each may take. Twenty-four sequential
/// round trips at the full request budget is how one slow server turned
/// Ctrl+. into a six-minute wait.
const MAX_RESOLVES: usize = 8;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// A running rust-analyzer, and the documents it has been shown.
pub struct LspClient {
    shared: Arc<Shared>,
    /// `None` when the session runs over a transport that is not a child
    /// process — the tests' in-memory pipes.
    child: Mutex<Option<Child>>,
}

/// Diagnostics and lifecycle, for exactly one consumer.
///
/// Ends — `recv` returns `None` — once the client is dropped and the reader
/// has seen the server go: nothing else holds the sender.
pub struct Events {
    rx: Receiver<LspEvent>,
}

impl Events {
    pub fn recv(&self) -> Option<LspEvent> {
        self.rx.recv().ok()
    }

    /// `None` on a timeout *or* on the end of the stream; a caller that has
    /// to tell them apart calls [`Events::recv`] on a thread of its own.
    pub fn recv_timeout(&self, within: Duration) -> Option<LspEvent> {
        self.rx.recv_timeout(within).ok()
    }
}

struct Doc {
    version: i64,
    text: String,
}

/// Everything the session's threads share: the writer, the correlation
/// table, the documents, and what the handshake learned.
pub(crate) struct Shared {
    writer: Mutex<Box<dyn Write + Send>>,
    /// Wake the puller for a path. Requests cannot be made from the reader
    /// thread — it would wait on a reply only itself can read — so refreshes
    /// hop threads through this. Set after the handshake, when the loop
    /// starts; the puller holds only a `Weak` to this struct, so dropping
    /// the last strong reference closes the channel and ends it.
    poke: Mutex<Option<Sender<String>>>,
    /// Requests awaiting a reply, by id. `None` down the channel means the
    /// reader is gone and no reply will come.
    pending: Mutex<HashMap<i64, Sender<Option<Value>>>>,
    docs: Mutex<HashMap<String, Doc>>,
    next_id: AtomicI64,
    /// False once the reader thread has ended: every request from then on
    /// fails at once instead of waiting its budget out for an answer that
    /// cannot arrive.
    alive: AtomicBool,
    /// Set once the handshake has read what the server picked. Diagnostics
    /// only arrive after `initialized`, so the default is never actually used.
    encoding: OnceLock<Encoding>,
    semantic_legend: OnceLock<Vec<String>>,
    pub(crate) root: PathBuf,
    pub(crate) events: Sender<LspEvent>,
}

impl LspClient {
    /// Start rust-analyzer for the project at `root`.
    ///
    /// `target` is the triple the firmware builds for, when the caller knows
    /// it — detection does — so cfg resolution matches the chip rather than
    /// the host.
    pub fn spawn(root: &Path, target: Option<&str>) -> Result<(LspClient, Events)> {
        let binary = discover::find_rust_analyzer().ok_or(Error::NotFound)?;
        let mut child = discover::command_for(&binary, root)
            .spawn()
            .map_err(Error::Spawn)?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        Self::connect(Box::new(stdout), Box::new(stdin), Some(child), root, target)
    }

    /// A session over an already-open transport.
    ///
    /// What [`LspClient::spawn`] builds once the process exists — and what
    /// the tests build over a pair of pipes with a fake server on the other
    /// end, so the handshake, the correlation and the shutdown are proved
    /// against something that answers, without a rust-analyzer on the
    /// machine.
    pub(crate) fn connect(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        child: Option<Child>,
        root: &Path,
        target: Option<&str>,
    ) -> Result<(LspClient, Events)> {
        let (events_tx, events_rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            writer: Mutex::new(writer),
            poke: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            docs: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(1),
            alive: AtomicBool::new(true),
            encoding: OnceLock::new(),
            semantic_legend: OnceLock::new(),
            root: root.to_path_buf(),
            events: events_tx,
        });
        pump(reader, Arc::clone(&shared));

        // The handshake, before anyone else gets the client. A failure here
        // must kill the child by hand — no `LspClient` exists yet to do it on
        // drop, and a leaked rust-analyzer holds the project's target dir open.
        if let Err(e) = handshake(&shared, root, target) {
            if let Some(mut child) = child {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err(e);
        }

        let (poke_tx, poke_rx) = mpsc::channel();
        *shared.poke.lock().expect("lsp poke") = Some(poke_tx);
        pull::pull_loop(poke_rx, Arc::downgrade(&shared));

        Ok((
            LspClient {
                shared,
                child: Mutex::new(child),
            },
            Events { rx: events_rx },
        ))
    }

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
                    "uri": self.uri(path),
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
                "textDocument": { "uri": self.uri(path), "version": version },
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
            json!({ "textDocument": { "uri": self.uri(path) } }),
        )
    }

    /// What could complete at this position. Columns are scalars, as
    /// everywhere on the frontend side.
    pub fn completion(&self, path: &str, line: u32, col: u32) -> Result<Vec<CompletionItem>> {
        let position = self.protocol_position(path, line, col);
        let result = self.shared.request(
            "textDocument/completion",
            json!({
                "textDocument": { "uri": self.uri(path) },
                "position": position,
            }),
        )?;
        let text = self.shared.open_text(path).unwrap_or_default();
        Ok(convert::completion_items(
            &result,
            &text,
            self.shared.encoding(),
        ))
    }

    /// What the thing under this position is, as prose, and how much text the
    /// answer covers — the range is what lets a tooltip stay up while the
    /// pointer moves within the same token.
    pub fn hover(&self, path: &str, line: u32, col: u32) -> Result<Option<HoverInfo>> {
        let position = self.protocol_position(path, line, col);
        let result = self.shared.request(
            "textDocument/hover",
            json!({
                "textDocument": { "uri": self.uri(path) },
                "position": position,
            }),
        )?;
        let text = self.shared.open_text(path);
        Ok(convert::hover_info(
            &result,
            text.as_deref(),
            self.shared.encoding(),
        ))
    }

    /// The signature of the call around this position, if the caret is inside
    /// one.
    pub fn signature_help(&self, path: &str, line: u32, col: u32) -> Result<Option<SignatureInfo>> {
        let position = self.protocol_position(path, line, col);
        let result = self.shared.request(
            "textDocument/signatureHelp",
            json!({
                "textDocument": { "uri": self.uri(path) },
                "position": position,
            }),
        )?;
        Ok(convert::signature_info(&result))
    }

    /// The quick fixes and refactorings available at a position, with their
    /// edits resolved and converted — ready to splice.
    ///
    /// Lazily-resolved actions get a `codeAction/resolve` round trip each, up
    /// to a budget. Actions that edit other files, or only carry a
    /// server-side command, are dropped: half of a multi-file fix is worse
    /// than none. A resolve that fails is swallowed only while there is
    /// something else to offer — an empty menu with a reason in hand is an
    /// error the caller should hear.
    pub fn code_actions(&self, path: &str, line: u32, col: u32) -> Result<Vec<CodeActionFix>> {
        let position = self.protocol_position(path, line, col);
        let result = self.shared.request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": self.uri(path) },
                "range": { "start": position, "end": position },
                // Empty is fine: rust-analyzer matches its own diagnostics by
                // range rather than trusting the client's copy.
                "context": { "diagnostics": [] },
            }),
        )?;

        let ours = self.uri(path);
        let text = self.shared.open_text(path).unwrap_or_default();
        let encoding = self.shared.encoding();
        let mut fixes = Vec::new();
        let mut resolves = 0usize;
        let mut failed: Option<Error> = None;
        for offer in result.as_array().into_iter().flatten() {
            let Some(title) = offer["title"].as_str() else {
                continue;
            };
            let kind = offer["kind"].as_str().map(str::to_string);

            let resolved;
            let action = if offer.get("edit").is_some() {
                offer
            } else {
                if resolves >= MAX_RESOLVES {
                    continue;
                }
                resolves += 1;
                match self.shared.request_within(
                    "codeAction/resolve",
                    offer.clone(),
                    RESOLVE_TIMEOUT,
                ) {
                    Ok(full) => {
                        resolved = full;
                        &resolved
                    }
                    Err(error) => {
                        failed.get_or_insert(error);
                        continue;
                    }
                }
            };

            if let Some(edits) = convert::single_file_edits(&action["edit"], &ours)
                && let Some(edits) = convert::action_edits(&edits, &text, encoding)
                && !edits.is_empty()
            {
                fixes.push(CodeActionFix {
                    title: title.to_string(),
                    kind,
                    edits,
                });
            }
        }
        match (fixes.is_empty(), failed) {
            (true, Some(error)) => Err(error),
            _ => Ok(fixes),
        }
    }

    /// Rename the symbol at this position, everywhere, and write the files.
    ///
    /// Applied here rather than returned, because converting the server's
    /// columns needs each file's own text and only a file this client can
    /// read has any. A code action refuses when other files are involved;
    /// a rename must not — a `pub fn` renamed in one file and not its callers
    /// is a broken build, and that is the *normal* case.
    ///
    /// Every file is read and converted before any is written. A file the
    /// server names that cannot be read, or that has changed since the
    /// server read it, refuses the whole rename — not the half of it that
    /// came after. The caller is expected to have saved first: these edits
    /// land on disk, and an unsaved buffer would be overwritten by its own
    /// stale bytes on the next save. Returns the paths that changed, newest
    /// knowledge for whoever has them open.
    pub fn rename(&self, path: &str, line: u32, col: u32, new_name: &str) -> Result<Vec<String>> {
        let position = self.protocol_position(path, line, col);
        let result = self.shared.request(
            "textDocument/rename",
            json!({
                "textDocument": { "uri": self.uri(path) },
                "position": position,
                "newName": new_name,
            }),
        )?;

        let encoding = self.shared.encoding();
        let mut planned: Vec<(PathBuf, String)> = Vec::new();
        for (uri, edits) in convert::edits_by_file(&result)? {
            let Some(file) = uri_to_absolute(&uri) else {
                return Err(Error::Server {
                    method: "textDocument/rename".into(),
                    message: format!("rust-analyzer named a file this client cannot locate: {uri}"),
                });
            };
            let file = PathBuf::from(file);
            let text = std::fs::read_to_string(&file).map_err(|source| Error::Apply {
                path: file.display().to_string(),
                source,
            })?;
            let Some(out) = convert::apply_text_edits(&text, &edits, encoding) else {
                return Err(Error::Server {
                    method: "textDocument/rename".into(),
                    message: format!(
                        "{} has changed since rust-analyzer last read it — save and try again",
                        file.display()
                    ),
                });
            };
            if out != text {
                planned.push((file, out));
            }
        }

        let mut changed = Vec::new();
        for (file, out) in planned {
            std::fs::write(&file, &out).map_err(|source| Error::Apply {
                path: file.display().to_string(),
                source,
            })?;
            changed.push(file.display().to_string());
        }
        changed.sort();
        Ok(changed)
    }

    /// The whole document's semantic colouring, as the server sees it — for
    /// an open document; there is nothing to convert against otherwise.
    pub fn semantic_tokens(&self, path: &str) -> Result<Vec<SemanticSpan>> {
        let result = self.shared.request(
            "textDocument/semanticTokens/full",
            json!({ "textDocument": { "uri": self.uri(path) } }),
        )?;
        let data: Vec<u32> = result["data"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|v| v as u32)
                    .collect()
            })
            .unwrap_or_default();
        let Some(text) = self.shared.open_text(path) else {
            return Ok(Vec::new());
        };
        Ok(convert::semantic_spans(
            &data,
            &text,
            &self.shared.legend(),
            self.shared.encoding(),
        ))
    }

    /// Where the thing under this position is defined.
    ///
    /// A definition in a dependency or the sysroot comes back with `external`
    /// set and an absolute path — most of what anyone Ctrl+clicks in firmware
    /// lives in esp-hal or `core`, and answering `None` for all of it made the
    /// gesture look broken.
    pub fn definition(&self, path: &str, line: u32, col: u32) -> Result<Option<Location>> {
        let position = self.protocol_position(path, line, col);
        let result = self.shared.request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": self.uri(path) },
                "position": position,
            }),
        )?;

        let first = result
            .as_array()
            .and_then(|a| a.first().cloned())
            .unwrap_or(result);
        let Some(uri) = first["uri"].as_str() else {
            return Ok(None);
        };
        let line = first["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;
        let character = first["range"]["start"]["character"].as_u64().unwrap_or(0) as u32;

        if let Some(rel) = uri_to_relative(uri, &self.shared.root) {
            let col = self.scalarize(&rel, line, character);
            return Ok(Some(Location {
                path: rel,
                line,
                col,
                external: false,
            }));
        }

        // Outside the project. The absolute path travels; the viewer decides
        // whether it is somewhere it is willing to read.
        let Some(absolute) = uri_to_absolute(uri) else {
            return Ok(None);
        };
        let col = std::fs::read_to_string(&absolute)
            .ok()
            .map_or(character, |text| {
                convert::scalar_at(&text, line, character, self.shared.encoding())
            });
        Ok(Some(Location {
            path: absolute.replace('\\', "/"),
            line,
            col,
            external: true,
        }))
    }

    fn uri(&self, path: &str) -> String {
        path_to_uri(&self.shared.root.join(path))
    }

    /// A frontend scalar column as a protocol position.
    fn protocol_position(&self, path: &str, line: u32, col: u32) -> Value {
        let encoding = self.shared.encoding();
        let docs = self.shared.docs.lock().expect("lsp docs");
        let character = docs
            .get(path)
            .and_then(|doc| doc.text.split('\n').nth(line as usize))
            .map(|line_text| scalar_to_character(line_text, col, encoding))
            .unwrap_or(col);
        json!({ "line": line, "character": character })
    }

    /// A protocol column as a scalar one, for a file that may not be open.
    fn scalarize(&self, path: &str, line: u32, character: u32) -> u32 {
        let encoding = self.shared.encoding();
        self.shared.text_of(path).map_or(character, |text| {
            convert::scalar_at(&text, line, character, encoding)
        })
    }
}

impl Drop for LspClient {
    /// Ask before killing. `shutdown` lets rust-analyzer finish what it is
    /// writing and release the target directory; `exit` ends it; the process
    /// is killed only if it lingers. A server that has already died answers
    /// neither, and `alive` makes both return at once rather than after a
    /// timeout — a project switch must not stall on the corpse of the last
    /// server.
    fn drop(&mut self) {
        let _ = self
            .shared
            .request_within("shutdown", Value::Null, SHUTDOWN_TIMEOUT);
        let _ = self.shared.notify("exit", Value::Null);

        let Ok(mut slot) = self.child.lock() else {
            return;
        };
        let Some(mut child) = slot.take() else {
            return;
        };
        let deadline = Instant::now() + EXIT_GRACE;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Shared {
    pub(crate) fn poke_pull(&self, path: &str) {
        if let Some(poke) = self.poke.lock().expect("lsp poke").as_ref() {
            let _ = poke.send(path.to_string());
        }
    }

    fn poke_all_open(&self) {
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

    pub(crate) fn encoding(&self) -> Encoding {
        *self.encoding.get().unwrap_or(&Encoding::Utf16)
    }

    /// The token-type legend the server declared at initialize; indexes in
    /// every semantic-tokens response point into this.
    fn legend(&self) -> Vec<String> {
        self.semantic_legend.get().cloned().unwrap_or_default()
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

    fn write(&self, message: &Value) -> Result<()> {
        let mut writer = self.writer.lock().expect("lsp writer");
        rpc::write_message(&mut **writer, message).map_err(Error::Io)
    }

    pub(crate) fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn respond(&self, id: Value, result: Value) -> Result<()> {
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    pub(crate) fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_within(method, params, REQUEST_TIMEOUT)
    }

    fn request_within(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let gone = || Error::Exited {
            method: method.to_string(),
        };
        if !self.alive.load(Ordering::Acquire) {
            return Err(gone());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().expect("lsp pending").insert(id, tx);
        // The reader may have ended between the check above and the insert,
        // after failing every waiter it could see; one registered after that
        // would never be told. Look again now that this one is registered.
        if !self.alive.load(Ordering::Acquire) {
            self.pending.lock().expect("lsp pending").remove(&id);
            return Err(gone());
        }

        if let Err(error) =
            self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        {
            self.pending.lock().expect("lsp pending").remove(&id);
            return Err(error);
        }

        match rx.recv_timeout(timeout) {
            Ok(Some(response)) => {
                if let Some(error) = response.get("error") {
                    Err(Error::Server {
                        method: method.to_string(),
                        message: error["message"]
                            .as_str()
                            .unwrap_or("unknown error")
                            .to_string(),
                    })
                } else {
                    Ok(response.get("result").cloned().unwrap_or(Value::Null))
                }
            }
            Ok(None) => Err(gone()),
            Err(_) => {
                self.pending.lock().expect("lsp pending").remove(&id);
                Err(Error::Timeout {
                    method: method.to_string(),
                })
            }
        }
    }

    /// The reader has stopped: every request still waiting learns so now
    /// rather than at its timeout, and every later one at once.
    fn reader_gone(&self) {
        self.alive.store(false, Ordering::Release);
        let waiters: Vec<Sender<Option<Value>>> = self
            .pending
            .lock()
            .expect("lsp pending")
            .drain()
            .map(|(_, waiter)| waiter)
            .collect();
        for waiter in waiters {
            let _ = waiter.send(None);
        }
    }
}

/// The `initialize` round trip.
fn handshake(shared: &Arc<Shared>, root: &Path, target: Option<&str>) -> Result<()> {
    let mut cargo = serde_json::Map::new();
    // Tests and benches do not build in `no_std` — there is no test harness —
    // so the default of checking `--all-targets` buries every real diagnostic
    // under "can't find crate for `test`".
    cargo.insert("allTargets".into(), json!(false));
    if let Some(target) = target {
        cargo.insert("target".into(), json!(target));
    }

    // Named only when there is something to name. An empty `linkedProjects`
    // is not the same as an absent one — it tells rust-analyzer the set of
    // projects is exactly nothing, and the root workspace stops loading.
    let mut options = serde_json::Map::new();
    let linked = discover::linked_projects(root);
    if !linked.is_empty() {
        let mut all = vec![root.join("Cargo.toml").to_string_lossy().into_owned()];
        all.extend(linked);
        options.insert("linkedProjects".into(), json!(all));
    }
    options.insert("cargo".into(), Value::Object(cargo));
    options.insert("check".into(), json!({ "allTargets": false }));
    // Off, deliberately. Embedded projects here use `build-std`, and
    // `cargo check` under build-std emits messages for packages that are not
    // in `cargo metadata` — rust-analyzer logs an error storm and, when the
    // run completes, publishes empty diagnostics that wipe the native ones.
    // Observed as: squiggles appear for a few seconds, then vanish. Native
    // diagnostics — type errors, unresolved names — are the ones the editor
    // needs live anyway.
    options.insert("checkOnSave".into(), json!(false));

    let params = json!({
        "processId": std::process::id(),
        "rootUri": path_to_uri(root),
        "capabilities": {
            // Offer utf-8 first: rust-analyzer takes it, and then "character"
            // means bytes, which is the cheap direction for a Rust client.
            "general": { "positionEncodings": ["utf-8", "utf-16"] },
            "textDocument": {
                "synchronization": { "didSave": true },
                "publishDiagnostics": {},
                // Pull, not just push. After the build-data workspace switch,
                // rust-analyzer stops recomputing pushed diagnostics for open
                // files — they get wiped and stay gone. Under the pull model it
                // asks the client to re-request instead, and freshness becomes
                // this client's job, which it can actually do.
                "diagnostic": { "relatedDocumentSupport": false },
                // No snippet support declared, so inserts arrive as plain text
                // rather than `$0` placeholders nothing here interprets.
                "completion": { "completionItem": { "snippetSupport": false } },
                "hover": { "contentFormat": ["plaintext", "markdown"] },
                "definition": {},
                // Actions come back as literals with lazily-resolved edits;
                // both halves are declared or rust-analyzer sends commands
                // this client cannot execute.
                "codeAction": {
                    "codeActionLiteralSupport": {
                        "codeActionKind": {
                            "valueSet": ["", "quickfix", "refactor", "refactor.rewrite"],
                        },
                    },
                    "resolveSupport": { "properties": ["edit"] },
                },
                // Semantic tokens — the colours only the compiler's view can
                // produce. `formats: ["relative"]` is mandatory; the token
                // types listed are the standard set, and the server's own
                // legend (captured below) is what decodes the reply.
                "semanticTokens": {
                    "requests": { "full": true },
                    "tokenTypes": [
                        "namespace", "type", "class", "enum", "interface", "struct",
                        "typeParameter", "parameter", "variable", "property",
                        "enumMember", "event", "function", "method", "macro",
                        "keyword", "modifier", "comment", "string", "number",
                        "regexp", "operator", "decorator",
                    ],
                    "tokenModifiers": [],
                    "formats": ["relative"],
                },
            },
            "window": { "workDoneProgress": false },
            "workspace": {
                "workspaceFolders": false,
                "configuration": false,
                "diagnostics": { "refreshSupport": true },
            },
        },
        "initializationOptions": Value::Object(options),
    });

    let reply = shared.request("initialize", params)?;
    let encoding = match reply["capabilities"]["positionEncoding"].as_str() {
        Some("utf-8") => Encoding::Utf8,
        _ => Encoding::Utf16,
    };
    let _ = shared.encoding.set(encoding);

    let legend: Vec<String> =
        reply["capabilities"]["semanticTokensProvider"]["legend"]["tokenTypes"]
            .as_array()
            .map(|types| {
                types
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
    let _ = shared.semantic_legend.set(legend);

    shared.notify("initialized", json!({}))
}

/// Read the server forever, feeding responses and diagnostics.
///
/// On the way out it fails every waiting request and says the server has
/// exited. It also drops its own reference to [`Shared`], which — once the
/// client is dropped too — closes the poke channel and the events channel:
/// the puller ends, and the consumer's `recv` returns `None`.
fn pump(reader: Box<dyn Read + Send>, shared: Arc<Shared>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        // An Err is treated as EOF: a mangled frame means the stream has
        // drifted and every later byte would misparse anyway.
        while let Ok(Some(message)) = rpc::read_message(&mut reader) {
            dispatch(&shared, message);
        }
        shared.reader_gone();
        let _ = shared.events.send(LspEvent::Exited {});
    });
}

fn dispatch(shared: &Shared, message: Value) {
    let id = message.get("id").cloned();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);

    match (id, method) {
        // A server-to-client request. Everything rust-analyzer sends with our
        // declared capabilities is satisfied by an empty answer — but it must
        // *get* one, or it waits forever and the session silently stalls.
        (Some(id), Some(method)) => {
            if method == "workspace/diagnostic/refresh" {
                // The server just switched workspaces (build data arrived, a
                // dependency changed) and wants every diagnostic re-requested.
                // This is the moment the push model silently wiped instead.
                let _ = shared.respond(id, Value::Null);
                shared.poke_all_open();
                return;
            }
            let result = if method == "workspace/configuration" {
                let asked = message["params"]["items"].as_array().map_or(0, Vec::len);
                Value::Array(vec![Value::Null; asked])
            } else {
                Value::Null
            };
            let _ = shared.respond(id, result);
        }
        (Some(id), None) => {
            if let Some(id) = id.as_i64()
                && let Some(waiter) = shared.pending.lock().expect("lsp pending").remove(&id)
            {
                let _ = waiter.send(Some(message));
            }
        }
        (None, Some(method)) if method == "textDocument/publishDiagnostics" => {
            pull::publish(shared, &message["params"]);
        }
        // Progress, logs, show-message: narration, not state.
        _ => {}
    }
}

/// A fake rust-analyzer on the other end of two pipes.
///
/// The unit tests elsewhere prove the arithmetic and the integration test
/// proves the real server; neither reaches the transport — correlation,
/// what happens when the server dies with a request outstanding, what a
/// drop does. These do, against a peer that speaks JSON-RPC over
/// `std::io::pipe` and answers only what each test needs.
#[cfg(test)]
mod tests {
    use std::io::PipeWriter;
    use std::sync::mpsc::RecvTimeoutError;

    use super::*;

    /// Every message the fake server received, in order.
    type Seen = Arc<Mutex<Vec<Value>>>;

    fn method(message: &Value) -> &str {
        message["method"].as_str().unwrap_or("")
    }

    /// Answer a request with `result`.
    fn reply(writer: &mut dyn Write, request: &Value, result: Value) {
        let _ = rpc::write_message(
            writer,
            &json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }),
        );
    }

    /// Answer a request with an error.
    fn refuse(writer: &mut dyn Write, request: &Value, message: &str) {
        let _ = rpc::write_message(
            writer,
            &json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "error": { "code": -32603, "message": message },
            }),
        );
    }

    /// The answers every session needs and no test cares about.
    fn default_handle(message: &Value, writer: &mut PipeWriter) -> bool {
        if method(message) == "textDocument/diagnostic" {
            reply(writer, message, json!({ "kind": "full", "items": [] }));
        }
        true
    }

    /// Start the peer. `handle` sees every message that is not part of the
    /// lifecycle — `initialize`, `shutdown` and `exit` are answered here —
    /// and returns `false` to hang up, which is what a crashed server looks
    /// like from the client's side: end of stream, no answer.
    fn fake_server(
        mut handle: impl FnMut(&Value, &mut PipeWriter) -> bool + Send + 'static,
    ) -> (Box<dyn Read + Send>, Box<dyn Write + Send>, Seen) {
        let (client_reads, server_writes) = std::io::pipe().expect("a pipe");
        let (server_reads, client_writes) = std::io::pipe().expect("a pipe");
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        thread::spawn(move || {
            let mut reader = BufReader::new(server_reads);
            let mut writer = server_writes;
            while let Ok(Some(message)) = rpc::read_message(&mut reader) {
                log.lock().unwrap().push(message.clone());
                match method(&message) {
                    "initialize" => reply(
                        &mut writer,
                        &message,
                        json!({ "capabilities": { "positionEncoding": "utf-8" } }),
                    ),
                    "shutdown" => reply(&mut writer, &message, Value::Null),
                    // Dropping the writer is the end of the stream on the
                    // client's reader, exactly as a real exit is.
                    "exit" => return,
                    _ => {
                        if !handle(&message, &mut writer) {
                            return;
                        }
                    }
                }
            }
        });
        (Box::new(client_reads), Box::new(client_writes), seen)
    }

    fn methods(seen: &Seen) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .map(|m| method(m).to_string())
            .collect()
    }

    /// Wait until the server has seen `wanted`, or give up.
    fn saw(seen: &Seen, wanted: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if methods(seen).iter().any(|m| m == wanted) {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Two requests in flight, answered in the other order. The reply's id
    /// decides who gets it — not arrival order, which the reader used to be
    /// the only thing guaranteeing.
    #[test]
    fn replies_are_matched_to_their_requests_by_id_not_by_order() {
        let root = tempfile::tempdir().unwrap();
        let (reader, writer, seen) = fake_server({
            let mut parked: Option<Value> = None;
            move |message, writer| match method(message) {
                // Hold the hover until the completion has been answered.
                "textDocument/hover" => {
                    parked = Some(message.clone());
                    true
                }
                "textDocument/completion" => {
                    reply(writer, message, json!({ "items": [{ "label": "later" }] }));
                    if let Some(hover) = parked.take() {
                        reply(writer, &hover, json!({ "contents": "the hover" }));
                    }
                    true
                }
                _ => default_handle(message, writer),
            }
        });
        let (client, _events) =
            LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
        client.did_open("a.rs", "fn a() {}\n").unwrap();

        thread::scope(|scope| {
            let hover = scope.spawn(|| client.hover("a.rs", 0, 3));
            assert!(saw(&seen, "textDocument/hover"), "{:?}", methods(&seen));
            let completion = client.completion("a.rs", 0, 3).expect("completion");
            assert_eq!(completion[0].label, "later");
            let hover = hover.join().unwrap().expect("hover").expect("some hover");
            assert_eq!(hover.text, "the hover");
        });
    }

    /// The server dies with a request outstanding. The caller must hear so
    /// at once — it used to wait the full fifteen-second budget to be told
    /// "timeout", which is the wrong answer as well as a slow one.
    #[test]
    fn a_server_that_dies_mid_request_fails_the_request_at_once() {
        let root = tempfile::tempdir().unwrap();
        let (reader, writer, _seen) = fake_server(|message, writer| {
            if method(message) == "textDocument/hover" {
                return false;
            }
            default_handle(message, writer)
        });
        let (client, events) =
            LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
        client.did_open("a.rs", "fn a() {}\n").unwrap();

        let started = Instant::now();
        let outcome = client.hover("a.rs", 0, 3);
        assert!(
            matches!(outcome, Err(Error::Exited { ref method }) if method == "textDocument/hover"),
            "{outcome:?}",
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}: the caller sat out the timeout",
            started.elapsed(),
        );
        // And every request after it fails the same way, immediately.
        assert!(matches!(
            client.completion("a.rs", 0, 3),
            Err(Error::Exited { .. })
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Some(LspEvent::Exited {}) => break,
                Some(_) => continue,
                None => panic!("the exit was never announced"),
            }
        }
    }

    /// Dropping the client ends the session properly — `shutdown`, `exit` —
    /// and, once the reader has seen the server go, closes the events stream.
    /// It used to leave the puller thread holding the session for ever, so
    /// the consumer blocked on `recv` never returned and leaked a thread per
    /// project switch. Also pins the wire order an open takes: the
    /// notification, then the pull it provokes — never the other way round.
    #[test]
    fn dropping_the_client_shuts_the_server_down_and_closes_the_events() {
        let root = tempfile::tempdir().unwrap();
        let (reader, writer, seen) = fake_server(default_handle);
        let (client, events) =
            LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
        client.did_open("a.rs", "fn a() {}\n").unwrap();

        // The open provokes a pull, and the pull's answer arrives as an event.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Some(LspEvent::Diagnostics { path, .. }) if path == "a.rs" => break,
                Some(_) => continue,
                None => panic!("no diagnostics for the opened file: {:?}", methods(&seen)),
            }
        }
        let order = methods(&seen);
        let opened = order
            .iter()
            .position(|m| m == "textDocument/didOpen")
            .unwrap();
        let pulled = order
            .iter()
            .position(|m| m == "textDocument/diagnostic")
            .unwrap();
        assert!(opened < pulled, "the pull overtook the open: {order:?}");

        drop(client);

        // `recv` on its own thread: `None` is the end, and a hang is the bug.
        let (ended_tx, ended_rx) = mpsc::channel();
        thread::spawn(move || {
            while events.recv().is_some() {}
            let _ = ended_tx.send(());
        });
        match ended_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => {
                panic!("the events stream never ended: {:?}", methods(&seen))
            }
            Err(RecvTimeoutError::Disconnected) => unreachable!(),
        }
        let order = methods(&seen);
        let shutdown = order.iter().position(|m| m == "shutdown");
        let exit = order.iter().position(|m| m == "exit");
        assert!(
            matches!((shutdown, exit), (Some(s), Some(e)) if s < e),
            "shutdown then exit, before any kill: {order:?}",
        );
    }

    /// A lazy action whose resolve fails, and nothing else to offer. That
    /// used to come back as an empty list — `Err(_) => continue` — so the
    /// menu was empty and the reason went nowhere.
    #[test]
    fn a_failed_resolve_is_reported_when_nothing_else_could_be_offered() {
        let root = tempfile::tempdir().unwrap();
        let (reader, writer, _seen) = fake_server(|message, writer| {
            match method(message) {
                "textDocument/codeAction" => reply(
                    writer,
                    message,
                    json!([{ "title": "Import HashMap", "kind": "quickfix" }]),
                ),
                "codeAction/resolve" => refuse(writer, message, "resolve exploded"),
                _ => return default_handle(message, writer),
            }
            true
        });
        let (client, _events) =
            LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
        client
            .did_open("a.rs", "let t = HashMap::new();\n")
            .unwrap();

        let outcome = client.code_actions("a.rs", 0, 9);
        assert!(
            matches!(
                outcome,
                Err(Error::Server { ref method, ref message })
                    if method == "codeAction/resolve" && message.contains("exploded")
            ),
            "{outcome:?}",
        );
    }

    /// A rename naming a file that cannot be read refuses the whole rename
    /// and writes nothing. It used to skip the file and report the rest as
    /// done — with a CJK directory, the *decoder* was what made the file
    /// unreadable, so every rename under one silently half-applied.
    #[test]
    fn a_rename_naming_an_unreadable_file_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        let main = root.path().join("src/main.rs");
        std::fs::write(&main, "fn radio() {}\n").unwrap();
        let readable = path_to_uri(&main);
        let missing = path_to_uri(&root.path().join("src/驱动/mod.rs"));

        let (reader, writer, _seen) = fake_server(move |message, writer| {
            if method(message) == "textDocument/rename" {
                let edit = json!({
                    "range": {
                        "start": { "line": 0, "character": 3 },
                        "end": { "line": 0, "character": 8 },
                    },
                    "newText": "tuner",
                });
                // Cloned per call: the handler may answer more than once,
                // and a key moved out of an `FnMut` cannot.
                let (readable, missing) = (readable.clone(), missing.clone());
                reply(
                    writer,
                    message,
                    json!({ "changes": { readable: [edit], missing: [edit] } }),
                );
                return true;
            }
            default_handle(message, writer)
        });
        let (client, _events) =
            LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
        client.did_open("src/main.rs", "fn radio() {}\n").unwrap();

        let outcome = client.rename("src/main.rs", 0, 4, "tuner");
        assert!(
            matches!(outcome, Err(Error::Apply { ref path, .. }) if path.contains("驱动")),
            "{outcome:?}",
        );
        assert_eq!(
            std::fs::read_to_string(&main).unwrap(),
            "fn radio() {}\n",
            "nothing may be written when part of the rename cannot be",
        );
    }
}
