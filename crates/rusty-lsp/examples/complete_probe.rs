//! Ask a real project for completions, headless — the check behind "the
//! editor completes nothing".
//!
//! `cargo run -p rusty-lsp --example complete_probe -- <project> <relative-file> [target]`
//!
//! Starts rust-analyzer the way the app does — `target` is the triple the app
//! would hand it for the project, or nothing — opens the file with a buffer of
//! its own (the disk is not touched) and asks for completions where a person
//! types: a keyword (`imp`), a type declared in the file (`impl Qu`) and a
//! path (`impl core::`). Each is asked every two seconds until it answers or
//! the budget runs out, because an empty answer from a server still loading
//! and one from a server that has no crate for the file look the same the
//! first time. The file's diagnostics and the server's progress are printed
//! as they arrive: an `unlinked-file` hint is the server saying which of the
//! two it is.

use std::time::{Duration, Instant};

use rusty_lsp::{Events, LspClient, LspEvent};

/// What the buffer holds above the line being completed.
const HEAD: &str = "// 四元数: q = w + xi + yj + zk\n\
pub struct Quaternion {\n    pub w: f32,\n    pub x: f32,\n    pub y: f32,\n    pub z: f32,\n}\n\n";

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("project path"));
    let file = args.next().expect("relative file");
    let target = args.next();
    let patience = std::env::var("RUSTY_PROBE_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);

    eprintln!(
        "spawning rust-analyzer for {} (target: {target:?})",
        root.display()
    );
    let started = Instant::now();
    let (client, events) = LspClient::spawn(&root, target.as_deref(), None).expect("spawn");
    client
        .did_open(&file, &format!("{HEAD}imp"))
        .expect("didOpen");

    let line = HEAD.lines().count() as u32;
    let mut budget = Duration::from_secs(patience);
    let mut failed = false;
    for tail in ["imp", "impl Qu", "impl core::"] {
        client
            .did_change(&file, &format!("{HEAD}{tail}"))
            .expect("didChange");
        let col = tail.chars().count() as u32;
        let asked = Instant::now();
        loop {
            drain(&events, &file);
            match client.completion(&file, line, col) {
                Ok(list) if !list.items.is_empty() => {
                    // What the popup would keep: the items starting with the
                    // word being typed, or the first few after a `::`.
                    let word = tail.rsplit([' ', ':']).next().unwrap_or("");
                    let first: Vec<&str> = list
                        .items
                        .iter()
                        .map(|item| item.label.as_str())
                        .filter(|label| label.to_lowercase().starts_with(&word.to_lowercase()))
                        .take(8)
                        .collect();
                    eprintln!(
                        "`{tail}`: {} items after {:.1?} ({:.1?} since spawn): {first:?}",
                        list.items.len(),
                        asked.elapsed(),
                        started.elapsed(),
                    );
                    break;
                }
                Ok(_) => {}
                Err(error) => eprintln!("`{tail}`: error {error}"),
            }
            if asked.elapsed() > budget {
                eprintln!("`{tail}`: NOTHING within {budget:?}");
                failed = true;
                // The server has had its chance to load; the next cases
                // only need long enough to answer.
                budget = Duration::from_secs(10);
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    drain(&events, &file);
    if failed {
        eprintln!("PROBE FAILED");
        std::process::exit(1);
    }
    eprintln!("PROBE OK");
}

/// Print what the server has said since the last look: the file's
/// diagnostics, hints included, and what it is busy with.
fn drain(events: &Events, file: &str) {
    while let Some(event) = events.recv_timeout(Duration::from_millis(10)) {
        match event {
            LspEvent::Diagnostics { path, items } if path == file => {
                for d in items {
                    eprintln!(
                        "   diagnostic {:?} {}:{} [{}] {}",
                        d.severity,
                        d.start_line,
                        d.start_col,
                        d.code.as_deref().unwrap_or(""),
                        d.message.lines().next().unwrap_or(""),
                    );
                }
            }
            LspEvent::Progress { text: Some(text) } => eprintln!("   progress: {text}"),
            LspEvent::Exited {} => {
                eprintln!("PROBE FAILED: rust-analyzer exited");
                std::process::exit(1);
            }
            _ => {}
        }
    }
}
