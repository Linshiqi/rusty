//! Replies as the wire model: `serde_json::Value` in, scalar-addressed types
//! out.
//!
//! Every conversion here takes the document's text and the negotiated
//! encoding and nothing else — no process, no lock — so each is a function a
//! test can call with a JSON literal. The client used to carry the same
//! `scalar` closure four times over, once per reply type, and the copies had
//! drifted in what they did about a line the document did not have.

use serde_json::Value;

use crate::{
    error::{Error, Result},
    model::{
        ActionEdit, CompletionItem, DiagSeverity, EditRange, FileDiagnostic, HoverInfo,
        SemanticSpan, SignatureInfo,
    },
    positions::{Encoding, byte_of_character, character_to_scalar},
    uri::same_file_uri,
};

/// A protocol column on `line` of `text`, as a scalar column.
///
/// A line the text does not have — the document moved under the reply —
/// answers with the column unconverted. Clamping is what every other
/// conversion does when the server and the client disagree by a version, and
/// a column is more use to the caller than nothing.
pub(crate) fn scalar_at(text: &str, line: u32, character: u32, encoding: Encoding) -> u32 {
    text.split('\n')
        .nth(line as usize)
        .map_or(character, |line_text| {
            character_to_scalar(line_text, character, encoding)
        })
}

/// A protocol `Range` as scalar columns, or `None` when it is not one.
pub(crate) fn edit_range(text: &str, range: &Value, encoding: Encoding) -> Option<EditRange> {
    let position = |which: &str| -> Option<(u32, u32)> {
        let line = range[which]["line"].as_u64()? as u32;
        let character = range[which]["character"].as_u64()? as u32;
        Some((line, scalar_at(text, line, character, encoding)))
    };
    let (start_line, start_col) = position("start")?;
    let (end_line, end_col) = position("end")?;
    Some(EditRange {
        start_line,
        start_col,
        end_line,
        end_col,
    })
}

/// A `textDocument/completion` reply. It is `CompletionItem[]` or a
/// `CompletionList`; both hold items.
pub(crate) fn completion_items(
    result: &Value,
    text: &str,
    encoding: Encoding,
) -> Vec<CompletionItem> {
    let items = result
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| result.as_array());
    // A hundred is more than any popup shows and keeps a `use`-everything
    // completion reply from shipping megabytes over the bridge.
    items
        .into_iter()
        .flatten()
        .take(100)
        .map(|item| {
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
                .and_then(|range| edit_range(text, range, encoding));
            CompletionItem {
                label,
                kind: item["kind"].as_u64().map(kind_name).map(str::to_string),
                detail: item["detail"].as_str().map(str::to_string),
                insert,
                edit: range,
            }
        })
        .collect()
}

/// A `textDocument/hover` reply. `contents` is MarkupContent | MarkedString |
/// MarkedString[]. The range needs the document's text to convert, so it is
/// absent when the document is not open.
pub(crate) fn hover_info(
    result: &Value,
    text: Option<&str>,
    encoding: Encoding,
) -> Option<HoverInfo> {
    let contents = &result["contents"];
    let prose = contents["value"]
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
        .filter(|t| !t.is_empty())?;
    let range = match (result.get("range"), text) {
        (Some(range), Some(text)) => edit_range(text, range, encoding),
        _ => None,
    };
    Some(HoverInfo { text: prose, range })
}

/// A `textDocument/signatureHelp` reply: one signature, and where its active
/// parameter sits in the label, as bytes.
///
/// The parameter comes either as a substring of the label or as a pair of
/// offsets — and the offsets are UTF-16 code units *regardless of the
/// negotiated position encoding*, which governs document positions only.
/// Both forms are resolved here, the second through the same arithmetic the
/// editor uses at the DOM boundary rather than a private copy of it.
pub(crate) fn signature_info(result: &Value) -> Option<SignatureInfo> {
    let signatures = result["signatures"].as_array()?;
    let active = result["activeSignature"].as_u64().unwrap_or(0) as usize;
    let signature = signatures.get(active).or_else(|| signatures.first())?;
    let label = signature["label"]
        .as_str()
        .filter(|label| !label.is_empty())?
        .to_string();

    // Per-signature wins over top-level, as the 3.16 spec added.
    let active_param = signature
        .get("activeParameter")
        .and_then(Value::as_u64)
        .or_else(|| result.get("activeParameter").and_then(Value::as_u64));

    let span = active_param
        .and_then(|index| signature["parameters"].as_array()?.get(index as usize))
        .and_then(|parameter| match &parameter["label"] {
            // A substring of the label. `find` is what the spec intends; a
            // parameter text that appears twice in one signature would have
            // been sent as offsets.
            Value::String(text) => {
                let start = label.find(text.as_str())?;
                Some((start, start + text.len()))
            }
            Value::Array(pair) => {
                let start = pair.first()?.as_u64()? as usize;
                let end = pair.get(1)?.as_u64()? as usize;
                Some((
                    byte_of_character(&label, start, Encoding::Utf16),
                    byte_of_character(&label, end, Encoding::Utf16),
                ))
            }
            _ => None,
        });

    let doc = signature.get("documentation").and_then(|doc| {
        doc.as_str()
            .map(str::to_string)
            .or_else(|| doc["value"].as_str().map(str::to_string))
    });

    Some(SignatureInfo {
        label,
        param_start: span.map(|(start, _)| start as u32),
        param_end: span.map(|(_, end)| end as u32),
        doc,
    })
}

/// A `textDocument/semanticTokens/full` reply.
///
/// The data is quintuples of u32 — deltaLine, deltaStart, length, type
/// index, modifier bits — relative-encoded, in the negotiated position
/// encoding. Decoded to absolute lines and Unicode-scalar columns, with the
/// type index resolved against the server's legend, so the frontend sees
/// names and scalars and nothing of the format.
pub(crate) fn semantic_spans(
    data: &[u32],
    text: &str,
    legend: &[String],
    encoding: Encoding,
) -> Vec<SemanticSpan> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut spans = Vec::with_capacity(data.len() / 5);
    let mut line = 0u32;
    let mut unit_col = 0u32;
    // Five integers per token, and the pattern names them; a trailing partial
    // token — a server bug — is in the remainder and is dropped, as
    // `chunks_exact` dropped it.
    let (tokens, _partial) = data.as_chunks::<5>();
    for &[delta_line, delta_start, unit_len, type_index, _modifiers] in tokens {
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
    spans
}

/// LSP diagnostics as the wire model, columns already scalar, sorted by
/// position. `text` is the document as this client knows it — or `None`, in
/// which case columns travel as the server sent them.
pub(crate) fn diagnostics(
    items: &Value,
    text: Option<&str>,
    encoding: Encoding,
) -> Vec<FileDiagnostic> {
    let scalar = |line: u32, character: u32| -> u32 {
        text.map_or(character, |text| scalar_at(text, line, character, encoding))
    };

    let mut out: Vec<FileDiagnostic> = items
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
    out.sort_by_key(|d| (d.start_line, d.start_col, d.severity));
    out
}

/// The LSP CompletionItemKind table, named so the frontend never holds a
/// second copy of these numbers.
pub(crate) fn kind_name(kind: u64) -> &'static str {
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

/// A WorkspaceEdit's text edits for the file at `ours` only — `None` when
/// the edit also touches other files or moves one. Half of a multi-file fix
/// is worse than none.
pub(crate) fn single_file_edits(edit: &Value, ours: &str) -> Option<Vec<Value>> {
    let mut collected = Vec::new();
    let mut take = |uri: &str, edits: &Value| -> bool {
        if !same_file_uri(uri, ours) {
            return false;
        }
        collected.extend(edits.as_array().into_iter().flatten().cloned());
        true
    };

    if let Some(changes) = edit["changes"].as_object() {
        for (uri, edits) in changes {
            if !take(uri, edits) {
                return None;
            }
        }
    }
    if let Some(documents) = edit["documentChanges"].as_array() {
        for change in documents {
            // A create/rename/delete file operation — beyond this client's
            // apply path.
            let uri = change["textDocument"]["uri"].as_str()?;
            if !take(uri, &change["edits"]) {
                return None;
            }
        }
    }
    Some(collected)
}

/// Text edits as the frontend applies them: scalar ranges against `text`.
pub(crate) fn action_edits(
    edits: &[Value],
    text: &str,
    encoding: Encoding,
) -> Option<Vec<ActionEdit>> {
    edits
        .iter()
        .map(|edit| {
            Some(ActionEdit {
                range: edit_range(text, &edit["range"], encoding)?,
                new_text: edit["newText"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// A rename's WorkspaceEdit grouped by file — `(uri, the server's edits)`.
///
/// Refuses a rename that also moves a file: rust-analyzer emits one when the
/// symbol is a module, and applying only the text half would leave the
/// project not building. The error says which part is missing.
pub(crate) fn edits_by_file(result: &Value) -> Result<Vec<(String, Vec<Value>)>> {
    let mut by_file: Vec<(String, Vec<Value>)> = Vec::new();
    let mut add = |uri: &str, edits: &Value| {
        let Some(list) = edits.as_array() else {
            return;
        };
        match by_file
            .iter_mut()
            .find(|(known, _)| same_file_uri(known, uri))
        {
            Some((_, existing)) => existing.extend(list.iter().cloned()),
            None => by_file.push((uri.to_string(), list.clone())),
        }
    };

    if let Some(changes) = result["changes"].as_object() {
        for (uri, edits) in changes {
            add(uri, edits);
        }
    }
    if let Some(documents) = result["documentChanges"].as_array() {
        for change in documents {
            let Some(uri) = change["textDocument"]["uri"].as_str() else {
                return Err(Error::Server {
                    method: "textDocument/rename".into(),
                    message: "this rename also moves a file, which rusty cannot apply \
                              yet — rename the module in the file tree instead"
                        .into(),
                });
            };
            add(uri, &change["edits"]);
        }
    }
    Ok(by_file)
}

/// Apply a server's text edits to a whole document.
///
/// Back to front by position, so an edit never moves the ones still to be
/// applied — the mistake that turns a rename into corruption at the second
/// occurrence in a line.
///
/// `None` when an edit names a line the file does not have. That means the
/// server and the disk disagree, and then *every* range is suspect: columns
/// clamp silently, so a stale edit would not fail, it would append text into
/// somebody's source. Refusing the whole file is the only honest answer.
pub(crate) fn apply_text_edits(text: &str, edits: &[Value], encoding: Encoding) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let offset = |line: u32, character: u32| -> usize {
        let mut at = 0usize;
        for row in lines.iter().take(line as usize) {
            at += row.chars().count() + 1;
        }
        let row = lines.get(line as usize).copied().unwrap_or("");
        at + character_to_scalar(row, character, encoding) as usize
    };

    for edit in edits {
        let last = edit["range"]["end"]["line"].as_u64().unwrap_or(0) as usize;
        if last >= lines.len() {
            return None;
        }
    }
    let mut ranges: Vec<(usize, usize, String)> = edits
        .iter()
        .filter_map(|edit| {
            let start = offset(
                edit["range"]["start"]["line"].as_u64()? as u32,
                edit["range"]["start"]["character"].as_u64()? as u32,
            );
            let end = offset(
                edit["range"]["end"]["line"].as_u64()? as u32,
                edit["range"]["end"]["character"].as_u64()? as u32,
            );
            Some((
                start.min(end),
                start.max(end),
                edit["newText"].as_str().unwrap_or("").to_string(),
            ))
        })
        .collect();
    ranges.sort_by_key(|(start, ..)| *start);

    let mut out: Vec<char> = text.chars().collect();
    for (start, end, replacement) in ranges.into_iter().rev() {
        if start > out.len() || end > out.len() {
            continue;
        }
        out.splice(start..end, replacement.chars());
    }
    Some(out.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn edit(line: u32, start: u32, end: u32, text: &str) -> Value {
        json!({
            "range": {
                "start": { "line": line, "character": start },
                "end": { "line": line, "character": end },
            },
            "newText": text,
        })
    }

    /// Two occurrences on one line. Applied front to back, the first edit
    /// shifts the second's columns and the rename lands in the wrong place —
    /// silently, as corrupted source rather than an error.
    #[test]
    fn edits_apply_back_to_front() {
        let text = "let radio = radio_new();";
        let out = apply_text_edits(
            text,
            &[edit(0, 4, 9, "tuner"), edit(0, 12, 17, "tuner")],
            Encoding::Utf8,
        );
        assert_eq!(out.as_deref(), Some("let tuner = tuner_new();"));
    }

    /// The server counts in the negotiated encoding, and rusty negotiates
    /// utf-8 — so a CJK comment above the edit must not shift it. The trap
    /// the LSP client keeps a 中文 comment in its other tests for.
    #[test]
    fn columns_are_read_in_the_negotiated_encoding() {
        let text = "// 中文注释\nlet radio = 1;";
        let out = apply_text_edits(text, &[edit(1, 4, 9, "tuner")], Encoding::Utf8);
        assert_eq!(out.as_deref(), Some("// 中文注释\nlet tuner = 1;"));
    }

    /// A range naming a line the file does not have means the server and
    /// the disk disagree. Every other range in that file is then suspect
    /// too — columns clamp silently, so a stale edit would not fail, it
    /// would append text into somebody's source. Refuse the file whole.
    #[test]
    fn a_file_the_server_and_the_disk_disagree_about_is_refused() {
        assert_eq!(
            apply_text_edits("one line", &[edit(9, 0, 1, "x")], Encoding::Utf8),
            None,
        );
        // And a range inside the file still applies.
        assert_eq!(
            apply_text_edits("one line", &[edit(0, 0, 3, "two")], Encoding::Utf8).as_deref(),
            Some("two line"),
        );
    }

    /// The four copies of this closure disagreed about a line the document
    /// does not have: one answered `None`, one column zero, one the column
    /// as sent. There is one now, and it clamps like every other conversion.
    #[test]
    fn a_line_the_document_does_not_have_answers_with_the_column_as_sent() {
        let text = "// 中文\nlet a = 1;";
        // Line 0, utf-16 unit 5 is after `// 中文` → scalar 5; in utf-8 the
        // same scalar column is unit 9.
        assert_eq!(scalar_at(text, 0, 9, Encoding::Utf8), 5);
        assert_eq!(scalar_at(text, 0, 5, Encoding::Utf16), 5);
        assert_eq!(
            scalar_at(text, 7, 3, Encoding::Utf8),
            3,
            "no line 7: unconverted"
        );
    }

    /// `ParameterInformation.label` offsets are UTF-16 by spec, whatever
    /// position encoding was negotiated — the negotiation covers document
    /// positions only. A label with a CJK parameter name before the active
    /// one is where the two systems part ways.
    #[test]
    fn signature_offsets_are_utf16_whatever_was_negotiated() {
        // "fn f(" = 5 units / 5 bytes; "名" = 1 unit / 3 bytes; ": u32, " = 7.
        let label = "fn f(名: u32, b: i32)";
        let result = json!({
            "signatures": [{
                "label": label,
                "parameters": [
                    { "label": [5, 11] },
                    { "label": [13, 19] },
                ],
                "activeParameter": 1,
            }],
        });
        let info = signature_info(&result).expect("a signature");
        let (start, end) = (
            info.param_start.expect("start") as usize,
            info.param_end.expect("end") as usize,
        );
        assert_eq!(&label[start..end], "b: i32", "{info:?}");

        // The substring form resolves through the label's own bytes.
        let by_text = json!({
            "signatures": [{
                "label": label,
                "parameters": [{ "label": "名: u32" }, { "label": "b: i32" }],
            }],
            "activeParameter": 0,
        });
        let info = signature_info(&by_text).expect("a signature");
        let (start, end) = (
            info.param_start.unwrap() as usize,
            info.param_end.unwrap() as usize,
        );
        assert_eq!(&label[start..end], "名: u32");
    }

    /// A completion's replacement range crosses the boundary like every
    /// other position: the CJK comment on the line before must not move it.
    #[test]
    fn completion_edits_arrive_as_scalar_columns() {
        let text = "// 中文\nlet x = fro;";
        let reply = json!({
            "items": [{
                "label": "frobnicate",
                "kind": 2,
                "textEdit": {
                    "range": {
                        "start": { "line": 1, "character": 8 },
                        "end": { "line": 1, "character": 11 },
                    },
                    "newText": "frobnicate()",
                },
            }],
        });
        let items = completion_items(&reply, text, Encoding::Utf8);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind.as_deref(), Some("method"));
        assert_eq!(items[0].insert, "frobnicate()");
        let range = items[0].edit.expect("a range");
        assert_eq!((range.start_col, range.end_col), (8, 11));
    }

    /// An edit for another file makes the whole action multi-file, and a
    /// file operation makes it unapplicable; both are `None`, never half.
    #[test]
    fn an_action_touching_another_file_is_refused_whole() {
        let ours = "file:///E:/proj/src/main.rs";
        let mine = json!({ "changes": { "file:///e:/proj/src/main.rs": [edit(0, 0, 1, "x")] } });
        assert_eq!(single_file_edits(&mine, ours).map(|e| e.len()), Some(1));

        let theirs = json!({ "changes": {
            "file:///E:/proj/src/main.rs": [edit(0, 0, 1, "x")],
            "file:///E:/proj/src/lib.rs": [edit(0, 0, 1, "x")],
        } });
        assert_eq!(single_file_edits(&theirs, ours), None);

        let moves =
            json!({ "documentChanges": [{ "kind": "rename", "oldUri": ours, "newUri": ours }] });
        assert_eq!(single_file_edits(&moves, ours), None);
    }
}
