//! Every diagnostic the client emits for a project, and who said it.
//!
//! `cargo run -p rusty-lsp --example diag_probe -- <project> <relative-file> [seconds]`
//!
//! Opens the file as it is on disk — saving nothing, because the client runs
//! `cargo check` itself once the server has settled — and prints each
//! publish for as long as asked:
//! the file, how many items, and each item's severity, position, source
//! (`rust-analyzer` for its own analysis, `rustc` for the check) and first
//! line. It is the check for "the error is in Output but not in the editor":
//! it says whether the server never sent it, or sent it and something later
//! took it away.

use std::time::{Duration, Instant};

use rusty_lsp::{LspClient, LspEvent};

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("project path"));
    let file = args.next().expect("relative file");
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(120);

    let text = std::fs::read_to_string(root.join(&file)).expect("read the file");
    let started = Instant::now();
    let (client, events) = LspClient::spawn(&root, None, None).expect("spawn");
    client.did_open(&file, &text).expect("didOpen");
    eprintln!("opened {file}; watching {seconds}s");

    let deadline = started + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_secs(5)) {
            Some(LspEvent::Diagnostics { path, items }) => {
                eprintln!(
                    "[{:>5.1}s] {} item(s) for {path}",
                    started.elapsed().as_secs_f32(),
                    items.len()
                );
                for d in &items {
                    eprintln!(
                        "         {:?} {}:{} [{}{}] {}",
                        d.severity,
                        d.start_line + 1,
                        d.start_col + 1,
                        d.source.as_deref().unwrap_or("?"),
                        d.code
                            .as_deref()
                            .map(|code| format!(" {code}"))
                            .unwrap_or_default(),
                        d.message.lines().next().unwrap_or(""),
                    );
                }
            }
            Some(LspEvent::Exited {}) => {
                eprintln!("rust-analyzer exited");
                std::process::exit(1);
            }
            _ => {}
        }
    }
    drop(client);
}
