//! The `initialize` round trip: what this client declares it can do, the
//! settings rust-analyzer is started with, and the two things its answer
//! decides for the rest of the session — the unit positions are counted in,
//! and the legend semantic tokens are read against.

use std::{path::Path, sync::Arc};

use serde_json::{Map, Value, json};

use super::Shared;
use crate::{discover, error::Result, positions::Encoding, uri::path_to_uri};

/// What this client can do, declared as the protocol asks for it.
fn capabilities() -> Value {
    json!({
        // Offer utf-8 first: rust-analyzer takes it, and then "character"
        // means bytes, which is the cheap direction for a Rust client.
        "general": { "positionEncodings": ["utf-8", "utf-16"] },
        // Ask rust-analyzer to say how it is getting on. Without this it
        // reports a failure to load the workspace to nobody, and a
        // server that can only parse is indistinguishable from one that
        // answers everything: the squiggles arrive, the completions are
        // empty, and nothing on screen says which of the two it is.
        "experimental": { "serverStatusNotification": true },
        "textDocument": {
            "synchronization": { "didSave": true },
            "publishDiagnostics": {},
            // Pull, not just push. After the build-data workspace switch,
            // rust-analyzer stops recomputing pushed diagnostics for open
            // files — they get wiped and stay gone. Under the pull model it
            // asks the client to re-request instead, and freshness becomes
            // this client's job, which it can actually do.
            "diagnostic": { "relatedDocumentSupport": false },
            // Snippets on: a function arrives as `name($0)` and a macro
            // as `println!($0)`, parentheses placed and the caret between
            // them, as VS Code has it, and postfix templates (`.if`,
            // `.match`) exist at all. The editor expands the placeholders
            // itself. `labelDetailsSupport` keeps the label a bare name:
            // without it rust-analyzer glues ` (use …)` onto the label,
            // and the row cannot set the note apart from the name.
            // `resolveSupport` for `additionalTextEdits` is what turns on
            // rust-analyzer's imports-on-the-fly: it will not offer an
            // item that is not yet in scope unless the client can fetch
            // the `use` line lazily, because computing one per candidate
            // is too slow to do eagerly. Without this, typing `Out` in a
            // file that does not import `esp_hal::gpio` offered nothing —
            // no `Output`, no import — while VS Code offered both.
            "completion": {
                "completionItem": {
                    "snippetSupport": true,
                    "labelDetailsSupport": true,
                    "resolveSupport": { "properties": ["additionalTextEdits"] },
                },
            },
            "hover": { "contentFormat": ["plaintext", "markdown"] },
            "definition": {},
            "references": {},
            "implementation": {},
            "typeDefinition": {},
            "documentHighlight": {},
            // Nested, so an outline has its impl blocks' methods under
            // them rather than beside them with a container name.
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "inlayHint": {},
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
                "requests": { "full": true, "range": true },
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
        // On, so the server narrates its indexing: without it a fresh
        // project shows "rust-analyzer" as ready while every completion
        // for the next minute comes back empty, and the user concludes
        // there is no completion.
        "window": { "workDoneProgress": true },
        "workspace": {
            "workspaceFolders": false,
            "configuration": false,
            "diagnostics": { "refreshSupport": true },
            // Told when to ask again: what was asked while the workspace
            // was still loading came back thin, and no edit is coming to
            // ask twice.
            "inlayHint": { "refreshSupport": true },
            "semanticTokens": { "refreshSupport": true },
            // The client watches the disk, so the server does not — and
            // on Windows a server watching for itself holds the
            // workspace's directories open, which no rename or move of
            // one survives. See `watched.rs`.
            "didChangeWatchedFiles": { "dynamicRegistration": true },
        },
    })
}

/// rust-analyzer's own settings for this project.
fn initialization_options(root: &Path, target: Option<&str>) -> Map<String, Value> {
    let mut cargo = Map::new();
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
    let mut options = Map::new();
    let linked = discover::linked_projects(root);
    if !linked.is_empty() {
        let mut all = vec![root.join("Cargo.toml").to_string_lossy().into_owned()];
        all.extend(linked);
        options.insert("linkedProjects".into(), json!(all));
    }
    options.insert("cargo".into(), Value::Object(cargo));
    options.insert("check".into(), json!({ "allTargets": false }));
    // On. It was off for a month on a wrong reading of one symptom: squiggles
    // that appeared for a few seconds and vanished were blamed on `build-std`,
    // and native diagnostics were said to cover "type errors, unresolved
    // names" anyway. Measured with `examples/diag_probe`, neither holds.
    // rust-analyzer's own analysis says nothing about `pub v: Vector3d` with
    // no such type, an unused import or a borrow error — only rustc does,
    // so an editor without the check showed those in Output after a build
    // and nowhere else. And the vanishing was this client sending the check's
    // results and the pulled ones as they arrived, each replacing the other;
    // they are merged now (`Shared::emit_diagnostics`). A build-std firmware
    // project watched for four minutes with the check on produced no storm
    // and no wipe.
    options.insert("checkOnSave".into(), json!(true));
    // Its default already, and said anyway: this is what makes the capability
    // above mean "ask the client" rather than "watch for yourself".
    options.insert("files".into(), json!({ "watcher": "client" }));
    // A function completes with its parentheses and the caret between them.
    // rust-analyzer's default fills the arguments in as placeholders to tab
    // through, which works only in an editor that walks tabstops; this one
    // places the caret and leaves the arguments to the signature card.
    options.insert(
        "completion".into(),
        json!({ "callable": { "snippets": "add_parentheses" } }),
    );
    // Every kind of symbol in the workspace search, not only types: its
    // default finds `struct Gpio` and not `fn set_high`, and a search for a
    // function that comes back empty reads as a search that does not work.
    options.insert(
        "workspace".into(),
        json!({ "symbol": { "search": { "kind": "all_symbols" } } }),
    );
    options
}

/// The `initialize` round trip.
pub(super) fn handshake(shared: &Arc<Shared>, root: &Path, target: Option<&str>) -> Result<()> {
    let options = initialization_options(root, target);
    let params = json!({
        "processId": std::process::id(),
        "rootUri": path_to_uri(root),
        "capabilities": capabilities(),
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

impl Shared {
    pub(crate) fn encoding(&self) -> Encoding {
        *self.encoding.get().unwrap_or(&Encoding::Utf16)
    }

    /// The token-type legend the server declared at initialize; indexes in
    /// every semantic-tokens response point into this.
    pub(super) fn legend(&self) -> Vec<String> {
        self.semantic_legend.get().cloned().unwrap_or_default()
    }
}
