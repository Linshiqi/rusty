//! Driven against a real clangd, when one is installed — the C half of the
//! editor: a PlatformIO or CMake project's server.
//!
//! What a mock cannot prove: that clangd takes the handshake rust-analyzer
//! takes, that its *pushed* diagnostics arrive whole (it declares no pull,
//! so the client must not wait for one), that a CJK comment does not shift
//! a C squiggle, and that completion and hover answer through the same
//! client. Skips with a message when clangd is absent.

use std::time::{Duration, Instant};

use rusty_lsp::{DiagSeverity, LspClient, LspEvent, ServerKind, find_clangd};

fn source() -> String {
    [
        "// 中文注释：逼出 UTF-8 与 UTF-16 位置换算的差异",
        "struct sensor { int raw; };",
        "",
        "static int sensor_read(const struct sensor *s) { return s->raw; }",
        "",
        "int main(void) {",
        "    struct sensor gyro = { 3 };",
        "    int reading = sensor_read(&gyro);",
        "    int mistake = undeclared_thing;",
        "    return reading + mistake;",
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

#[test]
fn clangd_end_to_end() {
    if find_clangd(None).is_none() {
        eprintln!("skipping: clangd is not installed on this machine");
        return;
    }
    assert!(ServerKind::Clangd.serves("src/main.c"));
    assert!(!ServerKind::Clangd.serves("src/main.rs"));

    let dir = tempfile::Builder::new()
        .prefix("rusty-clangd-")
        .tempdir()
        .expect("tempdir");
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    let text = source();
    std::fs::write(root.join("main.c"), &text).unwrap();

    let (client, events) = LspClient::spawn_clangd(&root, None, &[], None).expect("spawn clangd");
    client.did_open("main.c", &text).expect("didOpen");

    // ── pushed diagnostics, on the right line and column ─────────────────────
    let mistake_line = line_of(&text, "undeclared_thing");
    let deadline = Instant::now() + Duration::from_secs(60);
    let diagnostic = loop {
        assert!(
            Instant::now() < deadline,
            "no undeclared-identifier diagnostic arrived within 60s"
        );
        match events.recv_timeout(Duration::from_secs(5)) {
            Some(LspEvent::Diagnostics { path, items }) if path == "main.c" => {
                if let Some(found) = items.iter().find(|d| {
                    d.severity == DiagSeverity::Error && d.message.contains("undeclared_thing")
                }) {
                    break found.clone();
                }
            }
            Some(LspEvent::Exited {}) => panic!("clangd exited during the test"),
            _ => {}
        }
    };
    assert_eq!(diagnostic.start_line, mistake_line, "{diagnostic:?}");
    let line = text.lines().nth(mistake_line as usize).unwrap();
    assert_eq!(
        diagnostic.start_col,
        line.find("undeclared_thing").unwrap() as u32,
        "the squiggle starts under the name: {diagnostic:?}"
    );

    // ── completion of a function by its prefix ───────────────────────────────
    let call_line = line_of(&text, "sensor_read(&gyro)");
    let call = text.lines().nth(call_line as usize).unwrap();
    let after_prefix = (call.find("sensor_read").unwrap() + "sensor_r".len()) as u32;
    let completions = client
        .completion("main.c", call_line, after_prefix)
        .expect("completion");
    assert!(
        completions
            .items
            .iter()
            .any(|item| item.label.contains("sensor_read")),
        "{:?}",
        completions
            .items
            .iter()
            .map(|i| &i.label)
            .collect::<Vec<_>>()
    );

    // ── hover names the function's type ──────────────────────────────────────
    let hover = client
        .hover("main.c", call_line, after_prefix)
        .expect("hover")
        .expect("something to say");
    assert!(hover.text.contains("sensor_read"), "{}", hover.text);
    assert!(hover.text.contains("int"), "{}", hover.text);
}
