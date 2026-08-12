//! The rust-analyzer session.
//!
//! One server per project, spoken to over stdio. The shape mirrors the
//! terminal's: a reader thread feeds an event channel, requests are correlated
//! by id, and the whole thing dies with the child process.
//!
//! Two pieces of embedded-specific knowledge live here, because getting either
//! wrong looks like "rust-analyzer is broken" rather than like a setting:
//!
//! - The `rust-analyzer` on PATH is usually rustup's proxy, which dispatches by
//!   the project's pinned toolchain — and an ESP project pins `esp`, which has
//!   no rust-analyzer component, so the proxy fails *precisely for the projects
//!   this workbench serves*. The stable toolchain's real binary is resolved
//!   first instead; it analyses any toolchain's project fine, and reads the
//!   pinned toolchain's own sysroot for the target's `core`.
//! - `check.allTargets` defaults to on, which builds tests and benches. A
//!   `no_std` firmware has no test harness, so that default drowns every real
//!   diagnostic in "can't find crate for `test`". It is turned off.
//! - Diagnostics are **pulled** (LSP 3.17), not just received. After the
//!   build-data workspace switch, r-a never recomputes pushed diagnostics for
//!   open files — they wipe and stay gone. Under pull, it asks the client to
//!   re-request instead, and the puller thread owns freshness.

use std::{
    collections::HashMap,
    io::{BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicI64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use serde_json::{Value, json};

use crate::{
    error::{Error, Result},
    model::{
        CompletionItem, DiagSeverity, EditRange, FileDiagnostic, HoverInfo, Location,
        LspEvent, SemanticSpan, SignatureInfo,
    },
    positions::{
        Encoding, character_to_scalar, content_change, scalar_to_character,
    },
    rpc,
};

/// How long a request may take before the caller is told rather than kept
/// waiting. Generous because a cold index answers slowly; callers that want to
/// retry — completion during startup — retry above this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// A running rust-analyzer, and the documents it has been shown.
pub struct LspClient {
    shared: Arc<Shared>,
    child: Mutex<Child>,
}

/// Diagnostics and lifecycle, for exactly one consumer.
pub struct Events {
    rx: Receiver<LspEvent>,
}

impl Events {
    pub fn recv(&self) -> Option<LspEvent> {
        self.rx.recv().ok()
    }

    pub fn recv_timeout(&self, within: Duration) -> Option<LspEvent> {
        self.rx.recv_timeout(within).ok()
    }
}

struct Doc {
    version: i64,
    text: String,
}

struct Shared {
    writer: Mutex<Box<dyn Write + Send>>,
    /// Wake the puller for a path. Requests cannot be made from the reader
    /// thread — it would wait on a reply only itself can read — so refreshes
    /// hop threads through this.
    poke: Mutex<Option<Sender<String>>>,
    pending: Mutex<HashMap<i64, Sender<Value>>>,
    docs: Mutex<HashMap<String, Doc>>,
    next_id: AtomicI64,
    /// Set once the handshake has read what the server picked. Diagnostics
    /// only arrive after `initialized`, so the default is never actually used.
    encoding: OnceLock<Encoding>,
    semantic_legend: OnceLock<Vec<String>>,
    root: PathBuf,
    events: Sender<LspEvent>,
}

impl LspClient {
    /// Start rust-analyzer for the project at `root`.
    ///
    /// `target` is the triple the firmware builds for, when the caller knows
    /// it — detection does — so cfg resolution matches the chip rather than
    /// the host.
    pub fn spawn(root: &Path, target: Option<&str>) -> Result<(LspClient, Events)> {
        let binary = find_rust_analyzer().ok_or(Error::NotFound)?;

        let mut command = Command::new(&binary);
        command
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // rust-analyzer narrates progress on stderr. Piped-but-undrained
            // would fill the pipe and deadlock the server mid-index, so it is
            // discarded — except when someone is diagnosing "no diagnostics",
            // which is exactly when the server's own complaints are the answer.
            .stderr(if std::env::var_os("RUSTY_LSP_LOG").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            });
        no_console_window(&mut command);

        let mut child = command.spawn().map_err(Error::Spawn)?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");

        let (events_tx, events_rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            writer: Mutex::new(Box::new(stdin) as Box<dyn Write + Send>),
            poke: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            docs: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(1),
            encoding: OnceLock::new(),
        semantic_legend: OnceLock::new(),
            root: root.to_path_buf(),
            events: events_tx,
        });
        pump(stdout, Arc::clone(&shared));

        // The handshake, before anyone else gets the client. A failure here
        // must kill the child by hand — no `LspClient` exists yet to do it on
        // drop, and a leaked rust-analyzer holds the project's target dir open.
        if let Err(e) = handshake(&shared, root, target) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }

        let (poke_tx, poke_rx) = mpsc::channel();
        *shared.poke.lock().expect("lsp poke") = Some(poke_tx);
        pull_loop(poke_rx, Arc::clone(&shared));

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
        self.shared.poke_pull(path);
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
        )
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

        self.shared.poke_pull(path);
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
        )
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

        // The reply is CompletionItem[] or a CompletionList; both hold items.
        let items = result
            .get("items")
            .and_then(Value::as_array)
            .or_else(|| result.as_array())
            .cloned()
            .unwrap_or_default();

        let encoding = self.shared.encoding();
        let docs = self.shared.docs.lock().expect("lsp docs");
        let text = docs.get(path).map(|d| d.text.as_str()).unwrap_or("");

        // A hundred is more than any popup shows and keeps a `use`-everything
        // completion reply from shipping megabytes over the bridge.
        Ok(items.iter().take(100).map(|item| {
            let label = item["label"].as_str().unwrap_or_default().to_string();
            let edit = item["textEdit"].as_object();
            let insert = edit
                .and_then(|e| e.get("newText"))
                .or_else(|| item.get("insertText"))
                .and_then(Value::as_str)
                .unwrap_or(&label)
                .to_string();
            let range = edit
                .and_then(|e| e.get("range"))
                .and_then(|range| {
                    let scalar = |position: &Value| -> Option<(u32, u32)> {
                        let line = position["line"].as_u64()? as u32;
                        let character = position["character"].as_u64()? as u32;
                        let line_text = text.split('\n').nth(line as usize)?;
                        Some((line, character_to_scalar(line_text, character, encoding)))
                    };
                    let start = scalar(&range["start"])?;
                    let end = scalar(&range["end"])?;
                    Some(EditRange {
                        start_line: start.0,
                        start_col: start.1,
                        end_line: end.0,
                        end_col: end.1,
                    })
                });

            CompletionItem {
                label,
                kind: item["kind"].as_u64().map(kind_name).map(str::to_string),
                detail: item["detail"].as_str().map(str::to_string),
                insert,
                edit: range,
            }
        }).collect())
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

        // contents: MarkupContent | MarkedString | MarkedString[].
        let contents = &result["contents"];
        let text = contents["value"]
            .as_str()
            .map(str::to_string)
            .or_else(|| contents.as_str().map(str::to_string))
            .or_else(|| {
                contents.as_array().map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p.as_str().or_else(|| p["value"].as_str()))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            })
            .filter(|t| !t.is_empty());

        let range = result.get("range").and_then(|range| {
            let encoding = self.shared.encoding();
            let docs = self.shared.docs.lock().expect("lsp docs");
            let text = docs.get(path)?.text.as_str();
            let scalar = |position: &Value| -> Option<(u32, u32)> {
                let line = position["line"].as_u64()? as u32;
                let character = position["character"].as_u64()? as u32;
                let line_text = text.split('\n').nth(line as usize)?;
                Some((line, character_to_scalar(line_text, character, encoding)))
            };
            let start = scalar(&range["start"])?;
            let end = scalar(&range["end"])?;
            Some(EditRange {
                start_line: start.0,
                start_col: start.1,
                end_line: end.0,
                end_col: end.1,
            })
        });

        Ok(text.map(|text| HoverInfo { text, range }))
    }

    /// The signature of the call around this position, if the caret is inside
    /// one.
    ///
    /// The active parameter comes back as a byte range into the label. The
    /// protocol sends it either as a substring or as offsets — and the offsets
    /// are UTF-16 code units *regardless of the negotiated position encoding*,
    /// which only governs document positions. Both forms are resolved here.
    pub fn signature_help(&self, path: &str, line: u32, col: u32) -> Result<Option<SignatureInfo>> {
        let position = self.protocol_position(path, line, col);
        let result = self.shared.request(
            "textDocument/signatureHelp",
            json!({
                "textDocument": { "uri": self.uri(path) },
                "position": position,
            }),
        )?;

        let signatures = result["signatures"].as_array().cloned().unwrap_or_default();
        let active = result["activeSignature"].as_u64().unwrap_or(0) as usize;
        let Some(signature) = signatures.get(active).or_else(|| signatures.first()) else {
            return Ok(None);
        };
        let label = signature["label"].as_str().unwrap_or_default().to_string();
        if label.is_empty() {
            return Ok(None);
        }

        // Per-signature wins over top-level, as the 3.16 spec added.
        let active_param = signature
            .get("activeParameter")
            .and_then(Value::as_u64)
            .or_else(|| result.get("activeParameter").and_then(Value::as_u64));

        let span = active_param
            .and_then(|index| signature["parameters"].as_array()?.get(index as usize).cloned())
            .and_then(|parameter| match &parameter["label"] {
                // A substring of the label. `find` is what the spec intends;
                // a parameter text that appears twice in one signature would
                // have been sent as offsets.
                Value::String(text) => {
                    let start = label.find(text.as_str())?;
                    Some((start, start + text.len()))
                }
                Value::Array(pair) => {
                    let start = pair.first()?.as_u64()? as usize;
                    let end = pair.get(1)?.as_u64()? as usize;
                    Some((utf16_to_byte(&label, start), utf16_to_byte(&label, end)))
                }
                _ => None,
            });

        let doc = signature.get("documentation").and_then(|doc| {
            doc.as_str()
                .map(str::to_string)
                .or_else(|| doc["value"].as_str().map(str::to_string))
        });

        Ok(Some(SignatureInfo {
            label,
            param_start: span.map(|(start, _)| start as u32),
            param_end: span.map(|(_, end)| end as u32),
            doc,
        }))
    }

    /// The whole document's semantic colouring, as the server sees it.
    ///
    /// The reply is quintuples of u32 — deltaLine, deltaStart, length, type
    /// index, modifier bits — relative-encoded, in the negotiated position
    /// encoding. Decoded here to absolute lines and Unicode-scalar columns,
    /// with the type index resolved against the server's legend, so the
    /// frontend sees names and scalars and nothing of the format.
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

        let legend = self.shared.legend();
        let encoding = self.shared.encoding();
        let docs = self.shared.docs.lock().expect("lsp docs");
        let Some(doc) = docs.get(path) else {
            return Ok(Vec::new());
        };
        let lines: Vec<&str> = doc.text.split('\n').collect();

        let mut spans = Vec::with_capacity(data.len() / 5);
        let mut line = 0u32;
        let mut unit_col = 0u32;
        for token in data.chunks_exact(5) {
            let (delta_line, delta_start, unit_len, type_index) =
                (token[0], token[1], token[2], token[3]);
            if delta_line > 0 {
                line += delta_line;
                unit_col = delta_start;
            } else {
                unit_col += delta_start;
            }
            let Some(kind) = legend.get(type_index as usize) else {
                continue;
            };
            let Some(line_text) = lines.get(line as usize) else {
                continue;
            };
            let start = character_to_scalar(line_text, unit_col, encoding);
            let end = character_to_scalar(line_text, unit_col + unit_len, encoding);
            if end <= start {
                continue;
            }
            spans.push(SemanticSpan {
                line,
                start_col: start,
                length: end - start,
                kind: kind.clone(),
            });
        }
        Ok(spans)
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
            .and_then(|text| {
                let line_text = text.split('\n').nth(line as usize)?;
                Some(character_to_scalar(
                    line_text,
                    character,
                    self.shared.encoding(),
                ))
            })
            .unwrap_or(character);
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
        let docs = self.shared.docs.lock().expect("lsp docs");
        let from_docs = docs
            .get(path)
            .and_then(|doc| doc.text.split('\n').nth(line as usize))
            .map(|line_text| character_to_scalar(line_text, character, encoding));
        drop(docs);
        from_docs
            .or_else(|| {
                let text = std::fs::read_to_string(self.shared.root.join(path)).ok()?;
                let line_text = text.split('\n').nth(line as usize)?;
                Some(character_to_scalar(line_text, character, encoding))
            })
            .unwrap_or(character)
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Shared {
    fn poke_pull(&self, path: &str) {
        if let Some(poke) = self.poke.lock().expect("lsp poke").as_ref() {
            let _ = poke.send(path.to_string());
        }
    }

    fn poke_all_open(&self) {
        let open: Vec<String> = self.docs.lock().expect("lsp docs").keys().cloned().collect();
        for path in open {
            self.poke_pull(&path);
        }
    }

    fn encoding(&self) -> Encoding {
        *self.encoding.get().unwrap_or(&Encoding::Utf16)
    }

    /// The token-type legend the server declared at initialize; indexes in
    /// every semantic-tokens response point into this.
    fn legend(&self) -> Vec<String> {
        self.semantic_legend.get().cloned().unwrap_or_default()
    }

    fn write(&self, message: &Value) -> Result<()> {
        let mut writer = self.writer.lock().expect("lsp writer");
        rpc::write_message(&mut **writer, message).map_err(Error::Io)
    }

    fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn respond(&self, id: Value, result: Value) -> Result<()> {
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().expect("lsp pending").insert(id, tx);

        self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;

        match rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(response) => {
                if let Some(error) = response.get("error") {
                    Err(Error::Server {
                        method: method.to_string(),
                        message: error["message"].as_str().unwrap_or("unknown error").to_string(),
                    })
                } else {
                    Ok(response.get("result").cloned().unwrap_or(Value::Null))
                }
            }
            Err(_) => {
                self.pending.lock().expect("lsp pending").remove(&id);
                Err(Error::Timeout {
                    method: method.to_string(),
                })
            }
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
        "initializationOptions": {
            "cargo": Value::Object(cargo),
            "check": { "allTargets": false },
            // Off, deliberately. Embedded projects here use `build-std`, and
            // `cargo check` under build-std emits messages for packages that
            // are not in `cargo metadata` — rust-analyzer logs an error storm
            // and, when the run completes, publishes empty diagnostics that
            // wipe the native ones. Observed as: squiggles appear for a few
            // seconds, then vanish. Native diagnostics — type errors,
            // unresolved names — are the ones the editor needs live anyway.
            "checkOnSave": false,
        },
    });

    let reply = shared.request("initialize", params)?;
    let encoding = match reply["capabilities"]["positionEncoding"].as_str() {
        Some("utf-8") => Encoding::Utf8,
        _ => Encoding::Utf16,
    };
    let _ = shared.encoding.set(encoding);

    let legend: Vec<String> = reply["capabilities"]["semanticTokensProvider"]["legend"]
        ["tokenTypes"]
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
fn pump(stdout: ChildStdout, shared: Arc<Shared>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        // An Err is treated as EOF: a mangled frame means the stream has
        // drifted and every later byte would misparse anyway.
        while let Ok(Some(message)) = rpc::read_message(&mut reader) {
            dispatch(&shared, message);
        }
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
                let asked = message["params"]["items"]
                    .as_array()
                    .map_or(0, Vec::len);
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
                let _ = waiter.send(message);
            }
        }
        (None, Some(method)) if method == "textDocument/publishDiagnostics" => {
            publish(shared, &message["params"]);
        }
        // Progress, logs, show-message: narration, not state.
        _ => {}
    }
}

/// Pull diagnostics for pokes, forever, coalescing bursts.
///
/// Retries while the server is busy: a pull during indexing answers with
/// "content modified" or blocks, and both mean "later", not "never".
fn pull_loop(poke: mpsc::Receiver<String>, shared: Arc<Shared>) {
    thread::spawn(move || {
        while let Ok(first) = poke.recv() {
            // Typing produces a poke per pulse; only the newest matters.
            let mut wanted = vec![first];
            while let Ok(more) = poke.try_recv() {
                if !wanted.contains(&more) {
                    wanted.push(more);
                }
            }
            for path in wanted {
                for attempt in 0..10 {
                    match pull(&shared, &path) {
                        Ok(items) => {
                            let _ = shared.events.send(LspEvent::Diagnostics {
                                path: path.clone(),
                                items,
                            });
                            break;
                        }
                        Err(_) if attempt < 9 => {
                            thread::sleep(Duration::from_millis(600));
                        }
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
    let items = report.get("items").cloned().unwrap_or(Value::Array(Vec::new()));
    Ok(convert_items(shared, path, &items))
}

/// Convert one publishDiagnostics into the wire model and emit it.
fn publish(shared: &Shared, params: &Value) {
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
    let items = convert_items(shared, &path, &params["diagnostics"]);
    if items.is_empty() && shared.docs.lock().expect("lsp docs").contains_key(&path) {
        shared.poke_pull(&path);
        return;
    }

    let _ = shared.events.send(LspEvent::Diagnostics { path, items });
}

/// LSP diagnostics as the wire model, columns already scalar.
fn convert_items(shared: &Shared, path: &str, diagnostics: &Value) -> Vec<FileDiagnostic> {
    let encoding = shared.encoding();
    let text = shared
        .docs
        .lock()
        .expect("lsp docs")
        .get(path)
        .map(|doc| doc.text.clone())
        .or_else(|| std::fs::read_to_string(shared.root.join(path)).ok());

    let scalar = |line: u32, character: u32| -> u32 {
        text.as_deref()
            .and_then(|t| t.split('\n').nth(line as usize))
            .map(|line_text| character_to_scalar(line_text, character, encoding))
            .unwrap_or(character)
    };

    let mut items: Vec<FileDiagnostic> = diagnostics
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let range = &d["range"];
            let start_line = range["start"]["line"].as_u64()? as u32;
            let end_line = range["end"]["line"].as_u64()? as u32;
            Some(FileDiagnostic {
                severity: match d["severity"].as_u64() {
                    Some(2) => DiagSeverity::Warning,
                    Some(3) => DiagSeverity::Info,
                    Some(4) => DiagSeverity::Hint,
                    // Absent means the producer did not say; rustc's errors
                    // always do, so unmarked ones are treated as the worst.
                    _ => DiagSeverity::Error,
                },
                message: d["message"].as_str().unwrap_or_default().to_string(),
                source: d["source"].as_str().map(str::to_string),
                code: match &d["code"] {
                    Value::String(code) => Some(code.clone()),
                    Value::Number(code) => Some(code.to_string()),
                    _ => None,
                },
                start_line,
                start_col: scalar(start_line, range["start"]["character"].as_u64()? as u32),
                end_line,
                end_col: scalar(end_line, range["end"]["character"].as_u64()? as u32),
            })
        })
        .collect();
    items.sort_by_key(|d| (d.start_line, d.start_col, d.severity));
    items
}

/// Where rust-analyzer actually is.
///
/// The bare name on PATH is rustup's proxy, which dispatches by the pinned
/// toolchain — and `rust-toolchain.toml` pinning `esp` (every Xtensa project)
/// makes the proxy fail with "unknown binary in toolchain 'esp'". So: stable's
/// real binary first, the active toolchain's second, PATH last.
pub fn find_rust_analyzer() -> Option<PathBuf> {
    for toolchain in [Some("stable"), None] {
        let mut command = Command::new("rustup");
        command.arg("which");
        if let Some(toolchain) = toolchain {
            command.args(["--toolchain", toolchain]);
        }
        command.arg("rust-analyzer");
        no_console_window(&mut command);
        if let Ok(out) = command.output()
            && out.status.success()
        {
            let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if path.is_file() {
                return Some(path);
            }
        }
    }

    // No rustup: take PATH literally.
    let path = std::env::var_os("PATH")?;
    let name = if cfg!(windows) {
        "rust-analyzer.exe"
    } else {
        "rust-analyzer"
    };
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// `E:\x y\src` → `file:///E:/x%20y/src`.
fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for ch in text.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' | ':' => uri.push(ch),
            _ => {
                let mut buffer = [0u8; 4];
                for byte in ch.encode_utf8(&mut buffer).bytes() {
                    uri.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    uri
}

/// A `file://` URI as an absolute native path.
fn uri_to_absolute(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let mut input = rest.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input.next()?;
            let low = input.next()?;
            let hex = |b: u8| (b as char).to_digit(16);
            bytes.push((hex(high)? * 16 + hex(low)?) as u8);
        } else {
            bytes.push(byte);
        }
    }
    let mut decoded = String::from_utf8(bytes).ok()?;
    if decoded.len() >= 3 && decoded.as_bytes()[0] == b'/' && decoded.as_bytes()[2] == b':' {
        decoded.remove(0);
    }
    Some(decoded)
}

/// A `file://` URI as a path relative to `root`, or `None` if it is elsewhere.
///
/// Tolerant on purpose: rust-analyzer sends `file:///e%3A/...` — lowercased
/// drive, percent-encoded colon — for the same file this side calls
/// `file:///E:/...`.
fn uri_to_relative(uri: &str, root: &Path) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;

    let mut bytes = Vec::with_capacity(rest.len());
    let mut input = rest.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input.next()?;
            let low = input.next()?;
            let hex = |b: u8| (b as char).to_digit(16);
            bytes.push((hex(high)? * 16 + hex(low)?) as u8);
        } else {
            bytes.push(byte);
        }
    }
    let mut decoded = String::from_utf8(bytes).ok()?;

    // `/E:/x` → `E:/x` on Windows.
    if decoded.len() >= 3 && decoded.as_bytes()[0] == b'/' && decoded.as_bytes()[2] == b':' {
        decoded.remove(0);
    }

    let root = root.to_string_lossy().replace('\\', "/");
    let (folded_full, folded_root) = if cfg!(windows) {
        (decoded.to_lowercase(), root.to_lowercase())
    } else {
        (decoded.clone(), root.clone())
    };
    if !folded_full.starts_with(&folded_root) {
        return None;
    }
    Some(decoded[root.len()..].trim_start_matches('/').to_string())
}

fn no_console_window(command: &mut Command) {
    // Same reason as everywhere else a process is spawned on Windows: without
    // this, every rust-analyzer start flashes a console window over the app.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

/// A UTF-16 offset into `text`, as a byte offset.
///
/// For `ParameterInformation.label` offsets only: those are UTF-16 by spec no
/// matter what position encoding was negotiated — the negotiation covers
/// document positions, not offsets into strings the server sent.
fn utf16_to_byte(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (byte, ch) in text.char_indices() {
        if seen >= units {
            return byte;
        }
        seen += ch.len_utf16();
    }
    text.len()
}

fn kind_name(kind: u64) -> &'static str {
    // The LSP CompletionItemKind table, named so the frontend never holds a
    // second copy of these numbers.
    match kind {
        1 => "text",
        2 => "method",
        3 => "function",
        4 => "constructor",
        5 => "field",
        6 => "variable",
        7 => "class",
        8 => "interface",
        9 => "module",
        10 => "property",
        11 => "unit",
        12 => "value",
        13 => "enum",
        14 => "keyword",
        15 => "snippet",
        16 => "color",
        17 => "file",
        18 => "reference",
        19 => "folder",
        20 => "enum member",
        21 => "constant",
        22 => "struct",
        23 => "event",
        24 => "operator",
        25 => "type parameter",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_uris_round_trip_through_rust_analyzers_spelling() {
        let root = Path::new(r"E:\CodeBase\proj");
        assert_eq!(path_to_uri(root), "file:///E:/CodeBase/proj");

        // The server's own spelling of a file under that root: lowercased
        // drive, colon percent-encoded.
        assert_eq!(
            uri_to_relative("file:///e%3A/CodeBase/proj/src/main.rs", root).as_deref(),
            Some("src/main.rs"),
        );
        assert_eq!(
            uri_to_relative("file:///E:/CodeBase/proj/src/main.rs", root).as_deref(),
            Some("src/main.rs"),
        );
        // A dependency's source is not in the project.
        assert_eq!(
            uri_to_relative("file:///E:/other/place/lib.rs", root),
            None,
        );
    }

    #[test]
    fn spaces_survive_the_uri() {
        let root = Path::new(r"E:\code base\p");
        let uri = path_to_uri(root);
        assert_eq!(uri, "file:///E:/code%20base/p");
        assert_eq!(
            uri_to_relative(&format!("{uri}/src/a.rs"), root).as_deref(),
            Some("src/a.rs"),
        );
    }
}
