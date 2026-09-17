//! Finding your way around: where a thing is used, what implements it, what
//! its type is, the outline of a file and the symbols of the workspace, and
//! the other places a name occurs in the file it is in.
//!
//! Every answer that names places answers with the line each place is on, as
//! it reads now, because every consumer is a list somebody reads before
//! choosing a row — and a list of `src/lib.rs:41` rows is a list of riddles.

use std::collections::HashMap;

use serde_json::{Value, json};

use super::LspClient;
use crate::{
    error::Result,
    model::{EditRange, Location, Place, Symbol},
    positions::character_to_scalar,
    uri::{uri_to_absolute, uri_to_relative},
};

/// How much of a line a place carries. A preview, not the file: a minified
/// line of megabytes would otherwise cross the bridge once per place on it.
const LINE_PREVIEW: usize = 1_000;

impl LspClient {
    /// Everywhere the thing at this position is used, its declaration
    /// included — the list a rename is about to change.
    pub fn references(&self, path: &str, line: u32, col: u32) -> Result<Vec<Place>> {
        let mut params = self.position_params(path, line, col);
        params["context"] = json!({ "includeDeclaration": true });
        let result = self.shared.request("textDocument/references", params)?;
        Ok(self.places(&result))
    }

    /// What implements the trait or trait method at this position — or,
    /// on a type, the impls written for it.
    pub fn implementations(&self, path: &str, line: u32, col: u32) -> Result<Vec<Place>> {
        let result = self.shared.request(
            "textDocument/implementation",
            self.position_params(path, line, col),
        )?;
        Ok(self.places(&result))
    }

    /// Where the type of the thing at this position is defined.
    pub fn type_definition(&self, path: &str, line: u32, col: u32) -> Result<Vec<Place>> {
        let result = self.shared.request(
            "textDocument/typeDefinition",
            self.position_params(path, line, col),
        )?;
        Ok(self.places(&result))
    }

    /// Where the name at this position occurs in this file — what the editor
    /// marks while the caret rests on it.
    pub fn document_highlights(&self, path: &str, line: u32, col: u32) -> Result<Vec<EditRange>> {
        let result = self.shared.request(
            "textDocument/documentHighlight",
            self.position_params(path, line, col),
        )?;
        let text = self.shared.text_of(path).unwrap_or_default();
        let lines: Vec<&str> = text.split('\n').collect();
        let encoding = self.shared.encoding();
        let scalar = |line: u32, character: u32| {
            lines.get(line as usize).map_or(character, |text| {
                character_to_scalar(text, character, encoding)
            })
        };
        Ok(result
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| {
                let range = &item["range"];
                let start_line = range["start"]["line"].as_u64()? as u32;
                let end_line = range["end"]["line"].as_u64()? as u32;
                Some(EditRange {
                    start_line,
                    start_col: scalar(start_line, range["start"]["character"].as_u64()? as u32),
                    end_line,
                    end_col: scalar(end_line, range["end"]["character"].as_u64()? as u32),
                })
            })
            .collect())
    }

    /// The file's outline, in document order: each item after the item it
    /// sits in, with how deep it is and that item's name.
    pub fn document_symbols(&self, path: &str) -> Result<Vec<Symbol>> {
        let result = self.shared.request(
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": self.uri(path) } }),
        )?;
        let text = self.shared.text_of(path).unwrap_or_default();
        let lines: Vec<&str> = text.split('\n').collect();
        let mut out = Vec::new();
        for item in result.as_array().into_iter().flatten() {
            self.outline(item, path, &lines, 0, None, &mut out);
        }
        Ok(out)
    }

    /// Symbols anywhere in the workspace whose names match `query`, in the
    /// order rust-analyzer ranks them.
    pub fn workspace_symbols(&self, query: &str) -> Result<Vec<Symbol>> {
        let result = self
            .shared
            .request("workspace/symbol", json!({ "query": query }))?;
        let mut texts = HashMap::new();
        Ok(result
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| {
                let location = &item["location"];
                let place = self.place(
                    location["uri"].as_str()?,
                    location.get("range")?,
                    &mut texts,
                )?;
                Some(Symbol {
                    name: item["name"].as_str()?.to_string(),
                    kind: symbol_kind(&item["kind"]),
                    container: item["containerName"]
                        .as_str()
                        .filter(|name| !name.is_empty())
                        .map(str::to_string),
                    depth: 0,
                    location: place.location,
                })
            })
            .collect())
    }

    fn position_params(&self, path: &str, line: u32, col: u32) -> Value {
        json!({
            "textDocument": { "uri": self.uri(path) },
            "position": self.protocol_position(path, line, col),
        })
    }

    /// Protocol locations as places. A reply is a `Location`, a list of
    /// them, or a list of `LocationLink`s — rust-analyzer answers with links
    /// to a client that declares them and with locations to one that does
    /// not, and a server is free to change its mind — so all three are read.
    pub(super) fn places(&self, result: &Value) -> Vec<Place> {
        let items: Vec<&Value> = match result {
            Value::Array(items) => items.iter().collect(),
            Value::Null => Vec::new(),
            one => vec![one],
        };
        let mut texts = HashMap::new();
        items
            .into_iter()
            .filter_map(|item| {
                let (uri, range) = match item.get("targetUri") {
                    Some(uri) => (
                        uri.as_str()?,
                        item.get("targetSelectionRange")
                            .or_else(|| item.get("targetRange"))?,
                    ),
                    None => (item["uri"].as_str()?, item.get("range")?),
                };
                self.place(uri, range, &mut texts)
            })
            .collect()
    }

    /// One place, its columns in scalars and its line to show. `texts` holds
    /// each file read so far, so a hundred references in one file read it
    /// once.
    fn place(
        &self,
        uri: &str,
        range: &Value,
        texts: &mut HashMap<String, Option<String>>,
    ) -> Option<Place> {
        let line = range["start"]["line"].as_u64()? as u32;
        let start = range["start"]["character"].as_u64()? as u32;
        let end_line = range["end"]["line"]
            .as_u64()
            .map_or(line, |line| line as u32);
        let end = range["end"]["character"]
            .as_u64()
            .map_or(start, |character| character as u32);
        let (path, external) = match uri_to_relative(uri, &self.shared.root) {
            Some(relative) => (relative, false),
            // Outside the project: the absolute path travels, and the viewer
            // decides whether it is somewhere it is willing to read.
            None => (uri_to_absolute(uri)?.replace('\\', "/"), true),
        };
        let text = texts.entry(uri.to_string()).or_insert_with(|| {
            if external {
                std::fs::read_to_string(&path).ok()
            } else {
                self.shared.text_of(&path)
            }
        });
        let line_text = text
            .as_deref()
            .and_then(|text| text.split('\n').nth(line as usize))
            .map(|text| text.strip_suffix('\r').unwrap_or(text));
        let encoding = self.shared.encoding();
        let (col, end_col) = match line_text {
            Some(line_text) => {
                let col = character_to_scalar(line_text, start, encoding);
                let end_col = if end_line == line {
                    character_to_scalar(line_text, end, encoding)
                } else {
                    line_text.chars().count() as u32
                };
                (col, end_col)
            }
            None => (start, if end_line == line { end } else { start }),
        };
        Some(Place {
            location: Location {
                path,
                line,
                col,
                external,
            },
            end_col,
            text: line_text
                .unwrap_or_default()
                .chars()
                .take(LINE_PREVIEW)
                .collect(),
        })
    }

    /// One `DocumentSymbol` and everything under it, depth first. A server
    /// that answers with flat `SymbolInformation` instead still lands, every
    /// item at the top.
    fn outline(
        &self,
        item: &Value,
        path: &str,
        lines: &[&str],
        depth: u32,
        container: Option<&str>,
        out: &mut Vec<Symbol>,
    ) {
        let Some(name) = item["name"].as_str() else {
            return;
        };
        let range = item
            .get("selectionRange")
            .or_else(|| item.get("range"))
            .or_else(|| item["location"].get("range"));
        let Some((line, character)) = range.and_then(|range| {
            Some((
                range["start"]["line"].as_u64()? as u32,
                range["start"]["character"].as_u64()? as u32,
            ))
        }) else {
            return;
        };
        let col = lines.get(line as usize).map_or(character, |text| {
            character_to_scalar(text, character, self.shared.encoding())
        });
        out.push(Symbol {
            name: name.to_string(),
            kind: symbol_kind(&item["kind"]),
            container: container
                .map(str::to_string)
                .or_else(|| item["containerName"].as_str().map(str::to_string))
                .filter(|name| !name.is_empty()),
            depth,
            location: Location {
                path: path.to_string(),
                line,
                col,
                external: false,
            },
        });
        for child in item["children"].as_array().into_iter().flatten() {
            self.outline(child, path, lines, depth + 1, Some(name), out);
        }
    }
}

/// The protocol's symbol kind, named. Two names are rust-analyzer's rather
/// than the protocol's, because this client talks to nothing else: it sends
/// a trait as `Interface` and an impl block as `Object`, and a row reading
/// "interface" or "object" beside `impl Display for Point` would be a word
/// no Rust programmer uses for it.
fn symbol_kind(kind: &Value) -> String {
    let name = match kind.as_u64().unwrap_or(0) {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "trait",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "impl",
        20 => "key",
        21 => "null",
        22 => "variant",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type",
        _ => "symbol",
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{Seen, fake_server, method, reply};
    use crate::uri::path_to_uri;

    /// A client over a fake server that answers `handle`'s requests, in a
    /// project holding `files`, and every message the server received.
    fn client_with(
        files: &[(&str, &str)],
        handle: impl Fn(&Value, &std::path::Path) -> Option<Value> + Send + 'static,
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

    fn range(line: u32, start: u32, end: u32) -> Value {
        json!({ "start": { "line": line, "character": start },
                "end": { "line": line, "character": end } })
    }

    /// References arrive as locations in the project and outside it, with
    /// their lines read — and a `中` before the name, so a column counted in
    /// bytes cannot pass for one counted in scalars.
    #[test]
    fn references_come_back_as_places_with_their_lines() {
        let library = tempfile::tempdir().unwrap();
        let outside = library.path().join("lib.rs");
        std::fs::write(&outside, "pub fn gain() {}\n").unwrap();
        let outside_uri = path_to_uri(&outside);
        let (client, _root, seen) = client_with(
            &[("src/main.rs", "fn main() {\n    let 中 = gain();\n}\n")],
            move |message, root| {
                (method(message) == "textDocument/references").then(|| {
                    json!([
                        { "uri": path_to_uri(&root.join("src/main.rs")), "range": range(1, 14, 18) },
                        { "uri": outside_uri, "range": range(0, 7, 11) },
                    ])
                })
            },
        );
        let places = client.references("src/main.rs", 1, 12).unwrap();
        let asked = seen
            .lock()
            .unwrap()
            .iter()
            .find(|m| method(m) == "textDocument/references")
            .map(|m| m["params"]["context"]["includeDeclaration"].clone());
        assert_eq!(asked, Some(json!(true)), "the declaration is a use too");
        assert_eq!(places.len(), 2);
        assert_eq!(
            (places[0].location.path.as_str(), places[0].location.line),
            ("src/main.rs", 1)
        );
        assert_eq!(
            (places[0].location.col, places[0].end_col),
            (12, 16),
            "utf-8 bytes became scalars"
        );
        assert_eq!(places[0].text, "    let 中 = gain();");
        assert!(!places[0].location.external);
        assert!(places[1].location.external);
        assert_eq!(places[1].text, "pub fn gain() {}");
    }

    /// Implementations arrive as links; the selection range is the name.
    #[test]
    fn location_links_are_read_by_their_target() {
        let (client, _root, _) = client_with(
            &[("src/lib.rs", "trait T {}\nimpl T for u8 {}\n")],
            |message, root| {
                (method(message) == "textDocument/implementation").then(|| {
                    json!([{
                        "targetUri": path_to_uri(&root.join("src/lib.rs")),
                        "targetRange": range(1, 0, 16),
                        "targetSelectionRange": range(1, 11, 13),
                    }])
                })
            },
        );
        let places = client.implementations("src/lib.rs", 0, 6).unwrap();
        assert_eq!(places.len(), 1);
        assert_eq!(
            (
                places[0].location.line,
                places[0].location.col,
                places[0].end_col
            ),
            (1, 11, 13)
        );
        assert_eq!(places[0].text, "impl T for u8 {}");
    }

    /// An outline is flattened depth first, each item after its parent and
    /// carrying the parent's name.
    #[test]
    fn a_files_outline_is_flattened_with_depth_and_container() {
        let (client, _root, _) = client_with(
            &[("src/lib.rs", "struct P;\nimpl P {\n    fn new() {}\n}\n")],
            |message, _| {
                (method(message) == "textDocument/documentSymbol").then(|| {
                    json!([
                        { "name": "P", "kind": 23, "range": range(0, 0, 9), "selectionRange": range(0, 7, 8) },
                        { "name": "impl P", "kind": 19, "range": range(1, 0, 1), "selectionRange": range(1, 5, 6),
                          "children": [
                              { "name": "new", "kind": 12, "range": range(2, 4, 15), "selectionRange": range(2, 7, 10) },
                          ] },
                    ])
                })
            },
        );
        let symbols = client.document_symbols("src/lib.rs").unwrap();
        // One line per item: name, kind, depth, container, where.
        let outline: Vec<String> = symbols
            .iter()
            .map(|s| {
                format!(
                    "{} {} {} {:?} {}:{}",
                    s.name, s.kind, s.depth, s.container, s.location.line, s.location.col
                )
            })
            .collect();
        assert_eq!(
            outline,
            [
                "P struct 0 None 0:7",
                "impl P impl 0 None 1:5",
                "new function 1 Some(\"impl P\") 2:7",
            ]
        );
    }

    #[test]
    fn workspace_symbols_carry_their_container_and_place() {
        let (client, _root, _) = client_with(
            &[("src/lib.rs", "mod regs {\n    pub struct Gpio;\n}\n")],
            |message, root| {
                (method(message) == "workspace/symbol" && message["params"]["query"] == "gpi").then(|| {
                    json!([{
                        "name": "Gpio", "kind": 23, "containerName": "regs",
                        "location": { "uri": path_to_uri(&root.join("src/lib.rs")), "range": range(1, 15, 19) },
                    }])
                })
            },
        );
        let symbols = client.workspace_symbols("gpi").unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].container.as_deref(), Some("regs"));
        assert_eq!(
            (
                symbols[0].location.path.as_str(),
                symbols[0].location.line,
                symbols[0].location.col
            ),
            ("src/lib.rs", 1, 15)
        );
    }

    #[test]
    fn highlights_are_ranges_in_scalars() {
        let (client, _root, _) = client_with(
            &[("src/lib.rs", "// 中\nlet 中x = 中x;\n")],
            |message, _| {
                (method(message) == "textDocument/documentHighlight").then(|| {
                    json!([
                        { "range": range(1, 4, 8), "kind": 3 },
                        { "range": range(1, 11, 15), "kind": 2 },
                    ])
                })
            },
        );
        let found = client.document_highlights("src/lib.rs", 1, 5).unwrap();
        let spans: Vec<(u32, u32, u32)> = found
            .iter()
            .map(|r| (r.start_line, r.start_col, r.end_col))
            .collect();
        assert_eq!(spans, [(1, 4, 6), (1, 9, 11)]);
    }
}
