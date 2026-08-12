//! Drive the client against a real project, headless.
//!
//! `cargo run -p rusty-lsp --example probe -- <project> <relative-file>`
//!
//! Opens the file with a type error appended **in the buffer only** — the disk
//! is not touched — and waits for diagnostics. This is how "no squiggles in
//! the window" gets split into "the pipeline is broken" versus "the file
//! simply has no errors": if the injected error comes back with the right
//! line number, everything up to the frontend is working.

use std::time::{Duration, Instant};

use rusty_lsp::{LspClient, LspEvent};

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("project path"));
    let file = args.next().expect("relative file");

    let on_disk = std::fs::read_to_string(root.join(&file)).expect("read the file");
    let injected = format!("{on_disk}\nfn __rusty_probe() {{ let _x: u32 = \"not a u32\"; }}\n");
    let error_line = injected
        .lines()
        .position(|line| line.contains("__rusty_probe"))
        .expect("the injected line") as u32;

    eprintln!("spawning rust-analyzer for {}", root.display());
    let started = Instant::now();
    let (client, events) = LspClient::spawn(&root, None).expect("spawn");
    eprintln!("handshake in {:.1?}", started.elapsed());

    client.did_open(&file, &injected).expect("didOpen");
    eprintln!("opened {file} with an injected error at line {error_line}");

    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_secs(5)) {
            Some(LspEvent::Diagnostics { path, items }) => {
                eprintln!("← {} diagnostics for {path}", items.len());
                for d in &items {
                    eprintln!(
                        "   {:?} {}:{}..{}:{} [{}] {}",
                        d.severity,
                        d.start_line,
                        d.start_col,
                        d.end_line,
                        d.end_col,
                        d.source.as_deref().unwrap_or("?"),
                        d.message.lines().next().unwrap_or(""),
                    );
                }
                if path == file && items.iter().any(|d| d.start_line == error_line) {
                    eprintln!(
                        "injected error on line {error_line} after {:.1?}; watching for a wipe…",
                        started.elapsed(),
                    );
                    // The failure mode being probed is not "never arrives" but
                    // "arrives and is wiped moments later" — so hold on and see.
                    let quiet_until = Instant::now() + Duration::from_secs(45);
                    while Instant::now() < quiet_until {
                        if let Some(LspEvent::Diagnostics { path, items }) =
                            events.recv_timeout(Duration::from_secs(5))
                            && path == file
                            && !items.iter().any(|d| d.start_line == error_line)
                        {
                            eprintln!("PROBE FAILED: the diagnostic was wiped");
                            std::process::exit(1);
                        }
                    }
                    eprintln!("PROBE OK: the diagnostic survived 45s");
                    return;
                }
            }
            Some(LspEvent::Exited {}) => {
                eprintln!("PROBE FAILED: rust-analyzer exited");
                std::process::exit(1);
            }
            _ => eprintln!("… waiting ({:.0?})", started.elapsed()),
        }
    }
    eprintln!("PROBE FAILED: no diagnostic for the injected error within 180s");
    std::process::exit(1);
}
