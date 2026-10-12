//! What one save costs the server: the checks it sets running.
//!
//! `cargo run -p rusty-lsp --example save_probe -- <project> <relative-file> [seconds]`
//!
//! Opens the file, waits for the server to settle and its first check to
//! finish, then saves the file the way the app does — the text written back
//! unchanged, `didSave`, and the watcher's `didChangeWatchedFiles` for it —
//! and prints, with the time, every piece of progress the server reports
//! after, and every `Refresh`: the client emits one each time it sends
//! `rust-analyzer/runFlycheck` of its own, on the server turning quiescent.
//! One save is one check of the saved file's workspace; anything more is
//! a check nobody asked for, and on a machine that also builds firmware
//! with `build-std` that is the difference between an editor that keeps
//! up and one that lags behind every keystroke.

use std::time::{Duration, Instant};

use rusty_lsp::{LspClient, LspEvent, watched::FileChange};

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("project path"));
    let file = args.next().expect("relative file");
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(180);

    let text = std::fs::read_to_string(root.join(&file)).expect("read the file");
    let started = Instant::now();
    let at = || started.elapsed().as_secs_f32();
    let (client, events) = LspClient::spawn(&root, None, None).expect("spawn");
    client.did_open(&file, &text).expect("didOpen");
    eprintln!("opened {file}; watching {seconds}s");

    let deadline = started + Duration::from_secs(seconds);
    let mut last_progress = Instant::now();
    let mut settled = false;
    let mut saved: Option<f32> = None;
    let mut refreshes_after_save = 0;
    let mut checks_after_save = 0;
    let mut checking = false;
    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(500)) {
            Some(LspEvent::Refresh {}) => {
                eprintln!(
                    "[{:>6.1}s] Refresh — the client sent runFlycheck (every workspace)",
                    at()
                );
                settled = true;
                if saved.is_some() {
                    refreshes_after_save += 1;
                }
            }
            Some(LspEvent::Progress { text }) => {
                last_progress = Instant::now();
                let now_checking = text.as_deref().is_some_and(|t| t.contains("cargo check"));
                if now_checking && !checking && saved.is_some() {
                    checks_after_save += 1;
                }
                checking = now_checking;
                eprintln!(
                    "[{:>6.1}s] progress: {}",
                    at(),
                    text.as_deref().unwrap_or("(done)")
                );
            }
            Some(LspEvent::Health { level, message }) => {
                eprintln!(
                    "[{:>6.1}s] health {level:?}: {}",
                    at(),
                    message.unwrap_or_default()
                );
            }
            Some(_) => {}
            None => {}
        }
        // Settled, the first check done and quiet for a while: save once.
        if saved.is_none() && settled && last_progress.elapsed() > Duration::from_secs(8) {
            std::fs::write(root.join(&file), &text).expect("write the file back");
            client.did_save(&file).expect("didSave");
            client
                .did_change_watched_files(&[(file.clone(), FileChange::Changed)])
                .expect("didChangeWatchedFiles");
            saved = Some(at());
            eprintln!("[{:>6.1}s] ---- saved {file} ----", at());
        }
        if let Some(when) = saved
            && at() - when > 60.0
            && last_progress.elapsed() > Duration::from_secs(8)
        {
            break;
        }
    }
    match saved {
        Some(_) => eprintln!(
            "after the save: {checks_after_save} check(s) started, {refreshes_after_save} runFlycheck(s) sent by the client"
        ),
        None => eprintln!("never settled enough to save"),
    }
}
