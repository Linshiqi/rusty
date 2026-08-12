//! Driven against a real rust-analyzer, when one is installed.
//!
//! The unit tests prove the arithmetic; they cannot prove the handshake, the
//! framing against a real peer, or that diagnostics actually arrive — the
//! parts that fail in ways no mock predicts. Skips with a message when
//! rust-analyzer is absent, so a machine without it still gets a green suite.

use std::time::{Duration, Instant};

use rusty_lsp::{DiagSeverity, LspClient, LspEvent, find_rust_analyzer};

/// A CJK comment above the interesting lines, so a position system that
/// silently assumes ASCII cannot pass.
fn source() -> String {
    [
        "// 中文注释：逼出 UTF-8 与 UTF-16 位置换算的差异",
        "struct Widget;",
        "",
        "impl Widget {",
        "    fn frobnicate(&self) -> u32 {",
        "        7",
        "    }",
        "    fn mix(&self, gain: u32, bias: i32) -> u32 {",
        "        gain.wrapping_add(bias as u32)",
        "    }",
        "}",
        "",
        "fn build() -> Widget {",
        "    Widget",
        "}",
        "",
        "fn main() {",
        "    let w = build();",
        "    let _n = w.frobnicate();",
        "    let _m = w.mix(1, 2);",
        "    let _mistake: u32 = \"seven\";",
        "    let _table = HashMap::new();",
        "}",
        "",
    ]
    .join("\n")
}

fn line_of(text: &str, needle: &str) -> u32 {
    text.lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle}")) as u32
}

/// Retry a request while the index warms up. Cold rust-analyzer answers
/// slowly, emptily, or with "content modified" — none of which is a failure,
/// just earliness.
fn eventually<T>(within: Duration, mut attempt: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + within;
    loop {
        if let Some(value) = attempt() {
            return Some(value);
        }
        if Instant::now() > deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[test]
fn rust_analyzer_end_to_end() {
    if find_rust_analyzer().is_none() {
        eprintln!("skipping: rust-analyzer is not installed on this machine");
        return;
    }

    // No dot-named component anywhere in the path. rust-analyzer's VFS treats
    // dot-directories as hidden — with one anywhere above the project, native
    // analysis silently never loads: no diagnostics, and no message saying
    // why. tempfile's default `.tmpXXXX` has exactly that shape; flycheck used
    // to paper over the hole with rustc's own errors, which is how this test
    // passed before flycheck was turned off.
    let dir = tempfile::Builder::new()
        .prefix("rusty-lsp-")
        .tempdir()
        .expect("tempdir");
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let text = source();
    std::fs::write(root.join("src/main.rs"), &text).unwrap();

    let (client, events) = LspClient::spawn(&root, None).expect("spawn rust-analyzer");
    client.did_open("src/main.rs", &text).expect("didOpen");

    // ── diagnostics are pushed, and land on the right line ───────────────────
    let mistake_line = line_of(&text, "_mistake");
    let deadline = Instant::now() + Duration::from_secs(90);
    let diagnostic = loop {
        assert!(
            Instant::now() < deadline,
            "no type-mismatch diagnostic arrived within 90s",
        );
        match events.recv_timeout(Duration::from_secs(5)) {
            Some(LspEvent::Diagnostics { path, items }) if path == "src/main.rs" => {
                if let Some(found) = items
                    .iter()
                    .find(|d| d.severity == DiagSeverity::Error && d.message.contains("expected"))
                {
                    break found.clone();
                }
            }
            Some(LspEvent::Exited {}) => panic!("rust-analyzer exited during the test"),
            _ => {}
        }
    };
    assert_eq!(
        diagnostic.start_line, mistake_line,
        "the CJK comment above must not shift the diagnostic: {diagnostic:?}",
    );
    let mistake_text = text.lines().nth(mistake_line as usize).unwrap();
    let string_col = mistake_text.find('"').unwrap() as u32;
    assert_eq!(
        diagnostic.start_col, string_col,
        "the squiggle starts under the string literal: {diagnostic:?}",
    );

    // ── completion mid-identifier ────────────────────────────────────────────
    let call_line = line_of(&text, "w.frobnicate");
    let call_text = text.lines().nth(call_line as usize).unwrap();
    let partial_col = (call_text.find("frobnicate").unwrap() + 3) as u32; // after "fro"
    let completions = eventually(Duration::from_secs(45), || {
        client
            .completion("src/main.rs", call_line, partial_col)
            .ok()
            .filter(|items| items.iter().any(|i| i.label.starts_with("frobnicate")))
    })
    .expect("completion never offered `frobnicate`");
    let item = completions
        .iter()
        .find(|i| i.label.starts_with("frobnicate"))
        .unwrap();
    assert_eq!(item.kind.as_deref(), Some("method"));

    // ── hover says what it is, and what it covers ────────────────────────────
    let hover = eventually(Duration::from_secs(30), || {
        client
            .hover("src/main.rs", call_line, partial_col)
            .ok()
            .flatten()
            .filter(|info| info.text.contains("frobnicate"))
    })
    .expect("hover never described frobnicate");
    assert!(
        hover.text.contains("u32"),
        "the signature names the return type: {}",
        hover.text,
    );
    // The range is what keeps the tooltip up while the pointer moves within
    // the token, so it has to actually contain the queried column.
    let range = hover.range.expect("hover carries the token's range");
    assert_eq!(range.start_line, call_line);
    assert!(
        (range.start_col..range.end_col).contains(&partial_col),
        "the token range must cover the queried column: {range:?}",
    );

    // ── signature help knows which parameter the caret is on ─────────────────
    let mix_line = line_of(&text, "w.mix");
    let mix_text = text.lines().nth(mix_line as usize).unwrap();
    // Inside the second argument.
    let second_arg_col = (mix_text.rfind('2').unwrap()) as u32;
    let signature = eventually(Duration::from_secs(30), || {
        client
            .signature_help("src/main.rs", mix_line, second_arg_col)
            .ok()
            .flatten()
    })
    .expect("signature help never answered inside the call");
    assert!(
        signature.label.contains("fn mix"),
        "the label is the whole signature: {}",
        signature.label,
    );
    let (start, end) = (
        signature.param_start.expect("active parameter start") as usize,
        signature.param_end.expect("active parameter end") as usize,
    );
    assert_eq!(
        &signature.label[start..end],
        "bias: i32",
        "after the comma the second parameter is the active one: {}",
        signature.label,
    );

    // ── semantic tokens name the struct at the right scalar column ──────────
    let widget_line = line_of(&text, "struct Widget");
    let widget_col = text
        .lines()
        .nth(widget_line as usize)
        .unwrap()
        .find("Widget")
        .unwrap() as u32;
    let spans = eventually(Duration::from_secs(30), || {
        client
            .semantic_tokens("src/main.rs")
            .ok()
            .filter(|spans| !spans.is_empty())
    })
    .expect("semantic tokens never arrived");
    let widget = spans
        .iter()
        .find(|s| {
            s.line == widget_line
                && s.start_col <= widget_col
                && widget_col < s.start_col + s.length
        })
        .unwrap_or_else(|| panic!("no token covers Widget: {spans:?}"));
    assert_eq!(widget.kind, "struct", "{widget:?}");
    assert_eq!(
        widget.start_col, widget_col,
        "scalar columns, exactly — the CJK comment above would shift a byte count",
    );
    assert_eq!(widget.length, "Widget".chars().count() as u32);

    // ── definition lands on the declaration ──────────────────────────────────
    let use_line = line_of(&text, "let w = build");
    let use_text = text.lines().nth(use_line as usize).unwrap();
    let use_col = (use_text.find("build").unwrap() + 1) as u32;
    let location = eventually(Duration::from_secs(30), || {
        client
            .definition("src/main.rs", use_line, use_col)
            .ok()
            .flatten()
    })
    .expect("definition never resolved");
    assert_eq!(location.path, "src/main.rs");
    assert_eq!(location.line, line_of(&text, "fn build"));

    // ── code actions: auto-import resolves and applies ───────────────────────
    let table_line = line_of(&text, "HashMap::new");
    let table_col = text
        .lines()
        .nth(table_line as usize)
        .unwrap()
        .find("HashMap")
        .unwrap() as u32
        + 1;
    let fixes = eventually(Duration::from_secs(30), || {
        client
            .code_actions("src/main.rs", table_line, table_col)
            .ok()
            .filter(|fixes| fixes.iter().any(|f| f.title.contains("Import")))
    })
    .expect("no import action was ever offered for HashMap");
    let import = fixes
        .iter()
        .find(|f| f.title.contains("Import"))
        .expect("filtered above");
    assert!(!import.edits.is_empty(), "{import:?}");

    // Apply to a copy the way the frontend does: bottom-up splices.
    let mut patched = text.clone();
    let mut edits = import.edits.clone();
    edits.sort_by_key(|edit| {
        std::cmp::Reverse((edit.range.start_line, edit.range.start_col))
    });
    for edit in edits {
        let offset = |line: u32, col: u32| -> usize {
            let mut at = 0;
            for (index, l) in patched.split('\n').enumerate() {
                if index as u32 == line {
                    return at + l.chars().take(col as usize).map(char::len_utf8).sum::<usize>();
                }
                at += l.len() + 1;
            }
            patched.len()
        };
        let from = offset(edit.range.start_line, edit.range.start_col);
        let to = offset(edit.range.end_line, edit.range.end_col).max(from);
        patched.replace_range(from..to, &edit.new_text);
    }
    assert!(
        patched.contains("use std::collections::HashMap;"),
        "the import landed:
{patched}",
    );

    drop(client); // kills the server; the events channel closes behind it
}
