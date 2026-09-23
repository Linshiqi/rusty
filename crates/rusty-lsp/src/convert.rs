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
    model::{
        ActionEdit, CompletionItem, CompletionList, DiagSeverity, EditRange, FileDiagnostic,
        HoverInfo, SemanticSpan, SignatureInfo,
    },
    positions::{Encoding, byte_of_character, character_to_scalar, scalar_to_character},
    uri::same_file_uri,
};

/// A document's text, split into lines once for converting many of the
/// server's positions against it — a completion answer carries a range per
/// item.
///
/// Split at `\n`, as everywhere else: a text that ends in a newline has an
/// empty last line.
pub(crate) struct Lines<'a> {
    lines: Vec<&'a str>,
    encoding: Encoding,
}

impl<'a> Lines<'a> {
    pub(crate) fn new(text: &'a str, encoding: Encoding) -> Self {
        Lines {
            lines: text.split('\n').collect(),
            encoding,
        }
    }

    /// A protocol column on `line`, as a scalar column.
    ///
    /// A line the text does not have — the document moved under the reply —
    /// answers with the column unconverted. Clamping is what every other
    /// conversion does when the server and the client disagree by a version,
    /// and a column is more use to the caller than nothing.
    pub(crate) fn scalar(&self, line: u32, character: u32) -> u32 {
        self.lines
            .get(line as usize)
            .map_or(character, |line_text| {
                character_to_scalar(line_text, character, self.encoding)
            })
    }

    /// A protocol `Range` as scalar columns, or `None` when it is not one.
    pub(crate) fn range(&self, range: &Value) -> Option<EditRange> {
        let position = |which: &str| -> Option<(u32, u32)> {
            let line = range[which]["line"].as_u64()? as u32;
            let character = range[which]["character"].as_u64()? as u32;
            Some((line, self.scalar(line, character)))
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

    /// Where the text ends, as a protocol position: the last line, and how
    /// long it is in the negotiated units.
    pub(crate) fn end(&self) -> (u32, u32) {
        let last = self.lines.len().saturating_sub(1);
        let units = scalar_to_character(self.lines[last], u32::MAX, self.encoding);
        (last as u32, units)
    }
}

/// How many completion items cross the bridge, after sorting. The popup
/// filters and ranks what arrives itself, and asks again while the server
/// says its answer is incomplete; a thousand is past what any prefix
/// narrows to, and keeps a `use`-everything reply of thousands from crossing
/// on every keystroke. It was four hundred, cut before any filtering, and a
/// large scope lost `println!` and `Vec` to the alphabet before `pr` was
/// typed.
const MAX_COMPLETIONS: usize = 1000;

/// A `textDocument/completion` reply. It is `CompletionItem[]` or a
/// `CompletionList`; both hold items, and only the second can say it is
/// incomplete. `reply` is left for the client to number.
pub(crate) fn completion_items(result: &Value, text: &str, encoding: Encoding) -> CompletionList {
    let items = result
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| result.as_array());
    // The server's order is not a ranking. rust-analyzer sends its items in
    // whatever order it found them and puts the relevance into `sortText`,
    // which the client is expected to sort by — VS Code does. Taking the
    // first hundred *unsorted* shipped a hundred arbitrary slice methods for
    // `v.` and left `len` and `push` behind, so the popup read as noise and
    // typing `le` narrowed it to nothing: an editor with no completion.
    // Sorted first, then capped generously — a few hundred items is tens of
    // kilobytes, and a `use`-everything reply of thousands is what the cap
    // is for.
    // The index is the position in the server's reply, taken before sorting:
    // it is what names the raw item again when the accepted one is resolved.
    let mut items: Vec<(u32, &Value)> = items
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, item)| (index as u32, item))
        .collect();
    items.sort_by_cached_key(|(_, item)| {
        let label = item["label"].as_str().unwrap_or_default();
        let sort = item["sortText"].as_str().unwrap_or(label);
        (sort.to_string(), label.to_string())
    });
    let lines = Lines::new(text, encoding);
    let items = items
        .into_iter()
        .take(MAX_COMPLETIONS)
        .map(|(index, item)| {
            let label = item["label"].as_str().unwrap_or_default().to_string();
            let filter = item["filterText"]
                .as_str()
                .filter(|filter| *filter != label)
                .map(str::to_string);
            let edit = item["textEdit"].as_object();
            let insert = edit
                .and_then(|e| e.get("newText"))
                .or_else(|| item.get("insertText"))
                .and_then(Value::as_str)
                .unwrap_or(&label)
                .to_string();
            let range = edit
                .and_then(|e| e.get("range"))
                .and_then(|range| lines.range(range));
            CompletionItem {
                label,
                kind: item["kind"].as_u64().map(kind_name).map(str::to_string),
                detail: item["detail"].as_str().map(str::to_string),
                insert,
                edit: range,
                index,
                label_detail: item["labelDetails"]["detail"].as_str().map(str::to_string),
                filter,
                snippet: item["insertTextFormat"].as_u64() == Some(2),
                description: item["labelDetails"]["description"]
                    .as_str()
                    .map(str::to_string),
            }
        })
        .collect();
    CompletionList {
        items,
        incomplete: result
            .get("isIncomplete")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        reply: 0,
    }
}

/// The edits an accepted completion makes *besides* the insertion — for
/// rust-analyzer, the `use` line an item not yet in scope brings with it.
/// Read off a resolved item; an item without any yields none.
pub(crate) fn completion_additional_edits(
    item: &Value,
    text: &str,
    encoding: Encoding,
) -> Vec<ActionEdit> {
    item["additionalTextEdits"]
        .as_array()
        .and_then(|edits| action_edits(edits, text, encoding))
        .unwrap_or_default()
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
        (Some(range), Some(text)) => Lines::new(text, encoding).range(range),
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
    let lines = text.map(|text| Lines::new(text, encoding));
    let scalar = |line: u32, character: u32| -> u32 {
        lines
            .as_ref()
            .map_or(character, |lines| lines.scalar(line, character))
    };

    let mut out: Vec<FileDiagnostic> = items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let range = &d["range"];
            let start_line = range["start"]["line"].as_u64()? as u32;
            let end_line = range["end"]["line"].as_u64()? as u32;
            let code = match &d["code"] {
                Value::String(code) => Some(code.clone()),
                Value::Number(code) => Some(code.to_string()),
                _ => None,
            };
            let severity = match d["severity"].as_u64() {
                Some(2) => DiagSeverity::Warning,
                Some(3) => DiagSeverity::Info,
                // rust-analyzer files `unlinked-file` as a hint, and a hint is
                // what this editor keeps out of the Problems panel. It is the
                // one hint that switches the server off for the whole file —
                // no completion, no hover, no jump, while the syntax errors
                // keep arriving — so here it is the warning it is.
                Some(4) if code.as_deref() == Some("unlinked-file") => DiagSeverity::Warning,
                Some(4) => DiagSeverity::Hint,
                // Absent means the producer did not say; rustc's errors
                // always do, so unmarked ones are treated as the worst.
                _ => DiagSeverity::Error,
            };
            Some(FileDiagnostic {
                severity,
                message: d["message"].as_str().unwrap_or_default().to_string(),
                source: d["source"].as_str().map(str::to_string),
                code,
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

/// A code action's WorkspaceEdit split into the edits for the file at `ours`
/// and the edits for every other file — `(uri, the server's edits)` — or
/// `None` when the action also creates, renames or deletes a file, which is
/// beyond this client's apply path. Half of such a fix is worse than none.
///
/// Other files used to refuse the whole action. The fix for a file no `mod`
/// line declares — the one that makes rust-analyzer answer nothing at all
/// for the file — edits *only* another file, and was the fix nobody could
/// reach.
pub(crate) type SplitEdits = (Vec<Value>, Vec<(String, Vec<Value>)>);

pub(crate) fn split_edits(edit: &Value, ours: &str) -> Option<SplitEdits> {
    let mut mine = Vec::new();
    let mut theirs = Vec::new();
    for (uri, edits) in edits_by_file(edit)? {
        if same_file_uri(&uri, ours) {
            mine.extend(edits);
        } else {
            theirs.push((uri, edits));
        }
    }
    Some((mine, theirs))
}

/// Text edits as the frontend applies them: scalar ranges against `text`.
pub(crate) fn action_edits(
    edits: &[Value],
    text: &str,
    encoding: Encoding,
) -> Option<Vec<ActionEdit>> {
    let lines = Lines::new(text, encoding);
    edits
        .iter()
        .map(|edit| {
            Some(ActionEdit {
                range: lines.range(&edit["range"])?,
                new_text: edit["newText"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// A WorkspaceEdit grouped by file — `(uri, the server's edits)` — or `None`
/// when it also creates, renames or deletes a file: rust-analyzer emits one
/// for the rename of a module, and applying only the text half would leave
/// the project not building. The caller says which part is missing.
pub(crate) fn edits_by_file(result: &Value) -> Option<Vec<(String, Vec<Value>)>> {
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
            let uri = change["textDocument"]["uri"].as_str()?;
            add(uri, &change["edits"]);
        }
    }
    Some(by_file)
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

/// A sentence for a server complaint that is really about the machine, not
/// about the code.
///
/// One failure matters enough here to be named: `cargo metadata` refusing
/// `--lockfile-path`. It took three wrong explanations to get to this one,
/// and the third was wrong in a way worth keeping written down — the fix it
/// offered was "pin an older rust-analyzer", which does not even work.
///
/// rust-analyzer hands `cargo metadata` a copy of the lockfile rather than
/// let it rewrite the project's own, and picks how to say so from the
/// toolchain's version: `--lockfile-path` below 1.95, `-Zlockfile-path` plus
/// `CARGO_RESOLVER_LOCKFILE_PATH` up to 1.97, the variable alone after that.
/// cargo removed the flag *in* 1.95 — and a nightly is a **pre-release**,
/// which semver sorts below the release it is becoming. So a cargo calling
/// itself `1.95.0-nightly` is asked for the one spelling it has just lost.
///
/// Espressif's Xtensa fork calls itself exactly that, which is why this is
/// the ordinary state of an Xtensa project and not a bad week on nightly.
/// Measured on the user's own toolchain: the same `cargo metadata` that
/// fails with the flag answers with the whole dependency graph when it is
/// given `-Zlockfile-path` and the variable instead. The cargo is fine; only
/// the number rust-analyzer reads sends it down the wrong branch.
///
/// So the remedy is an **upgrade**, and rusty's own pin already names it:
/// Xtensa Rust 1.97.0.0, whose cargo says `1.97.0-nightly` and is asked the
/// way it understands. `toolchain::report` raises the same finding with that
/// exact command against the version it found installed.
///
/// What does *not* work, each measured rather than assumed: a newer
/// rust-analyzer (every one since 1.83 carries the flag, and the standalone
/// 0.3.3049 does too), an older one (1.90's has it; 1.82's does not, and is
/// from October 2024), a `CARGO` pointing elsewhere or a `cargo` shim on
/// `PATH` — with a sysroot in hand rust-analyzer runs rustup's proxy and
/// reads neither — and any rust-analyzer setting, since there is none.
///
/// `None` for anything not recognised: the server's text stands.
pub fn explain_health(message: &str) -> Option<String> {
    if !message.contains("--lockfile-path") {
        return None;
    }
    let manifest = message
        .split('`')
        .find(|part| part.ends_with("Cargo.toml"))
        .unwrap_or("this project");
    Some(format!(
        "Dependencies will not resolve here, and the toolchain is one version behind \
         the fix rather than {manifest} being wrong. rust-analyzer asks a cargo that \
         calls itself `1.95.0-nightly` for `--lockfile-path`, which cargo removed in \
         1.95 — a nightly sorts below the release it is becoming, so it is asked for \
         the spelling it has just lost. It carries on without the dependency graph: \
         your own code still has completion, hover and go-to-definition, while \
         `esp_hal::` and every other crate answer nothing. Updating the Xtensa \
         toolchain moves the version past that boundary, and rust-analyzer then asks \
         the way this cargo already understands — the Problems tab has the \
         command. The build is unaffected either way."
    ))
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
        assert_eq!(Lines::new(text, Encoding::Utf8).scalar(0, 9), 5);
        assert_eq!(Lines::new(text, Encoding::Utf16).scalar(0, 5), 5);
        assert_eq!(
            Lines::new(text, Encoding::Utf8).scalar(7, 3),
            3,
            "no line 7: unconverted"
        );
    }

    /// A range asked for up to the end of a document ends at the end of its
    /// last line, counted the way the server counts — and a text ending in a
    /// newline ends on the empty line after it, at column zero.
    #[test]
    fn a_document_ends_where_the_server_counts_its_last_line_to() {
        // `// ` is three units either way, 中文 six bytes or two units, and
        // the crab four bytes or a surrogate pair.
        let text = "fn a() {}\n// 中文🦀";
        assert_eq!(Lines::new(text, Encoding::Utf8).end(), (1, 3 + 6 + 4));
        assert_eq!(Lines::new(text, Encoding::Utf16).end(), (1, 3 + 2 + 2));
        assert_eq!(Lines::new("fn a() {}\n", Encoding::Utf16).end(), (1, 0));
        assert_eq!(Lines::new("", Encoding::Utf8).end(), (0, 0));
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
        let items = completion_items(&reply, text, Encoding::Utf8).items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind.as_deref(), Some("method"));
        assert_eq!(items[0].insert, "frobnicate()");
        let range = items[0].edit.expect("a range");
        assert_eq!((range.start_col, range.end_col), (8, 11));
    }

    /// rust-analyzer's order is arrival order; its ranking is `sortText`.
    /// The best items have to survive the cap, so sorting comes before it —
    /// unsorted, `v.` shipped a hundred arbitrary slice methods and dropped
    /// `len`.
    #[test]
    fn completions_are_ranked_by_sort_text_before_the_cap_applies() {
        let mut items: Vec<serde_json::Value> = (0..(MAX_COMPLETIONS + 50))
            .map(|n| json!({ "label": format!("method_{n:04}"), "sortText": "ffffffff" }))
            .collect();
        // Arrives last, ranks first.
        items.push(json!({ "label": "len", "sortText": "00000000" }));
        // Same rank as the crowd: the label decides, alphabetically.
        items.push(json!({ "label": "aaa", "sortText": "ffffffff" }));
        let reply = json!({ "items": items });
        let got = completion_items(&reply, "", Encoding::Utf8).items;
        assert_eq!(got.len(), MAX_COMPLETIONS);
        assert_eq!(got[0].label, "len");
        assert_eq!(got[1].label, "aaa");
        assert_eq!(got[2].label, "method_0000");
        // The index still names the raw item: `len` arrived second to last.
        assert_eq!(got[0].index as usize, MAX_COMPLETIONS + 50);
        assert_eq!(got[2].index, 0);
        // An item without sortText ranks by its label.
        let bare = json!({ "items": [{ "label": "zeta" }, { "label": "alpha" }] });
        let got = completion_items(&bare, "", Encoding::Utf8).items;
        assert_eq!(got[0].label, "alpha");
    }

    /// What the popup needs besides the text: whether to ask again as the
    /// word grows, which items are snippets to expand, what an item filters
    /// as when that is not its label, and the type to show beside it.
    #[test]
    fn a_reply_carries_its_incompleteness_snippets_filters_and_descriptions() {
        let reply = json!({
            "isIncomplete": true,
            "items": [
                {
                    "label": "push",
                    "kind": 2,
                    "insertTextFormat": 2,
                    "insertText": "push($0)",
                    "labelDetails": { "detail": "(…)", "description": "fn(&mut self, T)" },
                },
                {
                    "label": "if",
                    "filterText": "if",
                    "kind": 15,
                    "insertTextFormat": 2,
                    "insertText": "if ${1:cond} {\n\t$0\n}",
                },
                { "label": "if expr {}", "filterText": "if", "kind": 15 },
            ],
        });
        let list = completion_items(&reply, "", Encoding::Utf8);
        assert!(list.incomplete);
        let push = list.items.iter().find(|i| i.label == "push").unwrap();
        assert!(push.snippet);
        assert_eq!(push.insert, "push($0)");
        assert_eq!(push.label_detail.as_deref(), Some("(…)"));
        assert_eq!(push.description.as_deref(), Some("fn(&mut self, T)"));
        let keyword = list.items.iter().find(|i| i.label == "if").unwrap();
        assert_eq!(
            keyword.filter, None,
            "a filter that is the label says nothing"
        );
        let postfix = list.items.iter().find(|i| i.label == "if expr {}").unwrap();
        assert_eq!(postfix.filter.as_deref(), Some("if"));
        assert!(
            !postfix.snippet,
            "plain text unless the server says snippet"
        );

        // A bare array cannot say it is incomplete.
        let bare = completion_items(&json!([{ "label": "x" }]), "", Encoding::Utf8);
        assert!(!bare.incomplete);
    }

    /// An edit for another file no longer refuses the action: it is split
    /// into this file's edits and the rest, which the client writes the way
    /// a rename is written. A file operation still refuses it whole.
    #[test]
    fn an_action_touching_another_file_is_split_and_a_file_operation_refused() {
        let ours = "file:///E:/proj/src/main.rs";
        let mine = json!({ "changes": { "file:///e:/proj/src/main.rs": [edit(0, 0, 1, "x")] } });
        let (own, others) = split_edits(&mine, ours).expect("splits");
        assert_eq!((own.len(), others.len()), (1, 0));

        let theirs = json!({ "changes": {
            "file:///E:/proj/src/main.rs": [edit(0, 0, 1, "x")],
            "file:///E:/proj/src/lib.rs": [edit(0, 0, 1, "x")],
        } });
        let (own, others) = split_edits(&theirs, ours).expect("splits");
        assert_eq!(own.len(), 1);
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].0, "file:///E:/proj/src/lib.rs");

        let moves =
            json!({ "documentChanges": [{ "kind": "rename", "oldUri": ours, "newUri": ours }] });
        assert_eq!(split_edits(&moves, ours), None);
    }

    /// `unlinked-file` arrives as a hint and leaves as a warning: it is the
    /// one hint that turns the server off for the whole file, and a hint is
    /// what the Problems panel leaves out.
    #[test]
    fn an_unlinked_file_is_a_warning_however_the_server_files_it() {
        let items = json!([
            {
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
                "severity": 4,
                "code": "unlinked-file",
                "message": "not in the module tree",
            },
            {
                "range": { "start": { "line": 3, "character": 0 }, "end": { "line": 3, "character": 1 } },
                "severity": 4,
                "code": "inactive-code",
                "message": "cfg is off",
            },
        ]);
        let got = diagnostics(&items, None, Encoding::Utf8);
        assert_eq!(got[0].severity, DiagSeverity::Warning);
        assert_eq!(got[0].code.as_deref(), Some("unlinked-file"));
        assert_eq!(
            got[1].severity,
            DiagSeverity::Hint,
            "an ordinary hint stays one"
        );
    }
}

#[cfg(test)]
mod flyimport_tests {
    use super::*;
    use serde_json::json;

    /// An item not yet in scope carries its import as an additional edit,
    /// and the note beside the label says so — the two halves of what VS
    /// Code shows as `HashMap (use std::collections::HashMap)`.
    #[test]
    fn an_import_travels_as_an_additional_edit_with_its_note() {
        let text = "fn main() {\n    let m = HashM\n}\n";
        let reply = json!({ "items": [{
            "label": "HashMap",
            "kind": 22,
            "labelDetails": { "detail": " (use std::collections::HashMap)" },
            "sortText": "7fffffff",
            "data": { "position": 1 },
        }] });
        let items = completion_items(&reply, text, Encoding::Utf8).items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].index, 0);
        assert_eq!(
            items[0].label_detail.as_deref(),
            Some(" (use std::collections::HashMap)")
        );

        let resolved = json!({
            "label": "HashMap",
            "additionalTextEdits": [{
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 0 },
                },
                "newText": "use std::collections::HashMap;\n\n",
            }],
        });
        let edits = completion_additional_edits(&resolved, text, Encoding::Utf8);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "use std::collections::HashMap;\n\n");
        assert_eq!(
            (edits[0].range.start_line, edits[0].range.start_col),
            (0, 0)
        );
        // An item with nothing to add yields nothing, not an error.
        assert!(
            completion_additional_edits(&json!({ "label": "x" }), text, Encoding::Utf8).is_empty()
        );
    }

    /// The message a real esp-pinned project produced, cut to its shape:
    /// rust-analyzer passing a flag Espressif's cargo does not have, and the
    /// workspace not loading because of it. The sentence has to name the
    /// manifest and the remedy — the raw text names neither, and ninety
    /// lines of cargo usage in the dock read as rusty being broken.
    #[test]
    fn a_toolchain_in_the_broken_window_is_named_with_the_way_out() {
        let raw = "Failed to read Cargo metadata with dependencies for \
                   `E:\\CodeBase\\flyegg\\firmware\\Cargo.toml`: `cargo metadata` exited with \
                   an error: error: unexpected argument '--lockfile-path' found\n\n  tip: a \
                   similar argument exists: '--locked'\n";
        let named = explain_health(raw).expect("the flag mismatch is recognised");
        assert!(
            named.contains("E:\\CodeBase\\flyegg\\firmware\\Cargo.toml"),
            "the manifest that failed is named: {named}"
        );
        // And the remedy, which is an upgrade. Two earlier versions of this
        // sentence sent people somewhere that does not work — one to `espup
        // update` for the wrong reason, one to an older rust-analyzer, which
        // was measured afterwards as not fixing it at all.
        assert!(
            named.contains("Updating the Xtensa toolchain"),
            "the way out is named, and it is forwards: {named}"
        );
        assert!(
            !named.contains("rust-analyzer from before") && !named.contains("1.90"),
            "and it is never an older analyzer: {named}"
        );
        // And what it actually costs, which is not "nothing works":
        // rust-analyzer retries with `--no-deps` and carries on.
        assert!(
            named.contains("Dependencies will not resolve"),
            "the consequence is stated, and stated correctly: {named}"
        );

        // Everything else is the server's own words, untouched. A wrapper
        // that rephrased every failure would hide the ones nobody has
        // written a sentence for yet.
        assert!(explain_health("unresolved import `foo`").is_none());
        assert!(explain_health("").is_none());
    }
}
