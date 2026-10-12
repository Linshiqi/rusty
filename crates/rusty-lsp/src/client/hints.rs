//! Inlay hints: what rust-analyzer infers about the code and the editor shows
//! in it — the type of a binding nobody wrote down, the name of the
//! parameter an argument is for, what a method chain produces at each step,
//! which block a closing brace ends.

use serde_json::{Value, json};

use super::LspClient;
use crate::{
    convert::Lines,
    error::Result,
    model::{InlayHint, InlayHints, text_mark},
};

impl LspClient {
    /// The hints over lines `from..to` of a file, in scalar columns, and the
    /// mark of the text they are about. `to` past the end means to the end:
    /// the range asked for stops at the last line, since a position past it
    /// is one the server may refuse.
    ///
    /// The text is read and the question written in one turn
    /// (`Shared::ask_about`), so the text marked is the one the server
    /// answers about whatever else is being sent at the time. The editor
    /// draws the hints only over that text (`InlayHints`).
    pub fn inlay_hints(&self, path: &str, from: u32, to: u32) -> Result<InlayHints> {
        let encoding = self.shared.encoding();
        let uri = self.shared.uri(path);
        let (text, asked) = self
            .shared
            .ask_about(path, "textDocument/inlayHint", |text| {
                let (last, last_units) = Lines::new(text, encoding).end();
                let end = if to > last {
                    json!({ "line": last, "character": last_units })
                } else {
                    json!({ "line": to, "character": 0 })
                };
                json!({
                    "textDocument": { "uri": uri },
                    "range": { "start": { "line": from.min(last), "character": 0 }, "end": end },
                })
            })?;
        let result = self.shared.answer(asked)?;
        let lines = Lines::new(&text, encoding);
        let hints = result
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
            .collect();
        Ok(InlayHints {
            hints,
            about: text_mark(&text),
        })
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
    use std::{sync::Arc, thread};

    use super::*;
    use crate::client::tests::{client_with, method, server_copy};

    /// The report: hints drawn a line below their code, after two lines
    /// became one. The question had been written ahead of the change it was
    /// asked after, the server answered about the text before, and nothing
    /// in the answer said so. Asked here while another thread changes the
    /// file as fast as it can, every answer names the text the server had
    /// when the question reached it.
    #[test]
    fn a_hints_answer_names_the_text_the_server_had_when_it_was_asked() {
        let first = "fn main() {\n}\n";
        let (client, _root, seen) = client_with(&[("src/main.rs", first)], |message, _| {
            match method(message) {
                // The question's own id as its one hint, to tell the
                // answers apart.
                "textDocument/inlayHint" => Some(json!([{
                    "position": { "line": 0, "character": 0 },
                    "label": message["id"].to_string(),
                }])),
                "textDocument/diagnostic" => Some(json!({ "kind": "full", "items": [] })),
                _ => None,
            }
        });
        client.did_open("src/main.rs", first).unwrap();
        let client = Arc::new(client);
        let typing = {
            let client = Arc::clone(&client);
            thread::spawn(move || {
                for round in 0..300 {
                    let text = format!("fn main() {{\n{}}}\n", "    step();\n".repeat(round % 7));
                    client.did_change("src/main.rs", &text).unwrap();
                }
            })
        };
        let answers: Vec<(String, u64)> = (0..300)
            .map(|_| {
                let answer = client.inlay_hints("src/main.rs", 0, 99).unwrap();
                (answer.hints[0].label.clone(), answer.about)
            })
            .collect();
        typing.join().unwrap();

        let (_, when_asked) = server_copy(&seen, "textDocument/inlayHint");
        let texts: std::collections::HashSet<&String> = when_asked.values().collect();
        assert!(texts.len() > 1, "the file changed between the questions");
        for (id, about) in answers {
            assert_eq!(about, text_mark(&when_asked[&id]), "question {id}");
        }
    }

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
        let answer = client.inlay_hints("src/main.rs", 0, 99).unwrap();
        assert_eq!(answer.about, text_mark(text), "about the text it was shown");
        let hints = answer.hints;
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
