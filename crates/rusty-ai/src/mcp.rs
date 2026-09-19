//! The workbench's tools for assistants that are not rusty's own, over the
//! Model Context Protocol.
//!
//! `rusty-cli mcp` serves them on stdin and stdout, so Claude Code, Cursor or
//! any other MCP client calls the same `project_status`, `memory_report` and
//! `read_file` the built-in assistant does. The registry is the one list: a
//! tool added there is served here with no second step, and answers here
//! exactly as it answers there — the same JSON, the same refusals.
//!
//! The transport is MCP's stdio: one JSON-RPC message per line, nothing on
//! stdout that is not one. Only the tools half of the protocol, and only what
//! a server has to answer — `initialize`, `ping`, `tools/list`, `tools/call`.
//! Every tool reads, so there is no permission to ask a client for and no
//! notification to send; a request this server does not know is `Method not
//! found`, which is how a client learns a capability is absent.

use std::{
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use rusty_core::Workspace;
use rusty_embed::catalog::Catalog;
use serde_json::{Value, json};

use crate::tools::{LazyWorkspace, ToolContext, ToolRegistry};

/// The protocol revisions this server speaks, newest first. The tools half —
/// the only half served — is the same in all three.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Answer every line of `input` on `output` until the input ends — which is
/// how a client says it is done.
pub fn serve(
    root: impl Into<PathBuf>,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    let mut server = Server::new(root);
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        // Bytes, not `lines()`: a line that is not UTF-8 is a message this
        // server cannot parse and says so, rather than the end of the session.
        let text = String::from_utf8_lossy(&line);
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(answer) = server.answer_line(text) {
            writeln!(output, "{answer}")?;
            output.flush()?;
        }
    }
}

/// One project's tools, and the Cargo workspace the last call loaded.
pub struct Server {
    registry: ToolRegistry,
    root: PathBuf,
    kept: Option<Kept>,
}

/// A workspace kept between calls, with what it was resolved from.
///
/// A client stays connected for a whole session while the project changes
/// under it, so the workspace is loaded again once a manifest or the lockfile
/// has moved — otherwise a dependency added an hour ago would be missing from
/// every answer about the graph, which is a plausible answer to the wrong
/// question.
struct Kept {
    workspace: Arc<Workspace>,
    stamp: Stamp,
}

/// Each file's size and modification time, or nothing where it is absent.
#[derive(PartialEq)]
struct Stamp(Vec<(PathBuf, Option<(u64, SystemTime)>)>);

impl Stamp {
    fn of(paths: Vec<PathBuf>) -> Self {
        Stamp(
            paths
                .into_iter()
                .map(|path| {
                    let seen = std::fs::metadata(&path)
                        .ok()
                        .and_then(|meta| Some((meta.len(), meta.modified().ok()?)));
                    (path, seen)
                })
                .collect(),
        )
    }

    fn still_holds(&self) -> bool {
        *self == Stamp::of(self.0.iter().map(|(path, _)| path.clone()).collect())
    }
}

/// The files a workspace's graph is resolved from: the root manifest, the
/// lockfile, and every member's manifest.
fn manifests(root: &Path, workspace: &Workspace) -> Vec<PathBuf> {
    let mut paths = vec![root.join("Cargo.toml"), root.join("Cargo.lock")];
    paths.extend(
        workspace
            .graph()
            .workspace()
            .iter()
            .map(|member| member.manifest_path().as_std_path().to_path_buf()),
    );
    paths.sort();
    paths.dedup();
    paths
}

impl Server {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            registry: ToolRegistry::served(),
            root: root.into(),
            kept: None,
        }
    }

    /// The answer to one line of input: a message, a batch of them, or
    /// something that is neither. `None` when nothing is owed — a
    /// notification wants no answer.
    pub fn answer_line(&mut self, line: &str) -> Option<String> {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(e) => {
                return Some(
                    failure(Value::Null, PARSE_ERROR, &format!("not JSON: {e}")).to_string(),
                );
            }
        };
        let answer = match message {
            Value::Array(batch) if batch.is_empty() => {
                Some(failure(Value::Null, INVALID_REQUEST, "an empty batch"))
            }
            Value::Array(batch) => {
                let answers: Vec<Value> =
                    batch.into_iter().filter_map(|m| self.answer(m)).collect();
                (!answers.is_empty()).then_some(Value::Array(answers))
            }
            message => self.answer(message),
        };
        answer.map(|answer| answer.to_string())
    }

    /// The answer to one message, or `None` for a notification or a
    /// response — this server sends no requests, so a response is to nothing.
    pub fn answer(&mut self, message: Value) -> Option<Value> {
        let Value::Object(message) = message else {
            return Some(failure(
                Value::Null,
                INVALID_REQUEST,
                "a message is a JSON object",
            ));
        };
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            let is_response = message.contains_key("result") || message.contains_key("error");
            return match id {
                Some(id) if !is_response => {
                    Some(failure(id, INVALID_REQUEST, "a request names its method"))
                }
                _ => None,
            };
        };
        // `notifications/initialized`, `notifications/cancelled`: nothing here
        // waits on either, since every call is answered before the next line
        // is read.
        let id = id?;
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(failure(id, INVALID_REQUEST, "`jsonrpc` must be \"2.0\""));
        }
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        Some(match self.dispatch(method, &params) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, reason)) => failure(id, code, &reason),
        })
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(self.initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": self.tools() })),
            "tools/call" => self.call(params),
            _ => Err((
                METHOD_NOT_FOUND,
                format!("no method `{method}`: this server offers tools and nothing else"),
            )),
        }
    }

    /// The handshake. The client's revision when this server speaks it, and
    /// otherwise the newest this server does — the client then decides
    /// whether it can carry on, which is the protocol's rule and not ours.
    fn initialize(&self, params: &Value) -> Value {
        let asked = params.get("protocolVersion").and_then(Value::as_str);
        let version = asked
            .filter(|asked| PROTOCOL_VERSIONS.contains(asked))
            .unwrap_or(PROTOCOL_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "rusty", "version": env!("CARGO_PKG_VERSION") },
            "instructions": format!(
                "rusty's analyses of the embedded Rust project at {}: which chip and \
                 toolchain it builds for and every mismatch between them, where a built \
                 firmware's bytes went, the dependency graph and what a feature costs, the \
                 chips and boards rusty knows, and the project's files as rusty's Files \
                 panel sees them. The answers are computed, not inferred from the files — \
                 prefer them to reading .cargo/config.toml or a linker error and guessing. \
                 Every tool reads; none writes.",
                self.root.display()
            ),
        })
    }

    fn tools(&self) -> Vec<Value> {
        self.registry
            .defs()
            .into_iter()
            .map(|def| {
                json!({
                    "name": def.name,
                    "description": def.description,
                    "inputSchema": def.input_schema,
                    "annotations": {
                        "readOnlyHint": !def.capabilities.needs_approval(),
                        "openWorldHint": def.capabilities.network,
                    },
                })
            })
            .collect()
    }

    /// A tool's answer, or its refusal as a result the model reads — the
    /// agent loop's rule: a bad argument is something the caller can fix on
    /// its next turn. Only a name that is no tool at all is a protocol error.
    fn call(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err((
                INVALID_PARAMS,
                "`name` is required and must be a string".into(),
            ));
        };
        if !self.registry.defs().iter().any(|def| def.name == name) {
            return Err((INVALID_PARAMS, format!("Unknown tool: {name}")));
        }
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(arguments) => arguments.clone(),
        };

        // Read for every call rather than once: a board added to the
        // project's `.rusty/` and a build finished a minute ago are both
        // things the next answer should know about.
        let catalog = Catalog::load(Some(&self.root));
        let firmware = newest_firmware(&self.root);
        let lazy = match self.kept.take() {
            Some(kept) if kept.stamp.still_holds() => {
                LazyWorkspace::holding(&self.root, kept.workspace)
            }
            _ => LazyWorkspace::new(&self.root),
        };
        let context = ToolContext {
            workspace: None,
            workspace_on_demand: Some(&lazy),
            root: Some(&self.root),
            firmware,
            catalog: Some(&catalog),
        };
        let outcome = self.registry.call(name, &arguments, &context);
        if let Some(workspace) = lazy.loaded() {
            let stamp = Stamp::of(manifests(&self.root, &workspace));
            self.kept = Some(Kept { workspace, stamp });
        }

        let (text, is_error) = match outcome {
            Ok(value) => (value.to_string(), false),
            Err(e) => (e.to_string(), true),
        };
        Ok(json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error,
        }))
    }
}

/// The newest firmware under the crate cargo builds the chip's image in,
/// which for the standard layout is the excluded firmware crate rather than
/// the directory the client opened.
fn newest_firmware(root: &Path) -> Option<PathBuf> {
    let firmware_root = rusty_embed::project::firmware_root(root);
    let configured = rusty_embed::project::detect(&firmware_root)
        .ok()
        .and_then(|project| project.configured_target);
    rusty_embed::firmware::newest(&firmware_root, configured.as_deref())
        .map(|firmware| PathBuf::from(firmware.path))
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"blinky\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            "fn main() {\n    // 中文\n}\n",
        )
        .unwrap();
        dir
    }

    fn ask(server: &mut Server, message: Value) -> Value {
        let line = server
            .answer_line(&message.to_string())
            .expect("a request is answered");
        assert!(!line.contains('\n'), "one message is one line: {line}");
        serde_json::from_str(&line).unwrap()
    }

    fn request(id: u64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn the_handshake_agrees_a_revision_both_sides_speak() {
        let dir = project();
        let mut server = Server::new(dir.path());
        let answer = ask(
            &mut server,
            request(
                1,
                "initialize",
                json!({ "protocolVersion": "2025-03-26", "capabilities": {} }),
            ),
        );
        let result = &answer["result"];
        assert_eq!(answer["id"], 1);
        assert_eq!(
            result["protocolVersion"], "2025-03-26",
            "a revision spoken here is echoed"
        );
        assert!(
            result["capabilities"]["tools"].is_object(),
            "tools are what is offered"
        );
        assert_eq!(result["serverInfo"]["name"], "rusty");

        let newer = ask(
            &mut server,
            request(2, "initialize", json!({ "protocolVersion": "2099-01-01" })),
        );
        assert_eq!(
            newer["result"]["protocolVersion"], PROTOCOL_VERSIONS[0],
            "one this server does not speak is answered with the newest it does"
        );
    }

    #[test]
    fn every_workbench_tool_is_listed_with_its_schema() {
        let dir = project();
        let mut server = Server::new(dir.path());
        let answer = ask(&mut server, request(1, "tools/list", json!({})));
        let listed = answer["result"]["tools"].as_array().unwrap();
        let registry = ToolRegistry::served().defs();
        assert_eq!(listed.len(), registry.len());
        for def in &registry {
            let tool = listed
                .iter()
                .find(|tool| tool["name"] == def.name)
                .unwrap_or_else(|| panic!("{} is served", def.name));
            assert_eq!(tool["inputSchema"], def.input_schema);
            assert_eq!(tool["inputSchema"]["type"], "object");
            // Only the tool that builds and boots the firmware may say it
            // does more than read; a client asks the user before it runs.
            let reads = def.name != "simulate";
            assert_eq!(
                tool["annotations"]["readOnlyHint"], reads,
                "{} reads: {reads}",
                def.name
            );
        }
        assert!(
            listed.iter().any(|tool| tool["name"] == "simulate"),
            "the simulator is served"
        );
    }

    #[test]
    fn a_call_answers_what_the_assistant_would_be_told() {
        let dir = project();
        let mut server = Server::new(dir.path());
        let answer = ask(
            &mut server,
            request(
                7,
                "tools/call",
                json!({ "name": "read_file", "arguments": { "path": "src/main.rs" } }),
            ),
        );
        let result = &answer["result"];
        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["type"], "text");
        let text = result["content"][0]["text"].as_str().unwrap();
        let expected = ToolRegistry::workbench()
            .call(
                "read_file",
                &json!({ "path": "src/main.rs" }),
                &ToolContext {
                    workspace: None,
                    workspace_on_demand: None,
                    root: Some(dir.path()),
                    firmware: None,
                    catalog: None,
                },
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(text).unwrap(), expected);
        assert!(
            text.contains("中文"),
            "the file's text is in the answer: {text}"
        );
    }

    #[test]
    fn a_refusal_is_a_result_the_model_reads_and_a_missing_tool_is_an_error() {
        let dir = project();
        let mut server = Server::new(dir.path());
        let refused = ask(
            &mut server,
            request(
                1,
                "tools/call",
                json!({ "name": "read_file", "arguments": {} }),
            ),
        );
        assert_eq!(refused["result"]["isError"], true);
        let text = refused["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("`path`"),
            "the refusal names the argument: {text}"
        );

        let missing = ask(
            &mut server,
            request(2, "tools/call", json!({ "name": "flash_it" })),
        );
        assert_eq!(missing["error"]["code"], INVALID_PARAMS);
        assert!(
            missing["error"]["message"]
                .as_str()
                .unwrap()
                .contains("flash_it")
        );

        let unknown = ask(&mut server, request(3, "resources/list", json!({})));
        assert_eq!(unknown["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(unknown["id"], 3);
    }

    #[test]
    fn notifications_and_responses_are_not_answered_and_garbage_is() {
        let dir = project();
        let mut server = Server::new(dir.path());
        assert_eq!(
            server.answer_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            None
        );
        assert_eq!(
            server.answer_line(r#"{"jsonrpc":"2.0","id":4,"result":{}}"#),
            None
        );

        let garbage: Value =
            serde_json::from_str(&server.answer_line("{not json").unwrap()).unwrap();
        assert_eq!(garbage["error"]["code"], PARSE_ERROR);
        assert_eq!(garbage["id"], Value::Null);

        let batch: Value = serde_json::from_str(
            &server
                .answer_line(
                    &json!([
                        request(1, "ping", Value::Null),
                        { "jsonrpc": "2.0", "method": "notifications/initialized" },
                        request(2, "ping", Value::Null),
                    ])
                    .to_string(),
                )
                .unwrap(),
        )
        .unwrap();
        let ids: Vec<&Value> = batch.as_array().unwrap().iter().map(|a| &a["id"]).collect();
        assert_eq!(
            ids,
            [&json!(1), &json!(2)],
            "a batch answers its requests and not its notification"
        );
    }

    #[test]
    fn a_session_is_one_line_in_and_one_line_out() {
        let dir = project();
        let input = [
            request(1, "initialize", json!({ "protocolVersion": "2025-06-18" })).to_string(),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
            String::new(),
            request(2, "tools/list", json!({})).to_string(),
            request(
                3,
                "tools/call",
                json!({ "name": "list_files", "arguments": {} }),
            )
            .to_string(),
        ]
        .join("\r\n");
        let mut output = Vec::new();
        serve(dir.path(), input.as_bytes(), &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let answers: Vec<Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let ids: Vec<&Value> = answers.iter().map(|a| &a["id"]).collect();
        assert_eq!(ids, [&json!(1), &json!(2), &json!(3)]);
        assert!(!output.contains('\r'), "lines end in a bare newline");
        let files = answers[2]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(files.contains("main.rs"), "{files}");
    }

    /// A path-only workspace, so `cargo metadata` needs no registry.
    fn workspace_with(members: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let listed: Vec<String> = members.iter().map(|m| format!("\"{m}\"")).collect();
        fs::write(
            dir.path().join("Cargo.toml"),
            format!(
                "[workspace]\nmembers = [{}]\nresolver = \"2\"\n",
                listed.join(", ")
            ),
        )
        .unwrap();
        for member in members {
            write_member(dir.path(), member, "");
        }
        dir
    }

    fn write_member(root: &Path, name: &str, dependencies: &str) {
        fs::create_dir_all(root.join(name).join("src")).unwrap();
        fs::write(
            root.join(name).join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
                 [dependencies]\n{dependencies}"
            ),
        )
        .unwrap();
        fs::write(root.join(name).join("src/lib.rs"), "").unwrap();
    }

    /// How many direct dependencies the report says `member` declares.
    fn direct_deps(server: &mut Server, member: &str) -> u64 {
        let answer = ask(
            server,
            request(1, "tools/call", json!({ "name": "workspace_report" })),
        );
        assert_eq!(answer["result"]["isError"], false, "{answer}");
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        let report: Value = serde_json::from_str(text).unwrap();
        report["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == member)
            .unwrap_or_else(|| panic!("{member} is a member: {report}"))["directDeps"]
            .as_u64()
            .unwrap()
    }

    #[test]
    fn the_workspace_is_loaded_when_asked_for_and_again_when_a_manifest_moves() {
        let dir = workspace_with(&["alpha", "beta"]);
        let mut server = Server::new(dir.path());

        // A file tool needs no graph, so it does not load one.
        ask(
            &mut server,
            request(1, "tools/call", json!({ "name": "list_files" })),
        );
        assert!(server.kept.is_none(), "nothing asked for the workspace");

        assert_eq!(direct_deps(&mut server, "alpha"), 0);
        assert!(server.kept.is_some(), "the graph is kept for the next call");
        assert!(server.kept.as_ref().unwrap().stamp.still_holds());

        // `alpha` starts depending on `beta`: only a member's manifest moved,
        // and the graph is resolved again with the new edge in it.
        write_member(
            dir.path(),
            "alpha",
            "beta = { path = \"../beta\" }
",
        );
        assert!(
            !server.kept.as_ref().unwrap().stamp.still_holds(),
            "a member's manifest is part of the stamp"
        );
        assert_eq!(direct_deps(&mut server, "alpha"), 1);
    }

    #[test]
    fn a_workspace_that_will_not_load_says_why() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = ").unwrap();
        let mut server = Server::new(dir.path());
        let answer = ask(
            &mut server,
            request(1, "tools/call", json!({ "name": "workspace_report" })),
        );
        assert_eq!(answer["result"]["isError"], true);
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("cargo metadata"),
            "the refusal says what failed: {text}"
        );
        assert!(
            !text.contains("open a project"),
            "a project is open: {text}"
        );
    }
}
