//! A mistake typed and taken back: does the editor's squiggle go with it?
//!
//! `cargo run -p rusty-lsp --example fix_probe -- <project> <relative-file> <line> <from> <to> [--save]`
//!
//! Opens the file, waits for the server to settle, then replaces the first
//! `from` on `line` (1-based) with `to` in the draft — the mistake — and
//! after a while puts the line back, printing every diagnostics publish
//! for the file with the time. Without `--save` the mistake never reaches
//! the disk, which is the ordinary case of a typo noticed and mended; with
//! it the broken text is saved, so the check (`rustc`) reports it too, and
//! the mended text is left unsaved for a while before it is saved in turn.
//! What it settles is whether a squiggle left on screen is the server's —
//! still in its last answer — or the window's.

use std::time::{Duration, Instant};

use rusty_lsp::{LspClient, LspEvent, watched::FileChange};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = std::path::PathBuf::from(&args[0]);
    let file = args[1].clone();
    let line: usize = args[2].parse().expect("a line number");
    let (from, to) = (args[3].clone(), args[4].clone());
    let save = args.iter().any(|a| a == "--save");

    let good = std::fs::read_to_string(root.join(&file)).expect("read the file");
    let bad = good
        .split('\n')
        .enumerate()
        .map(|(n, text)| {
            if n + 1 == line {
                text.replacen(&from, &to, 1)
            } else {
                text.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(good, bad, "`{from}` is not on line {line}");

    let started = Instant::now();
    let at = || started.elapsed().as_secs_f32();
    let (client, events) = LspClient::spawn(&root, None, None).expect("spawn");
    client.did_open(&file, &good).expect("didOpen");
    eprintln!("opened {file}");

    let write = |text: &str| {
        std::fs::write(root.join(&file), text).expect("write");
        client.did_save(&file).expect("didSave");
        client
            .did_change_watched_files(&[(file.clone(), FileChange::Changed)])
            .expect("didChangeWatchedFiles");
    };
    let watch = |seconds: u64, what: &str| {
        eprintln!("[{:>6.1}s] ---- {what} ----", at());
        let until = Instant::now() + Duration::from_secs(seconds);
        let mut last = None;
        while Instant::now() < until {
            match events.recv_timeout(Duration::from_millis(250)) {
                Some(LspEvent::Diagnostics { path, items }) if path == file => {
                    eprintln!("[{:>6.1}s] {} item(s)", at(), items.len());
                    for d in &items {
                        eprintln!(
                            "           {:?} {}:{}..{}:{} [{}{}] {}",
                            d.severity,
                            d.start_line + 1,
                            d.start_col + 1,
                            d.end_line + 1,
                            d.end_col + 1,
                            d.source.as_deref().unwrap_or("?"),
                            d.code
                                .as_deref()
                                .map(|c| format!(" {c}"))
                                .unwrap_or_default(),
                            d.message.lines().next().unwrap_or(""),
                        );
                    }
                    last = Some(items.len());
                }
                Some(LspEvent::Progress { text: Some(text) }) if text.contains("check") => {
                    eprintln!("[{:>6.1}s] progress: {text}", at());
                }
                _ => {}
            }
        }
        last
    };

    // Settled: the server indexed and the client's first check ran.
    watch(60, "settling");
    client.did_change(&file, &bad).expect("the mistake");
    if save {
        write(&bad);
    }
    watch(if save { 45 } else { 15 }, "the mistake typed");
    client.did_change(&file, &good).expect("mended");
    let unsaved = watch(15, "mended, not saved");
    if save {
        write(&good);
        let saved = watch(45, "mended and saved");
        eprintln!(
            "after the mend: {unsaved:?} unsaved, {saved:?} saved (None: nothing new published)"
        );
    } else {
        eprintln!("after the mend: {unsaved:?} (None: nothing new published)");
    }
    std::fs::write(root.join(&file), &good).expect("put the file back");
}
