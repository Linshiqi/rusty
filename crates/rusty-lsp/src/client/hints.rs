//! Inlay hints: what rust-analyzer infers about the code and the editor shows
//! in it — the type of a binding nobody wrote down, the name of the
//! parameter an argument is for, what a method chain produces at each step,
//! which block a closing brace ends.

use serde_json::{Value, json};

use super::LspClient;
use crate::{convert::Lines, error::Result, model::InlayHint};

impl LspClient {
    /// The hints over lines `from..to` of a file, in scalar columns. `to` past
    /// the end means to the end: the range asked for stops at the last line,
    /// since a position past it is one the server may refuse.
    pub fn inlay_hints(&self, path: &str, from: u32, to: u32) -> Result<Vec<InlayHint>> {
        let text = self.shared.text_of(path).unwrap_or_default();
        let lines = Lines::new(&text, self.shared.encoding());
        let (last, last_units) = lines.end();
        let end = if to > last {
            json!({ "line": last, "character": last_units })
        } else {
            json!({ "line": to, "character": 0 })
        };
        let result = self.shared.request(
            "textDocument/inlayHint",
            json!({
                "textDocument": { "uri": self.shared.uri(path) },
                "range": { "start": { "line": from.min(last), "character": 0 }, "end": end },
            }),
        )?;
        Ok(result
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|hint| {
                let line = hint["position"]["line"].as_u64()? as u32;
                let character = hint["position"]["character"].as_u64()? as u32;
                Some(InlayHint {
                    line,
                    col: lines.scalar(line, character),
                    label: label(&hint["label"])?,
                    parameter: hint["kind"].as_u64() == Some(2),
                    pad_left: hint["paddingLeft"].as_bool() == Some(true),
                    pad_right: hint["paddingRight"].as_bool() == Some(true),
                })
            })
            .collect())
    }
}

/// A hint's label: a string, or parts to be read one after another — the
/// parts carry places to jump to, which a label drawn as text has no use
/// for.
fn label(label: &Value) -> Option<String> {
    match label {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| part["value"].as_str())
                .collect(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{client_with, method};

    /// Labels come as a string or as parts, and a column after a `中` is
    /// counted in characters — the server counts bytes.
    #[test]
    fn hints_arrive_with_their_labels_read_and_their_columns_in_characters() {
        let text = "fn main() {\n    let 中 = 1;\n    v.iter()\n}\n";
        let answer = json!([
            { "position": { "line": 1, "character": 11 }, "label": ": i32", "kind": 1 },
            { "position": { "line": 2, "character": 12 },
              "label": [{ "value": "impl " }, { "value": "Iterator", "location": {} }], "kind": 1 },
            { "position": { "line": 1, "character": 15 }, "label": "x:", "kind": 2,
              "paddingRight": true },
        ]);
        let (client, _root, seen) = client_with(&[("src/main.rs", text)], move |message, _| {
            (method(message) == "textDocument/inlayHint").then(|| answer.clone())
        });
        let hints = client.inlay_hints("src/main.rs", 0, 99).unwrap();
        assert_eq!(hints.len(), 3);
        assert_eq!(
            (hints[0].line, hints[0].col, hints[0].label.as_str()),
            (1, 9, ": i32")
        );
        assert_eq!(hints[1].label, "impl Iterator");
        assert!(!hints[1].parameter);
        assert!(hints[2].parameter);
        assert!(
            hints[2].pad_right && !hints[2].pad_left,
            "the padding comes as the server said"
        );
        assert!(!hints[0].pad_left && !hints[0].pad_right);
        // Past the end, the range stops at the end of the last line.
        let range = seen
            .lock()
            .unwrap()
            .iter()
            .find(|m| method(m) == "textDocument/inlayHint")
            .map(|m| m["params"]["range"].clone())
            .unwrap();
        assert_eq!(range["end"]["line"], 4);
    }
}
