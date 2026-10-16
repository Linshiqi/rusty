//! Ask a real project for a file's outline and the workspace's symbols,
//! headless — the check behind "the finder's `#` finds nothing".
//!
//! `cargo run -p rusty-lsp --example symbol_probe -- <project> <relative-file> <query>`
//!
//! Starts rust-analyzer the way the app does, opens the file, and asks for
//! its outline and for the workspace's symbols matching `query` every two
//! seconds until the workspace answers or the budget runs out — printing
//! each answer's size beside whether the server has settled
//! (`LspClient::is_quiescent`) and what it says it is busy with, because an
//! empty answer from a server still loading and one from a server that has
//! nothing by that name look the same the first time.

use std::time::{Duration, Instant};

use rusty_lsp::{Events, LspClient, LspEvent};

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("project path"));
    let file = args.next().expect("relative file");
    let query = args.next().expect("a name to search for");
    let patience = std::env::var("RUSTY_PROBE_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);

    eprintln!("spawning rust-analyzer for {}", root.display());
    let started = Instant::now();
    let (client, events) = LspClient::spawn(&root, None, None).expect("spawn");
    let text = std::fs::read_to_string(root.join(&file)).expect("the file reads");
    client.did_open(&file, &text).expect("didOpen");
    let budget = Duration::from_secs(patience);
    loop {
        drain(&events, &started);
        let outline = client.document_symbols(&file);
        let found = client.workspace_symbols(&query);
        eprintln!(
            "{:>5.1?}: settled {} · outline {} · `{query}` {}",
            started.elapsed(),
            client.is_quiescent(),
            match &outline {
                Ok(list) => format!("{} symbols", list.len()),
                Err(error) => format!("error {error}"),
            },
            match &found {
                Ok(list) => format!(
                    "{} symbols {:?}",
                    list.len(),
                    list.iter()
                        .take(6)
                        .map(|s| format!(
                            "{} {} {}:{}",
                            s.kind,
                            s.name,
                            s.location.path,
                            s.location.line + 1
                        ))
                        .collect::<Vec<_>>()
                ),
                Err(error) => format!("error {error}"),
            }
        );
        if found.as_ref().is_ok_and(|list| !list.is_empty()) {
            eprintln!("PROBE OK");
            return;
        }
        if started.elapsed() > budget {
            eprintln!("PROBE FAILED: nothing for `{query}` within {budget:?}");
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// What the server has said since the last look: what it is busy with, and
/// when its analysis moved on.
fn drain(events: &Events, started: &Instant) {
    while let Some(event) = events.recv_timeout(Duration::from_millis(10)) {
        match event {
            LspEvent::Progress { text: Some(text) } => {
                eprintln!("{:>5.1?}:    progress: {text}", started.elapsed())
            }
            LspEvent::Refresh {} => eprintln!("{:>5.1?}:    refresh", started.elapsed()),
            LspEvent::Health { level, message } => eprintln!(
                "{:>5.1?}:    health {level:?} {}",
                started.elapsed(),
                message.unwrap_or_default()
            ),
            LspEvent::Exited {} => {
                eprintln!("PROBE FAILED: rust-analyzer exited");
                std::process::exit(1);
            }
            _ => {}
        }
    }
}
