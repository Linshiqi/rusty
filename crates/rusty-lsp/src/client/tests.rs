//! A fake rust-analyzer on the other end of two pipes.
//!
//! The unit tests elsewhere prove the arithmetic and the integration test
//! proves the real server; neither reaches the transport — correlation,
//! what happens when the server dies with a request outstanding, what a
//! drop does. These do, against a peer that speaks JSON-RPC over
//! `std::io::pipe` and answers only what each test needs.

use std::{
    io::{BufReader, PipeWriter, Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::LspClient;
use crate::{
    error::Error,
    model::{FileDiagnostic, HealthLevel, LspEvent},
    rpc,
    uri::path_to_uri,
    watched::FileChange,
};

/// Every message the fake server received, in order.
pub(super) type Seen = Arc<Mutex<Vec<Value>>>;

pub(super) fn method(message: &Value) -> &str {
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

/// A client over a fake server that answers `handle`'s requests, in a
/// project holding `files`, and every message the server received.
pub(super) fn client_with(
    files: &[(&str, &str)],
    handle: impl Fn(&Value, &Path) -> Option<Value> + Send + 'static,
) -> (LspClient, tempfile::TempDir, Seen) {
    let root = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let file = root.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let at = root.path().to_path_buf();
    let (reader, writer, seen) = fake_server(move |message, writer| {
        if let Some(result) = handle(message, &at) {
            reply(writer, message, result);
        }
        true
    });
    let (client, _events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
    (client, root, seen)
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

/// A long file is asked about a range of lines, and the range travels as
/// whole lines; with none, the whole document is asked for.
#[test]
fn semantic_tokens_are_asked_for_the_lines_named_or_the_whole_document() {
    let root = tempfile::tempdir().unwrap();
    let (reader, writer, seen) = fake_server(|message, writer| {
        if method(message).starts_with("textDocument/semanticTokens/") {
            reply(writer, message, json!({ "data": [] }));
            return true;
        }
        default_handle(message, writer)
    });
    let (client, _events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
    client.semantic_tokens("a.rs", Some((120, 900))).unwrap();
    client.semantic_tokens("a.rs", None).unwrap();
    let asked: Vec<(String, Value)> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|m| method(m).starts_with("textDocument/semanticTokens/"))
        .map(|m| (method(m).to_string(), m["params"]["range"].clone()))
        .collect();
    assert_eq!(
        asked,
        [
            (
                "textDocument/semanticTokens/range".to_string(),
                json!({ "start": { "line": 120, "character": 0 },
                        "end": { "line": 900, "character": 0 } })
            ),
            ("textDocument/semanticTokens/full".to_string(), Value::Null),
        ]
    );
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
        assert_eq!(completion.items[0].label, "later");
        let hover = hover.join().unwrap().expect("hover").expect("some hover");
        assert_eq!(hover.text, "the hover");
    });
}

/// A fix that edits another file is offered, names the file, and is
/// written there on request — the fix for a file no `mod` line declares
/// edits only the parent module, and used to be dropped whole.
#[test]
fn a_fix_for_another_file_is_offered_and_written_there() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    let parent = root.path().join("src/lib.rs");
    std::fs::write(&parent, "pub mod vector;\n").unwrap();
    let parent_uri = path_to_uri(&parent);
    let (reader, writer, _seen) = fake_server(move |message, writer| {
        if method(message) == "textDocument/codeAction" {
            let at = json!({ "line": 1, "character": 0 });
            let parent_uri = parent_uri.clone();
            reply(
                writer,
                message,
                json!([{
                    "title": "Insert `mod fresh;`",
                    "kind": "quickfix",
                    "edit": { "changes": { parent_uri: [{
                        "range": { "start": at, "end": at },
                        "newText": "pub mod fresh;\n",
                    }] } },
                }]),
            );
            return true;
        }
        default_handle(message, writer)
    });
    let (client, _events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
    client
        .did_open("src/fresh.rs", "pub struct Fresh;\n")
        .unwrap();

    let answer = client.code_actions("src/fresh.rs", 0, 0).expect("actions");
    assert_eq!(answer.fixes.len(), 1, "{answer:?}");
    assert!(answer.fixes[0].edits.is_empty(), "nothing in this file");
    assert_eq!(answer.fixes[0].elsewhere, ["src/lib.rs"]);

    let changed = client
        .apply_action_elsewhere("src/fresh.rs", answer.reply, 0)
        .expect("apply");
    assert_eq!(changed.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&parent).unwrap(),
        "pub mod vector;\npub mod fresh;\n"
    );
    // An index the answer never had writes nothing.
    assert!(
        client
            .apply_action_elsewhere("src/fresh.rs", answer.reply, 7)
            .unwrap()
            .is_empty()
    );
    // Nor does an answer number nobody issued. That is what keeps the
    // caret's popup and a hover over a squiggle from applying each
    // other's fixes: both ask now, and the last asker used to win.
    assert!(
        client
            .apply_action_elsewhere("src/fresh.rs", answer.reply + 99, 0)
            .unwrap()
            .is_empty()
    );
}

/// A server that could not load the workspace says so, and the session
/// passes it on. Without it, a rust-analyzer that can only parse looks
/// exactly like one that answers everything: the file still gets its
/// syntax errors, and every completion is empty for ever.
#[test]
fn the_servers_own_health_reaches_the_frontend() {
    let root = tempfile::tempdir().unwrap();
    let (reader, writer, _seen) = fake_server(|message, writer| {
        if method(message) == "initialized" {
            let _ = rpc::write_message(
                writer,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "experimental/serverStatus",
                    "params": {
                        "health": "error",
                        "quiescent": true,
                        "message": "cargo metadata failed",
                    },
                }),
            );
        }
        default_handle(message, writer)
    });
    let (_client, events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Some(LspEvent::Health { level, message }) => {
                assert_eq!(level, HealthLevel::Error);
                assert_eq!(message.as_deref(), Some("cargo metadata failed"));
                return;
            }
            Some(_) => continue,
            None => panic!("the health notification never arrived"),
        }
    }
}

/// An accepted item is resolved against the answer it came from, even
/// after newer ones — the popup asks on every keystroke — and never
/// against another answer's item at the same index.
#[test]
fn an_item_is_resolved_against_its_own_answer() {
    let root = tempfile::tempdir().unwrap();
    let (reader, writer, _seen) = fake_server(|message, writer| match method(message) {
        "textDocument/completion" => {
            let col = message["params"]["position"]["character"].as_u64().unwrap();
            let items = json!([{ "label": format!("at{col}"), "data": col }]);
            reply(
                writer,
                message,
                json!({ "isIncomplete": true, "items": items }),
            );
            true
        }
        "completionItem/resolve" => {
            let col = message["params"]["data"].as_u64().unwrap();
            let at = json!({ "line": 0, "character": 0 });
            reply(
                writer,
                message,
                json!({
                    "label": format!("at{col}"),
                    "additionalTextEdits": [{
                        "range": { "start": at, "end": at },
                        "newText": format!("use at{col};\n"),
                    }],
                }),
            );
            true
        }
        _ => default_handle(message, writer),
    });
    let (client, _events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
    client.did_open("a.rs", "fn a() {}\n").unwrap();

    let first = client.completion("a.rs", 0, 1).expect("first");
    let second = client.completion("a.rs", 0, 2).expect("second");
    assert!(first.incomplete, "the server said so");
    assert_ne!(first.reply, second.reply);
    let edits = client
        .resolve_completion("a.rs", first.reply, 0)
        .expect("resolve");
    assert_eq!(
        edits[0].new_text, "use at1;\n",
        "the first answer's own item"
    );
    assert!(
        client
            .resolve_completion("a.rs", second.reply + 100, 0)
            .expect("unknown answer")
            .is_empty()
    );
    assert!(
        client
            .resolve_completion("b.rs", second.reply, 0)
            .expect("another file")
            .is_empty()
    );
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

/// The client offers to watch the disk; rust-analyzer takes it up by
/// registering `didChangeWatchedFiles`; and only from then on does the
/// client send what changed. A server that never registers is watching
/// for itself and is told nothing twice.
#[test]
fn watched_files_are_sent_once_the_server_asks_and_not_before() {
    let root = tempfile::tempdir().unwrap();
    let (reader, writer, seen) = fake_server(|message, writer| {
        if method(message) == "textDocument/didOpen" {
            rpc::write_message(
                writer,
                &json!({ "jsonrpc": "2.0", "id": 90, "method": "client/registerCapability",
                         "params": { "registrations": [{
                             "id": "watch", "method": "workspace/didChangeWatchedFiles",
                             "registerOptions": { "watchers": [{ "globPattern": "**/*.rs" }] } }] } }),
            )
            .unwrap();
            return true;
        }
        default_handle(message, writer)
    });
    let (client, _events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
    let initialize = seen.lock().unwrap()[0].clone();
    assert_eq!(
        initialize["params"]["capabilities"]["workspace"]["didChangeWatchedFiles"]["dynamicRegistration"],
        json!(true)
    );

    let events = vec![("src/new.rs".to_string(), FileChange::Created)];
    client.did_change_watched_files(&events).unwrap();
    client.did_open("a.rs", "fn a() {}\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !seen
        .lock()
        .unwrap()
        .iter()
        .any(|m| m.get("id") == Some(&json!(90)) && m.get("method").is_none())
    {
        assert!(
            Instant::now() < deadline,
            "the registration went unanswered"
        );
        thread::sleep(Duration::from_millis(10));
    }
    client.did_change_watched_files(&events).unwrap();
    assert!(saw(&seen, "workspace/didChangeWatchedFiles"));

    let sent: Vec<Value> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|m| method(m) == "workspace/didChangeWatchedFiles")
        .cloned()
        .collect();
    assert_eq!(sent.len(), 1, "nothing was sent before the registration");
    let change = &sent[0]["params"]["changes"][0];
    assert_eq!(change["type"], json!(1));
    assert!(
        change["uri"].as_str().unwrap().ends_with("src/new.rs"),
        "{change}"
    );
}

/// A closed tab is a closed document: the server hears `didClose` once,
/// and the file's diagnostics drop the client's own analysis — pulled
/// for open documents only, so nothing would keep it current — while
/// what the check published stays.
#[test]
fn closing_a_document_tells_the_server_once_and_keeps_only_the_checks_findings() {
    let root = tempfile::tempdir().unwrap();
    let uri = path_to_uri(&root.path().join("a.rs"));
    let item = |message: &str| {
        json!({
            "range": { "start": { "line": 0, "character": 0 },
                       "end": { "line": 0, "character": 2 } },
            "severity": 1,
            "message": message,
        })
    };
    let (reader, writer, seen) = fake_server(move |message, writer| {
        match method(message) {
            "textDocument/didOpen" => {
                rpc::write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                             "params": { "uri": uri, "diagnostics": [item("from the check")] } }),
                )
                .unwrap();
            }
            "textDocument/diagnostic" => reply(
                writer,
                message,
                json!({ "kind": "full", "items": [item("from the analysis")] }),
            ),
            _ => {}
        }
        true
    });
    let (client, events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
    let messages = |items: &[FileDiagnostic]| -> Vec<String> {
        items.iter().map(|item| item.message.clone()).collect()
    };
    let next_for_a = |wanted: usize| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Some(LspEvent::Diagnostics { path, items })
                    if path == "a.rs" && items.len() == wanted =>
                {
                    break items;
                }
                Some(_) => continue,
                None => panic!("no diagnostics of {wanted} for a.rs: {:?}", methods(&seen)),
            }
        }
    };

    client.did_open("a.rs", "fn a() {}\n").unwrap();
    let both = next_for_a(2);
    assert_eq!(messages(&both).len(), 2, "{both:?}");

    client.did_close("a.rs").unwrap();
    let after = next_for_a(1);
    assert_eq!(messages(&after), vec!["from the check".to_string()]);
    assert!(saw(&seen, "textDocument/didClose"));

    client.did_close("a.rs").unwrap();
    client.did_save("a.rs").unwrap();
    assert!(saw(&seen, "textDocument/didSave"));
    let closes = methods(&seen)
        .iter()
        .filter(|m| *m == "textDocument/didClose")
        .count();
    assert_eq!(closes, 1, "a second close of a closed file says nothing");
}

/// The server's indexing arrives as `$/progress` and leaves as an event
/// the status bar can show — with the create request answered, since a
/// server whose request goes unanswered stalls.
#[test]
fn indexing_progress_becomes_an_event_and_ends_with_none() {
    let root = tempfile::tempdir().unwrap();
    let (reader, writer, seen) = fake_server(|message, writer| {
        if method(message) == "textDocument/didOpen" {
            let notify = |writer: &mut PipeWriter, value: Value| {
                rpc::write_message(writer, &value).unwrap();
            };
            notify(
                writer,
                json!({ "jsonrpc": "2.0", "id": 77, "method": "window/workDoneProgress/create",
                        "params": { "token": "rustAnalyzer/Indexing" } }),
            );
            notify(
                writer,
                json!({ "jsonrpc": "2.0", "method": "$/progress", "params": {
                    "token": "rustAnalyzer/Indexing",
                    "value": { "kind": "begin", "title": "Indexing", "percentage": 0 } } }),
            );
            notify(
                writer,
                json!({ "jsonrpc": "2.0", "method": "$/progress", "params": {
                    "token": "rustAnalyzer/Indexing",
                    "value": { "kind": "report", "message": "12/45 (esp-hal)", "percentage": 26 } } }),
            );
            notify(
                writer,
                json!({ "jsonrpc": "2.0", "method": "$/progress", "params": {
                    "token": "rustAnalyzer/Indexing", "value": { "kind": "end" } } }),
            );
            return true;
        }
        default_handle(message, writer)
    });
    let (client, events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
    client.did_open("a.rs", "fn a() {}\n").unwrap();

    let mut texts = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while texts.len() < 3 {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Some(LspEvent::Progress { text }) => texts.push(text),
            Some(_) => continue,
            None => panic!("progress never arrived: {texts:?}"),
        }
    }
    assert_eq!(
        texts,
        vec![
            Some("Indexing 0%".to_string()),
            Some("Indexing 26% 12/45 (esp-hal)".to_string()),
            None,
        ]
    );
    // The create request was answered, by id.
    let deadline = Instant::now() + Duration::from_secs(5);
    let answered = loop {
        let done = seen
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.get("id") == Some(&json!(77)) && m.get("method").is_none());
        if done || Instant::now() > deadline {
            break done;
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert!(answered, "window/workDoneProgress/create went unanswered");
}

/// The server asking for its inlay hints and colours to be asked for
/// again is answered, and becomes an event the editor acts on.
#[test]
fn a_refresh_from_the_server_is_answered_and_passed_on() {
    let root = tempfile::tempdir().unwrap();
    let (reader, writer, seen) = fake_server(|message, writer| {
        if method(message) == "textDocument/didOpen" {
            rpc::write_message(
                writer,
                &json!({ "jsonrpc": "2.0", "id": 81, "method": "workspace/inlayHint/refresh" }),
            )
            .unwrap();
            return true;
        }
        default_handle(message, writer)
    });
    let (client, events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
    client.did_open("a.rs", "fn a() {}\n").unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Some(LspEvent::Refresh {}) => break,
            Some(_) => continue,
            None => panic!("the refresh never became an event"),
        }
    }
    // The answer reaches the fake server on its own thread.
    let answered = loop {
        let done = seen
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.get("id") == Some(&json!(81)) && m.get("method").is_none());
        if done || Instant::now() > deadline {
            break done;
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert!(answered, "workspace/inlayHint/refresh went unanswered");
}

/// Every request the server makes is answered, or it waits for ever:
/// `workspace/configuration` with one null per item it asked about, a
/// request this client has nothing to say to with a null, and a
/// diagnostic refresh with a null and then a fresh pull of every open
/// document — the moment the push model used to wipe them instead.
#[test]
fn every_request_from_the_server_is_answered() {
    let root = tempfile::tempdir().unwrap();
    let (reader, writer, seen) = fake_server({
        let mut pulls = 0;
        move |message, writer| {
            if method(message) != "textDocument/diagnostic" {
                return default_handle(message, writer);
            }
            pulls += 1;
            reply(writer, message, json!({ "kind": "full", "items": [] }));
            if pulls == 1 {
                for request in [
                    json!({ "jsonrpc": "2.0", "id": 61, "method": "workspace/configuration",
                            "params": { "items": [{ "section": "a" }, { "section": "b" }] } }),
                    json!({ "jsonrpc": "2.0", "id": 62, "method": "experimental/unheardOf" }),
                    json!({ "jsonrpc": "2.0", "id": 63, "method": "workspace/diagnostic/refresh" }),
                ] {
                    rpc::write_message(writer, &request).unwrap();
                }
            }
            true
        }
    });
    let (client, _events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
    client.did_open("a.rs", "fn a() {}\n").unwrap();

    let answer = |id: u64| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let found = seen
                .lock()
                .unwrap()
                .iter()
                .find(|m| m.get("id") == Some(&json!(id)) && m.get("method").is_none())
                .map(|m| m["result"].clone());
            if let Some(result) = found {
                break result;
            }
            assert!(Instant::now() < deadline, "request {id} went unanswered");
            thread::sleep(Duration::from_millis(10));
        }
    };
    assert_eq!(answer(61), json!([null, null]));
    assert_eq!(answer(62), Value::Null);
    assert_eq!(answer(63), Value::Null);
    let deadline = Instant::now() + Duration::from_secs(5);
    while methods(&seen)
        .iter()
        .filter(|m| *m == "textDocument/diagnostic")
        .count()
        < 2
    {
        assert!(
            Instant::now() < deadline,
            "the refresh pulled nothing: {:?}",
            methods(&seen)
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Settling after a load asks the editor to ask again, as a refresh
/// does: what it asked for while the server loaded came back empty, and
/// rust-analyzer's own refresh had come before the load was done. Once
/// per settling — a server that stays quiescent says so on every change
/// of its status and must not set off a round of asks each time.
#[test]
fn settling_after_a_load_asks_for_the_hints_again() {
    let root = tempfile::tempdir().unwrap();
    let status = |quiescent: bool| {
        json!({
            "jsonrpc": "2.0",
            "method": "experimental/serverStatus",
            "params": { "health": "ok", "quiescent": quiescent },
        })
    };
    let (reader, writer, _seen) = fake_server(move |message, writer| {
        if method(message) == "initialized" {
            for quiescent in [false, true, true] {
                let _ = rpc::write_message(writer, &status(quiescent));
            }
        }
        default_handle(message, writer)
    });
    let (_client, events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut refreshes = 0;
    while let Some(event) = events.recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        if matches!(event, LspEvent::Refresh {}) {
            refreshes += 1;
        }
    }
    assert_eq!(refreshes, 1, "one settling, one ask");
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

/// A rename is asked the way every positional request is — the document
/// and the position, the position in the server's units — with the new
/// name beside them. After a `中` the editor's column and the server's
/// differ, so a column passed through unconverted cannot pass.
#[test]
fn a_rename_is_asked_with_its_position_and_the_new_name() {
    let root = tempfile::tempdir().unwrap();
    let (reader, writer, seen) = fake_server(|message, writer| {
        if method(message) == "textDocument/rename" {
            reply(writer, message, json!({ "changes": {} }));
            return true;
        }
        default_handle(message, writer)
    });
    let (client, _events) =
        LspClient::connect(reader, writer, None, root.path(), None).expect("handshake");
    client
        .did_open("src/main.rs", "fn 中() { radio(); }\n")
        .unwrap();

    let changed = client.rename("src/main.rs", 0, 9, "tuner").expect("rename");
    assert!(changed.is_empty(), "an empty answer changes nothing");
    let asked = seen
        .lock()
        .unwrap()
        .iter()
        .find(|m| method(m) == "textDocument/rename")
        .map(|m| m["params"].clone())
        .expect("a rename was asked");
    assert_eq!(
        asked["textDocument"]["uri"],
        json!(path_to_uri(&root.path().join("src/main.rs")))
    );
    assert_eq!(
        asked["position"],
        json!({ "line": 0, "character": 11 }),
        "scalar 9 is byte 11 after the 中"
    );
    assert_eq!(asked["newName"], json!("tuner"));
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

/// A rename's answer that turns `radio` into `tuner` in each of `files`,
/// in that order: `documentChanges` is a list, where `changes` is a map
/// and keeps whatever order the map does.
fn radio_renamed_in(root: &Path, files: &[&str]) -> Value {
    let edit = json!({
        "range": {
            "start": { "line": 0, "character": 3 },
            "end": { "line": 0, "character": 8 },
        },
        "newText": "tuner",
    });
    let changes: Vec<Value> = files
        .iter()
        .map(|file| {
            json!({
                "textDocument": { "uri": path_to_uri(&root.join(file)), "version": null },
                "edits": [edit.clone()],
            })
        })
        .collect();
    json!({ "documentChanges": changes })
}

/// Every file of a rename is read before any is written, so when the
/// second cannot be read the first is not written and put back — it is
/// never written at all, and there is nothing to undo.
#[test]
fn a_rename_whose_second_file_cannot_be_read_leaves_the_first_alone() {
    let (client, root, _seen) =
        client_with(&[("src/main.rs", "fn radio() {}\n")], |message, root| {
            (method(message) == "textDocument/rename")
                .then(|| radio_renamed_in(root, &["src/main.rs", "src/tuner.rs"]))
        });
    client.did_open("src/main.rs", "fn radio() {}\n").unwrap();

    match client.rename("src/main.rs", 0, 4, "tuner") {
        Err(Error::Apply { path, undone, .. }) => {
            assert!(path.ends_with("src/tuner.rs"), "{path}");
            assert!(undone.is_empty(), "nothing was written: {undone:?}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(root.path().join("src/main.rs")).unwrap(),
        "fn radio() {}\n",
    );
}

/// A rename whose second file cannot be written puts the first back, and
/// says so: the edit lands whole or not at all. It used to stop at the
/// failure with the first file renamed and an error saying nothing had
/// been applied. The second file is read-only, which refuses the write on
/// every platform — to anyone but a superuser, who is told why the test
/// does not run.
#[test]
fn a_rename_whose_second_file_cannot_be_written_puts_the_first_back() {
    let (client, root, _seen) = client_with(
        &[
            ("src/main.rs", "fn radio() {}\n"),
            ("src/tuner.rs", "fn radio() {}\n"),
        ],
        |message, root| {
            (method(message) == "textDocument/rename")
                .then(|| radio_renamed_in(root, &["src/main.rs", "src/tuner.rs"]))
        },
    );
    let main = root.path().join("src/main.rs");
    let tuner = root.path().join("src/tuner.rs");
    let mut permissions = std::fs::metadata(&tuner).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&tuner, permissions).unwrap();
    if std::fs::OpenOptions::new().write(true).open(&tuner).is_ok() {
        eprintln!("skipped: this user can write a read-only file, so no write can be made to fail");
        return;
    }
    client.did_open("src/main.rs", "fn radio() {}\n").unwrap();

    let outcome = client.rename("src/main.rs", 0, 4, "tuner");
    match &outcome {
        Err(Error::Apply { path, undone, .. }) => {
            assert!(path.ends_with("src/tuner.rs"), "{path}");
            assert_eq!(undone.len(), 1, "{undone:?}");
            assert!(undone[0].ends_with("src/main.rs"), "{undone:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        outcome
            .unwrap_err()
            .to_string()
            .ends_with("what had been written was put back, so nothing was changed"),
    );
    for file in [&main, &tuner] {
        assert_eq!(
            std::fs::read_to_string(file).unwrap(),
            "fn radio() {}\n",
            "{}",
            file.display()
        );
    }
}
