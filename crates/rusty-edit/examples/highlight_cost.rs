//! What painting a file costs, whole and after an edit — the check that a
//! keystroke in a long file costs its own line and not the file.
//!
//! `cargo run -p rusty-edit --features backend --release --example highlight_cost -- <file>`
//!
//! Paints the file whole, then changes one line halfway down, then opens a
//! block comment at the top — the edit that genuinely repaints everything
//! below it — and says how long each took and how many lines went back.

use std::time::{Duration, Instant};

fn main() {
    let path = std::env::args().nth(1).expect("a file to paint");
    let text = std::fs::read_to_string(&path).expect("readable text");
    let files = rusty_edit::Files::new();

    let (whole, took) = timed(|| files.repaint(&path, &text, None, None));
    report("whole", &whole, took);

    let lines: Vec<&str> = text.split('\n').collect();
    let middle = lines.len() / 2;
    let mut edited = lines.clone();
    let changed = format!("{} // edited", lines[middle]);
    edited[middle] = &changed;
    let edited = edited.join("\n");
    let (edit, took) = timed(|| files.repaint(&path, &edited, Some(whole.version), None));
    report(&format!("line {} edited", middle + 1), &edit, took);

    let opened = format!("/*\n{edited}");
    let (comment, took) = timed(|| files.repaint(&path, &opened, Some(edit.version), None));
    report("comment opened at the top", &comment, took);
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let out = work();
    (out, started.elapsed())
}

fn report(what: &str, repaint: &rusty_edit::Repaint, took: Duration) {
    // `{"text":"…","token":"keyword"},` — what a span costs on the wire,
    // near enough.
    let wire: usize = repaint
        .lines
        .iter()
        .flat_map(|line| &line.spans)
        .map(|span| span.text.len() + 32)
        .sum();
    println!(
        "{what}: {} lines from line {} in {took:?}, about {} KB to send",
        repaint.lines.len(),
        repaint.from + 1,
        wire / 1024
    );
}
